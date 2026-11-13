# Checked scalar graphs and bounded streams

`fluxr-scalar-graph-v1` adds an immutable, reusable CPU graph over existing
`fluxr-scalar-library-v1` packages. The graph executes actual FluxR exports for
preprocessing, models, instruments, mixing and effects. It does not replace the
portable evaluator, change an existing source/artifact encoding or silently
claim to optimize the earlier record-heavy training workload.

## Exact contract

A graph contains an input column count, ordered scalar-call nodes and output
node indices. Each argument names an input column, exact finite binary32 bits or
an earlier node. Preparation checks the actual compiled library export arity;
unknown exports, forward edges, cycles, bad columns, unsupported ABI, nonfinite
constants and excess limits refuse. The v1 profile admits at most 256 nodes,
128 supplied libraries, 64 input/output columns, 32 arguments per call and a
64 KiB canonical graph. Canonical serialization rejects overflow before copying
an oversized encoded payload. These are execution bounds, not staffing limits.

Graph identity includes the graph ABI, numeric profile, canonical graph, every
supplied library's semantic program identity and its exact source SHA-256.
Whitespace-only source changes can preserve the library's semantic identity but
change the captured graph input identity. Supplied unused libraries are captured
too; dependency pruning is not inferred. Constants preserve signed-zero bits.
The graph retains `Arc` ownership of captured libraries and has no mutable public
fields. Hashes convey neither producer authentication nor host authority.

Preparation does not fuse, reorder, inline or eliminate calls. For each row,
every node runs exactly once in declaration order, including nodes not selected
as outputs. Calls use the same released `ScalarSession::call` path and fresh
instruction/cell/frame limits. They retain finite input/output checks, logical
admission/cleanup charges, first traps and exact float bits. A custom graph is a
new explicit composed program, not a resource-equivalence claim for a previous
monolithic function. Graph bookkeeping, hashing and allocations are additional
host costs, reported separately from portable logical charges.

## Stream state and publication

`ScalarGraph::stream` captures one payload budget, cancellation token and finite
limits for total admitted rows, maximum chunk rows and maximum attempts. Total
rows multiplied by all nodes must fit `u64`, including the sum across different
libraries. The maximum attempts cannot exceed 65,536. Each library has an
independent reusable VM workspace per stream; concurrent streams share only
immutable graph/programs and atomic payload-budget accounting.

An execution request supplies the graph identity, next sequence and immutable
rank-two input tensor. Stale identities, duplicate/reordered sequences, wrong
rank/columns, chunk limits, exhausted total row limits and unavailable memory
reject before execution. Every request inside the attempt envelope consumes one
receipt slot, including rejections. The caller must inspect `next_sequence()`
after errors. Exhaustion of the receipt envelope returns an explicit error and
executes no work; further out-of-envelope requests are not stored indefinitely.

Once admission succeeds, all requested rows consume the total row envelope.
Failures never refund unexecuted rows in that chunk. Receipts distinguish
admitted rows, attempted rows, completely evaluated rows and published rows;
attempted calls include traps, while completed calls exclude them. Cancellation
that reaches a library before call admission does not become a fabricated call.
Each detached receipt binds its graph identity and a versioned stream-contract
digest covering the exact graph, CPU/numeric policy, invocation/row/chunk/attempt
limits and payload-budget ceiling. `contract()` exposes that immutable contract.
This identifies an execution policy, not a unique distributed session: separate
streams with identical policies deliberately share the contract digest. Receipts
retain requested graph pins up to 128 bytes; longer rejected strings retain only
their byte length, without unbounded copying or hashing. Receipts include FTB1
input/output digests, the first failing node when applicable, status and exact
failure text. Digests are streamed without an extra full tensor
copy. Rejected shapes/identities have no input digest because they are not admitted.

Output has shape `[chunk_rows, graph_outputs]`. Any later row trap or cancellation
releases all output reservation and publishes no partial output. Previously
successful chunk tensors remain valid. A failed VM call cannot poison a later
chunk's workspace; the existing evaluator resets its scratch state after errors.
Successful synchronous completion publishes once under its receipt sequence.
A cancellation requested after completed execution is not a retroactive rollback.
This is a local Rust-handle protocol; no distributed exactly-once delivery,
durable session recovery or external authorization is asserted. Sequence checks
apply only to one live stream instance; a restarted/new stream starts at zero.

## Ownership and cost

