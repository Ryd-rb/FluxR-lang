# Concurrency and publication contracts

FluxR 1 uses the exact source header `fluxr 1` and one mandatory Rust runtime in compiled and injectable applications. FA1, compact, FSM, WK1 and FN1 are compatibility/import formats, not separate current languages. The current executable interfaces are documented in [unified-runtime-v1.md](unified-runtime-v1.md).

This document preserves implementation invariants and the broader target contracts. Proposed data structures and future obligations are not claims of shipped API coverage. [requirements.json](requirements.json) owns task/dependency/acceptance scope; [current-status.md](current-status.md) identifies open qualification boundaries. All 124 full tasks and 12 qualification scopes remain planned.

## Bounded tier preparation

The tier host keeps two bound worker instances for one immutable checked module: a callable portable process and at most one preparing/prepared process. One ticket binds generation and exact source/dictionary/executable/configuration identities. Both workers execute the same fully sealed binary snapshot; replacing its former path cannot substitute a worker. Readiness must match exact protocol shape/types, source/dictionary, canonical FSM and expanded-source identities. Only a complete accepted readiness response can publish the future-call route.

Publication occurs at a host-controlled call boundary. No active stack is
rewritten and no executable page, workspace or native pointer crosses processes.
Stale/absent/duplicate tickets, out-of-order replies, ambiguous Boolean integer
values and unsolicited output refuse. Cancellation kills/reaps the preparing
worker, invalidates its generation and keeps the portable worker. Worker failure
cannot silently retry the failing invocation. Event capacity is admitted before
mutation and reserves terminal/close evidence. Returned receipts are detached
copies, not a mutable authority channel.

Preparation stays outside the execution process that serves portable calls.
Its synchronous third-party compiler has a killable process boundary, wall/CPU/
address-space limits and no permitted process/thread creation. The watchdog is
running before source admission and remains active even when polling stops.
Queue capacity is one; saturation refuses instead of growing a hidden backlog.
The current host lock serializes calls and publication. This prototype does not
claim lock-free behavior, fair multi-tenant scheduling, distributed consensus,
partition recovery, on-stack replacement or accepted agent scaling.

These bounded supervision properties do not establish hostile-code isolation,
builder honesty or authorization. Source/artifact hashes provide identity only.
Frozen replay/grading admission and all semantic/higher-order conflict,
invariant-domain, authority, fairness/contention/sharding, crash/retry/rollback
and message-fault obligations remain separate. Keep design, compilation,
execution, review and test selection outside short publication. Unaccounted
dependencies still require conservative full validation.

## Session ownership of completion and cancellation

Completion and cancellation require the exact in-process ticket object issued by the session. Diagnostic value equality is insufficient, including identical source, dictionary, executable, bounds and generation in another session or export. Absent pending state rejects before ticket-field access. Invalid handles cannot change ownership, generation, native selection or events. Valid original handles can poll/cancel; cancellation reaps the owned worker. This is local handle ownership, not distributed authorization or protection from hostile host code.

## Immutable source materialization

A versioned source snapshot captures an immutable Git tree into a bounded CAS
manifest of canonical paths, file modes, byte lengths and digests. A fresh
materialization verifies its complete inventory and every payload before action
binding. Unsupported paths/kinds, stale source identities and incorrect declared
inputs refuse. Full-tree identity conservatively includes tracked configuration,
generated files and test discovery. It is not a complete external-dependency
extractor, hermetic toolchain, trusted execution receipt or publication authority.

Keep source, semantic, action, evidence, lease and authorization identities separate. A checksum cannot authenticate a builder or certify source independence. Compilation, execution, test selection and review remain outside short fenced publication; unknown dependencies require full validation. Shadow suites, higher-order conflicts, fairness, writer contention and recovery have independent qualification obligations. A source-snapshot microbenchmark cannot establish accepted coding-agent throughput or linear scaling.

## Interface identities and test selection

## Versioned identities and qualification authority

Contract v2 explicitly separates full artifact, normative interface and designated
documentation hashes. Exact full-content guards reject stale edits even when an
interface is unchanged. Nested descriptor fields stay normative. Migration retains
canonical v1 artifacts, receipts and restoration; released v1 pins are unchanged.
Interface identity may omit documentation only when the consuming action's
contract excludes it. Retrieval/documentation actions require exact content.

The new shadow comparator binds independent complete catalogs, original proposals,
baseline/candidate/selected snapshots, runner/toolchain/oracle/analyzer/selector
policy and every expected attempt. Missing/skipped/interrupted/unstable results
are inconclusive. Still-passing observation changes, hidden order and three-way
conflicts are tested. All reports require the full suite and deny omission
activation. Hashes do not authenticate results or discover undeclared inputs.

Interface-only hashing can avoid documentation-induced edit churn; it does not demonstrate SCC-local compilation or distributed scaling. Design, compilation, testing and review stay outside publication. Fairness, single-writer contention, sharding, authority, retry/crash/partition/clock/backup rollback and message-loss gates remain. Task-start prerequisite contracts and completion acceptance are separate; development capacity does not weaken execution limits or backpressure.

