# Stateful stream checkpoints — FSC1

`cpu_compute::stream::stateful::checkpoint` adds bounded data-only export and
explicit resume to the existing stateful CPU worker protocol. It does not change
the stateful graph, stream, receipt, scalar-library, FSM or tensor encodings.

## API and acceptance boundary

`StatefulStream::checkpoint(maximum_bytes)` exports an immutable view of the
committed state, exact stream policy, all library call budgets and the complete
attempt ledger. It returns `StatefulCheckpoint`, whose `bytes()` and
`sha256()` identify the exact serialized bytes. The maximum is mandatory,
nonzero and at most 64 MiB. A nonallocating sizing pass checks every encoded field
and the total against that cap before allocating the output extent. Insufficient
space or an unsupported oversized receipt returns an error; history is never
truncated. Repeated export of unchanged state is byte deterministic.

`StatefulGraph::resume_checkpoint(bytes, admission, limits, memory, cancellation)`
requires:

- the exact immutable graph, including initial-state specification, supplied
  library semantic identities and exact library source bytes;
- `CheckpointAdmission { maximum_bytes, expected_sha256 }`, supplied by the
  caller separately from the untrusted checkpoint;
- the original invocation, total row, chunk and attempt limits;
- the original payload-budget limit, though the `MemoryBudget` owner can be a
  fresh instance with that same limit;
- an uncancelled caller token and an uncancelled stored stream.

The API checks bounds and the expected byte identity, decodes data, validates
all ledger relationships, and only then constructs a fresh stream. Allocation
or cancellation failures drop fresh resources. It never overwrites a live
stream. A supplied cancellation handle is never cleared or changed. Cancelled
streams can be exported to preserve evidence, but their checkpoints refuse
resume even if the caller supplies a fresh token.

The external expected digest prevents a caller from accidentally accepting a
different checkpoint when that expected identity comes from trusted state.
A digest embedded in an untrusted message would not supply that property.
Neither SHA-256 nor structurally consistent counters prove authorization,
truthful execution, numerical correctness of historical outputs, independent
grading or complete external effects. No signature or producer authentication
is introduced.

Resume is explicitly a **fork** of a recorded stream endpoint. Replaying the
same admitted bytes twice can create two equal streams. No global anti-replay,
latest-checkpoint authority, distributed exactly-once processing, crash-durable
publication or external side-effect transaction is claimed. A caller needing
anti-rollback must maintain its own trusted expected checkpoint identity.
The example admission file demonstrates an external identity input; its access
control remains the host's responsibility.

## State and work preservation

The checkpoint preserves exact finite f32 state bits, including signed zero.
The stored state digest must match those bits and the graph identity. The
receipt chain begins at the original graph's initial state and revision zero.
Completed chunks advance the revision once, including empty chunks; rejected or
failed chunks retain the previous state pin. The final receipt state must equal
the checkpoint endpoint.

All original attempt receipts survive unchanged, including requested graph
strings/lengths, requested sequence/state, input/output identities, status,
first failed node, failure text and attempted/completed row/call counts.
The next sequence is the number of retained attempts. Exhausted ledgers stay
exhausted after resume. This format has no history-pruning, receipt compression
or reset operation.

Admitted rows are recomputed from completed and failed chunks, including their
unexecuted admitted tails. Rejected requests add no admitted work. Completed
rows remain distinct from published rows. Aggregates must exactly equal the
serialized statistics.

Per-library attempted, completed and remaining calls are retained. Validation
reconstructs their distribution from every completed row and each failed row's
ordered node prefix. A call that was entered and trapped is charged but not
completed; the validator never refunds that failed attempt or moves it to
another library. Whole-row admission guarantees sufficient per-library call
budget. The only failure before charging a call is monotone cancellation, whose
stored flag prevents resume. Such cancelled history can still be exported for
audit; an uncancelled file claiming that uncharged failure is inconsistent and
refuses even after its file hash and aggregate counters are recomputed. For each
library, attempted plus remaining must equal the original row budget times that
library's calls per row. A failed chunk without a failing node also represents
row/final cancellation: finite node results and checked output extents make the
post-row tensor check infallible. Such an uncancelled history is rejected even
when its complete-row and per-library counters are made internally consistent.

The live evaluator's reusable VM workspace and row scratch are not serialized.
They are fresh implementation storage, with no committed language state.
Tentative failed-chunk values are discarded; the resumed tentative state starts
from the last committed state. Pending native work does not exist in this
synchronous CPU profile. General asynchronous/device state requires a different
qualified checkpoint contract.

