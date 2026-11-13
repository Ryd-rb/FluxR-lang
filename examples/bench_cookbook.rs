//! Matched cookbook measurements, with exact imported Rust kernel controls.
//! Every distribution is empirical; the finite corpus does not qualify universal tails.
#![allow(unexpected_cfgs)]

#[path = "cookbook/support.rs"]
mod support;

#[cfg(not(fluxr_public_baseline))]
use fluxr::structured::ExecutionRoute;
use fluxr::structured::{Limits, Module, Workspace};
use serde_json::{Value as Json, json};
use std::{collections::BTreeMap, hint::black_box, time::Instant};
use support::{Case, KernelOutcome, Outcome, digest};

const WARMUP: usize = 128;
const LIFECYCLE: usize = 7;

struct Settings {
    samples: usize,
    rounds: usize,
    calls: usize,
    common_routes: bool,
}

fn setting(
    name: &str,
    default: usize,
    minimum: usize,
    maximum: usize,
) -> Result<usize, Box<dyn std::error::Error>> {
    let value = std::env::var(name)
        .ok()
        .map(|value| value.parse::<usize>())
        .transpose()?
        .unwrap_or(default);
    if !(minimum..=maximum).contains(&value) {
        return Err(format!("{name} must be in {minimum}..={maximum}").into());
    }
    Ok(value)
}

fn normalize(value: Result<fluxr::structured::Value, fluxr::Error>) -> Outcome {
    value.map_err(|error| error.to_string())
}

fn same(case: &Case, index: usize, actual: Outcome) -> Result<(), Box<dyn std::error::Error>> {
    let expected = (case.oracle)(&case.rows[index]);
    if actual != expected {
        return Err(format!(
            "{} row {index}: expected {expected:?}, got {actual:?}",
            case.name
        )
        .into());
    }
    Ok(())
}

fn statistics(samples: &[f64], unit: &str) -> Json {
    assert!(!samples.is_empty());
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let percentile = |tenths: usize| sorted[((sorted.len() * tenths).div_ceil(1000)).max(1) - 1];
    json!({"observation_unit":unit,"sample_count":samples.len(),"samples_ns":samples,
        "percentiles_p0_through_p100_ns":(0..=100).map(|p|percentile(p*10)).collect::<Vec<_>>(),
        "p99_9_ns":percentile(999),"quantile_method":"nearest-rank; p0 is minimum",
        "expected_observations_above_p99_9":samples.len() as f64/1000.0})
}

#[cfg(not(fluxr_public_baseline))]
fn route_name(route: ExecutionRoute) -> &'static str {
    match route {
        ExecutionRoute::Portable => "Portable",
        ExecutionRoute::AheadOfTime => "AheadOfTime",
        ExecutionRoute::NativeScalar => "NativeScalar",
        ExecutionRoute::TypedJit => "TypedJit",
        ExecutionRoute::PrecompiledScan => "PrecompiledScan",
    }
}

enum TimedResult {
    Kernel(KernelOutcome),
    Value(Outcome),
}
impl TimedResult {
    fn outcome(self) -> Outcome {
        match self {
            Self::Kernel(value) => support::kernel_outcome(value),
            Self::Value(value) => value,
        }
    }
}
type Lane<'a> = (
    &'static str,
    Box<dyn FnMut(usize) -> (TimedResult, &'static str) + 'a>,
);

fn batch_row(round: usize, index: usize, calls_per_batch: usize, rows: usize) -> usize {
    (round * calls_per_batch + index) % rows
}

