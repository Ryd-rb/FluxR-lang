//! Matched FA1/compact front-end comparison; runtime bytes must be identical.
//! Run with `cargo run --locked --release --example bench_agent_source`.
use fluxr::structured::{Module, Source};
use std::{hint::black_box, time::Instant};

fn measure<T>(mut operation: impl FnMut() -> T) -> serde_json::Value {
    let mut rounds = Vec::new();
    for _ in 0..31 {
        let start = Instant::now();
        for _ in 0..100 {
            black_box(operation());
        }
        rounds.push(start.elapsed().as_nanos() as u64 / 100);
    }
    rounds.sort_unstable();
    serde_json::json!({"median_ns": rounds[15], "p95_ns": rounds[29], "rounds": 31, "calls_per_round": 100})
}

fn main() {
    let mut results = Vec::new();
    for (name, compact) in [
        ("scalar", include_str!("fixtures/modern_leaf.flux")),
        ("methods", include_str!("fixtures/method_call_order.fluxr")),
        (
            "nested_values",
            include_str!("fixtures/shared_host_admission.fluxr"),
        ),
    ] {
        let module = Module::compile_compact(compact).unwrap();
        let agent = module.agent_text().unwrap();
        assert_eq!(
            Source::parse_compact(compact).unwrap(),
            Source::parse_agent(&agent).unwrap()
        );
        assert_eq!(
            module.encode(),
            Module::compile_agent(&agent).unwrap().encode()
        );
        results.push(serde_json::json!({
            "name": name, "compact_bytes": compact.len(), "agent_bytes": agent.len(),
            "canonical_bytes_equal": true,
            "compact_parse": measure(|| Source::parse_compact(black_box(compact)).unwrap()),
            "agent_parse": measure(|| Source::parse_agent(black_box(&agent)).unwrap()),
            "compact_compile": measure(|| Module::compile_compact(black_box(compact)).unwrap()),
            "agent_compile": measure(|| Module::compile_agent(black_box(&agent)).unwrap()),
            "checked_agent_projection": measure(|| module.agent_text().unwrap()),
        }));
    }
    println!("{}", serde_json::to_string_pretty(&serde_json::json!({
        "schema":"fluxr-agent-front-end-benchmark-v1", "profile":"release", "results":results,
        "scope":"Same source AST and FSM bytes; front-end wall time only, no agent or runtime speed claim."
    })).unwrap());
}
