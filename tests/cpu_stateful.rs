use fluxr::cpu_compute::stream::stateful::{STATEFUL_ABI, StatefulGraph, StatefulSpec};
use fluxr::cpu_compute::stream::{Argument, ChunkStatus, GRAPH_ABI, GraphSpec, Node, StreamLimits};
use fluxr::cpu_compute::{
    ABI, Cancellation, LibraryManifest, MemoryBudget, NUMERIC_PROFILE, ScalarLibrary, Tensor,
};
use fluxr::structured::Limits;
use sha2::{Digest, Sha256};
use std::sync::Arc;

#[path = "../examples/native/effects/stateful/reference.rs"]
mod reference;

fn library(source: &str, exports: &[(&str, usize)]) -> Arc<ScalarLibrary> {
    let manifest = LibraryManifest {
        abi: ABI.into(),
        numeric_profile: NUMERIC_PROFILE.into(),
        source_sha256: format!("{:x}", Sha256::digest(source.as_bytes())),
        package: "state.test".into(),
        version: 1,
        exports: exports.iter().map(|(s, n)| (s.to_string(), *n)).collect(),
        capabilities: vec!["compute.cpu".into()],
    };
    Arc::new(ScalarLibrary::load(&serde_json::to_vec(&manifest).unwrap(), source).unwrap())
}
fn effect() -> Arc<ScalarLibrary> {
    Arc::new(
        ScalarLibrary::load(
            include_bytes!("../examples/native/effects/stateful/manifest.json"),
            include_str!("../examples/native/effects/stateful/program.flux"),
        )
        .unwrap(),
    )
}
fn input(column: usize) -> Argument {
    Argument::Input { column }
}
fn previous(node: usize) -> Argument {
    Argument::Node { node }
}
fn node(export: &str, arguments: Vec<Argument>) -> Node {
    Node {
        library: 0,
        export: export.into(),
        arguments,
    }
}
fn spec() -> StatefulSpec {
    StatefulSpec {
        abi: STATEFUL_ABI.into(),
        external_columns: 4,
        transition: GraphSpec {
            abi: GRAPH_ABI.into(),
            input_columns: 6,
            nodes: vec![
                node("smooth", vec![input(0), input(1), input(4)]),
                node("residual", vec![input(0), previous(0)]),
                node("envelope", vec![previous(1), input(2), input(5)]),
                node("gain", vec![previous(0), input(3)]),
            ],
            outputs: vec![3, 2],
        },
        initial_state_bits: vec![0, 0],
        state_outputs: vec![0, 2],
    }
}
fn limits() -> StreamLimits {
    StreamLimits {
        invocation: Limits::default(),
        maximum_rows: 10000,
        maximum_chunk_rows: 1024,
        maximum_attempts: 10000,
    }
}
fn bits(values: &[f32]) -> Vec<u32> {
    values.iter().map(|v| v.to_bits()).collect()
}
fn rows(n: usize) -> Vec<f32> {
    let mut result = Vec::new();
    let mut random = 17u32;
    for i in 0..n {
        random = random.wrapping_mul(1664525).wrapping_add(1013904223);
        result.extend_from_slice(&[
            ((random >> 8) as f32 / 8388608.) - 1.,
            if i % 7 == 0 { 0.5 } else { 0.125 },
            0.25,
            (i % 9) as f32 * 0.125,
        ]);
    }
    result
}
fn oracle(data: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let mut state = vec![0., 0.];
    let mut out = Vec::new();
    for row in data.as_chunks::<4>().0 {
        let low = reference::smooth(row[0], row[1], state[0]);
        let high = reference::residual(row[0], low);
        let env = reference::envelope(high, row[2], state[1]);
        out.extend_from_slice(&[reference::gain(low, row[3]), env]);
        state[0] = low;
        state[1] = env;
    }
    (out, state)
}

