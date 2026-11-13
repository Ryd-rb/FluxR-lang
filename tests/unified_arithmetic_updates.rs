//! Arithmetic statement shorthand preserves exact checked expressions and costs.
use fluxr::runtime::Runtime;
use fluxr::structured::wordcode::{Dictionary, ExpansionLimits};
use fluxr::structured::{Invocation, Limits, Module, SourceDocument, Type, Value, Workspace};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[path = "../benchmarks/unified-arithmetic-v1/division.rs"]
mod division_reference;
#[path = "../benchmarks/unified-arithmetic-v1/float.rs"]
mod float_reference;
#[path = "../benchmarks/unified-arithmetic-v1/loop.rs"]
mod loop_reference;
#[path = "../benchmarks/unified-arithmetic-v1/signed.rs"]
mod signed_reference;
#[path = "../benchmarks/unified-arithmetic-v1/unsigned.rs"]
mod unsigned_reference;

const PAIRS: [(&str, &str); 5] = [
    (
        include_str!("../benchmarks/unified-arithmetic-v1/signed.before.fluxr"),
        include_str!("../benchmarks/unified-arithmetic-v1/signed.after.fluxr"),
    ),
    (
        include_str!("../benchmarks/unified-arithmetic-v1/unsigned.before.fluxr"),
        include_str!("../benchmarks/unified-arithmetic-v1/unsigned.after.fluxr"),
    ),
    (
        include_str!("../benchmarks/unified-arithmetic-v1/float.before.fluxr"),
        include_str!("../benchmarks/unified-arithmetic-v1/float.after.fluxr"),
    ),
    (
        include_str!("../benchmarks/unified-arithmetic-v1/division.before.fluxr"),
        include_str!("../benchmarks/unified-arithmetic-v1/division.after.fluxr"),
    ),
    (
        include_str!("../benchmarks/unified-arithmetic-v1/loop.before.fluxr"),
        include_str!("../benchmarks/unified-arithmetic-v1/loop.after.fluxr"),
    ),
];

fn pair(before: &str, after: &str) -> (Module, Module) {
    let long = SourceDocument::parse(before).unwrap();
    let short = SourceDocument::parse(after).unwrap();
    assert_eq!(long.source, short.source);
    let (long, short) = (long.compile().unwrap(), short.compile().unwrap());
    assert_eq!(long.encode(), short.encode());
    let printed = short.unified_text().unwrap();
    assert_eq!(long.unified_text().unwrap(), printed);
    assert_eq!(
        SourceDocument::parse(&printed)
            .unwrap()
            .compile()
            .unwrap()
            .encode(),
        long.encode()
    );
    assert_eq!(
        Module::compile_agent(&short.agent_text().unwrap())
            .unwrap()
            .encode(),
        long.encode()
    );
    (long, short)
}

fn run(module: &Module, export: &str, args: &[Value], limits: Limits) -> Result<Value, String> {
    module
        .run_portable_with(
            Invocation {
                export,
                inputs: &[],
                arguments: args,
                limits,
            },
            &mut Workspace::default(),
        )
        .map_err(|error| error.to_string())
}

fn same_budgets(long: &Module, short: &Module, export: &str, args: &[Value], max_fuel: u64) {
    let prepared = short.prepare_call(export).unwrap();
    let mut workspace = Workspace::default();
    for instructions in 0..=max_fuel {
        for cells in 0..=10 {
            for frames in 0..=1 {
                let limits = Limits {
                    instructions,
                    cells,
                    frames,
                };
                let expected = run(long, export, args, limits);
                assert_eq!(run(short, export, args, limits), expected);
                assert_eq!(
                    prepared
                        .run_with(&[], args, limits, &mut workspace)
                        .map_err(|error| error.to_string()),
                    expected,
                    "args={args:?}, limits={limits:?}"
                );
            }
        }
    }
}

#[test]
fn fixture_pairs_preserve_canonical_bytes_and_released_readers() {
    for (before, after) in PAIRS {
        let (_, module) = pair(before, after);
        assert_eq!(module.unified_text().unwrap(), after.trim_end());
    }
    for update in ["sub x 2", "div x 2", "rem x 2"] {
        let body = format!("pub calc 1 x int int do 0 2 {update} ret x");
        assert!(Module::compile_agent(&format!("fa1 v2 {body}")).is_err());
        assert!(
            Module::compile_wordcode(
                &format!("wk1 core v2 {body}"),
                &Dictionary::core(),
                ExpansionLimits::default(),
            )
            .is_err()
        );
    }
}

