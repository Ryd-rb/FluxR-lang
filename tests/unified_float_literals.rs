//! Decimal notation changes authored bytes, never checked binary32 semantics.
use fluxr::runtime::Runtime;
use fluxr::structured::wordcode::{Dictionary, ExpansionLimits};
use fluxr::structured::{Invocation, Limits, Module, SourceDocument, Type, Value, Workspace};
use std::sync::Arc;

#[path = "../benchmarks/unified-floats-v1/affine.rs"]
mod affine_reference;
#[path = "../benchmarks/unified-floats-v1/constants.rs"]
mod constants_reference;
#[path = "../benchmarks/unified-floats-v1/loop.rs"]
mod loop_reference;
#[path = "../benchmarks/unified-floats-v1/special.rs"]
mod special_reference;
#[path = "../benchmarks/unified-floats-v1/threshold.rs"]
mod threshold_reference;

const PAIRS: [(&str, &str); 5] = [
    (
        include_str!("../benchmarks/unified-floats-v1/affine.before.fluxr"),
        include_str!("../benchmarks/unified-floats-v1/affine.after.fluxr"),
    ),
    (
        include_str!("../benchmarks/unified-floats-v1/threshold.before.fluxr"),
        include_str!("../benchmarks/unified-floats-v1/threshold.after.fluxr"),
    ),
    (
        include_str!("../benchmarks/unified-floats-v1/constants.before.fluxr"),
        include_str!("../benchmarks/unified-floats-v1/constants.after.fluxr"),
    ),
    (
        include_str!("../benchmarks/unified-floats-v1/loop.before.fluxr"),
        include_str!("../benchmarks/unified-floats-v1/loop.after.fluxr"),
    ),
    (
        include_str!("../benchmarks/unified-floats-v1/special.before.fluxr"),
        include_str!("../benchmarks/unified-floats-v1/special.after.fluxr"),
    ),
];

fn pair(before: &str, after: &str) -> (Module, Module) {
    let before = SourceDocument::parse(before).unwrap();
    let after = SourceDocument::parse(after).unwrap();
    assert_eq!(before.source, after.source);
    let (before, after) = (before.compile().unwrap(), after.compile().unwrap());
    assert_eq!(before.encode(), after.encode());
    let printed = after.unified_text().unwrap();
    assert_eq!(before.unified_text().unwrap(), printed);
    assert_eq!(
        SourceDocument::parse(&printed)
            .unwrap()
            .compile()
            .unwrap()
            .encode(),
        before.encode()
    );
    // Legacy projection remains entirely expressible by its released reader.
    assert_eq!(
        Module::compile_agent(&after.agent_text().unwrap())
            .unwrap()
            .encode(),
        before.encode()
    );
    (before, after)
}

fn run(module: &Module, arguments: &[Value], limits: Limits) -> Result<Value, String> {
    module
        .run_portable_with(
            Invocation {
                export: "run",
                inputs: &[],
                arguments,
                limits,
            },
            &mut Workspace::default(),
        )
        .map_err(|error| error.to_string())
}

fn same_budgets(before: &Module, after: &Module, arguments: &[Value]) {
    let prepared = after.prepare_call("run").unwrap();
    let mut workspace = Workspace::default();
    for instructions in 0..=80 {
        for cells in 0..=18 {
            for frames in 0..=2 {
                let limits = Limits {
                    instructions,
                    cells,
                    frames,
                };
                let expected = run(before, arguments, limits);
                assert_eq!(run(after, arguments, limits), expected);
                assert_eq!(
                    prepared
                        .run_with(&[], arguments, limits, &mut workspace)
                        .map_err(|error| error.to_string()),
                    expected,
                    "arguments={arguments:?}, limits={limits:?}"
                );
            }
        }
    }
}

fn list(bits: impl IntoIterator<Item = u32>) -> Value {
    Value::List(Arc::new(bits.into_iter().map(Value::F32).collect()))
}

#[test]
fn retained_token_pairs_match_rust_and_every_small_resource_boundary() {
    for (index, (before, after)) in PAIRS.into_iter().enumerate() {
        let (before, after) = pair(before, after);
        assert_eq!(after.unified_text().unwrap(), PAIRS[index].1.trim_end());
        if index == 2 || index == 4 {
            let expected = if index == 2 {
                list(constants_reference::run())
            } else {
                list(special_reference::run())
            };
            assert_eq!(run(&after, &[], Limits::default()), Ok(expected));
            same_budgets(&before, &after, &[]);
            continue;
        }
        for bits in [0, 0x8000_0000, 1, 0x3f80_0000, 0x7f7f_ffff, 0x7fa0_0001] {
            let mut arguments = vec![Value::F32(bits)];
            let expected = match index {
                0 => Value::F32(affine_reference::run(bits)),
                1 => Value::Bool(threshold_reference::run(bits)),
                3 => {
                    arguments.push(Value::I64(3));
                    Value::F32(loop_reference::run(bits, 3).unwrap())
                }
                _ => unreachable!(),
            };
            assert_eq!(run(&after, &arguments, Limits::default()), Ok(expected));
            same_budgets(&before, &after, &arguments);
        }
    }
}

