use fluxr::cpu_compute::stream::{
    Argument, ChunkStatus, GRAPH_ABI, GraphSpec, Node, ScalarGraph, StreamLimits, tensor_digest,
};
use fluxr::cpu_compute::{
    ABI, Cancellation, LibraryManifest, MemoryBudget, NUMERIC_PROFILE, ScalarLibrary, Tensor,
};
use fluxr::structured::Limits;
use sha2::{Digest, Sha256};
use std::sync::Arc;

#[path = "../examples/native/effects/softclip/reference.rs"]
mod effect_reference;

fn library(source: &str, exports: &[(&str, usize)]) -> Arc<ScalarLibrary> {
    let manifest = LibraryManifest {
        abi: ABI.into(),
        numeric_profile: NUMERIC_PROFILE.into(),
        source_sha256: format!("{:x}", Sha256::digest(source.as_bytes())),
        package: "stream.test".into(),
        version: 1,
        exports: exports
            .iter()
            .map(|(name, arity)| (name.to_string(), *arity))
            .collect(),
        capabilities: vec!["compute.cpu".into()],
    };
    Arc::new(ScalarLibrary::load(&serde_json::to_vec(&manifest).unwrap(), source).unwrap())
}

fn input(column: usize) -> Argument {
    Argument::Input { column }
}
fn constant(value: f32) -> Argument {
    Argument::Constant {
        bits: value.to_bits(),
    }
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
fn affine_library() -> Arc<ScalarLibrary> {
    library(
        "v2 (pub affine (x w b:f32) f32 = (+ (* x w) b)) (pub square (x:f32) f32 = (* x x)) (pub identity (x:f32) f32 = x) (pub constant () f32 = 7f)",
        &[
            ("affine", 3),
            ("square", 1),
            ("identity", 1),
            ("constant", 0),
        ],
    )
}
fn spec() -> GraphSpec {
    GraphSpec {
        abi: GRAPH_ABI.into(),
        input_columns: 1,
        nodes: vec![
            node("affine", vec![input(0), constant(2.), constant(1.)]),
            node("square", vec![previous(0)]),
        ],
        outputs: vec![0, 1],
    }
}
fn limits() -> StreamLimits {
    StreamLimits {
        invocation: Limits::default(),
        maximum_rows: 100,
        maximum_chunk_rows: 32,
        maximum_attempts: 12,
    }
}

#[test]
fn prepared_graph_streams_real_fluxr_nodes_in_fixed_order() {
    let graph = ScalarGraph::prepare(spec(), &[affine_library()]).unwrap();
    let memory = MemoryBudget::new(4096);
    let mut stream = graph
        .stream(limits(), memory.clone(), Cancellation::default())
        .unwrap();
    let mut observed = Vec::new();
    for (sequence, values) in [&[-1., 0., 1.][..], &[2., 3.][..]].into_iter().enumerate() {
        let input = Tensor::new(&[values.len(), 1], values, &memory).unwrap();
        let result = stream
            .execute(graph.identity(), sequence as u64, &input)
            .unwrap();
        observed.extend_from_slice(result.output.values());
        assert_eq!(
            result.receipt.input_sha256.as_deref(),
            Some(tensor_digest(&input).as_str())
        );
        assert_eq!(
            result.receipt.output_sha256.as_deref(),
            Some(tensor_digest(&result.output).as_str())
        );
        assert_eq!(result.receipt.status, ChunkStatus::Completed);
        assert_eq!(result.receipt.attempted_calls, (values.len() * 2) as u64);
    }
    assert_eq!(observed, [-1., 1., 1., 1., 3., 9., 5., 25., 7., 49.]);
    assert_eq!(stream.statistics().published_rows, 5);
    assert_eq!(stream.statistics().attempted_calls, 10);
    assert_eq!(stream.statistics().completed_chunks, 2);
    assert_eq!(stream.attempts().len(), 2);
}

#[test]
fn graph_rejects_invalid_dag_signatures_profiles_and_float_constants() {
    let libraries = [affine_library()];
    for case in 0..10 {
        let mut bad = spec();
        match case {
            0 => bad.abi = "future".into(),
            1 => bad.nodes[0].arguments[0] = previous(0),
            2 => bad.nodes[0].arguments[0] = previous(1),
            3 => bad.nodes[0].arguments[0] = input(1),
            4 => bad.nodes[0].arguments[0] = constant(f32::NAN),
            5 => bad.nodes[0].arguments[0] = constant(f32::INFINITY),
            6 => bad.nodes[0].library = 1,
            7 => bad.nodes[0].export = "missing".into(),
            8 => bad.nodes[0].arguments.pop().map(|_| ()).unwrap(),
            _ => bad.outputs = vec![2],
        }
        assert!(
            ScalarGraph::prepare(bad, &libraries).is_err(),
            "case {case}"
        );
    }
    assert!(ScalarGraph::load(&vec![b' '; 65537], &libraries).is_err());
    let mut bad = serde_json::to_value(spec()).unwrap();
    bad["unknown"] = true.into();
    assert!(ScalarGraph::load(&serde_json::to_vec(&bad).unwrap(), &libraries).is_err());
}

#[test]
fn graph_identity_is_stable_but_binds_exact_sources_and_signed_zero() {
    let lib = affine_library();
    let graph = ScalarGraph::prepare(spec(), std::slice::from_ref(&lib)).unwrap();
    let reloaded = ScalarGraph::load(&graph.encode(), &[lib]).unwrap();
    assert_eq!(graph.identity(), reloaded.identity());
    let mut positive = spec();
    positive.nodes[0].arguments[2] = constant(0.);
    let mut negative = positive.clone();
    negative.nodes[0].arguments[2] = constant(-0.);
    assert_ne!(
        ScalarGraph::prepare(positive, &[affine_library()])
            .unwrap()
            .identity(),
        ScalarGraph::prepare(negative, &[affine_library()])
            .unwrap()
            .identity()
    );
    let a = library("v2 (pub identity (x:f32) f32 = x)", &[("identity", 1)]);
    let b = library("v2 (pub identity (x:f32) f32 = x) ", &[("identity", 1)]);
    assert_eq!(a.identity(), b.identity());
    let small = GraphSpec {
        abi: GRAPH_ABI.into(),
        input_columns: 1,
        nodes: vec![node("identity", vec![input(0)])],
        outputs: vec![0],
    };
    assert_ne!(
        ScalarGraph::prepare(small.clone(), &[a])
            .unwrap()
            .identity(),
        ScalarGraph::prepare(small, &[b]).unwrap().identity()
    );
}

#[test]
fn stale_identity_and_duplicate_sequence_are_recorded_before_work() {
    let graph = ScalarGraph::prepare(spec(), &[affine_library()]).unwrap();
    let memory = MemoryBudget::new(4096);
    let mut stream = graph
        .stream(limits(), memory.clone(), Cancellation::default())
        .unwrap();
    let input = Tensor::new(&[1, 1], &[2.], &memory).unwrap();
    assert!(stream.execute("stale", 0, &input).is_err());
    assert_eq!(stream.next_sequence(), 1);
    assert!(stream.execute(graph.identity(), 0, &input).is_err());
    assert_eq!(stream.statistics().attempted_calls, 0);
    assert_eq!(stream.statistics().admitted_rows, 0);
    assert_eq!(stream.statistics().rejected_chunks, 2);
    assert!(
        stream
            .attempts()
            .iter()
            .all(|r| r.status == ChunkStatus::Rejected && r.input_sha256.is_none())
    );
    assert_eq!(
        stream
            .execute(graph.identity(), 2, &input)
            .unwrap()
            .output
            .values(),
        [5., 25.]
    );
}

#[test]
fn late_trap_preserves_first_failed_node_discards_chunk_and_consumes_envelope() {
    let graph = ScalarGraph::prepare(spec(), &[affine_library()]).unwrap();
    let memory = MemoryBudget::new(4096);
    let mut config = limits();
    config.maximum_rows = 3;
    let mut stream = graph
        .stream(config, memory.clone(), Cancellation::default())
        .unwrap();
    let input = Tensor::new(&[3, 1], &[2., 1e20, 4.], &memory).unwrap();
    let live = memory.live_bytes();
    assert!(stream.execute(graph.identity(), 0, &input).is_err());
    assert_eq!(memory.live_bytes(), live);
    let receipt = &stream.attempts()[0];
    assert_eq!(receipt.status, ChunkStatus::Failed);
    assert_eq!(receipt.first_failed_node, Some(1));
    assert_eq!((receipt.attempted_rows, receipt.completed_rows), (2, 1));
    assert_eq!((receipt.attempted_calls, receipt.completed_calls), (4, 3));
    assert!(receipt.input_sha256.is_some());
    assert!(receipt.output_sha256.is_none());
    assert_eq!(stream.statistics().admitted_rows, 3);
    assert_eq!(stream.statistics().published_rows, 0);
    let retry = Tensor::new(&[1, 1], &[2.], &memory).unwrap();
    assert!(
        stream
            .execute(graph.identity(), 1, &retry)
            .unwrap_err()
            .to_string()
            .contains("row budget")
    );
}

#[test]
fn failed_call_does_not_poison_a_following_chunk_or_reorder_unused_nodes() {
    let lib = library(
        "v2 (pub divide (x:f32) f32 = (/ 1f x)) (pub identity (x:f32) f32 = x)",
        &[("divide", 1), ("identity", 1)],
    );
    let graph = ScalarGraph::prepare(
        GraphSpec {
            abi: GRAPH_ABI.into(),
            input_columns: 1,
            nodes: vec![
                node("divide", vec![input(0)]),
                node("identity", vec![input(0)]),
            ],
            outputs: vec![1],
        },
        &[lib],
    )
    .unwrap();
    let memory = MemoryBudget::new(4096);
    let mut stream = graph
        .stream(limits(), memory.clone(), Cancellation::default())
        .unwrap();
    let bad = Tensor::new(&[1, 1], &[0.], &memory).unwrap();
    assert!(stream.execute(graph.identity(), 0, &bad).is_err());
    assert_eq!(stream.attempts()[0].first_failed_node, Some(0));
    assert_eq!(stream.attempts()[0].attempted_calls, 1);
    let good = Tensor::new(&[1, 1], &[-0.5], &memory).unwrap();
    assert_eq!(
        stream
            .execute(graph.identity(), 1, &good)
            .unwrap()
            .output
            .values(),
        [-0.5]
    );
    assert_eq!(stream.statistics().completed_calls, 2);
}

#[test]
fn cancellation_and_attempt_ledger_exhaustion_never_execute_missing_work() {
    let graph = ScalarGraph::prepare(spec(), &[affine_library()]).unwrap();
    let memory = MemoryBudget::new(4096);
    let cancellation = Cancellation::default();
    let mut config = limits();
    config.maximum_attempts = 2;
    let mut stream = graph
        .stream(config, memory.clone(), cancellation.clone())
        .unwrap();
    let input = Tensor::new(&[1, 1], &[2.], &memory).unwrap();
    cancellation.cancel();
    for sequence in 0..2 {
        assert!(stream.execute(graph.identity(), sequence, &input).is_err());
    }
    assert_eq!(stream.attempts().len(), 2);
    assert_eq!(stream.statistics().attempted_calls, 0);
    assert_eq!(stream.statistics().rejected_chunks, 2);
    assert!(
        stream
            .execute(graph.identity(), 2, &input)
            .unwrap_err()
            .to_string()
            .contains("ledger exhausted")
    );
    assert_eq!(stream.attempts().len(), 2);
}

#[test]
fn stream_payload_peak_is_chunk_bounded_and_owned_outputs_hold_charges() {
    let graph = ScalarGraph::prepare(spec(), &[affine_library()]).unwrap();
    let scratch = (2 + 32) * 4;
    let memory = MemoryBudget::new(scratch + 4 + 8);
    let mut stream = graph
        .stream(limits(), memory.clone(), Cancellation::default())
        .unwrap();
    let input = Tensor::new(&[1, 1], &[2.], &memory).unwrap();
    let first = stream.execute(graph.identity(), 0, &input).unwrap();
    assert_eq!(memory.live_bytes(), scratch + 12);
    assert!(stream.execute(graph.identity(), 1, &input).is_err());
    assert_eq!(stream.statistics().admitted_rows, 1);
    drop(first);
    for sequence in 2..10 {
        let chunk = stream.execute(graph.identity(), sequence, &input).unwrap();
        assert_eq!(chunk.output.values(), [5., 25.]);
        drop(chunk);
    }
    assert_eq!(memory.peak_bytes(), scratch + 12);
    drop(input);
    drop(stream);
    assert_eq!(memory.live_bytes(), 0);
}

#[test]
fn instruction_cell_frame_limits_match_direct_scalar_calls() {
    let lib = affine_library();
    let graph = ScalarGraph::prepare(
        GraphSpec {
            abi: GRAPH_ABI.into(),
            input_columns: 1,
            nodes: vec![node("square", vec![input(0)])],
            outputs: vec![0],
        },
        std::slice::from_ref(&lib),
    )
    .unwrap();
    let memory = MemoryBudget::new(4096);
    let input = Tensor::new(&[1, 1], &[2.], &memory).unwrap();
    for instructions in 0..9 {
        for cells in 0..8 {
            for frames in 0..2 {
                let invocation = Limits {
                    instructions,
                    cells,
                    frames,
                };
                let mut config = limits();
                config.invocation = invocation;
                let mut stream = graph
                    .stream(config, memory.clone(), Cancellation::default())
                    .unwrap();
                let result = stream.execute(graph.identity(), 0, &input);
                let mut direct = lib.session(invocation, 1, Cancellation::default());
                let expected = direct.call("square", &[2.]);
                assert_eq!(
                    result
                        .map(|chunk| chunk.output.values()[0].to_bits())
                        .map_err(|e| e.to_string()),
                    expected.map(f32::to_bits).map_err(|e| e.to_string()),
                    "fuel={instructions} cells={cells} frames={frames}"
                );
                assert_eq!(
                    stream.statistics().attempted_calls,
                    direct.attempted_calls()
                );
                assert_eq!(
                    stream.statistics().completed_calls,
                    direct.completed_calls()
                );
            }
        }
    }
}

#[test]
fn zero_rows_and_zero_arity_preserve_declared_shape_and_calls() {
    let graph = ScalarGraph::prepare(
        GraphSpec {
            abi: GRAPH_ABI.into(),
            input_columns: 0,
            nodes: vec![node("constant", vec![])],
            outputs: vec![0, 0],
        },
        &[affine_library()],
    )
    .unwrap();
    let memory = MemoryBudget::new(4096);
    let mut stream = graph
        .stream(limits(), memory.clone(), Cancellation::default())
        .unwrap();
    let empty = Tensor::new(&[0, 0], &[], &memory).unwrap();
    assert_eq!(
        stream
            .execute(graph.identity(), 0, &empty)
            .unwrap()
            .output
            .shape(),
        [0, 2]
    );
    let input = Tensor::new(&[3, 0], &[], &memory).unwrap();
    let output = stream.execute(graph.identity(), 1, &input).unwrap();
    assert_eq!(output.output.shape(), [3, 2]);
    assert_eq!(output.output.values(), [7.; 6]);
    assert_eq!(stream.statistics().attempted_calls, 3);
}

#[test]
fn finite_bits_and_chunk_boundaries_match_independent_scalar_schedule() {
    let lib = affine_library();
    let graph = ScalarGraph::prepare(spec(), std::slice::from_ref(&lib)).unwrap();
    let memory = MemoryBudget::new(32768);
    let mut config = limits();
    config.maximum_rows = 512;
    config.maximum_attempts = 64;
    let mut stream = graph
        .stream(config, memory.clone(), Cancellation::default())
        .unwrap();
    let mut direct = lib.session(Limits::default(), 1024, Cancellation::default());
    let mut seed = 0x1234abcdu32;
    for sequence in 0..32 {
        let mut data = vec![0., -0., f32::MIN_POSITIVE, f32::from_bits(1)];
        for _ in 0..12 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            data.push((seed as i32) as f32 / 1e9);
        }
        let input = Tensor::new(&[16, 1], &data, &memory).unwrap();
        let chunk = stream.execute(graph.identity(), sequence, &input).unwrap();
        for (row, value) in data.into_iter().enumerate() {
            let a = direct.call("affine", &[value, 2., 1.]).unwrap();
            let b = direct.call("square", &[a]).unwrap();
            assert_eq!(chunk.output.values()[2 * row].to_bits(), a.to_bits());
            assert_eq!(chunk.output.values()[2 * row + 1].to_bits(), b.to_bits());
        }
        assert_eq!(
            tensor_digest(&input),
            format!("{:x}", Sha256::digest(input.encode()))
        );
    }
    assert_eq!(stream.statistics().attempted_calls, 1024);
}

