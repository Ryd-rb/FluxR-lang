//! Frozen structured host corpus with matched warm and lifecycle measurements.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]
use fluxr::Error;
use fluxr::structured::{
    Binding, Invocation, Limits, Module, ModuleSlot, PreparedAot, Type, Value, Workspace,
};
use sha2::{Digest, Sha256};
use std::{hint::black_box, time::Instant};

#[path = "fixtures/host_corpus_checks.rs"]
mod corpus_checks;
#[path = "fixtures/host_corpus_reference.rs"]
mod corpus_ref;
#[path = "fixtures/bench_counter_native.rs"]
mod counter_aot;
#[path = "fixtures/modern_reference.rs"]
#[allow(dead_code, unused_imports)]
mod reference;
#[path = "fixtures/bench_volume_typed_aot.rs"]
mod volume_aot;
#[path = "fixtures/bench_volume_host.rs"]
#[allow(
    clippy::needless_late_init,
    clippy::needless_question_mark,
    clippy::too_many_arguments
)]
mod volume_kernel_aot;
#[path = "fixtures/bench_volume_reference.rs"]
mod volume_ref;

const VOLUME: &str = include_str!("bench_volume.flux");
const COUNTER: &str = include_str!("../tests/fixtures/structured_counter.fluxm");
const LEAF: &str = include_str!("fixtures/modern_leaf.flux");
const SCAN: &str = include_str!("fixtures/host_scan.flux");
const DECISION: &str = include_str!("fixtures/host_decision.flux");
const SAMPLES: usize = 20_000;
const PASSES: usize = 21;
const COLD_SAMPLES: usize = 31;

fn report(name: &str, samples: &mut [f64]) {
    samples.sort_by(f64::total_cmp);
    let p95 = (samples.len() * 95).div_ceil(100) - 1;
    println!(
        "{name} p50={:.2} p95={:.2} min={:.2} max={:.2} ns",
        samples[samples.len() / 2],
        samples[p95],
        samples[0],
        samples[samples.len() - 1]
    );
}

fn time<T>(operation: impl FnOnce() -> T) -> (T, f64) {
    let start = Instant::now();
    let result = black_box(operation());
    (result, start.elapsed().as_nanos() as f64)
}

fn measure<const N: usize>(
    name: &str,
    expected: &[i64],
    mut routes: [(&str, &mut dyn FnMut(usize) -> i64); N],
) {
    for (label, route) in &mut routes {
        for (index, &value) in expected.iter().enumerate() {
            assert_eq!(route(index), value, "{name}.{label} sample {index}");
        }
    }
    // Order-sensitive, with the same fold on each timed route.
    let fold = |hash: u64, value: i64| hash.wrapping_mul(0x100000001b3) ^ value as u64;
    let checksum = expected.iter().copied().fold(0, fold);
    let mut times = [[0.; N]; PASSES];
    for (round, times) in times.iter_mut().enumerate() {
        for index in 0..N {
            let index = if round % 2 == 0 { index } else { N - 1 - index };
            let (_, run) = &mut routes[index];
            let (result, ns) = time(|| {
                (0..expected.len())
                    .map(|n| black_box(run(black_box(n))))
                    .fold(0, fold)
            });
            assert_eq!(result, checksum);
            times[index] = ns / expected.len() as f64;
        }
    }
    for (index, (label, _)) in routes.iter().enumerate() {
        report(
            &format!("{name}.{label}"),
            &mut times.map(|pass| pass[index]),
        );
    }
    println!("{name}.checksum={checksum:016x} samples={}", expected.len());
}

// Rotate route positions so no path consistently sees a warmer input block.
fn measure_blocks<const N: usize>(
    name: &str,
    expected: &[i64],
    block: usize,
    mut routes: [(&str, &mut dyn FnMut(usize) -> i64); N],
) {
    for (label, route) in &mut routes {
        for (index, &value) in expected.iter().enumerate() {
            assert_eq!(route(index), value, "{name}.{label} sample {index}");
        }
    }
    // Order-sensitive, with the same fold on each timed route.
    let fold = |hash: u64, value: i64| hash.wrapping_mul(0x100000001b3) ^ value as u64;
    let checksum = expected.iter().copied().fold(0, fold);
    let mut times: [Vec<f64>; N] =
        std::array::from_fn(|_| Vec::with_capacity(PASSES * expected.len().div_ceil(block)));
    for round in 0..PASSES {
        for (part, values) in expected.chunks(block).enumerate() {
            let start = part * block;
            let expected_hash = values.iter().copied().fold(0, fold);
            let phase = round + part;
            for index in 0..N {
                let index = if phase / N % 2 == 0 {
                    index
                } else {
                    N - 1 - index
                };
                let index = (index + phase) % N;
                let (_, run) = &mut routes[index];
                let (result, ns) = time(|| {
                    (start..start + values.len())
                        .map(|n| black_box(run(black_box(n))))
                        .fold(0, fold)
                });
                assert_eq!(result, expected_hash);
                times[index].push(ns / values.len() as f64);
            }
        }
    }
    for ((label, _), samples) in routes.iter().zip(&mut times) {
        report(&format!("{name}.{label}"), samples);
    }
    println!("{name}.checksum={checksum:016x} samples={}", expected.len());
}

