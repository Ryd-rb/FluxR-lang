# Implementation scope and open qualification

The pack supplies current source, build inputs, tests, examples and implementation contracts for integration. **All 124 full tasks and 12 qualification scopes remain planned.** A source feature being present, a passing local check, and complete product acceptance are distinct states. Packaging cleanup grants no additional runtime, platform, performance or release claim.

## Current architecture

FluxR 1 is the single current source language, with the exact header `fluxr 1` and project format `fluxr-project-v1`. Compiled and injectable programs retain the Rust-implemented runtime. FA1, compact, FSM, WK1 and FN1 are explicit compatibility/import formats. The 12 release profiles are qualification scopes, not alternative language versions.

Checked execution and trusted Rust interoperability coexist in the same source format. Rust owns native borrowing, lifetimes, layout, unsafe effects, atomic orderings and panic behavior. Checked code reaches only typed native services granted by its application. The runtime does not compile newly injected arbitrary Rust or add new crate dependencies at run time; those changes require rebuilding the application.

## Source and contract map

The table identifies available implementation areas and the documents that define their scope. It is an integration map, not a completed acceptance ledger.

| Area | Main contract and source entry | Qualification boundary |
| --- | --- | --- |
| Canonical source and project builds | [unified-runtime-v1.md](unified-runtime-v1.md); `tools/fluxr.py`, `src/structured/unified_source.rs` | Actual compiled entry, runtime catalog, standalone export and negative build behavior require execution checks. |
| Runtime revisions, live control and native services | [unified-runtime-v1.md](unified-runtime-v1.md); `src/runtime.rs`, `src/unified_cli.rs` | Expected-revision publication, retained snapshots, exact service signatures and source-bound AOT must agree. No on-stack replacement or automatic state migration. |
| Checked semantic core and optimization | [core-contracts.md](core-contracts.md); `src/structured/` | Exact values, first traps, logical fuel/cells/frames, floating bits and documented ownership/COW remain binding. The proposed common S05/S23 core is broader than existing backend coverage. |
| Bounded semantic differential campaigns | [semantic-campaign.md](semantic-campaign.md); `tools/semantic_campaign.py`, `examples/semantic_campaign_driver.rs`, `tests/fixtures/semantic-campaign/` | Independent signed-integer/Boolean result and first-trap checks, seeded branches, replay, bounded reduction and mutation sentinels. No compound, recursive, floating-point or release-wide model coverage is implied. |
| Typed JIT and prepared calls | [prepared-execution.md](prepared-execution.md) | Linux GNU x86-64 and Windows x86-64 backends for admitted Plan shapes, including checked scalar unsigned add/sub/mul/comparisons. Acyclic JIT graphs can pre-admit a conservative fuel/cell ceiling; tight budgets replay portable and loops retain exact straight-line region metering. AOT identities are unchanged. Actual target execution is required; preparation is not evidence of a native hit. |
| Source projections and typed authoring | [agent-source.md](agent-source.md), [agent-authoring-v1.md](agent-authoring-v1.md), [agent-context-v1.md](agent-context-v1.md) | Bounded whole-body drafts and explicit context selection do not establish arbitrary malformed-source recovery, expression holes, semantic rebase, SCC-local compilation or model competence. |
| Wordcode and process tier preparation | [wordcode.md](wordcode.md), [wordcode-tiering.md](wordcode-tiering.md) | Exact dictionaries/expansion and live session handles; Linux x86-64 supervisor, copied IPC values and serialized host calls. No arbitrary-candidate sandbox or real-time guarantee. |
| Source snapshots, CAS and action identity | [source-snapshot-v1.md](source-snapshot-v1.md), [agent-pipeline.md](agent-pipeline.md) | Declared source/input identity is not complete dependency extraction, hermeticity, producer authentication or publication authority. |
| Queue readiness and fairness | [agent-fair-dispatch.md](agent-fair-dispatch.md), [concurrency-contracts.md](concurrency-contracts.md) | Single-host SQLite has one writer. Candidate-scan progress is not a tenant-share, wall-time, distributed consensus or network-partition guarantee. |
| Interface composition and test selection | [contract-fragments.md](contract-fragments.md), [contract-interfaces-v2.md](contract-interfaces-v2.md), [test-selection-shadow.md](test-selection-shadow.md) | Structural pins are not semantic proofs. Omission requires independent complete dependency/catalog qualification; unknown coverage selects the full applicable suite. |
| Evidence, provider events and evaluation | [concurrent-verification.md](concurrent-verification.md), [agent-pipeline.md](agent-pipeline.md) | Complete outcomes, protected grading authority and separately trusted producer/artifact checks remain required. Hashes and self-reported success do not authenticate execution. |
| Native buffers, models and offline audio | [native-domains-v1.md](native-domains-v1.md), [native-stream-v1.md](native-stream-v1.md) | Payload quotas are not process/allocator/driver quotas. Full ML lifecycle, arbitrary formats and real-time callback qualification remain separate. |
| Stateful streaming and checkpoints | [native-stateful-v1.md](native-stateful-v1.md), [native-checkpoint-v1.md](native-checkpoint-v1.md) | Atomic chunks and bounded data-only resume preserve state/attempt/budget pins. Resume is a local fork, not anti-rollback, authentication, distributed exactly-once or general optimizer/RNG state capture. |
| Vulkan lowering, reuse and specialization | [vulkan-vkf1.md](vulkan-vkf1.md), [vulkan-pipeline-reuse-v1.md](vulkan-pipeline-reuse-v1.md), [vulkan-specialization-v1.md](vulkan-specialization-v1.md) | VKF1/VKS1 approximate arithmetic is explicit. Registry coverage, shader validation, driver availability and actual physical-device execution are independent gates. |
| Native interoperability and challenge fixture | [native-rust-v1.md](native-rust-v1.md); `examples/multicore-server/` | Imported Rust is attributed interoperability. Finite capabilities/conformance do not prove unrestricted Rust coverage, independent all-FluxR authorship or challenge victory. |
| Development resource supervision | [resource-safety-v1.md](resource-safety-v1.md) | Measured admission, watchdogs and task-owned cleanup are trusted local supervision, not hard kernel quotas, hostile-code isolation or a durable cross-file transaction. |
| Deterministic clean source packaging | [release-pack.md](release-pack.md); `tools/release_pack.py`, `tests/test_release_pack.py` | Pack 25 validates safe unique paths, canonical modes, source digests, exact extraction and repeated same-runtime ZIP bytes. It preserves pins/licenses/functional fixtures and excludes generated outputs, recovery material and dispatch prompts. Archive consistency does not authenticate producers or complete I22/runtime/platform acceptance. |
| Current-language syntax highlighting | [highlighting.md](highlighting.md); `syntaxes/fluxr.tmLanguage.json`, `tests/highlighting.cjs` | Lexical current/legacy scope fixtures use real TextMate/Oniguruma. Coloring does not validate source, recover exact native UTF-8 byte spans or implement the proposed language-server/authoring protocol. This editor-support slice leaves compiler, runtime and testing APIs unchanged. |
| Executable Rust-to-FluxR cookbook and test runner | [cookbook.md](cookbook.md), [testing.md](testing.md); `examples/cookbook/`, `src/testing.rs` | Finite paired examples, bounded checked tests, seeded scalar/flat-list properties and explicit native Cargo test forwarding. This does not establish full Rust framework parity, all-language safety or universal performance/token targets. |