Outputs are independent immutable tensors and remain charged to their original
budget until the final owner drops. Checkpointing does not capture or release
those outputs. A resumed stream using the same budget can therefore encounter
backpressure from previously retained output. The new stream's own scratch
reservation is additive, so keeping the old stream alive can refuse resume
under a tight shared budget.

## Encoding

FSC1 uses explicit little-endian integer fields, fixed-length hashes and
length-bounded UTF-8 text. It never deserializes executable objects, compiler
inputs, host addresses or file paths. There are no optional unknown fields,
extension tags, implicit numeric conversions or trailing padding.

| Order | Field | Encoding |
| --- | --- | --- |
| 1 | Magic/version | 8 bytes: `46 53 43 31 0d 0a 1a 0a` |
| 2 | Total length | u64, including magic and length field |
| 3 | Graph identity | 64 lowercase ASCII hexadecimal bytes |
| 4 | Stream contract identity | 64 lowercase ASCII hexadecimal bytes |
| 5 | Exact policy | Seven u64 values in the order below |
| 6 | Committed state pin | u64 revision, then 32 digest bytes |
| 7 | State cells | u16 count, then count × u32 f32 bits |
| 8 | Statistics | Nine u64 values in the order below |
| 9 | Library budgets | u16 count, then one fixed record per supplied library |
| 10 | Attempts | u32 count, then complete receipt records |
| 11 | Cancelled | One Boolean byte; must be false to resume |

Policy order is invocation instructions, invocation cells, invocation frames,
maximum rows, maximum chunk rows, maximum attempts and payload-budget bytes.
They must match the independently supplied policy, not merely each other.
The stream contract digest also binds the unchanged numeric profile, device,
ABI and whole-chunk publication semantics.

Statistics order is admitted rows, attempted rows, completed rows, published
rows, attempted calls, completed calls, completed chunks, failed chunks and
rejected chunks. A library record contains its 64-byte semantic identity,
64-byte source SHA-256 and u64 attempted, completed and remaining call counts.

A receipt record contains, in order:

1. requested state pin, state-before pin and state-after pin;
2. state-committed Boolean;
3. optional requested graph text, then its original requested byte length as u64;
4. u64 attempt and requested sequence;
5. status byte: 0 completed, 1 failed, 2 rejected;
6. requested rows as u64;
7. optional 64-byte input hash and optional 64-byte output hash;
8. u64 attempted rows, completed rows, attempted calls and completed calls;
9. optional failed node index as u64;
10. optional failure text.

Every optional field begins with a Boolean byte, exactly 0 or 1. Present text
then has a u32 byte length and exact UTF-8 bytes. Requested graph text is bounded
at 128 bytes; its absent form is permitted only when the original byte length
exceeded 128, preserving released receipt behavior. Failure text is bounded at
16 KiB. A larger already-retained failure prevents export under FSC1 rather than
silently dropping or rewriting that failure.

Graph/contract identities and static receipt ABI labels are encoded once and
reconstructed into each receipt. They were already identical for every receipt
of the original stream. No other receipt field is omitted. All identity strings
must be exactly lowercase hexadecimal. Unknown magic/status/Boolean values,
truncation, integer overflow, platform-size overflow, unexpected counts and
trailing bytes refuse.

The state count must equal the prepared graph's at-most-64 cells, library count
must equal its at-most-128 supplied libraries, and attempts must not exceed the
original limit or 65,536. Before reserving a receipt vector, the decoder also
checks the encoded input can contain at least 191 bytes per declared receipt.
This prevents a tiny input declaring an enormous ledger. Field validation is
followed by chain, aggregate and per-library validation; a matching file hash
does not bypass any check.

## Memory and cancellation

Checkpoint metadata uses its own explicit byte/count bounds and is not charged
as tensor payload. The returned byte vector is caller-owned until dropped.
The encoder first sizes without allocating the output and then reserves its
complete extent. The decoder borrows the input while retaining decoded strings,
receipts and counters. Construction can briefly retain decoded metadata and
fresh stream metadata together. These allocations, graph/source data, compiled
code, VM capacities and allocator overhead are outside `MemoryBudget`, whose
released meaning remains owned tensor/scratch payload.

Consequently, `payload_peak_bytes` is not total checkpoint or process peak
memory. The demo records encoded byte sizes and the metadata limit separately;
an enclosing resource runner must record process peak RSS. The hard format cap
does not promise an exact allocator peak. Input length, structural counts and
field bounds make allocations finite; process admission remains an independent
host responsibility.

Cancellation is checked before admission, during receipt decoding, before
construction and before returning the fresh stream. A bounded SHA-256 pass is
not preempted between every input byte. Export may run after cancellation to
retain audit evidence; its stored monotone cancellation flag is sampled at the
final encoding boundary. Later cancellation does not rewrite already-exported
bytes or revoke their external admission.

