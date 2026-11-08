use fluxr::{Program, RuleProgram, telemetry};
use serde_json::Value;

const TEXT: &str = "v1 20\n1 1 1 0:0 20:20\n1 2 0 0:2";

fn feature<'a>(snapshot: &'a Value, name: &str) -> &'a Value {
    snapshot["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|operation| operation["feature"] == name)
        .unwrap()
}

#[test]
fn selection_counts_exact_operations_without_changing_results() {
    let original = Program::compile_text(TEXT).unwrap();
    let encoded = original.encode();
    let typed = original.structured();
    let rule = RuleProgram::compile_text("r1 1 i0 2 *").unwrap();
    assert_eq!(
        feature(&telemetry::snapshot(), "compile-text")["accepted"],
        0
    );

    telemetry::configure("all").unwrap();
    assert!(telemetry::configure("invalid-feature").is_err());
    assert_eq!(Program::compile_text(TEXT).unwrap(), original);
    assert!(Program::compile_text("v2 secret-test-input").is_err());
    assert_eq!(Program::compile_structured(&typed).unwrap(), original);
    assert_eq!(Program::decode(&encoded).unwrap(), original);
    assert!(Program::decode(&[0]).is_err());
    assert_eq!(original.encode(), encoded);
    assert_eq!(original.replay("[[1,1,2,2]]").unwrap(), 1);
    assert!(original.replay("[[1,1,2,1]]").is_err());
    assert_eq!(original.sample((1, 1), 3), Some(3));
    assert_eq!(original.sample((1, 3), 3), None);
    assert_eq!(original.sample_all(4).collect::<Vec<_>>(), [4, 2]);
    telemetry::reload(true, TEXT.len(), 5);
    telemetry::reload(false, 8, 51);
    assert_eq!(rule.evaluate(&[4]).unwrap(), 8);
    assert!(rule.evaluate(&[i64::MAX]).is_err());
    telemetry::record_rule_evaluations(3, 2);

    let snapshot = telemetry::snapshot();
    for (name, accepted, rejected) in [
        ("compile-text", 1, 1),
        ("compile-typed", 1, 0),
        ("encode", 1, 0),
        ("decode", 1, 1),
        ("replay", 1, 1),
        ("sample", 1, 1),
        ("sample-batch", 1, 0),
        ("reload", 1, 1),
        ("rule-evaluate", 4, 3),
    ] {
        let stats = feature(&snapshot, name);
        assert_eq!(stats["accepted"], accepted, "{name}");
        assert_eq!(stats["rejected"], rejected, "{name}");
    }
    assert_eq!(feature(&snapshot, "compile-text")["keys"], 3);
    assert_eq!(feature(&snapshot, "sample-batch")["tracks"], 2);
    assert_eq!(feature(&snapshot, "encode")["output_bytes"], encoded.len());
    assert_eq!(feature(&snapshot, "replay")["assertions"], 1);
    let histogram = &feature(&snapshot, "reload")["observed_latency"];
    assert_eq!(histogram[1], 1);
    assert_eq!(histogram[3], 1);
    assert!(!snapshot.to_string().contains("secret-test-input"));

    telemetry::configure("compile-text").unwrap();
    Program::compile_text(TEXT).unwrap();
    assert_eq!(original.sample((1, 1), 3), Some(3));
    let next = telemetry::snapshot();
    assert_eq!(feature(&next, "compile-text")["accepted"], 2);
    assert_eq!(feature(&next, "sample")["accepted"], 1);
    assert_eq!(feature(&next, "sample")["enabled"], false);
}
