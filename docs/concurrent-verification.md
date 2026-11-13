# Concurrent authoring and verification primitives

These are primitives for a many-agent development system. The Rust
APIs concern admitted structured FluxR modules. The Python modules concern local
coordination, test planning and evidence policy. They are independently useful;
there is not yet a trusted end-to-end adapter from the compiler manifest to a
remote execution service or certified Git publication.

## Compiler queries and immutable generations

`Snapshot::dependency_manifest()` exports stable declaration keys, compiler-
resolved direct callees, unregistered helpers, declaration revisions, registered
target IDs and the conservative input/record/enum environment revision. Recursive
calls form cycles; a consumer must traverse a graph or condense strongly connected
components. Source-order indices are not durable external identities.

`Snapshot::affected_targets()` reports registered callers affected by proposed
target writes and rejects unknown target names. `Snapshot::impact_since()` compares
two snapshots, recording changed declarations, edit contracts, added/removed
targets and conservative current affected targets. A target's edit mode is part
of its contract even when execution of the function body is unchanged.

Behavior revisions are memoized once per immutable generation and shared across
parallel readers. A body-only edit invalidates its reverse call closure; unaffected
roots can retain their cache entries. Changing the schema or constructing a new
snapshot uses new state. Tests compare a sequence of warm edits against fresh
compilation and retain old snapshots during concurrent reads. This is a bounded
memo layer, not a persistent incremental compiler with incomplete-program queries.

The query manifest covers the admitted module's resolved calls. It does not cover
build scripts, test discovery, packages, file absence, runtime host values, external
state, toolchain changes or every future effect. A test-impact adapter must supply
those inputs and mark unknown dependencies incomplete.

## Local activation with an epoch

`VersionedProgram` is an opt-in API; `LiveProgram` remains compatible. A host
captures a `VersionedSnapshot` under its publication lock and releases that lock
before compilation, checking or review. `prepare()` and `prepare_checked()` create
a candidate bound to the capture. Activation requires the same source revision,
epoch and opaque program instance. Every successful activation advances the epoch,
including an unchanged source publication. Exhaustion refuses instead of wrapping.

The epoch distinguishes an A→B→A activation history from the original A. A stale
candidate cannot become current merely because source bytes match again. Separate
program instances cannot exchange candidates as bearer tokens. `ProgramVersion`
is serializable reporting data; a deserialized report cannot construct authority.

The host owns the evidence decision, lock and any host-state transaction. This API
does not persist authority across restarts, commit external effects or provide a
distributed consensus service. The supplied tick is receipt metadata, not a clock.

## Test-impact selection

Import `select_tests`, `plan_shards` and `collect_results` from
`tools/agent_pipeline/impact.py` (put `tools` on `PYTHONPATH`). These functions
never run commands. The input catalog must be an independent trusted inventory
of every candidate test and its exact action key. Test nodes in the candidate
graph must match that catalog exactly.

A `fluxr-test-graph-v1` object has `snapshot`, `analyzer`, `policy`, `complete` and
`nodes` fields. Nodes are named inputs, queries or tests with a fingerprint and
sorted unique dependency IDs. A test fingerprint equals its action key. Missing
lookups and directory membership are explicit input nodes whose fingerprints
change when a formerly absent declaration or file becomes visible. Every edge
must resolve to a declared node. Graphs, catalogs and policies are bounded and
detached from caller-owned mutable objects before they are used.

The selector compares node fingerprints and edges across both snapshots and
walks the union reverse graph. It also includes current execution prerequisites
of selected tests. This covers removed edges as well as additions and recursive
components. Missing, malformed, incomplete, mismatched or incompatible graphs
select the full candidate catalog. A dishonest or incomplete extractor cannot
be repaired by this selector; only trusted production analysis may attest that
the graph is complete.

Ordered tests require an explicit `test_order` containing the complete suite in
the order qualified by its runner. The planner preserves the selected projection
in one shard. It does not invent alphabetical order for an unqualified runner.
Selecting a subsequence is valid only if the dependency producer covers test
state/order effects. Execution-order conformance still belongs to the runner.

Parallel shards require a `fluxr-isolated-tests-v1` qualification containing the
exact runner, policy and qualification digest. These identifiers refer to actual
trusted isolation evidence; naming one does not establish isolation. Duration
estimates affect packing only. Stable longest-first packing creates a deterministic
plan that contains every selected test once. Empty selections create no shards
and report `no_tests_selected`, rather than pretending tests executed.

