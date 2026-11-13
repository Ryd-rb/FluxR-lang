# Checked CPU libraries, ML and offline music — experimental v1

This additive slice executes FluxR programs for a custom trainable neural model,
non-neural scoring and independently packaged instruments. It preserves the
released FSM/FXH readers, evaluation order, float behavior, logical ownership and
resource accounting. It does not promote the complete native-domain roadmap.
Heavy backends remain optional; these CPU modules add no dependency.

## Contracts and ownership

`cpu_compute::Tensor` owns immutable contiguous row-major finite binary32 values.
Shapes have at most eight dimensions; scalar rank zero contains one element;
zero extents contain none. Rank, product and byte calculations are checked before
allocation. A zero extent cannot conceal overflow in other extents. Values keep
signed-zero bits. Non-finite inputs refuse this explicit new profile rather than
changing the existing language's numerical semantics. The only implemented
`Device` is CPU, dtype is f32, and alignment is `align_of::<f32>()` (four bytes on
the qualified host). GPU transfer and cross-device ownership remain separate
unimplemented contracts. Layout metadata is descriptive of this actual storage;
it is not a promise that a different backend can accept the buffer.

Tensor clones share one immutable allocation and charge its payload once. There
is no mutable tensor view or implicit copy-on-write. `MemoryBudget` uses atomic
admission to cap live tensor/CSR/render-array payload across concurrent requests,
tracks a lifetime peak for that shared budget (including earlier requests), and
releases the charge with the last owner. It does not count
allocator metadata, shape handles, event/session/timing buffers, compiled code,
Rust host inputs, serialization buffers, OS thread stacks or VM workspaces. These distinctions are explicit:
structural count limits bound graph/events, per-invocation `Limits` bound FluxR
logical cells/frames/instructions, and the qualification runner separately
reports full-process peak RSS. No process-wide hard memory guarantee is claimed.
An allocation failure after reservation releases its charge. Cancellation and
traps never publish a partial output tensor.

`SparseCsr` checks terminal/monotone row offsets, in-range strictly increasing
column indices and finite values. Duplicate coordinates refuse instead of
silently choosing a summation order. CPU matvec multiplies then accumulates in
stored order without fusion and checks finite intermediates. It is a Rust host
primitive, explicitly attributed as such; FluxR scalar algorithms use
`ScalarSession::batch` over dense rows. Cancellation is checked between CSR rows,
not at every nonzero inside a row.

`FTB1` is a new little-endian tensor encoding: four-byte magic; rank/u8, dtype/u8
(1=f32), device/u8 (0=CPU), flags/u8 (0); rank u64 extents; exactly the expected f32
bit patterns. Decoding checks all fields, lengths, overflow and finite values.
Unknown flags, devices and dtypes refuse. It does not reinterpret released
language encoding bytes. The `encode` convenience returns an additional
host-owned buffer and that output allocation is outside `MemoryBudget`.

## Executable package ABI

A package directory contains `manifest.json` and `program.flux`. The manifest
selects `fluxr-scalar-library-v1`, version 1 and the exact
`fsm2-finite-f32-portable-v1` numerical profile. It records an exact source SHA-256,
package identity, unique export names with scalar f32 arities, and the one allowed
capability `compute.cpu`. The loader bounds manifest/source sizes, rejects
ambiguous duplicate export keys, validates the actual public function signatures,
and refuses ambient inputs or any other requested capability. The trusted host
chooses which package bytes to load. A supplied hash establishes byte binding;
it does not authenticate a publisher or grant permission. The compiled program
identity hashes the domain/profile plus canonical module bytes; it intentionally
excludes package display identity and other manifest metadata, so equivalent
code in differently named packages has the same program identity.

The existing compiler type-checks the whole module. All calls use the existing
portable evaluator with explicit instruction/cell/frame limits. Every admitted
call attempt consumes the session call budget, including wrong export/arity,
non-finite arguments, language traps and rejected non-finite results. Successful
calls have a separate counter. Cancellation before admission consumes no call.
The cancellation flag is checked between calls; an already-entered call runs
until completion or its finite VM limit. This is cooperative cancellation, not
native instruction preemption. No compilation happens as a side effect of a
portable call. Sessions own their mutable scratch workspace and cannot be shared
mutably between threads. Streaming callers may submit rows one at a time without
materializing a full tensor; batch results preserve row order.

