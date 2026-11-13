# Agent language and development design

FluxR 1 uses the exact source header `fluxr 1` and one mandatory Rust runtime in compiled and injectable applications. FA1, compact, FSM, WK1 and FN1 are compatibility/import formats, not separate current languages. The current executable interfaces are documented in [unified-runtime-v1.md](unified-runtime-v1.md).

This document preserves implementation invariants and the broader target contracts. Proposed data structures and future obligations are not claims of shipped API coverage. [requirements.json](requirements.json) owns task/dependency/acceptance scope; [current-status.md](current-status.md) identifies open qualification boundaries. All 124 full tasks and 12 qualification scopes remain planned.

## Checked common-word compatibility projection

WK1 is an explicit compatibility/import projection over checked FluxR semantics. Its 15 scalar recipes and five statement forms compress sequential syntax. The header pins `wk1 core` or a custom dictionary digest. The core alias is bound to its protocol revision; different meanings require an explicitly versioned contract or pinned custom dictionary. Existing FA1/compact readers and FSM bytes do not acquire implicit new meanings. Current applications use `fluxr 1`.

Custom dictionaries admit 1–64 definitions, at most eight scalar parameters per
definition, at most 64 KiB of JSON, 8,192 charged recipe nodes and depth 32. They
use typed indexed holes and only prior-definition references. Unknown keys,
duplicate/reserved words, ill-typed recipes, bad arities, cycles/forward edges,
unused/reordered first-use holes and excessive expansion refuse. Host-owned
definitions are inspected by reference before bounded cloning. Arbitrary native
code, names, calls, side effects or unchecked ownership operations cannot be
introduced by dictionary template nodes.

Repeated holes require scalar atom arguments; non-atomic expressions must first
be bound explicitly. Recipes are expanded syntax, not hidden eager function
calls or common-subexpression execution. Ordinary lazy Boolean behavior and
left-to-right first-trap behavior remain observable. `repeat` evaluates its count
once and decrements a private counter before the body, including `continue`;
`walk` rereads its atom bound/stride and retains fuel-bounded nonprogress.
The `wk_private_` prefix is reserved. Explicit `local WORD` resolves collisions
when migrating old locals. These are detailed contracts, not arbitrary spelling
substitutions or a general source compression codec.

Source/expanded text is bounded at 256 KiB with separate node/depth admission.
All expanded expressions pass the ordinary checker, including function-local
type obligations; output is inspectable FA1 and the same canonical FSM. The
strict numerical profile retains checked integer arithmetic, binary32 rounding,
signed zero/NaN policy, resource accounting and COW semantics. No implicit
reassociation, FMA, approximate profile or new effect boundary is admitted.

The <=10% equivalent-Rust source-token target remains unestablished. Measure complete strings with pinned tokenizer assets and explicit contexts; this cannot establish single-token behavior on most models, learned vocabulary competence or tokenizer/model-embedding compatibility. Include dictionary, tutorial and retrieval costs and symmetric Rust library baselines.

Criteria S01-AC05, A14-AC08, A15-AC07, A16-AC06 and A19-AC16 define WK1 and accounting requirements. Prefix/postfix, structural/DAG editing, contextual references, typed holes, incremental/constrained generation, compact diagnostics, localized retrieval, immutable binding and stale/ambiguous rejection remain product obligations. Expression templates do not complete general DAG editing, learned-library mining, trained vocabularies, model negotiation or live-agent trials.

## Typed whole-body authoring

The AuthoringDraft/BodyContext API supports typed whole-function body holes, immutable draft references and bounded signature-only retrieval on checked FA1 programs. Existing FA1/FSM1 encodings and meanings remain unchanged. This does not implement arbitrary malformed-source recovery, expression holes, inferred effect/resource safety, SCC incremental compilation, learned vocabularies, custom tokenizers or a complete live-agent protocol. Query tickets are process-local private values, not authenticated serialized capabilities; the host owns error/revision association.

