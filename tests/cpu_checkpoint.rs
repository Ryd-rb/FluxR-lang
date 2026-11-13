use fluxr::cpu_compute::stream::stateful::checkpoint::{CheckpointAdmission, MAX_CHECKPOINT_BYTES};
use fluxr::cpu_compute::stream::stateful::{STATEFUL_ABI, StatefulGraph, StatefulSpec};
use fluxr::cpu_compute::stream::{Argument, GRAPH_ABI, GraphSpec, Node, StreamLimits};
use fluxr::cpu_compute::{
    ABI, Cancellation, LibraryManifest, MemoryBudget, NUMERIC_PROFILE, ScalarLibrary, Tensor,
};
use fluxr::structured::Limits;
use sha2::{Digest, Sha256};
use std::sync::Arc;

fn library(source: &str, export: &str, arity: usize) -> Arc<ScalarLibrary> {
    let manifest = LibraryManifest {
        abi: ABI.into(),
        numeric_profile: NUMERIC_PROFILE.into(),
        source_sha256: format!("{:x}", Sha256::digest(source.as_bytes())),
        package: format!("checkpoint.{export}"),
        version: 1,
        exports: [(export.to_string(), arity)].into_iter().collect(),
        capabilities: vec!["compute.cpu".into()],
    };
    Arc::new(ScalarLibrary::load(&serde_json::to_vec(&manifest).unwrap(), source).unwrap())
}
fn graph_with_initial(bits: u32) -> StatefulGraph {
    StatefulGraph::prepare(
        StatefulSpec {
            abi: STATEFUL_ABI.into(),
            external_columns: 1,
            transition: GraphSpec {
                abi: GRAPH_ABI.into(),
                input_columns: 2,
                nodes: vec![
                    Node {
                        library: 0,
                        export: "add".into(),
                        arguments: vec![
                            Argument::Input { column: 0 },
                            Argument::Input { column: 1 },
                        ],
                    },
                    Node {
                        library: 1,
                        export: "inverse".into(),
                        arguments: vec![Argument::Input { column: 0 }],
                    },
                ],
                outputs: vec![0],
            },
            initial_state_bits: vec![bits],
            state_outputs: vec![0],
        },
        &[
            library("v2 (pub add (x s:f32) f32 = (+ x s))", "add", 2),
            library("v2 (pub inverse (x:f32) f32 = (/ 1f x))", "inverse", 1),
        ],
    )
    .unwrap()
}
fn graph() -> StatefulGraph {
    graph_with_initial(0)
}
fn limits() -> StreamLimits {
    StreamLimits {
        invocation: Limits::default(),
        maximum_rows: 64,
        maximum_chunk_rows: 16,
        maximum_attempts: 32,
    }
}
fn admission(bytes: &[u8]) -> CheckpointAdmission {
    CheckpointAdmission {
        maximum_bytes: 128 * 1024,
        expected_sha256: Sha256::digest(bytes).into(),
    }
}
fn tensor(data: &[f32]) -> Tensor {
    Tensor::new(&[data.len(), 1], data, &MemoryBudget::new(1 << 20)).unwrap()
}
fn bits(values: &[f32]) -> Vec<u32> {
    values.iter().map(|v| v.to_bits()).collect()
}
fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

