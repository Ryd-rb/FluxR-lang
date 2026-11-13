# Optional native compute profile VKF1

VKF1 compiles an already checked FluxR `Module` into a deterministic SPIR-V 1.0
compute module and can execute it through the optional `vulkan` feature. The
portable VM is the independent execution reference. This is an additive profile:
FSM, FA1, FR1, released floating-point observations, trap order, resource charging
and existing backends are unchanged. Kernel identity binds full FSM bytes, export,
profile and output shader. A shader belongs to that immutable revision.

## Accepted subset and ABI

One exported function has 1–8 `f32` parameters and one `f32` return expression.
Its tree consists of parameters, finite bit-pattern literals, addition,
subtraction and multiplication. It has no locals, receiver or host inputs.
Other statements, loops, calls, conversions, division, lists, effects and
nonfinite literals refuse. Expression depth is at most 32 and arithmetic count
at most 128. Local workgroup X is explicit, 1–256, and checked against the device.

Set 0 binds one read-only tightly packed f32 storage array per parameter, followed
by one write-only output array. Array stride is 4 bytes; the sole push constant is
an unsigned 32-bit element count at byte offset 0. The shader checks invocation X
against count before accessing storage, including partial final workgroups.
Storage uses the SPIR-V 1.0 Uniform/BufferBlock representation. LocalSize is fixed
in the shader; specialization constants are not admitted. There is no workgroup
memory, shader barrier, atomics, graphics stage or device pointer in this profile.
These are explicit unsupported acceptance gates, not silently approximated code.

The numeric policy is **explicit GPU approximation**, with finite host inputs
and `NoContraction` on every arithmetic result. The code does not promise VM NaN
canonicalization, subnormal preservation, overflow behavior, cross-device bit
identity or strict IEEE semantics. The fixture uses finite dyadic values and
requires bitwise equality to both the original FluxR VM and equivalent Rust.
Success on that fixture does not promote the whole profile to strict equivalence.

## Ownership, scheduling and trusted native code

`run_tensors` uses the same `cpu_compute::Tensor` and shared payload quota as the
ML/audio path. It requires exact shape equality, reserves host output bytes before
native work, transfers through the existing dispatch, and returns owned CPU storage
only after completion and finite-result validation. Failed shape/quota/output checks
release reservations without changing inputs. There is no implicit broadcasting or
persistent GPU tensor. `TensorDispatch` carries source/shader identity, device and
the explicit VKF1 numerical policy. `Tensor::numeric_profile` describes finite
storage admission; it cannot establish the arithmetic provenance of host-provided
or GPU-produced bits. FTB1 and the portable library profile remain unchanged.

The integration probe uses the same checked ML module for a CPU neural `predict`
job and a Vulkan `affine` job. It admits the CPU job before dispatch and records
whether it remains pending before/after Vulkan completion, then compares both
65,536-row outputs. This is concurrent admission and placement evidence. Software
Vulkan still uses CPU hardware; simultaneous physical GPU execution is not inferred.

`Executor` owns a loader, instance, device and compute queue. Calls require
exclusive mutable access. Each invocation validates shapes, finiteness, host
element budgets, workgroup counts, per-stage and per-set descriptor counts,
storage ranges, and the sum of actual padded device allocation sizes. The uncached
route requires coherent host-visible memory, copies inputs, allocates
the pipeline and binds buffers for each dispatch. Preparation/upload, submit/wait,
download, VM and matched Rust execution are reported separately. This route is an
integration baseline; optional live pipeline reuse has a separate contract in
[vulkan-pipeline-reuse-v1.md](vulkan-pipeline-reuse-v1.md).

The host writes before queue submission; a compute-shader-write to host-read
barrier plus observed fence completion precedes the host read. No output is
returned after cancellation, timeout or device error. Cancellation cannot preempt
a Vulkan driver call or an already submitted kernel. Fence waits poll in bounded
slices. A pending submission retains every buffer/pipeline/descriptor/command
handle and blocks further work until a nonblocking `reap` observes completion.
Submission errors retain ownership because a driver error is not evidence that
work stopped. Device loss is not rewritten as an empty successful result.

Dropping an executor whose completion cannot be observed deliberately quarantines
its one native context until process exit, rather than freeing in-use memory or
blocking indefinitely in `Drop`. This is bounded per executor, not process-wide:
repeated abandoned executors can exhaust memory. Production supervision must
bound context creation and isolate/restart a GPU worker process. Global quarantine
accounting, driver hang recovery, robust device loss and cross-process admission
remain open. Raw ash calls, loader/ICD integrity and native FFI are trusted. Neither
resource accounting nor a hash proves authorization or sandboxing.

