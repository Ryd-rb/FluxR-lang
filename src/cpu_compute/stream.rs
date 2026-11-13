//! Immutable, checked scalar graphs and bounded CPU stream execution.
//!
//! Graphs reuse the released scalar-library evaluator. Every row executes every
//! node in declaration order, including unused nodes, so neither arithmetic nor
//! first-trap/call-budget observations are reordered by this adapter. A chunk is
//! published only after all its rows succeed. This is an offline/worker API,
//! not a real-time callback or a hostile-native-code isolation boundary.
pub mod stateful;

use super::{
    Cancellation, MemoryBudget, NUMERIC_PROFILE, Reservation, ScalarLibrary, ScalarSession, Tensor,
    fail, finite, reserve_vec,
};
use crate::Error;
use crate::structured::Limits;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;

pub const GRAPH_ABI: &str = "fluxr-scalar-graph-v1";
const MAX_SPEC_BYTES: usize = 64 * 1024;
const MAX_NODES: usize = 256;
const MAX_LIBRARIES: usize = 128;
const MAX_COLUMNS: usize = 64;
const MAX_ATTEMPTS: usize = 65_536;

fn encode_spec(spec: &GraphSpec) -> Result<Vec<u8>, Error> {
    struct BoundedBytes(Vec<u8>);
    impl std::io::Write for BoundedBytes {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > MAX_SPEC_BYTES - self.0.len() {
                return Err(std::io::Error::other(
                    "scalar graph canonical byte budget exceeded",
                ));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut bytes = BoundedBytes(Vec::new());
    serde_json::to_writer(&mut bytes, spec).map_err(|error| fail(error.to_string()))?;
    Ok(bytes.0)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Argument {
    Input { column: usize },
    Constant { bits: u32 },
    Node { node: usize },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub library: usize,
    pub export: String,
    pub arguments: Vec<Argument>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphSpec {
    pub abi: String,
    pub input_columns: usize,
    pub nodes: Vec<Node>,
    pub outputs: Vec<usize>,
}

/// A compiled library's checked export signatures are authoritative. Caller
/// names, arities and dependency edges cannot override them. Earlier-node edges
/// give this v1 profile a deterministic topological order without hidden repair.
pub struct ScalarGraph {
    spec: GraphSpec,
    libraries: Vec<Arc<ScalarLibrary>>,
    calls_per_row: Vec<u64>,
    identity: String,
}

impl ScalarGraph {
    pub fn load(bytes: &[u8], libraries: &[Arc<ScalarLibrary>]) -> Result<Self, Error> {
        if bytes.len() > MAX_SPEC_BYTES {
            return Err(fail("scalar graph byte budget exceeded"));
        }
        let spec = serde_json::from_slice(bytes)
            .map_err(|error| fail(format!("invalid scalar graph: {error}")))?;
        Self::prepare(spec, libraries)
    }

    pub fn prepare(spec: GraphSpec, libraries: &[Arc<ScalarLibrary>]) -> Result<Self, Error> {
        if spec.abi != GRAPH_ABI
            || spec.input_columns > MAX_COLUMNS
            || spec.nodes.is_empty()
            || spec.nodes.len() > MAX_NODES
            || spec.outputs.is_empty()
            || spec.outputs.len() > MAX_COLUMNS
            || libraries.is_empty()
            || libraries.len() > MAX_LIBRARIES
        {
            return Err(fail("unsupported scalar graph ABI or graph budget"));
        }
        let mut calls_per_row = vec![0u64; libraries.len()];
        for (index, node) in spec.nodes.iter().enumerate() {
            let library = libraries
                .get(node.library)
                .ok_or_else(|| fail("scalar graph library index out of bounds"))?;
            if node.arguments.len() > super::MAX_ARGS
                || library.manifest().exports.get(&node.export) != Some(&node.arguments.len())
            {
                return Err(fail("scalar graph export or arity mismatch"));
            }
            for argument in &node.arguments {
                match argument {
                    Argument::Input { column } if *column >= spec.input_columns => {
                        return Err(fail("scalar graph input column out of bounds"));
                    }
                    Argument::Node { node } if *node >= index => {
                        return Err(fail("scalar graph edge is not an earlier node"));
                    }
                    Argument::Constant { bits } => {
                        finite(f32::from_bits(*bits))?;
                    }
                    _ => {}
                }
            }
            calls_per_row[node.library] += 1;
        }
        if spec.outputs.iter().any(|&index| index >= spec.nodes.len()) {
            return Err(fail("scalar graph output index out of bounds"));
        }
        // All supplied libraries are exact captured inputs, even if unused.
        // The semantic program id and the exact source digest have distinct
        // roles; neither hash authenticates a package or grants a capability.
        let canonical = encode_spec(&spec)?;
        let mut digest = Sha256::new();
        digest.update(GRAPH_ABI.as_bytes());
        digest.update([0]);
        digest.update(NUMERIC_PROFILE.as_bytes());
        digest.update((canonical.len() as u64).to_le_bytes());
        digest.update(canonical);
        for library in libraries {
            digest.update(library.identity().as_bytes());
            digest.update(library.manifest().source_sha256.as_bytes());
        }
        Ok(Self {
            spec,
            libraries: libraries.to_vec(),
            calls_per_row,
            identity: format!("{:x}", digest.finalize()),
        })
    }

    pub fn identity(&self) -> &str {
        &self.identity
    }

    pub fn input_columns(&self) -> usize {
        self.spec.input_columns
    }

    pub fn output_columns(&self) -> usize {
        self.spec.outputs.len()
    }

    pub fn node_count(&self) -> usize {
        self.spec.nodes.len()
    }

    pub fn encode(&self) -> Vec<u8> {
        // This immutable spec was already serialized within MAX_SPEC_BYTES.
        encode_spec(&self.spec).expect("checked scalar graph serialization")
    }

    pub fn stream(
        &self,
        limits: StreamLimits,
        memory: MemoryBudget,
        cancellation: Cancellation,
    ) -> Result<ScalarStream<'_>, Error> {
        if limits.maximum_chunk_rows == 0
            || limits.maximum_attempts == 0
            || limits.maximum_attempts > MAX_ATTEMPTS
        {
            return Err(fail("scalar stream chunk or attempt budget is invalid"));
        }
        limits
            .maximum_rows
            .checked_mul(self.spec.nodes.len() as u64)
            .ok_or_else(|| fail("scalar stream aggregate call budget overflow"))?;
        let mut sessions = Vec::with_capacity(self.libraries.len());
        for (library, &calls) in self.libraries.iter().zip(&self.calls_per_row) {
            let maximum_calls = limits
                .maximum_rows
                .checked_mul(calls)
                .ok_or_else(|| fail("scalar stream call budget overflow"))?;
            sessions.push(library.session(limits.invocation, maximum_calls, cancellation.clone()));
        }
        let scratch_count = self.spec.nodes.len() + super::MAX_ARGS;
        let scratch_reservation = memory.reserve_bytes(scratch_count * 4)?;
        let mut row_values = reserve_vec(self.spec.nodes.len())?;
        row_values.resize(self.spec.nodes.len(), 0.);
        let arguments = reserve_vec(super::MAX_ARGS)?;
        let attempts = reserve_vec(limits.maximum_attempts)?;
        let contract = StreamContract {
            abi: "fluxr-scalar-stream-contract-v1",
            graph_identity: self.identity.clone(),
            numeric_profile: NUMERIC_PROFILE,
            device: "cpu",
            limits,
            payload_budget_bytes: memory.limit(),
        };
        let contract_bytes =
            serde_json::to_vec(&contract).map_err(|error| fail(error.to_string()))?;
        let contract_identity = format!("{:x}", Sha256::digest(contract_bytes));
        Ok(ScalarStream {
            graph: self,
            sessions,
            row_values,
            arguments,
            _scratch_reservation: scratch_reservation,
            limits,
            memory,
            cancellation,
            statistics: StreamStatistics::default(),
            attempts,
            contract,
            contract_identity,
        })
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct StreamLimits {
    pub invocation: Limits,
    /// Rows admitted across chunks, including rows left unexecuted after a
    /// failed chunk. Failures never refund this work envelope.
    pub maximum_rows: u64,
    pub maximum_chunk_rows: usize,
    /// Bounded receipt storage. Once exhausted, no further request is admitted
    /// or executed; the explicit exhaustion error is outside this ledger.
    pub maximum_attempts: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct StreamContract {
    pub abi: &'static str,
    pub graph_identity: String,
    pub numeric_profile: &'static str,
    pub device: &'static str,
    pub limits: StreamLimits,
    pub payload_budget_bytes: usize,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct StreamStatistics {
    pub admitted_rows: u64,
    pub attempted_rows: u64,
    pub completed_rows: u64,
    pub published_rows: u64,
    pub attempted_calls: u64,
    pub completed_calls: u64,
    pub completed_chunks: u64,
    pub failed_chunks: u64,
    pub rejected_chunks: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChunkStatus {
    Completed,
    Failed,
    Rejected,
}

#[derive(Clone, Debug, Serialize)]
pub struct ChunkReceipt {
    pub abi: &'static str,
    pub graph_identity: String,
    pub stream_contract_identity: String,
    /// Captured only at at most 128 UTF-8 bytes. Longer rejected requests retain
    /// their byte length without hashing or copying an unbounded caller string.
    pub requested_graph: Option<String>,
    pub requested_graph_bytes: usize,
    pub attempt: u64,
    pub requested_sequence: u64,
    pub status: ChunkStatus,
    pub requested_rows: usize,
    pub input_sha256: Option<String>,
    pub output_sha256: Option<String>,
    pub attempted_rows: u64,
    pub completed_rows: u64,
    pub attempted_calls: u64,
    pub completed_calls: u64,
    pub first_failed_node: Option<usize>,
    pub failure: Option<String>,
}

#[derive(Debug)]
pub struct StreamChunk {
    pub output: Tensor,
    pub receipt: ChunkReceipt,
}

/// One stream owns each library's mutable evaluator workspace. Multiple streams
/// may share one immutable graph and payload budget; there is no shared mutable
/// model state or implicit queue. Dropping a failed chunk cannot publish it.
pub struct ScalarStream<'a> {
    graph: &'a ScalarGraph,
    sessions: Vec<ScalarSession<'a>>,
    row_values: Vec<f32>,
    arguments: Vec<f32>,
    _scratch_reservation: Reservation,
    limits: StreamLimits,
    memory: MemoryBudget,
    cancellation: Cancellation,
    statistics: StreamStatistics,
    attempts: Vec<ChunkReceipt>,
    contract: StreamContract,
    contract_identity: String,
}

impl ScalarStream<'_> {
    pub fn graph_identity(&self) -> &str {
        self.graph.identity()
    }

    pub fn contract(&self) -> &StreamContract {
        &self.contract
    }

    pub fn contract_identity(&self) -> &str {
        &self.contract_identity
    }

    pub fn next_sequence(&self) -> u64 {
        self.attempts.len() as u64
    }

    pub fn statistics(&self) -> &StreamStatistics {
        &self.statistics
    }

    pub fn attempts(&self) -> &[ChunkReceipt] {
        &self.attempts
    }

    /// Reject stale graph pins, duplicate/reordered sequence numbers, shape and
    /// work-envelope mismatches before execution. Each in-envelope request uses
    /// one receipt slot, including rejections; callers must inspect next_sequence
    /// after a failed request instead of silently retrying the same sequence.
    pub fn execute(
        &mut self,
        expected_graph: &str,
        sequence: u64,
        input: &Tensor,
    ) -> Result<StreamChunk, Error> {
        if self.attempts.len() == self.limits.maximum_attempts {
            return Err(fail("scalar stream attempt ledger exhausted"));
        }
        let attempt = self.next_sequence();
        let rows = input.shape().first().copied().unwrap_or(0);
        let before = self.statistics.clone();
        let mut receipt = ChunkReceipt {
            abi: "fluxr-scalar-stream-receipt-v1",
            graph_identity: self.graph.identity.clone(),
            stream_contract_identity: self.contract_identity.clone(),
            requested_graph: (expected_graph.len() <= 128).then(|| expected_graph.to_string()),
            requested_graph_bytes: expected_graph.len(),
            attempt,
            requested_sequence: sequence,
            status: ChunkStatus::Rejected,
            requested_rows: rows,
            input_sha256: None,
            output_sha256: None,
            attempted_rows: 0,
            completed_rows: 0,
            attempted_calls: 0,
            completed_calls: 0,
            first_failed_node: None,
            failure: None,
        };
        let result = self.execute_checked(expected_graph, sequence, input, &mut receipt);
        // Read actual call counters: cancellation can be observed by a library
        // between a row boundary and call entry, before the call is admitted.
        self.statistics.attempted_calls = self
            .sessions
            .iter()
            .map(ScalarSession::attempted_calls)
            .sum();
        self.statistics.completed_calls = self
            .sessions
            .iter()
            .map(ScalarSession::completed_calls)
            .sum();
        receipt.attempted_rows = self.statistics.attempted_rows - before.attempted_rows;
        receipt.completed_rows = self.statistics.completed_rows - before.completed_rows;
        receipt.attempted_calls = self.statistics.attempted_calls - before.attempted_calls;
        receipt.completed_calls = self.statistics.completed_calls - before.completed_calls;
        if let Err(error) = &result {
            receipt.failure = Some(error.to_string());
            match receipt.status {
                ChunkStatus::Rejected => self.statistics.rejected_chunks += 1,
                ChunkStatus::Failed => self.statistics.failed_chunks += 1,
                ChunkStatus::Completed => unreachable!("completed chunk cannot fail"),
            }
        }
        self.attempts.push(receipt.clone());
        result.map(|output| StreamChunk { output, receipt })
    }

    fn execute_checked(
        &mut self,
        expected_graph: &str,
        sequence: u64,
        input: &Tensor,
        receipt: &mut ChunkReceipt,
    ) -> Result<Tensor, Error> {
        if expected_graph != self.graph.identity() {
            return Err(fail("scalar stream graph identity mismatch"));
        }
        if sequence != receipt.attempt {
            return Err(fail("scalar stream stale or ambiguous sequence"));
        }
        let &[rows, columns] = input.shape() else {
            return Err(fail("scalar stream requires rank-two input"));
        };
        if columns != self.graph.input_columns() || rows > self.limits.maximum_chunk_rows {
            return Err(fail("scalar stream input shape or chunk bound mismatch"));
        }
        let rows_u64 = u64::try_from(rows).map_err(|_| fail("scalar stream row count overflow"))?;
        let admitted = self
            .statistics
            .admitted_rows
            .checked_add(rows_u64)
            .filter(|&count| count <= self.limits.maximum_rows)
            .ok_or_else(|| fail("scalar stream row budget exhausted"))?;
        self.cancellation.check()?;
        let output_count = rows
            .checked_mul(self.graph.output_columns())
            .ok_or_else(|| fail("scalar stream output extent overflow"))?;
        let output_bytes = output_count
            .checked_mul(4)
            .ok_or_else(|| fail("scalar stream output bytes overflow"))?;
        let reservation = self.memory.reserve_bytes(output_bytes)?;
        let mut output = reserve_vec(output_count)?;
        receipt.input_sha256 = Some(tensor_digest(input));
        receipt.status = ChunkStatus::Failed;
        self.statistics.admitted_rows = admitted;
        for row in 0..rows {
            self.cancellation.check()?;
            self.statistics.attempted_rows += 1;
            for (index, node) in self.graph.spec.nodes.iter().enumerate() {
                receipt.first_failed_node = Some(index);
                self.arguments.clear();
                for argument in &node.arguments {
                    self.arguments.push(match *argument {
                        Argument::Input { column } => input.values()[row * columns + column],
                        Argument::Constant { bits } => f32::from_bits(bits),
                        Argument::Node { node } => self.row_values[node],
                    });
                }
                self.row_values[index] =
                    self.sessions[node.library].call(&node.export, &self.arguments)?;
            }
            receipt.first_failed_node = None;
            for &index in &self.graph.spec.outputs {
                output.push(self.row_values[index]);
            }
            self.statistics.completed_rows += 1;
        }
        // Cancellation after complete synchronous execution is not a retroactive
        // rollback. The completed chunk is published once under its sequence.
        let output =
            Tensor::from_owned_reserved(&[rows, self.graph.output_columns()], output, reservation)?;
        receipt.output_sha256 = Some(tensor_digest(&output));
        receipt.status = ChunkStatus::Completed;
        self.statistics.completed_chunks += 1;
        self.statistics.published_rows += rows_u64;
        Ok(output)
    }
}

/// Hash the exact FTB1 representation without allocating a second payload.
pub fn tensor_digest(tensor: &Tensor) -> String {
    let mut digest = Sha256::new();
    digest.update(super::TENSOR_MAGIC);
    digest.update([tensor.shape().len() as u8, 1, 0, 0]);
    for &extent in tensor.shape() {
        digest.update((extent as u64).to_le_bytes());
    }
    for value in tensor.values() {
        digest.update(value.to_bits().to_le_bytes());
    }
    format!("{:x}", digest.finalize())
}