fn measure_injection<'a>(
    name: &str,
    bundled: &Module,
    source: &ModuleSlot,
    grants: &[Binding],
    call: impl Fn(usize) -> Invocation<'a>,
    expected: &[i64],
    mut rust_host: impl FnMut(usize) -> i64,
    unpack: fn(Value) -> i64,
) -> Result<(), Error> {
    let export = call(0).export;
    let candidate = Module::decode(&bundled.encode())?;
    let native = bundled.prepare_native(export);
    assert_eq!(source.module().prepare_native(export), native);
    assert_eq!(candidate.prepare_native(export), native);
    let mut artifact = ModuleSlot::new(bundled.clone(), grants)?;
    artifact.inject(0, candidate, Some(call(0)))?;
    let [
        mut ws_bundled,
        mut ws_source,
        mut ws_artifact,
        mut ws_portable,
    ] = std::array::from_fn(|_| Workspace::default());
    let mut bundled_call = |i| unpack(bundled.run_with(call(i), &mut ws_bundled).unwrap());
    let mut source = |i| unpack(source.run_with(call(i), &mut ws_source).unwrap());
    let mut artifact = |i| unpack(artifact.run_with(call(i), &mut ws_artifact).unwrap());
    measure_blocks(
        &format!("{name}_injection"),
        expected,
        expected.len(),
        [
            ("rust_host", &mut rust_host),
            ("bundled", &mut bundled_call),
            ("source", &mut source),
            ("artifact", &mut artifact),
        ],
    );
    if export == "pick" {
        // These percentiles describe 64-call means, including timer overhead.
        measure_blocks(
            &format!("{name}_blocks64"),
            expected,
            64,
            [
                ("rust_host", &mut rust_host),
                ("portable", &mut |i| {
                    unpack(
                        bundled
                            .run_portable_with(call(i), &mut ws_portable)
                            .unwrap(),
                    )
                }),
                ("bundled", &mut bundled_call),
                ("source", &mut source),
                ("artifact", &mut artifact),
            ],
        );
    }
    Ok(())
}

type PrepareAot = for<'a> fn(&'a Module) -> Result<PreparedAot<'a>, Error>;

fn lifecycle<'a>(
    export: &str,
    compile: impl Fn() -> Result<Module, Error>,
    grant: &[Binding],
    call: impl Fn() -> Invocation<'a>,
    aot: Option<PrepareAot>,
) -> Result<(), Box<dyn std::error::Error>> {
    let original = compile()?;
    let bytes = original.encode();
    for (name, decode) in [
        (export.to_owned(), false),
        (format!("{export}_artifact"), true),
    ] {
        let mut phases = [[0.; 6]; COLD_SAMPLES];
        let mut workspace = Workspace::default();
        let mut native = false;
        for [load_ns, prepare_ns, hit_ns, aot_ns, activate_ns, first_ns] in &mut phases {
            let mut slot = ModuleSlot::new(original.clone(), grant)?;
            let (candidate, ns) = time(|| {
                if decode {
                    Module::decode(black_box(&bytes))
                } else {
                    compile()
                }
            });
            let candidate = candidate?;
            *load_ns = ns;
            let (prepared, ns) = time(|| candidate.prepare_native(black_box(export)));
            native = prepared;
            *prepare_ns = ns;
            let (cached, ns) = time(|| candidate.prepare_native(black_box(export)));
            assert_eq!(cached, prepared);
            *hit_ns = ns;
            if let Some(aot) = aot {
                let (result, ns) = time(|| aot(&candidate));
                result?;
                *aot_ns = ns;
            }
            let (result, ns) = time(|| slot.inject(0, candidate, None));
            assert_eq!(result?, 1);
            *activate_ns = ns;
            let (result, ns) = time(|| slot.run_with(call(), &mut workspace));
            result?;
            *first_ns = ns;
        }
        println!("{name}.prepared_native={native}");
        for (index, label) in [
            if decode { "decode" } else { "compile" },
            "prepare_native",
            "prepare_cached",
            "prepare_aot",
            "activate_prepared",
            "first_call",
        ]
        .into_iter()
        .enumerate()
        {
            if index != 3 || aot.is_some() {
                report(
                    &format!("{name}.{label}"),
                    &mut phases.map(|round| round[index]),
                );
            }
        }
    }
    Ok(())
}

