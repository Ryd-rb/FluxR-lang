//! Bounded source lifecycle probe. This measures checked-source APIs, not the
//! Rust compiler, native-service execution, startup, or JIT invocation latency.
use fluxr::structured::{Invocation, Limits, SourceDocument, Value};
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use std::{hint::black_box, time::Instant};

const WARMUP: usize = 32;
const CHECKED: &str = "fluxr 1\npub run 1 x int int ret add x 17\n";

struct Case {
    name: &'static str,
    text: String,
    native_count: usize,
}

fn cases(decimal_literals: bool) -> Vec<Case> {
    let mut checked = CHECKED.to_owned();
    for index in 0..64 {
        checked.push_str(&format!(
            "fn helper{index} 1 x int int do 0 3 bump x 1 scale x 2 ret x\n"
        ));
    }
    let mut unquoted = CHECKED.to_owned();
    let mut quoted = CHECKED.to_owned();
    let mut unicode = CHECKED.to_owned();
    let mut raw = CHECKED.to_owned();
    for index in 0..64 {
        unquoted.push_str(&format!(
            "native-fn identity{index} 1 x i64 i64 1 tail path x\n"
        ));
        quoted.push_str(&format!(
            "native-fn \"identity{index}\" \"1\" \"x\" \"i64\" \"i64\" \"1\" \"tail\" \"path\" \"x\"\n"
        ));
        unicode.push_str(&format!(
            "native-fn greeting{index} 0 \"&'static str\" 1 tail str \"héllo β \\\"quoted\\\" \\\\ path\\n\"\n"
        ));
        let body = format!("fn raw{index}() -> &'static str {{ \"héllo β\" }}\n");
        raw.push_str(&format!("rust {}\n{body}\n", body.len()));
    }
    let mut float_raw = CHECKED.to_owned();
    let mut float_special = CHECKED.to_owned();
    let mut float_decimal = CHECKED.to_owned();
    for index in 0..64 {
        let ((gain_bits, offset_bits), (gain_decimal, offset_decimal)) = [
            (("0x3fc00000f", "0x80000000f"), ("1.5f", "-0f")),
            (("0x3e800000f", "0x3dcccccdf"), ("0.25f", "0.1f")),
            (("0x44800000f", "0x322bcc77f"), ("1024f", "1e-8f")),
            (("0x116c2f", "0xc0200000f"), ("1e-40f", "-2.5f")),
        ][index % 4];
        float_raw.push_str(&format!(
            "fn float{index} 1 x float float ret add mul x {gain_bits} {offset_bits}\n"
        ));
        float_decimal.push_str(&format!(
            "fn float{index} 1 x float float ret add mul x {gain_decimal} {offset_decimal}\n"
        ));
        float_special.push_str(&format!(
            "fn special{index} 0 list float ret list float 8 0x0f 0x80000000f 0x7f800000f 0xff800000f 0x7fc00000f 0x7fa00001f 0xffc12345f 0x1f\n"
        ));
    }
    let mut cases = vec![
        Case {
            name: "checked_small",
            text: CHECKED.to_owned(),
            native_count: 0,
        },
        Case {
            name: "checked_updates_64",
            text: checked,
            native_count: 0,
        },
        Case {
            name: "native_unquoted_64",
            text: unquoted,
            native_count: 64,
        },
        Case {
            name: "native_quoted_64",
            text: quoted,
            native_count: 64,
        },
        Case {
            name: "native_unicode_64",
            text: unicode,
            native_count: 64,
        },
        Case {
            name: "native_raw_64",
            text: raw,
            native_count: 64,
        },
        Case {
            name: "checked_floats_raw_64",
            text: float_raw,
            native_count: 0,
        },
        Case {
            name: "checked_floats_special_64",
            text: float_special,
            native_count: 0,
        },
    ];
    // The default corpus uses retained raw-bit syntax so an identical probe
    // can be compiled against older source for matched parser/printer costs.
    if decimal_literals {
        cases.push(Case {
            name: "checked_floats_decimal_64",
            text: float_decimal,
            native_count: 0,
        });
    }
    cases
}

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn statistics(samples: Vec<u64>) -> Json {
    let mut ordered = samples.clone();
    ordered.sort_unstable();
    let at = |tenths: usize| ordered[(ordered.len() * tenths).div_ceil(1000).max(1) - 1];
    json!({
        "observation_unit": "ns per individual source API call; disposal and validation excluded",
        "sample_count": samples.len(), "samples_ns": samples,
        "percentiles_p0_through_p100_ns": (0..=100).map(|p| at(p * 10)).collect::<Vec<_>>(),
        "p99_9_ns": at(999), "quantile_method": "nearest-rank; p0 is minimum",
        "expected_observations_above_p99_9": ordered.len() as f64 / 1000.0,
    })
}

fn measure<T>(
    samples: usize,
    mut operation: impl FnMut() -> T,
    mut validate: impl FnMut(&T),
) -> Json {
    for _ in 0..WARMUP {
        validate(&black_box(operation()));
    }
    let mut observations = Vec::with_capacity(samples);
    for _ in 0..samples {
        let start = Instant::now();
        let value = black_box(operation());
        let elapsed = start.elapsed().as_nanos();
        observations.push(u64::try_from(elapsed).expect("bounded measurement duration"));
        validate(&value);
        // Drop after the timer, exactly as for the checked invocation probe.
        drop(value);
    }
    statistics(observations)
}

