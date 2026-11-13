//! Matched checked kernels, boundary costs and immutable prepared calls.
//! Timing is observational: no universal native-Rust or .NET superiority claim.
use fluxr::structured::{ExecutionRoute, Invocation, Limits, Module, Value, Workspace};
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, hint::black_box, time::Instant};

#[path = "fixtures/jit_branch.rs"]
mod branch;
#[path = "fixtures/jit_calls.rs"]
mod calls;
#[path = "fixtures/jit_loop.rs"]
mod loops;
#[path = "fixtures/jit_scalar.rs"]
mod scalar;

struct Case {
    name: &'static str,
    source: &'static str,
    rust: &'static str,
    export: &'static str,
    arity: usize,
    kernel: fn([i64; 4]) -> Option<i64>,
    loop_bound: Option<usize>,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn observations() -> Json {
    let mut values = BTreeMap::new();
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        for line in status.lines() {
            if let Some((key, value)) = line.split_once(':')
                && matches!(key, "VmRSS" | "VmHWM" | "VmSize" | "Threads")
            {
                values.insert(key.to_owned(), value.trim().to_owned());
            }
        }
    }
    json!(values)
}

fn stats(samples: &[f64], unit: &str) -> Json {
    assert!(!samples.is_empty());
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let percentile = |tenths: usize| sorted[((sorted.len() * tenths).div_ceil(1000)).max(1) - 1];
    let all: Vec<_> = (0..=100).map(|p| percentile(p * 10)).collect();
    // Keep the released upper-middle p50 field even for an even number of
    // rounds. The separately named complete distribution uses nearest rank.
    json!({"samples_ns":samples,"p50_ns":sorted[sorted.len()/2],
        "p95_ns":sorted[(sorted.len()*95).div_ceil(100)-1],
        "minimum_ns":sorted[0],"maximum_ns":sorted[sorted.len()-1],
        "observation_unit":unit,"sample_count":samples.len(),
        "percentiles_p0_through_p100_ns":all,"p99_9_ns":percentile(999),
        "quantile_method":"nearest-rank; p0 is minimum",
        "expected_observations_above_p99_9":samples.len() as f64 / 1000.0})
}

fn value(value: Value) -> i64 {
    let Value::I64(value) = value else {
        panic!("expected signed result")
    };
    value
}

fn timed(count: usize, mut call: impl FnMut(usize) -> i64) -> (u64, f64) {
    let start = Instant::now();
    let mut checksum = 0u64;
    for n in 0..count {
        checksum = checksum.rotate_left(1) ^ black_box(call(black_box(n))) as u64;
    }
    (
        black_box(checksum),
        start.elapsed().as_nanos() as f64 / count as f64,
    )
}

// The boundary reference matches the timed success domain: exact arity/type,
// frame/cell/fuel admission and checked integer operations. Its conservative
// ceiling is not an independent replacement for low-budget VM error ordering.
fn rust_host(case: &Case, args: &[Value], limits: Limits, need: Limits) -> Option<Value> {
    if args.len() != case.arity
        || limits.instructions < need.instructions
        || limits.cells < need.cells
        || limits.frames < need.frames
    {
        return None;
    }
    let mut row = [0; 4];
    for (out, arg) in row.iter_mut().zip(args) {
        let Value::I64(n) = arg else {
            return None;
        };
        *out = *n;
    }
    (black_box(case.kernel))(row).map(Value::I64)
}