## Proposed concurrent-agent protocol

## Status, authority and scope

This document specifies target acceptance contracts for C01–C12. It complements [core-contracts.md](core-contracts.md), preserving released language semantics, S03/S05 identities, S16 dependencies, S17 admission and compatibility. MUST and MUST NOT define required behavior of an implementation claiming these contracts. They do not assert that the local prototype implements the entire protocol.

C01 owns work and action envelopes; C02 owns task authority; C04 owns incremental-query records; C05 owns test selection; C06 owns rebase receipts; C07 owns candidate publication; C08 owns contract-change governance; C09 owns evidence admission; C11 owns distributed failure behavior. A shared change needs the affected owners' reviewed version migration. The existing language-contract owner remains authoritative for language meanings. C10/C12 measure the resulting system without relaxing these contracts.

The goal is to perform independent preparation, analysis and execution concurrently while giving every certified result an exact subject and every noncommuting transition an explicit authority. CALM and invariant confluence justify analyzing *which* operations require coordination under a stated model; neither proves arbitrary edits independent. [N01](https://arxiv.org/html/1901.01930v2), [N02](https://www.vldb.org/pvldb/vol8/p185-bailis.pdf)

The JSON below is illustrative data shape, not a released wire format. Angle-bracket values are placeholders, and readers MUST reject them where a digest or real identity is required. A future production schema MUST specify bounds, unknown-field/version behavior, duplicate-key rejection and canonical byte fixtures. Limits are configuration identified by policy; this document invents no production capacity numbers.

## 1. Immutable identities and mutable authority

### 1.1 Separate each identity's meaning

| Identity | Required meaning | Invalid substitution |
| --- | --- | --- |
| `RepositoryId` | Trusted registry identity of the repository and object namespace | Display name, mutable clone URL, or another repository with the same tree |
| `SourceId` | Existing S03/S05 syntax-profile identity and exact source bytes | Whitespace-normalized text or a semantic hash |
| `NodeIndexId` | Exact source identity, node-index algorithm/version and mapping | A bare source span or editor lineage ID |
| `SymbolKey` | S04 resolved declaration identity in its module/package namespace | A global vector offset or current spelling alone |
| `SemanticId`, `CostId`, `AbiId`, `ProgramId` | Exactly the existing core meanings | Git commit, source buffer identity, runtime grant or proof of equivalence |
| `SemanticSccId` | Versioned local semantic encoding of a call SCC and its semantic dependencies | A whole-program index slice or recursively hashing an unclosed cycle |
| `BehaviorPin` | Required semantic, cost, ABI, schema, import and effect dependencies for the stated observation contract | Interface compatibility alone |
| `QueryAddress` | Versioned query kind, subject key and relevant compiler/profile configuration | The conclusion that a cached result remains valid |
| `ActionId` | Complete declared hermetic action identity | Source commit alone or a command display string |
| `ProposalId` | Immutable original patch proposal and its exact preconditions | A later rebased proposal silently replacing the original |
| `CandidateId` | Exact proposed repository tree, parent/base, applied proposals and policy context | An unqualified branch name or a worker's last tested commit |
| `EvidenceId` | Digest of exact evidence bytes | Authenticity, successful execution, authorization or independent review |
| `PolicyEpoch`, task/integrator fencing epoch | Monotonic authority version in the authoritative durable state | A content hash, agent timestamp, UUID ordering or lease expiry alone |

Content-derived IDs MUST use the existing SHA-256 family with distinct versioned domains and unambiguous length-delimited fields. The registry MUST define field order, type tags and byte encoding; it MUST reject overflow and enforce input limits before allocation. String encoding and normalization MUST match the relevant existing contract. No hash input may depend on native struct memory, debug formatting, map traversal accidents, machine addresses, locale or global interning allocation order. An action's transport JSON is not its hash encoding unless a separately versioned canonical JSON encoding is explicitly adopted and tested.

The same source may legitimately produce different results under different profiles. Equal semantic identities do not imply equal source-sidecar diagnostics. Equal content does not grant permissions. A worker MUST NOT choose a weaker host policy by placing its ID in an artifact.

An immutable snapshot reference records at least:

```json
{
  "schema": "fluxr-snapshot-ref/0",
  "repository_id": "<trusted-repository-id>",
  "object_format": "<git-object-format>",
  "commit_oid": "<commit>",
  "tree_oid": "<tree>",
  "source_manifest_digest": "<sha256>",
  "contract_bundle_digest": "<sha256>",
  "language_profiles": ["<exact-profile-id>"],
  "toolchain_bundle_digest": "<sha256>"
}
```

The source manifest covers relevant path bytes, file types, executable modes, symlink targets and content digests under the platform's explicit path policy. Submodules and generated inputs require their own exact identities. A mutable checkout MUST NOT be used as the evidence subject; an action reads an immutable materialization or detects and rejects mutation. File timestamps are never sufficient content evidence.

### 1.2 Query identity must not make every edit global

C04 MUST separate a logical query address from the snapshot-specific record demonstrating reuse:

```json
{
  "schema": "fluxr-query-record/0",
  "query_address": {
    "kind": "<query-kind/version>",
    "subject": "<resolved-symbol-or-syntax-key>",
    "compiler_contract": "<digest>",
    "configuration": "<relevant-profile-and-feature-digest>"
  },
  "origin_snapshot": "<snapshot-digest>",
  "dependencies": [
    {"kind": "symbol_body", "key": "<symbol>", "observed": "<digest>"},
    {"kind": "name_lookup", "key": "<scope-and-name>", "observed": "<lookup-result-digest>"},
    {"kind": "namespace_membership", "key": "<namespace>", "observed": "<set-digest>"}
  ],
  "result_digest": "<digest>",
  "source_sidecar_digest": "<digest-or-none>",
  "completion": "complete"
}
```

The trusted query engine, rather than the proposing agent, records dependency observations. A cache hit requires the current snapshot's dependencies to agree under the query's declared equivalence rule. An unchanged result may stop downstream invalidation only after the producer has been validly revalidated. Cancellation, partial recovery, out-of-budget work and unsupported analysis are distinct states; none is reusable as a completed execution-admission result. Error-tolerant S24 analysis remains a separate result class. Reusing typed semantic results does not justify reusing stale source spans.

Dependency recording MUST include negative and range observations: an absent name, the ordered search path, import/export membership, overload candidates, relevant feature flags, namespace shadowing, wildcard membership and configuration-dependent resolution. Adding a previously absent declaration or changing an earlier lookup scope MUST invalidate queries whose conclusion depended on absence. A direct-callee graph alone is insufficient. An untracked filesystem/environment read, unknown plugin dependency or unresolved dynamic dependency forces conservative invalidation or refusal of reuse. The engine MUST compare incremental results with clean recomputation over adversarial insertion, deletion, rename, shadowing, feature and cancellation sequences. Rust's red/green query design is a useful implementation precedent; its existence does not establish correctness for FluxR's dependency extractor. [N06](https://rustc-dev-guide.rust-lang.org/queries/incremental-compilation-in-detail.html)

### 1.3 Recursive dependencies without cyclic digests

S16/C04 MUST compute strongly connected components over the relevant resolved dependency graph. A component encoding orders members by stable resolved symbol keys and encodes each member's local typed body with deterministic function-local numbering. References within the component use those member keys; references outside it use already computed dependency identities on the condensation DAG. The encoding is hashed once for the whole component. It MUST NOT attempt to define an infinite equation in which each member hash includes the other member's final hash.

Semantic SCC identities exclude source trivia and cost amounts consistently with `SemanticId`; behavioral pins separately include required cost, effect, ABI and schema information. Whole-module canonical serialization may still renumber tables for the existing `ProgramId`. Such table movement MUST NOT by itself invalidate every declaration query. A new compiler-contract version can invalidate affected queries explicitly. This design makes no arbitrary graph-isomorphism, theorem-equivalence or rename-equivalence promise. Recursive value-schema restrictions remain exactly those in `core-contracts.md`.

## 2. Work packets, attempts and fencing

### 2.1 Work and authority are different objects

A work packet is immutable. It includes task ID and version, exact base snapshot, goal and acceptance manifest, declared writable surfaces, required producer versions, selected profile, resource envelope, oracle visibility policy and protected-policy bundle. A mutable task state points to the current packet and attempt. Updating the task specification creates a new packet and an explicit invalidation/review decision for prior results.

```json
{
  "schema": "fluxr-task-authority/0",
  "task_id": "<task>",
  "work_packet_digest": "<digest>",
  "state": "running",
  "epoch": 17,
  "owner_principal": "<authenticated-worker>",
  "lease_authority": "<authority-and-leader-epoch>",
  "lease_deadline": "<authority-clock-value>",
  "last_request_id": "<idempotency-key>",
  "policy_epoch": 4
}
```

Epoch values shown are examples. Epochs MUST be durable, monotonic and never reused for the same authority domain, including after backup restore or leadership change. Their encoding MUST preserve the exact integer in every supported client; an implementation with a bounded counter refuses allocation before overflow and never wraps. A newly acquired or reassigned attempt receives a new fencing epoch. A lease renewal may retain its epoch only through an atomic, still-current renewal. Expiry alone does not stop an old physical process; the recipient of every authority-bearing operation MUST validate the fence. Chubby's sequencer design is a primary precedent for checking generation at the receiving service. [N09](https://storage.googleapis.com/gweb-research2023-media/pubtools/4444.pdf)

The authoritative service serializes claim, renew, cancel, reassign and result acceptance against current state. It uses its defined time source for lease decisions; worker wall clocks cannot extend a lease. A renewal racing reassignment has exactly one valid ordering. A restart or failover that cannot reconstruct a trustworthy deadline invalidates old authority through a new durable epoch before new work is authorized.

For a given authenticated principal, operation type and idempotency key, the authority stores the request payload digest and outcome. An identical retry returns the recorded outcome without repeating its side effect. Reusing the key with another payload is an error. A timed-out client reconciles this record before retrying under a new key. Durable state transition and its durable event/outbox entry MUST commit atomically. Delivery of the event may repeat; consumers deduplicate it. The contract does not promise universal exactly-once execution of external effects.

### 2.2 Accepting a worker result is not certifying it

A worker submits immutable proposal/evidence references with the current task epoch. Acceptance checks current ownership, packet identity, policy, lease condition and content existence, then atomically records a result receipt. A stale result is retained only as explicitly unaccepted diagnostic material. It MUST NOT become the task's successful result by arriving after reassignment. Agent-provided `passed: true` fields have no grading authority.

An already accepted immutable result remains a historical accepted submission after its worker lease expires. Its subsequent validation is bound to that acceptance receipt, not to keeping the worker alive indefinitely. Publication has its own integration authority and epoch. This separation prevents both stale-worker completion and accidental invalidation of legitimate completed work whenever a worker disconnects.

Read-only proposals, speculative worktrees and rejected attempts may coexist. The service guarantees a single authoritative current attempt per task domain, not that only one physical process is running. An agent may prepare another task early only within that task's explicitly allowed `start_scope`; this does not satisfy `depends_on`, profile-scoped completion edges or missing producer contracts.

## 3. Proposals and explicit rebase provenance

### 3.1 Trusted preconditions

Every proposal records an exact base and typed preconditions. These include source/node identity for an edit, expected old operation/body digests, relevant interface and behavioral pins, resolution dependencies, contract versions and invariant domains. Free-form agent reasoning is useful review material but is not an authenticated dependency certificate. A trusted extractor MUST establish the read/write set used to claim independence; otherwise it is `unknown` and receives conservative treatment.

Source edits obey the exact `SourceId`/`NodeIndexId` requirement already present in A07. Semantic compatibility is not permission to apply an offset to a changed buffer. Structured operations must either address an exact current node or use a separately checked relocation certificate.

```json
{
  "schema": "fluxr-rebase-receipt/0",
  "original_proposal_id": "<immutable-original>",
  "original_base": "<snapshot>",
  "new_base": "<snapshot>",
  "rebased_proposal_id": "<new-immutable-proposal>",
  "precondition_checks_digest": "<complete-check-ledger>",
  "node_remap_digest": "<unique-old-to-new-mapping-or-none>",
  "result_tree": "<exact-tree>",
  "rebase_checker": "<binary-and-contract-digest>",
  "decision": "accepted",
  "new_validation_subject": "<candidate-id>"
}
```

A rebase receipt MUST preserve the original proposal and evidence. It records all old/new identities and every discharged or newly required obligation. A source-node map is valid only when the trusted mapper establishes a unique permitted correspondence and verifies the operation's expected old content at the new location. Ambiguous matches, changed resolution, missing dependencies, overlapping writes or uncertain behavioral relationships hold the proposal for regeneration/review. No tool may silently replace an old base ID merely to satisfy admission.

### 3.2 Composed changes need their own validation

A source-level nonoverlap check is a scheduling hint. Semantic read/write independence additionally needs compatible contracts, negative lookup dependencies, effects and invariant-domain observations. Even established pairwise independence is not a substitute for checking a global invariant over a whole batch. SafeMerge demonstrates that semantic merge is a relational verification problem under explicit assumptions; the prototype path planner is not an implementation of that verification method. [N03](https://arxiv.org/pdf/1802.06551)

For an accepted batch, the controller constructs one immutable candidate with a deterministic operation order, verifies the combined typed program and effects, computes its impacted tests, and evaluates all applicable global domains. If the base changes, existing action outputs may be reused only under their own valid identity rules; the candidate certification decision MUST be reconsidered for the new base. A successful old candidate check cannot be relabeled as the new candidate's check.

Ordinary implementation-language patches do not automatically have complete FluxR semantic dependency extraction. Unknown Rust, build-script, foreign-code, macro or generated-code interactions retain conservative file/module boundaries and full applicable checks until a trusted extractor supports them. The system MUST preserve this distinction in receipts.

## 4. Hermetic actions and evidence authority

### 4.1 Action identity

An `ActionId` covers the command as an argument vector, working-directory policy, declared input tree and generated inputs, declared environment, toolchain/runner digests, target platform and relevant host ABI, contract/profile versions, resource envelope, sandbox/network policy and required output schema. It includes the trusted test manifest and grader identity when the action is a check. Secret values are not copied into public manifests: any unavoidable secret-dependent action requires a trusted opaque version binding and restricted cache scope, or is explicitly nonshareable.

Unspecified environment access, current time, random seeds, ambient network, mutable package registries, undeclared host tools and mutable dependency files violate ordinary hermetic reuse. A dynamic dependency discovery phase may produce an exact manifest, but action acceptance requires a verified stable discovered set. An incomplete discovery is not a cache key. Platform declarations must capture differences that can affect outputs; a container-image digest alone does not establish that every host behaves identically.

The scheduler and rebuilder are separate responsibilities: one determines eligible work and resource placement; the other decides whether a prior result is valid. FluxR SHOULD keep those interfaces independently testable, as suggested by Build Systems à la Carte. [N04](https://www.microsoft.com/en-us/research/uploads/prod/2018/03/build-systems.pdf) Hermeticity and a correct declared-input boundary are prerequisites for shared-cache soundness, not properties conferred by hashing. [N08](https://bazel.build/basics/hermeticity)

### 4.2 CAS and attestations

CAS insertion verifies the requested digest against the actual bytes and uses atomic publication of immutable objects. Readers reverify integrity at the trust boundary. Partial uploads cannot satisfy a reference. The action cache maps a complete `ActionId` to a typed result envelope; it is distinct from the byte store. Storage/GC must keep every pinned object reachable from an accepted candidate, active work packet or required evidence-retention record. Cache misses and evictions may cause recomputation; they cannot change semantic outcomes or required checks. Bazel's documentation illustrates the action-cache/CAS distinction and the need to control trusted writers. [N07](https://bazel.build/remote/caching)

```json
{
  "schema": "fluxr-action-attestation/0",
  "action_id": "<digest>",
  "input_snapshot": "<snapshot>",
  "producer_principal": "<trusted-builder>",
  "runner_digest": "<digest>",
  "policy_epoch": 4,
  "outputs_manifest_digest": "<digest>",
  "execution_outcome": "completed",
  "grading_receipt_digest": "<trusted-grader-receipt-or-none>",
  "invocation_id": "<distinct-run-id>",
  "authenticator": "<signature-or-authenticated-service-receipt>"
}
```

A digest demonstrates byte identity. An admitted attestation additionally requires an authorized producer, a verified authenticator and a policy permitting that producer for that action class. The verifier checks the exact subject and authority epoch/key status. Authentication does not prove that a compromised producer executed honestly; the trust model, sandboxing, independent checking and audit evidence remain explicit.

Build completion, process exit zero, test success, semantic equivalence and independent review are different claims. Timeout, cancellation, infrastructure failure, skipped work, unsupported input and test failure MUST remain distinct outcomes. Failed-run artifacts may be retained for diagnosis without being served as success. Reuse rules for negative/failed results require an explicit policy and complete deterministic identity; they may not convert an incomplete check into a passing result.

Nondeterministic checks need a declared repeated-run/seed/acceptance policy and retain all attempts. Performance measurements are invocation-specific evidence tied to the calibrated hardware, concurrent load, repetition protocol and statistical analysis. A cached build may support a benchmark, but an old timing is not a newly executed measurement or proof of current capacity.

### 4.3 The worker cannot edit its grading authority

Normal worker proposals MUST NOT replace the active protected policy, trusted grader, hidden oracle, expected test ledger, producer trust root or review rules used to evaluate that proposal. These surfaces belong to separately authorized governance work with independent review under the previous accepted policy. A candidate changing such files carries proposed content; it does not make that content authoritative during its own evaluation. Publication checks the current policy epoch and revalidates when it changes.

Public tests and candidate-proposed test additions may live in ordinary worktrees. Sealed confirmatory tasks, expected answers and privileged checker credentials MUST be outside worker-readable mounts, accessible repositories and logs. The trusted evaluator operates on a separately controlled exact test/coverage manifest. A staging process records proposed test changes, and an independent coverage owner approves additions/removals or changed expectations. Reducing assertions, changing an expected output or omitting a case cannot self-certify a regression fix. Released hidden cases can become public regressions only through an explicit corpus-version transition; they stop supporting a claim of newly sealed evaluation.

Reviewer independence is a policy obligation naming distinct authorized roles and conflicts of interest; two cryptographic keys alone do not establish independent reasoning. Human or machine review receipts state the actual review scope and limitations. Candidate code, build scripts and candidate-authored tests execute with the declared isolation and limits even when their output is being graded by a trusted service.

## 5. Sound test impact and complete evidence ledgers

C05 may exclude a test only when a trusted, versioned dependency analysis establishes its irrelevance under the selected profile's policy. Impact input includes semantic and interface changes, build configuration, generated code, runtime/resource/cost policy, schemas, resolution/negative dependencies, host boundaries and invariant domains. Observed coverage from past test runs is useful evidence but not a universal proof that unexecuted branches cannot matter.

Unknown extraction, missing metadata, stale dependency graphs, changed test discovery, unclassified feature flags, unsupported external tools or changed oracle policy MUST select the complete applicable test manifest, or hold admission until that full suite can run. Falling back to the full suite is a correctness path with measured cost; it is not a hidden failure to be dropped from scaling results. The selected set and justification are content-addressed before execution.

```json
{
  "schema": "fluxr-check-ledger/0",
  "candidate_id": "<exact-candidate>",
  "policy_digest": "<digest>",
  "test_manifest_digest": "<complete-applicable-manifest>",
  "selection_receipt_digest": "<conservative-analysis-or-full-suite-receipt>",
  "expected_case_ids": ["<stable-case-id>"],
  "accepted_results": [
    {"case_id": "<stable-case-id>", "action_id": "<digest>", "trusted_result": "<receipt>"}
  ],
  "reused_results": [],
  "completion": "complete"
}
```

Aggregation verifies exact set equality against expected cases. Duplicate shard responses do not increase coverage; conflicting duplicates block completion. Missing, crashed, timed-out or silently skipped shards cannot yield success. An explicitly inapplicable case needs a policy-admitted disposition and justification; `not_applicable` is not the same as `passed`. Shard identity includes deterministic assignment, case identity and test-run policy. New cases or changed manifests invalidate the affected selection/aggregation record.

A reuse receipt states the original evidence subject, new subject, unchanged action identity or independently checked equivalence, policy and validity interval when applicable. It MUST NOT rewrite the original attestation. Per-action reuse can be valid across different repository trees when all relevant action inputs match; that does not establish that a candidate-wide invariant or integration check ran on the new tree. Candidate certification aggregates the exact required reused and fresh evidence under the current policy.

## 6. Certified publication and its linearization point

Validation occurs concurrently outside the publication critical section. Publication operates on a closed immutable certificate request, not on a live worker directory. The request names the current expected certified head, candidate tree/commit, accepted proposal/rebase receipts, complete evidence-ledger digest, all invariant-domain observations, reviewer authorization, policy epoch and integration-authority fencing epoch.

The sole publication authority MUST atomically establish that:

1. The expected certified head and relevant invariant-domain versions are still current.
2. The integration authority, authenticated principal and fence remain valid.
3. The accepted policy and required review/coverage decisions are still applicable.
4. The exact candidate has a complete valid certificate and durable pinned evidence.
5. The certified reference advances to that candidate and the durable publication record/event is committed.

The successful atomic compare-and-set transaction is the linearization point. An earlier test finish, reviewer message, prepared ticket, successful Git push or worker result is not that point. If any precondition changed, the request is held/rebased/revalidated; no process simply swaps in a new head while keeping the old certificate. Hosted merge queues similarly check combined candidate states, but the selected repository's hosting eligibility and actual configuration must be verified separately. [N10](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/configuring-pull-request-merges/managing-a-merge-queue)

A database transaction plus an unrelated Git ref update is not inherently atomic. The implementation MUST choose one authoritative certified-pointer service and define recovery for the other representation. For example, a durable certified-pointer ledger may publish a Git mirror through an idempotent transactional outbox; consumers then use that ledger as authority until the mirror catches up. Alternatively a Git server transaction may own the certified ref under a reviewed ticket protocol. A crash between stores cannot make either process invent a second authoritative success. Rollback is a new authorized exact-candidate transition with its own evidence/compatibility decision, not deletion of inconvenient history.

An intake branch may contain speculative unqualified work under an explicit separate identity. Release, deployment, shared SDK consumption and published assurance claims use a certified snapshot. Independent candidate validation can proceed concurrently; changes contending on one publication/invariant domain still require an ordered decision. Separate domains may publish independently only when their registered invariants and dependencies permit it.

## 7. Invariant domains and multi-region failure

### 7.1 The domain registry is part of the correctness boundary

An invariant domain records its stable name, schema/version, member-selection rule, relevant read/write operations, validation predicate/checker, authority owner, current version and interactions with other domains. Examples include unique exported names, an ABI contract, an effect/capability boundary, a package-resolution set, a resource-budget policy, and a cross-module schema migration. Domain membership and absence are dependencies; adding a new member can invalidate a previous validation.

The registry MUST make its coverage assumption explicit. A worker cannot establish independence by omitting a domain. Unknown domain classification or an unregistered global policy change requires conservative coordination and full validation under a trusted owner. A closed-domain result is valid only for the exact registry/version and membership observation it checked.

Pairwise conflict graphs are insufficient for arbitrary global predicates. For example, three disjoint edits each increase one counter from zero to one under the invariant that the total is at most two. Every pair is admissible, but the three-edit batch violates the invariant. A domain constraint may therefore be a hyperedge across a batch, not just an edge between two files. The combined candidate MUST satisfy the domain predicate at the final relevant version. Invariant confluence supplies a framework for deciding when coordination can be avoided under a specified invariant and merge model; it does not certify an undeclared invariant. [N02](https://www.vldb.org/pvldb/vol8/p185-bailis.pdf)

### 7.2 Authority cannot be eventually consistent by accident

Production C11 MUST document its process, storage, network and adversary model; consensus/transaction substrate; quorum and failover rules; clock assumptions; durable-write acknowledgement; and backup/restore behavior. The local SQLite prototype demonstrates only its declared single-authority boundary. It does not establish multi-region ownership, a consensus protocol or atomic cross-shard publication.

For one authority domain, claim/fence issuance, accepted completion and certified publication require a linearizable order. During partitions, a controller without valid authority MUST hold those transitions. It may continue isolated speculative preparation and append nonauthoritative observations, but it cannot certify a new head from an old epoch. Lease duration is an operational tradeoff; safety comes from the checked durable authority and fence, not an assumption that all workers' clocks agree.

Disjoint shards may make progress independently when the domain model proves their current operations independent. A cross-shard change MUST use a specified atomic transaction/commit protocol or rendezvous through a shared authority domain; two independent successful local updates are not an atomic global change. Prepared operations, dead coordinators and retries need explicit recovery states. The chosen protocol may sacrifice availability for the stated invariant during a partition; it must never silently weaken the invariant to report progress.

After restore, the service verifies its authority generation, candidate ancestry, accepted-result records, active packets, evidence reachability and policy/trust roots before resuming acceptance. Restoring an old epoch counter without invalidating old actors is forbidden. Required CAS replication/durability is verified before publication; a missing evidentiary object holds the transition even if an action-cache index exists. Garbage collection observes live transactions and retention pins so that it cannot race a newly committed certificate into unrecoverability.

External effects use independently specified idempotency or reconciliation protocols. A controller crash after an external service accepted a request but before a local acknowledgement cannot be repaired by assuming the effect did not occur. This document preserves the staged host-effect model in the core contract and makes no universal distributed exactly-once claim.

## 8. Linearization, tests and promotion

| Operation | Required linearization/validation boundary | Required negative case |
| --- | --- | --- |
| Claim or reassign | Durable authority transaction allocates the new current fence | Old worker resumes after reassignment |
| Renew | Atomic comparison against current owner, epoch and lease state | Renewal races expiry/reassignment |
| Accept result | Current-attempt check and immutable result receipt commit together | Old attempt completes after a replacement |
| Insert CAS object | Complete verified bytes become immutable and visible atomically | Truncated upload, digest mismatch, conflicting existing object |
| Admit attestation | Trusted issuer/policy and exact subject verification | Forged success, revoked authority, correct bytes from unauthorized producer |
| Admit rebase | Trusted remap/precondition checks bind the new exact proposal | Same span on changed source, namespace addition, changed effects |
| Complete test ledger | Exact expected result set and current selection policy verified | Missing shard, duplicate result, altered expected output |
| Publish | Authoritative exact-candidate compare-and-set commits | Head/policy/fence/domain changes after validation |
| Restore authority | New or recovered valid epoch and complete durable state established | Backup rolls back fencing counter or evidence pins |

C02/C07/C11 MUST retain reproducible fault-injection traces at the transitions above: process termination before/after writes, delayed duplicate RPCs, disconnects, partial CAS writes, stale controllers, concurrent claims, clock discontinuities, leader failover, missing evidence and partitioned domains. A bounded executable state model explores stated finite worker/task/candidate bounds and checks single accepted authority, no stale publication and durable evidence reachability. The report names the bounds and unsupported failures. Passing it is not an unbounded formal proof.

C04/C05 additionally compare incremental and selective execution with clean full recomputation/full applicable tests on retained adversarial workloads. C06 seeds hidden read dependencies, ambiguous remaps, triples violating a domain invariant and proposed edits to the grader/policy. C09 tests hash-valid but unauthenticated evidence, same idempotency key with another payload, replay from another repository/profile and tampering with the expected test inventory. The independent coverage owner verifies that the checker actually detects these injected faults.

Promotion is per declared profile, failure model and executed deployment envelope. C10/C12 report accepted nonregressing requirements, total latency and cost, retries/cancellations, queue/build/test/review wait, invalidation, rebase holds, failover behavior and remaining serial work. Extra resources may reduce some queues; they do not establish absence of coordination, verification costs or physical limits. No production claim follows from a fixture, schema, local unit test or this normative document alone.

## Local evidence and dispatch boundaries

The conservative graph selector consumes a trusted independent candidate catalog and both graph versions; missing/incompatible/incomplete analysis falls back to the full suite. Selected tests include current execution prerequisites. Order must be supplied by a qualified runner unless isolated execution is separately qualified. A selected-test ledger cannot silently certify omitted tests. These obligations do not declare a complete production extractor, runner or reuse policy.

The evidence predicate binds a stable repository namespace, exact candidate and
base snapshots, publication epoch, action, policy, oracle, toolchain, runner,
isolation, platform, target, case set and time window. Authentication and artifact
verification are supplied independently by trusted adapters; neither producer
strings nor hashes establish authority. Publication rechecks current conditions
at its linearization point. This is an exact-candidate eligibility primitive, not
a signed production attestation service or a distributed publication transaction.

The queue's bounded candidate window is distinct from a bounded lock duration.
Active scopes are scanned/indexed once for scoped dispatch, and SQLite retains a
single writer. The local epoch API protects one live program instance; restart
fencing, persistence and host-state/effect atomicity remain production work.
The complete C01–C12 contracts above still govern full task acceptance.

## Dependency readiness and structural composition

Schema 4 adds a derived dependency-ready index under the same local SQLite
authority. Successful current parent completion and decrementing its children's
remaining counts commit together. Failure/cancellation, stale completion, retry
and duplicate success cannot release edges early or twice. Migration derives
counts from retained dependency history, checks foreign keys and refuses
inconsistent active/terminal states. Missing pre-existing authority tables are
corruption, not a request to create empty replacement state.

`claim_ready` avoids dependency-blocked prefixes; scope-conflicting ready rows
may still need scanning. An empty bounded cursor window is not a global idle
certificate. Fan-out release still costs work per edge in one transaction.
The schema remains single-host/single-writer. Downgrade and authority restore are
explicit recovery operations, not code-version switches or counter resets.

Contract fragments give independent owners immutable records, exact dependency
pins and guarded replacement batches. The catalog checks all requirements and
export collisions after composition. It neither authenticates owners nor
interprets invariants. Content pins remain conservative, including prose;
semantic compatibility projections need an independent qualified producer.
Requirements and design decisions form their own versioned dependency layer,
since compiler independence alone cannot prove compatible product intent.

The verifier supplies child-specific temporary directories outside the source
checkout. This prevents fallback-related source drift without weakening the
endpoint inventory guard or granting hostile-execution isolation. All required
suites still run. The provider collector rejects unfinished item lifecycles and
retains attempted tool events even when a corresponding completion is missing.
The experimental FA1 benchmark catalog is separate from byte-pinned executable
replay authority. Changing a counting corpus cannot authorize generated code.

## Persistent fair sweeps

The opt-in fluxr-fair-ready-v1 schema retains the sweep ceiling while scans advance
past hot scopes; new arrivals wait for a subsequent sweep. A missing caller
cursor cannot repeatedly reset this progress. Lease, active scope, event and
cursor changes commit atomically, with explicit migration and process-crash
fixtures. This is finite candidate-scan progress under admitted authority, not
a writer-acquisition, wall-time, tenant-share or network-partition guarantee.
SQLite remains one local writer; unavailable dependencies remain unclaimable.

Benchmark fixed-total CPU allocation separately from expanding resources. Include waits, conflicts/retries, accepted jobs and result payloads; preserve adverse scaling outcomes. Dependency indexes, full-suite shadow qualification, independent evidence authority and higher-order conflict checks remain required. See [agent-fair-dispatch.md](agent-fair-dispatch.md).

## Finite development admission

One shared coordinator budgets expensive acquisition, build and native qualification; research/edit/review parallelism stays within measured aggregate headroom. Bounded compiler/verifier concurrency, an active prelaunch watchdog, numerical reserves, external source-bound targets, durable recovery checkpoints and ownership-token/device/inode cleanup are required for the resource-controlled workflow. These controls do not weaken semantic conflict, fairness, sharding, authority, crash/retry/partition/rollback/message-fault, short publication or unknown-dependency full-validation obligations.

FSC1 local stateful resume preserves a complete attempt/budget ledger but creates an explicit local fork. It adds no queue authority, distributed recovery, anti-rollback or exactly-once publication claim. Actual fixed-total/expanding-resource synthetic queue observations are distinct from accepted LLM-development throughput.

## Execution ownership and publication

Native preparation binds the checked immutable module/export and exact target,
ABI, compiler and feature configuration. A `PreparedCall` borrows those owners
and cannot be rebound to a replacement module. Concurrent retained revisions
therefore remain distinct subjects, and caller-owned workspaces and local
scratch remain disjoint across calls. No argument pointer is retained by the
emitted function. Cache-once native initialization is process-local and owned
by its module; equal source hashes or numeric indices do not authorize
cross-process executable reuse. Full dependency accounting, trusted producer
receipts, persistent cache admission, declaration/SCC reuse and bounded cache
churn retain their separate R16/C03/C04/C09 requirements.

Design, compilation, code protection/CFI registration, preparation, benchmarking and review occur outside short fenced publication. Performance results grant neither merge authority nor semantic independence. Failed preparation leaves prior immutable owners and the portable runtime usable. Synchronous third-party compilation and sampled trusted supervision do not qualify hard compiler-preemption, allocation-complexity bounds or arbitrary-candidate sandboxing.

Targets, caches, temporary SDK outputs and repeated builds stay outside verified source, bound to its exact content, toolchain and configuration. Only explicitly registered disposable paths may be pruned after their writers stop. Source, inputs, durable recovery records and deliverables remain protected. Resource scheduling retains all fairness, contention, partition/message-fault, crash/retry/rollback and fixed-total/expanding-resource acceptance gates.

## Native concurrency

Explicit native interoperability exposes Rust threads, Arc, synchronization, channels, atomic types/orderings, async syntax, sockets and database APIs. Rust owns Send/Sync and borrowing. These calls need no checked-VM transition, while the application retains the runtime. Checked fuel and cancellation are not promises about arbitrary native code. Qualification must exercise contention, wakeups, closure, timeout, panic/join, ownership rejection, real socket errors and live database transactions/recovery; compile-only examples and protocol mocks are insufficient.