#[test]
fn aggregate_budget_overflow_and_shape_failures_are_explicit() {
    let graph = ScalarGraph::prepare(spec(), &[affine_library()]).unwrap();
    let memory = MemoryBudget::new(4096);
    let mut config = limits();
    config.maximum_rows = u64::MAX;
    assert!(
        graph
            .stream(config, memory.clone(), Cancellation::default())
            .is_err()
    );
    let mut stream = graph
        .stream(limits(), memory.clone(), Cancellation::default())
        .unwrap();
    for (sequence, shape, data) in [
        (0, &[2][..], &[1., 2.][..]),
        (1, &[1, 2][..], &[1., 2.][..]),
        (2, &[33, 1][..], &[1.; 33][..]),
    ] {
        let input = Tensor::new(shape, data, &memory).unwrap();
        assert!(stream.execute(graph.identity(), sequence, &input).is_err());
    }
    assert_eq!(stream.statistics().attempted_calls, 0);
    assert_eq!(stream.statistics().rejected_chunks, 3);
}

#[test]
fn independent_stream_workspaces_share_program_and_keep_owned_results() {
    let graph = ScalarGraph::prepare(spec(), &[affine_library()]).unwrap();
    let memory = MemoryBudget::new(4096);
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for lane in 0..4 {
            let graph = &graph;
            let memory = memory.clone();
            handles.push(scope.spawn(move || {
                let mut stream = graph
                    .stream(limits(), memory.clone(), Cancellation::default())
                    .unwrap();
                let input = Tensor::new(&[1, 1], &[lane as f32], &memory).unwrap();
                stream.execute(graph.identity(), 0, &input).unwrap().output
            }));
        }
        for (lane, handle) in handles.into_iter().enumerate() {
            let result = handle.join().unwrap();
            let value = lane as f32 * 2. + 1.;
            assert_eq!(result.values(), [value, value * value]);
        }
    });
    assert_eq!(memory.live_bytes(), 0);
}

