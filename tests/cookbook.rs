//! Every shipped cookbook pair executes its exact independent Rust reference.
#![allow(unexpected_cfgs)]

#[path = "../examples/cookbook/support.rs"]
mod support;

use fluxr::structured::{Limits, Module, SourceDocument, Value, Workspace};
use serde_json::Value as Json;
use support::{Case, Outcome, Row};

fn result(value: Result<Value, fluxr::Error>) -> Outcome {
    value.map_err(|error| error.to_string())
}

fn ownership(case: &Case, row: &Row, actual: &Outcome) {
    match (case.name, actual, row.arguments.first()) {
        ("record_update", Ok(Value::Record(updated)), Some(Value::Record(original))) => {
            assert!(
                !std::sync::Arc::ptr_eq(updated, original),
                "shared record update must detach"
            );
        }
        (
            "list_push_pop" | "list_reverse",
            Ok(Value::List(updated)),
            Some(Value::List(original)),
        ) => {
            assert!(
                !std::sync::Arc::ptr_eq(updated, original),
                "list result must own its changed allocation"
            );
        }
        _ => {}
    }
}

fn verify_case(case: &Case) {
    let current = case
        .compile()
        .unwrap_or_else(|error| panic!("{} current source: {error}", case.name));
    let artifact = current.encode();
    let compact = Module::compile_compact(case.compact)
        .unwrap_or_else(|error| panic!("{} compact baseline: {error}", case.name));
    assert_eq!(compact.encode(), artifact, "{} compact identity", case.name);
    let decoded = Module::decode(&artifact).unwrap();
    let printed = SourceDocument::parse(&current.unified_text().unwrap())
        .unwrap()
        .compile()
        .unwrap();
    let legacy = Module::compile_agent(&current.agent_text().unwrap()).unwrap();
    let compact_printed = Module::compile_compact(&current.compact_text().unwrap()).unwrap();
    for module in [&decoded, &printed, &legacy, &compact_printed] {
        assert_eq!(module.encode(), artifact, "{} reader identity", case.name);
    }
    let identity = case.identity();
    for row in &case.rows {
        let mut kernel = (case.kernel)(row);
        assert_eq!(
            support::kernel_outcome(kernel()),
            (case.oracle)(row),
            "{} typed Rust kernel adapter",
            case.name
        );
    }
    for module in [
        &current,
        &compact,
        &decoded,
        &printed,
        &legacy,
        &compact_printed,
    ] {
        let prepared = module.prepare_call("run").unwrap();
        let mut portable_workspace = Workspace::default();
        let mut automatic_workspace = Workspace::default();
        let mut prepared_workspace = Workspace::default();
        for (index, row) in case.rows.iter().enumerate() {
            let expected = (case.oracle)(row);
            let portable = result(
                module
                    .run_portable_with(row.invocation(Limits::default()), &mut portable_workspace),
            );
            ownership(case, row, &portable);
            assert_eq!(
                portable, expected,
                "{} row {index}, portable, {row:?}",
                case.name
            );
            let automatic = result(
                module.run_with(row.invocation(Limits::default()), &mut automatic_workspace),
            );
            ownership(case, row, &automatic);
            assert_eq!(
                automatic, expected,
                "{} row {index}, automatic, {row:?}",
                case.name
            );
            let native = result(prepared.run_with(
                &row.inputs,
                &row.arguments,
                Limits::default(),
                &mut prepared_workspace,
            ));
            ownership(case, row, &native);
            assert_eq!(
                native, expected,
                "{} row {index}, prepared, {row:?}",
                case.name
            );
        }
    }
    // Retained host values must survive both successful COW writes and traps.
    assert_eq!(
        case.identity(),
        identity,
        "{} changed host inputs",
        case.name
    );
}

macro_rules! example {
    ($($name:ident),* $(,)?) => {$ (
        #[test]
        fn $name() {
            let case = support::cases().into_iter().find(|case|case.name==stringify!($name)).unwrap();
            verify_case(&case);
        }
    )*};
}

