use fluxr::structured::{Invocation, Limits, SourceDocument, Value};

#[test]
fn cross_document_calls_preserve_single_entry_exports_and_exact_host_grants() {
    let modules = SourceDocument::compile_entry_modules([
        "fluxr 1\nfn inc 1 x int int ret add x 1",
        "fluxr 1\npub left 0 int ret call inc 1 5\npub right 0 int ret host clock int 0",
    ])
    .unwrap();
    assert_eq!(modules.len(), 2);
    let left = &modules["left"];
    assert!(left.host_requirements().unwrap().is_empty());
    assert_eq!(
        left.export_signatures().keys().collect::<Vec<_>>(),
        vec!["left"]
    );
    assert_eq!(
        left.run(Invocation {
            export: "left",
            inputs: &[],
            arguments: &[],
            limits: Limits::default()
        })
        .unwrap(),
        Value::I64(6)
    );
    assert_eq!(modules["right"].host_requirements().unwrap().len(), 1);
}

#[test]
fn entry_modules_reject_duplicates_unknown_dependencies_and_native_items() {
    let twice = "fluxr 1\npub x 0 int ret 1";
    assert!(SourceDocument::compile_entry_modules([twice, twice]).is_err());
    assert!(
        SourceDocument::compile_entry_modules(["fluxr 1\npub x 0 int ret call missing 0"]).is_err()
    );
    assert!(
        SourceDocument::compile_entry_modules([twice, "fluxr 1\nnative-use core::fmt"]).is_err()
    );
    assert!(SourceDocument::compile_entry_modules(["fluxr 1\nfn private 0 int ret 3"]).is_err());
}
