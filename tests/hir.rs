use fluxr::RuleProgram;
use fluxr::hir::{Artifact, Hir};
use fluxr::structured::{Invocation, Limits, Module, Value, Workspace};

#[test]
fn released_guard_fxh_round_trips_exactly() {
    // Frozen FXH v1 bytes from the FXM-03.B.2 reader.
    let wire = include_bytes!("fixtures/released_guard.fxh");
    let hir = Hir::decode(wire).unwrap();
    assert_eq!(hir.encode().unwrap(), wire);
    let Artifact::Rule(restored) = hir.into_artifact() else {
        panic!("FXD origin changed");
    };
    let fxd = RuleProgram::compile_text("r3 2 i0 guard 120 i1 /")
        .unwrap()
        .encode();
    assert_eq!(restored.encode(), fxd);
    assert_eq!(restored.evaluate(&[0, 0]), Ok(0));
    assert_eq!(restored.evaluate(&[1, 3]), Ok(40));
    assert_eq!(
        restored.evaluate(&[1, 0]).unwrap_err().to_string(),
        "rule arithmetic overflow or division by zero"
    );
}

#[test]
fn fr1_route_and_pick_preserve_released_results_and_metering() {
    let wire = include_bytes!("fixtures/fr1/route.fxh");
    let fsm = include_bytes!("fixtures/fr1/route.fsm");
    let hir = Hir::decode(wire).unwrap();
    assert_eq!(hir.encode().unwrap(), wire);
    let projected = Module::compile(hir.source().unwrap()).unwrap();
    let Artifact::Structured(restored) = hir.into_artifact() else {
        panic!("FR1 origin changed");
    };
    let ids = |values: &[u64]| {
        Value::List(
            values
                .iter()
                .copied()
                .map(Value::U64)
                .collect::<Vec<_>>()
                .into(),
        )
    };
    let route = |enabled, values: &[u64]| Value::Record([Value::Bool(enabled), ids(values)].into());
    let event_id = |id| Value::Variant(1, Some(Value::U64(id).into()));
    let outside = Err("list index out of bounds".to_owned());
    for module in [projected, restored, Module::decode(fsm).unwrap()] {
        assert_eq!(module.encode(), fsm);
        let mut workspace = Workspace::default();
        for (export, arguments, expected, instructions, cells) in [
            (
                "route",
                vec![route(false, &[])],
                Ok(Value::List(vec![event_id(7)].into())),
                14,
                12,
            ),
            (
                "route",
                vec![route(true, &[13, 99])],
                Ok(Value::List(
                    vec![event_id(7), Value::Variant(0, None), event_id(99)].into(),
                )),
                28,
                21,
            ),
            ("route", vec![route(true, &[13])], outside.clone(), 23, 12),
            (
                "pick",
                vec![ids(&[13, u64::MAX]), Value::U64(1)],
                Ok(Value::U64(u64::MAX)),
                4,
                8,
            ),
            (
                "pick",
                vec![ids(&[13, 99]), Value::U64(2)],
                outside.clone(),
                3,
                7,
            ),
        ] {
            let mut run = |limits| {
                module
                    .run_portable_with(
                        Invocation {
                            export,
                            inputs: &[],
                            arguments: &arguments,
                            limits,
                        },
                        &mut workspace,
                    )
                    .map_err(|error| error.to_string())
            };
            let limits = Limits {
                instructions,
                cells,
                frames: 1,
            };
            assert_eq!(run(limits), expected);
            for (limits, error) in [
                (
                    Limits {
                        instructions: instructions - 1,
                        ..Limits::default()
                    },
                    "module instruction budget exhausted",
                ),
                (
                    Limits {
                        cells: cells - 1,
                        ..Limits::default()
                    },
                    "module cell budget exhausted",
                ),
                (
                    Limits {
                        frames: 0,
                        ..Limits::default()
                    },
                    "module frame limit exceeded",
                ),
            ] {
                assert_eq!(run(limits).unwrap_err(), error);
            }
        }
    }
}