## Open acceptance boundaries

The native Plan supports checked scalar `u64` addition,
subtraction, multiplication and equality/ordering comparisons in the typed JIT
and Rust AOT emitter. Exact unsigned tags and all 64 value bits are preserved;
overflow and resource declines retain portable replay. Dedicated Rust edge
oracles and per-call generated-JIT/compiled-AOT witnesses define the narrow
verification boundary. Unsigned division/remainder/bitwise operations, unsigned
records, strict-f32 arithmetic and native collection extensions remain outside
this slice. Existing fallback behavior, canonical artifacts, AOT identity rules,
resource accounting and the signed-only entry remain binding. **R10 and its
broader acceptance criteria remain open.** No performance gain or additional
platform qualification follows from this type extension.

The property-testing slice supports flat bool/i64/u64 lists with bounded length,
generated cells per case and aggregate generated cell work across selected
properties, including repeats, comparison backends and shrink reservations.
Deterministic shrinking tries list deletions and
scalar element reductions within the declared domains, preserves the original
failure signature, and shares one 128-evaluation limit across all arguments.
Scalar seed sequences, existing catalog readers and per-call runtime limits
retain their contracts. Empty lists, replay and admission boundaries require
the focused tests in `tests/testing.rs`; these finite capabilities do not close
broader I19 acceptance or provide arbitrary recursive strategies.