## Registry coverage and raw boundary

`fluxr::vulkan::raw` reexports ash **0.38.0+1.3.281**, generated from the matching
Vulkan **1.3.281** registry. `tools/vulkan_registry.py` verifies the exact registry
hash and version, retains every command/type/enum/flag/handle/alias/platform and
`pNext` relation, resolves aliases, and audits each Vulkan-required command against
both its canonical function-pointer type and exact loader symbol. Core and enabled
Vulkan extension requirements are counted separately from Vulkan SC/disabled-only
records; those other records remain in the generated catalog. The catalog can be
regenerated and is not redundantly shipped. Presence is not ABI proof.

Raw calls are unsafe host FFI. Their availability in generated bindings is distinct
from loader support, a device feature/extension being enabled, checked abstraction
coverage, or executed tests. Checked resource wrappers currently cover this compute
route only. Platform surfaces, general `pNext` validation, queues/descriptors beyond
the subset and full FluxR-callable raw dispatch remain required future work.

## Reproduce

Use the pinned Rust toolchain and locked dependencies.
Keep target and temporary directories outside the verified source checkout.

```sh
python3 tools/acquire_vulkan_sdk.py ../vulkan-sdk
python3 tools/verify_compute.py --output ../compute-evidence \
  --target-dir ../target-compute --spirv-tools ../vulkan-sdk/usr/bin \
  --icd ../vulkan-sdk/lvp-local.json
python3 tools/vulkan_registry.py --registry ../vk-1.3.281.xml --fetch \
  --ash-source "$CARGO_HOME/registry/src/INDEX/ash-0.38.0+1.3.281" \
  --output ../registry-coverage.json --catalog-output ../registry-catalog.json
```

Replace `INDEX` with the actual Cargo registry directory. The optional SDK route
downloads hash-pinned Debian packages into a new local directory and records
licenses; it changes no system installation. LLVM and other native dependencies
must resolve. This SDK example selects Mesa llvmpipe, a **software Vulkan CPU
device**. Execution on that route does not qualify physical GPU behavior.
SPIRV-Tools independently validates and disassembles shader bytes. The harness
checks reflected layouts, bindings, workgroups, read/write annotations and bounds
structure, rejects malformed modules, and compares repeated shader builds.

To disable this route, stop new host/compiler calls and observe completion before
releasing native owners. Released encodings remain valid. Retain failed
submissions/evidence and restart a quarantined worker process before reusing its
resource authority; do not treat timeout as completion.

## Source contracts

- [Vulkan registry, pinned 1.3.281](https://github.com/KhronosGroup/Vulkan-Headers/tree/v1.3.281/registry): command/type metadata for generation and coverage.
- [ash 0.38 release](https://github.com/ash-rs/ash/releases/tag/0.38.0): generated binding coverage and unsafe host boundary.
- [SPIR-V core grammar, SDK 1.3.280.0](https://github.com/KhronosGroup/SPIRV-Headers/blob/vulkan-sdk-1.3.280.0/include/spirv/unified1/spirv.core.grammar.json): opcode/enumerant definitions for deterministic emission.
- [SPIRV-Tools](https://github.com/KhronosGroup/SPIRV-Tools): independent shader validation and disassembly; record the exact version used for each qualification run.
- [Vulkan memory model](https://docs.vulkan.org/spec/latest/appendices/memorymodel.html): memory/synchronization context; the live page does not replace the pinned ABI contract.

Generated bindings do not establish all-call execution. Future profiles must expand
semantics explicitly, retaining VKF1 and its refusal tests.

## Exact validator process outcomes

`tools/verify_compute.py` records each real child command before accepting its
result. A malformed-input validation requires the exact expected normal exit code
(currently 1 from SPIRV-Tools), not merely any nonzero outcome. Unexpected exits,
signal termination, launch failures and timeouts remain failed attempts with
retained logs and ledger rows. Deadlines must be finite and positive. Existing
logs/command names cannot be reused to overwrite an earlier receipt. The helper
is trusted-fixture supervision, not hostile-process containment.

`tests/test_compute_verification.py` launches fixed Python child fixtures to
exercise expected 0/1, unexpected 2/7 and inverted success, actual POSIX signal
termination, launch failure, timeout and retained output. It is a required suite
in debug/Python/full verification profiles. A crash cannot be accepted as the
expected malformed-input refusal merely because both outcomes are nonzero.
