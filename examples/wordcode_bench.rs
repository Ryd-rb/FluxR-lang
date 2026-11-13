//! Authored WK1 fixtures: exact expansion and measured in-process host overhead.
use fluxr::structured::{Invocation, Limits, Value, Workspace, wordcode};
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use std::{error::Error, hint::black_box, time::Instant};

#[path = "../benchmarks/wordcode-v14/affine.rs"]
mod affine;
#[path = "../benchmarks/wordcode-v14/forest.rs"]
mod forest;
#[path = "../benchmarks/wordcode-v14/polynomial.rs"]
mod polynomial;
#[path = "../benchmarks/wordcode-v14/repeat.rs"]
mod repeat;
#[path = "../benchmarks/wordcode-v14/walk.rs"]
mod walk;

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn samples(mut call: impl FnMut(), rounds: usize, calls: usize) -> Json {
    let mut values = Vec::with_capacity(rounds);
    for _ in 0..rounds {
        let start = Instant::now();
        for _ in 0..calls {
            call();
        }
        values.push(start.elapsed().as_nanos() as f64 / calls as f64);
    }
    let mut sorted = values.clone();
    sorted.sort_by(f64::total_cmp);
    json!({"rounds":rounds,"calls_per_round":calls,"batch_average_ns_per_call":values,
        "median_batch_average_ns":sorted[rounds / 2],"individual_call_tail_latency":false})
}

struct Case {
    name: &'static str,
    source: &'static str,
    expanded: &'static str,
    dictionary: Option<&'static str>,
    arguments: Vec<i64>,
    reference: fn(&[i64]) -> Option<i64>,
    expected: i64,
}

fn main() -> Result<(), Box<dyn Error>> {
    let cases = [
        Case {
            name: "affine",
            source: include_str!("../benchmarks/wordcode-v14/affine.wk1"),
            expanded: include_str!("../benchmarks/wordcode-v14/affine.fa1"),
            dictionary: None,
            arguments: vec![7, 13, 3],
            reference: |a| affine::calc(a[0], a[1], a[2]),
            expected: 94,
        },
        Case {
            name: "polynomial",
            source: include_str!("../benchmarks/wordcode-v14/polynomial.wk1"),
            expanded: include_str!("../benchmarks/wordcode-v14/polynomial.fa1"),
            dictionary: None,
            arguments: vec![7, 13, 3],
            reference: |a| polynomial::calc(a[0], a[1], a[2]),
            expected: 78,
        },
        Case {
            name: "repeat",
            source: include_str!("../benchmarks/wordcode-v14/repeat.wk1"),
            expanded: include_str!("../benchmarks/wordcode-v14/repeat.fa1"),
            dictionary: None,
            arguments: vec![12, 7],
            reference: |a| repeat::calc(a[0], a[1]),
            expected: 19,
        },
        Case {
            name: "walk",
            source: include_str!("../benchmarks/wordcode-v14/walk.wk1"),
            expanded: include_str!("../benchmarks/wordcode-v14/walk.fa1"),
            dictionary: None,
            arguments: vec![2, 23, 3],
            reference: |a| walk::calc(a[0], a[1], a[2]),
            expected: 23,
        },
        Case {
            name: "forest",
            source: include_str!("../benchmarks/wordcode-v14/forest.wk1"),
            expanded: include_str!("../benchmarks/wordcode-v14/forest.fa1"),
            dictionary: Some(include_str!(
                "../benchmarks/wordcode-v14/forest.dictionary.json"
            )),
            arguments: vec![1],
            reference: |a| forest::calc(a[0]),
            expected: 1,
        },
    ];
    let mut observations = Vec::new();
    for case in cases {
        let make_dictionary = || match case.dictionary {
            Some(text) => wordcode::Dictionary::from_json(text).unwrap(),
            None => wordcode::Dictionary::core(),
        };
        let dictionary_admission = samples(
            || {
                black_box(make_dictionary());
            },
            5,
            10,
        );
        let dictionary = make_dictionary();
        let compiled = wordcode::compile(
            case.source,
            &dictionary,
            wordcode::ExpansionLimits::default(),
        )?;
        let reference = fluxr::structured::Module::compile_agent(case.expanded)?;
        if compiled.module.encode() != reference.encode() {
            return Err("canonical expansion differs".into());
        }
        let expansion = samples(
            || {
                black_box(
                    wordcode::compile(
                        black_box(case.source),
                        &dictionary,
                        wordcode::ExpansionLimits::default(),
                    )
                    .unwrap(),
                );
            },
            5,
            10,
        );
        let inputs: Vec<_> = case.arguments.iter().copied().map(Value::I64).collect();
        let module = compiled.module;
        let mut workspace = Workspace::default();
        let result = module.run_portable_with(
            Invocation {
                export: "calc",
                inputs: &[],
                arguments: &inputs,
                limits: Limits::default(),
            },
            &mut workspace,
        )?;
        if result != Value::I64(case.expected)
            || (case.reference)(&case.arguments) != Some(case.expected)
        {
            return Err("independent arithmetic value mismatch".into());
        }
        let preparation_start = Instant::now();
        let prepared = module.prepare_call("calc")?;
        let preparation_seconds = preparation_start.elapsed().as_secs_f64();
        let (prepared_result, route) =
            prepared.run_with_route(&[], &inputs, Limits::default(), &mut workspace);
        if prepared_result? != result {
            return Err("prepared value mismatch".into());
        }
        let rust = samples(
            || {
                let _ = black_box((case.reference)(black_box(&case.arguments)));
            },
            9,
            2_000,
        );
        let portable = samples(
            || {
                black_box(
                    module
                        .run_portable_with(
                            Invocation {
                                export: "calc",
                                inputs: &[],
                                arguments: black_box(&inputs),
                                limits: Limits::default(),
                            },
                            &mut workspace,
                        )
                        .unwrap(),
                );
            },
            9,
            2_000,
        );
        let native = samples(
            || {
                black_box(
                    prepared
                        .run_with(&[], black_box(&inputs), Limits::default(), &mut workspace)
                        .unwrap(),
                );
            },
            9,
            2_000,
        );
        let module_sha256 = digest(&module.encode());
        observations.push(json!({"name":case.name,"class":if case.name=="forest" {"synthetic_density_stress"} else {"authored_practical_microcase"},
            "source_sha256":digest(case.source.as_bytes()),"dictionary_sha256":dictionary.identity(),
            "reference_fa1_sha256":digest(case.expanded.as_bytes()),"module_sha256":module_sha256,
            "canonical_bytes_equal":true,"arguments":case.arguments,"expected":case.expected,
            "portable_value":result,"prepared_value":result,"rust_value":case.expected,
            "route":format!("{route:?}"),"dictionary_install_and_drop":dictionary_admission,
            "expansion_check_projection_receipt_and_drop":expansion,"native_preparation_seconds":preparation_seconds,
            "rust_host":rust,"portable_host":portable,"prepared_host":native,"expansion_report":compiled.report}));
    }
    println!(
        "{}",
        json!({"schema":"fluxr-wordcode-benchmark-v1","status":"passed",
        "binary_sha256":digest(&std::fs::read(std::env::current_exe()?)?),"cases":observations,
        "scope":"Authored fixed scalar arithmetic/termination domains. Rust host excludes VM logical resource accounting. Exact VM fuel/cells/frames/trap equivalence is separately tested against expanded FA1. Batch averages are not per-call tails; no representative workload or global Rust parity claim."})
    );
    Ok(())
}
