//! Bounded, portable stateful stream checkpoints.
//!
//! Checkpoints retain the complete attempt ledger and charged work. They are
//! data, not executable modules, attestations or authorization. Resume creates a
//! new stream with the exact old graph and policy; it does not replace a live
//! instance or prevent replay/forking. All operations are worker/offline only.
use super::{
    ChunkReceipt, ChunkStatus, MAX_ATTEMPTS, StatePin, StatefulGraph, StatefulReceipt,
    StatefulStream, StreamLimits, StreamStatistics, state_digest,
};
use crate::Error;
use crate::cpu_compute::{Cancellation, MemoryBudget, fail, finite, reserve_vec};
use sha2::{Digest, Sha256};

pub const CHECKPOINT_ABI: &str = "fluxr-stateful-checkpoint-v1";
pub const MAX_CHECKPOINT_BYTES: usize = 64 * 1024 * 1024;
const MAGIC: &[u8; 8] = b"FSC1\r\n\x1a\n";
const MAX_FAILURE_BYTES: usize = 16 * 1024;
const MIN_RECEIPT_BYTES: usize = 191;

/// Trusted caller admission. The expected hash must come from outside the
/// untrusted checkpoint. It identifies bytes; it does not authenticate history.
#[derive(Clone, Copy, Debug)]
pub struct CheckpointAdmission {
    pub maximum_bytes: usize,
    pub expected_sha256: [u8; 32],
}

/// Separately bounded metadata owned by the caller. These bytes do not consume
/// the tensor payload budget. Encoding/decoding can temporarily retain multiple
/// bounded metadata buffers; neither budget is a process RSS limit.
#[derive(Debug)]
pub struct StatefulCheckpoint {
    bytes: Vec<u8>,
    sha256: [u8; 32],
}
impl StatefulCheckpoint {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
}

