use fluxr::structured::Limits;
#[cfg(any(test, target_arch = "wasm32"))]
use fluxr::{
    RuleProgram,
    hir::{Artifact, Hir},
    structured::{Module, Value, Workspace},
};

pub const ADD: &str = "r4 2 i0 i1 f+";
pub const DIV: &str = "r4 2 i0 i1 f/";
pub const LE: &str = "r3 2 i0 i1 f<=";
pub const CHAIN: &str = "r4 3 i0 i1 f* i2 & i0 f+";
pub const EDGES: [u32; 20] = [
    0,
    0x8000_0000,
    1,
    0x8000_0001,
    0x007f_ffff,
    0x807f_ffff,
    0x0080_0000,
    0x8080_0000,
    0x3f80_0000,
    0xbf80_0000,
    0x7f7f_ffff,
    0xff7f_ffff,
    0x7f80_0000,
    0xff80_0000,
    0x7fc0_1234,
    0xffc0_4321,
    0x7f80_0001,
    0xff80_0001,
    0x4000_0000,
    0xc000_0000,
];

pub fn as_float(word: i64) -> f32 {
    f32::from_bits(word as u32)
}

pub fn as_word(value: f32) -> i64 {
    let bits = value.to_bits();
    if bits & 0x7fff_ffff > 0x7f80_0000 {
        0x7fc0_0000
    } else {
        i64::from(bits)
    }
}

pub fn chain([a, b, mask]: [i64; 3]) -> i64 {
    let product = as_word(as_float(a) * as_float(b));
    as_word(as_float(product & mask) + as_float(a))
}

pub const EXACT: Limits = Limits {
    instructions: 10,
    cells: 8,
    frames: 1,
};