fn measure(
    case: &Case,
    settings: &Settings,
    mut lanes: Vec<Lane<'_>>,
) -> Result<Json, Box<dyn std::error::Error>> {
    let count = lanes.len();
    let mut individual: Vec<Vec<f64>> = (0..count)
        .map(|_| Vec::with_capacity(settings.samples))
        .collect();
    let mut batch: Vec<Vec<f64>> = (0..count)
        .map(|_| Vec::with_capacity(settings.rounds))
        .collect();
    let mut routes: Vec<BTreeMap<&str, usize>> = (0..count).map(|_| BTreeMap::new()).collect();
    for (_, call) in &mut lanes {
        for index in 0..WARMUP {
            let row = index % case.rows.len();
            same(case, row, call(row).0.outcome())?;
        }
    }
    for sample in 0..settings.samples {
        let row = sample % case.rows.len();
        for offset in 0..count {
            let lane = (sample + offset) % count;
            let start = Instant::now();
            let (actual, route) = black_box(lanes[lane].1(black_box(row)));
            individual[lane].push(start.elapsed().as_nanos() as f64);
            *routes[lane].entry(route).or_default() += 1;
            // Value conversion, exact checking and destruction are outside
            // individual timing, including for typed native Rust results.
            same(case, row, actual.outcome())?;
        }
    }
    for round in 0..settings.rounds {
        for offset in 0..count {
            let lane = (round + offset) % count;
            let start = Instant::now();
            for index in 0..settings.calls {
                // Batches include native result destruction. Rust kernel
                // outputs are not converted to FluxR Values in this loop.
                let row = batch_row(round, index, settings.calls, case.rows.len());
                let _ = black_box(lanes[lane].1(black_box(row)));
            }
            batch[lane].push(start.elapsed().as_nanos() as f64 / settings.calls as f64);
        }
    }
    let mut result = serde_json::Map::new();
    for (index, (name, _)) in lanes.iter().enumerate() {
        result.insert((*name).into(),json!({
            "individual_call":statistics(&individual[index],"ns per invocation; timer and harness dispatch included; post-call checking/conversion/destruction excluded"),
            "batch_mean":statistics(&batch[index],"ns per call averaged over a batch; result destruction included"),
            "timed_individual_route_counts":routes[index]}));
    }
    Ok(Json::Object(result))
}

fn lifecycle(case: &Case) -> Result<Json, Box<dyn std::error::Error>> {
    let mut compile = vec![];
    let mut decode = vec![];
    let mut prepare = vec![];
    let mut first = vec![];
    let row = &case.rows[0];
    for _ in 0..LIFECYCLE {
        let start = Instant::now();
        let module = case.compile()?;
        compile.push(start.elapsed().as_nanos() as f64);
        let bytes = module.encode();
        let start = Instant::now();
        let decoded = Module::decode(black_box(&bytes))?;
        decode.push(start.elapsed().as_nanos() as f64);
        if decoded.encode() != bytes {
            return Err("decoded artifact identity changed".into());
        }
        let start = Instant::now();
        #[cfg(not(fluxr_public_baseline))]
        let prepared = module.prepare_call("run")?;
        #[cfg(fluxr_public_baseline)]
        black_box(module.prepare_native("run"));
        prepare.push(start.elapsed().as_nanos() as f64);
        let start = Instant::now();
        #[cfg(not(fluxr_public_baseline))]
        let value = prepared.run(&row.inputs, &row.arguments, Limits::default());
        #[cfg(fluxr_public_baseline)]
        let value = module.run(row.invocation(Limits::default()));
        first.push(start.elapsed().as_nanos() as f64);
        same(case, 0, normalize(value))?;
    }
    Ok(
        json!({"source_compile":statistics(&compile,"ns per source parse/check/compile; source format identified separately"),
        "artifact_decode":statistics(&decode,"ns per artifact decode/check"),
        "cold_prepare":statistics(&prepare,"ns per cold preparation; code publication included when admitted"),
        "first_call":statistics(&first,"ns per first call with fresh workspace; excludes preparation")}),
    )
}

