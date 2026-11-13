use fluxr::structured::{Invocation, Limits, Module, Source, Stmt, Value};

fn project(compact: &str) -> (Module, Module) {
    let original = Module::compile_compact(compact).unwrap();
    let agent = original.agent_text().unwrap();
    assert!(!agent.contains(['(', ')', '[', ']', '{', '}', ':', '.', ',', ';']));
    assert_eq!(
        Source::parse_agent(&agent).unwrap(),
        Source::parse_compact(compact).unwrap()
    );
    let rebuilt = Module::compile_agent(&agent).unwrap();
    assert_eq!(rebuilt.encode(), original.encode());
    assert_eq!(rebuilt.agent_text().unwrap(), agent);
    assert_eq!(
        Module::decode(&rebuilt.encode())
            .unwrap()
            .agent_text()
            .unwrap(),
        agent
    );
    (original, rebuilt)
}

#[test]
fn agent_projection_covers_existing_structured_corpus() {
    for (compact, agent) in [
        (
            include_str!("../examples/fixtures/modern_leaf.flux"),
            include_str!("../examples/fixtures/agent_scalar.fa1"),
        ),
        (
            include_str!("../examples/fixtures/native_projection.fluxr"),
            include_str!("../examples/fixtures/agent_projection.fa1"),
        ),
    ] {
        assert_eq!(
            Module::compile_compact(compact).unwrap().encode(),
            Module::compile_agent(agent).unwrap().encode()
        );
    }
    for source in [
        include_str!("../examples/fixtures/modern_leaf.flux"),
        include_str!("../examples/fixtures/modern_counter.flux"),
        include_str!("../examples/fixtures/native_projection.fluxr"),
        include_str!("../examples/fixtures/control_flow.fluxr"),
        include_str!("../examples/fixtures/loop_exits.fluxr"),
        include_str!("../examples/fixtures/list_boundary.fluxr"),
        include_str!("../examples/fixtures/variant_payload.fluxr"),
        include_str!("../examples/fixtures/method_call_order.fluxr"),
        concat!(
            "v2\n",
            include_str!("../examples/fixtures/method_edit_identity.fluxr")
        ),
        include_str!("../examples/fixtures/recursive_method_safety.fluxr"),
        include_str!("../examples/fixtures/nested_loop_calls.fluxr"),
        include_str!("../examples/fixtures/shared_host_admission.fluxr"),
        include_str!("../examples/fixtures/binary32_boundary.fluxr"),
    ] {
        project(source);
    }
}

#[test]
fn agent_tiny_source_is_delimiter_free_and_preserves_all_budget_outcomes() {
    let (old, new) = project("v2 (pub calc (x:i64 y:i64) i64 = (/ (+ x 1) y))");
    assert_eq!(
        new.agent_text().unwrap(),
        "fa1 v2\npub calc 2 x int y int int ret div add x 1 y"
    );
    for (x, y) in [(3, 2), (i64::MAX, 0), (1, 0), (i64::MIN, -1)] {
        for instructions in 0..20 {
            for cells in 0..8 {
                for frames in 0..3 {
                    let run = |module: &Module| {
                        format!(
                            "{:?}",
                            module.run(Invocation {
                                export: "calc",
                                inputs: &[],
                                arguments: &[Value::I64(x), Value::I64(y)],
                                limits: Limits {
                                    instructions,
                                    cells,
                                    frames
                                },
                            })
                        )
                    };
                    assert_eq!(
                        run(&old),
                        run(&new),
                        "x={x} y={y} fuel={instructions} cells={cells} frames={frames}"
                    );
                }
            }
        }
    }
}

