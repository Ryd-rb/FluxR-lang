# VKS1: finite typed FluxR parameter specialization

VKS1 is an additive, opt-in compiler/executor profile. It specializes one to
seven original `f32` parameters of the existing checked expression subset at
Vulkan pipeline creation. At least one parameter remains a dynamic storage
buffer. Use `Kernel::compile_specialized` with exact `Specialization` bit values.
The original checked FluxR module remains the CPU reference, including argument
order and its original FSM identity. VKF1, FA1, FSM, FTB1, released encodings,
existing refusal behavior, limits and approximation policies are unchanged.

`examples/vks1-affine.fluxr` is actual delimiter-free FluxR source. Its `scale`
and `bias` arguments become scalar SPIR-V `OpSpecConstant` instructions with
`SpecId` 1 and 2. The executor supplies exact four-byte values through
`VkSpecializationInfo`; it does not rewrite ordinary constants or reconstruct a
different source program. All remaining inputs bind in original parameter order
with specialized parameters removed. All existing shape/bounds/finite admission,
device limits, descriptor/storage layout, ownership and fence rules still apply.

## Identity and resource contract

The API rejects empty, duplicate, out-of-range, nonfinite and all-static parameter
sets before native execution. Metadata is bounded before cloning, with at most
seven entries. Caller entry order canonicalizes by parameter index. Signed zero
bits remain distinct. No mutation API exists for compiled kernels.

Template shader bytes can be identical for different values. `variant_sha256`
therefore binds the VKS1 domain, original FSM hash, exact shader hash, length-
prefixed export name and ordered parameter-index/value-bit pairs. The pipeline
cache compares the entire reflection, actual source and actual shader bytes,
including exact specialization bits. It never treats a shader hash as sufficient
variant identity. Its retained-key payload quota includes specialization entries
and the variant digest. `run_tensors` reports the actual profile and variant pin
alongside source/shader provenance; FTB1 describes finite storage only.

Specialization data and map arrays remain live throughout synchronous pipeline
creation. Offsets are contiguous four-byte entries in native host order, each ID
unique and every size/offset within the data slice. Pending submissions retain
their exact pipeline through observed completion. Opaque driver allocations are
outside logical key/payload quotas; per-executor abandoned-context quarantine
remains the existing limitation. There is no new global driver-memory claim.

## Numeric and supported-subset boundary

The profile remains **explicit approximate GPU f32**, with finite host inputs
and `NoContraction` on each arithmetic result. It does not promise strict IEEE,
subnormal/NaN canonicalization, cross-device bits or a performance benefit.
Specialization does not add workgroup-memory access, atomics, shader barriers,
graphics stages, arbitrary SPIR-V entry or workgroup specialization. All those
inherited gates remain open. The independent portable FluxR VM supplies results
for the same original module with constants reinserted at their parameter slots.

## Qualification

```sh
cargo test --locked --features vulkan --test compute_specialization --lib
python3 tools/verify_specialization.py --output "$WORK/vks1" \
  --target-dir "$WORK/target" --spirv-tools "$WORK/vulkan-sdk/usr/bin" \
  --icd "$WORK/vulkan-sdk/lvp-local.json"
```

The qualifier requires a clean committed source tree and keeps each command's
normal exit, logs and hashes. SPIRV-Tools independently validates/disassembles
the template, checks SpecIds/types/layout/bounds, freezes specialization values
and validates that transformed module. A malformed SpecId decoration requires
normal validator exit 1; signals/timeouts are failures. Repeated emission must
match exactly. The fixed probe executes 180 paired-matrix Vulkan dispatches
across six lengths (including partial workgroups), three constant variants and
five alternating trials, plus one shared-tensor dispatch and five refusals.
Every successful dispatch compares bits against the original FluxR VM and a
matched Rust operation. It records preparation, transfers, waits, actual padded
allocation bytes, cache statistics, setup costs and raw timing samples. Device
identity distinguishes software Vulkan from physical GPU evidence.

Experimental source-token pairs live in `benchmarks/vks1.toml`. The frozen
`benchmarks/cases.toml` replay corpus and admission hashes are deliberately
unchanged. Function cost, specialization context and host/tool costs must remain
separate; static counts are not a live-agent productivity result.

## Research and migration

The [pinned Vulkan 1.3.281 pipelines chapter](https://github.com/KhronosGroup/Vulkan-Docs/blob/v1.3.281/chapters/pipelines.adoc)
specialization section and [pinned SPIR-V grammar](https://github.com/KhronosGroup/SPIRV-Headers/blob/vulkan-sdk-1.3.280.0/include/spirv/unified1/spirv.core.grammar.json)
govern the ABI/opcode choices. Fresh acquisition hashes and exact access depth
are in the implementation pack research ledger. The official
[VkSpecializationInfo](https://docs.vulkan.org/refpages/latest/refpages/source/VkSpecializationInfo.html)
and [map entry](https://docs.vulkan.org/refpages/latest/refpages/source/VkSpecializationMapEntry.html)
pages were checked for bounds, ID uniqueness and byte-size rules; live pages
supplement, rather than replace, pinned authority. No full-spec read is claimed.

Rollback uses ordinary VKF1 with constant-valued dynamic buffers after stopping
new work and observing pending completion. Remove specialized callers before
reverting the implementation group. Keep original modules and evidence; no
released artifact or persistent database migration is required.
