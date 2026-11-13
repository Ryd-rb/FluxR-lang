//! Reproducible single-process collaboration microbenchmark.
//!
//! Run with `cargo run --release --example bench_collaboration -- --repeats 3`.
//! JSON goes to stdout. Compilation, verification and runtime execution are
//! measured separately or kept outside query timing. This does not launch agents
//! or measure distributed contention, merge throughput, or a production service.

use fluxr::collaboration::{Change, Edit, Mode, Revision, Snapshot, Target};
use fluxr::structured::{Binary, Expr, Invocation, Limits, Source, Stmt, Value};
use serde::Serialize;
use std::{collections::BTreeMap, error::Error, hint::black_box, process::Command, time::Instant};

const DIMENSIONS: [usize; 3] = [8, 32, 128];
const LEAF_STEPS: i64 = 12;

#[derive(Clone, Copy, Serialize)]
struct Options {
    repeats: usize,
    warm_iterations: usize,
    edit_iterations: usize,
}

impl Options {
    fn parse() -> Result<Self, Box<dyn Error>> {
        let mut options = Self {
            repeats: 3,
            warm_iterations: 1000,
            edit_iterations: 40,
        };
        let mut args = std::env::args().skip(1);
        while let Some(flag) = args.next() {
            let value: usize = args.next().ok_or("each option needs an integer")?.parse()?;
            match flag.as_str() {
                "--repeats" if (3..=30).contains(&value) => options.repeats = value,
                "--warm-iterations" if (10..=100_000).contains(&value) => {
                    options.warm_iterations = value
                }
                "--edit-iterations" if (1..=1000).contains(&value) => {
                    options.edit_iterations = value
                }
                _ => return Err(
                    "usage: --repeats 3..30 --warm-iterations 10..100000 --edit-iterations 1..1000"
                        .into(),
                ),
            }
        }
        Ok(options)
    }
}

#[derive(Serialize)]
struct Sample {
    targets: usize,
    repeat: usize,
    compact_source_bytes: usize,
    emitted_module_bytes: usize,
    changed_roots_per_edit: usize,
    checked_generations: usize,
    initial_snapshot_build_ns: u64,
    initial_cold_all_queries_ns: u64,
    warm_all_queries_total_ns: u64,
    warm_all_queries_mean_ns: f64,
    sparse_prepare_total_ns: u64,
    sparse_prepare_mean_ns: f64,
    sparse_all_queries_total_ns: u64,
    sparse_all_queries_mean_ns: f64,
    cold_edited_snapshot_build_total_ns: u64,
    cold_edited_snapshot_build_mean_ns: f64,
    cold_edited_all_queries_total_ns: u64,
    cold_edited_all_queries_mean_ns: f64,
}

fn elapsed(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_nanos()).expect("benchmark duration fits u64 nanoseconds")
}

fn name(index: usize) -> String {
    format!("f{index:03}")
}

fn workload(count: usize) -> Result<(Source, Vec<String>, usize), fluxr::Error> {
    assert!(count > 0 && count.is_multiple_of(2));
    let names: Vec<_> = (0..count).map(name).collect();
    let mut text = String::from("v1\n");
    for (index, name) in names.iter().enumerate() {
        if index % 2 == 0 {
            text.push_str(&format!("(pub {name} (x:i64) i64 (acc:i64) (= acc x) "));
            for _ in 0..LEAF_STEPS {
                text.push_str(&format!("(= acc (+ acc {})) ", index + 1));
            }
            text.push_str("(ret acc))\n");
        } else {
            text.push_str(&format!(
                "(pub {name} (x:i64) i64 () (ret (call {} x)))\n",
                names[index - 1]
            ));
        }
    }
    let source_bytes = text.len();
    Ok((Source::parse_compact(&text)?, names, source_bytes))
}

fn snapshot(source: &Source, names: &[String]) -> Result<Snapshot, fluxr::Error> {
    Snapshot::new(
        source.clone(),
        names
            .iter()
            .map(|name| Target {
                id: name.clone(),
                function: name.clone(),
                receiver: None,
                mode: Mode::Exclusive,
            })
            .collect(),
    )
}

