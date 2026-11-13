//! Explicit loop-carried scalar state over the released checked graph evaluator.
//!
//! This additive protocol commits output and state together after an entire
//! chunk succeeds. It is a CPU worker/offline API, not a real-time callback.
pub mod checkpoint;

use super::{
    Argument, ChunkReceipt, ChunkStatus, GraphSpec, MAX_ATTEMPTS, MAX_COLUMNS, ScalarGraph,
    StreamLimits, StreamStatistics, tensor_digest,
};
use crate::Error;
use crate::cpu_compute::{
    Cancellation, MemoryBudget, NUMERIC_PROFILE, Reservation, ScalarLibrary, ScalarSession, Tensor,
    fail, finite, reserve_vec,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;

pub const STATEFUL_ABI: &str = "fluxr-stateful-scalar-graph-v1";
const MAX_SPEC_BYTES: usize = 72 * 1024;

/// The transition graph's inputs are external row columns followed by previous
/// state cells. Its ordinary outputs are published; state_outputs independently
/// selects node results copied simultaneously into the next row's state.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatefulSpec {
    pub abi: String,
    pub external_columns: usize,
    pub transition: GraphSpec,
    pub initial_state_bits: Vec<u32>,
    pub state_outputs: Vec<usize>,
}

fn encode_spec(spec: &StatefulSpec) -> Result<Vec<u8>, Error> {
    struct Bounded(Vec<u8>);
    impl std::io::Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > MAX_SPEC_BYTES - self.0.len() {
                return Err(std::io::Error::other(
                    "stateful graph canonical byte budget exceeded",
                ));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut result = Bounded(Vec::new());
    serde_json::to_writer(&mut result, spec).map_err(|error| fail(error.to_string()))?;
    Ok(result.0)
}

pub struct StatefulGraph {
    transition: ScalarGraph,
    spec: StatefulSpec,
    identity: String,
}

impl StatefulGraph {
    pub fn load(bytes: &[u8], libraries: &[Arc<ScalarLibrary>]) -> Result<Self, Error> {
        if bytes.len() > MAX_SPEC_BYTES {
            return Err(fail("stateful graph byte budget exceeded"));
        }
        let spec = serde_json::from_slice(bytes)
            .map_err(|error| fail(format!("invalid stateful graph: {error}")))?;
        Self::prepare(spec, libraries)
    }

    pub fn prepare(spec: StatefulSpec, libraries: &[Arc<ScalarLibrary>]) -> Result<Self, Error> {
        let count = spec.initial_state_bits.len();
        if spec.abi != STATEFUL_ABI
            || count == 0
            || count > MAX_COLUMNS
            || spec.external_columns > MAX_COLUMNS
            || spec.state_outputs.len() != count
            || spec.external_columns.checked_add(count) != Some(spec.transition.input_columns)
        {
            return Err(fail("unsupported stateful graph ABI or state shape"));
        }
        for &bits in &spec.initial_state_bits {
            finite(f32::from_bits(bits))?;
        }
        if spec
            .state_outputs
            .iter()
            .any(|&index| index >= spec.transition.nodes.len())
        {
            return Err(fail("stateful graph update node out of bounds"));
        }
        // Validate the inherited graph and canonical encoding before retaining a
        // second spec copy. The inner graph remains the sole compiled payload.
        let canonical = encode_spec(&spec)?;
        let transition = ScalarGraph::prepare(spec.transition.clone(), libraries)?;
        let mut hash = Sha256::new();
        hash.update(STATEFUL_ABI.as_bytes());
        hash.update([0]);
        hash.update((canonical.len() as u64).to_le_bytes());
        hash.update(canonical);
        hash.update(transition.identity().as_bytes());
        Ok(Self {
            transition,
            spec,
            identity: format!("{:x}", hash.finalize()),
        })
    }

    pub fn identity(&self) -> &str {
        &self.identity
    }
    pub fn input_columns(&self) -> usize {
        self.spec.external_columns
    }
    pub fn output_columns(&self) -> usize {
        self.transition.output_columns()
    }
    pub fn state_cells(&self) -> usize {
        self.spec.initial_state_bits.len()
    }
    pub fn node_count(&self) -> usize {
        self.transition.node_count()
    }
    pub fn encode(&self) -> Vec<u8> {
        encode_spec(&self.spec).expect("checked stateful graph serialization")
    }

    pub fn stream(
        &self,
        limits: StreamLimits,
        memory: MemoryBudget,
        cancellation: Cancellation,
    ) -> Result<StatefulStream<'_>, Error> {
        if limits.maximum_chunk_rows == 0
            || limits.maximum_attempts == 0
            || limits.maximum_attempts > MAX_ATTEMPTS
        {
            return Err(fail("stateful stream chunk or attempt budget is invalid"));
        }
        limits
            .maximum_rows
            .checked_mul(self.node_count() as u64)
            .ok_or_else(|| fail("stateful stream aggregate call budget overflow"))?;
        let mut sessions = Vec::with_capacity(self.transition.libraries.len());
        for (library, &count) in self
            .transition
            .libraries
            .iter()
            .zip(&self.transition.calls_per_row)
        {
            let calls = limits
                .maximum_rows
                .checked_mul(count)
                .ok_or_else(|| fail("stateful stream call budget overflow"))?;
            sessions.push(library.session(limits.invocation, calls, cancellation.clone()));
        }
        let cells = self.node_count() + crate::cpu_compute::MAX_ARGS + self.state_cells() * 2;
        let reservation = memory.reserve_bytes(cells * 4)?;
        let mut row_values = reserve_vec(self.node_count())?;
        row_values.resize(self.node_count(), 0.);
        let mut committed = reserve_vec(self.state_cells())?;
        committed.extend(
            self.spec
                .initial_state_bits
                .iter()
                .map(|&bits| f32::from_bits(bits)),
        );
        let mut tentative = reserve_vec(self.state_cells())?;
        tentative.extend_from_slice(&committed);
        let arguments = reserve_vec(crate::cpu_compute::MAX_ARGS)?;
        let attempts = reserve_vec(limits.maximum_attempts)?;
        let (contract, contract_identity) = self.contract(limits, memory.limit())?;
        let state_pin = StatePin {
            revision: 0,
            sha256: state_digest(self.identity(), &committed),
        };
        Ok(StatefulStream {
            graph: self,
            sessions,
            row_values,
            arguments,
            committed,
            tentative,
            _reservation: reservation,
            limits,
            memory,
            cancellation,
            statistics: StreamStatistics::default(),
            attempts,
            contract,
            contract_identity,
            state_pin,
        })
    }

    fn contract(
        &self,
        limits: StreamLimits,
        payload_budget_bytes: usize,
    ) -> Result<(StatefulContract, String), Error> {
        let contract = StatefulContract {
            abi: "fluxr-stateful-scalar-stream-contract-v1",
            graph_identity: self.identity.clone(),
            numeric_profile: NUMERIC_PROFILE,
            device: "cpu",
            limits,
            payload_budget_bytes,
            state_publication: "whole-chunk-atomic; simultaneous next-row updates; failed work not refunded",
        };
        let bytes = serde_json::to_vec(&contract).map_err(|error| fail(error.to_string()))?;
        let identity = format!("{:x}", Sha256::digest(bytes));
        Ok((contract, identity))
    }
}

