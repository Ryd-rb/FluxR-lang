# Agent implementation operations

FluxR 1 uses the exact source header `fluxr 1` and one mandatory Rust runtime in compiled and injectable applications. FA1, compact, FSM, WK1 and FN1 are compatibility/import formats, not separate current languages. The current executable interfaces are documented in [unified-runtime-v1.md](unified-runtime-v1.md).

This document preserves implementation invariants and the broader target contracts. Proposed data structures and future obligations are not claims of shipped API coverage. [requirements.json](requirements.json) owns task/dependency/acceptance scope; [current-status.md](current-status.md) identifies open qualification boundaries. All 124 full tasks and 12 qualification scopes remain planned.

When authoring WK1, pin the dictionary/profile before generation, retrieve its
exact definitions and count that setup. Use ordinary checked expansion, explicit
collision escapes and bounded execution. Keep a model reply associated with its
originating immutable source/dictionary and existing AH1/AC1 ticket; compression
does not authorize rebinding stale output. Unknown dependencies still require
full conservative validation. The existing fair dispatch, immutable-source/CAS,
semantic rebase, SCC-local compilation and publication gates remain normative.

The process host admits at most one pending preparation and two workers per
session. Compile in the pending process while portable calls continue, validate
readiness, then switch future calls at the host boundary. Cancellation invalidates
the ticket and reaps the pending process. A failed call is returned as a failure
and is not automatically replayed on another worker. Prepared VM declines retain
their separately specified portable replay semantics. No on-stack replacement,
cross-process native pointer or new publication authority is introduced.

The reference tier host serializes calls/promotion with one host lock and bounded retention. It is not a fair multi-tenant compiler service or a qualified scaling result. Linux process/CPU/address/wall limits do not provide filesystem/network isolation or arbitrary-candidate grading admission. Solver unknown/timeouts, missing shards and inaccessible backends remain incomplete.

The <=10% equivalent-Rust source-token target remains open. Scripted transcript costs are static observations; live attempts, repairs, reviews, tools, cached/uncached billing, handoffs and accepted success need complete held-out episode records. Source density is not model competence, and fixture overlap is not throughput scaling.

## Owned completion handles

The tier host admits only the exact ticket object returned by its own `begin_compile`. Equal diagnostic fields, copies or foreign-session handles cannot poll/cancel pending work. Preserve the original handle locally; receipts are observations. Missing handles require closing/reaping and a new bounded session, not reconstruction from hashes.

## AC1 work and response routing

Opt into `query_compact` only under `fluxr-ac1-v1`; ordinary `query` retains AH1.
The model packet has one full draft digest while the host retains the complete
private ticket. Preserve the **originating ticket/request/reply association**
through retries, reviews and handoffs. Never pair an old body-only reply with a
new ticket. Every successful fill creates a new generation; requery it before
another fill. Packet text/digests have no authorization or ticket-deserialization
power. The full existing binding remains authoritative.

Explicit selection names exact records, enums and callable receiver/name pairs.
Validate all selectors before output truncation. Parameters, locals and module
inputs are mandatory; insufficient scope capacity refuses. `visible`, `selected`
and `total` distinguish rendered text, selected budget omissions and deliberate
nonselection. `selected_complete` says nothing about omitted definitions,
dependency closure, effects, capabilities, resource proof or executable admission.
Unknown dependencies still require conservative full validation. BodyChecker and
complete module admission are unchanged.

Output/work/cancellation bounds and AH1 contracts are specified in [agent-context-v1.md](agent-context-v1.md). Explicit selection performs bounded inventory scans; it is not SCC-local compilation or a minimal dependency retriever. Host buffers, drafts/tickets and outstanding requests need budgets; hard cancellation latency and callback safety are separate requirements.

## Token accounting and independent authority

Choose representations from measured full-string costs, including setup,
dictionary/tutorial/retrieval, selectors, raw requests, diagnostics, every
failure/repair, repeated context and complete host records when model-visible.
Retain cached/uncached input/output, tools, review/handoff, latency and accepted
success for actual live trials. Static transcripts and development specialists
are not live trials or held-out evaluation. Preserve the ≤10% equivalent-Rust
source target; context savings do not satisfy it. A focused selector can cost
more than it saves for small scopes once request overhead is exposed.

Keep grading and oracle authority separate from implementation. Use independently specified expectations alongside compiler/runtime differential checks. Hashes and matching artifacts do not establish semantic independence, truthful execution or publication rights. Generated replay requires qualified compilation and execution isolation. Solver unknown/timeouts remain inconclusive; performance gains cannot weaken numerical, ownership or effect requirements.

## Authority, task readiness and exact state

The public FluxR-lang repository and the development repository have independent Git histories. Cross-repository qualification compares both accepted revisions and runs equivalent checks against both checkouts. Record each verified revision and any unresolved coverage in the integration evidence.

## Finite execution