#[test]
fn decimal_conversion_is_direct_binary32_with_signed_zero_and_rounding_boundaries() {
    for (literal, bits) in [
        ("0f", 0),
        ("-0f", 0x8000_0000),
        ("-0.000e+09f", 0x8000_0000),
        ("0001.0E+00f", 0x3f80_0000),
        ("0.1f", 0x3dcc_cccd),
        ("1e-45f", 1),
        ("-1e-45f", 0x8000_0001),
        ("1e-100f", 0),
        ("-1e-100f", 0x8000_0000),
        ("1.17549435e-38f", 0x0080_0000),
        ("3.4028235e38f", 0x7f7f_ffff),
        // Values immediately around a binary32 midpoint must not be rounded
        // through f64, which would erase the second literal's distinction.
        ("1.000000059604644775390625f", 0x3f80_0000),
        ("1.000000059604644775390626f", 0x3f80_0001),
        ("-1.000000059604644775390625f", 0xbf80_0000),
        ("-1.000000059604644775390626f", 0xbf80_0001),
    ] {
        let before = format!("fluxr 1\npub run 0 float ret 0x{bits:x}f");
        let after = format!("fluxr 1\npub run 0 float ret {literal}");
        let (_, module) = pair(&before, &after);
        assert_eq!(
            run(&module, &[], Limits::default()),
            Ok(Value::F32(bits)),
            "{literal}"
        );
    }
}

#[test]
fn strict_decimal_grammar_rejects_overflow_and_preserves_identifier_and_legacy_syntax() {
    for literal in [
        "1.f",
        ".5f",
        "-.5f",
        "+1f",
        "1_0f",
        "1ef",
        "1e+f",
        "1e-f",
        "1.2.3f",
        "1e1.5f",
        "--1f",
        "-inff",
        "-NaNf",
        "1ff",
        "0xzzf",
        "3.4028236e38f",
        "-3.4028236e38f",
        "1e9999f",
        "1\u{00a0}f",
    ] {
        let text = format!("fluxr 1\npub run 0 float ret {literal}");
        assert!(SourceDocument::parse(&text).is_err(), "accepted {literal}");
    }
    let boundary = format!("0.{}f", "0".repeat(253));
    let text = format!("fluxr 1\npub run 0 float ret {boundary}");
    let module = SourceDocument::parse(&text).unwrap().compile().unwrap();
    assert_eq!(run(&module, &[], Limits::default()), Ok(Value::F32(0)));
    let oversized = format!("fluxr 1\npub run 0 float ret 0.{}f", "0".repeat(254));
    assert!(
        SourceDocument::parse(&oversized)
            .unwrap_err()
            .to_string()
            .contains("literal byte limit")
    );
    for expression in ["1f", "-0f", "1e-8f"] {
        assert!(
            Module::compile_agent(&format!("fa1 v2 pub run 0 float ret {expression}")).is_err()
        );
        assert!(
            Module::compile_wordcode(
                &format!("wk1 core v2 pub run 0 float ret {expression}"),
                &Dictionary::core(),
                ExpansionLimits::default(),
            )
            .is_err()
        );
    }
    for name in ["nanf", "inff", "NaNf", "Infinityf", "f"] {
        let text = format!("fluxr 1\npub run 1 {name} float float ret {name}");
        let module = SourceDocument::parse(&text).unwrap().compile().unwrap();
        assert_eq!(
            run(&module, &[Value::F32(0x8000_0000)], Limits::default()),
            Ok(Value::F32(0x8000_0000))
        );
    }
    for text in [
        "fluxr 1\npub run 0 int ret 1f",
        "fluxr 1\npub run 0 float ret add 1f 1",
        "fluxr 1\npub run 0 uint ret 0f",
    ] {
        assert!(SourceDocument::parse(text).unwrap().compile().is_err());
    }
}

#[test]
fn canonical_float_selection_keeps_raw_fixed_and_scientific_tie_order() {
    let cases = [
        (0x0000_0000_u32, "0f"),
        (0x8000_0000, "-0f"),
        (0x3f80_0000, "1f"),
        (0xbf80_0000, "-1f"),
        (0x3f00_0000, "0.5f"),
        (0x3e80_0000, "0.25f"),
        (0x3c23_d70a, "0.01f"), // Equal-length scientific form loses to fixed.
        (0xbf9e_0651, "0xbf9e0651f"), // Equal-length fixed form loses to raw.
        (0x447a_0000, "1e3f"),
        (0x4480_0000, "1024f"),
        (0x3a83_126f, "1e-3f"),
        (0x322b_cc77, "1e-8f"),
        (0x0000_0001, "0x1f"),
        (0x8000_0001, "-1e-45f"),
        (0x7f7f_ffff, "0x7f7ffffff"),
        (0x7f80_0000, "0x7f800000f"),
        (0xff80_0000, "0xff800000f"),
        (0x7fa0_0001, "0x7fa00001f"),
        (0xffc1_2345, "0xffc12345f"),
    ];
    let prefix = format!(
        "fluxr 1\npub run 0 list float ret list float {} ",
        cases.len()
    );
    let raw: Vec<_> = cases
        .iter()
        .map(|(bits, _)| format!("0x{bits:x}f"))
        .collect();
    let expected: Vec<_> = cases.iter().map(|(_, literal)| *literal).collect();
    let module = SourceDocument::parse(&format!("{prefix}{}", raw.join(" ")))
        .unwrap()
        .compile()
        .unwrap();
    assert_eq!(
        module.unified_text().unwrap(),
        format!("{prefix}{}", expected.join(" "))
    );
}

