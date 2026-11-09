use fluxr::structured::{Invocation, Limits, Module, Stmt, Value};
use std::fmt::Write;

#[test]
fn compact_signed_float_conversions_and_sqrt_round_trip() {
    let module =
        Module::compile_compact(include_str!("../examples/fixtures/native_projection.fluxr"))
            .unwrap();
    let bytes = module.encode();
    let module = Module::decode(&bytes).unwrap();
    assert_eq!(
        Module::compile_compact(&module.compact_text().unwrap())
            .unwrap()
            .encode(),
        bytes
    );
    for (input, expected) in [(24, -4), (25, -5), (-1, 0), (i64::MIN, 0)] {
        assert_eq!(
            module
                .run(Invocation {
                    export: "project",
                    inputs: &[],
                    arguments: &[Value::I64(input)],
                    limits: Limits::default(),
                })
                .unwrap(),
            Value::I64(expected),
        );
    }
    let edges = Module::compile_compact(
        "v2 (pub root (x:f32) f32 = (sqrt x)) (pub int (x:f32) i64 = (ftoi64 x))",
    )
    .unwrap();
    let run = |export, bits| {
        edges
            .run(Invocation {
                export,
                inputs: &[],
                arguments: &[Value::F32(bits)],
                limits: Limits::default(),
            })
            .unwrap()
    };
    assert_eq!(run("root", (-0.0_f32).to_bits()), Value::F32(0x8000_0000));
    assert_eq!(run("root", (-1.0_f32).to_bits()), Value::F32(0x7fc0_0000));
    assert_eq!(run("int", f32::INFINITY.to_bits()), Value::I64(i64::MAX));
    assert_eq!(
        run("int", f32::NEG_INFINITY.to_bits()),
        Value::I64(i64::MIN)
    );
    assert_eq!(run("int", 0x7fc0_0000), Value::I64(0));
}

#[test]
fn compact_declaration_and_canonical_budgets() {
    let mut source = String::from("v2\n");
    for i in 0..512 {
        writeln!(source, "(pub f{i} () i64 = {i})").unwrap();
    }
    let module = Module::decode(&Module::compile_compact(&source).unwrap().encode()).unwrap();
    assert_eq!(
        module
            .run(Invocation {
                export: "f511",
                inputs: &[],
                arguments: &[],
                limits: Limits::default(),
            })
            .unwrap(),
        Value::I64(511)
    );
    source.push_str("(pub extra () i64 = 0)");
    assert!(
        Module::compile_compact(&source)
            .unwrap_err()
            .to_string()
            .contains("declaration count")
    );

    let body = "(= x (+ x 1)) ".repeat(48);
    let mut source = String::from("v2\n");
    for i in 0..64 {
        writeln!(source, "(pub f{i} (x:i64) i64 () {body}(ret x))").unwrap();
    }
    let bytes = Module::compile_compact(&source).unwrap().encode();
    assert!(source.len() < 256 * 1024 && bytes.len() > 256 * 1024);
    let module = Module::decode(&bytes).unwrap();
    assert_eq!(module.encode(), bytes);
    assert_eq!(
        module
            .run(Invocation {
                export: "f31",
                inputs: &[],
                arguments: &[Value::I64(2)],
                limits: Limits::default(),
            })
            .unwrap(),
        Value::I64(50)
    );

    let mut oversized = b"FSM\0\x02\0".to_vec();
    oversized.resize(6 + 1024 * 1024 + 1, b' ');
    assert!(
        Module::decode(&oversized)
            .unwrap_err()
            .to_string()
            .contains("canonical byte budget")
    );
}