fn scalar(result: Value) -> i64 {
    let Value::I64(value) = result else {
        panic!("expected scalar");
    };
    value
}

fn record(result: Value) -> i64 {
    let Value::Record(fields) = result else {
        panic!("expected record");
    };
    let [Value::I64(total)] = &*fields else {
        panic!("expected counter record");
    };
    *total
}

fn index(result: Value) -> i64 {
    let Value::U64(index) = result else {
        panic!("expected unsigned index");
    };
    i64::try_from(index).unwrap()
}

fn fits(limits: Limits, needed: Limits) -> bool {
    limits.instructions >= needed.instructions
        && limits.cells >= needed.cells
        && limits.frames >= needed.frames
}

fn rust_volume_host(call: Invocation<'_>, need: Limits) -> Option<Value> {
    if call.export != "volume" || !call.inputs.is_empty() || !fits(call.limits, need) {
        return None;
    }
    let mut lanes = [0; 11];
    if lanes.len() != call.arguments.len() {
        return None;
    }
    for (out, value) in lanes.iter_mut().zip(call.arguments) {
        let Value::I64(value) = value else {
            return None;
        };
        *out = *value;
    }
    volume_ref::volume(lanes).map(Value::I64)
}

fn rust_counter_host(call: Invocation<'_>) -> Option<Value> {
    let [Value::I64(bonus)] = call.inputs else {
        return None;
    };
    let [Value::Record(state), Value::I64(count)] = call.arguments else {
        return None;
    };
    let [Value::I64(total)] = &**state else {
        return None;
    };
    if call.export != "tick" {
        return None;
    }
    // Same conservative path meter as the typed fixture. Each update reserves
    // one COW cell, and odd iterations call the checked helper.
    let entries = (*count).max(0) as u64;
    let odd = entries / 2;
    let instructions = entries
        .checked_mul(23)?
        .checked_add(odd * 4)?
        .checked_add(12)?;
    let cells = 14u64.checked_add(entries)?.checked_add(odd * 2)?;
    if !fits(
        call.limits,
        Limits {
            instructions,
            cells: usize::try_from(cells).ok()?,
            frames: 3,
        },
    ) {
        return None;
    }
    let result = reference::counter(*total, *count, *bonus)?;
    Some(if entries == 0 {
        Value::Record(state.clone())
    } else {
        Value::Record([Value::I64(result)].into())
    })
}

fn volume() -> Result<(), Box<dyn std::error::Error>> {
    let compile = || Module::compile_compact(black_box(VOLUME));
    static ZERO: [Value; 11] = [const { Value::I64(0) }; 11];
    lifecycle(
        "volume",
        compile,
        &[],
        || Invocation {
            export: "volume",
            inputs: &[],
            arguments: &ZERO,
            limits: Limits::default(),
        },
        Some(volume_aot::prepare),
    )?;
    let direct = compile()?;
    let mut injected = ModuleSlot::new(compile()?, &[])?;
    let aot = volume_aot::prepare(&direct)?;
    let need = direct.native_ceiling("volume", 11).unwrap();
    let identity: [u8; 32] = Sha256::digest(direct.encode()).into();
    assert_eq!(identity, volume_kernel_aot::IDENTITY);
    let lanes: Vec<[i64; 11]> = (0..SAMPLES as i64)
        .map(|i| {
            std::array::from_fn(|lane| {
                let lane = lane as i64;
                let value = i * (23 + 14 * lane) + 43 * lane;
                if lane < 8 {
                    value % 4093 - 2046
                } else {
                    value % 1025
                }
            })
        })
        .collect();
    let args: Vec<_> = lanes.iter().map(|row| row.map(Value::I64)).collect();
    let call = |i: usize| Invocation {
        export: "volume",
        inputs: &[],
        arguments: &args[i],
        limits: Limits::default(),
    };
    direct.prepare_native("volume");
    injected.inject(0, compile()?, Some(call(0)))?;
    let expected: Vec<_> = lanes
        .iter()
        .map(|&row| volume_ref::volume(row).unwrap())
        .collect();
    let [mut ws_vm, mut ws_direct, mut ws_injected, mut ws_aot] =
        std::array::from_fn(|_| Workspace::default());
    measure(
        "volume",
        &expected,
        [
            ("rust_kernel", &mut |i| {
                volume_ref::volume(lanes[i]).unwrap()
            }),
            ("rust_host", &mut |i| {
                scalar(rust_volume_host(call(i), need).unwrap())
            }),
            ("prior_aot_kernel", &mut |i| {
                volume_kernel_aot::run(lanes[i]).unwrap()
            }),
            ("aot_host", &mut |i| {
                scalar(aot.run_with(call(i), &mut ws_aot).unwrap())
            }),
            ("vm", &mut |i| {
                scalar(direct.run_portable_with(call(i), &mut ws_vm).unwrap())
            }),
            ("direct", &mut |i| {
                scalar(direct.run_with(call(i), &mut ws_direct).unwrap())
            }),
            ("injected", &mut |i| {
                scalar(injected.run_with(call(i), &mut ws_injected).unwrap())
            }),
        ],
    );
    measure_injection(
        "volume",
        &direct,
        &injected,
        &[],
        call,
        &expected,
        |i| scalar(rust_volume_host(call(i), need).unwrap()),
        scalar,
    )?;
    Ok(())
}

