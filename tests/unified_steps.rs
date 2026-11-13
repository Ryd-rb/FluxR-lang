//! Signed unit steps preserve their existing expanded program and all charges.
use fluxr::runtime::Runtime;
use fluxr::structured::{Invocation, Limits, Module, SourceDocument, Type, Value, Workspace};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[path = "../benchmarks/unified-updates-v1/loop.rs"]
mod ascending_reference;
#[path = "../benchmarks/unified-steps-v1/decrement.rs"]
mod decrement_reference;
#[path = "../benchmarks/unified-steps-v1/descending.rs"]
mod descending_reference;
#[path = "../benchmarks/unified-steps-v1/increment.rs"]
mod increment_reference;

const INCREMENT: (&str, &str) = (
    include_str!("../benchmarks/unified-steps-v1/increment.before.fluxr"),
    include_str!("../benchmarks/unified-steps-v1/increment.after.fluxr"),
);
const DECREMENT: (&str, &str) = (
    include_str!("../benchmarks/unified-steps-v1/decrement.before.fluxr"),
    include_str!("../benchmarks/unified-steps-v1/decrement.after.fluxr"),
);
const ASCENDING: (&str, &str) = (
    include_str!("../benchmarks/unified-updates-v1/loop.after.fluxr"),
    include_str!("../benchmarks/unified-steps-v1/ascending.after.fluxr"),
);
const DESCENDING: (&str, &str) = (
    include_str!("../benchmarks/unified-steps-v1/descending.before.fluxr"),
    include_str!("../benchmarks/unified-steps-v1/descending.after.fluxr"),
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
fn unit_steps_match_checked_rust_boundaries_and_expanded_resource_outcomes() {
    type Reference = fn(i64) -> Option<i64>;
    for (texts, reference) in [
        (INCREMENT, increment_reference::run as Reference),
        (DECREMENT, decrement_reference::run as Reference),
    ] {
        let (long, short) = pair(texts.0, texts.1);
        assert_eq!(short.unified_text().unwrap(), texts.1.trim_end());
        for x in [i64::MIN, i64::MIN + 1, -1, 0, 1, i64::MAX - 1, i64::MAX] {
            let args = [Value::I64(x)];
            assert_eq!(
                run(&short, "step", &args, Limits::default()).ok(),
                reference(x).map(Value::I64)
            );
            same_budgets(&long, &short, "step", &args, 16);
        }
        let legacy = short.agent_text().unwrap();
        assert!(!legacy.contains(" next ") && !legacy.contains(" prev "));
        assert_eq!(
            Module::compile_agent(&legacy).unwrap().encode(),
            short.encode()
        );
    }
}

#[test]
fn ascending_and_descending_steps_preserve_loop_order_and_dynamic_budgets() {
    type Reference = fn(i64) -> Option<i64>;
    for (texts, reference) in [
        (ASCENDING, ascending_reference::run as Reference),
        (DESCENDING, descending_reference::run as Reference),
    ] {
        let (long, short) = pair(texts.0, texts.1);
        assert_eq!(short.unified_text().unwrap(), texts.1.trim_end());
        for n in [-1, 0, 1, 4] {
            let args = [Value::I64(n)];
            assert_eq!(
                run(&short, "sum", &args, Limits::default()).ok(),
                reference(n).map(Value::I64)
            );
            same_budgets(&long, &short, "sum", &args, 64);
        }
    }
}

#[test]
fn step_keywords_remain_local_names_and_printer_requires_exact_signed_ast() {
    for name in [
        "next", "prev", "local", "host", "add", "sub", "bump", "scale",
    ] {
        let before = format!(
            "fluxr 1\npub step 1 {name} int int do 0 3 set {name} add local {name} 1 set {name} sub local {name} 1 ret local {name}"
        );
        let after = format!(
            "fluxr 1\npub step 1 {name} int int do 0 3 next {name} prev {name} ret local {name}"
        );
        let (_, module) = pair(&before, &after);
        assert_eq!(
            run(&module, "step", &[Value::I64(7)], Limits::default()),
            Ok(Value::I64(7))
        );
    }
    for (ty, update) in [
        ("uint", "bump x 1u"),
        ("float", "bump x 0x3f800000f"),
        ("int", "bump x -1"),
        ("int", "scale x 1"),
        ("int", "set x sub x 2"),
        ("int", "set x add 1 x"),
    ] {
        let text = format!("fluxr 1\npub step 1 x {ty} {ty} do 0 2 {update} ret x");
        let module = SourceDocument::parse(&text).unwrap().compile().unwrap();
        // Float spelling can shorten independently; the update must never
        // become an implicit signed unit step or change its operand type.
        assert_eq!(
            module.unified_text().unwrap(),
            text.replace("0x3f800000f", "1f")
                .replace("set x sub x 2", "sub x 2")
        );
    }
    for expression in ["add y 1", "sub y 1"] {
        let text = format!("fluxr 1\npub step 2 x int y int int do 0 2 set x {expression} ret x");
        let module = SourceDocument::parse(&text).unwrap().compile().unwrap();
        assert_eq!(module.unified_text().unwrap(), text);
    }
}

#[test]
fn step_admission_rejects_missing_names_extra_operands_and_implicit_type_changes() {
    for update in [
        "next",
        "prev",
        "next 1",
        "prev ?",
        "next x 1",
        "prev x extra",
    ] {
        let text = format!("fluxr 1\npub step 1 x int int do 0 1 {update}");
        assert!(SourceDocument::parse(&text).is_err(), "accepted {text}");
    }
    for update in ["next missing", "prev missing"] {
        let text = format!("fluxr 1\npub step 1 x int int do 0 2 {update} ret x");
        assert!(SourceDocument::parse(&text).unwrap().compile().is_err());
    }
    for ty in ["uint", "float", "bool"] {
        for update in ["next x", "prev x"] {
            let text = format!("fluxr 1\npub step 1 x {ty} {ty} do 0 2 {update} ret x");
            assert!(
                SourceDocument::parse(&text).unwrap().compile().is_err(),
                "accepted {text}"
            );
        }
    }
    for update in ["next x", "prev x"] {
        assert!(
            Module::compile_agent(&format!(
                "fa1 v2 pub step 1 x int int do 0 2 {update} ret x"
            ))
            .is_err()
        );
    }
}

#[test]
fn step_traps_keep_uninitialized_reads_and_later_host_effects_in_order() {
    for (short, long, overflow) in [
        ("next x", "set x add local x 1", i64::MAX),
        ("prev x", "set x sub local x 1", i64::MIN),
    ] {
        let before = format!("fluxr 1\npub step 0 int do 1 x int 2 {long} ret x");
        let after = format!("fluxr 1\npub step 0 int do 1 x int 2 {short} ret x");
        let (long_module, short_module) = pair(&before, &after);
        let expected = run(&long_module, "step", &[], Limits::default());
        assert!(expected.as_ref().unwrap_err().contains("uninitialized"));
        assert_eq!(run(&short_module, "step", &[], Limits::default()), expected);

        let runtime = Runtime::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&calls);
        runtime
            .register_host("effect", vec![], Type::I64, move |_| {
                observed.fetch_add(1, Ordering::SeqCst);
                Ok(Value::I64(7))
            })
            .unwrap();
        let mut errors = Vec::new();
        for (name, update) in [("expanded", long), ("step", short)] {
            let text = format!(
                "fluxr 1\npub step 1 x int int do 0 3 {update} set x host effect int 0 ret x"
            );
            runtime.install(name, None, &text).unwrap();
            errors.push(
                runtime
                    .call(
                        name,
                        "step",
                        &[],
                        &[Value::I64(overflow)],
                        Limits::default(),
                    )
                    .unwrap_err()
                    .to_string(),
            );
        }
        assert_eq!(errors[0], errors[1]);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn mixed_native_documents_and_runtime_replacement_keep_exact_identities() {
    let native = "native-fn greeting 0 \"&'static str\" 1 tail str \"héllo β\"";
    let before = format!("{}\n{native}\n", INCREMENT.0.trim_end());
    let after = format!("{}\n{native}\n", INCREMENT.1.trim_end());
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
    runtime.install("step", None, INCREMENT.0).unwrap();
    let old = runtime.snapshot("step").unwrap();
    runtime.install("step", Some(1), INCREMENT.1).unwrap();
    let args = [Value::I64(9)];
    assert_eq!(
        old.call("step", &[], &args, Limits::default())
            .unwrap()
            .value,
        Value::I64(10)
    );
    assert_eq!(
        runtime
            .call("step", "step", &[], &args, Limits::default())
            .unwrap()
            .value,
        Value::I64(10)
    );
}
