//! Current-source shorthand must preserve the full expanded checked program.
use fluxr::runtime::{Runtime, Value};
use fluxr::structured::{Invocation, Limits, Module, Source, SourceDocument, Workspace};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[path = "../benchmarks/unified-updates-v1/float.rs"]
mod float_reference;
#[path = "../benchmarks/unified-updates-v1/loop.rs"]
mod loop_reference;
#[path = "../benchmarks/unified-updates-v1/signed.rs"]
mod signed_reference;
#[path = "../benchmarks/unified-updates-v1/unsigned.rs"]
mod unsigned_reference;

const SIGNED: (&str, &str) = (
    include_str!("../benchmarks/unified-updates-v1/signed.before.fluxr"),
    include_str!("../benchmarks/unified-updates-v1/signed.after.fluxr"),
);
const UNSIGNED: (&str, &str) = (
    include_str!("../benchmarks/unified-updates-v1/unsigned.before.fluxr"),
    include_str!("../benchmarks/unified-updates-v1/unsigned.after.fluxr"),
);
const FLOAT: (&str, &str) = (
    include_str!("../benchmarks/unified-updates-v1/float.before.fluxr"),
    include_str!("../benchmarks/unified-updates-v1/float.after.fluxr"),
);
const LOOP: (&str, &str) = (
    include_str!("../benchmarks/unified-updates-v1/loop.before.fluxr"),
    include_str!("../benchmarks/unified-updates-v1/loop.after.fluxr"),
);

fn pair(before: &str, after: &str) -> (Module, Module) {
    let long = SourceDocument::parse(before).unwrap();
    let short = SourceDocument::parse(after).unwrap();
    assert_eq!(long.source, short.source);
    let (long, short) = (long.compile().unwrap(), short.compile().unwrap());
    assert_eq!(long.encode(), short.encode());
    assert_eq!(long.unified_text().unwrap(), short.unified_text().unwrap());
    assert_eq!(
        SourceDocument::parse(&short.unified_text().unwrap())
            .unwrap()
            .compile()
            .unwrap()
            .encode(),
        long.encode()
    );
    (long, short)
}

fn run(
    module: &Module,
    export: &str,
    arguments: &[Value],
    limits: Limits,
) -> Result<Value, String> {
    module
        .run_portable_with(
            Invocation {
                export,
                inputs: &[],
                arguments,
                limits,
            },
            &mut Workspace::default(),
        )
        .map_err(|error| error.to_string())
}

