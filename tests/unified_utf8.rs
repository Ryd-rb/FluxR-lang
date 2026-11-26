//! Current-source UTF-8 notation lowers to the released checked byte-list AST.
use fluxr::{
    Error,
    structured::{HostServices, Invocation, Limits, Source, SourceDocument, Value, Workspace},
};
use std::sync::Arc;

fn source(expression: &str) -> String {
    format!("fluxr 1\npub bytes 0 list uint ret {expression}")
}
fn expanded(bytes: &[u8]) -> String {
    format!(
        "list uint {} {}",
        bytes.len(),
        bytes
            .iter()
            .map(|b| format!("{b}u"))
            .collect::<Vec<_>>()
            .join(" ")
    )
}
fn execute(module: &fluxr::structured::Module, limits: Limits) -> Result<Value, String> {
    module
        .run_portable_with(
            Invocation {
                export: "bytes",
                inputs: &[],
                arguments: &[],
                limits,
            },
            &mut Workspace::default(),
        )
        .map_err(|e| e.to_string())
}

#[test]
fn retained_token_pair_matches_rust_byte_kernel() {
    let before = SourceDocument::parse(include_str!(
        "../benchmarks/unified-utf8-v1/greeting.before.fluxr"
    ))
    .unwrap();
    let after = SourceDocument::parse(include_str!(
        "../benchmarks/unified-utf8-v1/greeting.after.fluxr"
    ))
    .unwrap();
    assert_eq!(before.source, after.source);
    assert_eq!(
        before.compile().unwrap().encode(),
        after.compile().unwrap().encode()
    );
}

#[test]
fn builders_and_raw_text_preserve_portable_artifacts_and_execution() {
    let original = SourceDocument::parse(include_str!(
        "../benchmarks/unified-build-v1/controls.before.fluxr"
    ))
    .unwrap();
    let compact = SourceDocument::parse(include_str!(
        "../benchmarks/unified-build-v1/controls.after.fluxr"
    ))
    .unwrap();
    assert_eq!(compact.source, original.source);
    let module = compact.compile().unwrap();
    assert_eq!(module.encode(), original.compile().unwrap().encode());
    let value = module
        .run_portable_with(
            Invocation {
                export: "code",
                inputs: &[],
                arguments: &[],
                limits: Limits::default(),
            },
            &mut Workspace::default(),
        )
        .unwrap();
    assert_eq!(value, Value::I64(314));
    let literal = "café\n\\path\"#suffix";
    let raw = format!("@text r##\"{literal}\"##");
    assert_eq!(
        SourceDocument::parse(&source(&raw)).unwrap().source,
        SourceDocument::parse(&source(&expanded(literal.as_bytes())))
            .unwrap()
            .source
    );
    for malformed in [
        "@text abc",
        "@text r#########\"x\"#########",
        "@text r#\"x\"",
        "@text r\"\"tail",
        "@build f 0 1 0",
        "@build f 1 0 0",
        "@build f 1 2 0 1",
    ] {
        assert!(
            SourceDocument::parse(&source(malformed)).is_err(),
            "{malformed}"
        );
    }
    let unknown = SourceDocument::parse("fluxr 1\npub x 0 int ret @build missing 1 1 0 2").unwrap();
    assert!(unknown.compile().is_err());
}

