use fluxr::structured::{ExecutionRoute, Invocation, Limits, Module, Value, Workspace};
use std::sync::Arc;

fn list(values: &[i64]) -> Value {
    Value::List(Arc::new(values.iter().copied().map(Value::I64).collect()))
}

fn check_routes(
    module: &Module,
    arguments: &[Value],
    instructions: u64,
    expected: Result<Value, &str>,
) {
    let limits = Limits {
        instructions,
        ..Limits::default()
    };
    let invoke = || Invocation {
        export: "run",
        inputs: &[],
        arguments,
        limits,
    };
    let expected = expected.map_err(str::to_owned);
    let mut workspace = Workspace::default();
    assert_eq!(
        module
            .run_portable_with(invoke(), &mut workspace)
            .map_err(|error| error.to_string()),
        expected
    );
    assert_eq!(
        module
            .run_with(invoke(), &mut workspace)
            .map_err(|error| error.to_string()),
        expected
    );
    let prepared = module.prepare_call("run").unwrap();
    let (actual, route) = prepared.run_with_route(&[], arguments, limits, &mut workspace);
    assert_eq!(actual.map_err(|error| error.to_string()), expected);
    assert_eq!(route, ExecutionRoute::Portable);
}

#[test]
fn list_local_reads_preserve_exact_load_operation_and_return_fuel_order() {
    const FUEL: &str = "module instruction budget exhausted";
    const UNINITIALIZED: &str = "read of uninitialized local";
    for (operation, result_type) in [("empty", "bool"), ("last", "i64")] {
        let missing = Module::compile_compact(&format!(
            "v2 (pub run () {result_type} (xs:[i64]) (ret ({operation} xs)))"
        ))
        .unwrap();
        check_routes(&missing, &[], 0, Err(FUEL));
        for fuel in 1..=4 {
            check_routes(&missing, &[], fuel, Err(UNINITIALIZED));
        }
        let module = Module::compile_compact(&format!(
            "v2 (pub run (xs:[i64]) {result_type} = ({operation} xs))"
        ))
        .unwrap();
        for values in [&[][..], &[3, 7][..]] {
            for fuel in 0..=4 {
                let expected = if fuel < 2 {
                    Err(FUEL)
                } else if operation == "last" && values.is_empty() {
                    Err("last from empty list")
                } else if fuel < 3 {
                    Err(FUEL)
                } else if operation == "empty" {
                    Ok(Value::Bool(values.is_empty()))
                } else {
                    Ok(Value::I64(7))
                };
                check_routes(&module, &[list(values)], fuel, expected);
            }
        }
    }
}

#[test]
fn last_retains_nested_value_identity_without_retaining_the_outer_list() {
    let fields: Arc<[Value]> = vec![Value::I64(7)].into();
    let inner = Arc::new(vec![Value::I64(11)]);
    let payload = Arc::new(Value::I64(13));
    let cases = [
        (
            "v2 (rec R x:i64) (pub run (xs:[R]) R = (last xs))",
            Value::Record(Arc::clone(&fields)),
        ),
        (
            "v2 (pub run (xs:[[i64]]) [i64] = (last xs))",
            Value::List(Arc::clone(&inner)),
        ),
        (
            "v2 (enum E None Some:i64) (pub run (xs:[#E]) #E = (last xs))",
            Value::Variant(1, Some(Arc::clone(&payload))),
        ),
    ];
    for (source, value) in cases {
        let module = Module::compile_compact(source).unwrap();
        let artifact = module.encode();
        let decoded = Module::decode(&artifact).unwrap();
        assert_eq!(decoded.encode(), artifact);
        let outer = Arc::new(vec![value.clone()]);
        let arguments = [Value::List(Arc::clone(&outer))];
        let prepared = decoded.prepare_call("run").unwrap();
        let mut workspace = Workspace::default();
        for _ in 0..4 {
            let result = prepared
                .run_with(&[], &arguments, Limits::default(), &mut workspace)
                .unwrap();
            assert_eq!(result, value);
            match (&result, &value) {
                (Value::Record(actual), Value::Record(expected)) => {
                    assert!(Arc::ptr_eq(actual, expected));
                }
                (Value::List(actual), Value::List(expected)) => {
                    assert!(Arc::ptr_eq(actual, expected));
                }
                (Value::Variant(_, Some(actual)), Value::Variant(_, Some(expected))) => {
                    assert!(Arc::ptr_eq(actual, expected));
                }
                _ => panic!("nested fixture changed"),
            }
            assert_eq!(Arc::strong_count(&outer), 2);
        }
    }
}

#[test]
fn borrowed_reads_preserve_copy_on_write_after_success_and_exhaustion() {
    let module = Module::compile_compact(
        "v2 (pub run (xs:[i64] x:i64) [i64] ()
            (if (! (empty xs)) ((pop xs)))
            (push xs x) (push xs (last xs)) (ret xs))",
    )
    .unwrap();
    let original = Arc::new(vec![Value::I64(3), Value::I64(7), Value::I64(11)]);
    let arguments = [Value::List(Arc::clone(&original)), Value::I64(-4)];
    let prepared = module.prepare_call("run").unwrap();
    let mut workspace = Workspace::default();
    for fuel in 0..=24 {
        let result = prepared.run_with(
            &[],
            &arguments,
            Limits {
                instructions: fuel,
                ..Limits::default()
            },
            &mut workspace,
        );
        match result {
            Ok(Value::List(changed)) => {
                assert!(!Arc::ptr_eq(&changed, &original));
                assert_eq!(
                    changed.as_ref(),
                    &[Value::I64(3), Value::I64(7), Value::I64(-4), Value::I64(-4)]
                );
            }
            Ok(other) => panic!("list result changed to {other:?}"),
            Err(error) => assert_eq!(error.to_string(), "module instruction budget exhausted"),
        }
        assert_eq!(
            original.as_ref(),
            &[Value::I64(3), Value::I64(7), Value::I64(11)]
        );
        assert_eq!(Arc::strong_count(&original), 2);
    }
    assert_eq!(
        prepared
            .run_with(&[], &arguments, Limits::default(), &mut workspace)
            .unwrap(),
        list(&[3, 7, -4, -4])
    );
}