The semantic campaign supplies an independent bounded scalar model and frozen
expectations for portable/prepared comparison. It covers signed arithmetic,
Boolean short-circuiting, assignments, nested branches, exact observable error
text and the declared fuel/cell/frame boundaries. Replay recomputes expectations;
reduction preserves the selected discrepancy class within a finite budget;
mutation checks pair controlled faulty programs with passing controls. The campaign
uses the existing compiler and runtime interfaces. A native-execution claim
requires a witness in that retained run. The full S25 feature matrix, arbitrary
programs, recursive calls, ownership, floating-point and release-wide coverage
remain open; no product acceptance follows from a finite campaign.

The current optimization slice includes distinct scalar-register entries for
fixed-bound signed-only and tagged Boolean/integer JIT calls with at most four
combined input/argument lanes, and a fused validation/minimum pass for the
precompiled binary32 list scan. The tagged entry preserves exact lane/result
tags, conservative entry budgets and portable replay; it does not change the
Plan lowering for previously admitted programs or the AOT binding identity.
Finite decimal binary32 source literals
preserve rounded bits and negative zero, while raw-bit forms retain all IEEE
encodings and compatibility readers. Canonical printing selects decimal only
when shorter and exact on reparse. Other shapes
retain their existing native or portable route. Distinct scalar and generic JIT
owners avoid optional scalar dispatch and scratch on buffer calls. Shorter
`bump`/`scale`/`sub`/`div`/`rem` updates and signed `next`/`prev` unit steps preserve the expanded
AST and canonical bytes. Native syntax scanning borrows unquoted token slices;
quoted JSON tokens retain their decoding and validation. Checked-text projection
size is admitted before output allocation, and canonical-print validation moves
its owned AST instead of cloning it. The regression suites specify exact
budget, trap, type, ownership and source-admission obligations.

Dynamic typed-JIT execution groups exact charges along unconditional paths.
Function entry, conditional successors and residual unconditional cycles retain
metering; shared suffixes are reserved once on each completed path. This reduces
repeated checks without a maximum over untaken branches. Tight-budget calls keep
their existing native eligibility; failed reservations or arithmetic guards replay
the portable evaluator. Analysis overflow retains the original block schedule.
For already admitted plans, this metering optimization preserves Plan lowering,
AOT identity, caller-owned values and canonical artifacts. Graph-prefix and
whole-path oracles, nested/called/record loops, exact
budgets and competing first errors define the regression boundary.

Direct local-list `empty`/`last` execution borrows the local and avoids a temporary
wrapper reference-count operation. Both original instruction slots and their
ordered fuel charges are retained; unfused-oracle tests cover competing errors,
control flow, nested ownership and workspace cleanup. This does not extend the
native collection subset.

