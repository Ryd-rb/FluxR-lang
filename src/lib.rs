//! Portable scene programs with one compiler contract for text and typed JSON.
use std::fmt;
use std::sync::Arc;

#[cfg(all(feature = "host-transport", not(target_arch = "wasm32")))]
pub mod authenticated_tcp;
pub mod collaboration;
mod gate;
pub use gate::Gate;
pub mod hir;
#[cfg(all(feature = "native-runtime", not(target_arch = "wasm32")))]
pub mod native_runtime;
mod rules;
pub mod structured;
pub mod structured_series;
pub mod telemetry;
mod timeline_compact;
pub use rules::{PreparedRule, RuleProgram};
use telemetry::{Feature, Work, measured};

const VERSION: u16 = 1;
const MAX_SOURCE: usize = 256 * 1024;
const MAX_TRACKS: usize = 128;
const MAX_KEYS: usize = 4096;
const MAGIC: &[u8; 4] = b"LSP\0";

type Key = (u32, i32);
type TrackSource = (u16, u16, u8, Vec<Key>);
type Source = (u16, u32, Vec<TrackSource>);

#[derive(Debug, PartialEq, Eq)]
pub struct Error(String);

impl fmt::Display for Error {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(out)
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Curve {
    Hold = 0,
    Linear = 1,
    Smooth = 2,
}

impl TryFrom<u8> for Curve {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self, Error> {
        match value {
            0 => Ok(Self::Hold),
            1 => Ok(Self::Linear),
            2 => Ok(Self::Smooth),
            _ => Err(Error(format!("unknown curve {value}"))),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Track {
    address: (u16, u16),
    curve: Curve,
    keys: Arc<[Key]>,
}

impl Track {
    fn sample(&self, t: u32) -> i32 {
        sample_keys(&self.keys, self.curve, t)
    }

    fn translation_from(&self, base: &Self) -> Option<i64> {
        if self.curve != base.curve || self.keys.len() != base.keys.len() {
            return None;
        }
        let shift = i64::from(self.keys[0].1) - i64::from(base.keys[0].1);
        self.keys
            .iter()
            .zip(base.keys.iter())
            .all(|(&(t, value), &(base_t, base_value))| {
                t == base_t && i64::from(value) - i64::from(base_value) == shift
            })
            .then_some(shift)
    }
}

enum Samples<A, B> {
    Shared(A),
    Separate(B),
}

impl<A: Iterator<Item = i32>, B: Iterator<Item = i32>> Iterator for Samples<A, B> {
    type Item = i32;

    fn next(&mut self) -> Option<i32> {
        match self {
            Self::Shared(iter) => iter.next(),
            Self::Separate(iter) => iter.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Shared(iter) => iter.size_hint(),
            Self::Separate(iter) => iter.size_hint(),
        }
    }

    fn fold<T, F: FnMut(T, i32) -> T>(self, init: T, fold: F) -> T {
        match self {
            Self::Shared(iter) => iter.fold(init, fold),
            Self::Separate(iter) => iter.fold(init, fold),
        }
    }
}

impl<A: ExactSizeIterator<Item = i32>, B: ExactSizeIterator<Item = i32>> ExactSizeIterator
    for Samples<A, B>
{
}

/// A reusable sampler for one immutable program track.
#[derive(Clone, Debug)]
pub struct BoundTrack {
    duration: u32,
    curve: Curve,
    keys: Arc<[Key]>,
}

impl BoundTrack {
    /// Sample using the bound program's repeating duration and curve.
    pub fn sample(&self, time_ms: u32) -> i32 {
        let value = sample_keys(&self.keys, self.curve, time_ms % self.duration);
        telemetry::sampled(Feature::Sample, 1, true);
        value
    }
}

fn sample_keys(keys: &[Key], curve: Curve, t: u32) -> i32 {
    let ((start, from), (end, to)) = match keys {
        [only] => return only.1,
        [first, second, third] => {
            if t < second.0 {
                (*first, *second)
            } else if t < third.0 {
                (*second, *third)
            } else {
                return third.1;
            }
        }
        [first, second, third, fourth] => {
            if t < second.0 {
                (*first, *second)
            } else if t < third.0 {
                (*second, *third)
            } else if t < fourth.0 {
                (*third, *fourth)
            } else {
                return fourth.1;
            }
        }
        _ => {
            let right = keys.partition_point(|key| key.0 <= t);
            let from = keys[right - 1];
            let Some(&to) = keys.get(right) else {
                return from.1;
            };
            (from, to)
        }
    };
    interpolate(curve, from, to, t - start, end - start)
}

/// Fixed-point interpolation for a portable, replayable scene timeline.
fn interpolate(curve: Curve, from: i32, to: i32, step: u32, span: u32) -> i32 {
    if span == 0 || step >= span {
        return to;
    }
    let value = i64::from(from);
    let difference = i64::from(to) - value;
    match curve {
        Curve::Hold => from,
        Curve::Linear => (value + difference * i64::from(step) / i64::from(span)) as i32,
        Curve::Smooth => {
            const ONE: i64 = 1 << 16;
            let phase = i64::from(step) * ONE / i64::from(span);
            let eased = phase * phase * (3 * ONE - 2 * phase) / (ONE * ONE);
            (value + difference * eased / ONE) as i32
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    duration: u32,
    tracks: Vec<Track>,
    // When all tracks differ only by a value translation, sample the first
    // once and keep the starting values contiguous for batch consumers.
    batch_starts: Option<Arc<[i32]>>,
}

impl Program {
    pub fn duration_ms(&self) -> u32 {
        self.duration
    }

    pub fn compile_text(text: &str) -> Result<Self, Error> {
        measured(
            Feature::CompileText,
            text.len(),
            || Self::parse_text(text),
            Self::program_work,
        )
    }

    fn parse_text(text: &str) -> Result<Self, Error> {
        bound(text.len())?;
        let mut lines = text
            .lines()
            .enumerate()
            .filter(|(_, line)| !line.trim().is_empty());
        let (_, header) = lines
            .next()
            .ok_or_else(|| Error("expected version and duration".into()))?;
        let mut header = header.split_whitespace();
        let version = header.next().and_then(|word| word.strip_prefix('v'));
        let version = number(version, "version")?;
        let duration = number(header.next(), "duration")?;
        if header.next().is_some() {
            return Err(Error("unexpected header field".into()));
        }
        if version == 2 {
            return timeline_compact::parse(duration, lines);
        }
        let mut tracks = Vec::new();
        for (index, line) in lines {
            let track =
                text_track(line).map_err(|error| Error(format!("line {}: {error}", index + 1)))?;
            tracks.push(track);
        }
        Self::compile((version, duration, tracks))
    }

    pub fn compile_structured(text: &str) -> Result<Self, Error> {
        measured(
            Feature::CompileTyped,
            text.len(),
            || {
                bound(text.len())?;
                let input = serde_json::from_str(text).map_err(|error| Error(error.to_string()))?;
                Self::compile(input)
            },
            Self::program_work,
        )
    }

    fn program_work(&self) -> Work {
        Work {
            tracks: self.tracks.len() as u64,
            keys: self
                .tracks
                .iter()
                .map(|track| track.keys.len() as u64)
                .sum(),
            ..Work::default()
        }
    }

    fn compile((version, duration, source): Source) -> Result<Self, Error> {
        if version != VERSION {
            return Err(Error(format!("unsupported language version {version}")));
        }
        if !(1..=600_000).contains(&duration) || !(1..=MAX_TRACKS).contains(&source.len()) {
            return Err(Error("duration or track count outside limits".into()));
        }
        let mut tracks = Vec::with_capacity(source.len());
        let mut key_count = 0;
        for (object, field, curve, keys) in source {
            key_count += keys.len();
            if key_count > MAX_KEYS
                || keys.first().map(|key| key.0) != Some(0)
                || keys.last().is_some_and(|key| key.0 > duration)
                || keys.windows(2).any(|pair| pair[0].0 >= pair[1].0)
            {
                return Err(Error(format!(
                    "{object}.{field}: invalid key times or budget"
                )));
            }
            let curve = Curve::try_from(curve)?;
            tracks.push(Track {
                address: (object, field),
                curve,
                keys: Arc::from(keys),
            });
        }
        tracks.sort_unstable_by_key(|track| track.address);
        if tracks
            .windows(2)
            .any(|pair| pair[0].address == pair[1].address)
        {
            return Err(Error("duplicate track address".into()));
        }
        let batch_starts = if tracks.len() >= 2
            && tracks[1..]
                .iter()
                .all(|track| track.translation_from(&tracks[0]).is_some())
        {
            Some(tracks.iter().map(|track| track.keys[0].1).collect())
        } else {
            None
        };
        Ok(Self {
            duration,
            tracks,
            batch_starts,
        })
    }

    pub fn text(&self) -> String {
        use fmt::Write;
        let mut out = format!("v{VERSION} {}", self.duration);
        for track in &self.tracks {
            let (object, field) = track.address;
            write!(out, "\n{object} {field} {}", track.curve as u8).unwrap();
            for &(time, value) in track.keys.iter() {
                write!(out, " {time}:{value}").unwrap();
            }
        }
        out
    }

    pub fn structured(&self) -> String {
        let tracks: Vec<_> = self
            .tracks
            .iter()
            .map(|track| {
                (
                    track.address.0,
                    track.address.1,
                    track.curve as u8,
                    track.keys.as_ref(),
                )
            })
            .collect();
        serde_json::to_string(&(VERSION, self.duration, tracks)).unwrap()
    }

    pub fn addresses(&self) -> impl ExactSizeIterator<Item = (u16, u16)> + '_ {
        self.tracks.iter().map(|track| track.address)
    }

    pub fn keys(&self, address: (u16, u16)) -> Option<&[(u32, i32)]> {
        self.track(address).map(|track| track.keys.as_ref())
    }

    /// Resolve a track once and share its keys for repeated sampling.
    pub fn bind_track(&self, address: (u16, u16)) -> Option<BoundTrack> {
        self.track(address).map(|track| BoundTrack {
            duration: self.duration,
            curve: track.curve,
            keys: Arc::clone(&track.keys),
        })
    }

    pub fn sample(&self, address: (u16, u16), time_ms: u32) -> Option<i32> {
        let value = self.sample_track(address, time_ms);
        telemetry::sampled(
            Feature::Sample,
            usize::from(value.is_some()),
            value.is_some(),
        );
        value
    }

    fn sample_track(&self, address: (u16, u16), time_ms: u32) -> Option<i32> {
        self.track(address)
            .map(|track| track.sample(time_ms % self.duration))
    }

    pub fn sample_all(&self, time_ms: u32) -> impl ExactSizeIterator<Item = i32> + '_ {
        telemetry::sampled(Feature::SampleBatch, self.tracks.len(), true);
        let time = time_ms % self.duration;
        match &self.batch_starts {
            Some(starts) => {
                let delta = i64::from(self.tracks[0].sample(time)) - i64::from(starts[0]);
                Samples::Shared(
                    starts
                        .iter()
                        .map(move |&start| (i64::from(start) + delta) as i32),
                )
            }
            None => Samples::Separate(self.tracks.iter().map(move |track| track.sample(time))),
        }
    }

    pub fn replay(&self, assertions: &str) -> Result<u32, Error> {
        measured(
            Feature::Replay,
            assertions.len(),
            || self.check_replay(assertions),
            |count| Work {
                assertions: u64::from(*count),
                ..Work::default()
            },
        )
    }

    fn check_replay(&self, assertions: &str) -> Result<u32, Error> {
        bound(assertions.len())?;
        let rows: Vec<(u16, u16, u32, i32)> =
            serde_json::from_str(assertions).map_err(|error| Error(error.to_string()))?;
        if rows.is_empty() || rows.len() > MAX_KEYS {
            return Err(Error("scenario assertion count outside limits".into()));
        }
        for &(object, field, time, expected) in &rows {
            let actual = self.sample_track((object, field), time);
            if actual != Some(expected) {
                return Err(Error(format!(
                    "{object}.{field} at {time}ms: expected {expected}, got {actual:?}"
                )));
            }
        }
        Ok(rows.len() as u32)
    }

    fn track(&self, address: (u16, u16)) -> Option<&Track> {
        self.tracks
            .binary_search_by_key(&address, |track| track.address)
            .ok()
            .map(|index| &self.tracks[index])
    }

    pub fn encode(&self) -> Vec<u8> {
        let attempt = telemetry::Attempt::start(Feature::Encode, 0);
        let mut out = MAGIC.to_vec();
        out.extend(VERSION.to_le_bytes());
        out.extend(self.duration.to_le_bytes());
        out.extend((self.tracks.len() as u16).to_le_bytes());
        for track in &self.tracks {
            out.extend(track.address.0.to_le_bytes());
            out.extend(track.address.1.to_le_bytes());
            out.push(track.curve as u8);
            out.extend((track.keys.len() as u16).to_le_bytes());
            for &(time, value) in track.keys.iter() {
                out.extend(time.to_le_bytes());
                out.extend(value.to_le_bytes());
            }
        }
        if let Some(attempt) = attempt {
            attempt.finish_output(out.len());
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        measured(
            Feature::Decode,
            bytes.len(),
            || Self::read_compiled(bytes),
            Self::program_work,
        )
    }

    fn read_compiled(bytes: &[u8]) -> Result<Self, Error> {
        bound(bytes.len())?;
        let mut bytes = bytes;
        if &take::<4>(&mut bytes)? != MAGIC {
            return Err(Error("invalid compiled scene header".into()));
        }
        let version = u16::from_le_bytes(take(&mut bytes)?);
        if version != VERSION {
            return Err(Error(format!("unsupported compiled version {version}")));
        }
        let duration = u32::from_le_bytes(take(&mut bytes)?);
        let count = u16::from_le_bytes(take(&mut bytes)?) as usize;
        if count > MAX_TRACKS {
            return Err(Error("compiled track budget exceeded".into()));
        }
        let mut tracks = Vec::with_capacity(count);
        let mut key_count = 0;
        for _ in 0..count {
            let object = u16::from_le_bytes(take(&mut bytes)?);
            let field = u16::from_le_bytes(take(&mut bytes)?);
            let [curve] = take(&mut bytes)?;
            let count = u16::from_le_bytes(take(&mut bytes)?) as usize;
            key_count += count;
            if key_count > MAX_KEYS {
                return Err(Error("compiled key budget exceeded".into()));
            }
            let mut keys = Vec::with_capacity(count);
            for _ in 0..count {
                keys.push((
                    u32::from_le_bytes(take(&mut bytes)?),
                    i32::from_le_bytes(take(&mut bytes)?),
                ));
            }
            tracks.push((object, field, curve, keys));
        }
        if !bytes.is_empty() {
            return Err(Error("trailing bytes in compiled scene".into()));
        }
        Self::compile((version, duration, tracks))
    }
}

fn bound(length: usize) -> Result<(), Error> {
    if length > MAX_SOURCE {
        return Err(Error("program exceeds byte budget".into()));
    }
    Ok(())
}

fn take<const N: usize>(bytes: &mut &[u8]) -> Result<[u8; N], Error> {
    let (field, rest) = bytes
        .split_first_chunk::<N>()
        .ok_or_else(|| Error("truncated compiled scene".into()))?;
    *bytes = rest;
    Ok(*field)
}

fn text_track(line: &str) -> Result<TrackSource, Error> {
    let mut parts = line.split_whitespace();
    let object = number(parts.next(), "object")?;
    let field = number(parts.next(), "field")?;
    let curve = number(parts.next(), "curve")?;
    let mut keys = Vec::new();
    for pair in parts {
        keys.push(text_key(pair)?);
    }
    Ok((object, field, curve, keys))
}

fn text_key(pair: &str) -> Result<Key, Error> {
    let (time, value) = pair
        .split_once(':')
        .ok_or_else(|| Error("expected time:value".into()))?;
    Ok((number(Some(time), "time")?, number(Some(value), "value")?))
}

fn number<T: std::str::FromStr>(value: Option<&str>, name: &str) -> Result<T, Error> {
    value
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| Error(format!("invalid {name}")))
}

#[cfg(target_arch = "wasm32")]
mod browser;
