//! Public-API checks for fixed-bound scalar entry. Expected values and
//! exact budgets are derived from the source operations, independently of the
//! native Plan. The same tests exercise portable builds and generic boundaries.
use fluxr::structured::{ExecutionRoute, Invocation, Limits, Module, Value, Workspace};

fn assert_native(route: ExecutionRoute) {
    if cfg!(all(
        feature = "typed-jit",
        target_arch = "x86_64",
        any(
            target_os = "windows",
            all(target_os = "linux", target_env = "gnu")
        )
    )) {
        assert_eq!(route, ExecutionRoute::TypedJit);
    }
}

fn portable(
    module: &Module,
    inputs: &[Value],
    arguments: &[Value],
    limits: Limits,
    workspace: &mut Workspace,
) -> Result<Value, fluxr::Error> {
    module.run_portable_with(
        Invocation {
            export: "run",
            inputs,
            arguments,
            limits,
        },
        workspace,
    )
}

#[test]
fn every_scalar_arity_and_input_argument_partition_preserves_values_and_exact_limits() {
    let numbers = [11i64, -7, 5, 3, -2];
    // Five lanes deliberately cross the specialized ABI boundary and exercise
    // the generic entry alongside all zero-through-four-lane partitions.
    for count in 0..=numbers.len() {
        for input_count in 0..=count {
            let inputs_text = (0..input_count)
                .map(|i| format!(" x{i}:i64"))
                .collect::<String>();
            let arguments_text = (input_count..count)
                .map(|i| format!(" x{i}:i64"))
                .collect::<String>();
            let input_declaration = if input_count == 0 {
                String::new()
            } else {
                format!("(inputs{inputs_text})")
            };
            let mut terms = (0..count).map(|i| {
                let prefix = if i < input_count { "$" } else { "" };
                format!("(* {prefix}x{i} {})", i + 1)
            });
            let first = terms.next().unwrap_or_else(|| "7".to_owned());
            let expression = terms.fold(first, |sum, term| format!("(+ {sum} {term})"));
            // An initialized local keeps the older Win64 scalar leaf emitter
            // from preempting this test of the typed JIT's new ABI.
            let source = format!(
                "v2 {input_declaration} (pub run ({arguments_text}) i64 (answer:i64) (= answer {expression}) (ret answer))"
            );
            let module = Module::compile_compact(&source).unwrap();
            let prepared = module.prepare_call("run").unwrap();
            let values: Vec<_> = numbers[..count].iter().copied().map(Value::I64).collect();
            let (inputs, arguments) = values.split_at(input_count);
            let expected = if count == 0 {
                7
            } else {
                numbers[..count]
                    .iter()
                    .enumerate()
                    .try_fold(0i64, |sum, (i, value)| {
                        sum.checked_add(value.checked_mul((i + 1) as i64)?)
                    })
                    .unwrap()
            };
            // Each weighted term costs three instructions, each connecting
            // addition one, and store/load/return three. The constant-only
            // case costs four. There is one extra local slot in the frame.
            let exact = Limits {
                instructions: if count == 0 {
                    4
                } else {
                    (count * 4 + 2) as u64
                },
                cells: count + arguments.len() + 3,
                frames: 1,
            };
            let mut workspace = Workspace::default();
            let mut oracle = Workspace::default();
            for limits in [exact, Limits::default()] {
                let (actual, route) =
                    prepared.run_with_route(inputs, arguments, limits, &mut workspace);
                assert_eq!(actual, Ok(Value::I64(expected)), "{source}");
                assert_eq!(
                    actual,
                    portable(&module, inputs, arguments, limits, &mut oracle)
                );
                assert_native(route);
            }
            for limits in [
                Limits {
                    instructions: exact.instructions - 1,
                    ..exact
                },
                Limits {
                    cells: exact.cells - 1,
                    ..exact
                },
                Limits { frames: 0, ..exact },
            ] {
                let expected = portable(&module, inputs, arguments, limits, &mut oracle);
                assert!(expected.is_err());
                let (actual, route) =
                    prepared.run_with_route(inputs, arguments, limits, &mut workspace);
                assert_eq!(actual, expected, "{source} {limits:?}");
                assert_eq!(route, ExecutionRoute::Portable);
            }
            for index in 0..count {
                let mut malformed = values.clone();
                malformed[index] = Value::Bool(true);
                let (inputs, arguments) = malformed.split_at(input_count);
                for limits in [
                    Limits::default(),
                    Limits {
                        instructions: 0,
                        cells: 0,
                        frames: 0,
                    },
                ] {
                    let expected = portable(&module, inputs, arguments, limits, &mut oracle);
                    let (actual, route) =
                        prepared.run_with_route(inputs, arguments, limits, &mut workspace);
                    assert!(expected.is_err());
                    assert_eq!(actual, expected, "malformed lane {index}: {source}");
                    assert_eq!(route, ExecutionRoute::Portable);
                }
            }
            let mut extra = arguments.to_vec();
            extra.push(Value::I64(99));
            let (actual, route) =
                prepared.run_with_route(inputs, &extra, Limits::default(), &mut workspace);
            assert_eq!(
                actual,
                portable(&module, inputs, &extra, Limits::default(), &mut oracle)
            );
            assert!(actual.is_err());
            assert_eq!(route, ExecutionRoute::Portable);
            // A failed admission must not retain scratch or contaminate reuse.
            assert_eq!(
                prepared.run_with(inputs, arguments, exact, &mut workspace),
                Ok(Value::I64(expected))
            );
        }
    }
}

