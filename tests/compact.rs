use fluxr::structured::{Invocation, Limits, Module, Stmt, Value};

#[test]
fn compact_roundtrips_and_runs_with_binary32_fields() {
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
    let module = Module::compile_compact(compact).unwrap();
    assert_eq!(
        Module::decode(&module.encode()).unwrap().encode(),
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
