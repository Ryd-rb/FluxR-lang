use fluxr::RuleProgram;
use fluxr::hir::{Artifact, Hir};
use fluxr::structured::{Invocation, Limits, Module, Value, Workspace};

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
