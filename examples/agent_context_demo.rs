//! Reviewed, deterministic AC1/AH1 comparison. Emits exact fixture, query and
//! diagnostic strings for complete-string token counting, not a live LLM trial.
use fluxr::structured::authoring::{
    AuthoringDraft, BodyContext, BodyTarget, COMPACT_CONTEXT_PROFILE, Cancellation, QueryLimits,
    SignatureSelection,
};
use fluxr::structured::{Invocation, Limits, Module, Value};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{hint::black_box, io::Read, sync::Arc, time::Instant};

mod scalar_reference {
    include!("../benchmarks/context-compact-v1/scalar_reference.rs");
    pub fn expected(x: i64, y: i64) -> Option<i64> {
        calc(x, y)
    }
}
mod trap_reference {
    include!("../benchmarks/context-compact-v1/trap_order_reference.rs");
    pub fn expected(x: i64, y: i64) -> Option<i64> {
        calc(x, y)
    }
}
mod helper_reference {
    include!("../benchmarks/context-compact-v1/helper_reference.rs");
    pub fn expected(x: i64) -> Option<i64> {
        calc(x)
    }
}
mod cow_reference {
    include!("../benchmarks/context-compact-v1/cow_reference.rs");
    pub fn expected(xs: Arc<Vec<i64>>) -> Option<Arc<Vec<i64>>> {
        trim(xs)
    }
}

struct Fixture {
    id: &'static str,
    source: &'static str,
    completed: &'static str,
    fill: &'static str,
    rust: &'static str,
    export: &'static str,
}

fn fixtures() -> [Fixture; 4] {
    [
        Fixture {
            id: "scalar",
            source: include_str!("../benchmarks/context-compact-v1/scalar_before.fa1"),
            completed: include_str!("../benchmarks/context-compact-v1/scalar_after.fa1"),
            fill: include_str!("../benchmarks/context-compact-v1/scalar_fill.fa1"),
            rust: include_str!("../benchmarks/context-compact-v1/scalar_reference.rs"),
            export: "calc",
        },
        Fixture {
            id: "trap_order",
            source: include_str!("../benchmarks/context-compact-v1/trap_order_before.fa1"),
            completed: include_str!("../benchmarks/context-compact-v1/trap_order_after.fa1"),
            fill: include_str!("../benchmarks/context-compact-v1/trap_order_fill.fa1"),
            rust: include_str!("../benchmarks/context-compact-v1/trap_order_reference.rs"),
            export: "calc",
        },
        Fixture {
            id: "helper",
            source: include_str!("../benchmarks/context-compact-v1/helper_before.fa1"),
            completed: include_str!("../benchmarks/context-compact-v1/helper_after.fa1"),
            fill: include_str!("../benchmarks/context-compact-v1/helper_fill.fa1"),
            rust: include_str!("../benchmarks/context-compact-v1/helper_reference.rs"),
            export: "calc",
        },
        Fixture {
            id: "cow",
            source: include_str!("../benchmarks/context-compact-v1/cow_before.fa1"),
            completed: include_str!("../benchmarks/context-compact-v1/cow_after.fa1"),
            fill: include_str!("../benchmarks/context-compact-v1/cow_fill.fa1"),
            rust: include_str!("../benchmarks/context-compact-v1/cow_reference.rs"),
            export: "trim",
        },
    ]
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

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
    let mut batches = Vec::new();
    for _ in 0..31 {
        let started = Instant::now();
        for _ in 0..20 {
            black_box(operation());
        }
        batches.push(started.elapsed().as_nanos() as u64);
    }
    let mut ordered = batches.clone();
    ordered.sort_unstable();
    json!({
        "unit":"nanoseconds_per_batch", "rounds":31, "calls_per_round":20,
        "raw_batch_nanoseconds":batches,
        "median_batch_average_nanoseconds_per_call":ordered[15] as f64 / 20.0,
        "p95_batch_average_nanoseconds_per_call":ordered[29] as f64 / 20.0,
        "scope":"Sequential same-process query wall time including returned ticket destruction; percentiles are batch averages, not individual-call tails or a hard latency bound."
    })
}