fn run_case(case: &Case, settings: &Settings) -> Result<Json, Box<dyn std::error::Error>> {
    let module = case.compile()?;
    let compact = Module::compile_compact(case.compact)?;
    if module.encode() != compact.encode() {
        return Err(format!("{} current/compact artifacts differ", case.name).into());
    }
    let artifact = module.encode();
    let module = Module::decode(&artifact)?;
    let identity = case.identity();
    let mut kernels: Vec<_> = case.rows.iter().map(case.kernel).collect();
    let mut portable_workspace = Workspace::default();
    let mut named_workspace = Workspace::default();
    #[cfg(not(fluxr_public_baseline))]
    let mut prepared_workspace = Workspace::default();
    #[cfg(not(fluxr_public_baseline))]
    let prepared = module.prepare_call("run")?;
    #[cfg(fluxr_public_baseline)]
    black_box(module.prepare_native("run"));
    let mut validation_routes = BTreeMap::<&str, usize>::new();
    let mut expected_successes = 0;
    for (index, row) in case.rows.iter().enumerate() {
        expected_successes += usize::from((case.oracle)(row).is_ok());
        same(case, index, support::kernel_outcome(kernels[index]()))?;
        same(
            case,
            index,
            normalize(
                module
                    .run_portable_with(row.invocation(Limits::default()), &mut portable_workspace),
            ),
        )?;
        same(
            case,
            index,
            normalize(module.run_with(row.invocation(Limits::default()), &mut named_workspace)),
        )?;
        #[cfg(not(fluxr_public_baseline))]
        {
            let (value, route) = prepared.run_with_route(
                &row.inputs,
                &row.arguments,
                Limits::default(),
                &mut prepared_workspace,
            );
            same(case, index, normalize(value))?;
            *validation_routes.entry(route_name(route)).or_default() += 1;
        }
        #[cfg(fluxr_public_baseline)]
        {
            *validation_routes.entry("Unattributed").or_default() += 1;
        }
    }
    let mut lanes: Vec<Lane<'_>> = vec![
        (
            "rust_kernel",
            Box::new(|index| (TimedResult::Kernel(kernels[index]()), "RustKernel")),
        ),
        (
            "rust_value_adapter",
            Box::new(|index| {
                (
                    TimedResult::Value((case.oracle)(black_box(&case.rows[index]))),
                    "RustValueAdapter",
                )
            }),
        ),
        (
            "portable",
            Box::new(|index| {
                (
                    TimedResult::Value(normalize(module.run_portable_with(
                        case.rows[index].invocation(Limits::default()),
                        &mut portable_workspace,
                    ))),
                    "Portable",
                )
            }),
        ),
        (
            "named",
            Box::new(|index| {
                (
                    TimedResult::Value(normalize(module.run_with(
                        case.rows[index].invocation(Limits::default()),
                        &mut named_workspace,
                    ))),
                    "Unattributed",
                )
            }),
        ),
    ];
    #[cfg(not(fluxr_public_baseline))]
    if !settings.common_routes {
        lanes.push((
            "prepared",
            Box::new(|index| {
                let row = &case.rows[index];
                let (value, route) = prepared.run_with_route(
                    &row.inputs,
                    &row.arguments,
                    Limits::default(),
                    &mut prepared_workspace,
                );
                (TimedResult::Value(normalize(value)), route_name(route))
            }),
        ));
    }
    let timings = measure(case, settings, lanes)?;
    if identity != case.identity() {
        return Err(format!("{} host input mutated", case.name).into());
    }
    Ok(
        json!({"status":"passed","name":case.name,"topics":case.topics,
        "source_sha256":digest(case.source.as_bytes()),"rust_sha256":digest(case.rust_source.as_bytes()),
        "compact_sha256":digest(case.compact.as_bytes()),"artifact_sha256":digest(&artifact),"artifact_bytes":artifact.len(),
        "rows_sha256":digest(&serde_json::to_vec(&identity)?),"row_contract":identity,"validation_rows":case.rows.len(),
        "expected_successes":expected_successes,"expected_traps":case.rows.len()-expected_successes,
        "validation_route_counts":validation_routes,"timings":timings,"lifecycle":lifecycle(case)?}),
    )
}