#[test]
fn literals_match_independent_utf8_bytes_and_released_list_artifacts() {
    for (literal, expected) in [
        (r#"utf8 """#, vec![]),
        (r#"utf8 "RelAI tools""#, b"RelAI tools".to_vec()),
        (
            r#"utf8 "a\n\t\r\u0000\"\\/""#,
            vec![97, 10, 9, 13, 0, 34, 92, 47],
        ),
        (
            r#"utf8 "café 🦀""#,
            vec![99, 97, 102, 195, 169, 32, 240, 159, 166, 128],
        ),
        (
            r#"utf8 "\u00e9\ud83e\udd80""#,
            vec![195, 169, 240, 159, 166, 128],
        ),
    ] {
        let short = SourceDocument::parse(&source(literal)).unwrap();
        let long = SourceDocument::parse(&source(&expanded(&expected))).unwrap();
        assert_eq!(short.source, long.source);
        let a = short.compile().unwrap();
        let b = long.compile().unwrap();
        assert_eq!(a.encode(), b.encode());
        assert_eq!(
            execute(&a, Limits::default()).unwrap(),
            Value::List(Arc::new(
                expected.iter().map(|b| Value::U64(*b as u64)).collect()
            ))
        );
        let printed = a.unified_text().unwrap();
        assert!(printed.contains("utf8 "));
        assert_eq!(
            SourceDocument::parse(&printed)
                .unwrap()
                .compile()
                .unwrap()
                .encode(),
            a.encode()
        );
        let legacy = a.agent_text().unwrap();
        assert!(!legacy.contains("utf8 "));
        assert_eq!(Source::parse_agent(&legacy).unwrap(), short.source);
    }
}

#[test]
fn expanded_and_literal_preserve_all_vm_budget_failures() {
    let a = SourceDocument::parse(&source(r#"utf8 "é\nA""#))
        .unwrap()
        .compile()
        .unwrap();
    let b = SourceDocument::parse(&source("list uint 4 195u 169u 10u 65u"))
        .unwrap()
        .compile()
        .unwrap();
    for instructions in 0..48 {
        for cells in 0..24 {
            for frames in 0..3 {
                let limits = Limits {
                    instructions,
                    cells,
                    frames,
                };
                assert_eq!(
                    execute(&a, limits),
                    execute(&b, limits),
                    "fuel={instructions} cells={cells} frames={frames}"
                );
            }
        }
    }
}

#[test]
fn malformed_and_excessive_literals_are_rejected() {
    for expression in [
        "utf8",
        "utf8 true",
        r#"utf8 "bad\x00""#,
        r#"utf8 "\ud800""#,
        r#"utf8 "\udc00""#,
        r#"utf8 "unterminated"#,
        r#"utf8 "one"tail"#,
        "utf8 \"raw\nnewline\"",
    ] {
        assert!(
            SourceDocument::parse(&source(expression)).is_err(),
            "{expression}"
        );
    }
    let at_limit = format!("utf8 \"{}\"", "a".repeat(32768));
    assert!(SourceDocument::parse(&source(&at_limit)).is_ok());
    let over = format!("utf8 \"{}\"", "a".repeat(32769));
    assert!(
        SourceDocument::parse(&source(&over))
            .unwrap_err()
            .to_string()
            .contains("item limit")
    );
    let multibyte_over = format!("utf8 \"{}\"", "é".repeat(16385));
    assert!(SourceDocument::parse(&source(&multibyte_over)).is_err());
}

#[test]
fn literal_scanning_preserves_following_native_spans_and_local_names() {
    let native = "fn untouched() { let _ = \"é\"; }";
    let text = format!(
        "{} rust {}\n{} pub after 0 int ret 9",
        source(r#"utf8 "a b \"x\"""#),
        native.len(),
        native
    );
    let doc = SourceDocument::parse(&text).unwrap();
    assert_eq!(doc.native_spans.len(), 1);
    let span = &doc.native_spans[0];
    assert!(text[span.start..span.end].ends_with(native));
    assert!(doc.checked_text.contains("pub after"));
    assert!(!doc.checked_text.contains("untouched"));
    let named = SourceDocument::parse("fluxr 1\npub same 1 utf8 uint uint ret local utf8")
        .unwrap()
        .compile()
        .unwrap();
    let printed = named.unified_text().unwrap();
    assert!(printed.contains("local utf8"));
    assert_eq!(
        SourceDocument::parse(&printed)
            .unwrap()
            .compile()
            .unwrap()
            .encode(),
        named.encode()
    );
    // The released reader keeps utf8 as an ordinary unreserved local name.
    assert!(Source::parse_agent("fa1 v2 pub same 1 utf8 uint uint ret utf8").is_ok());
}

#[test]
fn invalid_utf8_or_nonbyte_lists_keep_expanded_printer() {
    for expression in [
        "list uint 1 255u",
        "list uint 1 256u",
        "list uint 1 add 1u 2u",
    ] {
        let module = SourceDocument::parse(&source(expression))
            .unwrap()
            .compile()
            .unwrap();
        let text = module.unified_text().unwrap();
        assert!(!text.contains("utf8 "));
        assert_eq!(
            SourceDocument::parse(&text)
                .unwrap()
                .compile()
                .unwrap()
                .encode(),
            module.encode()
        );
    }
}

#[test]
fn public_host_failure_constructor_keeps_diagnostic_and_workspace_recovers() {
    struct Fail;
    impl HostServices for Fail {
        fn call(&self, _: &str, _: &[Value]) -> Result<Value, Error> {
            Err(Error::host("fixture: exact diagnostic"))
        }
    }
    let module =
        SourceDocument::parse("fluxr 1\npub fail 0 int ret host fail int 0 pub good 0 int ret 7")
            .unwrap()
            .compile()
            .unwrap();
    let mut workspace = Workspace::default();
    let error = module
        .run_with_host(
            Invocation {
                export: "fail",
                inputs: &[],
                arguments: &[],
                limits: Limits::default(),
            },
            &mut workspace,
            &Fail,
        )
        .unwrap_err();
    assert_eq!(error.to_string(), "fixture: exact diagnostic");
    assert_eq!(
        module
            .run_with_host(
                Invocation {
                    export: "good",
                    inputs: &[],
                    arguments: &[],
                    limits: Limits::default()
                },
                &mut workspace,
                &Fail
            )
            .unwrap(),
        Value::I64(7)
    );
}