`CpuBatchExecutor` supplies optional asynchronous CPU completion. Its caller
sets a finite positive maximum number of in-flight jobs. Admission refuses at
capacity and does not accumulate a hidden unbounded queue. Each job owns its
inputs, receives a dedicated cancellation flag, and produces a complete output
or error through `wait`. The implementation currently creates one OS thread per
admitted job; thread-creation overhead is part of measured integration cost.
Dropping a job receipt requests cancellation and detaches the finite worker; the
worker retains its permit and owned memory until it exits. Dropping a receipt is
not success, and it does not free in-flight buffers early. Explicit waiting is a
host operation and is not permitted in a future real-time callback. This local
executor provides neither distributed authority nor crash-persistent receipts.

## ML path and limits

`examples/native/ml/program.flux` defines one hidden ReLU unit with four trainable
parameters. Forward-mode dual-number addition, multiplication and ReLU are
implemented in FluxR, including the explicit derivative choice of zero at the
ReLU kink. Exported gradient/step operations execute inside the FluxR evaluator;
the Rust host supplies deterministic dataset order and applies simultaneous
parameter results. This is a small operator-level AD library, not a compiler AD
pass or a general reverse-mode tensor differentiation system.

The qualification driver trains on four deterministic pairs, measures preparation
and training separately, compares all final parameter bits with a matched strict
Rust operation schedule, and requires summed half-squared error below `1e-7`.
The finite-difference test provides a separately structured gradient check away
from the ReLU kink. The tiny dataset is a correctness fixture, not evidence of
useful generalization or a representative large-model workload.

The `fluxr-model-v1` checkpoint binds parameters to the exact compiled program
identity and preserves raw parameter bits, completed-step count, RNG seed and
provenance text. The checked tiny pretrained fixture contains analytical
parameters for `ReLU(x)*2+1`; it is imported through the same bounded decoder and
must predict 2.25 at x=0.625. This route is a FluxR checkpoint import, **not ONNX
or a vendor model-zoo backend**. Provenance text is descriptive, not an attested
training history. Export/restore has exact-bit checks; wrong identity, shape,
version, size and non-finite parameters refuse. The demo uses deterministic
initial parameters and no random draws; a general RNG/checkpoint protocol remains
pending.

Additional real FluxR exports implement an affine statistical model, a decision
stump, nearest-centroid classification (first center wins ties) and a rational
distance kernel. They have exact scalar reference checks. Dense inference uses
4096 rows; four independently owned CPU sessions execute asynchronously with
shared program identity and bounded memory. This establishes concurrent CPU
placement, not concurrent CPU/GPU models. General training for tree/kernel/
clustering families, large pretrained imports, mixed devices, preprocessing
pipelines and production streaming remain required work.

Native-speed training performance is not established. VM dispatch, repeated
boundary validation and scalar record work are potential bottlenecks; measure
them against an equivalent optimized Rust algorithm. Optimization must retain
independent behavior checks and all released logical costs and trap order.

## Music path, extension proof and real-time boundary

`offline_audio::render` loads external instruments with the uniform
`sample(phase, age, velocity) -> f32` ABI. Supplied triangle, pulse/pluck,
triangle-harmonic bell and deterministic-noise packages are separate source and
manifest directories. The driver binary embeds none of their source. The
qualification runner first stages and loads triangle, adds the separate bell
package, then loads it with the same unchanged executable bytes. It records both
package behavior identities, different output bits and a SHA-256 of the binary
before and after every invocation. This demonstrates a FluxR program-package
extension without rebuilding Rust. It does not claim a CLAP/VST/native dynamic
library ABI.

The compact score is an eight-bar, 16-second, 24kHz stereo composition with 176
notes across bass, syncopated pluck, bell melody and percussion. It has a repeated
A-minor/F/C/G progression, note velocity variation, linear pan automation, a
fixed-order low-pass filter and a feedback delay. The instruments execute in
FluxR. Scheduling, mixing, filtering and delay execute in the Rust host and are
identified separately in reports. No untestable artistic-quality claim is made.
No copyrighted samples or large audio assets are needed. Sample playback,
expressive note controllers, richer synthesis/modulation and dynamic tempo maps
remain requirements beyond this slice.

Tick-to-sample conversion uses checked integer rational arithmetic from PPQN=960
quarter-note ticks and tempo in milli-beats/minute, avoiding cumulative clock
rounding. Notes occupy half-open sample ranges; ends at sample n precede starts
at n. Overlap is checked before rendering; stable start/track/event order fixes
floating-point accumulation order. Frame/track/event/active-voice/call bounds,
finite/nyquist frequency checks, gain/pan/filter/feedback validation and checked
memory admission reject malformed configurations. A zero-sample note refuses.
Cancellation is checked per block and again at every instrument call. The whole
output becomes visible only after successful completion. Shape and sample-rate
policy is explicit; there is no implicit audio-device resampling.