At an interruption boundary, save policy, requests, audit/results and ledger state; stop and reap owned writers before cleanup. Prune only registered outputs whose ownership token, device/inode and protected descendants still match. Source, inputs and durable deliverables are not disposable. Restart and observe the watchdog before new admission. A required terminal audit precedes successful result publication; audit failure remains audit_error and stops the queue. Separate files do not provide a cross-file transaction or durable exactly-once guarantee.

## Work packets, immutability and independent decisions

Each assignment carries task/criterion IDs, invariant domains, exact source/dependency/profile/compiler pins, declared reads/writes/effects, coverage completeness, acceptance commands, resource/output budgets, independent reviewer/oracle role, current lease/fence, evidence destination and rollback. Keep intent, oracle, theorem statement, frozen replay admission and review policy under separate authority. An implementation author cannot make a candidate pass by changing expected answers, dropping failures, relaxing thresholds or altering the grading authority.

Prepare, build, test, analyze and review outside publication. Compose immutable proposals through explicit semantic rebase and conservative conflict/dependency accounting. Unknown imports, negative lookups, toolchain effects, generated files or global invariants require conservative full validation. Only publish under the current expected predecessor/fence and exact applicable evidence; publication itself is short. A local job receipt, declared owner or digest establishes neither authorization nor truthful execution.

Keep fair-ready scan progress, dependency readiness, lease/fence expiry and scope conflicts distinct. An empty bounded window is not global idleness. Only current successful parent completion releases dependency edges; retries/stale/duplicate outcomes cannot release them twice. Retain failed attempts/artifacts before retry. SQLite remains a local single-writer prototype, with schema migration/backup/reconciliation obligations; it is not a distributed publication authority. Never reset fences or treat whole-backup rollback as detected anti-rollback.

## AH1 operational boundary

AH1 is opt-in and starts from an already checked immutable FA1 program. Select 1–512 distinct existing whole-function bodies; the sketch erases them and exposes no executable module while any hole remains. Query tickets have private host-retained fields and exact original source/buffer/profile/compiler/base-semantic/node-index/sketch/generation/hole-plan bindings. Requery after a successful fill; old tickets cannot modify a different generation. Compiler digest equality does not authenticate a compiler or authorize publication.

Model-facing packets list typed parameters/locals/inputs and nominal/callable signatures in deterministic order. Output bytes/items are bounded; oversized items are omitted whole and truncation is explicit. Effects, capabilities and resource obligations remain unproven. A complete signature inventory is not complete semantic dependency knowledge, cost proof or effect certification. Fragment allocation failure is a resource refusal, not an ordinary context omission.

Fill through the released FA1 body grammar and existing authoritative checker; retain immutable old drafts on failure. Every selected body must be filled before the unchanged full module compiler admits executable code and aggregate resource limits. Cancellation is terminal/request-local and checked between bounded phases/items; no hard latency or mid-compiler preemption is qualified. The exact original source is preserved, but equivalent structural fill text can canonicalize to one sketch identity. Retain every raw request, failed response, originating binding and token cost in the host ledger; error values themselves lack revision sidecars.

General malformed-source recovery, expression-hole inference, source spans, constrained model decoding, production incremental/SCC-local compilation and single-flight caches remain open. See [agent-authoring-v1.md](agent-authoring-v1.md). A bounded authoring slice does not close S24/C04 or a release profile.

## Preserve every domain's complete gates

| Area | Required domain obligations |
| --- | --- |
| Language and authoring | Prefix/postfix and structural/DAG editing, contextual references, typed holes, incremental/constrained generation, compact diagnostics, localized retrieval, exact semantic/revision binding and stale/ambiguous rejection; versioned hygienic deterministic learned libraries; independent validation; compatible trained tokenizer/model negotiation and explicit migrations |
| Concurrent agents | Versioned interfaces/invariant domains, immutable candidates, semantic/higher-order conflicts, rebase, declaration/SCC-local compilation, hermetic action DAGs, trustworthy caches, complete dependencies, qualified selective testing, fairness/contention/sharding/authority and crash/retry/partition/rollback/message faults |
| Shared native foundations | Checked dense/sparse tensors and metadata, ownership/lifetimes, asynchronous completion/cancellation, bounded memory/transfers, numerical/RNG policies, capabilities/effects, provenance and versioned ABIs; explicit static/runtime/trusted-native boundaries |
| ML | Full lifecycle, custom trainable FluxR operators/algorithms, relevant autodiff, imported/pretrained and representative non-neural models, batching/streaming/checkpoints/import-export, concurrent CPU/GPU placement and preparation/transfer/latency/throughput/peak-memory measurements |
| Vulkan | Pinned-registry all-call core/extension/platform/pNext coverage, raw access plus checked abstractions, validated/reflected FluxR SPIR-V, shader/ABI/synchronization/numerical contracts and CPU/malformed-input oracles; generated coverage, loader support, capabilities, software execution and hardware remain distinct |
| Audio/music and DSP | Expressive multitrack instruments/effects/automation/sequencing, deterministic offline rendering and independently packaged extensions on the same unchanged runtime binary; separate real-time device/callback/latency/graph-replacement/isolation/lifetime/failure gates, bounded callbacks and tail/miss/CPU/memory measurements |