#[test]
fn resume_matches_uninterrupted_output_state_and_entire_ledger() {
    let graph = graph();
    let mut whole = graph
        .stream(limits(), MemoryBudget::new(4096), Cancellation::default())
        .unwrap();
    let mut split = graph
        .stream(limits(), MemoryBudget::new(4096), Cancellation::default())
        .unwrap();
    let chunks = [vec![1., 2., 4.], vec![], vec![0.5, 0.25], vec![8., 16.]];
    let mut sum = 0f32;
    for data in &chunks {
        let input = tensor(data);
        let left = whole
            .execute(
                graph.identity(),
                whole.state_pin(),
                whole.next_sequence(),
                &input,
            )
            .unwrap();
        let right = split
            .execute(
                graph.identity(),
                split.state_pin(),
                split.next_sequence(),
                &input,
            )
            .unwrap();
        let expected: Vec<_> = data
            .iter()
            .map(|&x| {
                sum += x;
                sum.to_bits()
            })
            .collect();
        assert_eq!(bits(left.output.values()), expected);
        assert_eq!(bits(right.output.values()), expected);
        let checkpoint = split.checkpoint(128 * 1024).unwrap();
        let checkpoint_again = split.checkpoint(128 * 1024).unwrap();
        assert_eq!(checkpoint.bytes(), checkpoint_again.bytes());
        assert_eq!(checkpoint.sha256(), checkpoint_again.sha256());
        drop(split);
        split = graph
            .resume_checkpoint(
                checkpoint.bytes(),
                admission(checkpoint.bytes()),
                limits(),
                MemoryBudget::new(4096),
                Cancellation::default(),
            )
            .unwrap();
        assert_eq!(split.state_pin(), whole.state_pin());
        assert_eq!(bits(split.state()), bits(whole.state()));
        assert_eq!(
            serde_json::to_value(split.attempts()).unwrap(),
            serde_json::to_value(whole.attempts()).unwrap()
        );
        assert_eq!(
            serde_json::to_value(split.statistics()).unwrap(),
            serde_json::to_value(whole.statistics()).unwrap()
        );
        assert_eq!(
            split.checkpoint(128 * 1024).unwrap().bytes(),
            checkpoint.bytes()
        );
    }
}

#[test]
fn failed_rejected_and_empty_attempts_keep_all_charges_after_resume() {
    let graph = graph();
    let mut l = limits();
    l.maximum_rows = 5;
    l.maximum_attempts = 5;
    let mut stream = graph
        .stream(l, MemoryBudget::new(4096), Cancellation::default())
        .unwrap();
    let before = stream.state_pin();
    assert!(
        stream
            .execute(graph.identity(), before, 0, &tensor(&[1., 0., 3.]))
            .is_err()
    );
    assert!(stream.execute("wrong", before, 1, &tensor(&[1.])).is_err());
    stream
        .execute(graph.identity(), before, 2, &tensor(&[]))
        .unwrap();
    let previous_attempts = serde_json::to_value(stream.attempts()).unwrap();
    let checkpoint = stream.checkpoint(128 * 1024).unwrap();
    drop(stream);
    let mut restored = graph
        .resume_checkpoint(
            checkpoint.bytes(),
            admission(checkpoint.bytes()),
            l,
            MemoryBudget::new(4096),
            Cancellation::default(),
        )
        .unwrap();
    assert_eq!(
        serde_json::to_value(restored.attempts()).unwrap(),
        previous_attempts
    );
    assert_eq!(restored.statistics().admitted_rows, 3);
    assert_eq!(restored.statistics().attempted_calls, 4);
    assert_eq!(restored.statistics().completed_calls, 3);
    assert_eq!(restored.next_sequence(), 3);
    assert_eq!(restored.state_pin().revision, 1);
    let output = restored
        .execute(
            graph.identity(),
            restored.state_pin(),
            3,
            &tensor(&[2., 4.]),
        )
        .unwrap();
    assert_eq!(output.output.values(), [2., 6.]);
    assert!(
        restored
            .execute(graph.identity(), restored.state_pin(), 4, &tensor(&[1.]),)
            .is_err()
    );
    let statistics = serde_json::to_value(restored.statistics()).unwrap();
    let last = restored.checkpoint(128 * 1024).unwrap();
    let mut exhausted = graph
        .resume_checkpoint(
            last.bytes(),
            admission(last.bytes()),
            l,
            MemoryBudget::new(4096),
            Cancellation::default(),
        )
        .unwrap();
    assert!(
        exhausted
            .execute(graph.identity(), exhausted.state_pin(), 5, &tensor(&[]),)
            .unwrap_err()
            .to_string()
            .contains("ledger")
    );
    assert_eq!(
        serde_json::to_value(exhausted.statistics()).unwrap(),
        statistics
    );
    assert_eq!(exhausted.statistics().admitted_rows, 5);
    assert_eq!(exhausted.statistics().attempted_calls, 8);
    assert_eq!(exhausted.statistics().completed_calls, 7);
}