#[test]
fn fr1_semantic_edits_preserve_binary32_bits_and_trap_order() {
    let wire = include_bytes!("fixtures/fr1/semantic-edits.fxh");
    let fsm = include_bytes!("fixtures/fr1/semantic-edits.fsm");
    let hir = Hir::decode(wire).unwrap();
    assert_eq!(hir.encode().unwrap(), wire);
    let compiled = Module::compile(hir.source().unwrap()).unwrap();
    let Artifact::Structured(restored) = hir.into_artifact() else {
        panic!("FR1 origin changed");
    };

    let sample = |bits| Value::Variant(1, Some(Value::F32(bits).into()));
    let arguments = |index| {
        vec![
            Value::List(vec![Value::U64(13), Value::U64(u64::MAX)].into()),
            Value::U64(index),
        ]
    };
    let cases = [
        (
            "route",
            vec![Value::Record([Value::Bool(false)].into())],
            Ok(Value::List(vec![sample(0x8000_0001)].into())),
            14,
            11,
        ),
        (
            "route",
            vec![Value::Record([Value::Bool(true)].into())],
            Ok(Value::List(
                vec![
                    sample(0x8000_0001),
                    Value::Variant(0, Some(Value::F32(0x8000_0000).into())),
                    sample(0),
                    sample(0x7fc0_0031),
                ]
                .into(),
            )),
            32,
            23,
        ),
        ("pick", arguments(0), Ok(Value::U64(u64::MAX)), 6, 8),
        ("pick", arguments(1), Err("list index out of bounds"), 5, 7),
        (
            "pick",
            arguments(u64::MAX),
            Err("invalid or overflowing integer arithmetic"),
            4,
            7,
        ),
    ];

    for module in [compiled, restored, Module::decode(fsm).unwrap()] {
        assert_eq!(module.encode(), fsm);
        let mut workspace = Workspace::default();
        for (export, arguments, expected, instructions, cells) in &cases {
            let mut run = |limits| {
                module
                    .run_portable_with(
                        Invocation {
                            export,
                            inputs: &[],
                            arguments,
                            limits,
                        },
                        &mut workspace,
                    )
                    .map_err(|error| error.to_string())
            };
            let limits = Limits {
                instructions: *instructions,
                cells: *cells,
                frames: 1,
            };
            assert_eq!(run(limits), expected.clone().map_err(str::to_owned));
            for (limits, error) in [
                (
                    Limits {
                        instructions: instructions - 1,
                        ..Limits::default()
                    },
                    "module instruction budget exhausted",
                ),
                (
                    Limits {
                        cells: cells - 1,
                        ..Limits::default()
                    },
                    "module cell budget exhausted",
                ),
                (
                    Limits {
                        frames: 0,
                        ..Limits::default()
                    },
                    "module frame limit exceeded",
                ),
            ] {
                assert_eq!(run(limits).unwrap_err(), error);
            }
        }
    }
}

#[test]
fn shared_rule_projection_preserves_order_and_checked_arithmetic() {
    let probes = [i64::MIN, -2, -1, 0, 1, 2, i64::MAX];
    for text in [
        "r1 2 i0 i1 +",
        "r1 2 i0 i1 -",
        "r1 2 i0 i1 *",
        "r1 2 i0 i1 /",
        "r1 2 i0 i1 %",
        "r1 2 i0 i1 <",
        "r1 2 i0 i1 =",
        "r1 2 i0 i1 m",
        "r1 2 i0 i1 M",
        "r1 2 i0 42 9 i1 / ?",
        "r3 2 i0 guard 9 i1 /",
        "r3 2 9 i1 / i0 guard",
        "r3 2 12 i0 guard i1 guard",
    ] {
        let rule = RuleProgram::compile_text(text).unwrap();
        let wire = Hir::from_rule(rule.clone()).unwrap().encode().unwrap();
        let hir = Hir::decode(&wire).unwrap();
        let module = Module::compile(hir.source().unwrap()).unwrap();
        let Artifact::Rule(restored) = hir.into_artifact() else {
            panic!("FXD origin changed");
        };
        assert_eq!(restored.encode(), rule.encode());

        let mut workspace = Workspace::default();
        for left in probes {
            for right in probes {
                let expected = rule.evaluate(&[left, right]);
                assert_eq!(restored.evaluate(&[left, right]), expected);
                let arguments = [Value::I64(left), Value::I64(right)];
                let invocation = Invocation {
                    export: "rule",
                    inputs: &[],
                    arguments: &arguments,
                    limits: Limits::default(),
                };
                let actual = module.run_portable_with(invocation, &mut workspace);
                match expected {
                    Ok(value) => assert_eq!(actual, Ok(Value::I64(value)), "{text}"),
                    Err(error) => {
                        assert_eq!(
                            error.to_string(),
                            "rule arithmetic overflow or division by zero"
                        );
                        assert_eq!(
                            actual.unwrap_err().to_string(),
                            "invalid or overflowing integer arithmetic",
                            "{text}"
                        );
                    }
                }
            }
        }
    }

    let text = format!("r4 16 {} i15 2 3 ?", "1 guard ".repeat(126));
    let rule = RuleProgram::compile_text(&text).unwrap();
    let wire = Hir::from_rule(rule.clone()).unwrap().encode().unwrap();
    let Artifact::Rule(restored) = Hir::decode(&wire).unwrap().into_artifact() else {
        panic!("FXD origin changed");
    };
    assert_eq!(restored.encode(), rule.encode());
}

