use fluxr::structured::{Binding, Invocation, Limits, Module, ModuleSlot, Type, Value, Workspace};
use std::sync::Arc;

const VOLUME: &str = include_str!("../examples/bench_volume.flux");
const COUNTER: &str = include_str!("fixtures/structured_counter.fluxm");
const TYPED_JIT: bool = cfg!(all(
    feature = "typed-jit",
    target_os = "windows",
    target_arch = "x86_64"
));

#[path = "../examples/fixtures/bench_counter_native.rs"]
mod counter_aot;

fn tick<'a>(inputs: &'a [Value], arguments: &'a [Value], limits: Limits) -> Invocation<'a> {
    Invocation {
        export: "tick",
        inputs,
        arguments,
        limits,
    }
}

fn counter_grant() -> [Binding; 1] {
    [Binding {
        name: "bonus".into(),
        ty: Type::I64,
    }]
}

#[test]
fn typed_record_preserves_values_at_both_scratch_widths() {
    for (fields, last, width) in [
        ("a:i64 b:i64 c:i64", "c", 3),
        ("a:i64 b:i64 c:i64 d:i64", "d", 4),
    ] {
        let source = format!(
            "v2 (rec R {fields}) (pub bump (s:R) R () (= s.{last} (+ s.{last} 3)) (ret s))"
        );
        let module = Module::compile_compact(&source).unwrap();
        assert_eq!(module.prepare_native("bump"), TYPED_JIT);
        let mut workspace = Workspace::default();
        for last_value in [10, i64::MAX] {
            let mut fields = vec![Value::I64(-2); width];
            fields[width - 1] = Value::I64(last_value);
            let arguments = [Value::Record(fields.into())];
            let call = || Invocation {
                export: "bump",
                inputs: &[],
                arguments: &arguments,
                limits: Limits::default(),
            };
            let portable = module.run_portable_with(call(), &mut workspace);
            assert_eq!(module.run_with(call(), &mut workspace), portable);
            assert_eq!(portable.is_ok(), last_value == 10);
        }
    }
}

#[test]
fn volume_matches_portable_with_helper_calls_and_budget_precedence() {
    let module = Module::compile_compact(VOLUME).unwrap();
    assert_eq!(module.prepare_native("volume"), TYPED_JIT);
    let mut vm = Workspace::default();
    let ceiling = module.native_ceiling("volume", 11).unwrap();
    let mut samples = vec![
        [i64::MIN, i64::MAX, 0, 0, 0, 0, 0, 0, 1, 0, 0],
        [0, i64::MAX, 0, 0, 0, 0, 0, 0, 2, 0, 0],
        [0, i64::MIN, 0, 0, 0, 0, 0, 0, -1, 0, 0],
    ];
    samples.extend(
        (0..64).map(|i| std::array::from_fn(|j| ((i + 13) * (j as i64 + 17)) % 1025 - 512)),
    );
    for sample in samples {
        let arguments = sample.map(Value::I64);
        for limits in [
            Limits::default(),
            ceiling,
            Limits {
                cells: ceiling.cells - 1,
                ..ceiling
            },
            Limits {
                frames: ceiling.frames - 1,
                ..ceiling
            },
            Limits {
                instructions: ceiling.instructions - 1,
                ..ceiling
            },
            Limits {
                instructions: 8,
                ..ceiling
            },
        ] {
            let call = || Invocation {
                export: "volume",
                inputs: &[],
                arguments: &arguments,
                limits,
            };
            assert_eq!(
                module.run(call()),
                module.run_portable_with(call(), &mut vm)
            );
        }
    }
    let mut wrong = [0; 11].map(Value::I64);
    wrong[10] = Value::Bool(true);
    let call = || Invocation {
        export: "volume",
        inputs: &[],
        arguments: &wrong,
        limits: ceiling,
    };
    assert_eq!(
        module.run(call()),
        module.run_portable_with(call(), &mut vm)
    );
    assert!(Module::compile_compact(&VOLUME.replace("near:i64", "near:bool")).is_err());
    let bytes = module.encode();
    assert_eq!(Module::decode(&bytes).unwrap().encode(), bytes);
}