fn calibration(settings: &Settings) -> Json {
    let mut timer = vec![];
    let mut dispatch = vec![];
    let mut loop_cost = vec![];
    let mut identity: Box<dyn FnMut(usize) -> usize> = Box::new(black_box);
    for index in 0..settings.samples {
        let start = Instant::now();
        black_box(index);
        timer.push(start.elapsed().as_nanos() as f64);
        let start = Instant::now();
        black_box(identity(black_box(index)));
        dispatch.push(start.elapsed().as_nanos() as f64);
    }
    for _ in 0..settings.rounds {
        let start = Instant::now();
        for index in 0..settings.calls {
            black_box(identity(black_box(index % 64)));
        }
        loop_cost.push(start.elapsed().as_nanos() as f64 / settings.calls as f64);
    }
    json!({"empty_timer":statistics(&timer,"ns per empty timing observation; not subtracted"),
        "dispatch":statistics(&dispatch,"ns per dynamic identity closure; not subtracted"),
        "batch_loop":statistics(&loop_cost,"ns per empty loop/selection/dispatch; not subtracted")})
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().nth(1).as_deref() == Some("--startup") {
        println!("fluxr-cookbook-ready");
        return Ok(());
    }
    fluxr::telemetry::configure("off")?;
    let settings = Settings {
        samples: setting("FLUXR_COOKBOOK_SAMPLES", 1024, 64, 16384)?,
        rounds: setting("FLUXR_COOKBOOK_ROUNDS", 11, 3, 101)?,
        calls: setting("FLUXR_COOKBOOK_BATCH_CALLS", 64, 8, 4096)?,
        common_routes: match std::env::var("FLUXR_COOKBOOK_COMMON_ROUTES").as_deref() {
            Err(_) => cfg!(fluxr_public_baseline),
            Ok("1") => true,
            Ok("0") if !cfg!(fluxr_public_baseline) => false,
            _ => {
                return Err(
                    "FLUXR_COOKBOOK_COMMON_ROUTES must be 0 or 1; public baseline requires 1"
                        .into(),
                );
            }
        },
    };
    let start = Instant::now();
    let calibration = calibration(&settings);
    let mut failures = 0;
    let cases = support::cases()
        .iter()
        .map(|case| match run_case(case, &settings) {
            Ok(result) => result,
            Err(error) => {
                failures += 1;
                json!({"status":"failed","name":case.name,"error":error.to_string()})
            }
        })
        .collect::<Vec<_>>();
    println!(
        "{}",
        serde_json::to_string(&json!({"schema":"fluxr-cookbook-performance-v1",
        "catalog":"fluxr-cookbook-v1","probe_sha256":digest(include_bytes!("bench_cookbook.rs")),
        "support_sha256":digest(include_bytes!("cookbook/support.rs")),"catalog_sha256":digest(include_bytes!("cookbook/catalog.json")),
        "os":std::env::consts::OS,"arch":std::env::consts::ARCH,"typed_jit_feature":cfg!(feature="typed-jit"),
        "target_env":if cfg!(target_env="gnu"){"gnu"}else if cfg!(target_env="msvc"){"msvc"}else{"other"},
        "debug_assertions":cfg!(debug_assertions),"public_baseline_adapter":cfg!(fluxr_public_baseline),"telemetry":"off",
        "limits":Limits::default(),
        "compiled_source_format":if cfg!(fluxr_public_baseline){"compact-v2"}else{"fluxr-1"},
        "rust_kernel_scope":"Exact imported Rust reference, preconverted typed input, opaque inputs, dynamic closure and typed result enum. Value conversion and logical metering excluded.",
        "rust_value_adapter_scope":"Same Rust reference with per-call Value input/output adapters; logical metering excluded.",
        "settings":{"individual_samples":settings.samples,"batch_rounds":settings.rounds,"calls_per_batch":settings.calls,"warmup_calls_per_lane":WARMUP,"lifecycle_samples":LIFECYCLE,
            "common_routes":settings.common_routes,"timed_lanes":if settings.common_routes{vec!["rust_kernel","rust_value_adapter","portable","named"]}else{vec!["rust_kernel","rust_value_adapter","portable","named","prepared"]}},
        "route_order":"rotate first lane each individual sample and batch round","row_order":"individual: sample modulo rows; batches: (round * calls_per_batch + call) modulo rows; identical for every lane",
        "elapsed_ns":start.elapsed().as_nanos(),"calibration":calibration,"failed_cases":failures,"cases":cases,
        "limitations":["finite empirical distributions, not population-tail guarantees","Rust kernel and metered VM entry have different boundary costs, reported explicitly","source compilation uses current source for candidate and compact compatibility source for public baseline","process startup, native services, runtime injection, platform qualification and compiler build costs are separate gates"]}))?
    );
    if failures > 0 {
        return Err(format!("{failures} cookbook cases failed; see emitted outcomes").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::batch_row;
    use std::collections::BTreeSet;

    #[test]
    fn batch_rounds_reach_rows_beyond_the_first_batch_and_wrap() {
        // These are the largest current corpora: the old reset-to-zero loop
        // covered only one Boolean flag combination and one power base.
        for rows in [300, 455] {
            let sequence: Vec<_> = (0..11)
                .flat_map(|round| (0..64).map(move |call| batch_row(round, call, 64, rows)))
                .collect();
            assert_eq!(sequence[64], 64);
            assert_eq!(sequence[rows], 0);
            assert_eq!(sequence.into_iter().collect::<BTreeSet<_>>().len(), rows);
        }
        assert_eq!(batch_row(100, 4095, 4096, 1), 0);
    }
}
