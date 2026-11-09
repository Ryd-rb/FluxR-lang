use fluxr::structured::{Binding, Invocation, Limits, Module, ModuleSlot, Type, Value};

const SOURCE: &str =
    "v2\n(inputs offset:i64)\n(pub calc (x:i64 y:i64) i64 () (ret (+ (% x y) $offset)))";

#[test]
fn scalar_tier_preserves_checked_math_host_validation_and_metering() {
    let module = Module::compile_compact(SOURCE).unwrap();
    let native = cfg!(all(
        feature = "native-jit",
        target_os = "windows",
        target_arch = "x86_64"
    ));
    assert_eq!(module.prepare_native("calc"), native);
    assert!(!module.prepare_native("missing"));
    let run = |offset: Value, x: Value, y: Value, limits| {
        module.run(Invocation {
            export: "calc",
            inputs: &[offset],
            arguments: &[x, y],
            limits,
        })
    };
    let limits = Limits {
        instructions: 6,
        cells: 7,
        frames: 1,
    };
    for x in [i64::MIN, i64::MIN + 1, -97, -1, 0, 1, 97, i64::MAX] {
        for y in [i64::MIN, -3, -1, 0, 1, 3, i64::MAX] {
            for offset in [i64::MIN, -7, 0, 7, i64::MAX] {
                let expected = x
                    .checked_rem(y)
                    .and_then(|v| v.checked_add(offset))
                    .map(Value::I64);
                let actual = run(Value::I64(offset), Value::I64(x), Value::I64(y), limits);
                match expected {
                    Some(value) => assert_eq!(actual.unwrap(), value),
                    None => assert_eq!(
                        actual.unwrap_err().to_string(),
                        "invalid or overflowing integer arithmetic"
                    ),
                }
            }
        }
    }
    for (limits, error) in [
        (
            Limits {
                instructions: 5,
                ..limits
            },
            "module instruction budget exhausted",
        ),
        (
            Limits { cells: 6, ..limits },
            "module cell budget exhausted",
        ),
        (
            Limits {
                frames: 0,
                ..limits
            },
            "module frame limit exceeded",
        ),
    ] {
        assert_eq!(
            run(Value::I64(7), Value::I64(9), Value::I64(2), limits)
                .unwrap_err()
                .to_string(),
            error
        );
    }
    assert_eq!(
        run(Value::Bool(false), Value::I64(9), Value::I64(2), limits)
            .unwrap_err()
            .to_string(),
        "host value differs from I64"
    );
    assert_eq!(
        run(
            Value::I64(7),
            Value::I64(9),
            Value::I64(0),
            Limits {
                instructions: 3,
                ..limits
            }
        )
        .unwrap_err()
        .to_string(),
        "invalid or overflowing integer arithmetic"
    );
}

#[test]
fn injection_does_not_reuse_the_previous_native_body() {
    let module = Module::compile_compact(SOURCE).unwrap();
    let mut slot = ModuleSlot::new(
        module,
        &[Binding {
            name: "offset".into(),
            ty: Type::I64,
        }],
    )
    .unwrap();
    let invoke = |y| Invocation {
        export: "calc",
        inputs: &[Value::I64(7)],
        arguments: y,
        limits: Limits::default(),
    };
    let good = [Value::I64(9), Value::I64(4)];
    let bad = [Value::I64(9), Value::I64(0)];
    assert_eq!(slot.run(invoke(&good)).unwrap(), Value::I64(8));
    let candidate = Module::compile_compact(&SOURCE.replace("(+ (%", "(- (%")).unwrap();
    assert!(
        slot.inject(0, candidate.clone(), Some(invoke(&bad)))
            .is_err()
    );
    assert_eq!(slot.revision(), 0);
    assert_eq!(slot.run(invoke(&good)).unwrap(), Value::I64(8));
    assert_eq!(slot.inject(0, candidate, Some(invoke(&good))).unwrap(), 1);
    assert_eq!(slot.run(invoke(&good)).unwrap(), Value::I64(-6));
}
