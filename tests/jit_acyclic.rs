//! Public-API regressions for conservative acyclic JIT admission.
//! Expected values and exact logical budgets are specified independently of the
//! native Plan. The portable evaluator remains the oracle for first failures.
use fluxr::structured::{ExecutionRoute, Invocation, Limits, Module, Value, Workspace};
use std::sync::Arc;

fn assert_native(route: ExecutionRoute) {
    if cfg!(all(
        feature = "typed-jit",
        target_os = "linux",
        target_env = "gnu",
        target_arch = "x86_64"
    )) {
        assert_eq!(route, ExecutionRoute::TypedJit);
    }
    if cfg!(all(
        feature = "typed-jit",
        target_os = "windows",
        target_arch = "x86_64"
    )) {
        assert!(matches!(
            route,
            ExecutionRoute::TypedJit | ExecutionRoute::NativeScalar
        ));
    }
}

fn portable(
    module: &Module,
    export: &str,
    arguments: &[Value],
    limits: Limits,
    workspace: &mut Workspace,
) -> Result<Value, fluxr::Error> {
    module.run_portable_with(
        Invocation {
            export,
            inputs: &[],
            arguments,
            limits,
        },
        workspace,
    )
}

#[test]
fn asymmetric_branch_replays_affordable_short_path_below_native_ceiling() {
    let module = Module::compile_compact(
        "v2
         (fn slow (x:i64) i64 = (+ (* x 3) 5))
         (pub choose (quick:bool x:i64) i64 ()
           (if quick ((ret x)) ((ret (call slow x)))))",
    )
    .unwrap();
    let prepared = module.prepare_call("choose").unwrap();
    // Short path: condition/branch/return = 4 fuel; two arguments, result,
    // two slots and one frame = 6 cells. Calling slow adds 7 fuel, 2 cells
    // and a second active frame. The conservative ceiling covers either arm.
    let ceiling = Limits {
        instructions: 11,
        cells: 8,
        frames: 2,
    };
    let mut vm = Workspace::default();
    let mut workspace = Workspace::default();
    for (quick, expected) in [(true, 7), (false, 26)] {
        let args = [Value::Bool(quick), Value::I64(7)];
        let (actual, route) = prepared.run_with_route(&[], &args, ceiling, &mut workspace);
        assert_eq!(actual, Ok(Value::I64(expected)));
        assert_eq!(actual, portable(&module, "choose", &args, ceiling, &mut vm));
        assert_native(route);
    }
    let args = [Value::Bool(true), Value::I64(7)];
    for limits in [
        Limits {
            instructions: 4,
            ..ceiling
        },
        Limits {
            cells: 6,
            ..ceiling
        },
        Limits {
            instructions: 4,
            cells: 6,
            frames: 1,
        },
    ] {
        let (actual, route) = prepared.run_with_route(&[], &args, limits, &mut workspace);
        assert_eq!(actual, Ok(Value::I64(7)), "{limits:?}");
        assert_eq!(actual, portable(&module, "choose", &args, limits, &mut vm));
        assert_eq!(route, ExecutionRoute::Portable);
    }
    for (limits, message) in [
        (
            Limits {
                instructions: 3,
                ..ceiling
            },
            "module instruction budget exhausted",
        ),
        (
            Limits {
                cells: 5,
                ..ceiling
            },
            "module cell budget exhausted",
        ),
        (
            Limits {
                frames: 0,
                ..ceiling
            },
            "module frame limit exceeded",
        ),
    ] {
        let (actual, route) = prepared.run_with_route(&[], &args, limits, &mut workspace);
        assert_eq!(actual, portable(&module, "choose", &args, limits, &mut vm));
        assert_eq!(actual.unwrap_err().to_string(), message);
        assert_eq!(route, ExecutionRoute::Portable);
    }
}