fn counters(c: &BodyContext) -> serde_json::Value {
    json!({
        "profile":c.packet_profile(), "visible_items":c.visible_items(),
        "selected_items":c.selected_items(), "total_items":c.total_items(),
        "unselected_items":c.unselected_items(), "truncated":c.truncated(),
        "packet_bytes":c.packet().len()
    })
}

fn observations(fixture: &Fixture, module: &Module) -> Vec<serde_json::Value> {
    let mut rows = Vec::new();
    if fixture.id == "cow" {
        for values in [vec![7, 8], vec![3], vec![]] {
            let rust_original = Arc::new(values.clone());
            let expected = cow_reference::expected(Arc::clone(&rust_original));
            let original = Arc::new(
                values
                    .iter()
                    .map(|value| Value::I64(*value))
                    .collect::<Vec<_>>(),
            );
            let arguments = [Value::List(Arc::clone(&original))];
            let actual = module.run(Invocation {
                export: fixture.export,
                inputs: &[],
                arguments: &arguments,
                limits: Limits::default(),
            });
            let expected_value = expected.as_ref().map(|values| {
                Value::List(Arc::new(
                    values.iter().map(|value| Value::I64(*value)).collect(),
                ))
            });
            assert_eq!(actual.as_ref().ok(), expected_value.as_ref());
            assert_eq!(*rust_original, values);
            assert_eq!(
                *original,
                values
                    .iter()
                    .map(|value| Value::I64(*value))
                    .collect::<Vec<_>>()
            );
            rows.push(json!({
                "arguments":values,"equivalent_rust_output":expected.as_ref().map(|x| x.as_ref()),
                "actual_fluxr_output":actual.as_ref().ok(),
                "fluxr_error":actual.as_ref().err().map(ToString::to_string),
                "original_shared_values_unchanged":true,
                "rust_original_strong_count_after_call":Arc::strong_count(&rust_original),
                "fluxr_original_strong_count_after_call":Arc::strong_count(&original)
            }));
        }
        return rows;
    }
    let arguments: Vec<Vec<i64>> = match fixture.id {
        "scalar" => [(34, 53), (0, 0), (-3, 7), (i64::MAX, 2), (1, i64::MAX)]
            .into_iter()
            .map(|(x, y)| vec![x, y])
            .collect(),
        "trap_order" => [(3, 2), (i64::MAX, 0), (1, 0), (i64::MIN, -1)]
            .into_iter()
            .map(|(x, y)| vec![x, y])
            .collect(),
        "helper" => [7, 0, -9, i64::MAX, i64::MAX / 2]
            .into_iter()
            .map(|x| vec![x])
            .collect(),
        _ => unreachable!(),
    };
    for values in arguments {
        let expected = match fixture.id {
            "scalar" => scalar_reference::expected(values[0], values[1]),
            "trap_order" => trap_reference::expected(values[0], values[1]),
            "helper" => helper_reference::expected(values[0]),
            _ => unreachable!(),
        };
        let arguments: Vec<_> = values.iter().map(|value| Value::I64(*value)).collect();
        let actual = module.run(Invocation {
            export: fixture.export,
            inputs: &[],
            arguments: &arguments,
            limits: Limits::default(),
        });
        assert_eq!(actual.as_ref().ok(), expected.map(Value::I64).as_ref());
        rows.push(json!({
            "arguments":values,"equivalent_rust_output":expected,
            "actual_fluxr_output":actual.as_ref().ok(),
            "fluxr_error":actual.as_ref().err().map(ToString::to_string)
        }));
    }
    rows
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let compiler = compiler_binary_hash()?;
    let cancel = Cancellation::default();
    let mut cases = Vec::new();
    for fixture in fixtures() {
        let target = BodyTarget::function(fixture.export);
        let started = Instant::now();
        let draft = AuthoringDraft::open_fa1(
            fixture.source,
            1,
            compiler,
            std::slice::from_ref(&target),
            &cancel,
        )?;
        let open_ns = started.elapsed().as_nanos() as u64;
        assert!(draft.admit(draft.binding(), &cancel).is_err());
        let requested = if fixture.id == "helper" {
            vec![BodyTarget::function("step")]
        } else {
            vec![]
        };
        let selection = SignatureSelection::Explicit {
            records: &[],
            enums: &[],
            callables: &requested,
        };
        let limits = QueryLimits::default();
        let legacy = draft.query(draft.binding(), &target, limits, &cancel)?;
        let compact = draft.query_compact(
            draft.binding(),
            &target,
            SignatureSelection::All,
            limits,
            &cancel,
        )?;
        let focused = draft.query_compact(draft.binding(), &target, selection, limits, &cancel)?;
        let failed_text = "fa1 body 1 ret true";
        let failed = draft
            .fill_body(&focused, failed_text, &cancel)
            .err()
            .ok_or("invalid fill unexpectedly admitted")?;
        assert_eq!(draft.binding(), focused.binding());
        assert_eq!(draft.unresolved_count(), 1);
        let direct = Module::compile_agent(fixture.completed)?;
        let direct_bytes = direct.encode();
        let direct_sha = hex(&Sha256::digest(&direct_bytes));
        let mut arm_sha = serde_json::Map::new();
        for (arm, ticket) in [
            ("legacy", &legacy),
            ("compact", &compact),
            ("focused", &focused),
        ] {
            let refused = draft
                .fill_body(ticket, failed_text, &cancel)
                .err()
                .ok_or("invalid fill unexpectedly admitted")?;
            assert_eq!(refused, failed);
            let filled = draft.fill_body(ticket, fixture.fill, &cancel)?;
            let bytes = filled.admit(filled.binding(), &cancel)?.encode();
            assert_eq!(bytes, direct_bytes);
            arm_sha.insert(arm.to_string(), json!(hex(&Sha256::digest(&bytes))));
        }
        let started = Instant::now();
        let filled = draft.fill_body(&focused, fixture.fill, &cancel)?;
        let fill_ns = started.elapsed().as_nanos() as u64;
        let started = Instant::now();
        let module = filled.admit(filled.binding(), &cancel)?;
        let admit_ns = started.elapsed().as_nanos() as u64;
        let record = |profile: &str,
                      selected: serde_json::Value|
         -> Result<String, serde_json::Error> {
            serde_json::to_string(&json!({
                "profile":profile,"binding":draft.binding(),"target":&target,"selection":selected,
                "limits":{"max_bytes":limits.max_bytes,"max_items":limits.max_items}
            }))
        };
        cases.push(json!({
            "id":fixture.id,"source":fixture.source,"completed_source":fixture.completed,
            "fill":fixture.fill,"rust_source":fixture.rust,
            "legacy_packet":legacy.packet(),"compact_packet":compact.packet(),"focused_packet":focused.packet(),
            "binding":draft.binding(),"completed_binding":filled.binding(),
            "recorded_host_calls":{
                "legacy":record(legacy.packet_profile(),json!({"kind":"all"}))?,
                "compact":record(compact.packet_profile(),json!({"kind":"all"}))?,
                "focused":record(focused.packet_profile(),json!({"kind":"explicit","records":[],"enums":[],"callables":&requested}))?
            },
            "host_record_scope":"Exact in-process API arguments serialized for accounting. This JSON is an observation record, not a released transport or observed model request.",
            "selected_callables":requested,
            "failed_fill":{"text":failed_text,"kind":format!("{:?}",failed.kind),"detail":failed.detail(),"display":failed.to_string(),"unchanged":true},
            "counters":{"legacy":counters(&legacy),"compact":counters(&compact),"focused":counters(&focused)},
            "exact_fsm_equal":true,"canonical_fsm_sha256":direct_sha,"direct_fsm_sha256":direct_sha,
            "arm_fsm_sha256":arm_sha,"canonical_artifact_bytes":direct_bytes.len(),
            "observations":observations(&fixture,&module),
            "single_observation_nanoseconds":{"open_checked_base":open_ns,"fill":fill_ns,"whole_module_admit":admit_ns},
            "query_timing":{
                "legacy":measure(||draft.query(draft.binding(),&target,limits,&cancel).unwrap()),
                "compact":measure(||draft.query_compact(draft.binding(),&target,SignatureSelection::All,limits,&cancel).unwrap()),
                "focused":measure(||draft.query_compact(draft.binding(),&target,selection,limits,&cancel).unwrap())
            }
        }));
    }
    let mut locality_cases = Vec::new();
    for helpers in [0, 31, 127] {
        let mut source =
            include_str!("../benchmarks/context-compact-v1/scalar_before.fa1").to_string();
        for index in 0..helpers {
            source.push_str(&format!("fn unrelated{index} 1 value int int ret value\n"));
        }
        let completed = source.replacen("ret add x y", "ret rem add mul x y 17 997", 1);
        let fill = include_str!("../benchmarks/context-compact-v1/scalar_fill.fa1");
        let target = BodyTarget::function("calc");
        let draft =
            AuthoringDraft::open_fa1(&source, 1, compiler, std::slice::from_ref(&target), &cancel)?;
        let limits = QueryLimits {
            max_bytes: 65536,
            max_items: 4096,
        };
        let selection = SignatureSelection::Explicit {
            records: &[],
            enums: &[],
            callables: &[],
        };
        let legacy = draft.query(draft.binding(), &target, limits, &cancel)?;
        let compact = draft.query_compact(
            draft.binding(),
            &target,
            SignatureSelection::All,
            limits,
            &cancel,
        )?;
        let focused = draft.query_compact(draft.binding(), &target, selection, limits, &cancel)?;
        let direct = Module::compile_agent(&completed)?;
        let direct_bytes = direct.encode();
        for ticket in [&legacy, &compact, &focused] {
            let filled = draft.fill_body(ticket, fill, &cancel)?;
            assert_eq!(
                filled.admit(filled.binding(), &cancel)?.encode(),
                direct_bytes
            );
        }
        let filled = draft.fill_body(&focused, fill, &cancel)?;
        let module = filled.admit(filled.binding(), &cancel)?;
        let output = module.run(Invocation {
            export: "calc",
            inputs: &[],
            arguments: &[Value::I64(34), Value::I64(53)],
            limits: Limits::default(),
        })?;
        let expected = scalar_reference::expected(34, 53).ok_or("unexpected Rust failure")?;
        assert_eq!(output, Value::I64(expected));
        locality_cases.push(json!({
            "id":format!("locality_{helpers}"),"helpers":helpers,"unrelated_helpers":helpers,
            "source":source,"completed_source":completed,"fill":fill,"binding":draft.binding(),
            "legacy_packet":legacy.packet(),"compact_packet":compact.packet(),"focused_packet":focused.packet(),
            "counters":{"legacy":counters(&legacy),"compact":counters(&compact),"focused":counters(&focused)},
            "exact_fsm_equal":true,"canonical_fsm_sha256":hex(&Sha256::digest(&direct_bytes)),
            "direct_fsm_sha256":hex(&Sha256::digest(&direct_bytes)),
            "actual_fluxr_output":output,"equivalent_rust_output":expected,
            "query_timing":{
                "legacy":measure(||draft.query(draft.binding(),&target,limits,&cancel).unwrap()),
                "compact":measure(||draft.query_compact(draft.binding(),&target,SignatureSelection::All,limits,&cancel).unwrap()),
                "focused":measure(||draft.query_compact(draft.binding(),&target,selection,limits,&cancel).unwrap())
            }
        }));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema":"fluxr-ac1-context-observations-v1","profile":COMPACT_CONTEXT_PROFILE,
            "compiler_binding":{"kind":"executing_example_binary_sha256","sha256":hex(&compiler),"authentication":false},
            "protocol_setup":include_str!("../benchmarks/context-compact-v1/protocol.txt"),
            "cases":cases,"locality_cases":locality_cases,"source_token_target":0.1,
            "measurement_scope":"Reviewed development fixtures and synthetic locality inventories known to authors. Exact body fills and unchanged runtime; no live model episode, held-out evaluation, model competence or effect/resource proof. Query timing is separate from preparation. Payload bytes are not measured heap/RSS.",
            "model_usage":{"status":"not_executed","model":null,"cached_input_tokens":null,"uncached_input_tokens":null,"output_tokens":null,"reasoning_tokens":null,"review_tokens":null,"handoff_tokens":null,"accepted_task_success":null}
        }))?
    );
    Ok(())
}