#[test]
fn compact_and_released_json_encode_and_run_identically() {
    let compact = r#"v2
(rec Point x:f32 z:f32)
(enum Result none found:Point)
(fn Point.shift (self:Point amount:f32) Point ()
  (ret (new Point (+ self.x amount) self.z)))
(pub pick (values:[f32]) #Result (index:u64)
  (if (empty values) ((ret (tag Result.none))))
  (= index (minidx values))
  (ret (tag Result.found (m (new Point 0f -0f) shift (at values index)))))
"#;
    let json = r#"{"version":2,"records":[{"name":"Point","fields":[{"name":"x","ty":"f32"},{"name":"z","ty":"f32"}]}],"enums":[{"name":"Result","variants":[{"name":"none","payload":null},{"name":"found","payload":{"record":"Point"}}]}],"functions":[{"name":"shift","receiver":"Point","params":[{"name":"self","ty":{"record":"Point"}},{"name":"amount","ty":"f32"}],"result":{"record":"Point"},"body":[{"op":"return","value":{"op":"record","name":"Point","fields":[{"op":"binary","kind":"add","left":{"op":"field","record":{"op":"local","name":"self"},"name":"x"},"right":{"op":"local","name":"amount"}},{"op":"field","record":{"op":"local","name":"self"},"name":"z"}]}}]},{"name":"pick","export":true,"params":[{"name":"values","ty":{"list":"f32"}}],"locals":[{"name":"index","ty":"u64"}],"result":{"enum":"Result"},"body":[{"op":"if","test":{"op":"empty","list":{"op":"local","name":"values"}},"then":[{"op":"return","value":{"op":"variant","name":"Result","variant":"none","payload":null}}]},{"op":"set","local":"index","value":{"op":"arg_min_f32","list":{"op":"local","name":"values"}}},{"op":"return","value":{"op":"variant","name":"Result","variant":"found","payload":{"op":"method","receiver":{"op":"record","name":"Point","fields":[{"op":"f32","bits":0},{"op":"f32","bits":2147483648}]},"name":"shift","args":[{"op":"index","list":{"op":"local","name":"values"},"index":{"op":"local","name":"index"}}]}}}]}]}"#;
    let module = Module::compile_compact(compact).unwrap();
    assert_eq!(
        module.encode(),
        Module::compile_legacy_json(json).unwrap().encode()
    );
    assert_eq!(
        Module::decode(&module.encode()).unwrap().encode(),
        module.encode()
    );
    let migrated = Module::compile_legacy_json(json)
        .unwrap()
        .compact_text()
        .unwrap();
    assert_eq!(
        Module::compile_compact(&migrated).unwrap().encode(),
        module.encode()
    );
    let input = [Value::List(
        vec![Value::F32(2_f32.to_bits()), Value::F32(1_f32.to_bits())].into(),
    )];
    let actual = module
        .run(Invocation {
            export: "pick",
            inputs: &[],
            arguments: &input,
            limits: Limits::default(),
        })
        .unwrap();
    assert_eq!(
        actual,
        Value::Variant(
            1,
            Some(
                Value::Record([Value::F32(1_f32.to_bits()), Value::F32((-0_f32).to_bits())].into())
                    .into()
            )
        )
    );
}

#[test]
fn compact_source_rejects_malformed_or_excessive_inputs() {
    for text in ["(ret)", "(ret 1u))", "(ret 0x100000000f)", "(ret 1u"] {
        assert!(Stmt::parse_compact_body(text).is_err(), "{text}");
    }
    assert!(Stmt::parse_compact_body(&format!("(ret x{})", ".field".repeat(80))).is_err());
    assert!(
        Stmt::parse_compact_body(&format!("(ret {}0{})", "(! ".repeat(120), ")".repeat(120)))
            .is_err()
    );
    assert!(Module::compile_compact("v3 (pub f () u64 () (ret 0u))").is_err());
}

#[test]
fn compact_escaped_record_name_can_match_a_primitive() {
    let module =
        Module::compile_compact("v2 (rec bool x:u64) (pub f () @bool () (ret (new bool 1u)))")
            .unwrap();
    let output = module
        .run(Invocation {
            export: "f",
            inputs: &[],
            arguments: &[],
            limits: Limits::default(),
        })
        .unwrap();
    assert_eq!(output, Value::Record([Value::U64(1)].into()));
    assert_eq!(
        Module::compile_compact(&module.compact_text().unwrap())
            .unwrap()
            .encode(),
        module.encode()
    );
}

#[test]
fn migration_preserves_released_counter_and_reserved_identifiers() {
    let counter =
        Module::compile_legacy_json(include_str!("fixtures/structured_counter.fluxm")).unwrap();
    assert_eq!(
        counter.compact_text().unwrap(),
        include_str!("../examples/fixtures/modern_counter.flux")
    );
    let reserved = Module::compile_compact(
        "v1 (fn last (true:i64) i64 () (ret (local true))) (pub f () i64 () (ret (call last 7)))",
    )
    .unwrap();
    for module in [counter, reserved] {
        let bytes = module.encode();
        let migrated = Module::decode(&bytes).unwrap().compact_text().unwrap();
        let restored = Module::compile_compact(&migrated).unwrap();
        assert_eq!(restored.encode(), bytes);
        assert_eq!(restored.compact_text().unwrap(), migrated);
    }
}

