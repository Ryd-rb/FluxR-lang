//! Opt-in, process-local instrumentation. Runtime counters stay local.

use serde_json::{Value, json};
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering::Relaxed};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

#[derive(Clone, Copy)]
pub enum Feature {
    CompileText,
    CompileTyped,
    Encode,
    Decode,
    Replay,
    Sample,
    SampleBatch,
    Reload,
    RuleEvaluate,
}

impl Feature {
    const ALL: [Self; 9] = [
        Self::CompileText,
        Self::CompileTyped,
        Self::Encode,
        Self::Decode,
        Self::Replay,
        Self::Sample,
        Self::SampleBatch,
        Self::Reload,
        Self::RuleEvaluate,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::CompileText => "compile-text",
            Self::CompileTyped => "compile-typed",
            Self::Encode => "encode",
            Self::Decode => "decode",
            Self::Replay => "replay",
            Self::Sample => "sample",
            Self::SampleBatch => "sample-batch",
            Self::Reload => "reload",
            Self::RuleEvaluate => "rule-evaluate",
        }
    }

    const fn bit(self) -> u16 {
        1 << self as u16
    }

    pub fn enabled(self) -> bool {
        ENABLED.load(Relaxed) & self.bit() != 0
    }

    fn counters(self) -> &'static Counters {
        &COUNTERS[self as usize]
    }
}

const LATENCY_US: [u64; 8] = [1, 5, 20, 100, 500, 2_000, 10_000, 50_000];
const BUCKETS: usize = LATENCY_US.len() + 1;
static ENABLED: AtomicU16 = AtomicU16::new(0);
static COUNTERS: [Counters; Feature::ALL.len()] = [const { Counters::new() }; Feature::ALL.len()];

struct Counters {
    accepted: AtomicU64,
    rejected: AtomicU64,
    input_bytes: AtomicU64,
    output_bytes: AtomicU64,
    tracks: AtomicU64,
    keys: AtomicU64,
    assertions: AtomicU64,
    latency: [AtomicU64; BUCKETS],
}

impl Counters {
    const fn new() -> Self {
        Self {
            accepted: AtomicU64::new(0),
            rejected: AtomicU64::new(0),
            input_bytes: AtomicU64::new(0),
            output_bytes: AtomicU64::new(0),
            tracks: AtomicU64::new(0),
            keys: AtomicU64::new(0),
            assertions: AtomicU64::new(0),
            latency: [const { AtomicU64::new(0) }; BUCKETS],
        }
    }
}

/// Accepts `off`, `all`, or a comma-separated list of feature names.
/// An invalid configuration leaves the previous selection in effect.
pub fn configure(selection: &str) -> Result<(), String> {
    let mask = match selection.trim() {
        "off" => 0,
        "all" => (1 << Feature::ALL.len()) - 1,
        other => {
            let mut mask = 0;
            for name in other.split(',').map(str::trim) {
                let feature = Feature::ALL
                    .into_iter()
                    .find(|feature| feature.name() == name)
                    .ok_or_else(|| format!("unknown FluxR telemetry feature {name:?}"))?;
                mask |= feature.bit();
            }
            mask
        }
    };
    ENABLED.store(mask, Relaxed);
    Ok(())
}

#[derive(Default)]
pub(crate) struct Work {
    pub(crate) tracks: u64,
    pub(crate) keys: u64,
    pub(crate) assertions: u64,
}

impl Work {
    fn record(self, counters: &Counters) {
        for (counter, amount) in [
            (&counters.tracks, self.tracks),
            (&counters.keys, self.keys),
            (&counters.assertions, self.assertions),
        ] {
            if amount != 0 {
                counter.fetch_add(amount, Relaxed);
            }
        }
    }
}

pub struct Attempt {
    feature: Feature,
    input_bytes: usize,
    #[cfg(not(target_arch = "wasm32"))]
    started: Instant,
}

impl Attempt {
    pub fn start(feature: Feature, input_bytes: usize) -> Option<Self> {
        feature.enabled().then(|| Self {
            feature,
            input_bytes,
            #[cfg(not(target_arch = "wasm32"))]
            started: Instant::now(),
        })
    }

    pub fn finish(self, accepted: bool) {
        let counters = self.feature.counters();
        if self.input_bytes != 0 {
            counters
                .input_bytes
                .fetch_add(self.input_bytes as u64, Relaxed);
        }
        let outcome = if accepted {
            &counters.accepted
        } else {
            &counters.rejected
        };
        outcome.fetch_add(1, Relaxed);
        #[cfg(not(target_arch = "wasm32"))]
        observe_latency(
            counters,
            self.started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64,
        );
    }

    pub(crate) fn finish_output(self, output_bytes: usize) {
        self.feature
            .counters()
            .output_bytes
            .fetch_add(output_bytes as u64, Relaxed);
        self.finish(true);
    }
}

fn observe_latency(counters: &Counters, elapsed_us: u64) {
    let bucket = LATENCY_US.partition_point(|limit| *limit < elapsed_us);
    counters.latency[bucket].fetch_add(1, Relaxed);
}

