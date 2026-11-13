//! Independent adversarial review of the optional FA1 projection.
use fluxr::structured::{
    Binding, Expr, Function, Invocation, Limits, Module, Source, Stmt, Type, Value,
};
use std::{fs, path::Path, sync::Arc};

fn function(name: &str, params: Vec<Binding>, result: Type, value: Expr) -> Function {
    Function {
        name: name.into(),
        receiver: None,
        export: true,
        params,
        locals: vec![],
        result,
        body: vec![Stmt::Return { value }],
    }
}

fn project_ast(source: Source) -> Module {
    let module = Module::compile(source.clone()).unwrap();
    let text = module.agent_text().unwrap();
    assert_eq!(Source::parse_agent(&text).unwrap(), source);
    let projected = Module::compile_agent(&text).unwrap();
    assert_eq!(projected.encode(), module.encode());
    assert_eq!(projected.agent_text().unwrap(), text);
    projected
}

#[test]
fn independently_constructed_keyword_locals_roundtrip_without_lexical_rebinding() {
    let words = "fa1 v1 v2 pub fn method inputs rec enum unit bool int uint float record list set put push pop if while break continue ret do body none add sub mul div rem eq lt le and or not abs tof i64tof ftoi64 sqrt local input new field tag wrap is payload empty last at minidx call m true false";
    let mut source = Source {
        version: 2,
        inputs: vec![],
        records: vec![],
        enums: vec![],
        functions: vec![],
    };
    for word in words.split_whitespace() {
        source.functions.push(function(
            word,
            vec![Binding {
                name: word.into(),
                ty: Type::I64,
            }],
            Type::I64,
            Expr::Local { name: word.into() },
        ));
    }
    let module = project_ast(source);
    for word in words.split_whitespace() {
        assert_eq!(
            module
                .run(Invocation {
                    export: word,
                    inputs: &[],
                    arguments: &[Value::I64(-31)],
                    limits: Limits::default()
                })
                .unwrap(),
            Value::I64(-31)
        );
    }
}

