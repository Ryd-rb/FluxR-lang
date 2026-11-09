//! Matched release baselines; preparation and execution are reported separately.
use fluxr::structured::{Invocation, Limits, Module, ModuleSlot, Value, Workspace};
use fluxr::{Gate, Program, RuleProgram, telemetry};
use std::{fmt::Write, hint::black_box, time::Instant};

const CALLS: u32 = 100_000;
const TRACKS: u16 = 128;
const LEAF: &str = include_str!("fixtures/modern_leaf.flux");
const RULE: &str = include_str!("fixtures/modern_rule.flux");
const GATE: &str = include_str!("fixtures/modern_gate.flux");
const COUNTER: &str = include_str!("../tests/fixtures/structured_counter.fluxm");

#[path = "fixtures/native_projection.rs"]
mod projection;
#[path = "fixtures/modern_reference.rs"]
mod reference;

fn observe<T>(samples: &mut [f64], round: usize, calls: u32, run: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let result = black_box(run());
    samples[round] = start.elapsed().as_nanos() as f64 / f64::from(calls);
    result
}

fn distribution(samples: &mut [f64]) -> (f64, f64, f64) {
    samples.sort_by(f64::total_cmp);
    (
        samples[0],
        samples[samples.len() / 2],
        samples[samples.len() - 1],
    )
}

fn sum(calls: u32, mut f: impl FnMut(u32) -> i64) -> i64 {
    (0..calls).map(|i| black_box(f(black_box(i)))).sum()
}

fn compare(
    name: &str,
    calls: u32,
    mut rust: impl FnMut(u32) -> i64,
    mut flux: impl FnMut(u32) -> i64,
) {
    for i in 0..calls.min(4096) {
        assert_eq!(rust(i), flux(i), "{name}, input {i}");
    }
    let mut r = [0.; 9];
    let mut f = [0.; 9];
    for round in 0..r.len() {
        let (rs, fs) = if round % 2 == 0 {
            (
                observe(&mut r, round, calls, || sum(calls, &mut rust)),
                observe(&mut f, round, calls, || sum(calls, &mut flux)),
            )
        } else {
            let fs = observe(&mut f, round, calls, || sum(calls, &mut flux));
            (observe(&mut r, round, calls, || sum(calls, &mut rust)), fs)
        };
        assert_eq!(rs, fs, "{name}, round {round}");
    }
    let (rmin, r, rmax) = distribution(&mut r);
    let (fmin, f, fmax) = distribution(&mut f);
    println!(
        "{name}: Rust {r:.2} [{rmin:.2}, {rmax:.2}], FluxR {f:.2} [{fmin:.2}, {fmax:.2}] ns/call ({:.2}× Rust)",
        f / r
    );
}

fn lifecycle<T>(name: &str, mut run: impl FnMut() -> T) {
    let mut samples = [0.; 9];
    for round in 0..samples.len() {
        observe(&mut samples, round, 500, || {
            for _ in 0..500 {
                black_box(run());
            }
        });
    }
    let (min, median, max) = distribution(&mut samples);
    println!("{name}: {median:.2} [{min:.2}, {max:.2}] ns/call");
}

fn scalar(value: Value) -> i64 {
    let Value::I64(value) = value else {
        panic!("expected scalar")
    };
    value
}

