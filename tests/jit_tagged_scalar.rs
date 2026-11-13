//! Independent public-API checks for Boolean/signed scalar admission. These
//! expectations use source-level Rust calculations and the portable evaluator,
//! not the native Plan, emitter, or its private lane mask.
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
    export: &str,
    inputs: &[Value],
    arguments: &[Value],
    limits: Limits,
    workspace: &mut Workspace,
) -> Result<Value, fluxr::Error> {
    module.run_portable_with(
        Invocation {
            export,
            inputs,
            arguments,
            limits,
        },
        workspace,
    )
}

#[test]
fn every_scalar_tag_mask_and_input_partition_preserves_values_and_rejects_wrong_tags() {
    // Each lane affects the independently calculated score. Four lanes fit the
    // scalar ABI; zero lanes still cover a Boolean result. Both exports share
    // the layout while exercising different result tags.
    for count in 0..=4 {
        for boolean_mask in 0..(1usize << count) {
            for input_count in 0..=count {
                let mut inputs_text = String::new();
                let mut args_text = String::new();
                let mut body = "(= total 0)".to_owned();
                for index in 0..count {
                    let boolean = boolean_mask & (1 << index) != 0;
                    let ty = if boolean { "bool" } else { "i64" };
                    let (bindings, prefix) = if index < input_count {
                        (&mut inputs_text, "$")
                    } else {
                        (&mut args_text, "")
                    };
                    bindings.push_str(&format!(" x{index}:{ty}"));
                    let weight = index + 1;
                    body.push_str(&if boolean {
                        format!(
                            "(if {prefix}x{index} ((= total (+ total {weight}))) ((= total (- total {weight}))))"
                        )
                    } else {
                        format!("(= total (+ total (* {prefix}x{index} {weight})))")
                    });
                }
                let inputs_decl = if input_count == 0 {
                    String::new()
                } else {
                    format!("(inputs{inputs_text})")
                };
                let source = format!(
                    "v2 {inputs_decl}
                     (pub score ({args_text}) i64 (total:i64) {body} (ret total))
                     (pub predicate ({args_text}) bool (total:i64) {body} (ret (< total 0)))"
                );
                let module = Module::compile_compact(&source).unwrap();
                for export in ["score", "predicate"] {
                    let prepared = module.prepare_call(export).unwrap();
                    let mut workspace = Workspace::default();
                    let mut oracle = Workspace::default();
                    for row in 0..16usize {
                        let mut expected = 0i64;
                        let values: Vec<_> = (0..count)
                            .map(|index| {
                                let weight = (index + 1) as i64;
                                if boolean_mask & (1 << index) != 0 {
                                    let value = row & (1 << index) != 0;
                                    expected += if value { weight } else { -weight };
                                    Value::Bool(value)
                                } else {
                                    let value = -23 + 11 * index as i64 + row as i64;
                                    expected += value * weight;
                                    Value::I64(value)
                                }
                            })
                            .collect();
                        let (inputs, arguments) = values.split_at(input_count);
                        let expected = if export == "score" {
                            Value::I64(expected)
                        } else {
                            Value::Bool(expected < 0)
                        };
                        let (actual, route) = prepared.run_with_route(
                            inputs,
                            arguments,
                            Limits::default(),
                            &mut workspace,
                        );
                        assert_eq!(actual, Ok(expected.clone()), "{source} row={row}");
                        assert_eq!(
                            actual,
                            portable(
                                &module,
                                export,
                                inputs,
                                arguments,
                                Limits::default(),
                                &mut oracle
                            )
                        );
                        assert_native(route);
                        if row != 0 {
                            continue;
                        }
                        for lane in 0..count {
                            let mut malformed = values.clone();
                            malformed[lane] = if boolean_mask & (1 << lane) != 0 {
                                Value::I64(0)
                            } else {
                                Value::Bool(false)
                            };
                            let (bad_inputs, bad_args) = malformed.split_at(input_count);
                            for limits in [
                                Limits::default(),
                                Limits {
                                    instructions: 0,
                                    cells: 0,
                                    frames: 0,
                                },
                            ] {
                                let expected = portable(
                                    &module,
                                    export,
                                    bad_inputs,
                                    bad_args,
                                    limits,
                                    &mut oracle,
                                );
                                let (actual, route) = prepared.run_with_route(
                                    bad_inputs,
                                    bad_args,
                                    limits,
                                    &mut workspace,
                                );
                                assert!(expected.is_err(), "wrong tag was admitted: {source}");
                                assert_eq!(actual, expected, "{source} lane={lane} {limits:?}");
                                assert_eq!(route, ExecutionRoute::Portable);
                            }
                        }
                        // Exact counts precede lane packing, including an extra
                        // input on a no-input export or an extra fifth argument.
                        let mut too_many_inputs = inputs.to_vec();
                        too_many_inputs.push(Value::Bool(true));
                        let mut too_many_args = arguments.to_vec();
                        too_many_args.push(Value::I64(17));
                        for (bad_inputs, bad_args) in [
                            (too_many_inputs.as_slice(), arguments),
                            (inputs, too_many_args.as_slice()),
                        ] {
                            let (actual, route) = prepared.run_with_route(
                                bad_inputs,
                                bad_args,
                                Limits::default(),
                                &mut workspace,
                            );
                            assert!(actual.is_err());
                            assert_eq!(
                                actual,
                                portable(
                                    &module,
                                    export,
                                    bad_inputs,
                                    bad_args,
                                    Limits::default(),
                                    &mut oracle
                                )
                            );
                            assert_eq!(route, ExecutionRoute::Portable);
                        }
                        assert_eq!(
                            prepared.run_with(inputs, arguments, Limits::default(), &mut workspace),
                            Ok(expected)
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn boolean_identity_has_independent_exact_resource_limits() {
    for host_input in [false, true] {
        let source = if host_input {
            "v2 (inputs flag:bool) (pub run () bool = $flag)"
        } else {
            "v2 (pub run (flag:bool) bool = flag)"
        };
        let module = Module::compile_compact(source).unwrap();
        let prepared = module.prepare_call("run").unwrap();
        let mut workspace = Workspace::default();
        let mut oracle = Workspace::default();
        // Load + return; boundary input and output cells; one frame cell and
        // one local slot only when the Boolean is a function argument.
        let exact = Limits {
            instructions: 2,
            cells: if host_input { 3 } else { 4 },
            frames: 1,
        };
        for flag in [false, true] {
            let values = [Value::Bool(flag)];
            let (inputs, arguments) = if host_input {
                (values.as_slice(), &[][..])
            } else {
                (&[][..], values.as_slice())
            };
            let (actual, route) = prepared.run_with_route(inputs, arguments, exact, &mut workspace);
            assert_eq!(actual, Ok(Value::Bool(flag)));
            assert_native(route);
            for limits in [
                Limits {
                    instructions: 1,
                    ..exact
                },
                Limits {
                    cells: exact.cells - 1,
                    ..exact
                },
                Limits { frames: 0, ..exact },
            ] {
                let expected = portable(&module, "run", inputs, arguments, limits, &mut oracle);
                assert!(expected.is_err());
                let (actual, route) =
                    prepared.run_with_route(inputs, arguments, limits, &mut workspace);
                assert_eq!(actual, expected);
                assert_eq!(route, ExecutionRoute::Portable);
            }
        }
    }
}

#[test]
fn mixed_boolean_branches_preserve_short_budgets_and_first_traps() {
    let module = Module::compile_compact(
        "v2
         (fn helper (flag:bool x:i64) bool ()
           (if flag ((ret (< x 7))) ((ret (== (/ x 0) 0)))))
         (pub choose (flag:bool x:i64 y:i64) i64 ()
           (if flag ((ret x)) ((ret (+ x y)))))
         (pub predicate (flag:bool x:i64 y:i64) bool ()
           (if flag ((ret (< x y))) ((ret (== (/ x 0) y)))))
         (pub uninitialized (flag:bool x:i64) i64 (unset:i64)
           (if flag ((ret (+ unset (/ x 0)))) ((ret x))))
         (pub nested (flag:bool x:i64) bool = (call helper flag x))",
    )
    .unwrap();
    for (export, arguments, expected) in [
        (
            "choose",
            vec![Value::Bool(true), Value::I64(i64::MAX), Value::I64(1)],
            Ok(Value::I64(i64::MAX)),
        ),
        (
            "choose",
            vec![Value::Bool(false), Value::I64(i64::MAX), Value::I64(1)],
            Err("invalid or overflowing integer arithmetic"),
        ),
        (
            "predicate",
            vec![Value::Bool(true), Value::I64(-1), Value::I64(7)],
            Ok(Value::Bool(true)),
        ),
        (
            "predicate",
            vec![Value::Bool(false), Value::I64(-1), Value::I64(7)],
            Err("invalid or overflowing integer arithmetic"),
        ),
        (
            "uninitialized",
            vec![Value::Bool(true), Value::I64(7)],
            Err("read of uninitialized local"),
        ),
        (
            "uninitialized",
            vec![Value::Bool(false), Value::I64(7)],
            Ok(Value::I64(7)),
        ),
        (
            "nested",
            vec![Value::Bool(true), Value::I64(-1)],
            Ok(Value::Bool(true)),
        ),
        (
            "nested",
            vec![Value::Bool(false), Value::I64(-1)],
            Err("invalid or overflowing integer arithmetic"),
        ),
    ] {
        let prepared = module.prepare_call(export).unwrap();
        let mut workspace = Workspace::default();
        let mut oracle = Workspace::default();
        let (actual, route) =
            prepared.run_with_route(&[], &arguments, Limits::default(), &mut workspace);
        assert_eq!(
            actual.as_ref().map_err(|e| e.to_string()),
            expected.as_ref().map_err(|e| (*e).to_owned())
        );
        if actual.is_ok() {
            assert_native(route);
        } else {
            assert_eq!(route, ExecutionRoute::Portable);
        }
        for instructions in 0..=16 {
            for cells in 0..=12 {
                for frames in 0..=2 {
                    let limits = Limits {
                        instructions,
                        cells,
                        frames,
                    };
                    assert_eq!(
                        prepared.run_with(&[], &arguments, limits, &mut workspace),
                        portable(&module, export, &[], &arguments, limits, &mut oracle),
                        "{export} {arguments:?} {limits:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn five_mixed_lanes_retain_generic_native_boundary() {
    let module = Module::compile_compact(
        "v2 (inputs first:bool left:i64)
         (pub run (second:bool right:i64 third:bool) i64 (total:i64)
           (= total (+ $left right))
           (if $first ((= total (+ total 3))))
           (if second ((= total (- total 7))))
           (if third ((= total (* total 2)))) (ret total))",
    )
    .unwrap();
    let prepared = module.prepare_call("run").unwrap();
    for mask in 0..8 {
        let (first, second, third) = (mask & 1 != 0, mask & 2 != 0, mask & 4 != 0);
        let inputs = [Value::Bool(first), Value::I64(-11)];
        let arguments = [Value::Bool(second), Value::I64(23), Value::Bool(third)];
        let mut expected = 12 + if first { 3 } else { 0 } - if second { 7 } else { 0 };
        if third {
            expected *= 2;
        }
        let (actual, route) = prepared.run_with_route(
            &inputs,
            &arguments,
            Limits::default(),
            &mut Workspace::default(),
        );
        assert_eq!(actual, Ok(Value::I64(expected)));
        assert_native(route);
    }
}