#[test]
fn agent_prefix_grammar_covers_every_expression_and_statement_form() {
    let pairs = [
        ("unit", "unit"),
        ("true", "true"),
        ("false", "false"),
        ("-9223372036854775808", "-9223372036854775808"),
        ("18446744073709551615u", "18446744073709551615u"),
        ("0x7fc00001f", "0x7fc00001f"),
        ("(local true)", "local true"),
        ("(input value)", "input value"),
        ("(new Point 1 2)", "new Point 2 1 2"),
        ("(field r value)", "field r value"),
        ("(tag Result.none)", "tag Result none"),
        ("(tag Result.some 3)", "wrap Result some 3"),
        ("(is r some)", "is r some"),
        ("(payload r some)", "payload r some"),
        ("(list i64 1 2)", "list int 2 1 2"),
        ("(empty x)", "empty x"),
        ("(last x)", "last x"),
        ("(at x 1u)", "at x 1u"),
        ("(minidx x)", "minidx x"),
        ("(call f 1 2)", "call f 2 1 2"),
        ("(m r f 1 2)", "m r f 2 1 2"),
    ];
    for (compact, agent) in pairs {
        assert_eq!(
            Stmt::parse_compact_body(&format!("(ret {compact})")).unwrap(),
            Stmt::parse_agent_body(&format!("fa1 body 1 ret {agent}")).unwrap(),
            "{agent}"
        );
    }
    for (compact, agent) in [
        ("+", "add"),
        ("-", "sub"),
        ("*", "mul"),
        ("/", "div"),
        ("%", "rem"),
        ("==", "eq"),
        ("<", "lt"),
        ("<=", "le"),
        ("&&", "and"),
        ("||", "or"),
    ] {
        assert_eq!(
            Stmt::parse_compact_body(&format!("(ret ({compact} x y))")).unwrap(),
            Stmt::parse_agent_body(&format!("fa1 body 1 ret {agent} x y")).unwrap()
        );
    }
    for (compact, agent) in [
        ("!", "not"),
        ("abs", "abs"),
        ("tof", "tof"),
        ("i64tof", "i64tof"),
        ("ftoi64", "ftoi64"),
        ("sqrt", "sqrt"),
    ] {
        assert_eq!(
            Stmt::parse_compact_body(&format!("(ret ({compact} x))")).unwrap(),
            Stmt::parse_agent_body(&format!("fa1 body 1 ret {agent} x")).unwrap()
        );
    }
    assert_eq!(Stmt::parse_compact_body("(= x 1)(= r.value 2)(push xs x)(pop xs)(if true ((break)) ((continue)))(while false ((ret unit)))").unwrap(),
               Stmt::parse_agent_body("fa1 body 6 set x 1 put r value 2 push xs x pop xs if true 1 break 1 continue while false 1 ret unit").unwrap());
}

#[test]
fn agent_reserved_names_are_escaped_and_float_bits_are_exact() {
    let mut text = String::from("v2 (rec bool x:i64) (enum unit none some:[@bool])");
    for name in [
        "unit", "true", "false", "local", "add", "sub", "mul", "not", "input", "new", "field",
        "tag", "wrap", "is", "payload", "list", "empty", "last", "at", "minidx", "call", "m",
    ] {
        text.push_str(&format!(
            " (pub {name} ({name}:i64) i64 () (ret (local {name})))"
        ));
    }
    let (_, m) = project(&text);
    for name in ["add", "list", "m", "true"] {
        assert_eq!(
            m.run(Invocation {
                export: name,
                inputs: &[],
                arguments: &[Value::I64(73)],
                limits: Limits::default()
            })
            .unwrap(),
            Value::I64(73)
        );
    }
    for bits in [
        0,
        0x8000_0000,
        0x7f80_0000,
        0xff80_0000,
        0x7fc0_0001,
        0x7f80_0001,
        u32::MAX,
    ] {
        let (_, m) = project(&format!("v2 (pub f () f32 = 0x{bits:x}f)"));
        assert_eq!(
            m.run(Invocation {
                export: "f",
                inputs: &[],
                arguments: &[],
                limits: Limits::default()
            })
            .unwrap(),
            Value::F32(bits)
        );
    }
}

#[test]
fn agent_counts_names_depth_and_byte_limits_fail_closed() {
    for text in [
        "",
        "fa2 v2",
        "fa1 v3",
        "fa1 v2 pub f -1",
        "fa1 v2 pub f +0",
        "fa1 v2 pub f 00",
        "fa1 v2 pub f 99999999999999999999999999999999",
        "fa1 v2 pub f 257",
        "fa1 v2 pub f 0 int ret add 1",
        "fa1 v2 pub f 0 int ret 1 garbage",
        "fa1 v2 pub f 0 int ret (1)",
        "fa1 v2 pub café 0 int ret 1",
        "fa1 v2 pub f 0 int ret 9223372036854775808",
        "fa1 v2 pub f 0 uint ret 18446744073709551616u",
        "fa1 v2 pub f 0 float ret 0x100000000f",
        "fa1 v2 pub f 0 int do 0 32769",
        "fa1 v2 enum R 257",
        "fa1 v2 pub f 0 int ret list int 32769",
    ] {
        assert!(
            Source::parse_agent(text).is_err(),
            "accepted malformed {text}"
        );
    }
    for text in [
        "fa1 body 1 ret 1 extra",
        "fa1 body 2 ret 1",
        "fa1 body -1",
        "fa1 body 1 unknown",
    ] {
        assert!(Stmt::parse_agent_body(text).is_err());
    }
    assert!(Source::parse_agent(&format!("fa1 v2 pub {} 0 int ret 0", "x".repeat(65))).is_err());
    assert!(
        Source::parse_agent(&format!(
            "fa1 v2 pub f 0 bool ret {}true",
            "not ".repeat(100)
        ))
        .is_err()
    );
    assert!(
        Source::parse_agent(&format!(
            "fa1 v2 pub f 1 x {}int int ret 1",
            "list ".repeat(33)
        ))
        .is_err()
    );
    assert!(Source::parse_agent(&" ".repeat(256 * 1024 + 1)).is_err());
    assert!(Source::parse_agent(&format!("fa1 v2 {}", "pub f 0 int ret 0 ".repeat(513))).is_err());
    // A claimed large vector with missing elements refuses without reserving it.
    assert!(Source::parse_agent("fa1 v2 pub f 0 int ret call f 32768").is_err());
}