fn leaf(args: &[Value]) -> Invocation<'_> {
    Invocation {
        export: "calc",
        inputs: &[],
        arguments: args,
        limits: Limits::default(),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    telemetry::configure("off")?;
    let projection = Module::compile_compact(include_str!("fixtures/native_projection.fluxr"))?;
    let mut projection_workspace = Workspace::default();
    compare(
        "structured.projection.direct",
        CALLS,
        |i| projection::project(i64::from(i) - 100),
        |i| {
            scalar(
                projection
                    .run_with(
                        Invocation {
                            export: "project",
                            inputs: &[],
                            arguments: &[Value::I64(i64::from(i) - 100)],
                            limits: Limits::default(),
                        },
                        &mut projection_workspace,
                    )
                    .unwrap(),
            )
        },
    );
    let module = Module::compile_compact(LEAF)?;
    let mut slot = ModuleSlot::new(module.clone(), &[])?;
    let candidate = Module::compile_compact(LEAF)?;
    slot.inject(0, candidate, Some(leaf(&[Value::I64(3), Value::I64(7)])))?;
    let mut workspace = Workspace::default();
    let mut invoke = |i, slot: Option<&ModuleSlot>| {
        let args = [Value::I64(i64::from(i)), Value::I64(7)];
        scalar(
            match slot {
                Some(slot) => slot.run_with(leaf(&args), &mut workspace),
                None => module.run_with(leaf(&args), &mut workspace),
            }
            .unwrap(),
        )
    };
    let rust_leaf = |i| reference::leaf(i64::from(i), 7).unwrap();
    compare("structured.scalar.direct", CALLS, rust_leaf, |i| {
        invoke(i, None)
    });
    compare("structured.scalar.injected", CALLS, rust_leaf, |i| {
        invoke(i, Some(&slot))
    });
    lifecycle("structured.scalar.source_compile", || {
        Module::compile_compact(black_box(LEAF)).unwrap()
    });
    lifecycle("structured.scalar.prepare_activate", || {
        let mut target = ModuleSlot::new(module.clone(), &[]).unwrap();
        target.inject(0, slot.module().clone(), None).unwrap()
    });
    lifecycle("structured.scalar.compile_activate_first_call", || {
        let candidate = Module::compile_compact(black_box(LEAF)).unwrap();
        let mut target = ModuleSlot::new(module.clone(), &[]).unwrap();
        target.inject(0, candidate, None).unwrap();
        target
            .run_with(leaf(&[Value::I64(3), Value::I64(7)]), &mut workspace)
            .unwrap()
    });

    let counter = Module::compile_legacy_json(COUNTER)?;
    let state = Value::Record([Value::I64(10)].into());
    compare(
        "structured.record_loop",
        CALLS / 10,
        |i| reference::counter(10, i64::from(i % 32), 3).unwrap(),
        |i| {
            let value = counter
                .run_with(
                    Invocation {
                        export: "tick",
                        inputs: &[Value::I64(3)],
                        arguments: &[state.clone(), Value::I64(i64::from(i % 32))],
                        limits: Limits::default(),
                    },
                    &mut workspace,
                )
                .unwrap();
            let Value::Record(fields) = value else {
                panic!("expected record")
            };
            scalar(fields[0].clone())
        },
    );

    let rule = RuleProgram::compile_text(RULE)?;
    let prepared = rule.clone().prepare();
    println!("rules.prepared.native={}", prepared.uses_native_code());
    let rust_rule = |i| reference::rule(i64::from(i), 7).unwrap();
    compare("rules.vm", CALLS, rust_rule, |i| {
        rule.evaluate(&[i64::from(i), 7]).unwrap()
    });
    compare("rules.prepared", CALLS, rust_rule, |i| {
        prepared.evaluate(&[i64::from(i), 7]).unwrap()
    });
    lifecycle("rules.native_prepare", || black_box(rule.clone()).prepare());

    let gate = Gate::compile_text(GATE)?;
    let fact = |i: u32, index| (i >> index) & 1_u32 != 0_u32;
    compare(
        "gate.lazy",
        CALLS,
        |i| i64::from(reference::gate(|n| fact(i, n))),
        |i| i64::from(gate.evaluate(|n| fact(i, n))),
    );

    let mut source = String::from("v1 1000");
    for id in 0..TRACKS {
        write!(source, "\n{id} 1 1 0:{id} 500:{} 1000:{id}", id + 500)?;
    }
    let timeline = Program::compile_text(&source)?;
    compare(
        "timeline.batch",
        CALLS / 2,
        |i| {
            reference::timeline(i, TRACKS)
                .map(|v| i64::from(black_box(v)))
                .sum()
        },
        |i| {
            timeline
                .sample_all(i)
                .map(|v| i64::from(black_box(v)))
                .sum()
        },
    );
    let track = timeline.bind_track((4, 1)).unwrap();
    compare(
        "timeline.bound",
        CALLS,
        |i| i64::from(reference::timeline(i, 5).last().unwrap()),
        |i| i64::from(track.sample(i)),
    );
    lifecycle("timeline.source_compile", || {
        Program::compile_text(black_box(&source)).unwrap()
    });
    Ok(())
}
