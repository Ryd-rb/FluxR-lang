//! Dedicated unsigned success-domain probe; keep frozen fragment catalogs intact.
//! Compare identical checked u64 inputs/results in one release binary. Pure Rust
//! kernel time excludes the separately measured Value/type/resource boundary.
//! Prepared-JIT time includes that boundary and a route check on every call.
//! Raw observations establish neither a speed claim nor broader R10 acceptance.
use fluxr::structured::{ExecutionRoute, Invocation, Limits, Module, Value, Workspace};
use serde_json::{Value as Json, json};
use std::{error::Error, hint::black_box, time::Instant};

// Two operand loads, binary operation, return; two argument cells, two local
// slots plus frame, and one result cell. Independent of native Plan analysis.
const LIMITS: Limits = Limits {
    instructions: 4,
    cells: 6,
    frames: 1,
};
struct Case {
    name: &'static str,
    operator: &'static str,
    boolean: bool,
    kernel: fn(u64, u64) -> Option<u64>,
}
const CASES: [Case; 6] = [
    Case {
        name: "add",
        operator: "+",
        boolean: false,
        kernel: u64::checked_add,
    },
    Case {
        name: "sub",
        operator: "-",
        boolean: false,
        kernel: u64::checked_sub,
    },
    Case {
        name: "mul",
        operator: "*",
        boolean: false,
        kernel: u64::checked_mul,
    },
    Case {
        name: "eq",
        operator: "==",
        boolean: true,
        kernel: |x, y| Some(u64::from(x == y)),
    },
    Case {
        name: "lt",
        operator: "<",
        boolean: true,
        kernel: |x, y| Some(u64::from(x < y)),
    },
    Case {
        name: "le",
        operator: "<=",
        boolean: true,
        kernel: |x, y| Some(u64::from(x <= y)),
    },
];

fn corpus(case: &Case) -> Vec<[u64; 2]> {
    let high = 1u64 << 63;
    let mut rows = vec![
        [0, 0],
        [0, 1],
        [1, 0],
        [u64::MAX, 0],
        [u64::MAX, 1],
        [1, u64::MAX],
        [high - 1, 1],
        [high, high - 1],
        [high, 1],
        [u64::MAX - 1, 1],
        [u32::MAX as u64, u32::MAX as u64],
        [u64::MAX / 3, 3],
        [u64::MAX, u64::MAX],
        [high, high],
    ];
    let mut seed = 0x6a09_e667_f3bc_c909u64;
    for n in 0..64u64 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let (x, y) = (seed, seed.rotate_left(21));
        rows.push(match case.name {
            "add" => [x, y & !x],
            "sub" => [x.max(y), x.min(y)],
            "mul" => [x / (n % 17 + 1), n % 17 + 1],
            _ => [x, y],
        });
    }
    rows.retain(|&[x, y]| (case.kernel)(x, y).is_some());
    rows
}

fn value(case: &Case, bits: u64) -> Value {
    if case.boolean {
        Value::Bool(bits != 0)
    } else {
        Value::U64(bits)
    }
}

fn bits(value: Value) -> u64 {
    match value {
        Value::U64(n) => n,
        Value::Bool(n) => u64::from(n),
        _ => panic!("unexpected unsigned probe result tag"),
    }
}

// Match exact admission and checked arithmetic on the timed success domain.
// None is a decline, not an oracle for competing low-budget/trap error order;
// those failure contracts are exercised by tests/jit_unsigned.rs.
fn rust_host(case: &Case, inputs: &[Value], args: &[Value], limits: Limits) -> Option<Value> {
    if !inputs.is_empty()
        || args.len() != 2
        || limits.instructions < 4
        || limits.cells < 6
        || limits.frames < 1
    {
        return None;
    }
    let [Value::U64(x), Value::U64(y)] = args else {
        return None;
    };
    (black_box(case.kernel))(*x, *y).map(|n| value(case, n))
}

fn timed(count: usize, mut call: impl FnMut(usize) -> u64) -> (u64, u64) {
    let start = Instant::now();
    let mut checksum = 0u64;
    for n in 0..count {
        checksum = checksum.rotate_left(1) ^ black_box(call(black_box(n)));
    }
    (black_box(checksum), start.elapsed().as_nanos() as u64)
}