#[test]
fn repeated_nested_branch_calls_charge_every_invocation_at_exact_boundaries() {
    let module = Module::compile_compact(
        "v2
         (fn tweak (up:bool x:i64) i64 ()
           (if up ((ret (+ x 1))) ((ret (- x 1)))))
         (fn nested (up:bool x:i64) i64 = (call tweak up (call tweak up x)))
         (pub twice (up:bool x:i64) i64 =
           (+ (call nested up x) (call nested up x)))",
    )
    .unwrap();
    let prepared = module.prepare_call("twice").unwrap();
    // tweak costs 6 fuel and 3 frame/slot cells. nested costs 18 fuel and
    // 9 cells (two distinct tweak calls). twice costs 44 fuel and 24 total
    // cells including boundary values; the deepest active stack has 3 frames.
    let exact = Limits {
        instructions: 44,
        cells: 24,
        frames: 3,
    };
    let mut vm = Workspace::default();
    let mut workspace = Workspace::default();
    for up in [false, true] {
        for x in [-19i64, 0, 13] {
            let args = [Value::Bool(up), Value::I64(x)];
            let delta = if up { 2 } else { -2 };
            let expected = x.checked_add(delta).unwrap().checked_mul(2).unwrap();
            let (actual, route) = prepared.run_with_route(&[], &args, exact, &mut workspace);
            assert_eq!(actual, Ok(Value::I64(expected)));
            assert_eq!(actual, portable(&module, "twice", &args, exact, &mut vm));
            assert_native(route);
            for (limits, message) in [
                (
                    Limits {
                        instructions: 43,
                        ..exact
                    },
                    "module instruction budget exhausted",
                ),
                (
                    Limits { cells: 23, ..exact },
                    "module cell budget exhausted",
                ),
                (Limits { frames: 2, ..exact }, "module frame limit exceeded"),
            ] {
                let (actual, route) = prepared.run_with_route(&[], &args, limits, &mut workspace);
                assert_eq!(actual, portable(&module, "twice", &args, limits, &mut vm));
                assert_eq!(actual.unwrap_err().to_string(), message);
                assert_eq!(route, ExecutionRoute::Portable);
            }
        }
    }
}

#[test]
fn branch_updates_keep_conservative_copy_charges_and_original_record_identity() {
    let module = Module::compile_compact(
        "v2 (rec R left:i64 right:i64)
         (pub edit (s:R change:bool delta:i64) R ()
           (if change ((= s.left (+ s.left delta)) (= s.right (+ s.right 1))) ())
           (ret s))",
    )
    .unwrap();
    let prepared = module.prepare_call("edit").unwrap();
    let original: Arc<[Value]> = [Value::I64(17), Value::I64(-3)].into();
    let mut vm = Workspace::default();
    let mut workspace = Workspace::default();
    // Boundary + frame/slots = 12 cells. The portable evaluator copies the
    // shared two-field record once (14 total); native admission conservatively
    // reserves a full copy for each update (16 total).
    for change in [false, true] {
        let args = [
            Value::Record(original.clone()),
            Value::Bool(change),
            Value::I64(5),
        ];
        for limits in [
            Limits {
                instructions: 15,
                cells: 16,
                frames: 1,
            },
            Limits {
                instructions: if change { 15 } else { 4 },
                cells: if change { 14 } else { 12 },
                frames: 1,
            },
        ] {
            let expected = portable(&module, "edit", &args, limits, &mut vm);
            let (actual, route) = prepared.run_with_route(&[], &args, limits, &mut workspace);
            assert_eq!(actual, expected, "change={change} {limits:?}");
            let Value::Record(fields) = actual.unwrap() else {
                panic!("record required")
            };
            assert_eq!(Arc::ptr_eq(&fields, &original), !change);
            assert_eq!(
                &*fields,
                if change {
                    &[Value::I64(22), Value::I64(-2)]
                } else {
                    &*original
                }
            );
            if limits.cells == 16 {
                assert_native(route);
            } else {
                assert_eq!(route, ExecutionRoute::Portable);
            }
            assert_eq!(&*original, &[Value::I64(17), Value::I64(-3)]);
        }
        for limits in [
            Limits {
                instructions: if change { 14 } else { 3 },
                cells: 16,
                frames: 1,
            },
            Limits {
                instructions: 15,
                cells: if change { 13 } else { 11 },
                frames: 1,
            },
        ] {
            let expected = portable(&module, "edit", &args, limits, &mut vm);
            assert!(expected.is_err());
            assert_eq!(prepared.run(&[], &args, limits), expected);
            assert_eq!(&*original, &[Value::I64(17), Value::I64(-3)]);
        }
    }
    let overflow = [
        Value::Record(original.clone()),
        Value::Bool(true),
        Value::I64(i64::MAX),
    ];
    let (actual, route) =
        prepared.run_with_route(&[], &overflow, Limits::default(), &mut workspace);
    assert_eq!(
        actual.unwrap_err().to_string(),
        "invalid or overflowing integer arithmetic"
    );
    assert_eq!(route, ExecutionRoute::Portable);
    assert_eq!(&*original, &[Value::I64(17), Value::I64(-3)]);
}

