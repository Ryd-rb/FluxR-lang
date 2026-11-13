//! Checked CPU data and independently loaded FluxR scalar programs.
//!
//! `fluxr-compute-v1` is additive: existing FSM bytes and language semantics are
//! unchanged. Payload budgets account owned array bytes, not allocator overhead,
//! code, host inputs, VM cells, or process RSS. Invocation limits bound the latter
//! separately. CPU placement is the only implemented device route.
pub mod stream;

use crate::Error;
use crate::structured::{Invocation, Limits, Module, Source, Type, Value, Workspace};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

const MAX_RANK: usize = 8;
const MAX_PACKAGE_BYTES: usize = 256 * 1024;
const MAX_MANIFEST_BYTES: usize = 16 * 1024;
const MAX_EXPORTS: usize = 128;
const MAX_ARGS: usize = 32;
const TENSOR_MAGIC: &[u8; 4] = b"FTB1";
pub const ABI: &str = "fluxr-scalar-library-v1";
pub const NUMERIC_PROFILE: &str = "fsm2-finite-f32-portable-v1";

fn fail(message: impl Into<String>) -> Error {
    Error(message.into())
}
fn finite(value: f32) -> Result<f32, Error> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(fail("compute v1 requires finite f32"))
    }
}
fn reserve_vec<T>(count: usize) -> Result<Vec<T>, Error> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| fail("compute allocation failed"))?;
    Ok(values)
}