#[test]
fn integer_pairs_preserve_checked_rust_results_and_all_small_budgets() {
    let (long, short) = pair(PAIRS[0].0, PAIRS[0].1);
    for (x, y) in [(30, 3), (-30, 3), (0, 0), (i64::MIN, 1), (i64::MAX, -1)] {
        let args = [Value::I64(x), Value::I64(y)];
        assert_eq!(
            run(&short, "calc", &args, Limits::default()).ok(),
            signed_reference::run(x, y).map(Value::I64)
        );
        same_budgets(&long, &short, "calc", &args, 30);
    }
    let (long, short) = pair(PAIRS[1].0, PAIRS[1].1);
    for (x, y) in [(30, 3), (0, 0), (0, 1), (u64::MAX, 0)] {
        let args = [Value::U64(x), Value::U64(y)];
        assert_eq!(
            run(&short, "calc", &args, Limits::default()).ok(),
            unsigned_reference::run(x, y).map(Value::U64)
        );
        same_budgets(&long, &short, "calc", &args, 30);
    }
}

#[test]
fn division_and_remainder_keep_zero_and_signed_overflow_guards() {
    for op in ["div", "rem"] {
        let before = PAIRS[3].0.replace("div", op);
        let after = PAIRS[3].1.replace("div", op);
        let (long, short) = pair(&before, &after);
        for (x, y) in [
            (17, 3),
            (-17, 3),
            (17, -3),
            (i64::MIN, -1),
            (i64::MIN, 1),
            (9, 0),
        ] {
            let expected = if op == "div" {
                division_reference::run(x, y)
            } else {
                x.checked_rem(y)
            };
            let args = [Value::I64(x), Value::I64(y)];
            assert_eq!(
                run(&short, "calc", &args, Limits::default()).ok(),
                expected.map(Value::I64)
            );
            same_budgets(&long, &short, "calc", &args, 18);
        }
    }
}

#[test]
fn binary32_updates_preserve_rounding_signed_zero_and_canonical_nan() {
    let (long, short) = pair(PAIRS[2].0, PAIRS[2].1);
    for (x, y) in [
        (0x3f80_0000, 0x4000_0000),
        (0x8000_0000, 0),
        (0x8000_0000, 0x8000_0000),
        (0x7fa0_0001, 0x3f80_0000),
        (0x7f80_0000, 0x7f80_0000),
        (1, 0x8000_0001),
    ] {
        let args = [Value::F32(x), Value::F32(y)];
        assert_eq!(
            run(&short, "calc", &args, Limits::default()).unwrap(),
            Value::F32(float_reference::run(x, y))
        );
        same_budgets(&long, &short, "calc", &args, 24);
    }
    // Floating division by zero is IEEE binary32 behavior, not an integer trap.
    let (long, short) = pair(
        "fluxr 1\npub calc 1 x float float do 0 2 set x div x 0f ret x",
        "fluxr 1\npub calc 1 x float float do 0 2 div x 0f ret x",
    );
    for (input, expected) in [(0x3f80_0000, 0x7f80_0000), (0, 0x7fc0_0000)] {
        let args = [Value::F32(input)];
        assert_eq!(
            run(&short, "calc", &args, Limits::default()),
            Ok(Value::F32(expected))
        );
        same_budgets(&long, &short, "calc", &args, 18);
    }
}

#[test]
fn loop_subtraction_preserves_iteration_order_and_metering() {
    let (long, short) = pair(PAIRS[4].0, PAIRS[4].1);
    for n in [-1, 0, 1, 4] {
        let args = [Value::I64(n)];
        assert_eq!(
            run(&short, "reduce", &args, Limits::default()).ok(),
            loop_reference::run(n).map(Value::I64)
        );
        same_budgets(&long, &short, "reduce", &args, 80);
    }
}

#[test]
fn names_and_printer_matching_do_not_reorder_or_rebind_operands() {
    for name in ["sub", "div", "rem", "local", "host", "next", "prev"] {
        for op in ["sub", "div", "rem"] {
            let before = format!(
                "fluxr 1\npub calc 1 {name} int int do 0 2 set {name} {op} local {name} 2 ret local {name}"
            );
            let after =
                format!("fluxr 1\npub calc 1 {name} int int do 0 2 {op} {name} 2 ret local {name}");
            let (long, short) = pair(&before, &after);
            assert_eq!(
                run(&short, "calc", &[Value::I64(7)], Limits::default()),
                run(&long, "calc", &[Value::I64(7)], Limits::default())
            );
        }
    }
    for expression in [
        "sub y x", "div y x", "rem y x", "sub 2 x", "div 2 x", "rem 2 x",
    ] {
        let text = format!("fluxr 1\npub calc 2 x int y int int do 0 2 set x {expression} ret x");
        let module = SourceDocument::parse(&text).unwrap().compile().unwrap();
        assert_eq!(module.unified_text().unwrap(), text);
    }
    let (_, module) = pair(
        "fluxr 1\npub calc 1 x int int do 0 2 set x sub x 1 ret x",
        "fluxr 1\npub calc 1 x int int do 0 2 sub x 1 ret x",
    );
    assert!(module.unified_text().unwrap().contains("prev x"));
}