#[test]
fn signed_zero_and_zero_row_revision_survive_checkpoint() {
    let graph = graph_with_initial((-0f32).to_bits());
    let mut stream = graph
        .stream(limits(), MemoryBudget::new(4096), Cancellation::default())
        .unwrap();
    stream
        .execute(graph.identity(), stream.state_pin(), 0, &tensor(&[]))
        .unwrap();
    let checkpoint = stream.checkpoint(128 * 1024).unwrap();
    let restored = graph
        .resume_checkpoint(
            checkpoint.bytes(),
            admission(checkpoint.bytes()),
            limits(),
            MemoryBudget::new(4096),
            Cancellation::default(),
        )
        .unwrap();
    assert_eq!(bits(restored.state()), [(-0f32).to_bits()]);
    assert_eq!(restored.state_pin().revision, 1);
    assert_eq!(restored.next_sequence(), 1);
}

#[test]
fn caller_digest_graph_source_contract_and_memory_policy_are_required() {
    let graph = graph();
    let stream = graph
        .stream(limits(), MemoryBudget::new(4096), Cancellation::default())
        .unwrap();
    let checkpoint = stream.checkpoint(128 * 1024).unwrap();
    let mut bad = admission(checkpoint.bytes());
    bad.expected_sha256[0] ^= 1;
    assert!(
        graph
            .resume_checkpoint(
                checkpoint.bytes(),
                bad,
                limits(),
                MemoryBudget::new(4096),
                Cancellation::default()
            )
            .is_err()
    );
    let changed = graph_with_initial(1f32.to_bits());
    assert!(
        changed
            .resume_checkpoint(
                checkpoint.bytes(),
                admission(checkpoint.bytes()),
                limits(),
                MemoryBudget::new(4096),
                Cancellation::default()
            )
            .is_err()
    );
    let mut l = limits();
    l.invocation.instructions += 1;
    assert!(
        graph
            .resume_checkpoint(
                checkpoint.bytes(),
                admission(checkpoint.bytes()),
                l,
                MemoryBudget::new(4096),
                Cancellation::default()
            )
            .is_err()
    );
    assert!(
        graph
            .resume_checkpoint(
                checkpoint.bytes(),
                admission(checkpoint.bytes()),
                limits(),
                MemoryBudget::new(8192),
                Cancellation::default()
            )
            .is_err()
    );
}

#[test]
fn cancelled_checkpoints_and_cancelled_callers_cannot_resume() {
    let graph = graph();
    let cancellation = Cancellation::default();
    let stream = graph
        .stream(limits(), MemoryBudget::new(4096), cancellation.clone())
        .unwrap();
    let live = stream.checkpoint(128 * 1024).unwrap();
    cancellation.cancel();
    let cancelled = stream.checkpoint(128 * 1024).unwrap();
    assert_ne!(live.sha256(), cancelled.sha256());
    let fresh = Cancellation::default();
    assert!(
        graph
            .resume_checkpoint(
                cancelled.bytes(),
                admission(cancelled.bytes()),
                limits(),
                MemoryBudget::new(4096),
                fresh.clone()
            )
            .is_err()
    );
    assert!(!fresh.is_cancelled());
    assert!(
        graph
            .resume_checkpoint(
                live.bytes(),
                admission(live.bytes()),
                limits(),
                MemoryBudget::new(4096),
                cancellation
            )
            .is_err()
    );
}

#[test]
fn every_truncation_trailing_data_unknown_version_and_byte_cap_refuse() {
    let graph = graph();
    let stream = graph
        .stream(limits(), MemoryBudget::new(4096), Cancellation::default())
        .unwrap();
    let checkpoint = stream.checkpoint(128 * 1024).unwrap();
    for end in 0..checkpoint.bytes().len() {
        let bytes = &checkpoint.bytes()[..end];
        assert!(
            graph
                .resume_checkpoint(
                    bytes,
                    admission(bytes),
                    limits(),
                    MemoryBudget::new(4096),
                    Cancellation::default()
                )
                .is_err(),
            "truncation {end}"
        );
    }
    let mut bytes = checkpoint.bytes().to_vec();
    bytes.push(0);
    let length = bytes.len() as u64;
    put_u64(&mut bytes, 8, length);
    assert!(
        graph
            .resume_checkpoint(
                &bytes,
                admission(&bytes),
                limits(),
                MemoryBudget::new(4096),
                Cancellation::default()
            )
            .is_err()
    );
    let mut bytes = checkpoint.bytes().to_vec();
    bytes[3] = b'2';
    assert!(
        graph
            .resume_checkpoint(
                &bytes,
                admission(&bytes),
                limits(),
                MemoryBudget::new(4096),
                Cancellation::default()
            )
            .is_err()
    );
    assert!(stream.checkpoint(checkpoint.bytes().len() - 1).is_err());
    assert!(stream.checkpoint(0).is_err());
    assert!(stream.checkpoint(MAX_CHECKPOINT_BYTES + 1).is_err());
    let mut cap = admission(checkpoint.bytes());
    cap.maximum_bytes = checkpoint.bytes().len() - 1;
    assert!(
        graph
            .resume_checkpoint(
                checkpoint.bytes(),
                cap,
                limits(),
                MemoryBudget::new(4096),
                Cancellation::default()
            )
            .is_err()
    );
}