Immutable source snapshots, stateful scalar DAG execution and explicit shader specialization share the semantic infrastructure. Stateful graphs bind checked libraries and deterministic row/node scheduling; previous state is an explicit graph input, and state/output publication is a bounded chunk transaction. Snapshots bind complete tracked source inputs to bytes and modes. Specialization uses `fluxr-vks1-finite-f32-nocontract`, preserving VKF1 and released FSM/FR1/FA1/vocabulary meanings.

The product remains exclusively for autonomous LLM agents. Prefix/postfix
syntax, structural edits, contextual retrieval, typed holes, constrained decoding,
large semantic vocabularies, hygienic libraries and trained tokenizer/model
pairs remain explicit existing work. No static token or graph improvement
supplies evidence of live-agent competence, complete coding-episode superiority,
or the existing at-most-10% equivalent-Rust token target. Unlimited development
capacity preserves bounded experiments, cancellation, isolation and backpressure.

Checked representations include versioned interface identities, immutable edits, shared tensors, external scalar FluxR libraries, typed pure-expression SPIR-V lowering and full-suite shadow observations. Finite storage and producing arithmetic have distinct provenance. Optional approximate VKF1 does not redefine portable float observations or released FSM/FR1/FA1 meanings. Structural/contextual retrieval and learned libraries must bind immutable semantic identities and exact revisions.

The intended programmers are LLM agents. Human familiarity, short files, conventional punctuation and manual editing comfort are not adoption criteria. The objective is reliable accepted work per elapsed time and total model context, with exact language semantics, strong isolation, reproducible checks and practical execution performance. Generated views support human inspection without constraining native authoring.

## 1. Design around agent work, with a small semantic foundation

The durable program should be an immutable, typed graph. Its nodes represent
declarations, types, expressions, effects, requirements, and validated evidence.
The native editing operation should identify a logical declaration or typed
hole, state expected revisions and dependencies, and propose a bounded structural
change. An agent should obtain only the context necessary for that operation,
including uncertainties and negative lookup facts. A source file is one possible
projection and transport envelope; it need not be the unit of ownership,
compilation, testing, review, or publication.