fn counter() -> Result<(), Box<dyn std::error::Error>> {
    let compile = || Module::compile_legacy_json(black_box(COUNTER));
    let grant = [Binding {
        name: "bonus".into(),
        ty: Type::I64,
    }];
    let cold_args = [Value::Record([Value::I64(10)].into()), Value::I64(19)];
    // The cold call borrows the same inputs for every freshly activated module.
    let args = &cold_args;
    lifecycle(
        "tick",
        compile,
        &grant,
        || Invocation {
            export: "tick",
            inputs: &[Value::I64(3)],
            arguments: args,
            limits: Limits::default(),
        },
        Some(counter_aot::prepare),
    )?;
    let direct = compile()?;
    let mut injected = ModuleSlot::new(compile()?, &grant)?;
    let aot = counter_aot::prepare(&direct)?;
    let counts: Vec<_> = (0..SAMPLES).map(|i| (i % 32) as i64).collect();
    let snapshot = [Value::I64(3)];
    let state = Value::Record([Value::I64(10)].into());
    let args: Vec<_> = counts
        .iter()
        .map(|&count| [state.clone(), Value::I64(count)])
        .collect();
    let call = |i: usize| Invocation {
        export: "tick",
        inputs: &snapshot,
        arguments: &args[i],
        limits: Limits::default(),
    };
    direct.prepare_native("tick");
    injected.inject(0, compile()?, Some(call(0)))?;
    let expected: Vec<_> = counts
        .iter()
        .map(|&count| reference::counter(10, count, 3).unwrap())
        .collect();
    let [mut ws_vm, mut ws_direct, mut ws_injected, mut ws_aot] =
        std::array::from_fn(|_| Workspace::default());
    measure(
        "counter",
        &expected,
        [
            ("rust_kernel", &mut |i| {
                reference::counter(10, counts[i], 3).unwrap()
            }),
            ("rust_host", &mut |i| {
                record(rust_counter_host(call(i)).unwrap())
            }),
            ("aot_host", &mut |i| {
                record(aot.run_with(call(i), &mut ws_aot).unwrap())
            }),
            ("vm", &mut |i| {
                record(direct.run_portable_with(call(i), &mut ws_vm).unwrap())
            }),
            ("direct", &mut |i| {
                record(direct.run_with(call(i), &mut ws_direct).unwrap())
            }),
            ("injected", &mut |i| {
                record(injected.run_with(call(i), &mut ws_injected).unwrap())
            }),
        ],
    );
    measure_injection(
        "counter",
        &direct,
        &injected,
        &grant,
        call,
        &expected,
        |i| record(rust_counter_host(call(i)).unwrap()),
        record,
    )?;
    Ok(())
}

fn leaf() -> Result<(), Box<dyn std::error::Error>> {
    let compile = || Module::compile_compact(black_box(LEAF));
    let make_call = |arguments| Invocation {
        export: "calc",
        inputs: &[],
        arguments,
        limits: Limits::default(),
    };
    lifecycle(
        "calc",
        compile,
        &[],
        || make_call(&[Value::I64(3), Value::I64(7)]),
        None,
    )?;
    let module = compile()?;
    module.prepare_native("calc");
    let args: Vec<_> = (0..SAMPLES)
        .map(|i| [Value::I64(i as i64), Value::I64(7)])
        .collect();
    let expected: Vec<_> = (0..SAMPLES)
        .map(|i| reference::leaf(i as i64, 7).unwrap())
        .collect();
    let mut workspace = Workspace::default();
    measure(
        "leaf",
        &expected,
        [
            ("rust_kernel", &mut |i| {
                reference::leaf(i as i64, 7).unwrap()
            }),
            ("direct", &mut |i| {
                scalar(
                    module
                        .run_with(make_call(&args[i]), &mut workspace)
                        .unwrap(),
                )
            }),
        ],
    );
    Ok(())
}