#[test]
fn malformed_undefined_and_ill_typed_updates_fail() {
    for op in ["sub", "div", "rem"] {
        for update in [
            op.to_owned(),
            format!("{op} x"),
            format!("{op} 1 2"),
            format!("{op} x ?"),
            format!("{op} x 2 extra"),
        ] {
            let source = format!("fluxr 1\npub calc 1 x int int do 0 1 {update}");
            assert!(SourceDocument::parse(&source).is_err(), "accepted {source}");
        }
        for update in [format!("{op} missing 2"), format!("{op} x true")] {
            let source = format!("fluxr 1\npub calc 1 x int int do 0 2 {update} ret x");
            assert!(SourceDocument::parse(&source).unwrap().compile().is_err());
        }
    }
    for (ty, rhs) in [("bool", "false"), ("float", "1f")] {
        let source = format!("fluxr 1\npub calc 1 x {ty} {ty} do 0 2 rem x {rhs} ret x");
        assert!(SourceDocument::parse(&source).unwrap().compile().is_err());
    }
}

#[test]
fn target_read_precedes_rhs_traps_and_native_service_effects() {
    let runtime = Runtime::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&calls);
    runtime
        .register_host("effect", vec![], Type::I64, move |_| {
            observed.fetch_add(1, Ordering::SeqCst);
            Ok(Value::I64(2))
        })
        .unwrap();
    for op in ["sub", "div", "rem"] {
        let before = format!("fluxr 1\npub calc 0 int do 1 x int 2 set x {op} x div 1 0 ret x");
        let after = format!("fluxr 1\npub calc 0 int do 1 x int 2 {op} x div 1 0 ret x");
        let (long, short) = pair(&before, &after);
        assert!(
            run(&short, "calc", &[], Limits::default())
                .unwrap_err()
                .contains("uninitialized")
        );
        same_budgets(&long, &short, "calc", &[], 18);
        for (index, update) in [
            format!("set x {op} x host effect int 0"),
            format!("{op} x host effect int 0"),
        ]
        .into_iter()
        .enumerate()
        {
            let name = format!("{op}{index}");
            runtime
                .install(
                    &name,
                    None,
                    &format!("fluxr 1\npub calc 0 int do 1 x int 2 {update} ret x"),
                )
                .unwrap();
            assert!(
                runtime
                    .call(&name, "calc", &[], &[], Limits::default())
                    .unwrap_err()
                    .message
                    .contains("uninitialized")
            );
        }
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    // An earlier arithmetic failure also prevents a later effectful statement.
    for (index, update) in ["set x div x 0", "div x 0"].into_iter().enumerate() {
        let name = format!("trap{index}");
        runtime
            .install(
                &name,
                None,
                &format!(
                    "fluxr 1\npub calc 1 x int int do 0 3 {update} set x host effect int 0 ret x"
                ),
            )
            .unwrap();
        assert!(
            runtime
                .call(&name, "calc", &[], &[Value::I64(7)], Limits::default())
                .is_err()
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn mixed_native_documents_and_live_replacement_keep_checked_identities() {
    let native = "native-fn greeting 0 \"&'static str\" 1 tail str \"héllo β\"";
    let before = format!("{}\n{native}\n", PAIRS[0].0.trim_end());
    let after = format!("{}\n{native}\n", PAIRS[0].1.trim_end());
    let a = SourceDocument::parse(&before).unwrap();
    let b = SourceDocument::parse(&after).unwrap();
    assert_eq!(a.source, b.source);
    assert_eq!(a.compile().unwrap().encode(), b.compile().unwrap().encode());
    assert_eq!(a.native_digest, b.native_digest);
    for (document, text) in [(&a, &before), (&b, &after)] {
        assert_eq!(document.native_spans.len(), 1);
        let span = &document.native_spans[0];
        assert_eq!(&text[span.start..span.end], native);
    }
    let runtime = Runtime::new();
    runtime.install("calc", None, PAIRS[0].0).unwrap();
    let old = runtime.snapshot("calc").unwrap();
    runtime.install("calc", Some(1), PAIRS[0].1).unwrap();
    let args = [Value::I64(30), Value::I64(3)];
    let expected = Value::I64(signed_reference::run(30, 3).unwrap());
    assert_eq!(
        old.call("calc", &[], &args, Limits::default())
            .unwrap()
            .value,
        expected
    );
    assert_eq!(
        runtime
            .call("calc", "calc", &[], &args, Limits::default())
            .unwrap()
            .value,
        expected
    );
}