#[test]
fn rule_only_ir_preserves_bit_transport_and_refuses_structured_projection() {
    for (text, reason) in [
        ("r2 2 i0 i1 &", "bitwise and"),
        ("r3 2 i0 i1 f<", "raw binary32"),
        ("r3 2 i0 i1 f<=", "raw binary32"),
        ("r4 2 i0 i1 f+", "raw binary32"),
        ("r4 2 i0 i1 f-", "raw binary32"),
        ("r4 2 i0 i1 f*", "raw binary32"),
        ("r4 2 i0 i1 f/", "raw binary32"),
    ] {
        let rule = RuleProgram::compile_text(text).unwrap();
        let wire = Hir::from_rule(rule.clone()).unwrap().encode().unwrap();
        let hir = Hir::decode(&wire).unwrap();
        assert!(hir.source().unwrap_err().to_string().contains(reason));
        assert_eq!(hir.encode().unwrap(), wire);
        let Artifact::Rule(restored) = hir.into_artifact() else {
            panic!("FXD origin changed");
        };
        assert_eq!(restored.encode(), rule.encode());
        for inputs in [
            [0x1234_5678_7f80_0001, 0],
            [i64::MIN | 0x8000_0000, 0x8000_0000],
            [0x3f80_0000, 0x4000_0000],
            [-1, 1],
        ] {
            assert_eq!(restored.evaluate(&inputs), rule.evaluate(&inputs));
        }
        if text.ends_with("f+") {
            assert_eq!(
                restored.evaluate(&[0x1234_5678_7f80_0001, 0]),
                Ok(0x7fc0_0000)
            );
            assert_eq!(
                restored.evaluate(&[i64::MIN | 0x8000_0000, 0x8000_0000]),
                Ok(0x8000_0000)
            );
        }
    }
}

#[test]
fn fxh_rechecks_payload_and_rejects_inconsistent_rule_projection() {
    let structured =
        Module::compile_legacy_json(include_str!("fixtures/structured_counter.fluxm")).unwrap();
    let rule = RuleProgram::compile_text("r1 0 2").unwrap();
    let structured_wire = Hir::from_structured(structured.clone()).encode().unwrap();
    let rule_wire = Hir::from_rule(rule.clone()).unwrap().encode().unwrap();
    for wire in [&structured_wire, &rule_wire] {
        assert_eq!(Hir::decode(wire).unwrap().encode().unwrap(), *wire);
        for end in 0..wire.len() {
            assert!(Hir::decode(&wire[..end]).is_err(), "prefix {end}");
        }
        for (offset, replacement) in [(0, 0), (4, 2), (5, 0xff)] {
            let mut corrupted = wire.clone();
            corrupted[offset] = replacement;
            assert!(Hir::decode(&corrupted).is_err());
        }
        let mut extended = wire.clone();
        extended.push(b' ');
        assert!(Hir::decode(&extended).is_err());
    }
    let mut changed = rule_wire;
    let projected_constant = changed.iter().rposition(|&byte| byte == b'2').unwrap();
    changed[projected_constant] = b'3';
    assert!(Hir::decode(&changed).is_err());

    let Artifact::Structured(restored) = Hir::decode(&structured_wire).unwrap().into_artifact()
    else {
        panic!("FSM origin changed");
    };
    assert_eq!(restored.encode(), structured.encode());
}
