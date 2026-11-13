# Bounded stateful FluxR graphs — experimental v1

`cpu_compute::stream::stateful` adds explicit loop-carried scalar state to the
existing checked FluxR graph/library infrastructure. It supports deterministic
causal DSP, smoothing/envelope extraction and recurrent scalar algorithms. The
new `fluxr-stateful-scalar-graph-v1` ABI and receipt/contract names do not alter
released stateless graph, scalar library, FSM, FXH or FTB1 meanings.

## Transition contract

`StatefulSpec` names one ordinary `GraphSpec`, external input columns, exact
finite initial state bits and a node index for each next-state cell. The inner
graph inputs consist of external columns followed by previous-state columns.
Each row evaluates every ordinary FluxR graph node in declaration order. Its
ordinary outputs are published, and the selected state results are copied
simultaneously into the next row's state. Every state expression therefore sees
the same previous state; duplicate update sources are valid aliases of immutable
node results. Forward edges and cycles inside one transition remain rejected by
the inherited graph validator. Recurrence crosses row boundaries explicitly.

The outer profile permits 1–64 state cells, with external plus state columns at
most the inherited 64-column graph limit. The transition retains the 256-node,
128-library, 32-call-argument, 64-output and 64 KiB canonical graph bounds. The
complete outer canonical specification is bounded at 72 KiB. Bytes are checked
before decoding; bounded serialization refuses excessive caller-constructed
specifications before cloning them. Initial state must be finite; signed zero
bits participate in identity and are preserved. Unknown fields, wrong ABIs,
state dimensions, out-of-range update indices and malformed inner graphs refuse.

Identity binds the new ABI, the canonical outer spec and the inherited graph
identity, which captures every supplied library's semantic identity and exact
source SHA-256. No fusion, expression elimination, state inference, reduced
precision or altered arithmetic evaluation is introduced. Calls continue through
`ScalarSession::call` with the released instruction, cell, frame, finite-value,
logical ownership and cleanup checks. These hashes identify bytes and semantics;
they do not authenticate a publisher or authorize a host capability.

## Atomic state and output publication

`stream` captures finite row/chunk/attempt and invocation limits, a payload budget
and cooperative cancellation. Each `execute` request requires the graph identity,
the current `StatePin` (revision and a fixed 32-byte state digest), the next
attempt sequence and a rank-two input tensor. A stale graph, stale state revision
or digest, duplicate/reordered sequence, invalid input shape, insufficient memory
or exhausted work envelope rejects before calls. Every in-envelope request,
including rejection, consumes a receipt slot. After a rejection use
`next_sequence()`; do not assume the previous attempt number remains current.

Committed state and tentative state occupy two bounded arrays. Before execution
the tentative array is reset from committed state. It advances across rows while
committed state remains untouched. A language trap or observed cancellation in
any row discards the entire output and leaves committed state and its revision
unchanged. An admitted failed chunk still consumes all its admitted rows, including
unexecuted rows. Attempted/completed call and row counts record the work that
actually occurred; completed rows do not imply published rows.

After every row succeeds and the complete output tensor and hashes are ready,
cancellation is checked again at the publication boundary. Only then do output
and final state become visible together. Successful chunks, including zero-row
chunks, advance the state revision once. An empty chunk preserves the state
value digest and all signed-zero bits. A later cancellation is not a retroactive
rollback. A failed workspace is reset by the existing evaluator before reuse;
the next chunk restarts from the last committed state, never from tentative data.

Receipts bind the graph and exact stream policy, requested/current state pins,
input/output FTB1 hashes, attempts, work counters and first failing node. Their
versioned nested execution record is not labeled as a stateless-v1 receipt.
Request graph strings are retained only up to 128 bytes; longer refusals keep
only their length. State pins have fixed size. At most 65,536 receipts can be
stored; after that cap further requests explicitly refuse without execution and
must be retained by the caller if continued attempt accounting is required.

Pins belong to one live stream protocol. Independent streams with equal graph,
state and revision deliberately share the same pin. Durable resume/import uses
the separate [FSC1 checkpoint profile](native-checkpoint-v1.md), which
exports complete bounded state/attempt history and resumes a fresh stream
with the exact graph and policy, preserving failed-work budgets. It does not add
a global session nonce, cross-process authorization or distributed exactly-once
guarantee. The snapshot accessor borrows immutable committed state. The stream
owns all mutable workspace; independent streams share only immutable programs
and optional atomic budget accounting.

## Memory, cancellation and real-time limits

Construction reserves payload for graph node scratch, at most 32 call arguments,
and both state arrays. The sample's four nodes and two state cells reserve 160
bytes. Each output payload is reserved before execution and stays charged until
its last tensor owner drops. A retained output can cause backpressure without
advancing state. Stream teardown releases both state arrays and scratch; it does
not invalidate a retained successful output. Inputs keep their original budgets.
Memory does not grow with total executed rows except for the explicitly bounded
receipt count. No `frames × state_cells` history is allocated by the runtime.