#[test]
fn detached_receipts_bind_graph_policy_limits_and_actual_outputs() {
    let graph = ScalarGraph::prepare(spec(), &[affine_library()]).unwrap();
    let memory = MemoryBudget::new(4096);
    let input = Tensor::new(&[1, 1], &[2.], &memory).unwrap();
    let mut a = graph
        .stream(limits(), memory.clone(), Cancellation::default())
        .unwrap();
    let mut b = graph
        .stream(limits(), memory.clone(), Cancellation::default())
        .unwrap();
    assert_eq!(a.contract_identity(), b.contract_identity());
    let first = a.execute(graph.identity(), 0, &input).unwrap();
    let second = b.execute(graph.identity(), 0, &input).unwrap();
    assert_eq!(
        serde_json::to_vec(&first.receipt).unwrap(),
        serde_json::to_vec(&second.receipt).unwrap()
    );
    assert_eq!(first.receipt.graph_identity, graph.identity());
    assert_eq!(
        first.receipt.stream_contract_identity,
        a.contract_identity()
    );
    assert_eq!(
        first.receipt.output_sha256.as_deref(),
        Some(tensor_digest(&first.output).as_str())
    );
    let mut changed = limits();
    changed.invocation.instructions -= 1;
    let other = graph
        .stream(changed, memory, Cancellation::default())
        .unwrap();
    assert_ne!(a.contract_identity(), other.contract_identity());
    assert_eq!(a.contract().device, "cpu");
    let oversized = "z".repeat(129);
    assert!(a.execute(&oversized, 1, &input).is_err());
    assert!(a.attempts()[1].requested_graph.is_none());
    assert_eq!(a.attempts()[1].requested_graph_bytes, 129);
}