Unison's content-addressed code model demonstrates a useful separation between
names and code identity. Its documentation does not establish that arbitrary
program changes merge safely; FluxR must add its own dependency and invariant
contracts. This is a design inference, rather than a claim of measured FluxR
benefit. [Unison](https://www.unison-lang.org/docs/the-big-idea/)

Keep one semantic foundation with a precise verifier, interpreter, observation
ledger, and versioned cost model. A large surface vocabulary can lower into that
foundation without adding a separate semantic implementation for every spelling.
Every admitted operation needs defined values, errors, evaluation order,
capabilities, resource observations, and compatibility behavior. Backend-specific
shortcuts cannot supply missing language semantics. The existing
[core contract](core-contracts.md) remains binding.

The native agent protocol should support these separate operations:

| Operation | Required result | Refusal condition |
| --- | --- | --- |
| Inspect a typed hole | Expected type/effects, visible symbols, relevant requirements and immutable context identity | Missing or stale environment |
| Query dependencies | Positive and negative facts, affected declarations/SCCs, completeness status | Unqualified producer cannot claim complete coverage |
| Propose a structural edit | Target identity, expected revision, read/write/effect/invariant sets, original proposal identity | Ambiguous target or exceeded resource bound |
| Check a candidate | Exact candidate identity, diagnostics and independently applicable evidence | Any required check absent, stale or unsuccessful |
| Rebase a proposal | Explicit mapping and changed assumptions, new proposal identity | Ambiguous mapping or unreviewed semantic change |
| Publish a certified candidate | Current predecessor/fence/policy comparison and durable receipt | Changed authority, requirements or evidence |

These operations are the design target. Existing source/body projections, conservative contract composition and local queue primitives implement bounded parts of this service.

## 2. Parentheses can be removed; token savings must be measured

FA1 is an explicit compatibility prefix projection of the structured language. Fixed-arity expressions need no opening or closing parentheses; variable-arity calls, collections and blocks carry counts. A complete legacy source example is:

```text
fa1 v2
pub calc 2 x int y int int ret rem add mul x y 17 997
```

This source has no structural punctuation. Counts make boundaries unambiguous
without consulting a changing external declaration signature. The parser builds
the existing AST directly. The checked printer bounds its output, reparses it,
and compares exact AST and canonical FSM bytes. Independent `fa1 body COUNT ...`
edits enter the existing revision-fenced collaboration path. All current
expression and statement alternatives are covered. A valid existing module can
still exceed FA1 text, depth or node budgets and refuse projection. The body
parser supplies the existing edit API; it does not install a new remote protocol. The implemented grammar,
limits, APIs, and CLI are in `docs/agent-source.md` in the source tree.

Tokenizer cost is not a fixed character cost. Replacing punctuation can add counts, operator words, escaping or repeated context. Measure actual complete strings with the pinned tokenizer implementation, including boundaries and whitespace. [tiktoken](https://github.com/openai/tiktoken)

Adopt projections per measured workload and model profile. Do not automatically
replace the existing compact reader, rename old formats, or strip punctuation
from literal data. Future profiles may use postfix streams, typed DAG edits,
contextual argument omission, or binary tool payloads. Each needs an explicit
version, bounded reader, canonical lowering, compatibility tests, and a complete
cost account. If a context-dependent form omits call arity, its pinned interface
must be part of parse identity; a concurrent signature change cannot silently
reinterpret the source. Whitespace and newlines also cost tokens and require
measurement. Zero structural punctuation is an option, not a correctness or
performance theorem.

## 3. A huge vocabulary needs meaning, training, and versioning

FR1 transport has 65,536 frozen codewords. Preserve its released alphabet: changing an entry changes the wire mapping and invalidates existing artifacts. Token counts are model-specific; the same codeword may split differently under different encodings. A correction to a performance claim does not authorize changing released meanings.

Separate four mechanisms: transport compression represents bytes compactly for tools; semantic abstractions name reusable typed behavior; token spellings select model-specific IDs; model competence determines whether the agent can select and compose meanings. A short codeword does not teach its bit pattern to an untrained model. Adding local tokenizer entries does not add trained embeddings or change a hosted model vocabulary.

Fund both vocabulary research and model adaptation. Start with typed abstraction
mining from representative source/edit traces; propose operations that eliminate
repeated semantic work and context. DreamCoder and Stitch provide precedents for
learning reusable libraries in their studied synthesis domains. They support
this research direction, without establishing FluxR gains.
[DreamCoder](https://arxiv.org/abs/2006.08381),
[Stitch](https://arxiv.org/abs/2211.16605)

Every learned abstraction needs an immutable definition identity, parameter
types, effects/captures, capability requirements, deterministic expansion,
evaluation order, trap behavior, and cost semantics. Hygienic expansion must
prevent accidental name capture. Retain an independent checker and the reference
lowering. Existing strict observations, including the first trap and exact
resource thresholds, still apply. If a deliberately different cost model is
desirable, give it a new explicit profile and compatibility policy.

Train or adapt open models on graph edits, multiple projections and structured
diagnostics. Compare pretrained, instruction-trained and jointly trained
language/model pairs on held-out families. Keep model weights, tokenizer,
lexicon, compiler and decoding protocol independently versioned. Evaluate
vocabulary sizes through semantic success, context cost, invalid symbol rate,
training requirement, logits/mask memory, latency and repair burden. A vocabulary
with a million operations may be justified by evidence; its size alone is not
an advantage.

Ephemeral short aliases may reduce repeated names within one session. Bind the
alias dictionary to a revision, model/tokenizer profile and scope. Record the
full immutable identity behind each alias. Reject absent dictionaries, duplicate
aliases, stale revisions, ambiguous Unicode/control-token forms and accidental
retargeting. A reused short spelling may never acquire another meaning inside
an already identified edit. The current FR1 alphabet does not implement this
semantic alias system.

## 4. Optimize the complete coding episode

Use total tokens per independently accepted requirement as a primary measure:

```text
episode_tokens = requirement + retrieved_context + grammar_and_examples
               + generated_edits + tool_inputs_and_outputs + repair
               + verification_interpretation + review + retained_handoff
tokens_per_accepted_requirement = sum(all_scheduled_episode_tokens)
                                  / accepted_requirements
```

Count failures, abandoned branches, retries and reviewer work. Record unknown
usage as unknown. Report input/output, cached/uncached and persistent setup
separately where the provider exposes them. Byte count, prompt occupancy, inference
latency, wall-clock time and runtime performance remain separate dimensions.
Transport data hidden behind a tool must not be represented as if it taught the
model a zero-token program.

If a projection adds setup cost H and saves an average s tokens per successful edit before repair, its idealized break-even point is ceil(H/s) edits when s > 0. The experiment must also include failure, retrieval, repeated context and repair effects. Static syntax cards do not establish live-agent success.

Token Sugar reports token-efficient shorthand with model training on its
workloads. Its reported gains do not establish a tenfold FluxR improvement.
TokenEconomizer studies savings over long task sequences, which motivates
measuring setup and reuse over time rather than one output string. These are
research inputs to the experiment, not production claims.
[Token Sugar](https://arxiv.org/abs/2512.08266),
[TokenEconomizer](https://arxiv.org/abs/2610.22732)

Preserve the repository's target of at most 10% of equivalent Rust source tokens
on the named panel. Use matched observable semantics and comparable libraries;
do not inflate Rust with boilerplate that a competent agent would remove, or
count a preinstalled FluxR abstraction while charging all equivalent Rust
library code in every example. Report source-only and full-episode results
together. Neither the FA1 reduction relative to compact FluxR nor FR1 bytecode
compression establishes achievement of that target.

## 5. Constrained generation and typed context

Generate into typed holes with an immutable environment: expected type, allowed
effects, available symbols, caller assumptions, applicable requirement/invariant
IDs, resource profile and stable context version. Return diagnostics as precise
machine records with the implicated nodes, facts, expected forms and bounded
repair alternatives. Never require an agent to reverse engineer a prose compiler
message when a structured explanation is available.

PICARD and SynCode support incremental parser/grammar constraints. Type-Constrained
Code Generation and typed-hole context work motivate richer admissible
continuations. Their evaluated language fragments and tasks differ from FluxR;
type correctness does not establish application correctness.
[PICARD](https://aclanthology.org/2021.emnlp-main.779/),
[SynCode](https://arxiv.org/abs/2403.01632v4),
[type constraints](https://www.sri.inf.ethz.ch/publications/muendler2025typeawareconstraint),
[typed holes](https://arxiv.org/abs/2409.00921)

Implement bounded incremental FA1 state next. It must track remaining counts,
depth, available names, reserved-name escapes and environment identity. A generic
context-free grammar alone does not enforce arbitrary counted arities; specialize
the grammar under bounds or use parser attributes with a checked implementation.
Cache syntax masks by grammar, tokenizer and environment identity. Keep checks
off a single central per-token service. XGrammar's implementation and dynamic
grammar work offer useful mechanisms to study.
[XGrammar blog](https://blog.mlc.ai/2024/11/22/achieving-efficient-flexible-portable-structured-generation-with-xgrammar),
[XGrammar 2](https://arxiv.org/abs/2601.04426v4)

Grammar-Aligned Decoding documents distribution distortion under naive masking.
Accordingly, compare parse validity, semantic success, search quality and total
latency rather than optimizing parse rate alone.
[Grammar-Aligned Decoding](https://arxiv.org/abs/2405.21047v4)

I15's required delivery is headless machine-client/protocol conformance. An
editor configuration or GUI is an optional projection. A14's existing human
projection and A19's readability/debugging wording mean inspectable diagnostics
for agents and optional human oversight; they do not create a human-ergonomic
promotion veto. The native protocol may be difficult for humans if agents show
better correctness and cost under controlled tests.

## 6. Separate identity, dependency and authority

| Identity | Purpose | Must not be confused with |
| --- | --- | --- |
| Logical declaration ID | Follow one editable entity across revisions | Its current name or body hash |
| Content/definition digest | Identify exact canonical content | Behavioral compatibility or authorization |
| Behavioral/dependency digest | Bind compiler-derived resolved assumptions | A complete proof about every external effect |
| Snapshot identity | Identify an immutable program context | Current publication authority |
| Action identity | Bind declared command, environment and inputs | Hermetic execution or trusted production |
| Evidence/certificate identity | Bind observations and applicable policy | Truth guaranteed by hashing |
| Authority epoch/fence | Reject stale authoritative operations | Source version or worker heartbeat |

Include positive resolutions and negative dependencies. Adding an overload,
method, import, enum alternative, schema member or previously absent binding can
change meaning even when no old dependency edge points to it. Cache keys need
the relevant namespace/interface projection, compiler semantics, target/profile,
cost model and external capability contract. The producer must declare coverage
and its known gaps. Missing coverage triggers conservative recomputation.

Store query results immutably and incrementally by declaration or strongly
connected component. Compare against cold recomputation using mutation tests for
missing edges, added names and stale cache entries. Bound memory and cancellation
work; a fast cache that retains every obsolete snapshot becomes a resource
bottleneck. Salsa's pure input/query model and the scheduler/rebuilder separation
in Build Systems à la Carte are useful prior art, not proof that FluxR's current
dependency extraction is complete.
[Salsa](https://salsa-rs.github.io/salsa/overview.html),
[Build Systems à la Carte](https://www.microsoft.com/en-us/research/publication/build-systems-a-la-carte/)

Action construction freezes the full payload before validation and hashes those same owned bytes, preventing mutation between validation and identity. The digest cannot detect undeclared reads or ambient access; the execution boundary must enforce declared inputs.

## 7. Concurrent design and composition

Give each contract fragment a semantic owner and independent reviewers. Consumers pin accepted versions. Unrelated fragments should compose without editing one central file; a derived registry rejects duplicate IDs and conflicting exports. `agent_pipeline.contracts` implements immutable canonical fragments, exact pins, guarded batch replacements, deterministic ordering and bounded input, validating the whole proposed catalog.

Contract identity separates full content, normative interface and designated documentation. Exact full-content guards still reject stale edits; nested descriptor fields remain normative. Interface-only pins may omit documentation only when the consuming action contract explicitly excludes it. A caller asserting an unchanged digest cannot establish semantic compatibility.

Design requirements form a second dependency layer. Two type-correct changes may
implement incompatible product intentions. Version requirements, decisions,
acceptance oracles and invariant domains; bind them to proposals and certification.
Freeze requirement-unit granularity before throughput trials so that splitting
one task into hundreds cannot inflate accepted-work counts. Changes to these
protected inputs need independent review.

Compose semantic edits by target identity plus read/write/effect/invariant
preconditions. Textual disjointness is insufficient. Stable IDs can still become
ambiguous after deletion/recreation, schema changes or reassignment. A semantic
rebase must expose mappings and changed assumptions and produce a new identity.
The original proposal remains available for review. A three-way or higher-order
invariant may fail even when every pair of edits passes.

Invariant confluence describes conditions under which coordination can be
avoided for a specified invariant and merge model. It does not license arbitrary
coordination-free code changes. The CALM/Hydro talk similarly motivates explicit
state and coordination reasoning. FluxR should exploit proven independence and
retain a short decision for genuinely conflicting domains.
[Invariant confluence](https://www.vldb.org/pvldb/vol8/p185-bailis.pdf),
[CALM/Hydro presentation and reviewed transcript](https://www.infoq.com/presentations/programmable-cloud/)

## 8. Build and test without avoidable global queues

Describe compilation, linking, tests, proofs and artifact publication as an
immutable action DAG. Each action names complete inputs, toolchain, target,
features, environment, output namespace and policy. Use a content-addressed
store for bytes and an action cache for qualified results; these are different
indexes. Execute in isolated workspaces. Concurrent attempts cannot write the
same output directory. Build scripts, compiler plugins and generated code belong
inside the isolation boundary as well as runtime execution.

Bazel's documentation distinguishes hermetic input control from remote cache
lookup and describes action/CAS roles. FluxR's existing hashes and local CAS
provide useful primitives but do not yet enforce those production boundaries.
[Hermeticity](https://bazel.build/basics/hermeticity),
[remote caching](https://bazel.build/remote/caching)

Use short leases for duplicate-work suppression with fencing and recovery; do not hold a global mutex while a model reasons or compiler runs. Shard caches and queues by action/authority domains. Distinguish dependency-blocked from scope-conflicting jobs. Transactional pending counts and the ready index support `claim_ready`/`claim-next --ready-only` without repeatedly scanning dependency-blocked prefixes.

SQLite remains a single-host, single-writer authority. WAL does not make it a
distributed queue or suitable for a network filesystem. Large fan-out completion
still performs work proportional to released edges in a transaction. Measure hot
scopes, large fan-in/fan-out, queue aging, starvation, cancellation, retry storms
and cold migrations before promotion. A production distributed scheduler is a
separate C02/C11 deliverable with specified consensus, sharding and recovery.
[SQLite WAL](https://www.sqlite.org/wal.html)

For tests, create a trusted complete catalog and an independent dependency
producer. Run selective plans in shadow mode against full applicable suites
until missing-edge and mutation tests qualify omission. Unknown inputs or
changed global invariants demand broader checks. Every required shard must have
an explicit terminal result; missing, skipped, interrupted and failed are
different states. Test order and shared external state must be part of a qualified
runner model. More agents should not create more false confidence from incomplete
test ledgers.

Verification commands have their own external temporary directories through child-local TMPDIR/TMP/TEMP values. They do not mutate the parent environment or weaken source-stability checks. Complete profile verification remains required; temporary-directory isolation does not authorize selective test omission.

## 9. Exact composed candidates and short publication decisions

Validate speculation outside publication locks. Queue compatible proposals,
construct the actual composed candidate, and verify its current evidence. A green
leaf branch is not evidence for an unseen combination. Uber's merge-queue talk
and transcript illustrate why independently passing changes can fail together
and why speculation/prioritization matter. Use this as a design precedent; do
not transfer its reported production scale to FluxR.
[Reviewed talk transcript](https://www.infoq.com/presentations/mergequeue/)

Publication compares expected predecessor, exact candidate, current invariant
versions, review policy, evidence completeness and authority fence atomically.
No model call, build or test occurs inside that short decision. Record one
authoritative certified pointer and recover an asynchronous Git mirror with an
outbox or another explicitly verified protocol. Two unrelated atomic writes do
not become one cross-system transaction. Rollback is a new authorized publication
with a fresh epoch and compatible evidence.

Anthropic's Scion report motivates treating high-volume agent revisions and
review identity as first-class design concerns. It does not require abandoning
Git before FluxR has a qualified replacement. Preserve Git history and portable
exports while a graph-native authority is developed and shadow-tested.
[Scion report](https://www.anthropic.com/research/redesigning-source-control)

Production publication requires provider capabilities actually available to the destination repository and account. Verify those capabilities before choosing a hosted merge queue or external coordinator; CI configuration alone does not establish publication authority.

## 10. AI-assisted optimization with independently checked meaning

Allow agents to propose specialized data layouts, superinstructions, vectorized
loops, inlining, kernel selection and learned abstractions. Keep profitability
separate from correctness. A proposal must first satisfy the semantic and
resource contract, then beat the declared baseline on appropriate workloads.
Unsafe floating reassociation, changed first traps, altered short-circuiting,
different aliasing, and weaker fuel checks are not acceptable speedups under the
existing profile.

Equality saturation can explore many equivalent forms and translation validation
can check candidate transformations. The egg and Alive2 papers provide useful
approaches in their stated semantics and bounds; they are not ready-made proofs
of FluxR's interpreter, capability or resource behavior.
[egg](https://arxiv.org/abs/2004.03082),
[Alive2](https://users.cs.utah.edu/~regehr/alive2-pldi21.pdf)

Extend the independent observation ledger to every proposed optimization. Check
raw binary32 bits where specified, NaN payloads, signed zero, bounds errors,
receiver/argument order, retained snapshots, copy-on-write and exact instruction,
cell and frame thresholds. Run differential and metamorphic tests, seeded
semantic mutants, fuzzers and bounded models. Where proofs are required, pin
the theorem, trusted kernel and assumptions. Solver timeout, unknown or missing
proof is a hold or safe reference fallback, not a successful validation.

Unlimited development resources permit more alternatives, proofs and platform
work. Individual compiler, query, decoder and solver operations still need bounds
so one hostile or pathological source cannot monopolize an agent fleet. Budget
exhaustion must be deterministic and recoverable. Runtime targets remain matched
Rust performance, measured cold/warm latency, memory, compilation cost and
platform-specific execution, with the correct portable route available whenever
an accelerated route declines.

## 11. Fund parallel lanes by dependency and evidence

Assign independent implementation, review, formal-methods, benchmark and platform work according to dependency readiness and measured capacity. No fixed staffing or calendar cap is prescribed. Independent alternatives may be developed in parallel without declaring incompatible interfaces stable.

| Lane and task anchors | Next executable increment | Promotion evidence |
| --- | --- | --- |
| Semantics: S01/S03/S04/S05/S23/S25 | One typed core with portable interpreter and independent observation ledger | Bit-exact behavior, negative mutants, compatibility and resource thresholds |
| Native authoring: A14/A15/A16/S02/S17/I15 | Incremental FA1 parser state, typed-hole API and graph-edit adapter | Exact lowering, stale-context refusal, complete diagnostics and live-agent success |
| Vocabulary/models: A10/A19 | Frozen abstraction families, trained-model variants, versioned alias protocol | Held-out correctness and total episode tokens; leakage and dictionary fault probes |
| Queries: S16/S24/C04 | Complete positive/negative dependency producer and SCC-local caching | Cold/warm equivalence, cancellation, retention bounds and missing-edge detection |
| Contract/design: S04/S11/C08 | Structural fragments to qualified semantic compatibility projections and requirement graph | Breaking-change tests, protected policy/oracles, independent composition checks |
| Build/execution: A23/C01/C03 | Hermetic compile/run adapter and complete immutable action DAG | Undeclared-input/descendant tests, reproducible outputs and trusted producer evidence |
| Test/evidence: C05/C09 | Complete catalogs, shadow selection, authenticated receipts and retention | Omission mutants, full ledgers, artifact/provenance verification and revocation |
| Composition: S17/C06/C07 | Explicit semantic rebase and exact-candidate speculation | Ambiguous-ID, negative-resolution and multi-edit invariant failures detected |
| Authority/recovery: C02/C11 | Durable sharded authority with fenced writes and publication recovery | Crash/partition/duplicate/restore traces and stated finite model bounds |
| Runtime/platforms: R01–R22/I21 | Independent optimization candidates, native and Wasm platform qualification | Matched semantics/performance, refusal fallback and executed platform matrix |
| Scale trials: C10/C12/A19 | Preregistered factorial language/harness/model/resource trials | Accepted requirement throughput, latency distribution, quality, cost and uncertainty |
| Release parity: I24/I25 | Actual second repository identification and feature mapping | Equivalent checks on exact accepted heads preserving both histories |

Start with compatible experimental interfaces and contract fixtures. A mock can
unblock isolated preparatory work but cannot satisfy a production dependency.
The included fragment composer reduces central file-edit contention; semantic
review and compatibility remain explicit obligations. No new global manifest
should be edited by every implementation worker when it can be deterministically
derived from independent owned fragments.

## 12. Qualify scale honestly and retain negative results

Benchmark independent work, hot shared contracts, large call SCCs, broad schema
changes, generated code, shared tests and simultaneous failures. Increase agent
counts on independent hardware allocations, for example 1, 4, 16, 64 and 256,
continuing beyond that range when the system qualifies. These are experiment
levels, not a staffing cap or a claim that more agents always help. Separate
planning, implementation, testing, review and publication roles.

Report accepted nonregressing requirement units per hour; p50/p95/p99 time to
acceptance; model/setup/repair/review tokens; actual compute allocation and
headroom; useful versus duplicate work; cache work saved; invalidation precision;
queue/build/test/design/review wait; rebase rejection; failure recovery; and
publication critical-section time. Preserve all scheduled outcomes and the
frozen denominator. Isolate controlled performance comparisons from competing
benchmarks and keep realistic contended-service trials as a separate series.

The architecture aims to eliminate avoidable centralized work and reuse proven
results. Conflicting requirements, incomplete dependencies, finite hardware and
authoritative publication still create real decisions and costs. The development
program funds their reduction and verification without weakening correctness to
claim an impossible absence of all bottlenecks. A rejected syntax, optimizer or
coordination experiment is useful evidence; it remains in the record rather
than disappearing from the success denominator.

## Graphs and authoring qualification

Versioned immutable scalar graphs bind exact library source and monotonic chunk identity. They are host scheduling metadata around actual checked FluxR exports, not a tokenizer or model. Count host/graph setup and context separately. A smaller semantic graph cannot prove model competence. Live coding episodes require authorized provider access and qualified compilation/execution isolation.

## Faster execution preserves authoring contracts

Optional execution tiers and immutable host entry do not change source-language or authoring contracts. Prefix/postfix forms, structural/DAG edits, contextual references, typed holes, constrained/incremental generation, diagnostics, retrieval and source/revision binding retain their obligations. An execution witness does not establish parser coverage, source-span fidelity, SCC-local compilation, model competence or separate admission authority. Stale or ambiguous identities retain their refusal rules.

A `PreparedCall` is a host-side borrow of a checked immutable module/export and
its existing backend owners. It is not an agent-authored serialized capability,
semantic identifier, content-addressed permission, persistent machine-code
cache, or replacement for AH1/FA1 revision tickets. An agent cannot attach an
arbitrary digest, numeric export index or entry pointer to acquire native
execution authority. Generated native bytes are not interchangeable with
source artifacts. Typed holes still require complete filling and ordinary
full compiler admission before execution.

Conservative initialization-guard elimination preserves released dynamic
uninitialized-local and exact budget/first-trap behavior. It is an internal
backend proof obligation over the admitted Plan, not a source-level relaxed
profile or an implicit new definite-assignment rule. Learned libraries,
vocabularies and future rewrites still require versioned hygienic expansion,
deterministic type/effect/resource checks and independent validation. No
fast-math, reassociation, tokenizer or model embedding migration is inferred.

Faster native execution does not reduce model tokens or establish accepted-task throughput. Preserve the <=10% equivalent-Rust source-token target; measure whole strings, punctuation, whitespace, adjacency and JSON escaping with pinned tokenizers. Charge shared host bindings and retrieval symmetrically. Held-out coding episodes must include every attempt, cache/tool cost, repair, review, handoff, latency and accepted outcome under qualified isolation. Static counts and reviewed local programs do not establish live-model or .NET superiority.

## Native interoperability authoring

Canonical FluxR source supports explicit native declarations and length-delimited Rust fragments. They preserve Rust type/lifetime checks and native semantics without changing checked execution contracts. An imported Rust challenge application is an interoperability fixture; equivalent code generation or conformance does not establish independent FluxR authorship, universal API coverage or a faster server. The mandatory runtime remains present in compiled applications.