#[test]
fn malformed_finite_state_statistics_and_session_budgets_refuse_without_mutation() {
    let graph = graph();
    let mut stream = graph
        .stream(limits(), MemoryBudget::new(4096), Cancellation::default())
        .unwrap();
    let checkpoint = stream.checkpoint(128 * 1024).unwrap();
    let old_pin = stream.state_pin();
    // FSC1 fixed header: policy ends at 200, StatePin at 240, count at 242;
    // one state cell ends at 246, nine counters at 318, library count at 320.
    for case in 0..8 {
        let mut bytes = checkpoint.bytes().to_vec();
        match case {
            0 => bytes[242..246].copy_from_slice(&f32::NAN.to_bits().to_le_bytes()),
            1 => bytes[242..246].copy_from_slice(&f32::INFINITY.to_bits().to_le_bytes()),
            2 => bytes[242..246].copy_from_slice(&1f32.to_bits().to_le_bytes()),
            3 => put_u64(&mut bytes, 246, 1),
            4 => put_u64(&mut bytes, 320 + 128 + 16, 63),
            5 => bytes[240..242].copy_from_slice(&65u16.to_le_bytes()),
            6 => bytes[318..320].copy_from_slice(&127u16.to_le_bytes()),
            _ => bytes[624..628].copy_from_slice(&u32::MAX.to_le_bytes()),
        }
        assert!(
            graph
                .resume_checkpoint(
                    &bytes,
                    admission(&bytes),
                    limits(),
                    MemoryBudget::new(4096),
                    Cancellation::default()
                )
                .is_err(),
            "case {case}"
        );
        assert_eq!(stream.state_pin(), old_pin);
        assert_eq!(stream.next_sequence(), 0);
    }
    assert_eq!(
        stream
            .execute(graph.identity(), old_pin, 0, &tensor(&[2.]))
            .unwrap()
            .output
            .values(),
        [2.]
    );
}

#[test]
fn shared_budget_preserves_old_output_ownership_and_refuses_double_stream_reservation() {
    let graph = graph();
    // Two nodes + 32 argument cells + committed/tentative state = 144 bytes.
    let memory = MemoryBudget::new(148);
    let mut stream = graph
        .stream(limits(), memory.clone(), Cancellation::default())
        .unwrap();
    let output = stream
        .execute(graph.identity(), stream.state_pin(), 0, &tensor(&[1.]))
        .unwrap();
    let checkpoint = stream.checkpoint(128 * 1024).unwrap();
    assert!(
        graph
            .resume_checkpoint(
                checkpoint.bytes(),
                admission(checkpoint.bytes()),
                limits(),
                memory.clone(),
                Cancellation::default()
            )
            .is_err()
    );
    assert_eq!(memory.live_bytes(), 148);
    drop(stream);
    let mut restored = graph
        .resume_checkpoint(
            checkpoint.bytes(),
            admission(checkpoint.bytes()),
            limits(),
            memory.clone(),
            Cancellation::default(),
        )
        .unwrap();
    assert_eq!(memory.live_bytes(), 148);
    assert!(
        restored
            .execute(graph.identity(), restored.state_pin(), 1, &tensor(&[2.]))
            .is_err()
    );
    drop(output);
    assert_eq!(memory.live_bytes(), 144);
    assert_eq!(
        restored
            .execute(graph.identity(), restored.state_pin(), 2, &tensor(&[2.]))
            .unwrap()
            .output
            .values(),
        [3.]
    );
    drop(restored);
    assert_eq!(memory.live_bytes(), 0);
}

