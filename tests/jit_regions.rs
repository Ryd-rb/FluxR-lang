//! Independent checks for exact JIT metering across straight-line regions.
//! Expected values use checked Rust arithmetic; the released portable VM is the
//! oracle for exact resource boundaries and first errors, not the JIT planner.
use fluxr::structured::{ExecutionRoute, Invocation, Limits, Module, Value, Workspace};
use std::sync::Arc;

const LOOPS: &str = "v2
    (rec R value:i64)
    (fn twist (x:i64) i64 = (+ (* x 3) 1))
    (fn count (n:i64) i64 (sum:i64) (= sum 0)
      (while (< 0 n) ((= sum (+ sum n)) (= n (- n 1)))) (ret sum))
    (pub direct (n:i64) i64 ()
      (while (< 0 n) ((= n (- n 1)))) (ret n))
    (pub called (n:i64) i64 (sum:i64) (= sum 0)
      (while (< 0 n) ((= sum (+ sum (call twist n))) (= n (- n 1)))) (ret sum))
    (pub looping_callee (n:i64) i64 = (+ (call count n) 1))
    (pub branched (n:i64) i64 (sum:i64) (= sum 0)
      (while (< 0 n) ((if (== (% n 2) 0) ((= sum (+ sum n))) ((= sum (- sum n))))
        (= n (- n 1)))) (ret sum))
    (pub nested (n:i64) i64 (i:i64 j:i64 sum:i64) (= i 0) (= sum 0)
      (while (< i n) ((= j 0)
        (while (< j i) ((= sum (+ sum j)) (= j (+ j 1))))
        (= i (+ i 1)))) (ret sum))
    (pub early (n:i64 stop:i64) i64 ()
      (while (< 0 n) ((if (== n stop) ((ret n)) ()) (= n (- n 1)))) (ret 0))
    (pub edit (s:R n:i64 delta:i64) R ()
      (while (< 0 n) ((= s.value (+ s.value delta)) (= n (- n 1)))) (ret s))";

