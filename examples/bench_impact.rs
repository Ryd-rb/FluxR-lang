//! Impact-query locality benchmark, portable to the v4 implementation.
//! Run in release mode; output separates cold and warm queries from preparation.
use fluxr::collaboration::{Change, Edit, Mode, Snapshot, Target};
use fluxr::structured::{Expr, Invocation, Limits, Source, Stmt, Value};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    hint::black_box,
    time::Instant,
};

#[derive(Serialize)]
struct Sample {
    graph: &'static str,
    declarations: usize,
    repeat: usize,
    build_ns: u128,
    prepare_ns: u128,
    noop_prepare_mean_ns: u128,
    cold_impact_ns: u128,
    warm_impact_mean_ns: u128,
    local_reverse_query_mean_ns: u128,
    affected: usize,
}

fn name(index: usize) -> String {
    format!("f{index:03}")
}
fn run(count: usize, chain: bool, repeat: usize) -> Result<Sample, Box<dyn Error>> {
    let mut text = String::from("v1 ");
    for i in 0..count {
        let body = if chain && i != 0 {
            format!("(call {})", name(i - 1))
        } else {
            "0".into()
        };
        text.push_str(&format!("(pub {} () i64 () (ret {body})) ", name(i)));
    }
    let source = Source::parse_compact(&text)?;
    let targets: Vec<_> = (0..count)
        .map(|i| Target {
            id: name(i),
            function: name(i),
            receiver: None,
            mode: Mode::Exclusive,
        })
        .collect();
    let start = Instant::now();
    let base = Snapshot::new(source.clone(), targets.clone())?;
    let build_ns = start.elapsed().as_nanos();
    let body = vec![Stmt::Return {
        value: Expr::I64 { value: 1 },
    }];
    let changes = [Change {
        author: "impact-benchmark".into(),
        requires: BTreeMap::new(),
        edits: vec![Edit::ReplaceBody {
            target: name(0),
            expected: base.edit_revision(&name(0)).unwrap(),
            body: body.clone(),
        }],
    }];
    let start = Instant::now();
    let prepared = base.prepare(black_box(&changes))?;
    let prepare_ns = start.elapsed().as_nanos();
    assert_eq!(prepared.accepted.len(), 1);
    assert!(prepared.rejected.is_empty());
    let next = prepared.snapshot();
    let start = Instant::now();
    let cold = next.impact_since(black_box(&base))?;
    let cold_impact_ns = start.elapsed().as_nanos();
    let expected: Vec<_> = if chain {
        (0..count).map(name).collect()
    } else {
        vec![name(0)]
    };
    assert_eq!(cold.affected_targets, expected);
    assert_eq!(cold.changed_declarations.len(), 1);
    assert!(cold.changed_contracts.is_empty());
    // Independent full closure identity oracle, outside measured query work.
    let observed: Vec<_> = (0..count)
        .map(name)
        .filter(|target| {
            base.behavior_revision(target).unwrap() != next.behavior_revision(target).unwrap()
        })
        .collect();
    assert_eq!(observed, expected);
    let mut edited = source;
    edited.functions[0].body = body;
    let cold_module = Snapshot::new(edited, targets)?;
    assert_eq!(next.module().encode(), cold_module.module().encode());
    let output = next.module().run(Invocation {
        export: &name(count - 1),
        inputs: &[],
        arguments: &[],
        limits: Limits {
            instructions: 4096,
            frames: 512,
            cells: 100_000,
        },
    })?;
    assert_eq!(output, Value::I64(if chain { 1 } else { 0 }));
    let start = Instant::now();
    for _ in 0..100 {
        black_box(next.impact_since(black_box(&base))?);
    }
    let warm_impact_mean_ns = start.elapsed().as_nanos() / 100;
    let root = name(count - 1);
    assert_eq!(
        next.affected_targets([root.as_str()])?,
        BTreeSet::from([root.clone()])
    );
    let start = Instant::now();
    for _ in 0..1000 {
        black_box(next.affected_targets([black_box(root.as_str())])?);
    }
    let local_reverse_query_mean_ns = start.elapsed().as_nanos() / 1000;
    let start = Instant::now();
    for _ in 0..100 {
        assert_eq!(black_box(next.prepare(&[])?).snapshot().id(), next.id());
    }
    let noop_prepare_mean_ns = start.elapsed().as_nanos() / 100;
    Ok(Sample {
        graph: if chain { "chain" } else { "independent" },
        declarations: count,
        repeat,
        build_ns,
        prepare_ns,
        noop_prepare_mean_ns,
        cold_impact_ns,
        warm_impact_mean_ns,
        local_reverse_query_mean_ns,
        affected: expected.len(),
    })
}

fn main() -> Result<(), Box<dyn Error>> {
    if cfg!(debug_assertions) {
        return Err("use --release for benchmark timings".into());
    }
    let mut samples = Vec::new();
    for repeat in 0..5 {
        for count in if repeat % 2 == 0 {
            [16, 64, 256]
        } else {
            [256, 64, 16]
        } {
            for chain in [false, true] {
                samples.push(run(count, chain, repeat)?);
            }
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema": "fluxr-impact-locality-benchmark-v1", "profile": "release",
            "scope": "single-process synthetic graphs; no agent or distributed throughput claim",
            "os": std::env::consts::OS, "arch": std::env::consts::ARCH,
            "verification": "every report matches exhaustive behavior pins and exact cold module bytes; last export matches arithmetic oracle",
            "limitations": "wall time, shared host, no CPU pinning or confidence interval; warm impact includes allocations; no-op timing includes identity assertion",
            "samples": samples,
        }))?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn benchmark_oracles_cover_chain_and_sparse_graphs() {
        for chain in [false, true] {
            assert_eq!(
                run(16, chain, 0).unwrap().affected,
                if chain { 16 } else { 1 }
            );
        }
    }
}