#[test]
fn counter_aot_and_live_match_exact_fuel_cell_frame_and_snapshot_behavior() {
    let module = Module::compile_legacy_json(COUNTER).unwrap();
    assert_eq!(module.prepare_native("tick"), TYPED_JIT);
    let aot = counter_aot::prepare(&module).unwrap();
    let mut workspace = Workspace::default();
    let snapshot = [Value::I64(3)];
    let state: Arc<[Value]> = [Value::I64(10)].into();
    for count in [-2, 0, 1, 2, 3, 6] {
        let args = [Value::Record(state.clone()), Value::I64(count)];
        for limits in [Limits::default()]
            .into_iter()
            .chain((0..=30).map(|cells| Limits {
                cells,
                ..Limits::default()
            }))
            .chain((0..=4).map(|frames| Limits {
                frames,
                ..Limits::default()
            }))
            .chain((0..=175).map(|instructions| Limits {
                instructions,
                ..Limits::default()
            }))
        {
            let call = || tick(&snapshot, &args, limits);
            let expected = module.run_portable_with(call(), &mut workspace);
            for actual in [
                module.run_with(call(), &mut workspace),
                aot.run_with(call(), &mut workspace),
            ] {
                assert_eq!(actual, expected, "count {count}, limits {limits:?}");
                if let Ok(Value::Record(fields)) = actual {
                    assert_eq!(Arc::ptr_eq(&fields, &state), count <= 0);
                }
            }
        }
    }
    assert_eq!(&*state, &[Value::I64(10)]);
    let args = [Value::Record([Value::I64(i64::MAX)].into()), Value::I64(3)];
    for cells in 0..20 {
        for instructions in 0..45 {
            let limits = Limits {
                cells,
                instructions,
                frames: 3,
            };
            let call = || tick(&snapshot, &args, limits);
            let expected = module.run_portable_with(call(), &mut workspace);
            assert_eq!(module.run_with(call(), &mut workspace), expected);
            assert_eq!(aot.run_with(call(), &mut workspace), expected);
        }
    }
    let invalid = [Value::Record([Value::Bool(false)].into()), Value::I64(3)];
    let call = || tick(&snapshot, &invalid, Limits::default());
    let expected = module.run_portable_with(call(), &mut workspace);
    assert_eq!(module.run(call()), expected);
    assert_eq!(aot.run_with(call(), &mut workspace), expected);
}

#[test]
fn counter_injection_preserves_grant_and_rejects_bad_probe() {
    let original = Module::compile_legacy_json(COUNTER).unwrap();
    assert!(ModuleSlot::new(original.clone(), &[]).is_err());
    let mut slot = ModuleSlot::new(original.clone(), &counter_grant()).unwrap();
    let snapshot = [Value::I64(3)];
    let args = [Value::Record([Value::I64(10)].into()), Value::I64(3)];
    let bad = [Value::Record([Value::I64(i64::MAX)].into()), Value::I64(3)];
    let call = || tick(&snapshot, &args, Limits::default());
    let compact = original.compact_text().unwrap();
    let edit = compact.replace("(ret(* i 2))", "(ret(* i 4))");
    assert_ne!(compact, edit);
    let candidate = Module::compile_compact(&edit).unwrap();
    assert!(counter_aot::prepare(&candidate).is_err());
    assert!(
        slot.inject(
            0,
            candidate.clone(),
            Some(tick(&snapshot, &bad, Limits::default()))
        )
        .is_err()
    );
    assert_eq!(slot.revision(), 0);
    assert_eq!(
        slot.run(call()).unwrap(),
        Value::Record([Value::I64(18)].into())
    );
    assert_eq!(slot.inject(0, candidate, Some(call())).unwrap(), 1);
    assert_eq!(
        slot.run(call()).unwrap(),
        Value::Record([Value::I64(20)].into())
    );
    assert_eq!(
        original.run(call()).unwrap(),
        Value::Record([Value::I64(18)].into())
    );
}

#[test]
fn signed_division_and_remainder_decline_unsafe_operands() {
    let mut workspace = Workspace::default();
    for operator in ["/", "%"] {
        let module = Module::compile_compact(&format!(
            "v2\n(pub checked (x:i64 y:i64) i64 (r:i64) (= r ({operator} x y)) (ret r))",
        ))
        .unwrap();
        assert_eq!(module.prepare_native("checked"), TYPED_JIT);
        let need = module.native_ceiling("checked", 2).unwrap();
        for x in [i64::MIN, i64::MIN + 1, -9, -1, 0, 1, 9, i64::MAX] {
            for y in [i64::MIN, -3, -1, 0, 1, 3, i64::MAX] {
                let args = [Value::I64(x), Value::I64(y)];
                for instructions in 0..=need.instructions {
                    let call = || Invocation {
                        export: "checked",
                        inputs: &[],
                        arguments: &args,
                        limits: Limits {
                            instructions,
                            ..need
                        },
                    };
                    assert_eq!(
                        module.run(call()),
                        module.run_portable_with(call(), &mut workspace)
                    );
                }
            }
        }
    }
}

#[test]
fn uninitialized_local_replays_and_recursive_export_declines() {
    let module = Module::compile_compact(
        "v2\n(pub local (x:i64) i64 (y:i64) (if (< x 0) ((= y 1)) ()) (ret y))\n(pub recurse (x:i64) i64 () (ret (call recurse x)))",
    ).unwrap();
    assert_eq!(module.prepare_native("local"), TYPED_JIT);
    assert!(!module.prepare_native("recurse"));
    assert!(module.emit_native_rust("recurse").is_err());
    let mut workspace = Workspace::default();
    for x in [-1, 0] {
        let args = [Value::I64(x)];
        let call = || Invocation {
            export: "local",
            inputs: &[],
            arguments: &args,
            limits: Limits::default(),
        };
        assert_eq!(
            module.run(call()),
            module.run_portable_with(call(), &mut workspace)
        );
    }
}