/// Record one host reload. Browsers can supply a monotonic time observation.
pub fn reload(accepted: bool, bytes: usize, elapsed_us: u64) {
    if Feature::Reload.enabled() {
        let counters = Feature::Reload.counters();
        counters.input_bytes.fetch_add(bytes as u64, Relaxed);
        let outcome = if accepted {
            &counters.accepted
        } else {
            &counters.rejected
        };
        outcome.fetch_add(1, Relaxed);
        observe_latency(counters, elapsed_us);
    }
}

pub(crate) fn measured<T, E>(
    feature: Feature,
    input_bytes: usize,
    run: impl FnOnce() -> Result<T, E>,
    work: impl FnOnce(&T) -> Work,
) -> Result<T, E> {
    let attempt = Attempt::start(feature, input_bytes);
    let outcome = run();
    if let Some(attempt) = attempt {
        if let Ok(value) = &outcome {
            work(value).record(feature.counters());
        }
        attempt.finish(outcome.is_ok());
    }
    outcome
}

pub(crate) fn sampled(feature: Feature, hits: usize, accepted: bool) {
    if feature.enabled() {
        let counters = feature.counters();
        if hits != 0 {
            counters.tracks.fetch_add(hits as u64, Relaxed);
        }
        let outcome = if accepted {
            &counters.accepted
        } else {
            &counters.rejected
        };
        outcome.fetch_add(1, Relaxed);
    }
}

/// Count an interpreted or compiled rule without sampling a clock.
#[inline(always)]
pub fn record_rule_evaluation(accepted: bool) {
    if ENABLED.load(Relaxed) & Feature::RuleEvaluate.bit() != 0 {
        record_enabled_rule(accepted);
    }
}

#[cold]
fn record_enabled_rule(accepted: bool) {
    let counters = Feature::RuleEvaluate.counters();
    let outcome = if accepted {
        &counters.accepted
    } else {
        &counters.rejected
    };
    outcome.fetch_add(1, Relaxed);
}

/// Report exact evaluation counts once after a host batch.
pub fn record_rule_evaluations(accepted: u64, rejected: u64) {
    if (accepted != 0 || rejected != 0) && Feature::RuleEvaluate.enabled() {
        let counters = Feature::RuleEvaluate.counters();
        counters.accepted.fetch_add(accepted, Relaxed);
        counters.rejected.fetch_add(rejected, Relaxed);
    }
}

/// Relaxed aggregate counters. Observational timings are separate from counts.
/// A concurrent snapshot may observe different operations at different instants.
pub fn snapshot() -> Value {
    let mask = ENABLED.load(Relaxed);
    let operations = Feature::ALL.map(|feature| {
        let counters = feature.counters();
        json!({
            "feature": feature.name(), "enabled": mask & feature.bit() != 0,
            "accepted": counters.accepted.load(Relaxed), "rejected": counters.rejected.load(Relaxed),
            "input_bytes": counters.input_bytes.load(Relaxed), "output_bytes": counters.output_bytes.load(Relaxed),
            "tracks": counters.tracks.load(Relaxed), "keys": counters.keys.load(Relaxed),
            "assertions": counters.assertions.load(Relaxed),
            "observed_latency": counters.latency.each_ref().map(|count| count.load(Relaxed)),
        })
    });
    json!({"schema": 1, "tool": "fluxr", "latency_upper_us": LATENCY_US, "operations": operations})
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::{configure, snapshot};
    use std::{
        env, fs,
        io::{self, Read},
        path::{Path, PathBuf},
        thread,
        time::Duration,
    };

    pub fn configure_from_env() -> Result<(), String> {
        match env::var("FLUXR_TELEMETRY") {
            Ok(selection) => configure(&selection),
            Err(env::VarError::NotPresent) => Ok(()),
            Err(env::VarError::NotUnicode(_)) => Err("FLUXR_TELEMETRY must be Unicode".into()),
        }
    }

    /// Optional process-lifetime worker. Only this worker accesses snapshot and control files.
    pub fn start_snapshot_worker() -> io::Result<()> {
        let Some(target) = env::var_os("FLUXR_TELEMETRY_SNAPSHOT").map(PathBuf::from) else {
            return Ok(());
        };
        let control = env::var_os("FLUXR_TELEMETRY_CONTROL").map(PathBuf::from);
        thread::Builder::new()
            .name("fluxr-telemetry".into())
            .spawn(move || {
                let temporary = target.with_extension(format!("{}.tmp", std::process::id()));
                let mut last = Vec::new();
                loop {
                    if let Some(path) = &control
                        && let Ok(selection) = read_selection(path)
                    {
                        let _ = configure(&selection);
                    }
                    if let Ok(next) = serde_json::to_vec(&snapshot())
                        && last != next
                        && fs::write(&temporary, &next)
                            .and_then(|()| fs::rename(&temporary, &target))
                            .is_ok()
                    {
                        last = next;
                    }
                    thread::sleep(Duration::from_millis(250));
                }
            })?;
        Ok(())
    }

    fn read_selection(path: &Path) -> io::Result<String> {
        let mut text = String::new();
        fs::File::open(path)?.take(257).read_to_string(&mut text)?;
        if text.len() > 256 {
            return Err(io::ErrorKind::InvalidData.into());
        }
        Ok(text)
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::{configure_from_env, start_snapshot_worker};