fn verify(case: &Case) -> SourceDocument {
    let document = SourceDocument::parse(&case.text).unwrap();
    assert_eq!(document.native_spans.len(), case.native_count);
    let mut previous = 0;
    for span in &document.native_spans {
        assert!(span.start >= previous && span.end <= case.text.len() && span.start < span.end);
        assert!(case.text.is_char_boundary(span.start) && case.text.is_char_boundary(span.end));
        previous = span.end;
    }
    let module = document.compile().unwrap();
    assert_eq!(
        module
            .run(Invocation {
                export: "run",
                inputs: &[],
                arguments: &[Value::I64(1)],
                limits: Limits::default()
            })
            .unwrap(),
        Value::I64(18)
    );
    if case.name.starts_with("checked_") && case.name.ends_with("_64") {
        assert_eq!(document.source.functions.len(), 65);
    }
    let printed = module.unified_text().unwrap();
    assert_eq!(
        SourceDocument::parse(&printed)
            .unwrap()
            .compile()
            .unwrap()
            .encode(),
        module.encode()
    );
    document
}

fn run_case(case: &Case, samples: usize) -> Json {
    let document = verify(case);
    let module = document.compile().unwrap();
    let artifact = module.encode();
    let canonical = module.unified_text().unwrap();
    let parse = measure(
        samples,
        || SourceDocument::parse(black_box(&case.text)).unwrap(),
        |value| {
            assert_eq!(value.source, document.source);
            assert_eq!(value.checked_text, document.checked_text);
            assert_eq!(value.native_digest, document.native_digest);
            assert_eq!(value.native_spans, document.native_spans);
        },
    );
    let compile = measure(
        samples,
        || black_box(&document).compile().unwrap(),
        |value| {
            assert_eq!(value.encode(), artifact);
        },
    );
    let print = measure(
        samples,
        || black_box(&module).unified_text().unwrap(),
        |value| {
            assert_eq!(value, &canonical);
        },
    );
    json!({"name":case.name, "source_bytes":case.text.len(), "source_sha256":sha(case.text.as_bytes()),
        "artifact_sha256":sha(&artifact), "native_items":case.native_count,
        "canonical_bytes":canonical.len(), "canonical_sha256":sha(canonical.as_bytes()),
        "checked_functions":document.source.functions.len(),
        "phases":{"parse_document":parse,"compile_checked_projection":compile,"print_checked_projection":print},
        "validation":"exact source AST, projection text, native spans/digest, artifact and printed round trip; independent run(1)=18"})
}

fn main() {
    let samples = std::env::var("FLUXR_SOURCE_SAMPLES").map_or(10_000, |value| {
        value
            .parse::<usize>()
            .expect("FLUXR_SOURCE_SAMPLES is an integer")
    });
    assert!(
        (1024..=16_384).contains(&samples),
        "source samples must be 1024..=16384"
    );
    let decimal_literals = match std::env::var("FLUXR_SOURCE_DECIMAL").as_deref() {
        Err(std::env::VarError::NotPresent) | Ok("0") => false,
        Ok("1") => true,
        _ => panic!("FLUXR_SOURCE_DECIMAL must be 0 or 1"),
    };
    assert!(
        std::env::args().len() == 1,
        "this probe accepts configuration through FLUXR_SOURCE_SAMPLES and FLUXR_SOURCE_DECIMAL only"
    );
    let mut timer = measure(4096, || (), |_| {});
    timer["observation_unit"] = json!("ns per empty timer pair");
    let cases: Vec<_> = cases(decimal_literals)
        .iter()
        .map(|case| run_case(case, samples))
        .collect();
    println!(
        "{}",
        json!({
            "schema":"fluxr-source-performance-v1", "probe_sha256":sha(include_bytes!("bench_source.rs")),
            "settings":{"samples":samples,"warmup":WARMUP,"case_order":"fixed; phases parse, compile, print", "decimal_literals":decimal_literals},
            "debug_assertions":cfg!(debug_assertions), "typed_jit_feature":cfg!(feature="typed-jit"),
            "os":std::env::consts::OS,"arch":std::env::consts::ARCH,
            "target_env":if cfg!(target_env="gnu") {"gnu"} else if cfg!(target_env="msvc") {"msvc"} else if cfg!(target_env="musl") {"musl"} else {"other"},
            "timer_calibration":timer,"cases":cases,
            "scope":"source parse and checked projection APIs in a warm process; native Rust compilation/execution and compiler startup excluded; printing verifies its canonical round trip internally",
            "qualification":"finite empirical distributions; samples are correlated; independent process/build controls required; no universal performance claim"
        })
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_inputs_preserve_checked_result_native_spans_and_artifacts() {
        for case in cases(true) {
            verify(&case);
        }
    }

    #[test]
    fn float_corpora_preserve_the_same_checked_artifact() {
        let cases = cases(true);
        let raw = cases
            .iter()
            .find(|case| case.name == "checked_floats_raw_64")
            .unwrap();
        let decimal = cases
            .iter()
            .find(|case| case.name == "checked_floats_decimal_64")
            .unwrap();
        let raw = SourceDocument::parse(&raw.text).unwrap();
        let decimal = SourceDocument::parse(&decimal.text).unwrap();
        assert_eq!(raw.source, decimal.source);
        assert_eq!(
            raw.compile().unwrap().encode(),
            decimal.compile().unwrap().encode()
        );
    }

    #[test]
    fn every_order_statistic_endpoint_and_tail_uses_nearest_rank() {
        let result = statistics((1..=10_000).rev().collect());
        assert_eq!(result["percentiles_p0_through_p100_ns"][0], 1);
        assert_eq!(result["percentiles_p0_through_p100_ns"][50], 5000);
        assert_eq!(result["percentiles_p0_through_p100_ns"][100], 10_000);
        assert_eq!(result["p99_9_ns"], 9990);
        assert_eq!(result["samples_ns"][0], 10_000);
    }
}