Row scratch and argument-vector payload are reserved at stream construction.
Every output payload is reserved before row execution and stays charged while
any tensor owner remains. Failure, dropped outputs and stream teardown release
their respective reservations. Holding old outputs can correctly cause later
backpressure. Caller-owned input tensors have their existing ownership/budget;
the stream does not relabel their memory as its own allocation.

This payload quota does not measure ledger metadata, graph/source code, VM
capacity, input-construction buffers, allocator overhead or total process RSS.
Those remain separate bounded counts or observed process costs. Attempt metadata
is pre-reserved with a fixed entry cap. Input/output hashing and typed checks add
linear work under the admitted chunk envelope. Cancellation is cooperative at
rows/calls; an in-progress invocation retains its finite language limits. There
is no queue, implicit thread spawning, GPU transfer, file/network capability or
foreign-native plugin execution in the graph.

## Reproducible executable paths

```sh
cargo test --locked --test cpu_stream
cargo test --locked --release --test cpu_stream
cargo build --locked --release --example native_stream_demo
python3 tools/qualify_native_stream.py --binary /absolute/target/release/examples/native_stream_demo --output /absolute/external/evidence
python3 tools/bench_tokens.py --manifest benchmarks/native-stream.toml
```

The ML fixture imports the existing exact-program checkpoint, runs FluxR affine
preprocessing, custom neural prediction and nearest-centroid classification, and
streams 8,192 rows at chunk sizes 32, 256 and 1,024. Each row is compared with an
independent Rust schedule. All chunk boundaries must produce identical ordered
output bits. Preparation, input construction, execution, raw chunk latencies,
receipt attempts and payload peaks are reported. The direct FluxR reference and
optimized Rust reference both execute the same algorithms and hash outputs;
their timing omits the graph's tensor construction, receipt and FTB1 hash work.
That scope difference is explicit rather than a speedup claim.

The effects fixture composes externally loaded triangle and bell instruments,
FluxR mixing and a separately packaged FluxR soft clipper for one second of audio.
Effects are compared bit-for-bit with Rust applied to direct FluxR instrument
calls. The runner first records exact exit 1 for a missing effect package, adds
that package, then succeeds with the same runtime binary hash and zero rebuilds.
Repeated full runs verify identical graph and sample hashes. This complements
the separate 16-second multitrack composition; it is not a replacement for its
score, evidence or unclosed artistic/execution requirements.

The source-token catalog is experimental and separate from frozen replay. The
equivalent Rust effect package, full demo host and this context are accounted
separately. The <=10% equivalent-Rust target remains binding; each
pair's actual measured result must be reported, including failure. This
fixture is not a complete autonomous coding episode or live-model trial.

## Qualification boundaries and design references

The tests cover immutable identity, stale requests, malformed DAGs, exact finite
bits, failed-call recovery, unused-node traps, invocation limits, cancellation,
late failure output atomicity, zero extents/arity, bounded retained memory,
receipt exhaustion and concurrent independent workspaces. They are local subset
evidence, not closure of N01–N04, M01–M04 or D01–D04.

MLIR's official Linalg documentation describes structured operations whose
operand metadata determines iteration and whose payload is kept distinct from
transformations. This supports keeping this adapter's checked row iteration and
exact scalar payload schedule explicit; no MLIR dependency or integration-cost
claim is made. See [Rationale and
Payload-Carrying Ops, properties 1–2](https://mlir.llvm.org/docs/Dialects/Linalg/).

ONNX Runtime's official [I/O Binding documentation](https://onnxruntime.ai/docs/performance/tune-performance/iobinding.html)
explains why implicit transfers can dominate a graph-run measurement. The demo
reports preparation and input construction separately and states that its route
is CPU only. It does not integrate ONNX Runtime.

PortAudio's [Writing a Callback Function](https://portaudio.com/docs/v19-doxydocs/writing_a_callback.html)
describes exclusions including
allocation/destruction, locks and I/O. This graph uses allocation, hashing and
receipt storage, so neither faster offline execution nor stable samples make it
a qualified real-time callback. Audio devices, timing-policy negotiation, safe
graph replacement and native plugin isolation remain independent open gates.

## Integration and rollback

This module depends on checked tensors, scalar libraries and portable
VM cleanup. Existing `ScalarSession`, `batch`, ABI and tensor bytes are unchanged.
To roll back a caller, stop new graph work, let owned outputs drain and use the
existing direct scalar/batch route. Preserve graph/source identities and receipts;
an incompatible future graph must use a new ABI and explicit migration.