#[test]
fn conservative_admission_preserves_branch_specific_first_errors() {
    let module = Module::compile_compact(
        "v2 (pub first (uninitialized:bool x:i64) i64 (missing:i64)
           (if uninitialized
             ((ret (+ missing (/ x 0))))
             ((ret (+ (/ x 0) missing)))))",
    )
    .unwrap();
    let prepared = module.prepare_call("first").unwrap();
    let mut vm = Workspace::default();
    let mut workspace = Workspace::default();
    for uninitialized in [false, true] {
        let args = [Value::Bool(uninitialized), Value::I64(37)];
        let expected_error = if uninitialized {
            "read of uninitialized local"
        } else {
            "invalid or overflowing integer arithmetic"
        };
        assert_eq!(
            prepared
                .run(&[], &args, Limits::default())
                .unwrap_err()
                .to_string(),
            expected_error
        );
        for instructions in 0..=12 {
            for cells in 0..=8 {
                for frames in 0..=2 {
                    let limits = Limits {
                        instructions,
                        cells,
                        frames,
                    };
                    let expected = portable(&module, "first", &args, limits, &mut vm);
                    let (actual, route) =
                        prepared.run_with_route(&[], &args, limits, &mut workspace);
                    assert_eq!(actual, expected, "uninitialized={uninitialized} {limits:?}");
                    assert_eq!(route, ExecutionRoute::Portable);
                }
            }
        }
    }
}

#[test]
fn cyclic_exports_and_loop_callees_keep_dynamic_native_metering() {
    let module = Module::compile_compact(
        "v2
         (fn countdown (n:i64) i64 ()
           (while (< 0 n) ((= n (- n 1)))) (ret n))
         (pub direct (n:i64) i64 ()
           (while (< 0 n) ((= n (- n 1)))) (ret n))
         (pub outer (n:i64) i64 = (call countdown n))",
    )
    .unwrap();
    let mut vm = Workspace::default();
    let mut workspace = Workspace::default();
    for (export, extra_fuel, cells, frames) in [("direct", 0, 4, 1), ("outer", 3, 6, 2)] {
        let prepared = module.prepare_call(export).unwrap();
        for n in [0i64, 1, 2, 5] {
            // Each iteration adds condition (4), update (4), backedge (1).
            // The final false condition and return cost 6 fuel.
            let limits = Limits {
                instructions: 6 + 9 * n as u64 + extra_fuel,
                cells,
                frames,
            };
            let args = [Value::I64(n)];
            let (actual, route) = prepared.run_with_route(&[], &args, limits, &mut workspace);
            assert_eq!(actual, Ok(Value::I64(0)));
            assert_eq!(actual, portable(&module, export, &args, limits, &mut vm));
            assert_native(route);
            let short = Limits {
                instructions: limits.instructions - 1,
                ..limits
            };
            let (actual, route) = prepared.run_with_route(&[], &args, short, &mut workspace);
            assert_eq!(actual, portable(&module, export, &args, short, &mut vm));
            assert_eq!(
                actual.unwrap_err().to_string(),
                "module instruction budget exhausted"
            );
            assert_eq!(route, ExecutionRoute::Portable);
        }
        // A finite fuel limit must terminate an otherwise extremely long loop;
        // discovering a cycle cannot disable metering in the export or callee.
        let args = [Value::I64(i64::MAX)];
        let limits = Limits {
            instructions: 31,
            cells,
            frames,
        };
        let (actual, route) = prepared.run_with_route(&[], &args, limits, &mut workspace);
        assert_eq!(actual, portable(&module, export, &args, limits, &mut vm));
        assert_eq!(
            actual.unwrap_err().to_string(),
            "module instruction budget exhausted"
        );
        assert_eq!(route, ExecutionRoute::Portable);
    }
}

#[test]
fn jit_admission_does_not_rewrite_released_aot_output() {
    let module =
        Module::compile_compact(include_str!("../examples/fixtures/control_flow.fluxr")).unwrap();
    assert_eq!(
        module.emit_native_rust("walk").unwrap(),
        include_str!("../examples/fixtures/control_flow_aot.rs")
    );
    // Preparing a JIT owner must not alter shared portable artifacts or the
    // released AOT metering strategy on subsequent emission.
    let artifact = module.encode();
    let _prepared = module.prepare_call("walk").unwrap();
    assert_eq!(module.encode(), artifact);
    assert_eq!(
        module.emit_native_rust("walk").unwrap(),
        include_str!("../examples/fixtures/control_flow_aot.rs")
    );

    // Acyclic branches receive stronger admission only in the JIT owner.
    // Their released AOT kernels must still meter blocks under the old ABI.
    let branch =
        Module::compile_compact(include_str!("../examples/fixtures/host_decision.flux")).unwrap();
    let branch_aot = branch.emit_native_rust("decide").unwrap();
    assert!(branch_aot.contains("*fuel = fuel.checked_sub("));
    assert!(branch_aot.contains("*cells = cells.checked_sub("));
    let branch_artifact = branch.encode();
    let _branch_prepared = branch.prepare_call("decide").unwrap();
    assert_eq!(branch.encode(), branch_artifact);
    assert_eq!(branch.emit_native_rust("decide").unwrap(), branch_aot);
}
