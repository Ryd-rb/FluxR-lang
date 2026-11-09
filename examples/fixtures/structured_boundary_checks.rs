use fluxr::structured::{Binding, Invocation, Limits, Module, ModuleSlot, Type, Value};

const SOURCE: &str = "v2 (rec R x:i64 y:i64) (inputs delta:i64 bias:i64)
    (pub apply (s:R) R () (= s.x (+ s.x (input delta))) (= s.y (- s.y (input bias))) (ret s))";
const PREPARED: bool = cfg!(all(
    feature = "typed-jit",
    target_os = "windows",
    target_arch = "x86_64"
));

pub(super) fn replace_fsm_body(original: &[u8], body: &[u8]) -> Vec<u8> {
    let mut bytes = original[..6].to_vec();
    bytes.extend_from_slice(body);
    bytes
}

fn check_type_depth() {
    let source = |depth| {
        let open = "[".repeat(depth);
        let close = "]".repeat(depth);
        format!("v2 (pub accept (x:{open}f32{close}) u64 () (ret 1u))")
    };
    assert_eq!(
        Module::compile_compact(&source(32))
            .err()
            .unwrap()
            .to_string(),
        "type nesting limit exceeded"
    );
    let module = Module::compile_compact(&source(31)).unwrap();
    let wrap = |leaf| (0..31).fold(leaf, |value, _| Value::List(vec![value].into()));
    let run = |leaf, cells| {
        module.run(Invocation {
            export: "accept",
            inputs: &[],
            arguments: &[wrap(leaf)],
            limits: Limits {
                instructions: 2,
                cells,
                frames: 1,
            },
        })
    };
    assert_eq!(run(Value::F32(0), 35).unwrap(), Value::U64(1));
    assert_eq!(run(Value::Unit, 31).unwrap_err().to_string(), super::CELLS);
    assert_eq!(
        run(Value::Unit, 32).unwrap_err().to_string(),
        "host value differs from F32"
    );
}

pub(crate) fn check_structured_boundary() -> usize {
    check_type_depth();
    let module = Module::compile_compact(SOURCE).unwrap();
    assert_eq!(module.prepare_native("apply"), PREPARED);
    let original = module.encode();
    assert_eq!(Module::decode(&original).unwrap().encode(), original);
    let grants = ["delta", "bias"].map(|name| Binding {
        name: name.into(),
        ty: Type::I64,
    });
    let wrong = Binding {
        name: "delta".into(),
        ty: Type::U64,
    };
    for (provided, denied) in [
        (&[][..], "delta"),
        (&grants[..1], "bias"),
        (&[wrong, grants[1].clone()], "delta"),
    ] {
        assert_eq!(
            ModuleSlot::new(module.clone(), provided)
                .err()
                .unwrap()
                .to_string(),
            format!("host input {denied} is not granted")
        );
    }
    // Grant order is irrelevant. The module declaration controls positional ABI.
    let mut slot =
        ModuleSlot::new(module.clone(), &[grants[1].clone(), grants[0].clone()]).unwrap();
    let snapshot = [Value::I64(2), Value::I64(3)];
    let pair = |x, y| Value::Record([Value::I64(x), Value::I64(y)].into());
    let args = [pair(7, 4)];
    let call = |inputs| Invocation {
        export: "apply",
        inputs,
        arguments: &args,
        limits: Limits::default(),
    };
    assert_eq!(slot.run(call(&snapshot)).unwrap(), pair(9, 1));
    let edit = SOURCE.replace("(- s.y", "(+ s.y");
    let candidate = Module::compile_compact(&edit).unwrap();
    assert_eq!(candidate.prepare_native("apply"), PREPARED);
    let bad_snapshot = [Value::Bool(false), snapshot[1].clone()];
    let probe = || Some(call(&bad_snapshot));

    let incompatible = [
        SOURCE.replace("delta:i64 bias:i64", "bias:i64 delta:i64"),
        SOURCE.replace("x:i64 y:i64", "y:i64 x:i64"),
        SOURCE.replace("(pub apply", "(pub renamed"),
        SOURCE.replace("bias", "offset"),
    ];
    for text in &incompatible {
        let invalid = Module::compile_compact(text).unwrap();
        assert_eq!(
            slot.inject(0, invalid.clone(), probe())
                .unwrap_err()
                .to_string(),
            "replacement module has a different host interface"
        );
        assert_eq!(
            slot.inject(1, invalid, probe()).unwrap_err().to_string(),
            "module revision changed"
        );
        assert_eq!(slot.revision(), 0);
        assert_eq!(slot.module().encode(), original);
    }
    assert_eq!(
        slot.inject(0, candidate.clone(), probe())
            .unwrap_err()
            .to_string(),
        "host value differs from I64"
    );
    assert_eq!(slot.revision(), 0);
    assert_eq!(slot.run(call(&snapshot)).unwrap(), pair(9, 1));
    assert_eq!(slot.inject(0, candidate, Some(call(&snapshot))).unwrap(), 1);
    assert_eq!(slot.run(call(&snapshot)).unwrap(), pair(9, 7));
    assert_eq!(module.run(call(&snapshot)).unwrap(), pair(9, 1));

    // The portable decoder treats the envelope as untrusted source.
    for end in 0..original.len() {
        assert!(Module::decode(&original[..end]).is_err(), "prefix {end}");
    }
    let mut changed = original.clone();
    for index in 0..6 {
        changed[index] ^= 1;
        assert!(Module::decode(&changed).is_err(), "header byte {index}");
        changed[index] = original[index];
    }
    let body = std::str::from_utf8(&original[6..]).unwrap();
    for (before, after) in [
        ("\"op\":\"input\"", "\"op\":\"missing\""),
        ("\"name\":\"delta\"", "\"name\":\"bias\""),
        ("\"result\":{\"record\":\"R\"}", "\"result\":\"u64\""),
    ] {
        let rejected = body.replace(before, after);
        assert_ne!(body, rejected);
        let bytes = replace_fsm_body(&original, rejected.as_bytes());
        assert!(Module::decode(&bytes).is_err(), "mutated source {before}");
    }
    changed[6] = 0xff;
    assert!(Module::decode(&changed).is_err());
    original.len() + 6 + 3 + 1 // Rejected artifacts.
}