fn boundary_case(
    export: &str,
    declaration: &str,
    record_input: bool,
    needed: Limits,
    evaluate: impl Fn(&Value, &Value) -> Option<Value>,
    unpack: fn(Value) -> i64,
) -> Result<(), Box<dyn std::error::Error>> {
    let source = format!("v2 (rec R x:i64) (pub {export} {declaration})");
    let compile = || Module::compile_compact(black_box(&source));
    let arguments: Vec<_> = (0..SAMPLES)
        .map(|i| {
            let first = Value::I64(i as i64 - SAMPLES as i64 / 2);
            let first = if record_input {
                Value::Record([first].into())
            } else {
                first
            };
            [first, Value::I64(i as i64 % 71 - 35)]
        })
        .collect();
    let call = |i: usize| Invocation {
        export,
        inputs: &[],
        arguments: &arguments[i],
        limits: Limits::default(),
    };
    let host = |call: Invocation<'_>| {
        let [first, second] = call.arguments else {
            return None;
        };
        if call.export != export || !call.inputs.is_empty() || !fits(call.limits, needed) {
            return None;
        }
        evaluate(first, second)
    };
    lifecycle(export, compile, &[], || call(0), None)?;
    let module = compile()?;
    let mut source = ModuleSlot::new(module.clone(), &[])?;
    source.inject(0, compile()?, Some(call(0)))?;

    // Check the exact budget frontier and preserve portable trap precedence.
    let mut artifact = ModuleSlot::new(module.clone(), &[])?;
    artifact.inject(0, Module::decode(&module.encode())?, Some(call(0)))?;
    let prepared = module.prepare_native(export);
    if cfg!(all(
        feature = "typed-jit",
        target_os = "windows",
        target_arch = "x86_64"
    )) {
        assert!(prepared, "{export} failed to prepare");
    }
    let max = if record_input {
        Value::Record([Value::I64(i64::MAX)].into())
    } else {
        Value::I64(i64::MAX)
    };
    let overflow = [max, Value::I64(1)];
    let bad_tag = [arguments[0][0].clone(), Value::Bool(false)];
    let check = |arguments, limits| {
        let at_limit = || Invocation {
            arguments,
            limits,
            ..call(0)
        };
        let mut workspace = Workspace::default();
        let portable = module.run_portable_with(at_limit(), &mut workspace);
        assert_eq!(host(at_limit()), portable.as_ref().ok().cloned());
        let result = portable.map_err(|error| error.to_string());
        for route in [&module, source.module(), artifact.module()] {
            assert_eq!(
                route.run(at_limit()).map_err(|error| error.to_string()),
                result,
                "{export}"
            );
        }
    };
    for limits in [
        needed,
        Limits {
            instructions: needed.instructions - 1,
            ..needed
        },
        Limits {
            cells: needed.cells - 1,
            ..needed
        },
        Limits {
            frames: 0,
            ..needed
        },
    ] {
        check(&arguments[0], limits);
        check(&overflow, limits);
        check(&bad_tag, limits);
    }
    let mut workspace = Workspace::default();
    let expected: Vec<_> = (0..SAMPLES)
        .map(|i| {
            let result = module.run_portable_with(call(i), &mut workspace).unwrap();
            assert_eq!(host(call(i)).as_ref(), Some(&result));
            unpack(result)
        })
        .collect();
    measure_injection(
        export,
        &module,
        &source,
        &[],
        call,
        &expected,
        |i| unpack(host(call(i)).unwrap()),
        unpack,
    )?;
    Ok(())
}