/// Both fields must match. This bounded value identifies state within one live
/// stream; it is not a durable distributed session identity or authority token.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatePin {
    pub revision: u64,
    pub sha256: [u8; 32],
}

#[derive(Clone, Debug, Serialize)]
pub struct StatefulContract {
    pub abi: &'static str,
    pub graph_identity: String,
    pub numeric_profile: &'static str,
    pub device: &'static str,
    pub limits: StreamLimits,
    pub payload_budget_bytes: usize,
    pub state_publication: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct StatefulReceipt {
    pub abi: &'static str,
    pub execution: ChunkReceipt,
    pub requested_state: StatePin,
    pub state_before: StatePin,
    pub state_after: StatePin,
    pub state_committed: bool,
}

#[derive(Debug)]
pub struct StatefulChunk {
    pub output: Tensor,
    pub receipt: StatefulReceipt,
}

pub struct StatefulStream<'a> {
    graph: &'a StatefulGraph,
    sessions: Vec<ScalarSession<'a>>,
    row_values: Vec<f32>,
    arguments: Vec<f32>,
    committed: Vec<f32>,
    tentative: Vec<f32>,
    _reservation: Reservation,
    limits: StreamLimits,
    memory: MemoryBudget,
    cancellation: Cancellation,
    statistics: StreamStatistics,
    attempts: Vec<StatefulReceipt>,
    contract: StatefulContract,
    contract_identity: String,
    state_pin: StatePin,
}