fn queries(snapshot: &Snapshot, names: &[String]) -> Result<Vec<Revision>, fluxr::Error> {
    names
        .iter()
        .map(|name| {
            snapshot
                .behavior_revision(black_box(name))
                .map(|revision| revision.expect("all workload targets were registered"))
        })
        .collect()
}

fn edit_body(generation: i64, source: &Source) -> Vec<Stmt> {
    let mut body = source.functions[0].body.clone();
    *body.last_mut().expect("workload return") = Stmt::Return {
        value: Expr::Binary {
            kind: Binary::Add,
            left: Box::new(Expr::Local { name: "acc".into() }),
            right: Box::new(Expr::I64 { value: generation }),
        },
    };
    body
}

fn verify_outputs(
    snapshot: &Snapshot,
    names: &[String],
    generation: i64,
) -> Result<(), fluxr::Error> {
    for (index, name) in names.iter().enumerate() {
        let leaf = index - index % 2;
        let expected = 7 + LEAF_STEPS * (leaf as i64 + 1) + if leaf == 0 { generation } else { 0 };
        let actual = snapshot.module().run(Invocation {
            export: name,
            inputs: &[],
            arguments: &[Value::I64(7)],
            limits: Limits::default(),
        })?;
        assert_eq!(
            actual,
            Value::I64(expected),
            "independent arithmetic oracle for {name}"
        );
    }
    Ok(())
}

fn run_sample(count: usize, repeat: usize, options: Options) -> Result<Sample, Box<dyn Error>> {
    let (mut source, names, source_bytes) = workload(count)?;
    // Compute independently initialized reference pins outside all query timing.
    let reference = snapshot(&source, &names)?;
    let reference_pins = queries(&reference, &names)?;
    verify_outputs(&reference, &names, 0)?;

    let start = Instant::now();
    let mut current = snapshot(&source, &names)?;
    let initial_snapshot_build_ns = elapsed(start);
    assert_eq!(current.module().encode(), reference.module().encode());
    assert_eq!(current.affected_targets([names[0].as_str()])?.len(), 2);

    let start = Instant::now();
    let cold_pins = queries(&current, &names)?;
    let initial_cold_all_queries_ns = elapsed(start);
    assert_eq!(cold_pins, reference_pins);

    let start = Instant::now();
    for _ in 0..options.warm_iterations {
        black_box(queries(black_box(&current), black_box(&names))?);
    }
    let warm_all_queries_total_ns = elapsed(start);
    assert_eq!(queries(&current, &names)?, reference_pins);

    let mut sparse_prepare_total_ns = 0;
    let mut sparse_all_queries_total_ns = 0;
    let mut cold_edited_snapshot_build_total_ns = 0;
    let mut cold_edited_all_queries_total_ns = 0;
    for iteration in 0..options.edit_iterations {
        let generation = iteration as i64 + 1;
        let body = edit_body(generation, &source);
        let change = Change {
            author: "single-process-microbenchmark".into(),
            requires: BTreeMap::new(),
            edits: vec![Edit::ReplaceBody {
                target: names[0].clone(),
                expected: current.edit_revision(&names[0]).unwrap(),
                body: body.clone(),
            }],
        };
        let start = Instant::now();
        let candidate = current.prepare(black_box(std::slice::from_ref(&change)))?;
        sparse_prepare_total_ns += elapsed(start);
        assert_eq!(candidate.accepted.len(), 1);
        assert!(candidate.rejected.is_empty());
        let next = candidate.snapshot();
        let start = Instant::now();
        let incremental_pins = queries(next, &names)?;
        sparse_all_queries_total_ns += elapsed(start);

        source.functions[0].body = body;
        let start = Instant::now();
        let cold = snapshot(&source, &names)?;
        cold_edited_snapshot_build_total_ns += elapsed(start);
        let start = Instant::now();
        let cold_pins = queries(&cold, &names)?;
        cold_edited_all_queries_total_ns += elapsed(start);

        // Compare all pins and canonical module bytes before retaining a timing
        // result. The arithmetic oracle also checks actual execution separately.
        assert_eq!(incremental_pins, cold_pins);
        assert_eq!(next.module().encode(), cold.module().encode());
        verify_outputs(next, &names, generation)?;
        verify_outputs(&cold, &names, generation)?;
        current = next.clone();
    }
    Ok(Sample {
        targets: count,
        repeat,
        compact_source_bytes: source_bytes,
        emitted_module_bytes: current.module().encode().len(),
        changed_roots_per_edit: 2,
        checked_generations: options.edit_iterations + 1,
        initial_snapshot_build_ns,
        initial_cold_all_queries_ns,
        warm_all_queries_total_ns,
        warm_all_queries_mean_ns: warm_all_queries_total_ns as f64 / options.warm_iterations as f64,
        sparse_prepare_total_ns,
        sparse_prepare_mean_ns: sparse_prepare_total_ns as f64 / options.edit_iterations as f64,
        sparse_all_queries_total_ns,
        sparse_all_queries_mean_ns: sparse_all_queries_total_ns as f64
            / options.edit_iterations as f64,
        cold_edited_snapshot_build_total_ns,
        cold_edited_snapshot_build_mean_ns: cold_edited_snapshot_build_total_ns as f64
            / options.edit_iterations as f64,
        cold_edited_all_queries_total_ns,
        cold_edited_all_queries_mean_ns: cold_edited_all_queries_total_ns as f64
            / options.edit_iterations as f64,
    })
}