Each result binds the plan, candidate, shard, test and action identities. Missing
rows are incomplete; duplicate/extra/mismatched rows are invalid. Failure, timeout,
cancellation and infrastructure error remain visible. Every row needs retained
evidence references. A passed selected ledger establishes selected-test protocol
completion, not a signature, behavioral equivalence or full-suite acceptance.
Unselected tests still need independently valid reused evidence or a full run.

Until the compiler-to-suite adapter and reuse policy are qualified, keep the
full `verify_checkout.py` profile as the integration gate. Run selection in
shadow mode, comparing it with full validation and recording missed-impact
mutations before enabling it to skip work.

## Exact-candidate evidence policy

`assess_evidence()` in `tools/agent_pipeline/evidence.py` accepts a trusted
requirement ledger, receipt envelopes, a trusted clock, an authentication adapter
and an artifact verifier. The requirement context binds the repository namespace,
candidate and base
snapshots, publication epoch, policy, oracle, toolchain, runner, isolation, platform
and target. Each expected check binds its exact action and complete case ledger.

The authentication callback receives canonical payload bytes and an untrusted
proof and must return a principal already verified by trusted infrastructure.
The artifact callback must verify retained bytes at the named digest. Missing
callbacks, unknown producers, callback errors, mismatched contexts, stale epochs,
expired receipts, missing cases, duplicate checks and failed cases reject.
Receipt hashes establish identity; producer names cannot authenticate themselves.
The test-only authenticator fixture is explicitly not a deployment adapter.

Use a standard attestation system for the production verifier, with isolated
builder identities, explicit verifier policy and revocation. See the current
[SLSA verification guidance](https://slsa.dev/spec/v1.2/verifying-artifacts) and
[in-toto attestation framework](https://github.com/in-toto/attestation). No SLSA
level is claimed by this module. The v1 predicate permits exact-candidate evidence
only; semantic artifact reuse and approval reuse require separate qualified rules.

An eligible decision is not publication. The actual publisher must atomically
compare the current epoch/base and recheck policy, revocation and time validity.
Prepare and validate outside the critical section; hold authority only for that
short decision. Large logs, builds and model calls must not run under it.

## Local job dispatch and recovery

The [pipeline guide](agent-pipeline.md) documents schema migration, atomic
`claim_next()`, bounded candidate windows, fencing and `retry_failed()`. Retry
retains the prior failed receipt and a reconciliation note, advances authority
and supports replay of a durable request ID after a lost response. A changed
command, source or toolchain is a new task, not a retry of immutable inputs.

The indexed conflict planner preserves the original conservative greedy batches.
It still treats unknown analysis as isolated work and keeps cross-namespace
resources conflicting until a trusted mapping proves their relationship. Passing
that planner does not prove a combined candidate correct. Active-scope indexing
improves local dispatch work but does not eliminate SQLite's single writer.

## Qualification sequence

1. Adopt additive query/activation APIs in one host; retain compatibility tests.
2. Run queue migration and fault tests against a copy of local coordination data.
3. Connect compiler queries to a complete, versioned test catalog in shadow mode.
4. Qualify test isolation or record the existing suite's execution order and state
   dependencies. Keep full checks on unknown dependency coverage.
5. Implement a real trusted authentication/artifact adapter and test revoked,
   poisoned, stale, incomplete and replayed evidence across its boundary.
6. Add fenced publication only after exact composed-candidate checks. Integrate
   Git publication and host code/state/effect transactions as separate adapters.
7. Evaluate at increasing agent counts with matched tasks, complete failure/cost
   ledgers, cold/warm caches and measured queue/build/review/publication latency.

For design background, the [Rust incremental compiler guide](https://rustc-dev-guide.rust-lang.org/queries/incremental-compilation-in-detail.html)
motivates complete dependency tracking; [Build Systems à la Carte](https://www.microsoft.com/en-us/research/publication/build-systems-la-carte/)
separates scheduling from rebuilding; [dependent-test-aware regression testing](https://taoxie.cs.illinois.edu/publications/issta20-dependtest.pdf)
explains why selection and parallelization must account for test order. These are
design inputs, not measurements of FluxR or proofs of this implementation.