example!(
    unit,
    checked_affine,
    unsigned_affine,
    clamp,
    guarded_division,
    gcd,
    factorial,
    fibonacci,
    power,
    triangular,
    loop_exits,
    helper_calls,
    recursive_sum,
    list_sum,
    list_count,
    list_reverse,
    list_contains,
    list_last,
    list_push_pop,
    list_index,
    record_point,
    record_update,
    nested_record,
    method_shift,
    enum_option,
    enum_map,
    float_affine,
    float_sqrt,
    float_to_int,
    int_to_float,
    uint_to_float,
    float_minimum,
    input_adjust,
    five_arguments,
    uninitialized_order,
    tagged_payload,
    unsigned_division,
    enum_list,
    nested_list,
    float_absolute,
    boolean_logic,
);

#[test]
fn every_catalog_example_has_exact_sources_and_an_executed_cli_row() {
    let catalog: Json =
        serde_json::from_str(include_str!("../examples/cookbook/catalog.json")).unwrap();
    let suite: Json =
        serde_json::from_str(include_str!("../examples/cookbook/tests.json")).unwrap();
    let cases = support::cases();
    let entries = catalog["cases"].as_array().unwrap();
    assert_eq!(catalog["format"], "fluxr-cookbook-v1");
    assert_eq!(suite["format"], "fluxr-tests-v1");
    assert_eq!(entries.len(), cases.len());
    let mut names = std::collections::BTreeSet::new();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/cookbook");
    for entry in entries {
        let name = entry["name"].as_str().unwrap();
        assert!(names.insert(name));
        let case = cases.iter().find(|case| case.name == name).unwrap();
        for (field, exact) in [
            ("fluxr", case.source),
            ("rust", case.rust_source),
            ("compact", case.compact),
        ] {
            assert_eq!(
                std::fs::read_to_string(root.join(entry[field].as_str().unwrap())).unwrap(),
                exact
            );
        }
        assert_eq!(entry["topics"], serde_json::json!(case.topics));
        assert!(
            suite["tests"]
                .as_array()
                .unwrap()
                .iter()
                .any(|test| test["source"] == entry["fluxr"])
        );
    }
    for test in suite["tests"].as_array().unwrap() {
        let source = test["source"].as_str().unwrap();
        let case = cases
            .iter()
            .find(|case| source == format!("{}.fluxr", case.name))
            .unwrap();
        let row = Row {
            arguments: serde_json::from_value(
                test.get("arguments")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!([])),
            )
            .unwrap(),
            inputs: serde_json::from_value(
                test.get("inputs")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!([])),
            )
            .unwrap(),
        };
        let limits = test
            .get("limits")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .unwrap()
            .unwrap_or_default();
        let module = case.compile().unwrap();
        let actual =
            result(module.run_portable_with(row.invocation(limits), &mut Workspace::default()));
        if let Some(expected) = test["expect"].get("value") {
            let value = serde_json::from_value(expected.clone()).unwrap();
            assert_eq!(actual, Ok(value), "{}", test["name"]);
            // Hand-authored expectations must agree with the independent Rust too.
            assert_eq!(
                actual,
                (case.oracle)(&row),
                "Rust control: {}",
                test["name"]
            );
        } else {
            assert!(
                actual
                    .unwrap_err()
                    .contains(test["expect"]["error"].as_str().unwrap()),
                "{}",
                test["name"]
            );
        }
    }
}

