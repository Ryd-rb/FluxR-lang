//! Independent public-API oracle for prepared/native calls.
//! Expectations come from checked Rust arithmetic, the released portable
//! evaluator, and the retained identity/first-trap contracts. This test does
//! not construct or inspect the author's lowered Plan or emitted instructions.
use fluxr::structured::{ExecutionRoute, Invocation, Limits, Module, ModuleSlot, Value, Workspace};
use std::sync::Arc;

fn assert_linux_machine_route(route: ExecutionRoute) {
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

#[test]
fn independent_checked_arithmetic_and_success_route() {
    let edge = [
        i64::MIN,
        i64::MIN + 1,
        -97,
        -1,
        0,
        1,
        97,
        i64::MAX - 1,
        i64::MAX,
    ];
    let module = Module::compile_compact(
        "v2 (pub mixed (x:i64 y:i64 z:i64) i64 = (+ (* x y) z))
         (pub quotient (x:i64 y:i64 z:i64) i64 = (+ (/ x y) z))
         (pub remainder (x:i64 y:i64 z:i64) i64 = (- (% x y) z))",
    )
    .unwrap();
    let mut checked = 0usize;
    let mut machine_successes = 0usize;
    for export in ["mixed", "quotient", "remainder"] {
        let prepared = module.prepare_call(export).unwrap();
        let mut workspace = Workspace::default();
        for x in edge {
            for y in edge {
                for z in edge {
                    let expected = match export {
                        "mixed" => x.checked_mul(y).and_then(|a| a.checked_add(z)),
                        "quotient" => x.checked_div(y).and_then(|a| a.checked_add(z)),
                        "remainder" => x.checked_rem(y).and_then(|a| a.checked_sub(z)),
                        _ => unreachable!(),
                    };
                    let args = [Value::I64(x), Value::I64(y), Value::I64(z)];
                    let (actual, route) =
                        prepared.run_with_route(&[], &args, Limits::default(), &mut workspace);
                    if let Some(value) = expected {
                        assert_eq!(actual.unwrap(), Value::I64(value), "{export} {x} {y} {z}");
                        assert_linux_machine_route(route);
                        if route == ExecutionRoute::TypedJit {
                            machine_successes += 1;
                        }
                    } else {
                        assert_eq!(
                            actual.unwrap_err().to_string(),
                            "invalid or overflowing integer arithmetic"
                        );
                        if cfg!(all(
                            feature = "typed-jit",
                            target_os = "linux",
                            target_env = "gnu",
                            target_arch = "x86_64"
                        )) {
                            assert_eq!(
                                route,
                                ExecutionRoute::Portable,
                                "native decline must be visible"
                            );
                        }
                    }
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, 2187);
    eprintln!("independent_arithmetic cases={checked} typed_jit_successes={machine_successes}");
}

#[test]
fn independent_exhaustive_small_budgets_and_first_traps() {
    let module = Module::compile_compact(
        "v2
         (pub arithmetic_first (x:i64) i64 (a:i64 b:i64) (= a (/ x 0)) (ret b))
         (pub uninitialized_first (x:i64) i64 (a:i64) (ret (+ a (/ x 0))))
         (pub untaken_trap (choose:bool x:i64) i64 ()
           (if choose ((ret x)) ((ret (/ x 0)))))",
    )
    .unwrap();
    let cases = [
        (
            "arithmetic_first",
            vec![Value::I64(7)],
            "invalid or overflowing integer arithmetic",
        ),
        (
            "uninitialized_first",
            vec![Value::I64(7)],
            "read of uninitialized local",
        ),
        (
            "untaken_trap",
            vec![Value::Bool(false), Value::I64(7)],
            "invalid or overflowing integer arithmetic",
        ),
    ];
    let mut comparisons = 0;
    for (export, args, first_error) in cases {
        let prepared = module.prepare_call(export).unwrap();
        assert_eq!(
            prepared
                .run(&[], &args, Limits::default())
                .unwrap_err()
                .to_string(),
            first_error
        );
        let mut ordinary = Workspace::default();
        let mut prepared_workspace = Workspace::default();
        for instructions in 0..=15 {
            for cells in 0..=12 {
                for frames in 0..=3 {
                    let limits = Limits {
                        instructions,
                        cells,
                        frames,
                    };
                    let expected = module.run_portable_with(
                        Invocation {
                            export,
                            inputs: &[],
                            arguments: &args,
                            limits,
                        },
                        &mut ordinary,
                    );
                    let (actual, route) =
                        prepared.run_with_route(&[], &args, limits, &mut prepared_workspace);
                    assert_eq!(actual, expected, "{export} {limits:?}");
                    if cfg!(all(
                        feature = "typed-jit",
                        target_os = "linux",
                        target_env = "gnu",
                        target_arch = "x86_64"
                    )) {
                        assert_eq!(route, ExecutionRoute::Portable);
                    }
                    comparisons += 1;
                }
            }
        }
    }
    let prepared = module.prepare_call("untaken_trap").unwrap();
    let (value, route) = prepared.run_with_route(
        &[],
        &[Value::Bool(true), Value::I64(37)],
        Limits::default(),
        &mut Workspace::default(),
    );
    assert_eq!(value.unwrap(), Value::I64(37));
    assert_linux_machine_route(route);
    assert_eq!(comparisons, 2496);
    eprintln!("independent_budget_first_trap comparisons={comparisons}");
}

#[test]
fn independent_host_shapes_and_counts_are_checked_before_native_loads() {
    let module =
        Module::compile_compact("v2 (inputs offset:i64) (pub add (x:i64) i64 = (+ x $offset))")
            .unwrap();
    assert!(module.prepare_call("missing").is_err());
    let prepared = module.prepare_call("add").unwrap();
    let cases = [
        (vec![], vec![Value::I64(1)]),
        (vec![Value::I64(1)], vec![]),
        (vec![Value::I64(1)], vec![Value::I64(1), Value::I64(2)]),
        (vec![Value::I64(1), Value::I64(2)], vec![Value::I64(1)]),
        (vec![Value::Bool(true)], vec![Value::I64(1)]),
        (vec![Value::I64(1)], vec![Value::Bool(true)]),
    ];
    for (inputs, args) in cases {
        for limits in [
            Limits::default(),
            Limits {
                instructions: 0,
                cells: 0,
                frames: 0,
            },
        ] {
            let expected = module.run_portable_with(
                Invocation {
                    export: "add",
                    inputs: &inputs,
                    arguments: &args,
                    limits,
                },
                &mut Workspace::default(),
            );
            let (actual, route) =
                prepared.run_with_route(&inputs, &args, limits, &mut Workspace::default());
            assert!(actual.is_err());
            assert_eq!(actual, expected);
            assert_eq!(route, ExecutionRoute::Portable);
        }
    }
}

#[test]
fn independent_record_identity_cow_and_immutable_revision_binding() {
    let module = Module::compile_compact(
        "v2 (rec R a:i64 b:i64 c:i64 d:i64)
         (pub keep (s:R delta:i64) R = s)
         (pub change (s:R delta:i64) R () (= s.d (+ s.d delta)) (ret s))",
    )
    .unwrap();
    let original: Arc<[Value]> =
        [Value::I64(2), Value::I64(3), Value::I64(5), Value::I64(7)].into();
    for export in ["keep", "change"] {
        let prepared = module.prepare_call(export).unwrap();
        let args = [Value::Record(original.clone()), Value::I64(11)];
        let (result, route) =
            prepared.run_with_route(&[], &args, Limits::default(), &mut Workspace::default());
        assert_linux_machine_route(route);
        let Value::Record(fields) = result.unwrap() else {
            panic!("record required");
        };
        assert_eq!(Arc::ptr_eq(&fields, &original), export == "keep");
        assert_eq!(fields[3], Value::I64(if export == "keep" { 7 } else { 18 }));
        assert_eq!(original[3], Value::I64(7));
    }
    let old = Module::compile_compact("v2 (pub revision (x:i64) i64 = (+ x 1))").unwrap();
    let old_call = old.prepare_call("revision").unwrap();
    let mut slot = ModuleSlot::new(old.clone(), &[]).unwrap();
    let new = Module::compile_compact("v2 (pub revision (x:i64) i64 = (+ x 2))").unwrap();
    slot.inject(0, new, None).unwrap();
    let args = [Value::I64(40)];
    assert_eq!(
        slot.run(Invocation {
            export: "revision",
            inputs: &[],
            arguments: &args,
            limits: Limits::default()
        })
        .unwrap(),
        Value::I64(42)
    );
    assert_eq!(
        old_call.run(&[], &args, Limits::default()).unwrap(),
        Value::I64(41)
    );
}

#[test]
fn independent_shared_immutable_prepared_execution() {
    let module =
        Module::compile_compact("v2 (pub calc (x:i64 y:i64) i64 = (- (* x 3) y))").unwrap();
    let prepared = module.prepare_call("calc").unwrap();
    std::thread::scope(|scope| {
        for thread in 0..4i64 {
            let prepared = &prepared;
            scope.spawn(move || {
                let mut workspace = Workspace::default();
                for item in 0..64i64 {
                    let x = thread * 64 + item;
                    let args = [Value::I64(x), Value::I64(thread)];
                    let (actual, route) =
                        prepared.run_with_route(&[], &args, Limits::default(), &mut workspace);
                    assert_eq!(actual.unwrap(), Value::I64(x * 3 - thread));
                    assert_linux_machine_route(route);
                }
            });
        }
    });
}

#[test]
fn independent_unsupported_float_bits_remain_portable() {
    let module = Module::compile_compact("v2 (pub fp (x:f32) f32 = x)").unwrap();
    let prepared = module.prepare_call("fp").unwrap();
    for bits in [
        0,
        0x8000_0000,
        1,
        0x007f_ffff,
        0x0080_0000,
        0x7f80_0000,
        0xff80_0000,
        0x7fc0_1234,
    ] {
        let value = Value::F32(bits);
        let (actual, route) = prepared.run_with_route(
            &[],
            std::slice::from_ref(&value),
            Limits::default(),
            &mut Workspace::default(),
        );
        assert_eq!(actual.unwrap(), value);
        assert_eq!(route, ExecutionRoute::Portable);
    }
}

#[test]
fn independent_failed_record_update_preserves_alias_and_first_failure() {
    let module = Module::compile_compact(
        "v2 (rec R a:i64 b:i64)
         (pub update (s:R delta:i64) R ()
           (= s.a (+ s.a delta)) (= s.b (/ s.b 0)) (ret s))",
    )
    .unwrap();
    let prepared = module.prepare_call("update").unwrap();
    let original: Arc<[Value]> = [Value::I64(i64::MAX), Value::I64(17)].into();
    for delta in [0, 1] {
        let args = [Value::Record(original.clone()), Value::I64(delta)];
        let high_error = prepared.run(&[], &args, Limits::default()).unwrap_err();
        assert_eq!(
            high_error.to_string(),
            "invalid or overflowing integer arithmetic"
        );
        for instructions in 0..=14 {
            for cells in 0..=18 {
                let limits = Limits {
                    instructions,
                    cells,
                    frames: 2,
                };
                let expected = module.run_portable_with(
                    Invocation {
                        export: "update",
                        inputs: &[],
                        arguments: &args,
                        limits,
                    },
                    &mut Workspace::default(),
                );
                let actual = prepared.run(&[], &args, limits);
                assert_eq!(actual, expected, "delta={delta} {limits:?}");
                assert_eq!(&*original, &[Value::I64(i64::MAX), Value::I64(17)]);
            }
        }
    }
    eprintln!("independent_record_failure comparisons=570 original_alias_unchanged=true");
}

#[test]
fn independent_initialization_guards_on_zero_trip_and_backedges() {
    let module = Module::compile_compact(
        "v2
         (pub loop_initialized (n:i64) i64 (i:i64 y:i64)
           (= i 0) (while (< i n) ((= y i) (= i (+ i 1)))) (ret y))
         (pub read_before_store (n:i64) i64 (i:i64 y:i64)
           (= i 0) (while (< i n) ((= y (+ y 1)) (= i (+ i 1)))) (ret 17))
         (pub both_arms (choose:bool) i64 (y:i64)
           (if choose ((= y 11)) ((= y 23))) (ret y))",
    )
    .unwrap();
    let mut comparisons = 0;
    for export in ["loop_initialized", "read_before_store"] {
        let prepared = module.prepare_call(export).unwrap();
        for n in [-1, 0, 1, 2, 7] {
            let args = [Value::I64(n)];
            let (result, route) =
                prepared.run_with_route(&[], &args, Limits::default(), &mut Workspace::default());
            let succeeds = if export == "loop_initialized" {
                n > 0
            } else {
                n <= 0
            };
            if succeeds {
                assert_eq!(
                    result.unwrap(),
                    Value::I64(if export == "loop_initialized" {
                        n - 1
                    } else {
                        17
                    })
                );
                assert_linux_machine_route(route);
            } else {
                assert_eq!(
                    result.unwrap_err().to_string(),
                    "read of uninitialized local"
                );
                assert_eq!(route, ExecutionRoute::Portable);
            }
            for instructions in 0..=95 {
                let limits = Limits {
                    instructions,
                    cells: 20,
                    frames: 2,
                };
                let expected = module.run_portable_with(
                    Invocation {
                        export,
                        inputs: &[],
                        arguments: &args,
                        limits,
                    },
                    &mut Workspace::default(),
                );
                assert_eq!(
                    prepared.run(&[], &args, limits),
                    expected,
                    "{export} n={n} instructions={instructions}"
                );
                comparisons += 1;
            }
        }
    }
    let prepared = module.prepare_call("both_arms").unwrap();
    for (choose, expected) in [(false, 23), (true, 11)] {
        let (result, route) = prepared.run_with_route(
            &[],
            &[Value::Bool(choose)],
            Limits::default(),
            &mut Workspace::default(),
        );
        assert_eq!(result.unwrap(), Value::I64(expected));
        assert_linux_machine_route(route);
    }
    assert_eq!(comparisons, 960);
    eprintln!("independent_initialization comparisons={comparisons}");
}

#[test]
fn independent_punctuation_free_agent_source_reaches_prepared_native_call() {
    let source = "fa1 v2\npub calc 2 x int y int int ret rem add mul x y 17 997";
    assert!(
        source
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c.is_ascii_whitespace())
    );
    let module = Module::compile_agent(source).unwrap();
    let compact =
        Module::compile_compact("v2 (pub calc (x:i64 y:i64) i64 = (% (+ (* x y) 17) 997))")
            .unwrap();
    assert_eq!(module.encode(), compact.encode());
    let prepared = module.prepare_call("calc").unwrap();
    let args = [Value::I64(11), Value::I64(7)];
    let (result, route) =
        prepared.run_with_route(&[], &args, Limits::default(), &mut Workspace::default());
    assert_eq!(result.unwrap(), Value::I64(94));
    assert_linux_machine_route(route);
    eprintln!("independent_agent_source result=94 route={route:?}");
}
