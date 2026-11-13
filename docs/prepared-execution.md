# Prepared execution and native tiers

The optional `typed-jit` feature executes checked typed plans on Linux GNU
x86-64 and Windows x86-64. Linux uses the System V ABI; Windows uses its native
ABI and executable-memory owner. Default builds and unsupported targets retain
the portable evaluator. Source semantics and portable artifact bytes are shared
across these routes. Generated machine code is owned by the current process and
is not a portable artifact.

## Prepare once, invoke with checked inputs

```rust
use fluxr::structured::{Limits, SourceDocument, Value, Workspace};

fn prepared_example() -> Result<(), fluxr::Error> {
    let document = SourceDocument::parse(
        "fluxr 1\npub calc 2 x int y int int ret rem add mul x y 17 997"
    )?;
    let module = document.compile()?;
    let call = module.prepare_call("calc")?;
    let mut workspace = Workspace::default();
    let (result, route) = call.run_with_route(
        &[], &[Value::I64(11), Value::I64(7)], Limits::default(), &mut workspace
    );
    assert_eq!(result?, Value::I64(94));
    println!("{route:?}");
    Ok(())
}
```

`PreparedCall` borrows one immutable `Module`, resolves its export and native
cache once, and retains that revision. It cannot be rebound to another module,
a stale export index or a caller-supplied code pointer. `run`, `run_with` and
`run_with_route` preserve input/argument checks and logical limits. The caller
owns the workspace and input/result storage.

Preparation can compile and allocate. A prepared invocation does not start new
compilation, but portable fallback, record construction, error strings and
workspace growth can still allocate. A prepared call is therefore not an
allocation-free or real-time guarantee. The runtime's revision-owning snapshots
provide a separate live-publication interface; see
[the runtime guide](unified-runtime-v1.md).

## Route attribution and fallback

Attribution belongs to each invocation, without global mutable counters.
`TypedJit` identifies a successful generated-machine-code result;
`PrecompiledScan` identifies the specialized binary32 list scan;
`NativeScalar` identifies the Windows scalar tier. A declined native invocation
replays the portable evaluator and reports `Portable`, including its precise
error. Feature enablement, successful preparation or `prepare_native` alone
cannot establish that a call executed natively.

The native path reads an isolated snapshot and publishes only a successful
result. It may conservatively decline after partial computation; portable replay
then applies the original charges and determines the first error. Speculation
is restricted to code without externally visible effects. Effectful host calls
stay on the checked host-service VM path so that fallback cannot duplicate a
callback's effects.

Successful native calls can return before portable scratch cleanup. Portable
calls and declines clear retained scratch values on success and failure. Reuse a
workspace across both kinds of outcome when checking changes to these paths.

## Admitted plans and resource semantics

The general typed JIT and Rust AOT emitter admit checked signed `i64`, booleans,
scalar `u64` addition/subtraction/multiplication and equality/ordering comparisons,
small flat signed records, supported branches/loops and bounded acyclic calls.
Unsigned division, remainder, bitwise operations and records with unsigned
fields remain outside this native subset. Binary32 arithmetic, nested collections,
recursive calls, tensors, ML and DSP are not implied. A float-list scan is a
separate specialization.

Unsigned constants, inputs, arguments, locals, calls and results preserve all
64 bits in the existing raw `i64` lane ABI, with exact `Value::U64` boundary tags.
An internal unsigned operation distinguishes carry, borrow and full-product
overflow from signed overflow. Unsigned comparisons use the full `u64` order,
including values at and above `2^63`; Boolean results retain their own tag.
Generated Rust applies `u64::checked_add`, `checked_sub` and `checked_mul` before
returning raw result bits. The canonical module/artifact encodings and AOT
identity derivation are unchanged. Previously admitted signed/Boolean programs
retain their exact emitted Rust bytes.

The dedicated `tests/jit_unsigned.rs` suite compares edge cases with independent
Rust operations, requires actual `TypedJit` results on supported feature/target
combinations, and executes checked-in generated Rust fixtures with per-call
`AheadOfTime` witnesses. It covers exact and insufficient fuel/cell/frame limits,
first-error competition, malformed tags, branches/calls/loops and preserved
fallbacks. A preparation success or a portable result cannot satisfy a native
success assertion. These scalar checks do not complete R10; strict-f32, scalar
collections, broader backend selection and full target qualification stay open.
The Windows scalar leaf emitter retains precedence where its subset applies.