#[test]
fn canonical_printing_preserves_raw_nan_payloads_and_deterministic_bit_pattern_sample() {
    let mut bits = vec![
        0,
        0x8000_0000,
        1,
        0x8000_0001,
        0x007f_ffff,
        0x0080_0000,
        0x3f80_0000,
        0x7f7f_ffff,
        0xff7f_ffff,
        0x7f80_0000,
        0xff80_0000,
        0x7f80_0001,
        0x7fc0_0000,
        0xffc1_2345,
    ];
    let mut state = 0x6375_7272_u32;
    for _ in 0..1024 {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        bits.push(state);
    }
    let literals: Vec<_> = bits.iter().map(|value| format!("0x{value:x}f")).collect();
    let text = format!(
        "fluxr 1\npub run 0 list float ret list float {} {}",
        bits.len(),
        literals.join(" ")
    );
    let module = SourceDocument::parse(&text).unwrap().compile().unwrap();
    let printed = module.unified_text().unwrap();
    assert!(printed.len() <= text.len());
    for (bits, raw) in bits.iter().zip(&literals) {
        if !f32::from_bits(*bits).is_finite() {
            assert!(printed.split_ascii_whitespace().any(|token| token == raw));
        }
    }
    let reparsed = SourceDocument::parse(&printed).unwrap().compile().unwrap();
    assert_eq!(reparsed.encode(), module.encode());
    assert_eq!(reparsed.unified_text().unwrap(), printed);
    assert_eq!(run(&reparsed, &[], Limits::default()), Ok(list(bits)));
    assert_eq!(
        Module::decode(&module.encode())
            .unwrap()
            .unified_text()
            .unwrap(),
        printed
    );
}

#[test]
fn decimal_literals_keep_conversion_boundaries_native_spans_and_injected_value_bits() {
    let (before, after) = pair(
        "fluxr 1\npub run 0 int ret ftoi64 0x7f7ffffff",
        "fluxr 1\npub run 0 int ret ftoi64 3.4028235e38f",
    );
    // The existing float-to-int conversion saturates; checked arithmetic
    // following that conversion still traps on integer overflow.
    assert_eq!(
        run(&after, &[], Limits::default()),
        Ok(Value::I64(i64::MAX))
    );
    same_budgets(&before, &after, &[]);
    let (before, after) = pair(
        "fluxr 1\npub run 0 int ret add ftoi64 0x7f7ffffff 1",
        "fluxr 1\npub run 0 int ret add ftoi64 3.4028235e38f 1",
    );
    assert!(run(&after, &[], Limits::default()).is_err());
    same_budgets(&before, &after, &[]);

    let native = "native-fn spelling 0 \"&'static str\" 1 tail str \"1f -0f\"";
    let raw_text = format!("{}\n{native}\n", PAIRS[0].0.trim_end());
    let decimal_text = format!("{}\n{native}\n", PAIRS[0].1.trim_end());
    let a = SourceDocument::parse(&raw_text).unwrap();
    let b = SourceDocument::parse(&decimal_text).unwrap();
    assert_eq!(a.source, b.source);
    assert_eq!(a.native_digest, b.native_digest);
    assert_eq!(a.native_spans.len(), 1);
    assert_eq!(b.native_spans.len(), 1);
    for (document, text) in [(&a, &raw_text), (&b, &decimal_text)] {
        let span = &document.native_spans[0];
        assert_eq!(&text[span.start..span.end], native);
    }

    let runtime = Runtime::new();
    runtime
        .register_host("echo", vec![Type::F32], Type::F32, |arguments| {
            Ok(arguments[0].clone())
        })
        .unwrap();
    let raw = "fluxr 1\npub run 0 float ret host echo float 1 float 0x80000000f";
    let decimal = "fluxr 1\npub run 0 float ret host echo float 1 float -0f";
    runtime.install("literal", None, raw).unwrap();
    let old = runtime.snapshot("literal").unwrap();
    runtime.install("literal", Some(1), decimal).unwrap();
    assert_eq!(
        old.call("run", &[], &[], Limits::default()).unwrap().value,
        Value::F32(0x8000_0000)
    );
    assert_eq!(
        runtime
            .call("literal", "run", &[], &[], Limits::default())
            .unwrap()
            .value,
        Value::F32(0x8000_0000)
    );
}