fn command_output(program: &str, arguments: &[&str]) -> Option<String> {
    let output = Command::new(program).args(arguments).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn main() -> Result<(), Box<dyn Error>> {
    if cfg!(debug_assertions) {
        return Err(
            "benchmark requires --release; debug timings must not be reported as release".into(),
        );
    }
    let options = Options::parse()?;
    let mut samples = Vec::new();
    for repeat in 0..options.repeats {
        // Alternate size order to expose rather than silently fix warm-up order.
        let dimensions = if repeat % 2 == 0 {
            DIMENSIONS
        } else {
            [128, 32, 8]
        };
        for count in dimensions {
            samples.push(run_sample(count, repeat, options)?);
        }
    }
    let output = serde_json::json!({
        "schema": "fluxr-collaboration-microbenchmark-v1",
        "scope": "Single process, serial synthetic leaf/caller pairs; no autonomous agents or distributed services",
        "timing_unit": "nanoseconds",
        "timing_method": "Instant wall time; query phases include result-vector allocation; validation is outside timed phases",
        "ordering": "Cold build/query then warm loop; each edit uses prepare, sparse query, cold build/query, oracle validation",
        "profile": "release",
        "crate_version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "logical_parallelism_visible": std::thread::available_parallelism().map(|value| value.get()).ok(),
        "rustc_version_observed_at_run": command_output("rustc", &["-Vv"]),
        "checkout_head_observed_at_run": command_output("git", &["rev-parse", "HEAD"]),
        "checkout_status_observed_at_run": command_output("git", &["status", "--porcelain=v1"]),
        "options": options,
        "samples": samples,
        "verification": "All canonical bytes, behavioral pins and independent arithmetic outputs agreed in every retained generation",
        "limitations": [
            "No CPU pinning, isolation from host load, statistical confidence interval or broad performance claim",
            "No measurement of general compiler queries, test selection, merge queues or remote execution",
            "Snapshot construction includes source cloning and target binding; sparse prepare includes canonical serialization"
        ]
    });
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimensions_use_exact_targets_and_two_root_sparse_impact() {
        for count in DIMENSIONS {
            let (source, names, _) = workload(count).unwrap();
            let snapshot = snapshot(&source, &names).unwrap();
            assert_eq!(
                snapshot.dependency_manifest().unwrap().declarations.len(),
                count
            );
            assert_eq!(
                snapshot
                    .affected_targets([names[0].as_str()])
                    .unwrap()
                    .len(),
                2
            );
            verify_outputs(&snapshot, &names, 0).unwrap();
        }
    }

    #[test]
    fn benchmark_sample_checks_every_edited_generation() {
        let sample = run_sample(
            8,
            0,
            Options {
                repeats: 3,
                warm_iterations: 10,
                edit_iterations: 3,
            },
        )
        .unwrap();
        assert_eq!(sample.checked_generations, 4);
        assert_eq!(sample.changed_roots_per_edit, 2);
    }
}
