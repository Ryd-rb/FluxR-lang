use fluxr::RuleProgram;
use fluxr::hir::{Artifact, Hir};
use fluxr::structured::{Limits, Module, Value, Workspace};

pub const AND: &str = "r2 2 i0 i1 &";
#[cfg(not(target_arch = "wasm32"))]
pub const ADD: &str = "r2 2 i0 i1 +";

pub fn check() -> usize {
    let edge = [
        i64::MIN,
        i64::MIN + 1,
        -0x5555_5555_5555_5556,
        -2,
        -1,
        0,
        1,
        2,
        0x5555_5555_5555_5555,
        i64::MAX,
    ];
    let mut probes: Vec<_> = edge
        .iter()
        .flat_map(|&a| edge.iter().flat_map(move |&b| edge.map(|c| [a, b, c])))
        .collect();
    let mut seed = 0x4d53_7c93_eba7_6981u64;
    for _ in 0..1024 {
        probes.push(std::array::from_fn(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed as i64
        }));
    }

    type Oracle = fn([i64; 3]) -> Option<i64>;
    let cases: [(&str, usize, Oracle); 8] = [
        (AND, 2, |[a, b, _]| Some(a & b)),
        (
            "r2 2 i0 i1 & -9223372036854775808 9223372036854775807 ?",
            2,
            |[a, b, _]| Some(if a & b != 0 { i64::MIN } else { i64::MAX }),
        ),
        (
            "r2 3 i2 i0 i1 & -9223372036854775808 i1 / ?",
            3,
            |[a, b, c]| {
                let alternate = i64::MIN.checked_div(b)?;
                Some(if c != 0 { a & b } else { alternate })
            },
        ),
        ("r2 3 i2 9 i1 / i0 i1 & ?", 3, |[a, b, c]| {
            let first = 9i64.checked_div(b)?;
            Some(if c != 0 { first } else { a & b })
        }),
        (
            "r3 3 i0 i1 & guard -9223372036854775808 i2 /",
            3,
            |[a, b, c]| {
                if a & b == 0 {
                    Some(0)
                } else {
                    i64::MIN.checked_div(c)
                }
            },
        ),
        (
            "r3 3 -9223372036854775808 i2 / i0 i1 & guard",
            3,
            |[a, b, c]| {
                let prior = i64::MIN.checked_div(c)?;
                Some(if a & b == 0 { 0 } else { prior })
            },
        ),
        ("r4 2 i0 i1 & 1 +", 2, |[a, b, _]| (a & b).checked_add(1)),
        ("r4 3 i0 i1 & i2 %", 3, |[a, b, c]| (a & b).checked_rem(c)),
    ];
    let mut workspace = Workspace::default();
    for (text, n, oracle) in cases {
        let rule = RuleProgram::compile_text(text).unwrap();
        let fxd = rule.encode();
        let fxh = Hir::from_rule(rule.clone()).unwrap().encode().unwrap();
        let hir = Hir::decode(&fxh).unwrap();
        assert_eq!(
            hir.source().unwrap_err().to_string(),
            "structured projection lacks rule integer bitwise and"
        );
        let projection = hir.prepare_rule_projection().unwrap();
        assert_eq!(hir.encode().unwrap(), fxh);
        let Artifact::Rule(restored) = hir.into_artifact() else {
            panic!("expected FXD");
        };
        assert_eq!(restored.encode(), fxd);
        let prepared = restored.prepare();
        for &probe in &probes {
            let args = probe.map(Value::I64);
            let expected = oracle(probe);
            let original = rule.evaluate(&probe[..n]);
            assert_eq!(prepared.evaluate(&probe[..n]), original, "{text} {probe:?}");
            let projected = projection.run_with(&args[..n], Limits::default(), &mut workspace);
            match expected {
                Some(value) => {
                    assert_eq!(original, Ok(value), "{text} {probe:?}");
                    assert_eq!(projected, Ok(Value::I64(value)), "{text} {probe:?}");
                }
                None => {
                    assert_eq!(
                        original.unwrap_err().to_string(),
                        "rule arithmetic overflow or division by zero",
                        "{text} {probe:?}"
                    );
                    assert_eq!(
                        projected.unwrap_err().to_string(),
                        "invalid or overflowing integer arithmetic",
                        "{text} {probe:?}"
                    );
                }
            }
        }
    }

    for (text, inputs, expected) in [
        (
            format!("r2 2 {}i1 {}", "i0 ".repeat(31), "& ".repeat(31)),
            vec![i64::MIN, -1],
            i64::MIN,
        ),
        (
            format!("r4 16 {}i15 i0 &", "1 guard ".repeat(126)),
            vec![-1; 16],
            -1,
        ),
        (
            format!("r4 16 {}i15 i0 & 1 2 ?", "1 guard ".repeat(125)),
            vec![-1; 16],
            1,
        ),
    ] {
        let rule = RuleProgram::compile_text(&text).unwrap();
        assert_eq!(rule.evaluate(&inputs), Ok(expected));
        let projection = Hir::from_rule(rule)
            .unwrap()
            .prepare_rule_projection()
            .unwrap();
        let args: Vec<_> = inputs.into_iter().map(Value::I64).collect();
        assert_eq!(
            projection.run_with(&args, Limits::default(), &mut workspace),
            Ok(Value::I64(expected))
        );
    }

    for (text, reason) in [
        (
            "r4 2 i0 i1 & 0 f+",
            "structured projection lacks rule integer bitwise and",
        ),
        (
            "r4 2 i0 i1 f+ i1 &",
            "structured projection lacks rule raw binary32 transport",
        ),
    ] {
        let hir = Hir::from_rule(RuleProgram::compile_text(text).unwrap()).unwrap();
        assert_eq!(hir.source().unwrap_err().to_string(), reason);
    }
    let structured = "v2\n(pub rule (a:i64 b:i64) i64 () (ret (&& a b)))";
    assert_eq!(
        Module::compile_compact(structured).unwrap_err().to_string(),
        "expected Bool, got I64"
    );
    let projection = Hir::from_rule(RuleProgram::compile_text(AND).unwrap())
        .unwrap()
        .prepare_rule_projection()
        .unwrap();
    let args = [Value::I64(i64::MIN), Value::I64(-1)];
    let exact = Limits {
        instructions: 10,
        cells: 8,
        frames: 1,
    };
    for (limits, error) in [
        (
            Limits {
                instructions: 9,
                ..exact
            },
            "module instruction budget exhausted",
        ),
        (Limits { cells: 7, ..exact }, "module cell budget exhausted"),
        (Limits { frames: 0, ..exact }, "module frame limit exceeded"),
    ] {
        assert_eq!(
            projection
                .run_with(&args, limits, &mut workspace)
                .unwrap_err()
                .to_string(),
            error
        );
        assert_eq!(
            projection.run_with(&args, exact, &mut workspace),
            Ok(args[0].clone())
        );
    }
    for (args, error) in [
        (
            &args[..1],
            "host argument or input count differs from interface",
        ),
        (
            &[Value::Bool(true), Value::I64(-1)][..],
            "host value differs from I64",
        ),
    ] {
        assert_eq!(
            projection
                .run_with(args, exact, &mut workspace)
                .unwrap_err()
                .to_string(),
            error
        );
    }
    cases.len() * probes.len() + 3
}