#[derive(Debug)]
struct BudgetState {
    limit: usize,
    live: AtomicUsize,
    peak: AtomicUsize,
}
/// A process-local shared limit on owned array payload bytes.
#[derive(Clone, Debug)]
pub struct MemoryBudget(Arc<BudgetState>);
#[derive(Debug)]
pub(crate) struct Reservation {
    budget: MemoryBudget,
    bytes: usize,
}
impl MemoryBudget {
    pub fn new(limit: usize) -> Self {
        Self(Arc::new(BudgetState {
            limit,
            live: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        }))
    }
    pub fn limit(&self) -> usize {
        self.0.limit
    }
    pub fn live_bytes(&self) -> usize {
        self.0.live.load(Ordering::Acquire)
    }
    pub fn peak_bytes(&self) -> usize {
        self.0.peak.load(Ordering::Acquire)
    }
    pub(crate) fn reserve_bytes(&self, bytes: usize) -> Result<Reservation, Error> {
        let mut old = self.0.live.load(Ordering::Acquire);
        loop {
            let next = old
                .checked_add(bytes)
                .filter(|&next| next <= self.0.limit)
                .ok_or_else(|| fail("compute payload memory budget exhausted"))?;
            match self
                .0
                .live
                .compare_exchange_weak(old, next, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => break,
                Err(observed) => old = observed,
            }
        }
        self.0.peak.fetch_max(old + bytes, Ordering::AcqRel);
        Ok(Reservation {
            budget: self.clone(),
            bytes,
        })
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget.0.live.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

/// Cooperative cancellation is checked at row/call/block boundaries. An already
/// entered FluxR call completes or hits its finite instruction/frame/cell limit.
#[derive(Clone, Default, Debug)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    pub fn check(&self) -> Result<(), Error> {
        if self.is_cancelled() {
            Err(fail("compute cancelled"))
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Device {
    Cpu,
}
#[derive(Debug)]
struct TensorStorage {
    values: Vec<f32>,
    _reservation: Reservation,
}
/// Immutable row-major dense binary32 tensor. Clones share ownership and charge
/// payload once; there is no mutable view or hidden copy-on-write path.
#[derive(Clone, Debug)]
pub struct Tensor {
    shape: Arc<[usize]>,
    storage: Arc<TensorStorage>,
}
impl Tensor {
    pub fn new(shape: &[usize], values: &[f32], memory: &MemoryBudget) -> Result<Self, Error> {
        let len = elements(shape)?;
        if len != values.len() {
            return Err(fail("tensor shape/data length mismatch"));
        }
        for &value in values {
            finite(value)?;
        }
        let bytes = len
            .checked_mul(4)
            .ok_or_else(|| fail("tensor byte count overflow"))?;
        let reservation = memory.reserve_bytes(bytes)?;
        let mut owned = reserve_vec(len)?;
        owned.extend_from_slice(values);
        Ok(Self {
            shape: shape.into(),
            storage: Arc::new(TensorStorage {
                values: owned,
                _reservation: reservation,
            }),
        })
    }
    pub(crate) fn from_owned_reserved(
        shape: &[usize],
        values: Vec<f32>,
        reservation: Reservation,
    ) -> Result<Self, Error> {
        if elements(shape)? != values.len()
            || values.len().checked_mul(4) != Some(reservation.bytes)
        {
            return Err(fail("owned tensor reservation mismatch"));
        }
        for &value in &values {
            finite(value)?;
        }
        Ok(Self {
            shape: shape.into(),
            storage: Arc::new(TensorStorage {
                values,
                _reservation: reservation,
            }),
        })
    }
    pub fn shape(&self) -> &[usize] {
        &self.shape
    }
    pub fn values(&self) -> &[f32] {
        &self.storage.values
    }
    pub fn device(&self) -> Device {
        Device::Cpu
    }
    pub fn dtype(&self) -> &'static str {
        "f32"
    }
    pub fn layout(&self) -> &'static str {
        "row-major-contiguous"
    }
    pub fn alignment(&self) -> usize {
        std::mem::align_of::<f32>()
    }
    /// Finite storage/admission compatibility, not the provenance of the
    /// arithmetic which produced these values. A native operation must retain
    /// its separate backend/numerical policy (for example Vulkan TensorDispatch).
    pub fn numeric_profile(&self) -> &'static str {
        NUMERIC_PROFILE
    }
    /// Portable little-endian tensor ABI, distinct from FSM. No authority is
    /// conveyed by these bytes. Header, rank, extents and finite bits are checked.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + self.shape.len() * 8 + self.values().len() * 4);
        out.extend_from_slice(TENSOR_MAGIC);
        out.extend_from_slice(&[self.shape.len() as u8, 1, 0, 0]); // rank, f32, CPU, flags
        for &extent in self.shape.iter() {
            out.extend_from_slice(&(extent as u64).to_le_bytes());
        }
        for &value in self.values() {
            out.extend_from_slice(&value.to_bits().to_le_bytes());
        }
        out
    }
    pub fn decode(bytes: &[u8], memory: &MemoryBudget) -> Result<Self, Error> {
        if bytes.len() < 8 || &bytes[..4] != TENSOR_MAGIC || bytes[5..8] != [1, 0, 0] {
            return Err(fail("invalid tensor ABI header"));
        }
        let rank = usize::from(bytes[4]);
        if rank > MAX_RANK {
            return Err(fail("tensor rank limit exceeded"));
        }
        let header = 8 + rank * 8;
        if bytes.len() < header {
            return Err(fail("truncated tensor shape"));
        }
        let mut shape = [0usize; MAX_RANK];
        for (index, target) in shape[..rank].iter_mut().enumerate() {
            let extent =
                u64::from_le_bytes(bytes[8 + index * 8..16 + index * 8].try_into().unwrap());
            *target = usize::try_from(extent).map_err(|_| fail("tensor extent overflow"))?;
        }
        let len = elements(&shape[..rank])?;
        let size = len
            .checked_mul(4)
            .and_then(|n| n.checked_add(header))
            .ok_or_else(|| fail("tensor byte count overflow"))?;
        if bytes.len() != size {
            return Err(fail("tensor encoded length mismatch"));
        }
        // Reserve the final payload before materializing any values.
        let reservation = memory.reserve_bytes(len * 4)?;
        let mut values = reserve_vec(len)?;
        for bits in bytes[header..].as_chunks::<4>().0 {
            values.push(finite(f32::from_bits(u32::from_le_bytes(*bits)))?);
        }
        Ok(Self {
            shape: (&shape[..rank]).into(),
            storage: Arc::new(TensorStorage {
                values,
                _reservation: reservation,
            }),
        })
    }
}
fn elements(shape: &[usize]) -> Result<usize, Error> {
    if shape.len() > MAX_RANK {
        return Err(fail("tensor rank limit exceeded"));
    }
    // A zero extent remains valid, but cannot hide overflowing other extents.
    let mut product = 1usize;
    let mut zero = false;
    for &extent in shape {
        zero |= extent == 0;
        product = product
            .checked_mul(extent.max(1))
            .ok_or_else(|| fail("tensor shape product overflow"))?;
    }
    Ok(if zero { 0 } else { product })
}