#[test]
fn scalar_branch_ceiling_retains_affordable_path_and_first_trap_order() {
    let module = Module::compile_compact(
        "v2 (fn costly (x:i64) i64 = (+ (* x 3) 5))
         (pub run (x:i64) i64 ()
           (if (< x 0) ((ret x)) ((ret (call costly x)))))",
    )
    .unwrap();
    let prepared = module.prepare_call("run").unwrap();
    let mut workspace = Workspace::default();
    let mut oracle = Workspace::default();
    for x in [-7, 0, 13, i64::MAX] {
        let args = [Value::I64(x)];
        for instructions in 0..=16 {
            for cells in 0..=9 {
                for frames in 0..=2 {
                    let limits = Limits {
                        instructions,
                        cells,
                        frames,
                    };
                    let expected = portable(&module, &[], &args, limits, &mut oracle);
                    assert_eq!(
                        prepared.run_with(&[], &args, limits, &mut workspace),
                        expected,
                        "x={x} {limits:?}"
                    );
                }
            }
        }
    }
    let short = Limits {
        instructions: 6,
        cells: 4,
        frames: 1,
    };
    let (value, route) = prepared.run_with_route(&[], &[Value::I64(-7)], short, &mut workspace);
    assert_eq!(value, Ok(Value::I64(-7)));
    assert_eq!(route, ExecutionRoute::Portable);
    let (value, route) =
        prepared.run_with_route(&[], &[Value::I64(13)], Limits::default(), &mut workspace);
    assert_eq!(value, Ok(Value::I64(44)));
    assert_native(route);
}

#[test]
fn dynamic_loop_and_boolean_result_preserve_native_execution() {
    let module = Module::compile_compact(
        "v2
         (pub walk (n:i64) i64 (i:i64 sum:i64)
           (= i 0) (= sum 0)
           (while (< i n) ((= sum (+ sum i)) (= i (+ i 1)))) (ret sum))
         (pub compare (x:i64 y:i64) bool = (< x y))",
    )
    .unwrap();
    for (export, args, expected) in [
        ("walk", vec![Value::I64(8)], Value::I64(28)),
        (
            "compare",
            vec![Value::I64(-9), Value::I64(2)],
            Value::Bool(true),
        ),
    ] {
        let prepared = module.prepare_call(export).unwrap();
        let (value, route) =
            prepared.run_with_route(&[], &args, Limits::default(), &mut Workspace::default());
        assert_eq!(value, Ok(expected));
        assert_native(route);
    }
}