fn byte_bound(maximum: usize) -> Result<(), Error> {
    if maximum == 0 || maximum > MAX_CHECKPOINT_BYTES {
        Err(fail("checkpoint byte limit must be in 1..=64 MiB"))
    } else {
        Ok(())
    }
}
fn size64(value: usize) -> Result<u64, Error> {
    u64::try_from(value).map_err(|_| fail("checkpoint size does not fit u64"))
}
fn policy(limits: StreamLimits, payload: usize) -> Result<[u64; 7], Error> {
    Ok([
        limits.invocation.instructions,
        size64(limits.invocation.cells)?,
        size64(limits.invocation.frames)?,
        limits.maximum_rows,
        size64(limits.maximum_chunk_rows)?,
        size64(limits.maximum_attempts)?,
        size64(payload)?,
    ])
}
fn statistics_values(s: &StreamStatistics) -> [u64; 9] {
    [
        s.admitted_rows,
        s.attempted_rows,
        s.completed_rows,
        s.published_rows,
        s.attempted_calls,
        s.completed_calls,
        s.completed_chunks,
        s.failed_chunks,
        s.rejected_chunks,
    ]
}
fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
struct Writer {
    bytes: Vec<u8>,
    maximum: usize,
    written: usize,
    measure_only: bool,
}
impl Writer {
    fn put(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let next = self
            .written
            .checked_add(bytes.len())
            .filter(|&n| n <= self.maximum)
            .ok_or_else(|| fail("checkpoint byte budget exceeded"))?;
        if !self.measure_only {
            // The immutable sizing pass reserved this exact byte extent.
            require(next <= self.bytes.capacity(), "checkpoint sizing changed")?;
            self.bytes.extend_from_slice(bytes);
        }
        self.written = next;
        Ok(())
    }
    fn u64(&mut self, value: u64) -> Result<(), Error> {
        self.put(&value.to_le_bytes())
    }
    fn flag(&mut self, value: bool) -> Result<(), Error> {
        self.put(&[u8::from(value)])
    }
    fn hash(&mut self, value: &str) -> Result<(), Error> {
        if !valid_hash(value) {
            return Err(fail("checkpoint hash is not lowercase SHA-256"));
        }
        self.put(value.as_bytes())
    }
    fn optional_hash(&mut self, value: &Option<String>) -> Result<(), Error> {
        self.flag(value.is_some())?;
        if let Some(value) = value {
            self.hash(value)?;
        }
        Ok(())
    }
    fn text(&mut self, value: &Option<String>, maximum: usize) -> Result<(), Error> {
        self.flag(value.is_some())?;
        if let Some(value) = value {
            if value.len() > maximum {
                return Err(fail("checkpoint text field exceeds its byte bound"));
            }
            self.put(&(value.len() as u32).to_le_bytes())?;
            self.put(value.as_bytes())?;
        }
        Ok(())
    }
    fn pin(&mut self, value: StatePin) -> Result<(), Error> {
        self.u64(value.revision)?;
        self.put(&value.sha256)
    }
    fn receipt(&mut self, value: &StatefulReceipt) -> Result<(), Error> {
        let e = &value.execution;
        self.pin(value.requested_state)?;
        self.pin(value.state_before)?;
        self.pin(value.state_after)?;
        self.flag(value.state_committed)?;
        self.text(&e.requested_graph, 128)?;
        self.u64(size64(e.requested_graph_bytes)?)?;
        self.u64(e.attempt)?;
        self.u64(e.requested_sequence)?;
        self.put(&[match e.status {
            ChunkStatus::Completed => 0,
            ChunkStatus::Failed => 1,
            ChunkStatus::Rejected => 2,
        }])?;
        self.u64(size64(e.requested_rows)?)?;
        self.optional_hash(&e.input_sha256)?;
        self.optional_hash(&e.output_sha256)?;
        for n in [
            e.attempted_rows,
            e.completed_rows,
            e.attempted_calls,
            e.completed_calls,
        ] {
            self.u64(n)?;
        }
        self.flag(e.first_failed_node.is_some())?;
        if let Some(node) = e.first_failed_node {
            self.u64(size64(node)?)?;
        }
        self.text(&e.failure, MAX_FAILURE_BYTES)
    }
}
struct Reader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], Error> {
        let end = self
            .cursor
            .checked_add(count)
            .filter(|&n| n <= self.bytes.len())
            .ok_or_else(|| fail("truncated checkpoint"))?;
        let result = &self.bytes[self.cursor..end];
        self.cursor = end;
        Ok(result)
    }
    fn byte(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }
    fn flag(&mut self) -> Result<bool, Error> {
        match self.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(fail("invalid checkpoint Boolean")),
        }
    }
    fn u16(&mut self) -> Result<usize, Error> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()) as usize)
    }
    fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, Error> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn size(&mut self) -> Result<usize, Error> {
        usize::try_from(self.u64()?).map_err(|_| fail("checkpoint size exceeds this platform"))
    }
    fn hash(&mut self) -> Result<String, Error> {
        let value = std::str::from_utf8(self.take(64)?)
            .map_err(|_| fail("invalid checkpoint hash UTF-8"))?;
        if !valid_hash(value) {
            return Err(fail("invalid checkpoint hash"));
        }
        Ok(value.to_string())
    }
    fn optional_hash(&mut self) -> Result<Option<String>, Error> {
        if self.flag()? {
            Ok(Some(self.hash()?))
        } else {
            Ok(None)
        }
    }
    fn text(&mut self, maximum: usize) -> Result<Option<String>, Error> {
        if !self.flag()? {
            return Ok(None);
        }
        let count = self.u32()? as usize;
        if count > maximum {
            return Err(fail("checkpoint text field exceeds its byte bound"));
        }
        let value = std::str::from_utf8(self.take(count)?)
            .map_err(|_| fail("invalid checkpoint text UTF-8"))?;
        Ok(Some(value.to_string()))
    }
    fn pin(&mut self) -> Result<StatePin, Error> {
        Ok(StatePin {
            revision: self.u64()?,
            sha256: self.take(32)?.try_into().unwrap(),
        })
    }
    fn receipt(
        &mut self,
        graph: &str,
        contract: &str,
        node_count: usize,
    ) -> Result<StatefulReceipt, Error> {
        let requested_state = self.pin()?;
        let state_before = self.pin()?;
        let state_after = self.pin()?;
        let state_committed = self.flag()?;
        let requested_graph = self.text(128)?;
        let requested_graph_bytes = self.size()?;
        let attempt = self.u64()?;
        let requested_sequence = self.u64()?;
        let status = match self.byte()? {
            0 => ChunkStatus::Completed,
            1 => ChunkStatus::Failed,
            2 => ChunkStatus::Rejected,
            _ => return Err(fail("unknown checkpoint chunk status")),
        };
        let requested_rows = self.size()?;
        let input_sha256 = self.optional_hash()?;
        let output_sha256 = self.optional_hash()?;
        let attempted_rows = self.u64()?;
        let completed_rows = self.u64()?;
        let attempted_calls = self.u64()?;
        let completed_calls = self.u64()?;
        let first_failed_node = if self.flag()? {
            let node = self.size()?;
            if node >= node_count {
                return Err(fail("checkpoint failed node is out of bounds"));
            }
            Some(node)
        } else {
            None
        };
        let failure = self.text(MAX_FAILURE_BYTES)?;
        Ok(StatefulReceipt {
            abi: "fluxr-stateful-scalar-stream-receipt-v1",
            requested_state,
            state_before,
            state_after,
            state_committed,
            execution: ChunkReceipt {
                abi: "fluxr-stateful-scalar-stream-execution-v1",
                graph_identity: graph.to_string(),
                stream_contract_identity: contract.to_string(),
                requested_graph,
                requested_graph_bytes,
                attempt,
                requested_sequence,
                status,
                requested_rows,
                input_sha256,
                output_sha256,
                attempted_rows,
                completed_rows,
                attempted_calls,
                completed_calls,
                first_failed_node,
                failure,
            },
        })
    }
}