The payload limit excludes graph/source metadata, receipt metadata, compiled
code, VM capacities, allocator overhead and caller-owned fixtures/oracles. These
are separate structural limits or reported process costs. There is no process
RSS hard-cap claim. The row/call work envelope bounds execution; cancellation is
cooperative between rows/calls and immediately before publication, and a running invocation completes or reaches its
VM limit. There are no hidden worker threads, queue, GPU transfers or ambient
file/network effects in this adapter.

This remains an offline/worker interface. It allocates output/receipt objects,
hashes data and uses the ordinary VM, so it is unsuitable for a qualified audio
callback. Safe live graph replacement, device/sample-rate negotiation, callback
allocation/destruction exclusion, foreign-native plugin isolation, device clocks
and physical audio execution remain open. A completed stream may be dropped and
a new immutable graph prepared outside execution; that is not a live swap proof.

## Usable FluxR package and reproducible qualification

`examples/native/effects/stateful/program.flux` independently packages four real
FluxR exports: smoothing recurrence, residual subtraction, envelope recurrence
and gain. The demo loads this source/manifest from disk; none is embedded in its
Rust runtime. It runs a two-state graph over 24,000 frames at 24 kHz, with an exact
dyadic input, coefficient changes at frames 7,999 and 16,001, and gain changes on
every frame. Parameter columns are consumed for each row regardless of chunk
boundaries. These changes are explicit sample-index automation, not a general
musical-event/timeline compiler or an implicit block-rate policy.

The matched independent Rust schedule and direct FluxR export schedule both
produce every output and final state bit. Stateful chunks of 32, 256 and 1,024
frames must match the same reference. Reports retain preparation, input tensor
construction, execution, raw chunk timings, p50/p95/p99/max, simulated deadline
misses, complete attempt receipts and owned payload peak. Direct Rust/FluxR
measurements append the same output vectors; graph execution additionally pays
its tensor admission and state/input/output receipt hashes. Compare these scopes
explicitly. Actual process CPU/RSS is recorded by the enclosing evidence runner.
The 16-second multitrack composition remains a separate test.

```sh
cargo test --locked --test cpu_stateful --test cpu_stream
cargo test --locked --release --test cpu_stateful --test cpu_stream
cargo build --locked --release --example stateful_stream_demo
"$CARGO_TARGET_DIR/release/examples/stateful_stream_demo" examples/native "$WORK/stateful.json"
python3 tools/bench_tokens.py --manifest benchmarks/native-stateful.toml
```

Run the exact same binary before/after staging the separately packaged effect:
its absence must produce exit 1 and no output report; adding the package must
succeed with the same runtime hash and no Rust rebuild. Repeat full runs and
compare graph, source, output and final state identities. This establishes a
FluxR program-package extension, not CLAP/VST/native shared-library compatibility.

`tests/cpu_stateful.rs` covers independent recurrence and frame automation across
six chunkings, malformed graphs, finite/sign-bit identity, stale state/sequence,
simultaneous updates, late failure rollback with charged work, unused-node traps,
retained output backpressure, cancellation admission, empty commits, independent
concurrent streams and finite invocation limits. A pseudorandom finite fixture
uses a fixed seed; these are bounded differential/adversarial tests, not a proof
for every numerical input or scheduler interleaving. The token catalog remains
experimental and cannot admit arbitrary evaluated Rust into frozen replay. The
≤10% equivalent-Rust token target remains binding; record actual tokenizer
results and failures, host/setup/context separately, and make no live-agent or
full-episode superiority inference.

## Design references and integration

The transition and scheduling contracts use these explicit design references:

- [ONNX Loop operator, opset 25](https://onnx.ai/onnx/operators/onnx__Loop.html#loop-25),
  Summary, sample body, loop-carried dependence explanation, Attributes and
  Inputs/Outputs. Its explicit prior/next state roles informed the transition
  adapter. This is not ONNX compatibility or an ONNX runtime integration.
- [W3C Web Audio API 1.1 draft of 22 September 2026](https://www.w3.org/TR/2026/WD-webaudio-1.1-20260922/),
  §1.6 AudioParam's per-sample versus per-block rates, informed the explicit
  per-frame columns and boundary-invariance oracle. This work-in-progress draft
  does not establish browser conformance.
- [PortAudio v19, Writing a Callback Function](https://portaudio.com/docs/v19-doxydocs/writing_a_callback.html),
  callback exclusions and example, define restrictions relevant to the
  offline-only boundary. PortAudio and physical callback behavior are not qualified.

Integration uses the locked serde,
SHA-256, FluxR compiler/evaluator and checked tensor/library code; the runtime
adapter overhead is measured against direct calls. Native trust remains in
Rust/compiler/allocator/OS/CPU behavior and existing FluxR validation. Keep all
failures and exact source identity. Before disabling this route, stop new chunks,
preserve the committed state and let outstanding output owners drain. A caller
using a different route must explicitly carry compatible state. No encoding is rewritten; any
future incompatible state contract needs a new ABI and explicit migration.