fn boundaries() -> Result<(), Box<dyn std::error::Error>> {
    let limits = |instructions, cells| Limits {
        instructions,
        cells,
        frames: 1,
    };
    let signed_add = |x: &Value, d: &Value| {
        let (Value::I64(x), Value::I64(d)) = (x, d) else {
            return None;
        };
        x.checked_add(*d).map(Value::I64)
    };
    boundary_case(
        "signed_add",
        "(x:i64 d:i64) i64 () (ret (+ x d))",
        false,
        limits(4, 6),
        signed_add,
        scalar,
    )?;
    // A local excludes the signed-leaf tier while preserving the arithmetic.
    boundary_case(
        "signed_typed",
        "(x:i64 d:i64) i64 (v:i64) (= v (+ x d)) (ret v)",
        false,
        limits(6, 7),
        signed_add,
        scalar,
    )?;
    boundary_case(
        "record_read",
        "(s:R d:i64) i64 (v:i64) (= v (+ s.x d)) (ret v)",
        true,
        limits(7, 8),
        |s, d| {
            let Value::Record(s) = s else { return None };
            let ([Value::I64(x)], Value::I64(d)) = (&**s, d) else {
                return None;
            };
            x.checked_add(*d).map(Value::I64)
        },
        scalar,
    )?;
    boundary_case(
        "record_reuse",
        "(s:R d:i64) R () (ret s)",
        true,
        limits(2, 8),
        |s, d| {
            let Value::Record(fields) = s else {
                return None;
            };
            let ([Value::I64(_)], Value::I64(_)) = (&**fields, d) else {
                return None;
            };
            Some(s.clone())
        },
        record,
    )?;
    boundary_case(
        "record_write",
        "(s:R d:i64) R () (= s.x (+ s.x d)) (ret s)",
        true,
        limits(7, 9),
        |s, d| {
            let Value::Record(s) = s else { return None };
            let ([Value::I64(x)], Value::I64(d)) = (&**s, d) else {
                return None;
            };
            Some(Value::Record([Value::I64(x.checked_add(*d)?)].into()))
        },
        record,
    )
}

fn read_float(value: &Value) -> f32 {
    let Value::F32(bits) = value else {
        unreachable!("validated float");
    };
    f32::from_bits(*bits)
}

fn rust_scan_host(call: Invocation<'_>) -> Option<Value> {
    let [Value::List(values)] = call.arguments else {
        return None;
    };
    if call.export != "pick"
        || !call.inputs.is_empty()
        || !fits(
            call.limits,
            Limits {
                instructions: values.len() as u64 + 3,
                cells: values.len() + 4,
                frames: 1,
            },
        )
        || !values.iter().all(|value| matches!(value, Value::F32(_)))
    {
        return None;
    }
    Some(Value::U64(corpus_ref::pick(values.iter().map(read_float))))
}

fn scan() -> Result<(), Box<dyn std::error::Error>> {
    let compile = || Module::compile_compact(black_box(SCAN));
    let cold = [scan_values(&[3., 1., 1.])];
    let call = |arguments| Invocation {
        export: "pick",
        inputs: &[],
        arguments,
        limits: Limits::default(),
    };
    lifecycle("pick", compile, &[], || call(&cold), None)?;
    let direct = compile()?;
    assert_eq!(
        direct.prepare_native("pick"),
        cfg!(all(
            feature = "typed-jit",
            target_os = "windows",
            target_arch = "x86_64"
        ))
    );
    let mut injected = ModuleSlot::new(compile()?, &[])?;
    injected.inject(0, compile()?, Some(call(&cold)))?;

    // Same host admission and argument clone as pick, with no scan.
    let validation_control =
        Module::compile_compact("v2 (pub pick (values:[f32]) u64 () (ret 0u))")?;
    for len in [16, 256] {
        let inputs: Vec<Vec<f32>> = (0..2048).map(|row| scan_input(row, len)).collect();
        let arguments: Vec<_> = inputs.iter().map(|row| [scan_values(row)]).collect();
        let expected: Vec<_> = inputs
            .iter()
            .map(|row| corpus_ref::pick(row.iter().copied()) as i64)
            .collect();
        let call = |i: usize| Invocation {
            export: "pick",
            inputs: &[],
            arguments: &arguments[i],
            limits: Limits::default(),
        };
        let [mut ws_vm, mut ws_direct, mut ws_injected, mut ws_control] =
            std::array::from_fn(|_| Workspace::default());
        measure(
            &format!("scan{len}"),
            &expected,
            [
                ("rust_kernel", &mut |i| {
                    corpus_ref::pick(inputs[i].iter().copied()) as i64
                }),
                ("rust_host", &mut |i| {
                    index(rust_scan_host(call(i)).unwrap())
                }),
                ("vm", &mut |i| {
                    index(direct.run_portable_with(call(i), &mut ws_vm).unwrap())
                }),
                ("direct", &mut |i| {
                    index(direct.run_with(call(i), &mut ws_direct).unwrap())
                }),
                ("injected", &mut |i| {
                    index(injected.run_with(call(i), &mut ws_injected).unwrap())
                }),
            ],
        );
        measure(
            &format!("scan{len}_control"),
            &vec![0; inputs.len()],
            [("vm_validation", &mut |i| {
                index(
                    validation_control
                        .run_portable_with(call(i), &mut ws_control)
                        .unwrap(),
                )
            })],
        );
        measure_injection(
            &format!("scan{len}"),
            &direct,
            &injected,
            &[],
            call,
            &expected,
            |i| index(rust_scan_host(call(i)).unwrap()),
            index,
        )?;
    }
    Ok(())
}