Checked VM instructions lower to typed blocks with explicit local reads/writes,
calls, branches and resource charges. Current bounds include 512 expanded
operations, eight native call-depth levels, 32 boundary lanes and four fields in
an admitted flat record. Operand stacks crossing unsupported block boundaries
cause a decline. Must-initialization analysis intersects predecessor facts,
includes loop entry edges and observes statement order; uncertain loads and
record updates retain guards. It does not change dynamic uninitialized-read
behavior or introduce an independent semantic IR.

The planner checks boundary shape and maximum frame depth before entry.
Straight-line plans can use a static resource bound. The typed JIT also computes
a conservative ceiling for reachable acyclic branch/call graphs during
preparation. It adds each block's charges and every call occurrence, memoizes
shared successors, and takes the maximum fuel and cell totals independently
across alternatives. Checked addition, cycle detection and the existing Plan
size/depth limits bound this analysis. Frame setup is charged once per call;
each record update still reserves the full flat-record copy.

When that ceiling fits the invocation's limits, the generated JIT kernel omits
per-block resource debits. A limit below either ceiling declines before native
entry, even if the selected path would be cheaper; portable replay can still
succeed at that path's exact budget. Cycles, looping callees and unrepresentable
ceilings keep dynamic metering. Arithmetic and uncertain-initialization guards
remain in place. This admission change is confined to the JIT's owned plan;
shared AOT lowering, emitted Rust fixtures and the released AOT binding identity
retain their original metering contract.

For dynamically metered plans, the emitter combines charges along exact
straight-line regions. Function entry and every conditional successor start a
region. Any cycle consisting only of unconditional jumps retains a metered
entry. Following unconditional edges stops at the next region entry or a
conditional/return exit; checked addition computes the exact fuel/cell sum.
An unconditional suffix shared by two predecessors can belong to both regions,
but each executed path reserves that suffix once. Each callee retains its own
charges. No maximum over untaken branch arms is introduced, so exact-budget
loop calls retain their native eligibility.

Every executed prefix is funded before it runs, and a completed path pays the
same total as the original block schedule. An arithmetic trap, uninitialized
read or failed region reservation declines to the original portable evaluator,
which preserves the exact first error. This is safe only for the existing
effect-free native subset; host callbacks are still excluded. Overflow or an
unsupported graph retains the old per-block schedule. The analysis stays within
the existing 512-operation Plan bound and changes only JIT emission, leaving
shared Plan data, AOT output and portable artifacts unchanged.

The region tests independently compare every enumerated path prefix and
complete path against the original block charges, including shared suffixes,
nested and irreducible loops, and unconditional cycles. Public-API tests cover
exact-budget native success, nested/called/early-return loops, competing traps,
record ownership and copy costs, frame limits and extreme iteration counts.
The expanded fragment corpus measures five loop success-path shapes: nested
loops, branches, acyclic helpers inside loops, looping callees and record
updates. Budget and trap stress tests remain separate.