fn assert_native(route: ExecutionRoute) {
    if cfg!(all(
        feature = "typed-jit",
        target_arch = "x86_64",
        any(
            target_os = "windows",
            all(target_os = "linux", target_env = "gnu")
        )
    )) {
        // Every export below contains a loop or calls one, so it cannot be
        // mistaken for the separate Windows signed scalar leaf specialization.
        assert_eq!(route, ExecutionRoute::TypedJit);
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
fn loop_regions_keep_joint_exact_budgets_native_with_branches_calls_and_nested_loops() {
    let module = Module::compile_compact(LOOPS).unwrap();
    let mut vm = Workspace::default();
    let mut workspace = Workspace::default();
    for export in [
        "direct",
        "called",
        "looping_callee",
        "branched",
        "nested",
        "early",
    ] {
        let prepared = module.prepare_call(export).unwrap();
        for n in 0i64..=7 {
            let mut args = vec![Value::I64(n)];
            let expected = match export {
                "direct" => 0,
                "called" => (1..=n)
                    .try_fold(0i64, |sum, i| {
                        sum.checked_add(i.checked_mul(3)?.checked_add(1)?)
                    })
                    .unwrap(),
                "looping_callee" => (1..=n).try_fold(1i64, |sum, i| sum.checked_add(i)).unwrap(),
                "branched" => (1..=n)
                    .try_fold(0i64, |sum, i| {
                        if i % 2 == 0 {
                            sum.checked_add(i)
                        } else {
                            sum.checked_sub(i)
                        }
                    })
                    .unwrap(),
                "nested" => (0..n)
                    .try_fold(0i64, |sum, i| {
                        (0..i).try_fold(sum, |sum, j| sum.checked_add(j))
                    })
                    .unwrap(),
                "early" => {
                    args.push(Value::I64(3));
                    if n >= 3 { 3 } else { 0 }
                }
                _ => unreachable!(),
            };
            let expected = Value::I64(expected);
            let exact = (0..=2_000)
                .find(|&instructions| {
                    portable(
                        &module,
                        export,
                        &args,
                        Limits {
                            instructions,
                            ..Limits::default()
                        },
                        &mut vm,
                    ) == Ok(expected.clone())
                })
                .expect("finite fixture must complete inside the independent search bound");
            // These scalar fixtures have no conservative record-copy charge.
            // Their exact portable cell threshold must also remain native,
            // including repeated and conditional calls inside the loops.
            let exact_cells = (0..=256)
                .find(|&cells| {
                    portable(
                        &module,
                        export,
                        &args,
                        Limits {
                            instructions: exact,
                            cells,
                            ..Limits::default()
                        },
                        &mut vm,
                    ) == Ok(expected.clone())
                })
                .expect("scalar fixture must fit the independent cell search bound");
            for instructions in [exact, exact + 1, exact + 19] {
                let limits = Limits {
                    instructions,
                    cells: exact_cells,
                    ..Limits::default()
                };
                let (actual, route) = prepared.run_with_route(&[], &args, limits, &mut workspace);
                assert_eq!(actual, Ok(expected.clone()), "{export} n={n} {limits:?}");
                assert_native(route);
            }
            let limits = Limits {
                instructions: exact - 1,
                ..Limits::default()
            };
            let (actual, route) = prepared.run_with_route(&[], &args, limits, &mut workspace);
            assert_eq!(actual, portable(&module, export, &args, limits, &mut vm));
            assert_eq!(
                actual.unwrap_err().to_string(),
                "module instruction budget exhausted"
            );
            assert_eq!(route, ExecutionRoute::Portable);
            let limits = Limits {
                instructions: exact,
                cells: exact_cells - 1,
                ..Limits::default()
            };
            let (actual, route) = prepared.run_with_route(&[], &args, limits, &mut workspace);
            assert_eq!(actual, portable(&module, export, &args, limits, &mut vm));
            assert_eq!(
                actual.unwrap_err().to_string(),
                "module cell budget exhausted"
            );
            assert_eq!(route, ExecutionRoute::Portable);
        }
    }
}

#[test]
fn region_reservations_preserve_competing_first_traps_at_every_small_budget() {
    let module = Module::compile_compact(
        "v2 (pub first (n:i64 missing_first:bool) i64 (sum:i64 missing:i64)
          (= sum 0) (while (< 0 n)
            ((if missing_first
               ((= sum (+ missing (/ n 0))))
               ((= sum (+ (/ n 0) missing))))
             (= n (- n 1)))) (ret sum))",
    )
    .unwrap();
    let prepared = module.prepare_call("first").unwrap();
    let mut vm = Workspace::default();
    let mut workspace = Workspace::default();
    for n in [0, 1, 2] {
        for missing_first in [false, true] {
            let args = [Value::I64(n), Value::Bool(missing_first)];
            let expected = if n == 0 {
                Ok(Value::I64(0))
            } else {
                Err(if missing_first {
                    "read of uninitialized local"
                } else {
                    "invalid or overflowing integer arithmetic"
                })
            };
            assert_eq!(
                prepared
                    .run(&[], &args, Limits::default())
                    .map_err(|e| e.to_string()),
                expected.map_err(str::to_owned)
            );
            for instructions in 0..=36 {
                for cells in 0..=10 {
                    for frames in 0..=2 {
                        let limits = Limits {
                            instructions,
                            cells,
                            frames,
                        };
                        let expected = portable(&module, "first", &args, limits, &mut vm);
                        let (actual, route) =
                            prepared.run_with_route(&[], &args, limits, &mut workspace);
                        assert_eq!(
                            actual, expected,
                            "n={n} missing_first={missing_first} {limits:?}"
                        );
                        if actual.is_err() {
                            assert_eq!(route, ExecutionRoute::Portable);
                        }
                    }
                }
            }
        }
    }
    // A prior decline or trap cannot contaminate a reused call/workspace.
    let (actual, route) = prepared.run_with_route(
        &[],
        &[Value::I64(0), Value::Bool(true)],
        Limits::default(),
        &mut workspace,
    );
    assert_eq!(actual, Ok(Value::I64(0)));
    assert_native(route);
}

#[test]
fn record_loop_regions_preserve_copy_costs_original_identity_and_failed_writes() {
    let module = Module::compile_compact(LOOPS).unwrap();
    let prepared = module.prepare_call("edit").unwrap();
    let original: Arc<[Value]> = [Value::I64(11)].into();
    let mut vm = Workspace::default();
    let mut workspace = Workspace::default();
    for n in 0..=3 {
        let args = [
            Value::Record(original.clone()),
            Value::I64(n),
            Value::I64(3),
        ];
        let (result, route) =
            prepared.run_with_route(&[], &args, Limits::default(), &mut workspace);
        assert_native(route);
        let Value::Record(fields) = result.unwrap() else {
            panic!("record required")
        };
        assert_eq!(&*fields, &[Value::I64(11 + n * 3)]);
        assert_eq!(Arc::ptr_eq(&fields, &original), n == 0);
        for instructions in 0..=48 {
            for cells in 0..=16 {
                for frames in 0..=2 {
                    let limits = Limits {
                        instructions,
                        cells,
                        frames,
                    };
                    assert_eq!(
                        prepared.run_with(&[], &args, limits, &mut workspace),
                        portable(&module, "edit", &args, limits, &mut vm),
                        "n={n} {limits:?}",
                    );
                    assert_eq!(&*original, &[Value::I64(11)]);
                }
            }
        }
    }
    let args = [
        Value::Record(original.clone()),
        Value::I64(2),
        Value::I64(i64::MAX),
    ];
    let (result, route) = prepared.run_with_route(&[], &args, Limits::default(), &mut workspace);
    assert_eq!(
        result.unwrap_err().to_string(),
        "invalid or overflowing integer arithmetic"
    );
    assert_eq!(route, ExecutionRoute::Portable);
    assert_eq!(&*original, &[Value::I64(11)]);
    // Both native execution and replay must release temporary ownership.
    drop(args);
    assert_eq!(Arc::strong_count(&original), 1);
}

#[test]
fn region_metering_bounds_extreme_loop_counts_and_retains_call_frame_limits() {
    let module = Module::compile_compact(LOOPS).unwrap();
    let mut vm = Workspace::default();
    let mut workspace = Workspace::default();
    for export in ["direct", "called", "looping_callee", "branched", "nested"] {
        let prepared = module.prepare_call(export).unwrap();
        let args = [Value::I64(i64::MAX)];
        for instructions in [1, 31, 127] {
            for frames in 0..=2 {
                let limits = Limits {
                    instructions,
                    frames,
                    ..Limits::default()
                };
                let expected = portable(&module, export, &args, limits, &mut vm);
                assert!(expected.is_err());
                let (actual, route) = prepared.run_with_route(&[], &args, limits, &mut workspace);
                assert_eq!(actual, expected, "{export} {limits:?}");
                assert_eq!(route, ExecutionRoute::Portable);
            }
        }
    }
}