fn scan_input(row: usize, len: usize) -> Vec<f32> {
    let mut values: Vec<_> = (0..len)
        .map(|i| ((row * (23 + 14 * i) + 43 * i) % 509) as f32 - 254.)
        .collect();
    let first = row % (len - 2);
    values[first] = -300.;
    values[first + 1] = -300.;
    match row % 4 {
        0 => values[0] = f32::from_bits(0x7fc0_0031),
        1 => values[len - 1] = f32::from_bits(0x7f80_0001),
        _ => {}
    }
    values
}

fn scan_values(values: &[f32]) -> Value {
    Value::List(
        values
            .iter()
            .map(|value| Value::F32(value.to_bits()))
            .collect::<Vec<_>>()
            .into(),
    )
}

fn rust_decision_host(call: Invocation<'_>, need: Limits) -> Option<Value> {
    let [Value::I64(a), Value::I64(b), Value::I64(c), Value::I64(x)] = call.arguments else {
        return None;
    };
    if call.export != "decide" || !call.inputs.is_empty() || !fits(call.limits, need) {
        return None;
    }
    corpus_ref::decide([*a, *b, *c, *x]).map(Value::I64)
}

fn decision() -> Result<(), Box<dyn std::error::Error>> {
    let compile = || Module::compile_compact(black_box(DECISION));
    let cold = [0; 4].map(Value::I64);
    let call = |arguments| Invocation {
        export: "decide",
        inputs: &[],
        arguments,
        limits: Limits::default(),
    };
    lifecycle("decide", compile, &[], || call(&cold), None)?;
    let direct = compile()?;
    let need = direct.native_ceiling("decide", 4).unwrap();
    direct.prepare_native("decide");
    let mut injected = ModuleSlot::new(compile()?, &[])?;
    injected.inject(0, compile()?, Some(call(&cold)))?;
    for (name, bias) in [("decision_uniform", 1), ("decision_skewed", 7)] {
        let lanes = decision_inputs(bias);
        let mut leaves = [0; 8];
        for &[a, b, c, _] in &lanes {
            leaves[(a < 0) as usize * 4 + (b < 0) as usize * 2 + (c < 0) as usize] += 1;
        }
        println!("{name}.leaves={leaves:?}");
        let arguments: Vec<_> = lanes.iter().map(|row| row.map(Value::I64)).collect();
        let expected: Vec<_> = lanes
            .iter()
            .map(|&row| corpus_ref::decide(row).unwrap())
            .collect();
        let call = |i: usize| Invocation {
            export: "decide",
            inputs: &[],
            arguments: &arguments[i],
            limits: Limits::default(),
        };
        let [mut ws_vm, mut ws_direct, mut ws_injected] =
            std::array::from_fn(|_| Workspace::default());
        measure(
            name,
            &expected,
            [
                ("rust_kernel", &mut |i| {
                    corpus_ref::decide(lanes[i]).unwrap()
                }),
                ("rust_host", &mut |i| {
                    scalar(rust_decision_host(call(i), need).unwrap())
                }),
                ("vm", &mut |i| {
                    scalar(direct.run_portable_with(call(i), &mut ws_vm).unwrap())
                }),
                ("direct", &mut |i| {
                    scalar(direct.run_with(call(i), &mut ws_direct).unwrap())
                }),
                ("injected", &mut |i| {
                    scalar(injected.run_with(call(i), &mut ws_injected).unwrap())
                }),
            ],
        );
        measure_injection(
            name,
            &direct,
            &injected,
            &[],
            call,
            &expected,
            |i| scalar(rust_decision_host(call(i), need).unwrap()),
            scalar,
        )?;
    }
    Ok(())
}

fn decision_inputs(bias: u64) -> Vec<[i64; 4]> {
    let mut state = 0xca3a_5eed_b528_08aeu64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    (0..SAMPLES)
        .map(|_| {
            let mut row = std::array::from_fn(|_| {
                let word = next();
                let magnitude = (word >> 3) as i64 % 4093 + 1;
                if word & bias == 0 {
                    -magnitude
                } else {
                    magnitude
                }
            });
            row[3] = (next() % 8191) as i64 - 4095;
            row
        })
        .collect()
}