This design uses the general observation that repeated fuel instrumentation
has execution cost, documented by [Wasmtime's fuel configuration](https://docs.wasmtime.dev/api/wasmtime/struct.Config.html#method.consume_fuel).
FluxR retains its own exact first-trap and resource contract through bounded,
effect-free speculation and portable replay. It does not adopt epoch timing or
relaxed trap accounting. Benchmark interpretation follows the repeated-build
and repeated-process guidance in [Kalibera and Jones, *Rigorous Benchmarking in
Reasonable Time*](https://kar.kent.ac.uk/33611/); local empirical percentiles
remain observations rather than population-tail guarantees.

Signed division/remainder hazards, checked overflow, short-circuiting and the
first observable trap remain authoritative. Native admission can reserve more
than a path actually needs: an early-return path may succeed with one frame in
the portable evaluator even when native entry requires the call graph's larger
maximum. Such a native decline must preserve the successful portable result.

Fixed-bound signed-only exports with at most four combined host-input and
argument lanes use a scalar JIT entry. Checked values travel as four `i64`
arguments, with unused lanes set to zero, followed by one result pointer and a
status return. This removes the generated kernel's input-buffer loads and the
larger output scratch buffer. The entry is selected only after the full fuel,
cell and maximum-frame ceiling is known; every invocation still checks exact
input/argument counts, all signed tags and those limits before entering code.
Arithmetic guards and portable replay retain the first-error contract.

Fixed-bound boundaries containing Boolean or unsigned inputs, arguments or
results use a separate tagged scalar owner when all boundary values are `i64`,
`u64` or `bool` and the combined input/argument count is at most four. Its
admission checks each declared tag before packing: Boolean lanes receive only
canonical `0` or `1`, signed lanes accept only `Value::I64`, and unsigned lanes
accept only `Value::U64`. The checked result type selects the returned value
tag. Boolean identity/equality, signed and unsigned predicates, unsigned checked
arithmetic and mixed scalar branches use this entry when their Plan is eligible;
this does not add short-circuit or other previously unsupported Plan shapes. Signed-only calls keep their original owner and packing path.

Both scalar entries use the platform's System V or Win64 calling convention,
including Win64's stack placement of the fifth argument. They are process-owned
JIT code, not public callable pointers or serialized ABIs. Records, larger
boundaries and dynamically metered loops retain the existing lane-buffer entry.
The register-entry optimization preserves AOT output for previously admitted
programs and the released AOT binding identity contract. The scalar-entry
regression suite covers zero through four lanes,
every host-input/argument partition, the five-lane generic boundary, malformed
values, exact/short resource limits and retained portable error order. Tagged
scalar regressions additionally cover every Boolean/signed lane mask and
input/argument partition, both result tags, both Boolean values, wrong-tag
replay, conservative branch ceilings and skipped or competing traps. The
dedicated unsigned suite adds mixed signed/unsigned/Boolean tags, full-width
unsigned results and the generated AOT controls described above.

Signed scalar, tagged scalar, small-buffer and wide-buffer code have distinct
owned variants chosen during preparation. A scalar owner retains its admission
record and registered page; a generic owner retains the full boundary and
registered page. The small
and wide dispatch arms select their fixed buffer capacity directly. Scalar
argument/result scratch remains in the scalar call, so a generic call does not
reserve it or test an optional scalar admission. These internal ownership choices
do not change the generated signatures, the registered page lifetime, boundary
validation, portable replay, or the public route names.

Generated Rust AOT uses the same checked blocks. `emit-native-structured` emits
an eligible export, and preparation binds its kernel to canonical module bytes,
export and ABI version. The unified application builder additionally binds the
checked source and installed revision. Body edits rebuild native caches even
when a caller's checked body is reused. Unsupported AOT shapes retain an
explicit VM route in a compiled application.

## Boundary lanes and ownership

The signed/record ABI uses fixed lane offsets in checked Rust buffers, bounded
input and result arrays, fuel/cell arguments and a status result. Host inputs and
arguments are packed from separate slices while retaining contiguous lane order.
A one-based origin lane identifies an original host record; a different marker
identifies a newly written record. Returning an unchanged admitted record clones
its existing `Arc` and preserves allocation identity. A write constructs an
independently owned result.

The specialized packer for an export with no host inputs and two arguments—a
one-field signed record and a signed integer—checks exact tags and field count,
then writes origin, field and scalar into three lanes. Other shapes use the
general packer. Arity, malformed tags, unused lanes, exact/short limits and
returned ownership remain part of the boundary contract.

A prepared binary32 scan is limited to a checked body that directly loads one
`[f32]` argument or input, applies `minidx` and returns its `u64` result. It borrows
the admitted list and uses the portable strict-comparison loop. For length `n`
and `s` export slots, success requires one frame, `n + s + 3` cells and `n + 3`
fuel. Admission reserves wrapper, element, frame, slot and result cells plus all
scan fuel before comparison. One fallible strict-comparison pass checks each
element's tag while computing a candidate minimum. A malformed tag discards
that candidate and replays the portable path to retain its exact failure order;
no partial result escapes. The portable evaluator uses the same comparison loop
after its ordinary value admission. Selection uses integer ordering of IEEE
binary32 encodings, with unordered NaNs and equal signed zeros handled
explicitly. Pairs retain source-order selection; NaN/zero checks are needed only
when a raw ordering key could replace the current minimum. Every element tag is
still checked. It executes no floating comparison on a signaling NaN and does not
depend on the host's denormal/flush-to-zero modes. See
[the semantic guide](current-semantics.md) for NaN, signed-zero and tie rules.

## Executable-memory ownership

On Linux, the publisher accepts only internally emitted, verified,
relocation-free code. It rejects unavailable or unsafe personality queries and
`READ_IMPLIES_EXEC`. It allocates a private read/write mapping, constructs code
and call-frame information, and changes the mapping to read/execute before
publication. Code and unwind bytes share a bounded 16 KiB allocation; a writable
executable mapping is not published. Gimli emits System V `.eh_frame` at the
actual address. Libgcc registers it after publication and deregisters it before
the owner releases the mapping. Borrowed calls cannot outlive the owner.

Windows uses owned read/execute memory and registered Win64 unwind information.
External relocations and unexpected trap metadata are rejected. These are
bounded native backends with a trusted checker, code generator and operating
system. Memory-permission and FDE tests do not establish hostile-candidate
isolation, all profiler/signal stack-walking behavior, universal unwind safety
or a hard process-memory quota.

## Verification and measurement

Use an external Cargo target directory and the pinned toolchain. The focused
checks and matched benchmark are:

```sh
cargo test --locked --release --features typed-jit --test jit_independent -- --nocapture
cargo test --locked --release --features typed-jit --test jit_acyclic
cargo test --locked --release --features typed-jit --test jit_scalar_abi
cargo test --locked --release --features typed-jit --test jit_scan
cargo test --locked --release --features typed-jit --example bench_typed
cargo run --locked --release --features typed-jit --example bench_jit
python3 -B tools/bench_tokens.py --manifest benchmarks/jit-v12.toml
```

Set `CARGO_TARGET_DIR` as described in [integration](integration.md). Sustained
qualification should use the [resource coordinator](resource-safety-v1.md).
A successful cross-target build is separate from actual execution on that target.

The JIT benchmark checks 2,048 fixed generated rows per workload against checked
Rust and the portable VM, verifies routes, and records 31 rotating-order rounds
of 8,192 consumed-result calls. The four workloads cover arithmetic, a branch
tree, inlined calls and a bounded loop. Report compilation, cold preparation,
cached preparation, first call, destruction and warm execution separately.

Raw Rust kernels exclude host admission. The Rust host control matches the timed
success domain under a sufficient checked ceiling; low-budget error behavior is
checked independently. Warm execution p0 through p100 and p99.9 values describe
batch-average call times, not individual-call tail latency. Lifecycle
distributions measure one phase operation per observation. All raw samples,
sample counts and observation units are retained. The 31 batch observations and
eleven lifecycle observations make p99.9 equal to the observed maximum; they do
not provide a precise population-tail estimate. Process RSS includes host and compiler
storage. Paired diagnostic controls can overlap in setup and result handling;
their differences must not be summed into a purported instruction-level cost.

`benchmarks/jit-v12.toml` is a compatibility-named measurement catalog. Its token
counting does not change executable admission through the frozen
`benchmarks/cases.toml` corpus. Count whitespace, punctuation, wire quoting,
bindings and context explicitly. The Rust-performance and 10%-source-token
objectives require qualified results; no fixed speed ratio is guaranteed.

### Validate a benchmark report

The retained report validator checks the exact Linux x86-64 checked-integer
benchmark contract against source fixtures in this repository:

```sh
cargo run --locked --release --features typed-jit --example bench_jit > ../fluxr-jit-report.json
python3 -B tools/jit_performance_contract.py ../fluxr-jit-report.json
```

An optional `--repository /absolute/source/root` selects another fixture root.
The CLI returns the derived JSON summary with exit code zero when consistent;
invalid input returns a JSON error on standard error and a nonzero exit code.
Input is bounded to 1 MiB. The released `fluxr-jit-performance-v12` label is the
report schema and remains unchanged. Current reports carry the additive
`nearest-rank-all-percentiles-v1` distribution contract, with complete integer
percentiles, p99.9, sample counts and phase-specific observation units. Legacy
reports without that marker remain supported: the validator derives the same
complete distributions directly from their retained raw samples. Existing
`p50_ns` retains its upper-middle definition if an even number of rounds is
requested; the new complete percentile array always uses nearest rank. The
required 31-round contract has no median-definition difference.

Admission requires all four workload/source pairs, the five exact routes
`rust_checked_kernel`, `rust_checked_host`, `portable`, `named_native` and
`prepared_native`, and 2,048 validation calls attributed exclusively to
`TypedJit` for each workload. Warm observations must contain 31 rounds of 8,192
calls with the declared five-route rotating order. Each of `source_compile`,
`cold_prepare`, `first_call`, `cached_prepare` and `module_drop` must have eleven
observations. The validator rejects duplicate JSON fields, wrong source hashes,
missing routes/phases, nonfinite or nonpositive observations and summaries that
do not match the raw samples. It also rejects nonfinite or underflowed derived
ratios. The numerical profile must be strict checked signed `i64`, with telemetry
off and the typed-JIT feature enabled.

The derived summary retains complete warm/lifecycle distributions and compares
every empirical rank and reported percentile of prepared execution against the
portable route, named native entry and matched sufficient-budget Rust host
control. A median improvement cannot hide a worse maximum or an intermediate
rank. The isolated Rust kernel distribution remains separately labeled because
it excludes host admission; it is not treated as an equivalent host API. Supplied
complete-distribution fields are recomputed and checked, including their exact
phase units. An unknown, partial or inconsistent distribution contract fails.

This validates report consistency and fixture binding. A passing summary is not
proof that the reported execution occurred, a new performance measurement, a
timing-target pass, an arbitrary-code sandbox or a result for another platform.
The synthetic validator tests exercise admission only.

### Matched-comparison protocol

Compare the portable route, ordinary named native entry, prepared native entry,
an equivalent checked Rust kernel and a Rust host boundary where implemented.
Bind source identity and paths, toolchain and dependency-lock hashes, OS/CPU and
architecture, features, optimization/codegen settings, instrumentation, workload
seeds and the actual execution route. Use the same machine, release settings,
input distribution and functionality. Every integer arm must use checked
arithmetic; match argument validation, resource obligations and strict float
policy, or label the precise non-equivalence.

Attest the native route before timing. Consume `black_box` inputs and outputs,
exclude logging from hot loops, and exclude warmup while recording it separately.
Retain failed and declined attempts. Repeat with the same source/configuration
and a fixed resource queue. Separate source parsing/checking, code generation,
permission/unwind publication, first call, warm calls, decline/replay, reload
and retained-old-owner costs.

Preserve raw samples and order, iterations/checksums, p50/p95 with minimum and
maximum, CPU and elapsed time, peak RSS, target/dependency size and generated
code/mapping bytes when available. Report per-workload Rust ratios. Estimate
break-even call counts from measured additional cold cost and per-call savings
only when the warm saving is positive. Preparing an immutable call is not
dynamic profile-guided optimization or on-stack replacement; total application
cost includes compilation as well as invocation.

Include .NET only when a matched candidate was actually acquired and executed.
Record `dotnet --info`, executable hash, `TargetFramework`, SDK pin, GC mode and
the values of `DOTNET_TieredCompilation`, `DOTNET_TieredPGO`,
`DOTNET_TC_QuickJit`, `DOTNET_TC_QuickJitForLoops` and `DOTNET_ReadyToRun`.
Require evidence of optimized-tier warmup, checked C# arithmetic and equivalent
first-failure/resource semantics, or state the differences explicitly. Missing
comparisons and measurements remain incomplete; they are not zero or passed.

Further tiering/feedback work requires a demonstrated dispatch or code-quality
cost, bounded instrumentation/code retention and validated state transfer,
cancellation, retry and rollback. Effect-aware transforms require an explicit
pure/effect distinction and rule-specific differential or formal validation of
first traps and logical charges; an unknown result cannot qualify a transform.
Boundary specialization must retain type checks, caps, original record identity
and copy-on-write behavior and be measured across the full matched host path.

To force portable behavior, disable `typed-jit` or call `run_portable_with`.
Retire borrowed prepared handles before releasing their modules. Wider native
subsets, adaptive tiering, cancellation of compilation, persistent code caches,
cache eviction, other platforms and live-agent qualification retain their
explicit implementation and acceptance gates in [requirements](requirements.json)
and [current status](current-status.md).