#[test]
fn genuine_fluxr_state_and_sample_automation_are_chunk_invariant() {
    let graph = StatefulGraph::prepare(spec(), &[effect()]).unwrap();
    let data = rows(257);
    let (expected, state) = oracle(&data);
    for chunk_rows in [1, 2, 3, 16, 64, 257] {
        let memory = MemoryBudget::new(1 << 20);
        let mut stream = graph
            .stream(limits(), memory.clone(), Cancellation::default())
            .unwrap();
        let mut observed = Vec::new();
        for chunk in data.chunks(chunk_rows * 4) {
            let input = Tensor::new(&[chunk.len() / 4, 4], chunk, &memory).unwrap();
            let output = stream
                .execute(
                    graph.identity(),
                    stream.state_pin(),
                    stream.next_sequence(),
                    &input,
                )
                .unwrap();
            assert_eq!(output.receipt.execution.status, ChunkStatus::Completed);
            assert!(output.receipt.state_committed);
            assert_eq!(
                output.receipt.state_after.revision,
                output.receipt.state_before.revision + 1
            );
            observed.extend_from_slice(output.output.values());
        }
        assert_eq!(bits(&observed), bits(&expected));
        assert_eq!(bits(stream.state()), bits(&state));
        assert_eq!(stream.statistics().attempted_calls, 257 * 4);
        assert_eq!(stream.statistics().published_rows, 257);
        drop(stream);
        assert_eq!(memory.live_bytes(), 0);
    }
}

#[test]
fn malformed_state_graphs_and_nonfinite_initial_state_refuse() {
    for changed in 0..7 {
        let mut s = spec();
        match changed {
            0 => s.abi = "fluxr-stateful-scalar-graph-v2".into(),
            1 => s.initial_state_bits.clear(),
            2 => s.initial_state_bits = vec![0; 65],
            3 => s.initial_state_bits[0] = f32::NAN.to_bits(),
            4 => s.state_outputs.pop().map(|_| ()).unwrap(),
            5 => s.state_outputs[1] = 99,
            _ => s.external_columns = 5,
        }
        assert!(
            StatefulGraph::prepare(s, &[effect()]).is_err(),
            "case {changed}"
        );
    }
    assert!(StatefulGraph::load(&vec![b' '; 72 * 1024 + 1], &[effect()]).is_err());
}

#[test]
fn graphs_and_initial_bits_are_immutable_identity_inputs() {
    let a = StatefulGraph::prepare(spec(), &[effect()]).unwrap();
    let copy = StatefulGraph::load(&a.encode(), &[effect()]).unwrap();
    assert_eq!(a.identity(), copy.identity());
    let mut s = spec();
    s.initial_state_bits[0] = (-0f32).to_bits();
    let b = StatefulGraph::prepare(s, &[effect()]).unwrap();
    assert_ne!(a.identity(), b.identity());
    let mut s = spec();
    s.state_outputs.swap(0, 1);
    assert_ne!(
        a.identity(),
        StatefulGraph::prepare(s, &[effect()]).unwrap().identity()
    );
}

#[test]
fn stale_state_and_sequence_reject_before_any_work() {
    let graph = StatefulGraph::prepare(spec(), &[effect()]).unwrap();
    let memory = MemoryBudget::new(4096);
    let mut stream = graph
        .stream(limits(), memory.clone(), Cancellation::default())
        .unwrap();
    let input = Tensor::new(&[1, 4], &[1., 0.5, 0.5, 1.], &memory).unwrap();
    let old = stream.state_pin();
    stream.execute(graph.identity(), old, 0, &input).unwrap();
    assert!(stream.execute(graph.identity(), old, 1, &input).is_err());
    let mut wrong = stream.state_pin();
    wrong.sha256[0] ^= 1;
    assert!(stream.execute(graph.identity(), wrong, 2, &input).is_err());
    assert!(
        stream
            .execute(graph.identity(), stream.state_pin(), 2, &input)
            .is_err()
    );
    assert!(
        stream
            .execute("wrong", stream.state_pin(), 4, &input)
            .is_err()
    );
    assert_eq!(stream.statistics().attempted_calls, 4);
    assert_eq!(stream.statistics().rejected_chunks, 4);
    assert_eq!(stream.state_pin().revision, 1);
    assert!(
        stream.attempts()[1..]
            .iter()
            .all(|r| r.state_before == r.state_after && !r.state_committed)
    );
    stream
        .execute(graph.identity(), stream.state_pin(), 5, &input)
        .unwrap();
    assert_eq!(stream.statistics().published_rows, 2);
}

#[test]
fn simultaneous_updates_see_only_previous_state_even_when_aliased() {
    let library = library("v2 (pub id (x:f32) f32 = x)", &[("id", 1)]);
    let graph = StatefulGraph::prepare(
        StatefulSpec {
            abi: STATEFUL_ABI.into(),
            external_columns: 0,
            transition: GraphSpec {
                abi: GRAPH_ABI.into(),
                input_columns: 2,
                nodes: vec![node("id", vec![input(1)]), node("id", vec![input(0)])],
                outputs: vec![0, 1],
            },
            initial_state_bits: vec![1f32.to_bits(), 2f32.to_bits()],
            state_outputs: vec![0, 1],
        },
        &[library],
    )
    .unwrap();
    let memory = MemoryBudget::new(4096);
    let mut stream = graph
        .stream(limits(), memory.clone(), Cancellation::default())
        .unwrap();
    let input = Tensor::new(&[3, 0], &[], &memory).unwrap();
    let output = stream
        .execute(graph.identity(), stream.state_pin(), 0, &input)
        .unwrap();
    assert_eq!(output.output.values(), [2., 1., 1., 2., 2., 1.]);
    assert_eq!(stream.state(), [2., 1.]);
}