impl StatefulStream<'_> {
    /// Export exact state, policy and the complete attempt ledger. Insufficient
    /// byte budget refuses; it never drops failed attempts or truncates history.
    /// A cancelled stream may be exported for audit, but cannot be resumed.
    pub fn checkpoint(&self, maximum_bytes: usize) -> Result<StatefulCheckpoint, Error> {
        byte_bound(maximum_bytes)?;
        let mut measurement = Writer {
            bytes: Vec::new(),
            maximum: maximum_bytes,
            written: 0,
            measure_only: true,
        };
        self.write_checkpoint(&mut measurement)?;
        let mut writer = Writer {
            bytes: reserve_vec(measurement.written)?,
            maximum: maximum_bytes,
            written: 0,
            measure_only: false,
        };
        self.write_checkpoint(&mut writer)?;
        let length = size64(writer.bytes.len())?;
        writer.bytes[8..16].copy_from_slice(&length.to_le_bytes());
        let sha256 = Sha256::digest(&writer.bytes).into();
        Ok(StatefulCheckpoint {
            bytes: writer.bytes,
            sha256,
        })
    }

    fn write_checkpoint(&self, writer: &mut Writer) -> Result<(), Error> {
        writer.put(MAGIC)?;
        writer.u64(0)?; // Total length filled after all bounded writes.
        writer.hash(self.graph.identity())?;
        writer.hash(self.contract_identity())?;
        for n in policy(self.limits, self.memory.limit())? {
            writer.u64(n)?;
        }
        writer.pin(self.state_pin)?;
        writer.put(&(self.committed.len() as u16).to_le_bytes())?;
        for value in &self.committed {
            writer.put(&value.to_bits().to_le_bytes())?;
        }
        for n in statistics_values(&self.statistics) {
            writer.u64(n)?;
        }
        writer.put(&(self.sessions.len() as u16).to_le_bytes())?;
        for session in &self.sessions {
            writer.hash(session.library.identity())?;
            writer.hash(&session.library.manifest().source_sha256)?;
            writer.u64(session.attempted)?;
            writer.u64(session.completed)?;
            writer.u64(session.remaining)?;
        }
        writer.put(&(self.attempts.len() as u32).to_le_bytes())?;
        for receipt in &self.attempts {
            writer.receipt(receipt)?;
        }
        // State/counters are immutably borrowed. Cancellation alone can change
        // concurrently; capture its monotone flag at the final encoding boundary.
        writer.flag(self.cancellation.is_cancelled())
    }
}