#[test]
fn collection_forms_and_boolean_chains_preserve_portable_lowering() {
    let shared = "v2 (rec Row a:bool b:bool c:bool)";
    let compact = "(pub pick (rows:[Row] n:u64) u64 (i:u64 r:Row)
        (find i n r rows (& r.a (! r.b) r.c)))
        (pub collect (rows:[Row] n:u64) [u64] (i:u64 result:[u64])
        (indices result i n (field (at rows i) a)))";
    let expanded = "(pub pick (rows:[Row] n:u64) u64 (i:u64 r:Row)
        (= i 0u)
        (while (< i n) (
            (= r (at rows i))
            (if (&& (&& r.a (! r.b)) r.c) ((ret i)))
            (= i (+ i 1u))))
        (ret n))
        (pub collect (rows:[Row] n:u64) [u64] (i:u64 result:[u64])
        (= result (list u64))
        (= i 0u)
        (while (< i n) (
            (if (field (at rows i) a) ((push result i)))
            (= i (+ i 1u))))
        (ret result))";
    let module = Module::compile_compact(&format!("{shared} {compact}")).unwrap();
    assert_eq!(
        module.encode(),
        Module::compile_compact(&format!("{shared} {expanded}"))
            .unwrap()
            .encode()
    );
    assert!(Stmt::parse_compact_body("(ret (& true))").is_err());
    assert!(Stmt::parse_compact_body("(find i 1u i rows true)").is_err());
}

#[test]
fn bounded_scan_preserves_edit_lowering_and_loop_effects() {
    let compact = "(scan i n (
        (= n (- n 1u))
        (when (< (at xs i) 0) (continue)))) (ret i)";
    let expanded = "(= i 0u) (while (< i n) (
        (= n (- n 1u))
        (when (< (at xs i) 0) (continue))
        (= i (+ i 1u)))) (ret i)";
    assert_eq!(
        Stmt::parse_compact_body(compact).unwrap(),
        Stmt::parse_compact_body(expanded).unwrap(),
    );
    assert!(Stmt::parse_compact_body("(scan i n)").is_err());
    assert!(Stmt::parse_compact_body("(scan i n () extra)").is_err());

    let module = |body| {
        Module::compile_compact(&format!("v2 (pub f (xs:[i64] n:u64) u64 (i:u64) {body})")).unwrap()
    };
    let compact = module(compact);
    let expanded = module(expanded);
    assert_eq!(compact.encode(), expanded.encode());
    assert_eq!(
        compact.compact_text().unwrap(),
        expanded.compact_text().unwrap()
    );

    for (values, bound, expected) in [
        (vec![], 0, Some(0)),
        (vec![-1], 4, Some(0)),
        (vec![1, 2], 3, Some(2)),
        (vec![1], 3, None),
    ] {
        let arguments = [
            Value::List(std::sync::Arc::new(
                values.into_iter().map(Value::I64).collect(),
            )),
            Value::U64(bound),
        ];
        let invoke = |module: &Module, instructions| {
            module.run(Invocation {
                export: "f",
                inputs: &[],
                arguments: &arguments,
                limits: Limits {
                    instructions,
                    ..Limits::default()
                },
            })
        };
        for fuel in 0..96 {
            assert_eq!(invoke(&compact, fuel), invoke(&expanded, fuel));
        }
        match expected {
            Some(index) => assert_eq!(invoke(&compact, 96).unwrap(), Value::U64(index)),
            None => assert!(
                invoke(&compact, 96)
                    .unwrap_err()
                    .to_string()
                    .contains("index")
            ),
        }
    }
}

#[test]
fn gates_preserve_exact_lazy_fact_order() {
    use fluxr::Gate;
    let gate = Gate::compile_text("(& a (| b c) (! d))").unwrap();
    for mask in 0..16 {
        let value = |i: usize| mask & (1 << i) != 0;
        let mut expected_calls = Vec::new();
        let mut fact = |i| {
            expected_calls.push(i);
            value(i)
        };
        let expected = fact(0) && (fact(1) || fact(2)) && !fact(3);
        let mut actual_calls = Vec::new();
        assert_eq!(
            gate.evaluate(|i| {
                actual_calls.push(i);
                value(i)
            }),
            expected
        );
        assert_eq!(actual_calls, expected_calls);
    }
    assert!(
        gate.rust_function("sample")
            .unwrap()
            .contains("p0() && (p1() || p2()) && !p3()")
    );
    for invalid in ["&aac", "&ac", "(& a 4u)", "(& a b) 0u", "&abcq"] {
        assert!(Gate::compile_text(invalid).is_err());
    }
    for mask in 0..32 {
        let value = |i: usize| mask & (1 << i) != 0;
        assert_eq!(
            Gate::compile_text("&abcde").unwrap().evaluate(value),
            mask == 31
        );
    }
}