/// Canonical CSR: row offsets begin at zero, end at nnz, and columns increase
/// strictly within each row. Duplicate coordinates are refused, never summed.
#[derive(Debug)]
pub struct SparseCsr {
    rows: usize,
    columns: usize,
    offsets: Vec<usize>,
    indices: Vec<usize>,
    values: Vec<f32>,
    _reservation: Reservation,
}
impl SparseCsr {
    pub fn new(
        rows: usize,
        columns: usize,
        offsets: &[usize],
        indices: &[usize],
        values: &[f32],
        memory: &MemoryBudget,
    ) -> Result<Self, Error> {
        if rows.checked_add(1) != Some(offsets.len())
            || indices.len() != values.len()
            || offsets.first() != Some(&0)
            || offsets.last() != Some(&values.len())
        {
            return Err(fail("invalid CSR lengths or terminal offsets"));
        }
        for pair in offsets.windows(2) {
            if pair[0] > pair[1] || pair[1] > values.len() {
                return Err(fail("invalid CSR row offsets"));
            }
            let row = &indices[pair[0]..pair[1]];
            if row.iter().any(|&column| column >= columns)
                || row.windows(2).any(|pair| pair[0] >= pair[1])
            {
                return Err(fail("CSR columns must be in range, sorted and unique"));
            }
        }
        for &value in values {
            finite(value)?;
        }
        let bytes = offsets
            .len()
            .checked_add(indices.len())
            .and_then(|n| n.checked_mul(std::mem::size_of::<usize>()))
            .and_then(|n| values.len().checked_mul(4).and_then(|m| n.checked_add(m)))
            .ok_or_else(|| fail("CSR byte count overflow"))?;
        let reservation = memory.reserve_bytes(bytes)?;
        let mut owned_offsets = reserve_vec(offsets.len())?;
        owned_offsets.extend_from_slice(offsets);
        let mut owned_indices = reserve_vec(indices.len())?;
        owned_indices.extend_from_slice(indices);
        let mut owned_values = reserve_vec(values.len())?;
        owned_values.extend_from_slice(values);
        Ok(Self {
            rows,
            columns,
            offsets: owned_offsets,
            indices: owned_indices,
            values: owned_values,
            _reservation: reservation,
        })
    }
    pub fn matvec(
        &self,
        vector: &Tensor,
        memory: &MemoryBudget,
        cancel: &Cancellation,
    ) -> Result<Tensor, Error> {
        cancel.check()?;
        if vector.shape() != [self.columns] {
            return Err(fail("CSR vector shape mismatch"));
        }
        let bytes = self
            .rows
            .checked_mul(4)
            .ok_or_else(|| fail("CSR output overflow"))?;
        let reservation = memory.reserve_bytes(bytes)?;
        let mut values = reserve_vec(self.rows)?;
        for row in 0..self.rows {
            cancel.check()?;
            let mut sum = 0f32;
            for index in self.offsets[row]..self.offsets[row + 1] {
                // Explicit multiply then add: no fusion or reordering profile.
                let product = finite(self.values[index] * vector.values()[self.indices[index]])?;
                sum = finite(sum + product)?;
            }
            values.push(sum);
        }
        Ok(Tensor {
            shape: Arc::from([self.rows]),
            storage: Arc::new(TensorStorage {
                values,
                _reservation: reservation,
            }),
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryManifest {
    pub abi: String,
    pub numeric_profile: String,
    pub source_sha256: String,
    pub package: String,
    pub version: u32,
    /// Exact public export names and scalar arities; semantic signatures are
    /// checked from the compiled source, not trusted from this declaration.
    #[serde(deserialize_with = "unique_exports")]
    pub exports: BTreeMap<String, usize>,
    pub capabilities: Vec<String>,
}
fn unique_exports<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, usize>, D::Error> {
    struct Exports;
    impl<'de> serde::de::Visitor<'de> for Exports {
        type Value = BTreeMap<String, usize>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a bounded map of unique export names")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> Result<Self::Value, A::Error> {
            let mut exports = BTreeMap::new();
            while let Some((name, arity)) = map.next_entry::<String, usize>()? {
                if exports.len() >= MAX_EXPORTS || exports.insert(name, arity).is_some() {
                    return Err(serde::de::Error::custom(
                        "duplicate export or export limit exceeded",
                    ));
                }
            }
            Ok(exports)
        }
    }
    deserializer.deserialize_map(Exports)
}
#[derive(Clone)]
pub struct ScalarLibrary {
    module: Module,
    manifest: LibraryManifest,
    identity: String,
}
impl ScalarLibrary {
    pub fn load(manifest_bytes: &[u8], source: &str) -> Result<Self, Error> {
        if manifest_bytes.len() > MAX_MANIFEST_BYTES || source.len() > MAX_PACKAGE_BYTES {
            return Err(fail("library byte budget exceeded"));
        }
        let manifest: LibraryManifest = serde_json::from_slice(manifest_bytes)
            .map_err(|e| fail(format!("invalid library manifest: {e}")))?;
        if manifest.abi != ABI
            || manifest.numeric_profile != NUMERIC_PROFILE
            || manifest.version != 1
        {
            return Err(fail("unsupported library ABI/profile/version"));
        }
        if manifest.package.is_empty()
            || manifest.package.len() > 128
            || !manifest
                .package
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            return Err(fail("invalid library package identity"));
        }
        if manifest.capabilities != ["compute.cpu"] {
            return Err(fail("library capability denied"));
        }
        if manifest.source_sha256 != format!("{:x}", Sha256::digest(source.as_bytes())) {
            return Err(fail("library source digest mismatch"));
        }
        if manifest.exports.is_empty()
            || manifest.exports.len() > MAX_EXPORTS
            || manifest.exports.values().any(|&n| n > MAX_ARGS)
        {
            return Err(fail("invalid library export budget"));
        }
        let parsed = Source::parse_compact(source)?;
        if parsed.version != 2 || !parsed.inputs.is_empty() {
            return Err(fail("library requires FSM2 with no ambient inputs"));
        }
        let mut actual = BTreeMap::new();
        for function in &parsed.functions {
            if function.export {
                if function.result != Type::F32 || function.params.iter().any(|p| p.ty != Type::F32)
                {
                    return Err(fail(
                        "library exports require scalar f32 parameters and result",
                    ));
                }
                actual.insert(function.name.clone(), function.params.len());
            }
        }
        if actual != manifest.exports {
            return Err(fail("library declared export ABI differs from source"));
        }
        let module = Module::compile(parsed)?;
        let mut digest = Sha256::new();
        digest.update(b"fluxr-scalar-library-v1\0");
        digest.update(NUMERIC_PROFILE.as_bytes());
        digest.update(module.encode());
        let identity = format!("{:x}", digest.finalize());
        Ok(Self {
            module,
            manifest,
            identity,
        })
    }
    pub fn identity(&self) -> &str {
        &self.identity
    }
    pub fn manifest(&self) -> &LibraryManifest {
        &self.manifest
    }
    pub fn session(
        &self,
        limits: Limits,
        maximum_calls: u64,
        cancellation: Cancellation,
    ) -> ScalarSession<'_> {
        ScalarSession {
            library: self,
            workspace: Workspace::default(),
            arguments: Vec::with_capacity(MAX_ARGS),
            limits,
            remaining: maximum_calls,
            completed: 0,
            attempted: 0,
            cancellation,
        }
    }
}
pub struct ScalarSession<'a> {
    library: &'a ScalarLibrary,
    workspace: Workspace,
    arguments: Vec<Value>,
    limits: Limits,
    remaining: u64,
    completed: u64,
    attempted: u64,
    cancellation: Cancellation,
}
impl ScalarSession<'_> {
    pub fn attempted_calls(&self) -> u64 {
        self.attempted
    }
    pub fn completed_calls(&self) -> u64 {
        self.completed
    }
    pub fn call(&mut self, export: &str, arguments: &[f32]) -> Result<f32, Error> {
        self.cancellation.check()?;
        if self.remaining == 0 {
            return Err(fail("library call budget exhausted"));
        }
        // Every attempt admitted under the call budget consumes one slot, even a
        // type/ABI failure or language trap; no success-only accounting.
        self.remaining -= 1;
        self.attempted += 1;
        if self.library.manifest.exports.get(export) != Some(&arguments.len()) {
            return Err(fail("library export or arity mismatch"));
        }
        self.arguments.clear();
        for &argument in arguments {
            self.arguments.push(Value::F32(finite(argument)?.to_bits()));
        }
        let result = self.library.module.run_portable_with(
            Invocation {
                export,
                inputs: &[],
                arguments: &self.arguments,
                limits: self.limits,
            },
            &mut self.workspace,
        )?;
        let Value::F32(bits) = result else {
            return Err(fail("library returned a non-f32 result"));
        };
        let value = finite(f32::from_bits(bits))?;
        self.completed += 1;
        Ok(value)
    }
    /// Row-major [rows, arity] -> [rows], preserving row order. Output is only
    /// published on complete success; a trap/cancel releases its reservation.
    pub fn batch(
        &mut self,
        export: &str,
        input: &Tensor,
        memory: &MemoryBudget,
    ) -> Result<Tensor, Error> {
        self.cancellation.check()?;
        let [rows, columns] = *input.shape() else {
            return Err(fail("scalar batch requires rank two"));
        };
        if self.library.manifest.exports.get(export) != Some(&columns) {
            return Err(fail("batch export arity mismatch"));
        }
        if rows as u64 > self.remaining {
            return Err(fail("batch exceeds remaining call budget"));
        }
        let reservation = memory.reserve_bytes(
            rows.checked_mul(4)
                .ok_or_else(|| fail("batch output overflow"))?,
        )?;
        let mut values = reserve_vec(rows)?;
        for row in 0..rows {
            values.push(self.call(export, &input.values()[row * columns..(row + 1) * columns])?);
        }
        Ok(Tensor {
            shape: Arc::from([rows]),
            storage: Arc::new(TensorStorage {
                values,
                _reservation: reservation,
            }),
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelCheckpoint {
    pub abi: String,
    pub program_identity: String,
    pub parameter_bits: Vec<u32>,
    pub completed_steps: u64,
    pub rng_seed: u64,
    pub provenance: String,
}
impl ModelCheckpoint {
    pub fn new(
        library: &ScalarLibrary,
        parameters: &[f32],
        completed_steps: u64,
        rng_seed: u64,
        provenance: &str,
    ) -> Result<Self, Error> {
        if parameters.len() > 4096 || provenance.len() > 1024 {
            return Err(fail("model checkpoint budget exceeded"));
        }
        for &value in parameters {
            finite(value)?;
        }
        Ok(Self {
            abi: "fluxr-model-v1".into(),
            program_identity: library.identity().into(),
            parameter_bits: parameters.iter().map(|p| p.to_bits()).collect(),
            completed_steps,
            rng_seed,
            provenance: provenance.into(),
        })
    }
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        serde_json::to_vec(self).map_err(|e| fail(e.to_string()))
    }
    pub fn decode(
        bytes: &[u8],
        library: &ScalarLibrary,
        parameter_count: usize,
    ) -> Result<Self, Error> {
        if bytes.len() > 64 * 1024 || parameter_count > 4096 {
            return Err(fail("model checkpoint byte/count budget exceeded"));
        }
        let checkpoint: Self = serde_json::from_slice(bytes).map_err(|e| fail(e.to_string()))?;
        if checkpoint.abi != "fluxr-model-v1"
            || checkpoint.program_identity != library.identity()
            || checkpoint.parameter_bits.len() != parameter_count
            || checkpoint.provenance.len() > 1024
        {
            return Err(fail("model checkpoint identity, shape or ABI mismatch"));
        }
        for &bits in &checkpoint.parameter_bits {
            finite(f32::from_bits(bits))?;
        }
        Ok(checkpoint)
    }
    pub fn parameters(&self) -> Vec<f32> {
        self.parameter_bits
            .iter()
            .copied()
            .map(f32::from_bits)
            .collect()
    }
}

#[derive(Debug)]
struct CpuSlots {
    maximum: usize,
    active: AtomicUsize,
    peak: AtomicUsize,
}
struct CpuPermit(Arc<CpuSlots>);
impl Drop for CpuPermit {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::AcqRel);
    }
}
/// Bounded asynchronous CPU batches. The implementation uses one OS thread per
/// admitted job; this explicit simple route is measured before pooling is added.
/// Admission refuses at capacity (backpressure), rather than building a hidden
/// unbounded queue. Jobs retain their input/output ownership until completion.
#[derive(Clone)]
pub struct CpuBatchExecutor {
    slots: Arc<CpuSlots>,
}
pub struct CpuBatchJob {
    cancellation: Cancellation,
    thread: Option<std::thread::JoinHandle<Result<Tensor, Error>>>,
}
impl CpuBatchExecutor {
    pub fn new(maximum_in_flight: usize) -> Result<Self, Error> {
        if maximum_in_flight == 0 {
            return Err(fail("CPU executor capacity must be positive"));
        }
        Ok(Self {
            slots: Arc::new(CpuSlots {
                maximum: maximum_in_flight,
                active: AtomicUsize::new(0),
                peak: AtomicUsize::new(0),
            }),
        })
    }
    pub fn active_jobs(&self) -> usize {
        self.slots.active.load(Ordering::Acquire)
    }
    pub fn peak_active_jobs(&self) -> usize {
        self.slots.peak.load(Ordering::Acquire)
    }
    fn permit(&self) -> Result<CpuPermit, Error> {
        let mut old = self.slots.active.load(Ordering::Acquire);
        loop {
            let next = old
                .checked_add(1)
                .filter(|&n| n <= self.slots.maximum)
                .ok_or_else(|| fail("CPU executor backpressure"))?;
            match self.slots.active.compare_exchange_weak(
                old,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    self.slots.peak.fetch_max(next, Ordering::AcqRel);
                    return Ok(CpuPermit(self.slots.clone()));
                }
                Err(current) => old = current,
            }
        }
    }
    pub fn submit(
        &self,
        library: Arc<ScalarLibrary>,
        export: String,
        input: Tensor,
        memory: MemoryBudget,
        invocation_limits: Limits,
        maximum_calls: u64,
    ) -> Result<CpuBatchJob, Error> {
        let &[rows, columns] = input.shape() else {
            return Err(fail("CPU batch requires rank two"));
        };
        if rows as u64 > maximum_calls || library.manifest.exports.get(&export) != Some(&columns) {
            return Err(fail("CPU batch call budget or ABI mismatch"));
        }
        let permit = self.permit()?;
        let cancellation = Cancellation::default();
        let worker_cancel = cancellation.clone();
        let thread = std::thread::Builder::new()
            .name("fluxr-cpu-batch".into())
            .spawn(move || {
                let _permit = permit;
                let mut session = library.session(invocation_limits, maximum_calls, worker_cancel);
                session.batch(&export, &input, &memory)
            })
            .map_err(|error| fail(format!("CPU worker creation failed: {error}")))?;
        Ok(CpuBatchJob {
            cancellation,
            thread: Some(thread),
        })
    }
}
impl CpuBatchJob {
    pub fn request_cancel(&self) {
        self.cancellation.cancel();
    }
    pub fn cancellation_requested(&self) -> bool {
        self.cancellation.is_cancelled()
    }
    pub fn is_finished(&self) -> bool {
        self.thread
            .as_ref()
            .is_none_or(|thread| thread.is_finished())
    }
    /// Waiting is an explicit host operation. Cancellation cannot preempt an
    /// in-progress VM instruction; its finite invocation budget remains active.
    pub fn wait(mut self) -> Result<Tensor, Error> {
        self.thread
            .take()
            .unwrap()
            .join()
            .map_err(|_| fail("CPU batch worker panicked"))?
    }
}
impl Drop for CpuBatchJob {
    fn drop(&mut self) {
        // A dropped receipt is cancellation, not evidence of successful work.
        // The detached finite worker retains its permit and owned memory until
        // it exits; dropping this handle never frees in-flight buffers early.
        if self.thread.is_some() {
            self.cancellation.cancel();
        }
    }
}

#[cfg(test)]
mod executor_tests {
    use super::*;
    #[test]
    fn admitted_permits_bound_authority_and_release_after_drop() {
        let executor = CpuBatchExecutor::new(2).unwrap();
        let first = executor.permit().unwrap();
        let second = executor.permit().unwrap();
        assert!(executor.permit().is_err());
        assert_eq!(executor.active_jobs(), 2);
        drop(first);
        let replacement = executor.permit().unwrap();
        assert_eq!(executor.active_jobs(), 2);
        drop(second);
        drop(replacement);
        assert_eq!(executor.active_jobs(), 0);
    }
}