#[test]
fn late_failure_rolls_back_state_and_output_without_refunding_work() {
    let library = library(
        "v2 (pub add (x s:f32) f32 = (+ x s)) (pub div (x:f32) f32 = (/ 1f x))",
        &[("add", 2), ("div", 1)],
    );
    let graph = StatefulGraph::prepare(
        StatefulSpec {
            abi: STATEFUL_ABI.into(),
            external_columns: 1,
            transition: GraphSpec {
                abi: GRAPH_ABI.into(),
                input_columns: 2,
                nodes: vec![
                    node("add", vec![input(0), input(1)]),
                    node("div", vec![input(0)]),
                ],
                outputs: vec![0],
            },
            initial_state_bits: vec![7f32.to_bits()],
            state_outputs: vec![0],
        },
        &[library],
    )
    .unwrap();
    let memory = MemoryBudget::new(4096);
    let mut stream = graph
        .stream(limits(), memory.clone(), Cancellation::default())
        .unwrap();
    let bad = Tensor::new(&[3, 1], &[1., 0., 3.], &memory).unwrap();
    let original = stream.state_pin();
    let live = memory.live_bytes();
    assert!(stream.execute(graph.identity(), original, 0, &bad).is_err());
    assert_eq!(stream.state_pin(), original);
    assert_eq!(stream.state(), [7.]);
    assert_eq!(memory.live_bytes(), live);
    assert_eq!(stream.statistics().admitted_rows, 3);
    assert_eq!(stream.statistics().completed_rows, 1);
    assert_eq!(stream.statistics().attempted_calls, 4);
    assert_eq!(stream.statistics().published_rows, 0);
    assert_eq!(stream.attempts()[0].execution.first_failed_node, Some(1));
    let good = Tensor::new(&[1, 1], &[2.], &memory).unwrap();
    let output = stream
        .execute(graph.identity(), original, 1, &good)
        .unwrap();
    assert_eq!(output.output.values(), [9.]);
    assert_eq!(stream.state(), [9.]);
}

#[test]
fn unused_nodes_still_trap_and_do_not_commit_state() {
    let library = library(
        "v2 (pub id (x:f32) f32 = x) (pub trap () f32 = (/ 1f 0f))",
        &[("id", 1), ("trap", 0)],
    );
    let graph = StatefulGraph::prepare(
        StatefulSpec {
            abi: STATEFUL_ABI.into(),
            external_columns: 0,
            transition: GraphSpec {
                abi: GRAPH_ABI.into(),
                input_columns: 1,
                nodes: vec![node("id", vec![input(0)]), node("trap", vec![])],
                outputs: vec![0],
            },
            initial_state_bits: vec![0],
            state_outputs: vec![0],
        },
        &[library],
    )
    .unwrap();
    let memory = MemoryBudget::new(4096);
    let mut stream = graph
        .stream(limits(), memory.clone(), Cancellation::default())
        .unwrap();
    let input = Tensor::new(&[1, 0], &[], &memory).unwrap();
    let before = stream.state_pin();
    assert!(stream.execute(graph.identity(), before, 0, &input).is_err());
    assert_eq!(stream.state_pin(), before);
    assert_eq!(stream.attempts()[0].execution.first_failed_node, Some(1));
}

#[test]
fn state_and_output_payload_ownership_enforce_backpressure() {
    let graph = StatefulGraph::prepare(spec(), &[effect()]).unwrap();
    // Four nodes, 32 args, two committed + two tentative cells: 160 bytes.
    let memory = MemoryBudget::new(168);
    let inputs = MemoryBudget::new(1024);
    let input = Tensor::new(&[1, 4], &[1., 0.5, 0.5, 1.], &inputs).unwrap();
    let mut stream = graph
        .stream(limits(), memory.clone(), Cancellation::default())
        .unwrap();
    let first = stream
        .execute(graph.identity(), stream.state_pin(), 0, &input)
        .unwrap();
    let retained = first.output.clone();
    drop(first);
    let pin = stream.state_pin();
    assert!(stream.execute(graph.identity(), pin, 1, &input).is_err());
    assert_eq!(stream.state_pin(), pin);
    assert_eq!(stream.statistics().admitted_rows, 1);
    drop(retained);
    stream.execute(graph.identity(), pin, 2, &input).unwrap();
    assert_eq!(memory.live_bytes(), 160);
    drop(stream);
    assert_eq!(memory.live_bytes(), 0);
}