This is an **offline renderer only**. It has no audio device callback, device
clock, underrun recovery, device latency or safe concurrent graph-replacement
claim. VM workspace allocation/destruction, metadata allocation and host cleanup
have not been eliminated or certified for callbacks. Block p50/p95/p99/max wall
latencies and deadline comparisons are observations under a simulated block
schedule. `render_wall_over_audio_ratio` is render wall duration divided by
audio duration, not measured CPU utilization. The process runner records user
and system CPU time separately. A low observed deadline-miss count does not
qualify real-time behavior.
Graph replacement is currently possible only by finishing/cancelling a render
and preparing another immutable composition outside execution. External native
plugins need a separately versioned ABI and isolation/lifetime enforcement.

## Reproducible validation

Use the pinned Rust toolchain with locked dependencies and writable target/cache/
temporary directories outside the source checkout. Then:

```sh
cargo test --locked --lib --test cpu_compute --test offline_audio
cargo test --locked --release --lib --test cpu_compute --test offline_audio
cargo clippy --locked --lib --test cpu_compute --test offline_audio --example native_domain_demo -- -D warnings
cargo build --locked --release --example native_domain_demo
python3 tools/qualify_native_domains.py --binary /absolute/target/release/examples/native_domain_demo --output /absolute/outside-source/evidence
```

The runner records commands, exit statuses, timeouts, stdout/stderr, wall/user/
system time and Linux `wait4` peak RSS. It executes repeated ML and audio runs
under the same binary and requires identical behavioral/checkpoint results and
audio tensor hashes. It is for trusted fixtures and is **not an OS sandbox for
arbitrary evaluated candidates**. `native_domain_demo audio examples/native
REPORT.json OUTPUT.wav` generates a WAV outside the source from the supplied
score and programs.

`benchmarks/native-domains.toml` is an experimental token-count-only catalog. Its
triangle FluxR/Rust pair has equivalent scalar operations. It does not alter the
frozen executable replay catalog or its pins. Actual tokenizer results must be
recorded, including failure of the 10%-of-equivalent-Rust target.
Algorithm source length does not establish full-episode cost or agent superiority.

## Design references and native trust

The [DLPack Python specification](https://dmlc.github.io/dlpack/latest/python_spec.html)
defines ownership, device support, stream coordination and version negotiation.
These concerns motivate separating actual CPU ownership from a future interchange
adapter. The current API does not implement DLPack.

The [CLAP plugin lifecycle header](https://github.com/free-audio/clap/blob/main/include/clap/plugin.h)
separates activation, processing and extension lifetimes. Preparation,
callback-valid data and object ownership need distinct contracts; the current
FluxR program-package API does not establish CLAP compatibility.

The [ONNX IR specification](https://onnx.ai/onnx/repo-docs/IR.html)
separates format, operator and model versions. The FluxR checkpoint is not
ONNX-compatible. DLPack/ONNX/CLAP integration requires measured adapters and independent
malformed-input qualification rather than format-name claims.

Baydin, Pearlmutter, Radul and Siskind's
[automatic-differentiation survey](https://arxiv.org/abs/1502.05767v4)
distinguishes AD from symbolic differentiation. The implementation uses explicit
dual-number rules whose behavior requires its own tests.

Rust/compiler/allocator/OS/CPU floating-point behavior and the existing checked
FluxR compiler/evaluator remain trusted native components. Tests are evidence of
this bounded implementation, not a proof of all numerical, scheduling or memory
properties. Disable new callers and drain owned work before removing optional
modules/packages. Released encodings/profiles and frozen replay artifacts retain
their meanings. Existing data files under this ABI must be retained and
explicitly migrated if any future incompatible version is introduced.

## Optional tensor transfer integration

The Vulkan `Executor::run_tensors` bridge shares these immutable CPU tensors
and payload reservations. Host output is reserved before dispatch and is published
only after completion and finite checks. `TensorDispatch` retains the generating
source, shader, device and approximate VKF1 numerical policy separately. FTB1 does
not become a GPU allocation format and does not claim arithmetic provenance.
The integration probe schedules this module's neural `predict` on the CPU and its
pure `affine` export on a Vulkan device before collecting both results. A software
Vulkan device still executes on the CPU; physical GPU model execution needs its
own qualification.

The qualification runner rejects NaN/infinite and nonpositive timeout values before
creating an output directory or launching a process. It continues to execute only
reviewed fixtures; this wall deadline is not an arbitrary-code sandbox.