#[test]
fn deliberate_replay_is_a_fork_and_does_not_claim_exactly_once() {
    let graph = graph();
    let stream = graph
        .stream(limits(), MemoryBudget::new(4096), Cancellation::default())
        .unwrap();
    let checkpoint = stream.checkpoint(128 * 1024).unwrap();
    let mut branches = (0..2)
        .map(|_| {
            graph
                .resume_checkpoint(
                    checkpoint.bytes(),
                    admission(checkpoint.bytes()),
                    limits(),
                    MemoryBudget::new(4096),
                    Cancellation::default(),
                )
                .unwrap()
        })
        .collect::<Vec<_>>();
    let mut observed = Vec::new();
    for branch in &mut branches {
        observed.push(
            branch
                .execute(
                    graph.identity(),
                    branch.state_pin(),
                    branch.next_sequence(),
                    &tensor(&[1., 2.]),
                )
                .unwrap()
                .output
                .values()
                .to_vec(),
        );
    }
    assert_eq!(observed[0], observed[1]);
    assert_eq!(branches[0].state_pin(), branches[1].state_pin());
}

#[test]
fn rehashed_uncancelled_failed_call_cannot_refund_the_charged_attempt() {
    let graph = graph();
    let mut stream = graph
        .stream(limits(), MemoryBudget::new(4096), Cancellation::default())
        .unwrap();
    assert!(
        stream
            .execute(graph.identity(), stream.state_pin(), 0, &tensor(&[0.]))
            .is_err()
    );
    assert_eq!(stream.statistics().attempted_calls, 2);
    assert_eq!(stream.statistics().completed_calls, 1);
    let checkpoint = stream.checkpoint(128 * 1024).unwrap();
    let mut forged = checkpoint.bytes().to_vec();
    // Keep the malicious file internally aggregate-consistent: refund the
    // failed inverse call in the stream total, second library and its receipt.
    // FSC1 offsets for one state cell, two libraries and a first failed chunk.
    put_u64(&mut forged, 246 + 4 * 8, 1);
    put_u64(&mut forged, 320 + 152 + 128, 0);
    put_u64(&mut forged, 320 + 152 + 144, 64);
    put_u64(&mut forged, 628 + 305, 1);
    assert!(
        graph
            .resume_checkpoint(
                &forged,
                admission(&forged),
                limits(),
                MemoryBudget::new(4096),
                Cancellation::default(),
            )
            .is_err()
    );
    assert_eq!(stream.statistics().attempted_calls, 2);
    assert_eq!(
        stream
            .execute(graph.identity(), stream.state_pin(), 1, &tensor(&[1.]))
            .unwrap()
            .output
            .values(),
        [1.]
    );
}

#[test]
fn rehashed_uncancelled_boundary_failure_cannot_masquerade_as_resumable_history() {
    let graph = graph();
    let mut stream = graph
        .stream(limits(), MemoryBudget::new(4096), Cancellation::default())
        .unwrap();
    assert!(
        stream
            .execute(graph.identity(), stream.state_pin(), 0, &tensor(&[0.]))
            .is_err()
    );
    let checkpoint = stream.checkpoint(128 * 1024).unwrap();
    let mut forged = checkpoint.bytes().to_vec();
    // Convert a charged node trap into a fully computed, unpublished row,
    // including consistent whole-ledger and per-library completion counters.
    // Such an endpoint can only result from terminal cancellation in v1.
    put_u64(&mut forged, 246 + 2 * 8, 1);
    put_u64(&mut forged, 246 + 5 * 8, 2);
    put_u64(&mut forged, 320 + 152 + 136, 1);
    put_u64(&mut forged, 628 + 297, 1);
    put_u64(&mut forged, 628 + 313, 2);
    forged[628 + 321] = 0;
    forged.drain(628 + 322..628 + 330);
    let length = forged.len() as u64;
    put_u64(&mut forged, 8, length);
    let error = graph
        .resume_checkpoint(
            &forged,
            admission(&forged),
            limits(),
            MemoryBudget::new(4096),
            Cancellation::default(),
        )
        .err()
        .unwrap();
    assert!(error.to_string().contains("boundary failure"));
    assert_eq!(stream.statistics().completed_calls, 1);
    assert_eq!(stream.next_sequence(), 1);
}
