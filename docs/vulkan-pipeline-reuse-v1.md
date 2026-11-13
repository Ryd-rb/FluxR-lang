# Bounded live Vulkan pipeline reuse

`Executor::with_pipeline_cache(PipelineCacheLimits)` opts into reusable immutable
VKF1 pipeline objects within one executor and its one logical device.
`Executor::new()` retains the uncached policy. The emitted SPIR-V, VKF1 numerical
profile, canonical FSM bytes, shader bounds, buffer ownership and dispatch
resource checks do not change. This extension introduces no new language syntax.

## Identity and admission

A hit requires equality of the complete canonical source FSM bytes, complete
SPIR-V words and every reflection field, including export, profile, layouts,
workgroup and numerical policy. Hashes remain descriptive provenance and are
insufficient for a hit. Canonically equivalent source spellings may share the
same FSM identity. An unrelated source change forces a miss even if the emitted
shader remains equal. Handles never cross an executor/device boundary.

`max_entries` and `max_key_bytes` must be positive. Retained key bytes charge the
source and shader payloads and all owned reflection strings. Fixed-size structure
headers and allocator overhead are excluded; static strings are not duplicated.
The LRU vector scan is bounded by `max_entries`; the complete source/shader byte
comparison is bounded by the already admitted kernel and the retained keys.
Keys exceeding the retained-byte bound execute uncached and increment bypasses.
No unbounded fallback collection or disk driver cache is created.

Eviction happens before submission while the executor has no pending job. The
newly prepared pipeline can transiently coexist with the retained entries before
insertion/eviction, so native objects are bounded by `max_entries + 1` during
preparation, then `max_entries` retained entries. A pending submission also owns
an `Arc` to its pipeline; duplicate owners do not create duplicate native objects.
These limits do not measure opaque driver allocation bytes or process RSS.
`DispatchLimits::max_allocation_bytes` still bounds actual padded Vulkan buffer
allocations. Process-wide abandoned-context quotas and driver containment remain
open requirements, as in VKF1.

## Lifetime, cancellation and failure

Per-submission descriptor pools, buffers, command pools and fences remain
separate from immutable shader/pipeline/layout objects. Partial preparation
releases only unsubmitted objects. Timeout, cancellation or queue errors retain
the whole submitted object graph until a fence reports completion; a cache clear
is refused while a pending submission exists. Cache insertion is not evidence
that a dispatch completed or passed its reference comparison.

The executor explicitly drops cached objects before destroying its device. An
unobserved final fence preserves the existing bounded-per-executor quarantine
policy, now retaining cached owners with the pending job. It never destroys an
in-use pipeline or blocks in `Drop`. This is trusted native execution, not an
arbitrary-code sandbox or a guarantee that a hung driver can be interrupted.

The Vulkan specification requires completion before pipeline destruction and
external synchronization of host access. Exclusive `&mut Executor` operations,
submission ownership and completion checks implement those obligations for this
narrow path. See [vkDestroyPipeline, VUID-00765](https://docs.vulkan.org/refpages/latest/refpages/source/vkDestroyPipeline.html).
The Khronos [Pipeline Management sample](https://docs.vulkan.org/samples/latest/samples/performance/pipeline_cache/README.html)
describes both creation dependencies and resource-object reuse. We use in-memory
resource-object reuse; no opaque `VkPipelineCache` data is persisted or admitted.
These freshly read live pages supplement the unchanged pinned 1.3.281 ABI.

## Reproduce and interpret measurements

```sh
cargo test --locked --features vulkan --lib vulkan::cache_tests
cargo build --locked --release --features vulkan --example pipeline_cache_probe
VK_DRIVER_FILES=/absolute/sdk/lvp-local.json \
  /absolute/target/release/examples/pipeline_cache_probe /absolute/evidence/cache.json
```

Run from the reconstructed source with target and output outside the checkout.
The trusted probe executes the same FluxR affine function on finite dyadic inputs
at 1, 65, 4097 and 65537 elements. It alternates cached/uncached order, records all
nine trials per size, compares every output bit with the FluxR VM, and retains
compilation, pipeline creation, buffer preparation/upload, submit/wait, download
and end-to-end timings. Extra device probes cover workgroup identity, LRU eviction,
cache clearing, key-budget bypass and unchanged admission refusals. No new shader
subset or physical-GPU claim follows from this software-device measurement.

Cache counters describe successful preparation and lookups, including misses
that may later fail buffer preparation; they are not a full native-call trace.
`pipeline_creations` counts newly created pipelines in successfully prepared jobs.
A failed native operation remains an error and is retained by the command ledger.
The default uncached policy and cold-first cached sample remain available in the
report. Source syntax/token comparisons reuse the inherited equivalent affine
pair; static tokens cannot establish coding-episode superiority.

Rollback uses `Executor::new()` or removes the optional caller. Clear an idle cache
before reconfiguration. Retain all failed and timed-out evidence; never reinterpret
a timeout as permission to reclaim pending native objects.