#[cfg(any(test, target_arch = "wasm32"))]
pub fn check() -> usize {
    let mut probes = Vec::new();
    for a in EDGES {
        for b in EDGES {
            probes.push([i64::from(a), i64::from(b), -1]);
            probes.push([
                i64::MIN | i64::from(a),
                0x7fed_cba9_0000_0000 | i64::from(b),
                0x7fff_ffff,
            ]);
        }
    }
    let mut seed = 0x4d53_7c93_eba7_6981u64;
    for _ in 0..4096 {
        probes.push(std::array::from_fn(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed as i64
        }));
    }
    for (a, b) in [
        (0, 0x3f80_0000),
        (0x3f80_0000, 0),
        (0x8000_0000, 0x3f80_0000),
        (0x7f80_0001, 0),
    ] {
        for c in [0, -1, i64::MAX] {
            probes.push([a, b, c]);
        }
    }

    type Oracle = fn([i64; 3]) -> Option<i64>;
    let cases: [(&str, usize, Oracle); 14] = [
        (ADD, 2, |[a, b, _]| Some(as_word(as_float(a) + as_float(b)))),
        ("r4 2 i0 i1 f-", 2, |[a, b, _]| {
            Some(as_word(as_float(a) - as_float(b)))
        }),
        ("r4 2 i0 i1 f*", 2, |[a, b, _]| {
            Some(as_word(as_float(a) * as_float(b)))
        }),
        (DIV, 2, |[a, b, _]| Some(as_word(as_float(a) / as_float(b)))),
        ("r3 2 i0 i1 f<", 2, |[a, b, _]| {
            Some(i64::from(as_float(a) < as_float(b)))
        }),
        (LE, 2, |[a, b, _]| {
            Some(i64::from(as_float(a) <= as_float(b)))
        }),
        (CHAIN, 3, |probe| Some(chain(probe))),
        ("r4 3 i0 i1 f<= i0 i1 f/ i2 1 + ?", 3, |[a, b, c]| {
            let alternate = c.checked_add(1)?;
            Some(if as_float(a) <= as_float(b) {
                as_word(as_float(a) / as_float(b))
            } else {
                alternate
            })
        }),
        (
            "r4 3 i0 i1 f< -9223372036854775808 i2 / i0 i1 f- ?",
            3,
            |[a, b, c]| {
                let selected = i64::MIN.checked_div(c)?;
                Some(if as_float(a) < as_float(b) {
                    selected
                } else {
                    as_word(as_float(a) - as_float(b))
                })
            },
        ),
        (
            "r4 3 i0 i1 f< guard -9223372036854775808 i2 %",
            3,
            |[a, b, c]| {
                if as_float(a) < as_float(b) {
                    i64::MIN.checked_rem(c)
                } else {
                    Some(0)
                }
            },
        ),
        (
            "r4 3 -9223372036854775808 i2 / i0 i1 f<= guard",
            3,
            |[a, b, c]| {
                let prior = i64::MIN.checked_div(c)?;
                Some(if as_float(a) <= as_float(b) { prior } else { 0 })
            },
        ),
        ("r4 2 i0 i1 f* guard 7", 2, |[a, b, _]| {
            Some(if as_word(as_float(a) * as_float(b)) == 0 {
                0
            } else {
                7
            })
        }),
        ("r4 2 i0 i1 f- -1 m", 2, |[a, b, _]| {
            Some(as_word(as_float(a) - as_float(b)).min(-1))
        }),
        ("r4 2 i0 i1 & 0 f+", 2, |[a, b, _]| {
            Some(as_word(as_float(a & b) + 0.0))
        }),
    ];

    let mut workspace = Workspace::default();
    for (text, n, oracle) in cases {
        let rule = RuleProgram::compile_text(text).unwrap();
        let fxd = rule.encode();
        let fxh = Hir::from_rule(rule.clone()).unwrap().encode().unwrap();
        let mut expected_fxh = b"FXH\0\x01\x01".to_vec();
        expected_fxh.extend(((fxd.len() + 2) as u32).to_le_bytes());
        expected_fxh.extend((fxd.len() as u16).to_le_bytes());
        expected_fxh.extend(&fxd);
        assert_eq!(fxh, expected_fxh);
        let hir = Hir::decode(&fxh).unwrap();
        let reason = if text == "r4 2 i0 i1 & 0 f+" {
            "structured projection lacks rule integer bitwise and"
        } else {
            "structured projection lacks rule raw binary32 transport"
        };
        assert_eq!(hir.source().unwrap_err().to_string(), reason);
        let projected = hir.prepare_rule_projection().unwrap();
        assert_eq!(hir.encode().unwrap(), fxh);
        let Artifact::Rule(restored) = hir.into_artifact() else {
            panic!("expected restored rule");
        };
        assert_eq!(restored.encode(), fxd);
        let prepared = restored.prepare();
        for &probe in &probes {
            let args = probe.map(Value::I64);
            let legacy = rule.evaluate(&probe[..n]);
            let result = projected.run_with(&args[..n], Limits::default(), &mut workspace);
            assert_eq!(prepared.evaluate(&probe[..n]), legacy, "{text} {probe:?}");
            match oracle(probe) {
                Some(value) => {
                    assert_eq!(legacy, Ok(value), "{text} {probe:?}");
                    assert_eq!(result, Ok(Value::I64(value)), "{text} {probe:?}");
                }
                None => {
                    assert_eq!(
                        legacy.unwrap_err().to_string(),
                        "rule arithmetic overflow or division by zero"
                    );
                    assert_eq!(
                        result.unwrap_err().to_string(),
                        "invalid or overflowing integer arithmetic"
                    );
                }
            }
        }
    }

    for text in ["r2 2 i0 i1 f<", "r3 2 i0 i1 f+"] {
        assert!(RuleProgram::compile_text(text).is_err(), "{text}");
    }
    assert!(Module::compile_compact("v1 (pub rule (a:i64 b:i64) i64 = (f+ a b))").is_err());

    let hir = Hir::from_rule(RuleProgram::compile_text(ADD).unwrap()).unwrap();
    let projected = hir.prepare_rule_projection().unwrap();
    let args = [Value::I64(-2_147_483_648), Value::I64(0x8000_0000)];
    for (limits, diagnostic) in [
        (
            Limits {
                instructions: 9,
                ..EXACT
            },
            "module instruction budget exhausted",
        ),
        (Limits { cells: 7, ..EXACT }, "module cell budget exhausted"),
        (Limits { frames: 0, ..EXACT }, "module frame limit exceeded"),
    ] {
        assert_eq!(
            projected
                .run_with(&args, limits, &mut workspace)
                .unwrap_err()
                .to_string(),
            diagnostic
        );
        assert_eq!(
            projected.run_with(&args, EXACT, &mut workspace),
            Ok(Value::I64(0x8000_0000))
        );
    }
    for (args, diagnostic) in [
        (
            &args[..1],
            "host argument or input count differs from interface",
        ),
        (
            &[Value::F32(0), Value::I64(0)][..],
            "host value differs from I64",
        ),
    ] {
        assert_eq!(
            projected
                .run_with(args, EXACT, &mut workspace)
                .unwrap_err()
                .to_string(),
            diagnostic
        );
    }
    cases.len() * probes.len()
}