#[test]
fn independently_constructed_raw_binary32_payloads_are_not_normalized() {
    let mut source = Source {
        version: 2,
        inputs: vec![],
        records: vec![],
        enums: vec![],
        functions: vec![],
    };
    let mut state: u32 = 0x5eed_1234;
    let mut words = vec![
        0,
        1,
        0x007f_ffff,
        0x0080_0000,
        0x8000_0000,
        0x8000_0001,
        0x7f80_0000,
        0xff80_0000,
        0x7f80_0001,
        0xff80_0001,
        0xffff_ffff,
    ];
    for _ in 0..245 {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        words.push(state);
    }
    for (index, &bits) in words.iter().enumerate() {
        source.functions.push(function(
            &format!("f{index}"),
            vec![],
            Type::F32,
            Expr::F32 { bits },
        ));
    }
    let module = project_ast(source);
    for (index, &bits) in words.iter().enumerate() {
        assert_eq!(
            module
                .run(Invocation {
                    export: &format!("f{index}"),
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
fn zero_counts_keep_empty_record_list_call_and_absent_payload_distinct() {
    let text = "fa1 v2 rec Empty 0 enum Choice 2 none none with_unit unit
        fn nop 0 unit ret unit
        pub record_zero 0 record Empty ret new Empty 0
        pub list_zero 0 list int ret list int 0
        pub no_payload 0 enum Choice ret tag Choice none
        pub unit_payload 0 enum Choice ret wrap Choice with_unit unit
        pub call_zero 0 unit ret call nop 0
        pub empty_blocks 0 unit do 0 3 if false 0 0 while false 0 ret unit";
    let module = project_ast(Source::parse_agent(text).unwrap());
    for (export, expected) in [
        ("record_zero", Value::Record(Arc::from([]))),
        ("list_zero", Value::List(Arc::new(vec![]))),
        ("no_payload", Value::Variant(0, None)),
        (
            "unit_payload",
            Value::Variant(1, Some(Arc::new(Value::Unit))),
        ),
        ("call_zero", Value::Unit),
        ("empty_blocks", Value::Unit),
    ] {
        assert_eq!(
            module
                .run(Invocation {
                    export,
                    inputs: &[],
                    arguments: &[],
                    limits: Limits::default()
                })
                .unwrap(),
            expected
        );
    }
    assert_eq!(Stmt::parse_agent_body("fa1 body 0").unwrap(), vec![]);
    for invalid in [
        "fa1 v2 enum Empty 0 pub f 0 unit ret unit",
        "fa1 v2 pub f 0 unit do 0 0",
    ] {
        assert!(Source::parse_agent(invalid).is_ok());
        assert!(Module::compile_agent(invalid).is_err());
    }
}

#[test]
fn receiver_then_arguments_keep_first_trap_and_resource_boundaries() {
    let old = Module::compile_compact(include_str!("../examples/fixtures/method_call_order.fluxr"))
        .unwrap();
    let new = Module::compile_agent(&old.agent_text().unwrap()).unwrap();
    let point = Value::Record([Value::I64(4), Value::U64(10)].into());
    let inputs = [Value::List(Arc::new(vec![point.clone()])), point];
    for (index, divisor, scale, expected) in [
        (1, 0, Value::Variant(0, None), "list index out of bounds"),
        (
            0,
            0,
            Value::Variant(0, None),
            "invalid or overflowing integer arithmetic",
        ),
        (
            0,
            1,
            Value::Variant(0, None),
            "enum payload does not match variant",
        ),
    ] {
        let arguments = [Value::U64(index), Value::I64(divisor), scale];
        for module in [&old, &new] {
            assert_eq!(
                module
                    .run(Invocation {
                        export: "indexed",
                        inputs: &inputs,
                        arguments: &arguments,
                        limits: Limits::default()
                    })
                    .unwrap_err()
                    .to_string(),
                expected
            );
        }
        for instructions in 0..40 {
            for cells in 0..20 {
                for frames in 0..3 {
                    let run = |module: &Module| {
                        module
                            .run(Invocation {
                                export: "indexed",
                                inputs: &inputs,
                                arguments: &arguments,
                                limits: Limits {
                                    instructions,
                                    cells,
                                    frames,
                                },
                            })
                            .map_err(|error| error.to_string())
                    };
                    assert_eq!(run(&old), run(&new));
                }
            }
        }
    }
}

#[test]
fn bounded_token_corruption_never_panics_or_breaks_successful_projection_identity() {
    let original =
        Module::compile_compact(include_str!("../examples/fixtures/variant_payload.fluxr"))
            .unwrap();
    let text = original.agent_text().unwrap();
    let words: Vec<&str> = text.split_ascii_whitespace().collect();
    let corrupt = [
        "",
        "(",
        ")",
        "{",
        ";",
        "-1",
        "00",
        "32769",
        "999999999999999999999999999999",
        "local",
        "unit",
    ];
    for index in 0..words.len() {
        for replacement in corrupt {
            let mut changed = words.clone();
            changed[index] = replacement;
            let mutated = changed.join(" ");
            let parsed = std::panic::catch_unwind(|| Source::parse_agent(&mutated));
            assert!(parsed.is_ok(), "parser panicked at token {index}");
            if let Ok(source) = parsed.unwrap()
                && let Ok(module) = Module::compile(source)
            {
                let projection = module.agent_text().unwrap();
                assert_eq!(
                    Module::compile_agent(&projection).unwrap().encode(),
                    module.encode()
                );
            }
        }
    }
    for depth in [96, 97, 1024, 16_384] {
        let nested = format!("fa1 body 1 ret {}true", "not ".repeat(depth));
        assert!(Stmt::parse_agent_body(&nested).is_err());
    }
}

#[test]
fn every_checked_released_fsm_fixture_preserves_exact_artifact_bytes() {
    fn walk(root: &Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out)
            } else if path.extension().is_some_and(|x| x == "fsm") {
                out.push(path)
            }
        }
    }
    let mut files = vec![];
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"),
        &mut files,
    );
    files.sort();
    assert!(files.len() >= 8);
    for path in files {
        let bytes = fs::read(&path).unwrap();
        let original = Module::decode(&bytes).unwrap();
        let text = original.agent_text().unwrap();
        assert_eq!(
            Module::compile_agent(&text).unwrap().encode(),
            bytes,
            "{}",
            path.display()
        );
    }
}

#[test]
fn cli_refusals_preserve_existing_outputs_including_projection_expansion() {
    use std::process::Command;
    let temp = std::env::temp_dir().join(format!("fluxr-fa1-independent-{}", std::process::id()));
    fs::create_dir(&temp).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(temp.clone());
    let sentinel = b"existing validated output must survive refused ingress";
    let module = Module::compile_agent("fa1 v2 pub f 0 int ret 7").unwrap();
    let bytes = module.encode();
    let mut corrupt = bytes.clone();
    corrupt[4] = 255;
    let mut trailing = bytes.clone();
    trailing.extend_from_slice(b"trailing data");
    let cli = |command: &str| {
        Command::new(env!("CARGO_BIN_EXE_fluxr-cli"))
            .current_dir(&temp)
            .args([command, "input", "output"])
            .output()
            .unwrap()
    };
    for (command, input) in [
        ("project-agent", bytes[..5].to_vec()),
        ("project-agent", corrupt),
        ("project-agent", trailing),
        (
            "project-agent",
            b"{\"version\":2,\"functions\":[]}".to_vec(),
        ),
        ("encode-agent", b"fa1 v2 pub f 0 int ret true".to_vec()),
        ("encode-agent", b"fa1 v2 pub f 0 int ret 7\0".to_vec()),
        (
            "encode-agent",
            "fa1 v2 pub caf\u{00e9} 0 int ret 7".as_bytes().to_vec(),
        ),
        ("encode-agent", vec![b' '; 256 * 1024 + 1]),
    ] {
        fs::write(temp.join("input"), input).unwrap();
        fs::write(temp.join("output"), sentinel).unwrap();
        assert!(
            !cli(command).status.success(),
            "accepted invalid {command} input"
        );
        assert_eq!(fs::read(temp.join("output")).unwrap(), sentinel);
    }

    // A valid released artifact can expand beyond FA1's authoring byte budget.
    // This is a bounded, explicit projection refusal, never partial publication.
    let name = "x".repeat(64);
    let mut source = Source {
        version: 2,
        inputs: vec![],
        records: vec![],
        enums: vec![],
        functions: vec![],
    };
    for index in 0..256 {
        let local = || Expr::Local { name: name.clone() };
        let mut value = local();
        for _ in 1..16 {
            value = Expr::Binary {
                kind: fluxr::structured::Binary::Add,
                left: Box::new(value),
                right: Box::new(local()),
            };
        }
        source.functions.push(function(
            &format!("f{index}"),
            vec![Binding {
                name: name.clone(),
                ty: Type::I64,
            }],
            Type::I64,
            value,
        ));
    }
    let expanded = Module::compile(source).unwrap();
    assert!(expanded.agent_text().is_err());
    fs::write(temp.join("input"), expanded.encode()).unwrap();
    fs::write(temp.join("output"), sentinel).unwrap();
    let refusal = cli("project-agent");
    assert!(!refusal.status.success());
    assert!(String::from_utf8_lossy(&refusal.stderr).contains("byte budget"));
    assert_eq!(fs::read(temp.join("output")).unwrap(), sentinel);

    // Leading whitespace on the released JSON reader and exact source-byte
    // boundaries retain their existing, explicit dialect dispatch behavior.
    let mut legacy = b" \t\r\n".to_vec();
    legacy.extend_from_slice(&bytes[6..]);
    fs::write(temp.join("input"), legacy).unwrap();
    assert!(cli("project-agent").status.success());
    let text = fs::read_to_string(temp.join("output")).unwrap();
    assert_eq!(Module::compile_agent(&text).unwrap().encode(), bytes);
    let mut padded = "fa1 v2 pub f 0 int ret 7".to_string();
    padded.extend(std::iter::repeat_n(' ', 256 * 1024 - padded.len()));
    fs::write(temp.join("input"), padded).unwrap();
    assert!(cli("encode-agent").status.success());
    assert_eq!(fs::read(temp.join("output")).unwrap(), bytes);
}