These functions allocate, hash, validate and destroy metadata. They are
**worker/offline operations**. Do not call them from a qualified audio callback.
No callback device, native plugin ABI, live graph replacement, arbitrary model
checkpoint, scheduler/clock migration or GPU state serialization is implemented.

## File-based, unchanged-runtime example

The existing independently packaged FluxR recurrence remains at
`examples/native/effects/stateful`. The new
`examples/stateful_checkpoint_demo.rs` embeds no FluxR package bytes. It loads
the manifest/source at runtime and has separate save and resume processes.

Run all commands through the shared resource coordinator, with external
`CARGO_TARGET_DIR` and writable `TMPDIR`:

```sh
cargo fmt --all -- --check
cargo test --locked --offline --test cpu_checkpoint --test cpu_stateful --test cpu_stream
cargo test --locked --offline --release --test cpu_checkpoint --test cpu_stateful
cargo build --locked --offline --release --example stateful_checkpoint_demo
"$CARGO_TARGET_DIR/release/examples/stateful_checkpoint_demo" save examples/native "$WORK/prefix.fsc" "$WORK/admission.json"
"$CARGO_TARGET_DIR/release/examples/stateful_checkpoint_demo" resume examples/native "$WORK/prefix.fsc" "$WORK/admission.json" "$WORK/resumed.json"
```

Output paths use create-new semantics, so repeated trials need distinct names.
The save phase executes 2,048 frames in 256-frame chunks and writes the checkpoint
plus separate admission file. The resume phase starts from those files in a new
process, executes the remaining 2,048 frames and compares against direct Rust and
an uninterrupted FluxR run with the same chunk schedule. Every output/state bit,
entire attempt ledger, statistic and final checkpoint byte sequence must agree.

The paired Rust continuation performs the same recurrence, residual, envelope
and gain operations on the same arrays and appends the same outputs. FluxR
additionally pays tensor admission, VM calls and receipt hashes. Reports separate
preparation, file reading, checkpoint encode/resume, continuation execution and
raw chunk times. Enclosing command receipts supply toolchain, source/tree,
runtime-binary hashes, exits, CPU and process peak RSS. No ratio is inferred
before execution.

To test package independence, run save against a missing package root and retain
the failure; stage the original package into that root and retry with the same
binary hash. Repeat resume from the same admitted checkpoint into separate
report paths. This is a program-package extension and explicit replay test, not
an independently loaded foreign Rust shared library.

The FluxR arithmetic source is shared with the
`benchmarks/native-stateful.toml` comparison.
`benchmarks/native-checkpoint.toml` records that same pair with the checkpoint host,
context, exact two-process command route and runtime comparison scope. Any new Rust
orchestration/admission code and checkpoint setup belong in separately reported
host and complete-episode costs. The ≤10% source-token target and
the live-model qualification gate remain binding; unsuccessful measurements are nonpasses.

## Tests, integration and rollback

`tests/cpu_checkpoint.rs` covers exact ledger/bit differential continuation,
failure/rejection/empty budget persistence, signed zero, subject/policy/digest
binding, cancellation, every small-fixture truncation, malformed metadata,
unchanged original state, shared-budget/output lifetime and intentional fork
cases. Independent continuation oracles must also be executed against the final
candidate; agreement with a shared implementation alone does not establish
semantic independence. These test definitions are not pass claims.

Run the complete applicable integration/feature matrix after changes.
Preserve stateful compatibility tests and encodings. To disable checkpoint/resume,
stop new callers and retain live stream owners until their work and outputs drain.
Keep FSC1 files and trusted admission records; a consumer without FSC1 support
must refuse them rather than guessing. A future incompatible policy or format
needs a new ABI, explicit migration and retained original acceptance criteria.

## Design references

Relevant state and serialization contracts include:

- [LV2 State C API](https://lv2plug.in/c/html/group__state.html): compatibility,
  consistent save, exclusive restore and portable-state flags.
- [PyTorch serialization](https://docs.pytorch.org/docs/2.15/notes/serialization.html):
  restricted loading still requires resource and downstream metadata checks.
- [Safetensors format](https://github.com/safetensors/safetensors/blob/main/README.md):
  explicit lengths/shapes/offsets are separate from numerical validation.
- [PortAudio callback restrictions](https://portaudio.com/docs/v19-doxydocs/writing_a_callback.html):
  serialization and host I/O stay outside callbacks.

FSC1 requires no additional model weights or framework dependency. These references
provide design context, not LV2/PyTorch/Safetensors compatibility, external
performance or device qualification.