fn run_case(case: &Case, rounds: usize, count: usize) -> Result<Json, Box<dyn std::error::Error>> {
    let memory_before = observations();
    let mut seed = 0x62c3_04f1_e80a_51dbu64;
    let mut rows = Vec::with_capacity(2048);
    for _ in 0..2048 {
        let mut row = std::array::from_fn(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % 4093) as i64 - 2046
        });
        if case.loop_bound.is_some() {
            row[1] = (seed % 64) as i64;
        }
        rows.push(row);
    }
    let args: Vec<Vec<Value>> = rows
        .iter()
        .map(|row| row[..case.arity].iter().copied().map(Value::I64).collect())
        .collect();
    let expected: Vec<i64> = rows
        .iter()
        .map(|row| (case.kernel)(*row).unwrap())
        .collect();
    let limits = Limits::default();
    let module = Module::compile_compact(case.source)?;
    let encoded = module.encode();
    let decoded = Module::decode(&encoded)?;
    let prepare_start = Instant::now();
    let prepared = decoded.prepare_call(case.export)?;
    let prepare_ns = prepare_start.elapsed().as_nanos();
    let need = match case.loop_bound {
        Some(n) => decoded.native_loop_ceiling(case.export, case.arity, n),
        None => decoded.native_ceiling(case.export, case.arity),
    }
    .ok_or("expected a bounded reference admission")?;
    let invoke = |n: usize| Invocation {
        export: case.export,
        inputs: &[],
        arguments: &args[n % args.len()],
        limits,
    };
    let [mut portable_ws, mut named_ws, mut prepared_ws] =
        std::array::from_fn(|_| Workspace::default());
    let mut route_counts = BTreeMap::<String, usize>::new();
    for n in 0..args.len() {
        let (result, route) = prepared.run_with_route(&[], &args[n], limits, &mut prepared_ws);
        assert_eq!(value(result?), expected[n]);
        if cfg!(all(
            feature = "typed-jit",
            target_os = "linux",
            target_env = "gnu",
            target_arch = "x86_64"
        )) {
            assert_eq!(
                route,
                ExecutionRoute::TypedJit,
                "{} must execute machine code",
                case.name
            );
        }
        *route_counts.entry(format!("{route:?}")).or_default() += 1;
        assert_eq!(
            value(decoded.run_portable_with(invoke(n), &mut portable_ws)?),
            expected[n]
        );
        assert_eq!(
            value(decoded.run_with(invoke(n), &mut named_ws)?),
            expected[n]
        );
        assert_eq!(
            value(rust_host(case, &args[n], limits, need).ok_or("Rust boundary admission")?),
            expected[n]
        );
    }
    let expected_checksum = (0..count).fold(0u64, |h, n| {
        h.rotate_left(1) ^ expected[n % expected.len()] as u64
    });
    let mut samples: [Vec<f64>; 5] = std::array::from_fn(|_| Vec::with_capacity(rounds));
    let mut orders = Vec::with_capacity(rounds);
    for round in 0..rounds {
        let mut order = Vec::new();
        for offset in 0..5 {
            let route = (round + offset) % 5;
            order.push(route);
            let (checksum, ns) = match route {
                0 => timed(count, |n| {
                    (black_box(case.kernel))(black_box(rows[n % rows.len()])).unwrap()
                }),
                1 => timed(count, |n| {
                    value(
                        rust_host(
                            case,
                            black_box(&args[n % args.len()]),
                            black_box(limits),
                            need,
                        )
                        .unwrap(),
                    )
                }),
                2 => timed(count, |n| {
                    value(
                        decoded
                            .run_portable_with(invoke(n), &mut portable_ws)
                            .unwrap(),
                    )
                }),
                3 => timed(count, |n| {
                    value(decoded.run_with(invoke(n), &mut named_ws).unwrap())
                }),
                4 => timed(count, |n| {
                    value(
                        prepared
                            .run_with(
                                &[],
                                black_box(&args[n % args.len()]),
                                black_box(limits),
                                &mut prepared_ws,
                            )
                            .unwrap(),
                    )
                }),
                _ => unreachable!(),
            };
            assert_eq!(checksum, expected_checksum);
            samples[route].push(ns);
        }
        orders.push(order);
    }
    let mut compile = Vec::new();
    let mut cold_prepare = Vec::new();
    let mut first = Vec::new();
    let mut cached = Vec::new();
    let mut drop_ns = Vec::new();
    for _ in 0..11 {
        let start = Instant::now();
        let m = Module::compile_compact(black_box(case.source))?;
        compile.push(start.elapsed().as_nanos() as f64);
        let start = Instant::now();
        let p = m.prepare_call(case.export)?;
        cold_prepare.push(start.elapsed().as_nanos() as f64);
        let start = Instant::now();
        assert_eq!(value(p.run(&[], &args[0], limits)?), expected[0]);
        first.push(start.elapsed().as_nanos() as f64);
        let start = Instant::now();
        black_box(m.prepare_call(case.export)?);
        cached.push(start.elapsed().as_nanos() as f64);
        let start = Instant::now();
        drop(m);
        drop_ns.push(start.elapsed().as_nanos() as f64);
    }
    let names = [
        "rust_checked_kernel",
        "rust_checked_host",
        "portable",
        "named_native",
        "prepared_native",
    ];
    let timings: BTreeMap<_, _> = names
        .iter()
        .zip(&samples)
        .map(|(name, sample)| {
            (
                *name,
                stats(
                    sample,
                    "mean ns per call averaged over a batch; result disposal and checksum included",
                ),
            )
        })
        .collect();
    Ok(
        json!({"name":case.name,"source_sha256":digest(case.source.as_bytes()),"rust_reference_sha256":digest(case.rust.as_bytes()),
        "module_bytes":encoded.len(),"module_sha256":digest(&encoded),"rounds":rounds,"calls_per_round":count,
        "validation_input_rows":args.len(),"input_generation":"xorshift64 seed 0x62c304f1e80a51db; fixed test distribution, not held-out coding tasks",
        "route_counts":route_counts,"checksum":format!("{expected_checksum:016x}"),"order_indices":orders,
        "limits":limits,"rust_host_sufficient_admission":need,"timings":timings,"initial_prepare_ns":prepare_ns,
        "lifecycle":{
            "source_compile":stats(&compile,"ns per compact source parse/check/compile"),
            "cold_prepare":stats(&cold_prepare,"ns per fresh-module preparation; includes code publication when admitted"),
            "first_call":stats(&first,"ns per first invocation with fresh workspace; result assertion included"),
            "cached_prepare":stats(&cached,"ns per cached preparation"),
            "module_drop":stats(&drop_ns,"ns per prepared module destruction")},
        "memory_before":memory_before,"memory_after":observations()}),
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    fluxr::telemetry::configure("off")?;
    let cases = [
        Case {
            name: "checked_scalar",
            source: include_str!("fixtures/jit_scalar.fluxr"),
            rust: include_str!("fixtures/jit_scalar.rs"),
            export: "calc",
            arity: 2,
            kernel: |x| scalar::run(x[0], x[1]),
            loop_bound: None,
        },
        Case {
            name: "branch_tree",
            source: include_str!("fixtures/host_decision.flux"),
            rust: include_str!("fixtures/jit_branch.rs"),
            export: "decide",
            arity: 4,
            kernel: |x| branch::run(x[0], x[1], x[2], x[3]),
            loop_bound: None,
        },
        Case {
            name: "inlined_calls",
            source: include_str!("fixtures/jit_calls.fluxr"),
            rust: include_str!("fixtures/jit_calls.rs"),
            export: "combine",
            arity: 2,
            kernel: |x| calls::run(x[0], x[1]),
            loop_bound: None,
        },
        Case {
            name: "bounded_loop",
            source: include_str!("fixtures/jit_loop.fluxr"),
            rust: include_str!("fixtures/jit_loop.rs"),
            export: "sum",
            arity: 2,
            kernel: |x| loops::run(x[0], x[1]),
            loop_bound: Some(64),
        },
    ];
    let rounds = std::env::var("FLUXR_JIT_BENCH_ROUNDS")
        .ok()
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(31usize);
    let count = std::env::var("FLUXR_JIT_BENCH_CALLS")
        .ok()
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(8192usize);
    if !(5..=101).contains(&rounds) || !(256..=1_000_000).contains(&count) {
        return Err("benchmark bounds exceeded".into());
    }
    let results = cases
        .iter()
        .map(|case| run_case(case, rounds, count))
        .collect::<Result<Vec<_>, _>>()?;
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"schema":"fluxr-jit-performance-v12","telemetry":"off",
        "distribution_contract":"nearest-rank-all-percentiles-v1",
        "profile":"strict checked signed i64; no relaxed numerical mode","typed_jit_feature":cfg!(feature="typed-jit"),
        "os":std::env::consts::OS,"arch":std::env::consts::ARCH,"round_order":"rotating route start, all raw observations retained",
        "rust_kernel_call_boundary":"checked optimized Rust through function pointer, same result folding; raw kernels exclude host admission",
        "rust_host_scope":"matching timed success inputs under sufficient checked ceilings; exact low-budget errors independently qualified against the portable oracle",
        "limitations":["four local workloads, not universal performance","no .NET runtime trial","process high-water memory includes host/source/dependency runtime; no physical GPU or real-time claim","static source-token counts are separate from full model coding episodes"],"cases":results})
        )?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_distribution_keeps_endpoints_and_legacy_even_median() {
        let samples: Vec<_> = (1..=1000).map(f64::from).collect();
        let measured = stats(&samples, "ns per synthetic operation");
        assert_eq!(measured["sample_count"], 1000);
        assert_eq!(measured["observation_unit"], "ns per synthetic operation");
        assert_eq!(measured["p50_ns"], 501.0);
        assert_eq!(measured["percentiles_p0_through_p100_ns"][0], 1.0);
        assert_eq!(measured["percentiles_p0_through_p100_ns"][50], 500.0);
        assert_eq!(measured["percentiles_p0_through_p100_ns"][100], 1000.0);
        assert_eq!(measured["p99_9_ns"], 999.0);
    }

    #[test]
    fn complete_distribution_retains_rare_slow_observations() {
        let mut samples = vec![10.0; 1000];
        samples[0] = 100_000.0;
        samples[500] = 100_000.0;
        let measured = stats(&samples, "mean ns per synthetic batch call");
        assert_eq!(measured["p50_ns"], 10.0);
        assert_eq!(measured["p99_9_ns"], 100_000.0);
        assert_eq!(measured["percentiles_p0_through_p100_ns"][100], 100_000.0);
        assert_eq!(measured["samples_ns"][0], 100_000.0);
        assert_eq!(measured["samples_ns"][500], 100_000.0);
    }
}