impl StatefulStream<'_> {
    pub fn state(&self) -> &[f32] {
        &self.committed
    }
    pub fn state_pin(&self) -> StatePin {
        self.state_pin
    }
    pub fn next_sequence(&self) -> u64 {
        self.attempts.len() as u64
    }
    pub fn statistics(&self) -> &StreamStatistics {
        &self.statistics
    }
    pub fn attempts(&self) -> &[StatefulReceipt] {
        &self.attempts
    }
    pub fn contract(&self) -> &StatefulContract {
        &self.contract
    }
    pub fn contract_identity(&self) -> &str {
        &self.contract_identity
    }

    pub fn execute(
        &mut self,
        expected_graph: &str,
        expected_state: StatePin,
        sequence: u64,
        input: &Tensor,
    ) -> Result<StatefulChunk, Error> {
        if self.attempts.len() == self.limits.maximum_attempts {
            return Err(fail("stateful stream attempt ledger exhausted"));
        }
        let before = self.statistics.clone();
        let mut receipt = StatefulReceipt {
            abi: "fluxr-stateful-scalar-stream-receipt-v1",
            requested_state: expected_state,
            state_before: self.state_pin,
            state_after: self.state_pin,
            state_committed: false,
            execution: ChunkReceipt {
                abi: "fluxr-stateful-scalar-stream-execution-v1",
                graph_identity: self.graph.identity.clone(),
                stream_contract_identity: self.contract_identity.clone(),
                requested_graph: (expected_graph.len() <= 128).then(|| expected_graph.to_string()),
                requested_graph_bytes: expected_graph.len(),
                attempt: self.next_sequence(),
                requested_sequence: sequence,
                status: ChunkStatus::Rejected,
                requested_rows: input.shape().first().copied().unwrap_or(0),
                input_sha256: None,
                output_sha256: None,
                attempted_rows: 0,
                completed_rows: 0,
                attempted_calls: 0,
                completed_calls: 0,
                first_failed_node: None,
                failure: None,
            },
        };
        let result = self.execute_checked(
            expected_graph,
            expected_state,
            sequence,
            input,
            &mut receipt,
        );
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
        let execution = &mut receipt.execution;
        execution.attempted_rows = self.statistics.attempted_rows - before.attempted_rows;
        execution.completed_rows = self.statistics.completed_rows - before.completed_rows;
        execution.attempted_calls = self.statistics.attempted_calls - before.attempted_calls;
        execution.completed_calls = self.statistics.completed_calls - before.completed_calls;
        if let Err(error) = &result {
            execution.failure = Some(error.to_string());
            match execution.status {
                ChunkStatus::Rejected => self.statistics.rejected_chunks += 1,
                ChunkStatus::Failed => self.statistics.failed_chunks += 1,
                ChunkStatus::Completed => unreachable!("completed stateful chunk cannot fail"),
            }
        }
        self.attempts.push(receipt.clone());
        result.map(|output| StatefulChunk { output, receipt })
    }

    fn execute_checked(
        &mut self,
        expected_graph: &str,
        expected_state: StatePin,
        sequence: u64,
        input: &Tensor,
        receipt: &mut StatefulReceipt,
    ) -> Result<Tensor, Error> {
        if expected_graph != self.graph.identity() {
            return Err(fail("stateful stream graph identity mismatch"));
        }
        if expected_state != self.state_pin {
            return Err(fail("stateful stream stale state revision or digest"));
        }
        if sequence != receipt.execution.attempt {
            return Err(fail("stateful stream stale or ambiguous sequence"));
        }
        let &[rows, columns] = input.shape() else {
            return Err(fail("stateful stream requires rank-two input"));
        };
        if columns != self.graph.input_columns() || rows > self.limits.maximum_chunk_rows {
            return Err(fail("stateful stream input shape or chunk bound mismatch"));
        }
        let rows_u64 =
            u64::try_from(rows).map_err(|_| fail("stateful stream row count overflow"))?;
        let admitted = self
            .statistics
            .admitted_rows
            .checked_add(rows_u64)
            .filter(|&count| count <= self.limits.maximum_rows)
            .ok_or_else(|| fail("stateful stream row budget exhausted"))?;
        let next_revision = self
            .state_pin
            .revision
            .checked_add(1)
            .ok_or_else(|| fail("stateful stream revision exhausted"))?;
        self.cancellation.check()?;
        let output_count = rows
            .checked_mul(self.graph.output_columns())
            .ok_or_else(|| fail("stateful stream output extent overflow"))?;
        let output_bytes = output_count
            .checked_mul(4)
            .ok_or_else(|| fail("stateful stream output byte count overflow"))?;
        let reservation = self.memory.reserve_bytes(output_bytes)?;
        let mut output = reserve_vec(output_count)?;
        receipt.execution.input_sha256 = Some(tensor_digest(input));
        receipt.execution.status = ChunkStatus::Failed;
        self.statistics.admitted_rows = admitted;
        self.tentative.copy_from_slice(&self.committed);
        for row in 0..rows {
            self.cancellation.check()?;
            self.statistics.attempted_rows += 1;
            for (index, node) in self.graph.transition.spec.nodes.iter().enumerate() {
                receipt.execution.first_failed_node = Some(index);
                self.arguments.clear();
                for argument in &node.arguments {
                    self.arguments.push(match *argument {
                        Argument::Input { column } if column < columns => {
                            input.values()[row * columns + column]
                        }
                        Argument::Input { column } => self.tentative[column - columns],
                        Argument::Constant { bits } => f32::from_bits(bits),
                        Argument::Node { node } => self.row_values[node],
                    });
                }
                self.row_values[index] =
                    self.sessions[node.library].call(&node.export, &self.arguments)?;
            }
            receipt.execution.first_failed_node = None;
            for &index in &self.graph.transition.spec.outputs {
                output.push(self.row_values[index]);
            }
            // Every state expression observed the same previous state. Updating
            // these slots cannot affect the already-computed node results.
            for (slot, &index) in self
                .tentative
                .iter_mut()
                .zip(&self.graph.spec.state_outputs)
            {
                *slot = self.row_values[index];
            }
            self.statistics.completed_rows += 1;
        }
        let output =
            Tensor::from_owned_reserved(&[rows, self.graph.output_columns()], output, reservation)?;
        let output_sha256 = tensor_digest(&output);
        let next_pin = StatePin {
            revision: next_revision,
            sha256: state_digest(self.graph.identity(), &self.tentative),
        };
        // Observe cancellation after payload hashing as well as at row/call
        // boundaries. A refusal here drops the fully computed output but does
        // not commit state or pretend its completed rows were published.
        self.cancellation.check()?;
        receipt.execution.output_sha256 = Some(output_sha256);
        // No fallible operation follows publication. Later cancellation is not
        // a retroactive rollback.
        std::mem::swap(&mut self.committed, &mut self.tentative);
        self.state_pin = next_pin;
        receipt.state_after = next_pin;
        receipt.state_committed = true;
        receipt.execution.status = ChunkStatus::Completed;
        self.statistics.completed_chunks += 1;
        self.statistics.published_rows += rows_u64;
        Ok(output)
    }
}

fn state_digest(graph: &str, values: &[f32]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"fluxr-stateful-scalar-state-v1\0");
    hash.update(graph.as_bytes());
    hash.update((values.len() as u64).to_le_bytes());
    for value in values {
        hash.update(value.to_bits().to_le_bytes());
    }
    hash.finalize().into()
}