The [fragment measurement protocol](performance-fragments.md) covers a finite
60-case cross-section of checked operations with all empirical integer percentiles and
p99.9, separate observation units and actual route witnesses. The matched
checked-Rust/JIT report now exposes the same complete distributions while
retaining its released raw format. An optional strict comparison gate checks
every observed rank and retains failures; its outcome is not a population-tail
confidence claim. Earlier exact fragment catalogs remain readable. The fragment collector caps main/startup output
and process time, retains failed attempts, and checks exact workload catalogs;
its bounded process collection requires POSIX and an outer resource guard for
escaped descendants. Offline report validation remains portable. Current-source
round trips, compiled applications, native services and compatibility readers
have additional gates. A bounded source probe separates parsing, compilation
of the checked projection and checked canonical printing, with native span and
artifact checks, including finite and exceptional float literal workloads. Repeated paired builds/runs and same-binary controls remain
necessary for small performance claims. Unmeasured operations/platforms, statistically qualified
population tails and improvement at every percentile across the language remain
open. The token-pair counter measures static source and context costs; it does
not establish live-agent gains or the full Rust-token ratio.

The final combined runtime/application/export path requires complete source-bound qualification in the integration environment. The supplied implementation status does not establish that whole gate. In particular:

- Native callback panic catching requires `panic=unwind`; an abort profile retains native process-termination behavior. Native callbacks supply their own I/O deadlines, cancellation and side-effect policy. VM fuel cannot interrupt arbitrary trusted Rust or roll back external effects.
- Filesystem Unix-socket support must be exercised on the destination platform. This capability remains unresolved for full acceptance; source availability or a TCP pass cannot substitute for it.
- Generated arbitrary-candidate compilation and execution require a qualified isolation adapter and protected grading authority. Bounded subprocesses, seccomp process-creation refusal, timeouts, checksums and a userspace watchdog do not establish that complete boundary.
- Universal Rust-speed parity, .NET superiority, the **source-token ratio at or below 0.10 of equivalent Rust**, held-out live-agent success and accepted-development-throughput gains remain unestablished. Keep kernel, host entry, lookup, startup, compilation, injection and publication costs separate.
- The pinned multicore comparison target is **more than 230,000 held users** under the specified shared-host, database, load-generator, warmup/search/hold and error/latency conditions. It is a retained evaluation target, not a statement about a current public leaderboard. Official conformance and a matched-hardware performance score are separate gates.
- Full native ML/training/import/export, all-call Vulkan/SPIR-V coverage, physical GPU execution, independently packaged DSP and real-time device/callback/tail-latency requirements remain open where their complete checks are unmet.
- Production distributed scheduling, authenticated remote caches, semantic/higher-order conflict handling, dependency completeness, certified publication and crash/partition/restore behavior require their full acceptance scope. A local queue or fixture benchmark cannot close them.
- Windows, ARM64, Wasm and device behavior require actual target execution for execution claims. Cross-target compilation is a separate, narrower observation.
- Cross-repository parity is checked per integration checkpoint. Record the accepted revisions, shared source inventory and equivalent conformance results for each checkout.

## Acceptance data

[requirements.json](requirements.json) is the canonical current contract: **124 tasks, 940 active acceptance criteria and 12 qualification scopes**. It contains task ownership, implementation steps, target paths, dependency/start-order rules, risks, rollback conditions, promotion rules and complete evidence obligations.

Repeated fields are defined once under `defaults`. Omitted `start_after` means `depends_on`; omitted optional dependencies mean an empty list. Each criterion's `evidence` names the complete required-evidence list in `evidence_profiles`. Task `reference_sources` resolve through the root URL map. Target paths are planning hints and may name files to create; they are not file-existence assertions. All criteria inherit open status until independently accepted. Scope closures and shared-contract ownership remain explicit.

Broader target contracts are in [core-contracts.md](core-contracts.md), [concurrency-contracts.md](concurrency-contracts.md) and [agent-design.md](agent-design.md). Day-to-day preparation, authority, resource and recovery rules are in [agent-operations.md](agent-operations.md). Run the checks in the [integration guide](integration.md) against the integrated candidate; do not infer unexecuted results from these documents.