fn run_case(case: &Case, count: usize, samples: usize) -> Result<Json, Box<dyn Error>> {
    let source = format!(
        "v2 (pub run (x:u64 y:u64) {} = ({} x y))",
        if case.boolean { "bool" } else { "u64" },
        case.operator
    );
    let rows = corpus(case);
    let args: Vec<_> = rows
        .iter()
        .map(|&[x, y]| [Value::U64(x), Value::U64(y)])
        .collect();
    let expected: Vec<_> = rows
        .iter()
        .map(|&[x, y]| (case.kernel)(x, y).unwrap())
        .collect();
    let expected_checksum =
        (0..count).fold(0u64, |h, n| h.rotate_left(1) ^ expected[n % rows.len()]);
    assert!(rust_host(case, &[], &[Value::I64(0), Value::U64(0)], LIMITS).is_none());
    for limits in [
        Limits {
            instructions: 3,
            ..LIMITS
        },
        Limits { cells: 5, ..LIMITS },
        Limits {
            frames: 0,
            ..LIMITS
        },
    ] {
        assert!(rust_host(case, &[], &args[0], limits).is_none());
    }
    let mut observations = Vec::new();
    for sample in 0..samples {
        let start = Instant::now();
        let module = Module::compile_compact(&source)?;
        let source_compile_ns = start.elapsed().as_nanos() as u64;
        let start = Instant::now();
        let prepared = module.prepare_call("run")?;
        let cold_prepare_ns = start.elapsed().as_nanos() as u64;
        // Record whether compilation/publication succeeded independently of the
        // per-call witness; a true preparation still never substitutes for it.
        let native_prepared = module.prepare_native("run");
        let mut workspace = Workspace::default();
        let mut portable = Workspace::default();
        for (n, arguments) in args.iter().enumerate() {
            let expected = value(case, expected[n]);
            let (actual, route) = prepared.run_with_route(&[], arguments, LIMITS, &mut workspace);
            assert_eq!(
                route,
                ExecutionRoute::TypedJit,
                "{} must execute native code: sample={sample} row={n} arguments={arguments:?} actual={actual:?} limits={LIMITS:?} native_prepared={native_prepared} source={source:?}",
                case.name,
            );
            assert_eq!(actual?, expected);
            assert_eq!(
                rust_host(case, &[], arguments, LIMITS),
                Some(expected.clone())
            );
            assert_eq!(
                module.run_portable_with(
                    Invocation {
                        export: "run",
                        inputs: &[],
                        arguments,
                        limits: LIMITS,
                    },
                    &mut portable
                )?,
                expected
            );
            black_box((black_box(case.kernel))(rows[n][0], rows[n][1]).unwrap());
        }
        let mut times = [0u64; 3];
        for phase in 0..3 {
            let lane = (sample + phase) % 3;
            let (checksum, elapsed_ns) = match lane {
                0 => timed(count, |n| {
                    let [x, y] = black_box(rows[n % rows.len()]);
                    (black_box(case.kernel))(x, y).unwrap()
                }),
                1 => timed(count, |n| {
                    bits(
                        rust_host(
                            case,
                            black_box(&[]),
                            black_box(&args[n % rows.len()]),
                            black_box(LIMITS),
                        )
                        .unwrap(),
                    )
                }),
                _ => timed(count, |n| {
                    let (actual, route) = prepared.run_with_route(
                        black_box(&[]),
                        black_box(&args[n % rows.len()]),
                        black_box(LIMITS),
                        &mut workspace,
                    );
                    assert_eq!(
                        route,
                        ExecutionRoute::TypedJit,
                        "{} timed invocation declined: sample={sample} iteration={n} arguments={:?} actual={actual:?} limits={LIMITS:?} native_prepared={native_prepared}",
                        case.name,
                        args[n % rows.len()]
                    );
                    bits(actual.unwrap())
                }),
            };
            assert_eq!(checksum, expected_checksum, "{} lane {lane}", case.name);
            times[lane] = elapsed_ns;
        }
        observations.push(json!({"sample": sample, "source_compile_ns": source_compile_ns,
            "cold_prepare_ns": cold_prepare_ns, "phase_order_start": sample % 3,
            "rust_checked_kernel_elapsed_ns": times[0], "rust_checked_host_elapsed_ns": times[1],
            "prepared_jit_elapsed_ns": times[2], "timed_native_witnesses": count,
            "validation_native_witnesses": rows.len(), "checksum": format!("{expected_checksum:016x}")}));
    }
    Ok(
        json!({"operation": case.name, "source": source, "input_pairs": rows,
        "warmup_calls_per_arm": args.len(), "samples": observations}),
    )
}

fn main() -> Result<(), Box<dyn Error>> {
    let (mut iterations, mut samples) = (100_000usize, 5usize);
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        if flag == "--help" {
            println!(
                "bench_unsigned_jit [--iterations 1..1000000] [--samples 1..15]; requires release typed-jit"
            );
            return Ok(());
        }
        let number: usize = args.next().ok_or("missing option value")?.parse()?;
        match flag.as_str() {
            "--iterations" if (1..=1_000_000).contains(&number) => iterations = number,
            "--samples" if (1..=15).contains(&number) => samples = number,
            _ => return Err("unknown option or value outside the documented bound".into()),
        }
    }
    if !cfg!(all(
        feature = "typed-jit",
        target_arch = "x86_64",
        any(
            target_os = "windows",
            all(target_os = "linux", target_env = "gnu")
        )
    )) {
        return Err("requires typed-jit on Windows x86-64 or Linux GNU x86-64".into());
    }
    if cfg!(debug_assertions) {
        return Err("requires a release build".into());
    }
    let cases = CASES
        .iter()
        .map(|case| run_case(case, iterations, samples))
        .collect::<Result<Vec<_>, _>>()?;
    println!(
        "{}",
        serde_json::to_string(&json!({"schema": "fluxr-unsigned-jit-probe-v1",
        "iterations_per_arm": iterations, "samples_per_operation": samples,
        "limits": {"instructions": 4, "cells": 6, "frames": 1},
        "os": std::env::consts::OS, "arch": std::env::consts::ARCH,
        "observation_unit": "whole batch elapsed nanoseconds; divide by iterations for batch mean",
        "scope": "same successful checked u64 scalar values; kernel excludes host boundary; Rust host and prepared JIT include exact type/resource admission; JIT includes per-call native witness check",
        "preparation": "fresh module per sample; source compilation separate; first prepare may initialize the shared ISA",
        "claims": "raw observations only; no speed ratio, population tails, or broader R10 acceptance",
        "cases": cases}))?
    );
    Ok(())
}