#[test]
fn every_example_preserves_ordered_budget_failures_and_workspace_reuse() {
    let health = Module::compile_compact("v2 (pub run () i64 = 42)").unwrap();
    let health_call = health.prepare_call("run").unwrap();
    let health_row = Row::new(vec![]);
    for case in support::cases() {
        let module = case.compile().unwrap();
        let prepared = module.prepare_call("run").unwrap();
        let row = case
            .rows
            .iter()
            .find(|row| (case.oracle)(row).is_ok())
            .unwrap_or(&case.rows[0]);
        let mut portable = Workspace::default();
        let mut automatic = Workspace::default();
        let mut native = Workspace::default();
        let mut budgets = vec![];
        for instructions in [0, 1, 2, 3, 4, 7, 8, 15, 16, 31, 32, 63, 64, 127, 128] {
            budgets.push(Limits {
                instructions,
                ..Limits::default()
            });
        }
        for cells in [0, 1, 2, 3, 4, 7, 8, 15, 16, 31, 32, 63, 64] {
            budgets.push(Limits {
                cells,
                ..Limits::default()
            });
        }
        for frames in [0, 1, 2, 3, 4] {
            budgets.push(Limits {
                frames,
                ..Limits::default()
            });
        }
        for limits in budgets {
            let expected = result(module.run_portable_with(row.invocation(limits), &mut portable));
            assert_eq!(
                result(module.run_with(row.invocation(limits), &mut automatic)),
                expected,
                "{} {limits:?}",
                case.name
            );
            assert_eq!(
                result(prepared.run_with(&row.inputs, &row.arguments, limits, &mut native)),
                expected,
                "{} {limits:?}",
                case.name
            );
            let expected = (case.oracle)(row);
            assert_eq!(
                result(module.run_portable_with(row.invocation(Limits::default()), &mut portable)),
                expected
            );
            assert_eq!(
                result(module.run_with(row.invocation(Limits::default()), &mut automatic)),
                expected
            );
            assert_eq!(
                result(prepared.run_with(
                    &row.inputs,
                    &row.arguments,
                    Limits::default(),
                    &mut native
                )),
                expected
            );
            // Always demonstrate successful reuse, including examples whose
            // intended behavior is an unconditional uninitialized-read trap.
            assert_eq!(
                result(
                    health
                        .run_portable_with(health_row.invocation(Limits::default()), &mut portable)
                ),
                Ok(Value::I64(42))
            );
            assert_eq!(
                result(health.run_with(health_row.invocation(Limits::default()), &mut automatic)),
                Ok(Value::I64(42))
            );
            assert_eq!(
                result(health_call.run_with(&[], &[], Limits::default(), &mut native)),
                Ok(Value::I64(42))
            );
        }
    }
}

#[test]
fn malformed_host_values_fail_without_poisoning_reused_workspaces() {
    let health = Module::compile_compact("v2 (pub run () i64 = 42)").unwrap();
    let health_call = health.prepare_call("run").unwrap();
    let health_row = Row::new(vec![]);
    for case in support::cases() {
        let module = case.compile().unwrap();
        let prepared = module.prepare_call("run").unwrap();
        let good = case
            .rows
            .iter()
            .find(|row| (case.oracle)(row).is_ok())
            .unwrap_or(&case.rows[0]);
        let mut malformed = good.clone();
        if let Some(first) = malformed.inputs.first_mut() {
            *first = Value::Unit;
        } else if let Some(first) = malformed.arguments.first_mut() {
            *first = Value::Unit;
        } else {
            malformed.arguments.push(Value::Bool(false));
        }
        let mut portable = Workspace::default();
        let mut automatic = Workspace::default();
        let mut native = Workspace::default();
        let expected = result(
            module.run_portable_with(malformed.invocation(Limits::default()), &mut portable),
        );
        assert!(
            expected.is_err(),
            "{} accepted a malformed boundary",
            case.name
        );
        assert_eq!(
            result(module.run_with(malformed.invocation(Limits::default()), &mut automatic)),
            expected
        );
        assert_eq!(
            result(prepared.run_with(
                &malformed.inputs,
                &malformed.arguments,
                Limits::default(),
                &mut native
            )),
            expected
        );
        let expected = (case.oracle)(good);
        assert_eq!(
            result(module.run_portable_with(good.invocation(Limits::default()), &mut portable)),
            expected
        );
        assert_eq!(
            result(module.run_with(good.invocation(Limits::default()), &mut automatic)),
            expected
        );
        assert_eq!(
            result(prepared.run_with(
                &good.inputs,
                &good.arguments,
                Limits::default(),
                &mut native
            )),
            expected
        );
        assert_eq!(
            result(
                health.run_portable_with(health_row.invocation(Limits::default()), &mut portable)
            ),
            Ok(Value::I64(42))
        );
        assert_eq!(
            result(health.run_with(health_row.invocation(Limits::default()), &mut automatic)),
            Ok(Value::I64(42))
        );
        assert_eq!(
            result(health_call.run_with(&[], &[], Limits::default(), &mut native)),
            Ok(Value::I64(42))
        );
    }
}