FSC1 continues to represent bounded data-only CPU worker/offline checkpoints with exact finite state, immutable pins, complete attempt/counter history and remaining budgets. Resume is a local fork, not authorization, truthful history, anti-rollback, durable exactly-once, general optimizer/RNG state serialization or real-time graph migration. Preserve ownership charges, cancellation and failed-work accounting. Software Vulkan and offline audio observations cannot be promoted to physical GPU or callback qualification.

## Checked JIT and prepared-call operations

Use `Module::prepare_call(export)` outside callback/publication/latency-critical execution. It borrows the exact immutable module, resolves the export/native owner once, and exposes no raw entry pointer. Invocation keeps host count/type checks, fuel/cells/frames, strict checked arithmetic and record identity/COW. Each caller supplies its own workspace; shared handles retain their old revision after unrelated publication. Preparation may allocate/compile; invocation may still allocate on record construction, fallback or errors and is not generally callback-safe.

Treat `run_with_route` as per-call attribution: `TypedJit` is successful generated code, `NativeScalar` the retained Windows tier, `PrecompiledScan` an existing specialization, and `Portable` fallback/replay. Decline is not a new observable early error; exact portable replay preserves first failures and logical charges. No effects may escape a failed speculative path. Unsupported forms remain portable.

The typed-JIT platform includes Linux GNU x86-64 with System V ABI, relocation refusal, a 16 KiB combined code/CFI cap, incompatible-personality refusal, RW-to-RX ownership and ordered unwind registration/deregistration. Must-analysis removes only proven redundant initialization guards over a bounded Plan. This does not qualify every backend, asynchronous unwinding, arbitrary-candidate execution, adaptive tiering, global JIT cache budgets or hard preparation latency.

The independent oracle covers checked arithmetic, budget/first-trap comparisons,
failed record writes, initialization, malformed hosts, aliases/revisions,
concurrency and punctuation-free FA1. The acyclic JIT corpus additionally checks
conservative admission against exact path budgets, repeated calls, copy-on-write
identity and retained loop metering. Require actual native-route assertions and
matched measurements; these finite cases do not establish universal proof or
performance parity. JIT-only admission bounds must not alter the released AOT
binding contract or its emitted fixtures.

## Evaluation, research and result interpretation

Keep the <=10% equivalent-Rust source-token target and fair functionality/library baselines. Measure whole strings with punctuation, whitespace, escaping and adjacency. Track bytes, compression, abstractions and vocabulary separately from trained-model competence. Complete episodes account for every attempt, setup/dictionary/retrieval, cached/uncached input/output, tools, repairs, review, handoffs, latency and accepted success. Unknown live-episode fields remain unknown, not zero cost.

Measure cold compilation/preparation, first calls, warmed host/kernel costs, transfers, memory and integration overhead separately. Compare exact equivalent functionality. Batch-average quantiles are not individual-call tail latency. Native parity, .NET superiority and accepted-agent throughput need their own qualification. Separate fixed-total from expanding-resource trials and disclose CPU affinity, contention, every scheduled job and all failures.

The evaluator owns frozen cases and grading authority. Isolate arbitrary-candidate compilation and execution before generated replay. `benchmarks/cases.toml`, `benchmarks/token-replay.rs` and admission rules are protected evaluator inputs; static measurement catalogs cannot authorize execution. Hashes supply neither authority nor truthful execution or semantic independence. Solver unknown/timeouts are not proofs; missing, failed, cancelled, skipped and interrupted outcomes remain distinct.

## Work selection and durable checkpoints

Read the destination repository instructions and the applicable task in [requirements.json](requirements.json). Its start_after edges allow only the stated preparatory scope and do not waive depends_on or acceptance. Preserve unrelated changes. Assign source ownership, declared reads/writes, resource budgets, acceptance checks and an independent reviewer before parallel work.

Measure aggregate disk/quota, inodes, RAM and process capacity; reserve numerical finalization and emergency headroom. Start and observe the resource watchdog before expensive work. Keep targets, toolchains, caches, SDK outputs and temporary files outside verified source. Use bounded compiler/test concurrency, and serialize heavy work when the measured envelope requires it. Record unavailable quota/cgroup constraints as unknown. Register only task-owned disposable roots; stop and reap their writers before cleanup. See [resource-safety-v1.md](resource-safety-v1.md).

Save recoverable source checkpoints and concise resume instructions in durable storage before expensive phases. A checkpoint identifies exact source, configuration, unfinished work and current test status; a prose summary alone cannot replace source bytes. Keep checkpoints outside disposable build roots.

Before handoff, validate the actual extracted final source, checksums and current instructions. Describe source checks separately from product qualification. Handoff exact configuration, failures, current lease/ledger state and next executable steps. For fallback, disable typed-jit or use run_portable_with; let prepared handles drain before owners. Preserve drafts, fences, native owners, checkpoint charges and terminal-audit guarantees through rollback. Continue authorized independent work when an unrelated gate is blocked.
