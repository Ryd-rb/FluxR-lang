//! Deterministic, bounded offline music rendering using external FluxR libraries.
//!
//! This is not an audio-device callback API. Compilation, allocation, destruction
//! and checkpoint/file IO are outside a future callback contract. Measured block
//! deadlines do not qualify this interpreter or host as hard real-time safe.
use crate::Error;
use crate::cpu_compute::{Cancellation, MemoryBudget, ScalarLibrary, Tensor};
use crate::structured::Limits;
use serde::{Deserialize, Serialize};
use std::time::Instant;

fn fail(message: impl Into<String>) -> Error {
    Error(message.into())
}
pub const PPQN: u64 = 960;
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tempo {
    pub beats_per_minute_milli: u32,
    pub beats_per_bar: u8,
    pub beat_unit: u8,
}
impl Tempo {
    pub fn sample_at(&self, tick: u64, sample_rate: u32) -> Result<u64, Error> {
        if self.beats_per_minute_milli == 0
            || self.beats_per_bar == 0
            || self.beats_per_bar > 32
            || ![1, 2, 4, 8, 16, 32].contains(&self.beat_unit)
            || !(8000..=192000).contains(&sample_rate)
        {
            return Err(fail("invalid tempo/meter/sample-rate policy"));
        }
        let numerator = u128::from(tick) * u128::from(sample_rate) * 60_000;
        let denominator = u128::from(self.beats_per_minute_milli) * u128::from(PPQN);
        u64::try_from(numerator / denominator).map_err(|_| fail("sample time overflow"))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Note {
    pub start_tick: u64,
    pub duration_ticks: u64,
    pub frequency_hz: f32,
    pub velocity: f32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Track {
    pub instrument: usize,
    pub notes: Vec<Note>,
    pub gain: f32,
    /// Linear automation over the whole composition; -1 is left, +1 right.
    pub pan_start: f32,
    pub pan_end: f32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Composition {
    pub tempo: Tempo,
    pub sample_rate: u32,
    pub duration_ticks: u64,
    pub tracks: Vec<Track>,
    pub delay_ticks: u64,
    pub delay_feedback: f32,
    /// One-pole filter coefficient in (0,1]. The operation order is fixed.
    pub lowpass: f32,
}
#[derive(Clone, Copy, Debug)]
pub struct RenderLimits {
    pub max_frames: usize,
    pub max_tracks: usize,
    pub max_events: usize,
    pub max_voices: usize,
    pub max_calls: u64,
    pub block_frames: usize,
    pub invocation: Limits,
}
impl Default for RenderLimits {
    fn default() -> Self {
        Self {
            max_frames: 192_000 * 60,
            max_tracks: 64,
            max_events: 16384,
            max_voices: 128,
            max_calls: 48_000_000,
            block_frames: 128,
            invocation: Limits {
                instructions: 4096,
                cells: 4096,
                frames: 32,
            },
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct RenderReport {
    pub sample_rate: u32,
    pub frames: usize,
    pub channels: usize,
    pub notes: usize,
    pub tracks: usize,
    pub library_identities: Vec<String>,
    pub calls: u64,
    pub blocks: usize,
    pub block_us_p50: f64,
    pub block_us_p95: f64,
    pub block_us_p99: f64,
    pub block_us_max: f64,
    pub missed_block_deadlines: usize,
    pub render_seconds: f64,
    pub audio_seconds: f64,
    pub render_wall_over_audio_ratio: f64,
    pub peak_abs: f32,
    pub real_time_qualified: bool,
    pub active_voice_peak: usize,
    pub payload_peak_bytes: usize,
}
pub struct RenderedAudio {
    pub samples: Tensor,
    pub report: RenderReport,
}
#[derive(Clone)]
struct Event {
    start: usize,
    end: usize,
    frequency: f32,
    velocity: f32,
    track: usize,
    order: usize,
}

pub fn render(
    composition: &Composition,
    libraries: &[ScalarLibrary],
    limits: RenderLimits,
    memory: &MemoryBudget,
    cancellation: &Cancellation,
) -> Result<RenderedAudio, Error> {
    cancellation.check()?;
    if composition.tracks.is_empty()
        || composition.tracks.len() > limits.max_tracks
        || limits.block_frames == 0
        || limits.max_voices == 0
    {
        return Err(fail("render track/block/voice limit invalid"));
    }
    let frames = usize::try_from(
        composition
            .tempo
            .sample_at(composition.duration_ticks, composition.sample_rate)?,
    )
    .map_err(|_| fail("render frame overflow"))?;
    if frames == 0 || frames > limits.max_frames {
        return Err(fail("render frame budget exhausted"));
    }
    let delay_frames = usize::try_from(
        composition
            .tempo
            .sample_at(composition.delay_ticks, composition.sample_rate)?,
    )
    .map_err(|_| fail("delay frame overflow"))?;
    if delay_frames == 0
        || delay_frames > frames
        || !composition.delay_feedback.is_finite()
        || composition.delay_feedback.abs() >= 1.
        || !composition.lowpass.is_finite()
        || !(0.0..=1.0).contains(&composition.lowpass)
        || composition.lowpass == 0.
    {
        return Err(fail("invalid delay/filter configuration"));
    }
    let samples_len = frames
        .checked_mul(2)
        .ok_or_else(|| fail("render sample count overflow"))?;
    let event_count = composition
        .tracks
        .iter()
        .try_fold(0usize, |count, track| count.checked_add(track.notes.len()))
        .ok_or_else(|| fail("event count overflow"))?;
    if event_count > limits.max_events {
        return Err(fail("render event budget exhausted"));
    }
    let mut events = Vec::with_capacity(event_count);
    let mut projected_calls = 0u64;
    for (track_index, track) in composition.tracks.iter().enumerate() {
        if track.instrument >= libraries.len()
            || !track.gain.is_finite()
            || !(0.0..=1.0).contains(&track.gain)
            || !track.pan_start.is_finite()
            || !track.pan_end.is_finite()
            || !(-1.0..=1.0).contains(&track.pan_start)
            || !(-1.0..=1.0).contains(&track.pan_end)
        {
            return Err(fail("invalid track instrument/gain/pan"));
        }
        if libraries[track.instrument].manifest().exports.get("sample") != Some(&3) {
            return Err(fail("instrument ABI requires sample(f32,f32,f32)->f32"));
        }
        for note in &track.notes {
            if note.duration_ticks == 0
                || !note.frequency_hz.is_finite()
                || note.frequency_hz <= 0.
                || note.frequency_hz >= composition.sample_rate as f32 * 0.5
                || !note.velocity.is_finite()
                || !(0.0..=1.0).contains(&note.velocity)
            {
                return Err(fail("invalid note frequency/duration/velocity"));
            }
            let end_tick = note
                .start_tick
                .checked_add(note.duration_ticks)
                .ok_or_else(|| fail("note time overflow"))?;
            if end_tick > composition.duration_ticks {
                return Err(fail("note exceeds composition duration"));
            }
            let start = usize::try_from(
                composition
                    .tempo
                    .sample_at(note.start_tick, composition.sample_rate)?,
            )
            .map_err(|_| fail("note start overflow"))?;
            let end = usize::try_from(
                composition
                    .tempo
                    .sample_at(end_tick, composition.sample_rate)?,
            )
            .map_err(|_| fail("note end overflow"))?;
            if end <= start {
                return Err(fail("note duration rounds to zero samples"));
            }
            projected_calls = projected_calls
                .checked_add((end - start) as u64)
                .ok_or_else(|| fail("render call count overflow"))?;
            events.push(Event {
                start,
                end,
                frequency: note.frequency_hz,
                velocity: note.velocity,
                track: track_index,
                order: events.len(),
            });
        }
    }
    if projected_calls > limits.max_calls {
        return Err(fail("render call budget exhausted"));
    }
    events.sort_by_key(|event| (event.start, event.track, event.order));
    // Preflight overlap before rendering or allocating PCM. This is based on
    // half-open sample ranges; ends at sample n precede starts at sample n.
    let mut edges = Vec::with_capacity(
        event_count
            .checked_mul(2)
            .ok_or_else(|| fail("event edge overflow"))?,
    );
    for event in &events {
        edges.push((event.start, 1i64));
        edges.push((event.end, -1));
    }
    edges.sort_unstable();
    let mut voices = 0i64;
    let mut voice_peak = 0usize;
    for (_, change) in edges {
        voices += change;
        voice_peak = voice_peak.max(voices as usize);
    }
    if voice_peak > limits.max_voices {
        return Err(fail("render active voice budget exhausted"));
    }
    // Rendering storage is precharged, then only converted to an immutable
    // tensor at successful completion. The public budget also sees delay data.
    let output_reservation = memory.reserve_bytes(
        samples_len
            .checked_mul(4)
            .ok_or_else(|| fail("render memory overflow"))?,
    )?;
    let _delay_reservation = memory.reserve_bytes(
        delay_frames
            .checked_mul(8)
            .ok_or_else(|| fail("delay memory overflow"))?,
    )?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(samples_len)
        .map_err(|_| fail("render allocation failed"))?;
    output.resize(samples_len, 0f32);
    let mut delay = Vec::new();
    delay
        .try_reserve_exact(delay_frames)
        .map_err(|_| fail("delay allocation failed"))?;
    delay.resize(delay_frames, [0f32; 2]);
    let mut sessions: Vec<_> = libraries
        .iter()
        .map(|library| library.session(limits.invocation, limits.max_calls, cancellation.clone()))
        .collect();
    let mut active: Vec<usize> = Vec::with_capacity(voice_peak);
    let mut cursor = 0usize;
    let mut filter = [0f32; 2];
    let mut times = Vec::with_capacity(frames.div_ceil(limits.block_frames));
    let mut misses = 0usize;
    let mut peak = 0f32;
    let total_start = Instant::now();
    for block_start in (0..frames).step_by(limits.block_frames) {
        cancellation.check()?;
        let block_end = (block_start + limits.block_frames).min(frames);
        let start_time = Instant::now();
        for frame in block_start..block_end {
            active.retain(|&index| events[index].end > frame);
            while cursor < events.len() && events[cursor].start == frame {
                active.push(cursor);
                cursor += 1;
            }
            let mut mixed = [0f32; 2];
            for &index in &active {
                let event = &events[index];
                let track = &composition.tracks[event.track];
                let elapsed = frame - event.start;
                // f64 phase accumulation is host scheduling policy, reduced to
                // a finite f32 input. The selected instrument algorithm is FluxR.
                let phase = ((elapsed as f64 * f64::from(event.frequency)
                    / f64::from(composition.sample_rate))
                    % 1.) as f32;
                let age = elapsed as f32 / (event.end - event.start) as f32;
                let sample = sessions[track.instrument]
                    .call("sample", &[phase, age, event.velocity])?
                    * track.gain;
                let pan = track.pan_start
                    + (track.pan_end - track.pan_start) * (frame as f32 / frames as f32);
                mixed[0] += sample * (1. - pan) * 0.5;
                mixed[1] += sample * (1. + pan) * 0.5;
            }
            let slot = frame % delay_frames;
            for channel in 0..2 {
                let wet = mixed[channel] + delay[slot][channel] * composition.delay_feedback;
                delay[slot][channel] = wet;
                filter[channel] += composition.lowpass * (wet - filter[channel]);
                if !filter[channel].is_finite() {
                    return Err(fail("non-finite audio result"));
                }
                output[frame * 2 + channel] = filter[channel];
                peak = peak.max(filter[channel].abs());
            }
        }
        let seconds = start_time.elapsed().as_secs_f64();
        times.push(seconds * 1e6);
        misses += usize::from(
            seconds > (block_end - block_start) as f64 / f64::from(composition.sample_rate),
        );
    }
    let render_seconds = total_start.elapsed().as_secs_f64();
    let calls: u64 = sessions
        .iter()
        .map(|session| session.completed_calls())
        .sum();
    if calls != projected_calls {
        return Err(fail("render call ledger mismatch"));
    }
    times.sort_by(f64::total_cmp);
    let quantile = |percent: usize| {
        times[(times.len() * percent)
            .div_ceil(100)
            .saturating_sub(1)
            .min(times.len() - 1)]
    };
    let audio_seconds = frames as f64 / f64::from(composition.sample_rate);
    let peak_payload = memory.peak_bytes();
    // Move the charged PCM allocation; no full extra output copy is required.
    let samples = Tensor::from_owned_reserved(&[frames, 2], output, output_reservation)?;
    Ok(RenderedAudio {
        samples,
        report: RenderReport {
            sample_rate: composition.sample_rate,
            frames,
            channels: 2,
            notes: event_count,
            tracks: composition.tracks.len(),
            library_identities: libraries
                .iter()
                .map(|library| library.identity().into())
                .collect(),
            calls,
            blocks: times.len(),
            block_us_p50: quantile(50),
            block_us_p95: quantile(95),
            block_us_p99: quantile(99),
            block_us_max: *times.last().unwrap(),
            missed_block_deadlines: misses,
            render_seconds,
            audio_seconds,
            render_wall_over_audio_ratio: render_seconds / audio_seconds,
            peak_abs: peak,
            real_time_qualified: false,
            active_voice_peak: voice_peak,
            payload_peak_bytes: peak_payload,
        },
    })
}