#[test]
fn cancellation_and_finite_envelopes_never_publish_success() {
    let graph = StatefulGraph::prepare(spec(), &[effect()]).unwrap();
    let memory = MemoryBudget::new(4096);
    let cancellation = Cancellation::default();
    let mut l = limits();
    l.maximum_rows = 1;
    l.maximum_attempts = 2;
    let mut stream = graph
        .stream(l, memory.clone(), cancellation.clone())
        .unwrap();
    let input = Tensor::new(&[2, 4], &rows(2), &memory).unwrap();
    let pin = stream.state_pin();
    assert!(stream.execute(graph.identity(), pin, 0, &input).is_err());
    cancellation.cancel();
    let input = Tensor::new(&[1, 4], &rows(1), &memory).unwrap();
    assert!(stream.execute(graph.identity(), pin, 1, &input).is_err());
    assert!(
        stream
            .execute(graph.identity(), pin, 2, &input)
            .unwrap_err()
            .to_string()
            .contains("ledger")
    );
    assert_eq!(stream.statistics().attempted_calls, 0);
    assert_eq!(stream.state_pin(), pin);
}

#[test]
fn zero_row_commit_advances_revision_preserving_negative_zero_bits() {
    let mut s = spec();
    s.initial_state_bits = [(-0f32).to_bits(), 1f32.to_bits()].to_vec();
    let graph = StatefulGraph::prepare(s, &[effect()]).unwrap();
    let memory = MemoryBudget::new(4096);
    let mut stream = graph
        .stream(limits(), memory.clone(), Cancellation::default())
        .unwrap();
    let before = stream.state_pin();
    let empty = Tensor::new(&[0, 4], &[], &memory).unwrap();
    let result = stream.execute(graph.identity(), before, 0, &empty).unwrap();
    assert_eq!(bits(stream.state()), [(-0f32).to_bits(), 1f32.to_bits()]);
    assert_eq!(stream.state_pin().sha256, before.sha256);
    assert_eq!(stream.state_pin().revision, 1);
    assert_eq!(result.output.shape(), [0, 2]);
    assert_eq!(stream.statistics().attempted_calls, 0);
}

#[test]
fn concurrent_streams_keep_independent_state_and_release_budget() {
    let graph = StatefulGraph::prepare(spec(), &[effect()]).unwrap();
    let memory = MemoryBudget::new(1 << 20);
    std::thread::scope(|scope| {
        let handles: Vec<_> = (1..5)
            .map(|n| {
                let graph = &graph;
                let memory = memory.clone();
                scope.spawn(move || {
                    let mut stream = graph
                        .stream(limits(), memory.clone(), Cancellation::default())
                        .unwrap();
                    let data = rows(n * 17);
                    let expected = oracle(&data);
                    let input = Tensor::new(&[n * 17, 4], &data, &memory).unwrap();
                    let result = stream
                        .execute(graph.identity(), stream.state_pin(), 0, &input)
                        .unwrap();
                    assert_eq!(bits(result.output.values()), bits(&expected.0));
                    assert_eq!(bits(stream.state()), bits(&expected.1));
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
    });
    assert_eq!(memory.live_bytes(), 0);
}

#[test]
fn invocation_limits_fail_atomically_and_contract_pins_include_policy() {
    let graph = StatefulGraph::prepare(spec(), &[effect()]).unwrap();
    let memory = MemoryBudget::new(4096);
    let mut l = limits();
    l.invocation.instructions = 1;
    let mut a = graph
        .stream(l, memory.clone(), Cancellation::default())
        .unwrap();
    let b = graph
        .stream(limits(), memory.clone(), Cancellation::default())
        .unwrap();
    assert_ne!(a.contract_identity(), b.contract_identity());
    let before = a.state_pin();
    let input = Tensor::new(&[1, 4], &rows(1), &memory).unwrap();
    assert!(a.execute(graph.identity(), before, 0, &input).is_err());
    assert_eq!(a.state_pin(), before);
    assert_eq!(a.attempts()[0].execution.first_failed_node, Some(0));
    assert_eq!(a.statistics().attempted_calls, 1);
    assert_eq!(a.statistics().completed_calls, 0);
}