#[test]
fn graph_canonical_expansion_is_rejected_before_unbounded_encoding() {
    let name = "f".repeat(64);
    let parameters = (0..32)
        .map(|index| format!("p{index}"))
        .collect::<Vec<_>>()
        .join(" ");
    let source = format!("v2 (pub {name} ({parameters}:f32) f32 = 1f)");
    let library = library(&source, &[(&name, 32)]);
    let spec = GraphSpec {
        abi: GRAPH_ABI.into(),
        input_columns: 0,
        nodes: vec![node(&name, vec![constant(1.); 32]); 256],
        outputs: vec![0],
    };
    let result = ScalarGraph::prepare(spec, &[library]);
    assert!(
        result
            .err()
            .unwrap()
            .to_string()
            .contains("canonical byte budget")
    );
}

#[test]
fn new_external_fluxr_effect_package_matches_compiled_rust_function_pair() {
    let library = ScalarLibrary::load(
        include_bytes!("../examples/native/effects/softclip/manifest.json"),
        include_str!("../examples/native/effects/softclip/program.flux"),
    )
    .unwrap();
    let mut session = library.session(Limits::default(), 8193, Cancellation::default());
    for index in 0..4096 {
        let left = (index as f32 - 2048.) / 1024.;
        let right = -left * 0.25;
        let mixed = session.call("mix", &[left, right, 0.35]).unwrap();
        let expected_mix = effect_reference::mix(left, right, 0.35);
        assert_eq!(mixed.to_bits(), expected_mix.to_bits());
        let actual = session.call("softclip", &[mixed, 1.5]).unwrap();
        let expected = effect_reference::softclip(expected_mix, 1.5);
        assert_eq!(actual.to_bits(), expected.to_bits());
    }
    assert!(session.call("softclip", &[f32::MAX, 2.]).is_err());
    assert_eq!(
        (session.attempted_calls(), session.completed_calls()),
        (8193, 8192)
    );
}