// The wasm32-unknown-unknown binary runs the same deterministic cases without
// using Instant or measuring Node's call boundary.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn host_corpus_check() -> u32 {
    corpus_checks::check() as u32
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn list_effect_check() -> u32 {
    corpus_checks::check_list_effects() as u32
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn typed_effect_check() -> u32 {
    corpus_checks::check_typed_effects() as u32
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn universal_tape_check() -> u32 {
    corpus_checks::check_universal_tape() as u32
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn structured_boundary_check() -> u32 {
    corpus_checks::check_structured_boundary() as u32
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn compound_effect_check() -> u32 {
    corpus_checks::check_compound_effects() as u32
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn recursive_effect_check() -> u32 {
    corpus_checks::check_recursive_effects() as u32
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn control_flow_check() -> u32 {
    corpus_checks::check_control_flow() as u32
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn loop_exits_check() -> Vec<u32> {
    let (boolean, loops) = corpus_checks::check_loop_exits();
    vec![boolean as u32, loops as u32]
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn nested_loop_calls_check() -> u32 {
    corpus_checks::check_nested_loop_calls() as u32
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn binary32_boundary_check() -> u32 {
    corpus_checks::check_binary32_boundary() as u32
}

#[cfg(test)]
#[test]
fn binary32_boundary_matches_frozen_bits_and_limits() {
    assert_eq!(corpus_checks::check_binary32_boundary(), 372);
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn list_boundary_check() -> u32 {
    corpus_checks::check_list_boundary() as u32
}

#[cfg(test)]
#[test]
fn list_boundary_matches_effect_ledger() {
    assert_eq!(corpus_checks::check_list_boundary(), 33_601);
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn variant_payload_check() -> u32 {
    corpus_checks::check_variant_payload() as u32
}

#[cfg(test)]
#[test]
fn variant_payload_matches_effect_ledger() {
    assert_eq!(corpus_checks::check_variant_payload(), 30_619);
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn native_projection_check() -> u32 {
    #[path = "fixtures/native_projection.rs"]
    mod reference;
    let module =
        Module::compile_compact(include_str!("fixtures/native_projection.fluxr")).unwrap();
    let module = Module::decode(&module.encode()).unwrap();
    let cases = [i64::MIN, -1, 0, 24, 25, i64::MAX];
    for input in cases {
        let value = module
            .run(Invocation {
                export: "project",
                inputs: &[],
                arguments: &[Value::I64(input)],
                limits: Limits::default(),
            })
            .unwrap();
        assert_eq!(value, Value::I64(reference::project(input)));
    }
    cases.len() as u32
}

#[cfg(test)]
#[test]
fn host_corpus_matches_reference() {
    assert_eq!(corpus_checks::check(), 35);
}

#[cfg(test)]
#[test]
fn list_effects_match_oracle() {
    assert_eq!(corpus_checks::check_list_effects(), 4164);
}

#[cfg(test)]
#[test]
fn typed_effects_match_specification() {
    assert_eq!(corpus_checks::check_typed_effects(), 19_770);
}

#[cfg(test)]
#[test]
fn compound_effects_match_specification() {
    assert_eq!(corpus_checks::check_compound_effects(), 24_253);
}

#[cfg(test)]
#[test]
fn recursive_effects_match_specification() {
    assert_eq!(corpus_checks::check_recursive_effects(), 27_014);
}

#[cfg(test)]
#[test]
fn control_flow_matches_specification() {
    assert_eq!(corpus_checks::check_control_flow(), 41_110);
}

#[cfg(test)]
#[test]
fn loop_exits_and_booleans_match_specification() {
    assert_eq!(corpus_checks::check_loop_exits(), (24_492, 34_596));
}

#[cfg(test)]
#[test]
fn nested_loop_calls_match_source_ledger() {
    assert_eq!(corpus_checks::check_nested_loop_calls(), 52_511);
}

#[cfg(test)]
#[test]
fn universal_tape_matches_reference_and_resumes() {
    assert_eq!(corpus_checks::check_universal_tape(), 75);
}

#[cfg(test)]
#[test]
fn structured_boundary_rejects_artifacts_and_preserves_capabilities() {
    assert_eq!(corpus_checks::check_structured_boundary(), 761);
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    fluxr::telemetry::configure("off")?;
    println!(
        "typed-jit={} native-jit={} telemetry=off passes={PASSES}",
        cfg!(feature = "typed-jit"),
        cfg!(feature = "native-jit")
    );
    #[cfg(target_arch = "x86_64")]
    println!(
        "cpu.sse2={} sse42={} avx={} avx2={} fma={} bmi2={} avx512f={}",
        is_x86_feature_detected!("sse2"),
        is_x86_feature_detected!("sse4.2"),
        is_x86_feature_detected!("avx"),
        is_x86_feature_detected!("avx2"),
        is_x86_feature_detected!("fma"),
        is_x86_feature_detected!("bmi2"),
        is_x86_feature_detected!("avx512f"),
    );
    println!("host_corpus.checks={}", corpus_checks::check());
    volume()?;
    counter()?;
    scan()?;
    decision()?;
    leaf()?;
    boundaries()
}