#[test]
fn agent_parser_does_not_admit_semantically_invalid_programs() {
    for text in [
        "fa1 v2",
        "fa1 v2 pub f 0 int ret true",
        "fa1 v2 pub f 0 int ret absent",
        "fa1 v2 pub f 0 int ret call missing 0",
        "fa1 v2 pub f 0 int do 0 1 break",
        "fa1 v2 pub f 0 int ret 1 pub f 0 int ret 2",
    ] {
        assert!(Module::compile_agent(text).is_err(), "{text}");
    }
    assert!(Module::compile_compact("fa1 v2 pub f 0 int ret 1").is_err());
    assert!(Module::compile_agent("v2 (pub f () i64 = 1)").is_err());
    let (old, new) = project("v1 (pub f () i64 () (ret 1))");
    assert_eq!(old.encode(), new.encode());
}

#[test]
fn agent_edit_bodies_preserve_collaboration_identity_and_conflict_checks() {
    use fluxr::collaboration::{Change, Edit, Mode, Snapshot, Target};
    use std::collections::BTreeMap;
    let base = Snapshot::new(
        Source::parse_compact("v2 (pub f () i64 = 1)").unwrap(),
        vec![Target {
            id: "stable_f".into(),
            function: "f".into(),
            receiver: None,
            mode: Mode::Exclusive,
        }],
    )
    .unwrap();
    let change = |body| Change {
        author: "agent".into(),
        requires: BTreeMap::new(),
        edits: vec![Edit::ReplaceBody {
            target: "stable_f".into(),
            expected: base.edit_revision("stable_f").unwrap(),
            body,
        }],
    };
    let fa = change(Stmt::parse_agent_body("fa1 body 1 ret add 4 2").unwrap());
    let old = change(Stmt::parse_compact_body("(ret (+ 4 2))").unwrap());
    let a = base.prepare(std::slice::from_ref(&fa)).unwrap();
    let b = base.prepare(&[old]).unwrap();
    assert_eq!(a.snapshot().id(), b.snapshot().id());
    assert_eq!(
        a.snapshot().module().encode(),
        b.snapshot().module().encode()
    );
    assert_eq!(a.accepted.len(), 1);
    let stale = a.snapshot().prepare(&[fa]).unwrap();
    assert!(stale.accepted.is_empty());
    assert_eq!(stale.rejected.len(), 1);
}

#[test]
fn agent_cli_projects_compact_and_fsm_with_bounded_refusals() {
    use std::{
        fs,
        process::Command,
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let temp = std::env::temp_dir().join(format!(
        "fluxr-fa1-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&temp).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(temp.clone());
    let module = Module::compile_compact("v2 (pub f () i64 = 7)").unwrap();
    fs::write(temp.join("in.flux"), "v2 (pub f () i64 = 7)").unwrap();
    fs::write(temp.join("in.fsm"), module.encode()).unwrap();
    let cli = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_fluxr-cli"))
            .current_dir(&temp)
            .args(args)
            .output()
            .unwrap()
    };
    for source in ["in.flux", "in.fsm"] {
        let projected = cli(&["project-agent", source, "out.fa1"]);
        assert!(projected.status.success(), "{projected:?}");
        let encoded = cli(&["encode-agent", "out.fa1", "out.fsm"]);
        assert!(encoded.status.success(), "{encoded:?}");
        assert_eq!(fs::read(temp.join("out.fsm")).unwrap(), module.encode());
    }
    fs::write(temp.join("invalid.fa1"), "fa1 v2 pub f 0 int ret true").unwrap();
    assert!(
        !cli(&["encode-agent", "invalid.fa1", "must-not-exist"])
            .status
            .success()
    );
    assert!(!temp.join("must-not-exist").exists());
    fs::File::create(temp.join("huge.fa1"))
        .unwrap()
        .set_len(4 * 1024 * 1024)
        .unwrap();
    let refused = cli(&["encode-agent", "huge.fa1", "must-not-exist"]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("program exceeds byte budget"));
    assert!(!temp.join("must-not-exist").exists());
}