fn same_budgets(long: &Module, short: &Module, export: &str, arguments: &[Value], fuel: u64) {
    let prepared = short.prepare_call(export).unwrap();
    let mut workspace = Workspace::default();
    for instructions in 0..fuel {
        for cells in 0..8 {
            for frames in 0..3 {
                let limits = Limits {
                    instructions,
                    cells,
                    frames,
                };
                let expected = run(long, export, arguments, limits);
                assert_eq!(run(short, export, arguments, limits), expected);
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

#[test]
fn numeric_update_pairs_preserve_values_traps_and_all_small_budget_outcomes() {
    let (long, short) = pair(SIGNED.0, SIGNED.1);
    assert_eq!(short.unified_text().unwrap(), SIGNED.1.trim_end());
    for (x, y) in [
        (2, 5),
        (-7, 3),
        (i64::MAX, 1),
        (i64::MAX / 2, 0),
        (i64::MIN, -1),
    ] {
        let args = [Value::I64(x), Value::I64(y)];
        assert_eq!(
            run(&short, "calc", &args, Limits::default()).ok(),
            signed_reference::run(x, y).map(Value::I64)
        );
        same_budgets(&long, &short, "calc", &args, 30);
    }
    let (long, short) = pair(UNSIGNED.0, UNSIGNED.1);
    for (x, y) in [(2, 5), (0, 0), (u64::MAX, 1), (u64::MAX / 2, 0)] {
        let args = [Value::U64(x), Value::U64(y)];
        assert_eq!(
            run(&short, "calc", &args, Limits::default()).ok(),
            unsigned_reference::run(x, y).map(Value::U64)
        );
        same_budgets(&long, &short, "calc", &args, 30);
    }
    let (long, short) = pair(FLOAT.0, FLOAT.1);
    for (x, y) in [
        (0x3f80_0000, 0x4000_0000),
        (0x8000_0000, 0x8000_0000),
        (0x7fa0_0001, 0x3f80_0000),
        (0x7f80_0000, 0xff80_0000),
        (1, 0x8000_0001),
    ] {
        let args = [Value::F32(x), Value::F32(y)];
        assert_eq!(
            run(&short, "calc", &args, Limits::default()).unwrap(),
            Value::F32(float_reference::run(x, y))
        );
        same_budgets(&long, &short, "calc", &args, 30);
    }
}

#[test]
fn loop_updates_keep_iteration_order_and_dynamic_metering() {
    let (long, short) = pair(LOOP.0, LOOP.1);
    assert_eq!(
        short.unified_text().unwrap(),
        include_str!("../benchmarks/unified-steps-v1/ascending.after.fluxr").trim_end()
    );
    for n in [-1, 0, 1, 4] {
        let args = [Value::I64(n)];
        assert_eq!(
            run(&short, "sum", &args, Limits::default()).ok(),
            loop_reference::run(n).map(Value::I64)
        );
        same_budgets(&long, &short, "sum", &args, 100);
    }
}

#[test]
fn keyword_names_and_nonmatching_assignments_do_not_rebind_operands() {
    for name in ["add", "mul", "local", "host", "bump", "scale"] {
        let long = format!(
            "fluxr 1\npub calc 1 {name} int int do 0 3 set {name} add local {name} 2 set {name} mul local {name} 3 ret local {name}"
        );
        let short = format!(
            "fluxr 1\npub calc 1 {name} int int do 0 3 bump {name} 2 scale {name} 3 ret local {name}"
        );
        let (_, module) = pair(&long, &short);
        assert_eq!(
            run(&module, "calc", &[Value::I64(4)], Limits::default()).unwrap(),
            Value::I64(18)
        );
    }
    for expression in ["add y x", "add y 1", "sub y x", "mul y x"] {
        let text = format!("fluxr 1\npub calc 2 x int y int int do 0 2 set x {expression} ret x");
        let module = SourceDocument::parse(&text).unwrap().compile().unwrap();
        assert_eq!(module.unified_text().unwrap(), text);
    }
}

#[test]
fn source_shorthand_preserves_legacy_readers_and_rejects_malformed_updates() {
    let (_, module) = pair(SIGNED.0, SIGNED.1);
    let legacy = module.agent_text().unwrap();
    assert!(legacy.contains("set x add x y set x mul x 3"));
    assert_eq!(
        Module::compile_agent(&legacy).unwrap().encode(),
        module.encode()
    );
    for update in ["bump x 1", "scale x 2"] {
        assert!(Source::parse_agent(&format!("fa1 v2 pub f 1 x int int do 0 1 {update}")).is_err());
    }
    for update in [
        "bump",
        "bump x",
        "scale x",
        "bump 1 2",
        "scale x ?",
        "bump x 1 extra",
    ] {
        assert!(
            SourceDocument::parse(&format!("fluxr 1\npub f 1 x int int do 0 1 {update}")).is_err()
        );
    }
    for update in ["bump missing 1", "scale x true"] {
        assert!(
            SourceDocument::parse(&format!("fluxr 1\npub f 1 x int int do 0 2 {update} ret x"))
                .unwrap()
                .compile()
                .is_err()
        );
    }
}

#[test]
fn target_read_precedes_rhs_traps_and_injected_native_service_effects() {
    for (short, long) in [
        ("bump x div 1 0", "set x add x div 1 0"),
        ("scale x div 1 0", "set x mul x div 1 0"),
    ] {
        let before = format!("fluxr 1\npub calc 0 int do 1 x int 2 {long} ret x");
        let after = format!("fluxr 1\npub calc 0 int do 1 x int 2 {short} ret x");
        let (long, short) = pair(&before, &after);
        assert!(
            run(&short, "calc", &[], Limits::default())
                .unwrap_err()
                .contains("uninitialized")
        );
        same_budgets(&long, &short, "calc", &[], 20);
    }
    let runtime = Runtime::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&calls);
    runtime
        .register_host("effect", vec![], fluxr::runtime::Type::I64, move |_| {
            observed.fetch_add(1, Ordering::SeqCst);
            Ok(Value::I64(2))
        })
        .unwrap();
    for (index, update) in ["bump x host effect int 0", "set x add x host effect int 0"]
        .into_iter()
        .enumerate()
    {
        let name = format!("update{index}");
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
        assert_eq!(calls.load(Ordering::SeqCst), index);
        runtime
            .install(
                &name,
                Some(1),
                &format!("fluxr 1\npub calc 0 int do 1 x int 3 set x 5 {update} ret x"),
            )
            .unwrap();
        assert_eq!(
            runtime
                .call(&name, "calc", &[], &[], Limits::default())
                .unwrap()
                .value,
            Value::I64(7)
        );
        assert_eq!(calls.load(Ordering::SeqCst), index + 1);
    }
}
