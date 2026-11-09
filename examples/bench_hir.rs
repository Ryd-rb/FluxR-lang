//! Time FXH v1 preparation and restored execution separately. Pass a directory
//! containing matching <case>.fxh and <case>.fxr files and optionally "reverse".
use fluxr::RuleProgram;
use fluxr::hir::{Artifact, Hir};
use fluxr::structured::{Invocation, Limits, Module, Value, Workspace};
use std::{hint::black_box, path::PathBuf, time::Instant};

fn measure(case: &str, passes: usize, reverse: bool, routes: &mut [(&str, &mut dyn FnMut())]) {
    for (_, run) in routes.iter_mut() {
        for _ in 0..256 {
            run();
        }
    }
    for round in 0..11 {
        for step in 0..routes.len() {
            let index = if (round % 2 == 1) ^ reverse {
                routes.len() - step - 1
            } else {
                step
            };
            let (route, run) = &mut routes[index];
            let start = Instant::now();
            for _ in 0..passes {
                run();
            }
            println!(
                "{case},{route},{round},{:.2}",
                start.elapsed().as_secs_f64() * 1e9 / passes as f64
            );
        }
    }
}

fn main() {
    let mut args = std::env::args_os().skip(1);
    let Some(directory) = args.next().map(PathBuf::from) else {
        return; // cargo test --all-targets
    };
    let reverse = args.next().is_some_and(|arg| arg == "reverse");
    println!("case,route,round,ns_per_operation");
    for (case, inputs) in [
        ("guard", [1, 3]),
        ("raw-binary32", [0x8000_0000, 0x8000_0000]),
        ("guard-limit", [1, 3]),
    ] {
        let fxh = std::fs::read(directory.join(format!("{case}.fxh"))).unwrap();
        let fxd = std::fs::read(directory.join(format!("{case}.fxr"))).unwrap();
        let legacy = RuleProgram::decode(&fxd).unwrap();
        let checked = Hir::decode(&fxh).unwrap();
        assert_eq!(checked.encode().unwrap(), fxh);
        let Artifact::Rule(restored) = Hir::decode(&fxh).unwrap().into_artifact() else {
            panic!("expected FXD")
        };
        assert_eq!(restored.encode(), fxd);
        assert_eq!(restored.evaluate(&inputs), legacy.evaluate(&inputs));
        let count = if case == "guard-limit" { 200 } else { 10_000 };
        measure(
            case,
            count,
            reverse,
            &mut [
                ("legacy_decode", &mut || {
                    black_box(RuleProgram::decode(black_box(&fxd)).unwrap());
                }),
                ("fxh_decode", &mut || {
                    black_box(Hir::decode(black_box(&fxh)).unwrap().into_artifact());
                }),
                ("fxh_encode", &mut || {
                    black_box(black_box(&checked).encode().unwrap());
                }),
                ("legacy_execute", &mut || {
                    let _ = black_box(legacy.evaluate(black_box(&inputs)));
                }),
                ("restored_execute", &mut || {
                    let _ = black_box(restored.evaluate(black_box(&inputs)));
                }),
            ],
        );
    }
    let case = "fr1";
    let fxh = std::fs::read(directory.join("fr1.fxh")).unwrap();
    let fsm = std::fs::read(directory.join("fr1.fxr")).unwrap();
    let legacy = Module::decode(&fsm).unwrap();
    let checked = Hir::decode(&fxh).unwrap();
    let Artifact::Structured(restored) = Hir::decode(&fxh).unwrap().into_artifact() else {
        panic!("expected FSM")
    };
    assert_eq!(checked.encode().unwrap(), fxh);
    assert_eq!(restored.encode(), fsm);
    let arguments = [Value::U64(7)];
    let invocation = || Invocation {
        export: "total",
        inputs: &[],
        arguments: &arguments,
        limits: Limits::default(),
    };
    let [mut old, mut new] = std::array::from_fn(|_| Workspace::default());
    assert_eq!(
        legacy.run_portable_with(invocation(), &mut old),
        restored.run_portable_with(invocation(), &mut new)
    );
    measure(
        case,
        10_000,
        reverse,
        &mut [
            ("legacy_decode", &mut || {
                black_box(Module::decode(black_box(&fsm)).unwrap());
            }),
            ("fxh_decode", &mut || {
                black_box(Hir::decode(black_box(&fxh)).unwrap().into_artifact());
            }),
            ("fxh_encode", &mut || {
                black_box(black_box(&checked).encode().unwrap());
            }),
            ("legacy_execute", &mut || {
                let _ = black_box(legacy.run_portable_with(black_box(invocation()), &mut old));
            }),
            ("restored_execute", &mut || {
                let _ = black_box(restored.run_portable_with(black_box(invocation()), &mut new));
            }),
        ],
    );
}