fn add(target: &mut u64, n: u64) -> Result<(), Error> {
    *target = target
        .checked_add(n)
        .ok_or_else(|| fail("checkpoint counter overflow"))?;
    Ok(())
}
fn require(condition: bool, message: &str) -> Result<(), Error> {
    if condition {
        Ok(())
    } else {
        Err(fail(message))
    }
}

impl StatefulGraph {
    /// Resume an explicitly admitted byte identity with unchanged graph/policy.
    /// All input validation precedes construction of the returned fresh stream.
    /// No live stream, supplied cancellation handle or caller input is mutated
    /// on refusal. Restoring bytes is not anti-replay or authorization.
    pub fn resume_checkpoint(
        &self,
        bytes: &[u8],
        admission: CheckpointAdmission,
        limits: StreamLimits,
        memory: MemoryBudget,
        cancellation: Cancellation,
    ) -> Result<StatefulStream<'_>, Error> {
        byte_bound(admission.maximum_bytes)?;
        require(
            bytes.len() <= admission.maximum_bytes,
            "checkpoint byte budget exceeded",
        )?;
        cancellation.check()?;
        require(
            limits.maximum_chunk_rows > 0
                && limits.maximum_attempts > 0
                && limits.maximum_attempts <= MAX_ATTEMPTS,
            "checkpoint stream limits are invalid",
        )?;
        limits
            .maximum_rows
            .checked_mul(self.node_count() as u64)
            .ok_or_else(|| fail("checkpoint aggregate call budget overflow"))?;
        let mut reader = Reader { bytes, cursor: 0 };
        require(reader.take(8)? == MAGIC, "unsupported checkpoint ABI")?;
        require(reader.size()? == bytes.len(), "checkpoint length mismatch")?;
        let actual_digest: [u8; 32] = Sha256::digest(bytes).into();
        require(
            actual_digest == admission.expected_sha256,
            "checkpoint expected digest mismatch",
        )?;
        let (_, contract_identity) = self.contract(limits, memory.limit())?;
        require(
            reader.hash()? == self.identity(),
            "checkpoint graph mismatch",
        )?;
        require(
            reader.hash()? == contract_identity,
            "checkpoint stream contract mismatch",
        )?;
        for expected in policy(limits, memory.limit())? {
            require(reader.u64()? == expected, "checkpoint policy mismatch")?;
        }
        let final_pin = reader.pin()?;
        let state_count = reader.u16()?;
        require(
            state_count == self.state_cells(),
            "checkpoint state shape mismatch",
        )?;
        // Count and exact remaining bytes are checked before allocation.
        require(
            reader.bytes.len() - reader.cursor >= state_count * 4,
            "truncated checkpoint state",
        )?;
        let mut state = reserve_vec(state_count)?;
        for _ in 0..state_count {
            state.push(finite(f32::from_bits(reader.u32()?))?);
        }
        require(
            state_digest(self.identity(), &state) == final_pin.sha256,
            "checkpoint state digest mismatch",
        )?;
        let mut declared_statistics = [0u64; 9];
        for value in &mut declared_statistics {
            *value = reader.u64()?;
        }
        let library_count = reader.u16()?;
        require(
            library_count == self.transition.libraries.len(),
            "checkpoint library count mismatch",
        )?;
        let mut sessions = reserve_vec(library_count)?;
        for (library, &per_row) in self
            .transition
            .libraries
            .iter()
            .zip(&self.transition.calls_per_row)
        {
            require(
                reader.hash()? == library.identity()
                    && reader.hash()? == library.manifest().source_sha256,
                "checkpoint library or source identity mismatch",
            )?;
            let attempted = reader.u64()?;
            let completed = reader.u64()?;
            let remaining = reader.u64()?;
            let maximum = limits
                .maximum_rows
                .checked_mul(per_row)
                .ok_or_else(|| fail("checkpoint library budget overflow"))?;
            require(
                completed <= attempted && attempted.checked_add(remaining) == Some(maximum),
                "checkpoint library budget inconsistency",
            )?;
            sessions.push((attempted, completed, remaining));
        }
        let count = reader.u32()? as usize;
        require(
            count <= limits.maximum_attempts
                && count <= (reader.bytes.len() - reader.cursor) / MIN_RECEIPT_BYTES,
            "checkpoint attempt count or encoded ledger bound exceeded",
        )?;
        let mut attempts = reserve_vec(count)?;
        let mut statistics = StreamStatistics::default();
        let initial_state: Vec<f32> = self
            .spec
            .initial_state_bits
            .iter()
            .map(|&bits| f32::from_bits(bits))
            .collect();
        let mut pin = StatePin {
            revision: 0,
            sha256: state_digest(self.identity(), &initial_state),
        };
        let mut attempted_by_library = vec![0u64; library_count];
        let mut completed_by_library = vec![0u64; library_count];
        for index in 0..count {
            cancellation.check()?;
            let receipt = reader.receipt(self.identity(), &contract_identity, self.node_count())?;
            self.validate_checkpoint_receipt(
                &receipt,
                index,
                pin,
                limits,
                &mut statistics,
                &mut attempted_by_library,
                &mut completed_by_library,
            )?;
            pin = receipt.state_after;
            attempts.push(receipt);
        }
        require(!reader.flag()?, "cancelled checkpoint cannot resume")?;
        require(reader.cursor == bytes.len(), "trailing checkpoint bytes")?;
        require(pin == final_pin, "checkpoint final state chain mismatch")?;
        require(
            statistics_values(&statistics) == declared_statistics,
            "checkpoint aggregate statistics mismatch",
        )?;
        require(
            statistics.admitted_rows <= limits.maximum_rows,
            "checkpoint admitted row budget exceeded",
        )?;
        for (index, &(attempted, completed, _)) in sessions.iter().enumerate() {
            require(
                attempted == attempted_by_library[index]
                    && completed == completed_by_library[index],
                "checkpoint per-library ledger mismatch",
            )?;
        }
        cancellation.check()?;
        // This allocation may fail under retained-output pressure. No existing
        // stream or tensor is replaced, and rollback drops every fresh resource.
        let mut stream = self.stream(limits, memory, cancellation.clone())?;
        stream.committed.copy_from_slice(&state);
        stream.tentative.copy_from_slice(&state);
        stream.state_pin = final_pin;
        stream.statistics = statistics;
        // Keep stream()'s complete ledger reservation; resume must not add a
        // new metadata growth point to the next admitted execute request.
        stream.attempts.extend(attempts);
        for (session, (attempted, completed, remaining)) in stream.sessions.iter_mut().zip(sessions)
        {
            session.attempted = attempted;
            session.completed = completed;
            session.remaining = remaining;
        }
        cancellation.check()?;
        Ok(stream)
    }

    #[allow(clippy::too_many_arguments)]
    fn validate_checkpoint_receipt(
        &self,
        receipt: &StatefulReceipt,
        index: usize,
        before: StatePin,
        limits: StreamLimits,
        statistics: &mut StreamStatistics,
        attempted_by_library: &mut [u64],
        completed_by_library: &mut [u64],
    ) -> Result<(), Error> {
        let e = &receipt.execution;
        require(
            e.attempt == index as u64 && receipt.state_before == before,
            "checkpoint attempt or state chain mismatch",
        )?;
        require(
            match &e.requested_graph {
                Some(value) => value.len() == e.requested_graph_bytes,
                None => e.requested_graph_bytes > 128,
            },
            "checkpoint requested graph length mismatch",
        )?;
        require(
            e.completed_rows <= e.attempted_rows && e.completed_calls <= e.attempted_calls,
            "checkpoint completed work exceeds attempted work",
        )?;
        let rows = size64(e.requested_rows)?;
        match e.status {
            ChunkStatus::Rejected => {
                require(
                    !receipt.state_committed
                        && receipt.state_after == before
                        && e.input_sha256.is_none()
                        && e.output_sha256.is_none()
                        && e.first_failed_node.is_none()
                        && e.attempted_rows == 0
                        && e.completed_rows == 0
                        && e.attempted_calls == 0
                        && e.completed_calls == 0
                        && e.failure.as_ref().is_some_and(|s| !s.is_empty()),
                    "checkpoint rejected receipt is inconsistent",
                )?;
                add(&mut statistics.rejected_chunks, 1)?;
                return Ok(());
            }
            ChunkStatus::Completed | ChunkStatus::Failed => {
                require(
                    e.requested_graph.as_deref() == Some(self.identity())
                        && e.requested_sequence == index as u64
                        && receipt.requested_state == before
                        && e.requested_rows <= limits.maximum_chunk_rows
                        && e.input_sha256.is_some()
                        && e.attempted_rows <= rows,
                    "checkpoint admitted receipt is inconsistent",
                )?;
                add(&mut statistics.admitted_rows, rows)?;
            }
        }
        let base_calls = e
            .completed_rows
            .checked_mul(self.node_count() as u64)
            .ok_or_else(|| fail("checkpoint row call count overflow"))?;
        let (prefix, failed_charged) = match e.status {
            ChunkStatus::Completed => {
                require(
                    receipt.state_committed
                        && receipt.state_after.revision
                            == before
                                .revision
                                .checked_add(1)
                                .ok_or_else(|| fail("checkpoint state revision overflow"))?
                        && (rows != 0 || receipt.state_after.sha256 == before.sha256)
                        && e.attempted_rows == rows
                        && e.completed_rows == rows
                        && e.attempted_calls == base_calls
                        && e.completed_calls == base_calls
                        && e.output_sha256.is_some()
                        && e.first_failed_node.is_none()
                        && e.failure.is_none(),
                    "checkpoint completed receipt is inconsistent",
                )?;
                add(&mut statistics.completed_chunks, 1)?;
                add(&mut statistics.published_rows, rows)?;
                (0, false)
            }
            ChunkStatus::Failed => {
                require(
                    !receipt.state_committed
                        && receipt.state_after == before
                        && e.output_sha256.is_none()
                        && e.failure.as_ref().is_some_and(|s| !s.is_empty()),
                    "checkpoint failed receipt is inconsistent",
                )?;
                add(&mut statistics.failed_chunks, 1)?;
                if let Some(node) = e.first_failed_node {
                    let completed = base_calls
                        .checked_add(node as u64)
                        .ok_or_else(|| fail("checkpoint failed prefix overflow"))?;
                    require(
                        e.attempted_rows
                            == e.completed_rows
                                .checked_add(1)
                                .ok_or_else(|| fail("checkpoint attempted row overflow"))?
                            && e.completed_calls == completed
                            // With this graph's whole-row admission, a library
                            // cannot exhaust its call envelope mid-row. The only
                            // uncharged call refusal is monotone cancellation,
                            // whose checkpoint is not resumable.
                            && completed.checked_add(1) == Some(e.attempted_calls),
                        "checkpoint failed call prefix is inconsistent",
                    )?;
                    (node, true)
                } else {
                    // The checked output extent and finite node results make
                    // post-row tensor validation infallible for a live stream.
                    // A failure outside a node therefore observed cancellation
                    // at a row/final boundary and cannot resume as uncancelled.
                    return Err(fail("checkpoint boundary failure cannot resume"));
                }
            }
            ChunkStatus::Rejected => unreachable!(),
        };
        for (library, &per_row) in self.transition.calls_per_row.iter().enumerate() {
            let calls = e
                .completed_rows
                .checked_mul(per_row)
                .ok_or_else(|| fail("checkpoint per-library call overflow"))?;
            add(&mut attempted_by_library[library], calls)?;
            add(&mut completed_by_library[library], calls)?;
        }
        for node in self.transition.spec.nodes.iter().take(prefix) {
            add(&mut attempted_by_library[node.library], 1)?;
            add(&mut completed_by_library[node.library], 1)?;
        }
        if failed_charged {
            let library = self.transition.spec.nodes[prefix].library;
            add(&mut attempted_by_library[library], 1)?;
        }
        add(&mut statistics.attempted_rows, e.attempted_rows)?;
        add(&mut statistics.completed_rows, e.completed_rows)?;
        add(&mut statistics.attempted_calls, e.attempted_calls)?;
        add(&mut statistics.completed_calls, e.completed_calls)?;
        Ok(())
    }
}
