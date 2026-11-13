//! Reproducible AH1 authoring episode and matched preparation/runtime timings.
//! The emitted JSON contains complete source, edit and context strings for real
//! tokenizer accounting. This is a reviewed deterministic fixture, not an LLM trial.
use fluxr::structured::authoring::{AuthoringDraft, BodyTarget, Cancellation, QueryLimits};
use fluxr::structured::{Invocation, Limits, Module, Value};
use sha2::{Digest, Sha256};
use std::{hint::black_box, io::Read, time::Instant};

fn compiler_binary_hash() -> Result<[u8; 32], Box<dyn std::error::Error>> {
    let mut file = std::fs::File::open(std::env::current_exe()?)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(hash.finalize().into())
}

fn measure<T>(mut operation: impl FnMut() -> T) -> serde_json::Value {
    let mut samples = Vec::new();
    for _ in 0..31 {
        let started = Instant::now();
        for _ in 0..20 {
            black_box(operation());
        }
        samples.push(started.elapsed().as_nanos() as u64 / 20);
    }
    let mut ordered = samples.clone();
    ordered.sort_unstable();
    serde_json::json!({
        "unit":"nanoseconds_per_call", "rounds":31, "calls_per_round":20,
        "samples":samples,"min":ordered[0],"median":ordered[15],"p95":ordered[29],"max":ordered[30]
    })
}

fn rust_calc(x: i64, y: i64) -> Option<i64> {
    x.checked_mul(y)?.checked_add(17)?.checked_rem(997)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let compiler = compiler_binary_hash()?;
    let compiler_hex: String = compiler.iter().map(|b| format!("{b:02x}")).collect();
    let target = BodyTarget::function("calc");
    let cancel = Cancellation::default();
    let fill = include_str!("fixtures/authoring_fill.fa1");
    let mut cases = Vec::new();
    for (label, helpers) in [("small", 0), ("medium", 31), ("large", 127)] {
        let mut before = include_str!("fixtures/authoring_before.fa1").to_string();
        for index in 0..helpers {
            before.push_str(&format!("fn helper{index} 1 value int int ret value\n"));
        }
        let after = before.replace("ret add x y", "ret rem add mul x y 17 997");
        let open = || {
            AuthoringDraft::open_fa1(&before, 1, compiler, std::slice::from_ref(&target), &cancel)
                .unwrap()
        };
        let draft = open();
        assert!(draft.admit(draft.binding(), &cancel).is_err());
        let context = draft.query(draft.binding(), &target, QueryLimits::default(), &cancel)?;
        let filled = draft.fill_body(&context, fill, &cancel)?;
        let module = filled.admit(filled.binding(), &cancel)?;
        let direct = Module::compile_agent(&after)?;
        assert_eq!(module.encode(), direct.encode());
        let arguments = [Value::I64(34), Value::I64(53)];
        let run = || {
            module
                .run(Invocation {
                    export: "calc",
                    inputs: &[],
                    arguments: black_box(&arguments),
                    limits: Limits::default(),
                })
                .unwrap()
        };
        let observed = run();
        let expected = rust_calc(34, 53).ok_or("unexpected Rust arithmetic failure")?;
        assert_eq!(observed, Value::I64(expected));
        let timing = serde_json::json!({
            "open_checked_base_and_erase":measure(open),
            "query_context":measure(|| draft.query(draft.binding(), &target, QueryLimits::default(), &cancel).unwrap()),
            "fill_one_typed_body":measure(|| draft.fill_body(&context, fill, &cancel).unwrap()),
            "admit_complete_module":measure(|| filled.admit(filled.binding(), &cancel).unwrap()),
            "direct_full_compile_after":measure(|| Module::compile_agent(black_box(&after)).unwrap()),
            "fluxr_host_run":measure(run),
            "equivalent_checked_rust":measure(|| rust_calc(black_box(34),black_box(53)).unwrap()),
            "complete_authoring_episode":measure(|| {
                let draft = open();
                let c = draft.query(draft.binding(),&target,QueryLimits::default(),&cancel).unwrap();
                let filled = draft.fill_body(&c,fill,&cancel).unwrap();
                filled.admit(filled.binding(),&cancel).unwrap()
            })
        });
        cases.push(serde_json::json!({
            "label":label,"declarations":helpers+1,"original_source":before,"completed_source":after,
            "fill_body":fill,"context_packet":context.packet(),"context_bytes":context.packet().len(),
            "context_visible_items":context.visible_items(),"context_total_items":context.total_items(),
            "context_truncated":context.truncated(),"exact_fsm_bytes_equal":true,
            "binding":draft.binding(),"completed_binding":filled.binding(),
            "actual_fluxr_output":observed,"equivalent_rust_output":expected,
            "canonical_artifact_bytes":module.encode().len(),"timing":timing
        }));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema":"fluxr-ah1-authoring-observations-v1", "profile":"release",
            "compiler_binding":{"kind":"executing_example_binary_sha256","sha256":compiler_hex,"authentication":false},
            "measurement_scope":"Same process; serialized wall-clock rounds. Payload byte sizes are not heap/RSS measurements. Full compiler remains admission oracle. No live model episode or static effect/resource proof.",
            "cases":cases
        }))?
    );
    Ok(())
}
