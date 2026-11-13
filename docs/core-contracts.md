# Core implementation contracts

FluxR 1 uses the exact source header `fluxr 1` and one mandatory Rust runtime in compiled and injectable applications. FA1, compact, FSM, WK1 and FN1 are compatibility/import formats, not separate current languages. The current executable interfaces are documented in [unified-runtime-v1.md](unified-runtime-v1.md).

This document preserves implementation invariants and the broader target contracts. Proposed data structures and future obligations are not claims of shipped API coverage. [requirements.json](requirements.json) owns task/dependency/acceptance scope; [current-status.md](current-status.md) identifies open qualification boundaries. All 124 full tasks and 12 qualification scopes remain planned.

## Checked expansion and worker boundaries

WK1 expands into the retained checked FA1/FSM program. It changes neither the
released artifact ABI nor ordinary invocation behavior. Scalar recipes are
typed/hygienic and deterministic, with bounded input, expansion and dictionary
admission. Repeated expression holes cannot silently duplicate effectful or
trapping non-atomic work. Syntactic DAG reuse does not install common-subexpression
execution. Exact evaluation/first-trap order, fuel/cells/frames, signed arithmetic
checks, binary32 rounding and ownership/COW observations remain contract terms.

The global source-byte diagnostic takes precedence over the narrower WK1 projection bound. Neither bound may be weakened, and refusal must preserve the requested output path.

Independent witnesses compare fixed arithmetic/control-flow answers, precise
resource boundaries, overflow versus uninitialized-read order, signed zero/NaN,
FMA-distinguishing bits and host Arc identity. A separate hand-written compact
loop supplies collection/COW resource comparisons. These finite witnesses
support the stated slice; they are not universal proof or complete S01 closure.

The worker ABI is a bounded length-prefixed JSON process protocol. It copies
host values; it does not claim zero-copy ownership across processes. Source is
at most 256 KiB, a custom dictionary at most 64 KiB, frames at most 512 KiB,
diagnostics at most 64 KiB and calls at most 65,536 per worker. Host graphs are
bounded before JSON allocation or sequence/deadline changes. Malformed types,
outcome shapes, timestamps, duplicate fields, deep JSON, oversized messages,
unsolicited replies and stale identity refuse. Native pages remain owned by the
worker that prepared them and are reclaimed by closing/reaping that process.

Default worker bounds are 1 GiB virtual address space, 20 cumulative CPU seconds,
10 seconds for initialization, 2 seconds per call and 120 seconds per session.
The Linux x86-64 launcher verifies a sealed executable, applies resource limits
and denies process/thread creation with a fail-closed seccomp filter. The outer
coordinator handles aggregate session resources. These scopes are explicit:
process limits do not cover another process's allocation and are not filesystem/
network isolation, grading authority, hard host quota or real-time guarantees.

The strict/deterministic and explicitly approximate existing profiles remain
separate. Shared dense/sparse buffers and tensors, ownership/lifetimes, async
completion, cancellation, transfers, numerical/RNG policy, capabilities/effects,
versioned ABIs and provenance retain every existing acceptance obligation.
The ML lifecycle/custom training/import/export, pinned-registry all-call Vulkan/
SPIR-V/device execution, and packaged stateless/stateful DSP/music/real-time
contracts remain open where their complete gates are unmet. WK1 compresses source
for existing checked paths; it does not silently qualify new backend semantics.

## Completion-handle ownership refinement

Completion and cancellation use the exact live-session object returned by
`begin_compile`. Hashes, equal fields and serialized receipt dictionaries do not
substitute for that handle. Absent, copied and foreign-session handles refuse
before state/event mutation. Worker readiness independently retains its exact
source, dictionary, export and artifact checks. This local host rule introduces
no new artifact ABI, cross-process pointer or distributed authority guarantee.

## Stateful scalar streams and shader specialization

Stateful scalar streams bind immutable graph/library identities, finite initial state, state mapping, per-call resources and complete chunk policy. Every row reads one prior-row state snapshot; next-state slots update simultaneously. Whole-chunk failure leaves externally committed state and output unchanged. Attempt accounting, revision/sequence rejection and output ownership remain explicit. An in-memory handle is not a distributed exactly-once or durable checkpoint service.

VKS1 specialization is a separate approximate f32 profile. Exact finite
parameter bits, deterministic specialization IDs, SPIR-V template, original
source and full reflection distinguish a variant. Native maps are bounded and
owned through pipeline creation; cache equality includes effective values.
Existing VKF1 words, serialization, VM observations and default uncached behavior
stay available. Shader validation, actual device execution and independent CPU
comparisons are distinct evidence classes. Raw Vulkan calls and opaque driver
memory retain trusted-native limits.

Memory quotas cover their specified payloads, not all allocator, VM, driver or
process memory. Cancellation is cooperative at declared execution boundaries;
uncertain native submission cannot justify destruction. Stateful DSP remains
separately qualified from real-time callbacks. N01–N04, M01–M04, V01–V04 and
D01–D04 retain their complete requirements and open gates.

## Native tensor and extension boundaries

### Checked native additions

Native execution facilities add immutable dense f32 tensors, checked CSR, FTB1 storage serialization, exact-source scalar libraries and explicit CPU completion. Shape overflow, rank/layout/dtype, finite bits, signed zero, ownership and shared payload lifetime are checked before admission/publication. Quotas measure tensor/CSR payload, not all host/allocator/VM/driver memory; process RSS is a separate observation.

FTB1 and Tensor.numeric_profile describe finite storage/admission compatibility,
not producing arithmetic provenance. A Vulkan TensorDispatch carries explicit
source/shader/device/VKF1 operation metadata. No implicit broadcasting, hidden
capability, unbounded native extension, portable-float claim or native authority
is inferred from a hash. CPU cancellation is observed at bounded call boundaries;
uncertain GPU work retains owned resources and blocks new work until completion.
Driver nonpreemption and repeated abandoned-executor quarantine remain limits.

Native scalar packages are opt-in, versioned and checked for exact source,
signature and absence of ambient inputs. Adding an instrument uses unchanged
runtime bytes. This ABI does not assert compatibility with arbitrary native
CLAP/VST/DLPack/ONNX formats. Offline graph preparation/execution/replacement are
separate from future real-time callback obligations. Existing evaluator order,
first trap, exact numeric observations, COW and logical resource semantics remain
the independent portable reference. Incompatible meanings require a new profile
and migration with preserved original artifacts.

## Proposed common semantic core

S24 owns error-tolerant typed sketches and S25 the independent semantic-model/extension contract. I21 checks complete shipping-model coverage. Current `CheckedChange` and behavioral pins implement a bounded slice; they are not the full SourceId/NodeIndexId subtree protocol. Local queue, CAS and transcript-grader primitives do not constitute a distributed service, executable cache or OS sandbox. Bracketed Cxx source-reference labels in semantic examples are distinct from C01–C12 infrastructure task IDs.

The following common-core structures are proposed interfaces for implementation. S05 owns this contract and its version changes. Other agents may propose amendments but must not independently redefine identifiers, effects, hashes or resource semantics.

The first vertical slice is deliberately limited: monomorphic, first-order functions over unit, bool, i64, u64, and nominal flat records of those scalars; initialized locals; branches; and direct acyclic calls. Before S05 completes for its R06 consumer, extend the same contract with ordinary scalar CFG backedges, loop-carried block parameters and logical handles, verifier invariants and fuel-bounded reference event semantics. S08 separately owns new certified source iteration, including once-evaluated extents and continue-to-latch behavior; generic CFG loops do not depend on that syntax.

S23 is the explicit next portable-core extension. It owns nested records, nominal variants, typed lists, strict binary32, and recursive function calls, including their verifier, reference-evaluator, logical ownership, and cost rules. S05 remains accountable for approving changes to the common core contract. S06, S07, S08, collection libraries, and new compound types consume the applicable S23 contract instead of inventing their own backend-specific type systems. R10 implements selected-backend coverage over the approved S23 definitions; it does not own language semantics. Imports and new host commands have separate owners, I03 and S11/S19.

Unsupported released programs continue through their checked legacy evaluator. A refusal to lower is not permission to reject a previously supported artifact at the compatibility entry point. A program being accepted by the legacy language does not mean that it has entered the new core, strict profile, or native backend.

## Delivery scope and coverage

Keep a machine-readable capability matrix whose entries name the profile, exact feature or shape, preparation disposition, execution route, owner, test IDs, and evidence state. The following is the proposed division of work, not a report of implemented features.

| Feature | S05 portable core | S23 portable core | Strict or surface owner | Optimized route obligation |
| --- | --- | --- | --- | --- |
| Initialized scalars and flat scalar records | Required | Retained | S06 checks initialization | R06 adapts admitted existing shapes |
| Scalar branches and ordinary CFG loops | Required after the acyclic first increment | Retained | S06 checks joins and backedges | Unsupported shapes decline safely |
| Nested records and nominal variants | Legacy fallback | Required | S06 refinement and S07 match | Explicit coverage or portable fallback |
| Typed lists and their existing operations | Legacy fallback | Required | S08 adds new bounded iteration | R07 storage and R10 selected coverage |
| Strict binary32 and conversions | Legacy fallback | Required | S23 numeric contract | R10 proves route-specific agreement |
| Direct and mutual function recursion | Legacy fallback | Required with live-frame and fuel limits | S06 analysis handles call SCCs | Portable fallback remains valid |
| Legacy potentially uninitialized locals | Legacy fallback | Fallback until an explicit initialization-state extension is admitted | S06 rejects in strict mode | Never synthesize an undefined SSA value |
| Imports, generic specialization, and new text/map/array types | Outside S05 | Use approved core extension points | I03, I04, I06, I07, I08 | No implicit promise of native coverage |
| Host command intents | Pure inputs only | No ambient host authority | S11 and S19 define staging | R21 consumes the same checked protocol |

Each new type or operation must update five surfaces together: the type/operation registry, verifier rules, portable reference semantics, logical event/cost rules, and the feature matrix. Parser, printer, nominal encoding, source mapping, host admission, and artifact compatibility fixtures must also be updated wherever affected. A backend can decline a supported core operation, but the user-visible entry point must retain an available semantics-correct execution route.

S23 does not add closures, first-class function values, traits, arbitrary compile-time evaluation, or recursive value schemas. Function recursion and recursive value types are separate features; the current acyclic nominal-value rule remains until a separate reviewed extension changes it. The 25–40 engineering-day S23 estimate is provisional and must be re-estimated after S05's first verified loop slice; it is not a calendar commitment.

Experimental contract acceptance and product promotion are separate gates. S05 can complete with a reviewed versioned contract, executable fixtures, an independent verifier, and the declared portable subset. Downstream consumers may then integrate against that accepted experimental version. The end-to-end parse/edit/probe/activate demonstration gates the integrated slice; it cannot be a prerequisite for initially accepting the contract that its consumers require.

## 1 Ownership and artifacts

S05 owns `CoreModule`, its verifier, canonical encoding, and reference interpretation. S23 extends that same core for existing compound and recursive programs under S05 review. S02 owns source spans and diagnostics. S04 owns resolved symbols and nominal-schema identities. S16 owns behavioral dependency graphs. S17 owns patch admission and receipt semantics. R06 consumes the verified core and lowers it to the existing native `Plan` or its successor; it does not create a second language-level SSA contract.

S12 owns validated-snapshot construction, identity, and validity rules. R07 owns internal typed storage and legacy `Value` adapters. R08 consumes both contracts to implement and benchmark admission. An R08 optimization cannot weaken S12 validity or change legacy logical charges.

Keep `src/hir.rs` and FXH v1 as checked migration infrastructure. The existing `Hir` wraps structured modules or rule origins/projections (`hir.rs:23–37,123–184`). Name any new persistent core encoding separately; do not reinterpret existing magic bytes.

Canonical internal encoding is needed for identities and fixtures; it does not itself require a new public serialized-IR entry point. Keep the initial core in memory and experimental. Exposing a persistent or externally supplied core format requires an explicit S03/S05 format decision, bounded reader, compatibility policy, and executable-admission rules below. Existing checked source and FXH origins remain sufficient interchange for the first slice.

## 2 Core data model

The following Rust-like pseudocode fixes conceptual roles, not concrete allocation containers or public ABI layout:

```rust
struct TypeId(u32);
struct FunctionId(u32);
struct BlockId(u32);
struct ValueId(u32);
struct EffectId(u32);
struct EventId(u32);
struct SourceNodeId(u128); // editing identity; excluded from semantic hashes

struct CoreModule {
    profiles: ProfileTuple,
    types: Vec<TypeDef>,
    functions: Vec<Function>,
    interface: Interface,
    cost_plan: CostPlan,
}

enum TypeDef {
    Unit, Bool, I64, U64,
    Record { nominal_key: SymbolKey, fields: Vec<Field> },
}
struct Function {
    key: SymbolKey,
    params: Vec<TypeId>,
    result: TypeId,
    entry: BlockId,
    blocks: Vec<Block>,
    effects: EffectSummary,
}
struct Block {
    params: Vec<(ValueId, TypeId)>,
    effect_in: EffectId,
    instructions: Vec<Instruction>,
    terminator: Terminator,
}
enum Instruction {
    Pure { result: (ValueId, TypeId), op: PureOp },
    Step {
        before: EffectId,
        after: EffectId,
        event: EventId,
        results: Vec<(ValueId, TypeId)>,
        op: EffectOp,
    },
}
enum EffectOp {
    Charge { resource: Resource, amount: CostExpr },
    CheckedInt { kind: IntOp, left: ValueId, right: ValueId },
    AllocateRecord { ty: TypeId, fields: Vec<ValueId> },
    Alias { value: ValueId },
    DropHandle { value: ValueId },
    UpdateField { record: ValueId, field: u32, value: ValueId },
    Call { function: FunctionId, args: Vec<ValueId> },
}
enum Terminator {
    Return { value: ValueId, effect: EffectId },
    Jump { to: BlockId, args: Vec<ValueId>, effect: EffectId },
    Branch {
        condition: ValueId,
        yes: Edge,
        no: Edge,
        effect: EffectId,
    },
}
enum Resource { Fuel, Cells }
// Frames are a maximum live-depth limit, not a cumulative consumable.
enum CostExpr {
    Constant(u64),
    AdmittedOccurrenceCells(u32),
    RecordDetachCells(ValueId),
    Add(Box<CostExpr>, Box<CostExpr>),
}
struct SourceSidecar {
    source_id: SourceId,
    node_spans: Map<SourceNodeId, ByteSpan>,
    origins: Map<EventId, Vec<SourceNodeId>>,
}
```

`PureOp` initially permits constants, scalar comparisons, boolean operations, and statically valid flat-field projection. Checked arithmetic, allocation, calls, and logical ownership changes are sequenced steps. A pure operation may still have a separately sequenced logical fuel charge. Purity does not mean free execution.

ValueId and BlockId are scoped to a function; EventId is unique across the module. Record-valued SSA results denote logical handles. Projection borrows a live handle; Alias creates another; DropHandle consumes one; UpdateField consumes its input handle and yields the updated handle. Calls transfer explicit argument handles, and frame cleanup releases remaining handles according to the cost plan. Scalars can be copied freely. The verifier checks handle liveness and disallows use after consumption; backend reference counts are not this ownership state.

A consuming handle is transferred exactly once on the selected control-flow edge. Join parameters represent that transfer, including loop-carried handles; syntactically alternative edges do not create simultaneous aliases. Edge cleanup releases values that leave scope. Returning a borrowed compound projection requires an explicit owned handle in S23. The verifier must reject use-after-consumption, duplicated consuming arguments, mismatched live-handle joins, and missing frame cleanup, including on traps. The reference evaluator determines the normative logical lifetime; an optimizer's earlier physical free or retained cache reference cannot change it.

Each block has one current effect token. A `Step` consumes it and produces the next. The terminator forwards the current token to the selected successor; a join receives one incoming token through its block interface. Tokens cannot be stored in records, duplicated as ordinary data, or fabricated. Branch edges are mutually exclusive, so forwarding the same token on two syntactic alternatives is permitted.

Legacy lowering must reproduce its released schedule. For example, current `BinaryOperands` charges aggregated operand/operator fuel before reading either operand (`compile.rs:73–82,944–948`; `vm.rs:417–440`). It must not be expanded into a different failure schedule merely because the new IR has finer nodes. Legacy uninitialized-slot behavior requires a later explicit initialization-state representation or stays on the origin-backed evaluator; the adapter must not insert a supposedly initialized undefined SSA value.

## 3 Verifier invariants

Verification is a total, bounded operation returning structured diagnostics. A successful result creates an opaque `VerifiedCore` handle whose fields cannot be mutated through public APIs.

The verifier checks:

1. Every identifier resolves within its declared table; every function has one entry and every reachable block has one terminator. Reject unreachable blocks initially to simplify canonicalization.
2. SSA definitions are unique and dominate uses; block-edge arguments match parameter count and types. The branch condition is bool and returned values match function results.
3. Record construction/projection/update respects nominal identity and field order. Equal layouts do not make distinct nominal types interchangeable.
4. Effect tokens follow the single-current-token rule. Event IDs are unique within a module and resolve to the correct cost/profile domain. Calls cannot conceal effects absent from their transitive summaries.
5. Cost expressions use checked integer arithmetic and approved size variables. Unknown operations, missing event mappings, cyclic first-slice calls, excessive expansion, and unsupported profile features reject.

Verification after a transform checks structural invariants; it does not prove equivalence to the transform's input. Equivalence needs a separately stated obligation and evidence. Deserialized core is always verified. During initial development, verify after every transformation; any later production elision needs an explicit trust decision.

Structural `VerifiedCore` is not yet an executable authorization. S05 also owns profile admission, producing an opaque `ExecutableCore` only after the normative cost/event schedule is recomputed and validated for that profile. For legacy schedules that depend on released lowering, retain the checked origin, re-lower it through the trusted compatibility compiler, and compare the required canonical plan. Serialized optimized code needs an accepted checked certificate or trusted regeneration, not self-declared costs. Hashes bind inputs but do not validate a cost formula. Remove, reorder, or zero charge events in negative tests, including a zero-charge backedge; each forged plan must be refused before execution. This is a requirement for the proposed core reader, not a reported flaw in existing FSM/FXH readers.

S05 itself supplies finite reader/verifier limits and checked size arithmetic for its subset; these cannot wait for the later R13 cache and whole-pipeline work. R13 consolidates limits across preparation, backends, queues, and caches. Adding a public constructor must identify which layer has already validated its preconditions and must not create an unbounded bypass around an opaque verified handle.

### S23 extension obligations

Extend one feature at a time with an explicit contract version. Each increment has direct-source, decoded-artifact, core-reference, and legacy-reference comparisons on the shapes it admits. Cross-route agreement includes result bits, first trap, threshold-minus-one/threshold/threshold-plus-one fuel and cells, frame entry and cleanup, and promised host identity. Lowering coverage and execution coverage are separate report fields.

For compound values, retain nominal identity, declared field/tag order, occurrence-based admission costs, depth limits, and logical COW aliases. Define child-handle borrow/retain/drop rules and nested update behavior before exposing operations to optimization. Source-level empty/out-of-range/wrong-tag cases keep their profile-correct trap order. S07 optional access and new exhaustive matches cannot borrow undefined legacy fallback behavior as their specification.

For recursion, calculate type/effect summaries over deterministic call-graph SCCs with a bounded worklist. Check live-frame capacity at the specified call-entry point and debit frame/slot cells independently. A recursive function may run until its configured fuel/frame limit; no termination theorem is implied. Every executable dynamic control-flow cycle must follow the normative fuel schedule. Do not inline recursive SCCs indefinitely or use solver success as a substitute for the profile schedule.

For binary32, define exact accepted bit patterns, NaN canonicalization points, signed zero, subnormal handling, rounding, conversions, division, square root, and contraction policy from the selected profile. Pin the relevant host assumptions and preserve operation order. Backend acceptance requires bitwise and first-trap comparisons over edge cases and the registered differential corpus; enabling an f32 instruction or relying on a platform default is not sufficient.

S06 runs definite-assignment and refinement analysis on a resolved, typed source representation that still expresses local slots and source control flow, before successful strict lowering erases them into SSA. It must not conclude that an originally uninitialized local was initialized merely because the backend consumes SSA. Legacy programs that fail the new static rules retain their legacy execution path and dynamic traps.

## 4 Identities and canonical hashes

Use SHA-256, matching the existing repository dependency, with domain-separated length-delimited fields. Never hash Rust debug strings, native struct memory, map iteration accidents, addresses, or host-dependent integer layouts.

Maintain these identities:

| Identity | Included material | Excluded material |
| --- | --- | --- |
| SourceId | Syntax profile and exact UTF-8 source bytes | Runtime state and permissions |
| SemanticId | Semantic profile, typed operations/control, nominal schemas, declared effects and interface | Spans, comments, private local spellings, cost amounts |
| CostId | SemanticId, cost profile, ordered charge placement/formulas and logical ownership schedule | Machine instruction counts and allocator addresses |
| AbiId | Host value/layout contract, calling/serialization conventions, identity guarantees | Actual host grants |
| ProgramId | SemanticId, CostId, AbiId and required feature identities | Source formatting and invocation-specific limits |

A syntax profile controls accepted spelling. A semantic profile controls value and trap behavior. A cost profile controls logical charging. The ABI has a separate identity because value equality does not exhaust the host contract. Artifacts declare required capabilities; only trusted host policy grants authority. An artifact cannot select a permissive host policy by declaring its identifier.

Canonical encoding uses little-endian fixed-width integers, two's-complement signed values, bool bytes 0/1, u32 lengths/indices, and exact UTF-8 string bytes without implicit Unicode normalization. Type/operation discriminants belong to one S05-owned versioned registry; agents consume shared encoding fixtures instead of assigning tags independently. Sort named functions/types by resolved symbol key; preserve record field and variant order. Number blocks by a deterministic traversal with ordered successors, and renumber values/events by first definition. Private local alpha-renaming is erased. Do not promise arbitrary graph-isomorphism or mathematical-equivalence hashing. SourceNodeId is editor lineage metadata, not semantic identity.

Behavioral pins incorporate transitive callees, schemas, profiles, and imports through deterministic strongly connected components. Interface-only pins remain distinct. The legacy `Snapshot::revision` hashes declarations (`collaboration.rs:165–168`); it must not be silently redefined. Legacy formats derive profiles from their released version and entry point rather than being rewritten to embed new fields.

## 5 Logical ownership and observable identity

Current COW charges use actual `Arc::strong_count` (`vm.rs:375–408`). A new representation cannot substitute its physical refcounts and assume that budgets remain compatible. Removing a dead alias, retaining an extra cache reference, or flattening a record can change whether a physical copy occurs.

The compatibility model therefore tracks semantic object identity and logical handles independently of backend storage. Boundary values retain a host-owned origin identity. A legacy load creates the logical alias required by the old evaluator; stores, calls, returns, and frame exits move or release handles according to the legacy schedule. COW charging depends on that modeled ownership state. The adapter must establish agreement with released behavior for its admitted shapes; otherwise it declines to the legacy evaluator.

Preserve documented host-visible identity guarantees separately from allocator statistics. Existing tests check `Arc::ptr_eq` for unchanged returned records (`tests/native_structured.rs:90,197`; `native_ir.rs:303`). An optimized result that still denotes the original object must reconstruct the original `Arc` handle where that guarantee applies. A changed result must detach as specified. New ABIs may relax identity promises only explicitly; ordinary internal allocation addresses are not generally language semantics.

S12's `ValidatedSnapshot` is opaque and bound to an owned immutable graph, nominal schema fingerprint, profile/ABI identity, depth, occurrence-based logical cell count, and representation validity. Repeated references are counted repeatedly where the legacy visitor does so. Physical validation may be cached, but logical charges cannot disappear. Admission itself has no dependence on pointer sharing: its cached count preserves logical occurrences; later COW behavior additionally consults the separately modeled handle state. Mutation creates a new graph or invalidates the handle. Raw pointer addresses are insufficient cache keys. Overflow while constructing certificate metadata is a refusal with a compatible uncached path, not a wrapped smaller charge.

## 6 Observations, admission, and preparation limits

A profile defines observable results, first trap class, committed state/effect intents, applicable host-identity guarantees, and outcomes under each permitted budget. Diagnostic event traces describe canonical logical events, not native instruction dispatch. A superinstruction may implement several logical events; trace mode reconstructs their canonical sequence or uses a compatible interpreter.

Exact-profile native admission must establish either preserved ordered checks or sufficient conditions that exclude every resource failure inside the admitted region while preserving remaining observations. A conservative bound that is insufficient for admission causes fallback, not premature rejection of a call the VM could complete. A new region-precharge policy requires a distinct cost profile. Resource exhaustion is not generally equivalent to an earlier arithmetic or initialization trap.

Frame capacity is checked at the profile-defined call-entry point; entry increments live depth and exit decrements it. Frame/slot cell charges remain cumulative and are emitted separately. A function call expands its checked callee event plan; it is not an opaque permission to reorder frame checks or omit nested charges. Only the external result boundary performs the result-admission charges required by the legacy profile.

Whole-invocation replay after a native decline is allowed only for pure evaluation over immutable snapshots with no committed external effects. Local allocations are discarded. Bound speculative work and allow at most one native attempt before portable replay; report the duplicated physical work in performance evidence rather than hiding it inside logical fuel. Input state, output state, logical budgets, traps, and identity behavior must agree with a fresh portable execution. Once host interactions exist, use staged intents or an explicitly specified resumable state; never replay an externally executed effect casually.

`PrepareLimits` is distinct from `InvocationLimits`. It bounds bytes per reader, expanded syntax nodes, declarations, type depth, IR instructions/blocks, diagnostic output, generated code, queued preparation work, cache bytes, and optional analyzer/solver work. Preserve current reader distinctions: compact input is bounded at 256 KiB while canonical FSM source permits 1 MiB (`lib.rs:22`; `compile.rs:10–16`). New numeric defaults require calibration; they are not established by this proposal.

Charge/check preparation limits before expansion or allocation where possible. Use checked size arithmetic and fallible reservations. A solver timeout is `unknown`, not a proof. A trusted SMT `unsat` response places that solver in the trust base; call it independently checked only when a separately implemented checker validates a supported certificate. Physical execution time and peak bytes require their own evidence; logical fuel/cells do not automatically prove those bounds.

The runtime boundary distinguishes admission failure from guest failure:

```rust
enum NativeAttempt {
    Completed(Value),
    Trapped(Trap),
    Declined(DeclineReason),
}
enum PrepareDisposition {
    Core(ExecutableCore),
    Legacy(CheckedLegacyProgram),
}
```

`Declined` selects the portable path; it is not a guest trap. `Trapped` is terminal only when the backend establishes the profile-correct first trap. Unsupported lowering of a recognized valid legacy artifact returns a checked legacy disposition. Malformed artifacts return preparation errors. Neither a model nor an artifact can construct the opaque verified handles. Diagnostic resource statistics are explicitly labeled logical or physical; optional instrumentation must not change admission or successful results.

## 7 Agent-facing example objects

The following are valid JSON examples with explicit placeholder identities. They are not real receipts, measured checks, released wire formats, or executable authorization. Production parsers reject placeholder hashes. S02/S16/S17 own the corresponding schemas and version negotiation.

Diagnostic:

```json
{
  "protocol": "fluxr-diagnostic/0",
  "code": "E_TYPE_MISMATCH",
  "severity": "error",
  "source_id": "EXAMPLE_SOURCE_ID",
  "node_id": "example-node-17",
  "span": {"start_byte": 0, "end_byte": 1},
  "expected": "i64",
  "actual": "bool",
  "message": "Expected i64 in the selected expression",
  "related": []
}
```

HoleContext:

```json
{
  "protocol": "fluxr-hole/0",
  "base_program": "EXAMPLE_PROGRAM_ID",
  "source_id": "EXAMPLE_SOURCE_ID",
  "node_index_id": "EXAMPLE_NODE_INDEX_ID",
  "target": "example-policy",
  "node_id": "example-node-17",
  "expected_type": "i64",
  "bindings": [{"id": "example-value-1", "name": "amount", "type": "i64"}],
  "allowed_effects": ["trap.integer_overflow"],
  "behavioral_pins": {"example-helper": "EXAMPLE_BEHAVIOR_ID"},
  "host_grant_reference": "EXAMPLE_HOST_CONTROLLED_HANDLE",
  "resource_obligations": {"profile": "EXAMPLE_COST_PROFILE", "status": "unresolved"}
}
```

PatchEnvelope:

```json
{
  "protocol": "fluxr-patch/0",
  "base_program": "EXAMPLE_PROGRAM_ID",
  "base_source": "EXAMPLE_SOURCE_ID",
  "node_index_id": "EXAMPLE_NODE_INDEX_ID",
  "target": "example-policy",
  "expected_body": "EXAMPLE_BODY_ID",
  "interface_pins": {"example-policy": "EXAMPLE_INTERFACE_ID"},
  "behavioral_pins": {"example-helper": "EXAMPLE_BEHAVIOR_ID"},
  "edits": [{"kind": "replace_expression", "node_id": "example-node-17", "text": "(+ amount 1)"}],
  "claimed_properties": [],
  "proof_artifacts": []
}
```

Draft Receipt, intentionally containing no completed evidence:

```json
{
  "protocol": "fluxr-receipt/0",
  "status": "draft",
  "base_program": "EXAMPLE_PROGRAM_ID",
  "base_source": "EXAMPLE_SOURCE_ID",
  "candidate_program": "EXAMPLE_CANDIDATE_ID",
  "candidate_source": "EXAMPLE_CANDIDATE_SOURCE_ID",
  "patch_id": "EXAMPLE_PATCH_ID",
  "compiler_id": "EXAMPLE_COMPILER_ID",
  "checker_id": "EXAMPLE_CHECKER_ID",
  "checks": [{"kind": "typecheck", "status": "not_run"}],
  "dependency_set_id": "EXAMPLE_DEPENDENCY_SET_ID",
  "evidence": [],
  "activation_eligible": false
}
```

An accepted receipt records all required checks and the actual evidence identities. A source edit normally intends changed behavior: successful type checking does not imply equivalence to the previous body. Optimization-equivalence claims and application-invariant claims are separate check kinds. The host authenticates proposers and authorizes targets; author strings and model-supplied claims do not grant permission.

### Source identity and patch admission

`ProgramId` deliberately excludes formatting, source spans, and editing lineage. It therefore cannot by itself identify the syntax tree addressed by an edit. Bind each patch and hole response to `SourceId` and the exact revision-scoped `NodeIndexId`, in addition to the semantic/body/dependency identities. A trusted remapping operation may carry nodes across formatting-only edits after proving the A06 correspondence; ordinary patch application does not guess a remapping. An expired session, different source snapshot, repeated-subtree ambiguity, or mismatched index must produce a bounded stale-target response before mutation.

Limit request bytes, edit count, tree depth, replacement expansion, dependency pins, proof bytes, and returned diagnostics. Validate all patch preconditions against one captured base snapshot, then typecheck and profile-admit the complete candidate. A model-supplied expected type, effect ceiling, cost bound, or successful-check flag is evidence to verify, not a substitute for these checks. A stored receipt is an audit record, not a bearer capability to activate code. Receipt replay rechecks the relevant identities, host authorization, current policy, and required evidence; reusable trusted validation handles remain opaque and scoped.

A07 defines deterministic **same-base batch composition**. Nonoverlapping syntax nodes are only an initial conflict filter. Reject overlapping writes, conflicting read/write dependencies, ambiguous remapping, or a combined candidate that fails required checks; preserve the atomicity of each submitted change. Order independence means the same batch against the same base produces the same accepted/rejected partition and resulting candidate. It does not promise that every pair of disjoint edits is semantically independent, or that sequential application after a changed revision cannot stale-reject. Commutativity is required only for cases whose dependency conditions establish independence. General invariant-preserving semantic merge remains S18 research.

For example, replacing an assignment from variant A to variant B and independently replacing its `is A` guard with `true` touches different expressions. Each edit can be safe in isolation while their combination attempts an A payload on B. Full-candidate checks and application invariants must decide that combination; node nonoverlap cannot certify it.

S17 completes a versioned **experimental** protocol with deterministic offline checks, fixtures, and development-set feedback. It does not wait for A19's confirmatory experiment, which consumes that protocol. S07 syntax screening likewise uses development tasks. A19 freezes candidate protocol/syntax versions before opening the confirmatory holdout; A22 owns any subsequent stable authoring-profile promotion and compatibility commitment. Negative or inconclusive cost results may leave the protocol experimental while useful diagnostics and explicit full-body edits remain available.

### First-order collection callbacks

S08 and I10 use statically named callbacks with explicit capture arguments. At each call site, the compiler resolves the callback symbol and concrete type arguments, specializes the bounded combinator, and emits ordinary direct calls or verified inlining. Captured values are ordinary explicitly typed arguments with the usual alias, evaluation, and resource rules. The callback identity participates in behavioral pins and specialization keys.

The first implementation accepts no runtime function values, escaping environments, arbitrary callback dispatch, or implicit mutable captures. Evaluate the collection/extent and capture arguments exactly once in specified source order. Define whether a callback may trap, allocate, or request effects; pure collection helpers initially reject host-command callbacks. Check callback signatures and summaries before specialization, and bound specialization count and code growth through I04/R13. Preserve element order and early-exit behavior. This supports reusable map/filter/fold/find without making mandatory library work depend on the optional closure experiment I05.

### Diagnostic and editor positions

Compiler spans remain half-open UTF-8 byte ranges attached to an immutable `SourceId`; an empty span is a documented insertion point. Preserve original-source ranges through desugaring and report explicit truncation when limits apply. I15 must convert these ranges to the editor's negotiated line/character encoding and document version, rather than copying byte offsets into LSP character fields. Test multibyte text, supplementary-plane characters, CRLF, empty final lines, unsaved buffers, cancellation, and stale diagnostics. CLI and editor results agree on the selected source text and diagnostic identity after conversion; their numeric offsets need not use the same units.

## 8 Live execution and commit protocol

Separate immutable `ProgramId` from a monotonically checked activation generation, and separate code identity from host state epoch. Avoid ABA errors when identical code is later reactivated.

```rust
struct CommitVersion {
    generation: u64,
    state_epoch: u64,
    policy_epoch: u64,
}
struct InvocationTicket {
    version: CommitVersion,
    program: ProgramId,
    input_snapshot: SnapshotId,
}
struct PendingResult {
    ticket: InvocationTicket,
    next_state: ValidatedValue,
    intents: EffectJournal,
}
```

The host acquires code, state snapshot, and policy from one coherent transaction boundary. Invocation produces a `PendingResult` without committed effects. Commit atomically compares the full version, validates the result/schema/intents, advances the state epoch, and publishes state according to host policy. Activation atomically publishes the new code generation and, when necessary, migrated state; migration failure leaves both old code and state active. Epoch overflow refuses publication.

Old code may remain allocated until calls finish, but their stale results cannot overwrite new state. Reject, retry, or explicitly migrate those results according to a documented policy. For external services, a durable outbox and idempotency may be appropriate; this contract does not promise general exactly-once effects or rollback after external delivery.

The CAS establishes at most one acceptance for the same captured invocation ticket and version. Retrying an operation with a newly acquired epoch is a distinct attempt unless the host supplies a separately specified durable deduplication policy. A stale, unaccepted journal must not enter the outbox. An intent already committed before activation may finish external delivery later according to host policy; that is not a new stale commit.

## 9 Proof obligations and counterexamples

Before enabling an optimization, state the exact property and trust assumptions. Useful initial obligations include typed lowering preservation; event-order preservation; no hidden capability use; a checked upper cost bound; preservation of documented identity; and stale-result exclusion. Freeze statements outside the proposing agent's write scope.

Required counterexamples include:

- Checked reassociation: `(MAX + 1) + (-1)` traps, whereas `MAX + (1 + (-1))` succeeds. Mathematical equality is insufficient.
- Fuel granularity: the reproduced legacy fused expression with an uninitialized operand exhausts fuel at budgets 0–2 and reaches the initialization trap at 3. A different schedule needs a new contract.
- Dead alias removal: a retained unused record alias can force a COW charge. Removing its physical reference cannot remove the legacy logical charge.
- Snapshot deduplication: two references to one allocation still incur two occurrence-based visits where specified.
- Payload refinement followed by assignment: the earlier tag fact cannot authorize access on the replacement value.
- Stale commit: an old invocation completing after code/state activation cannot publish its old result or intents.

For bounded iteration extensions, evaluate extent once, protect the lexical index, and route `continue` through the latch. Existing `scan` is while-loop sugar (`compact.rs:350–355,555–570`) and is not itself a termination certificate. For binary32 extensions, specify subnormals, signed zero, NaNs, conversion, rounding, and contraction before adding native shortcuts.

## 10 Research grounding and implementation gate

The design adapts established work rather than claiming a new theorem. Tang and Lindley's [modal-effect framework](https://homepages.inf.ed.ac.uk/slindley/papers/met-encodings.pdf) motivates separating effect representation from surface notation. Chu, Guo, and Hoffmann's [resource analysis](https://www.cs.cmu.edu/~janh/assets/pdf/ChuGH26.pdf) motivates explicit treatment of non-local control in bounds. The [progressful interpreter work](https://doi.org/10.1145/3704858) motivates a tractable executable semantic model. The [2026 compiler-proof experience report](https://arxiv.org/html/2602.20082v1) motivates independent ownership of theorem statements and proof assumptions.

Accept the initial S05 experimental contract after its verifier, portable evaluator, profile admission, and scoped fixtures pass independent review. After the dependent authoring and host-coordination packages are integrated, gate the complete vertical slice on one scalar/record policy that can parse, resolve, verify, interpret, expose a hole, accept a checked edit, and activate with stale-result protection. The integrated demonstration is downstream evidence, not a circular prerequisite for accepting S05.

Preserve legacy fixtures and publish all refusals. Expand types and control flow through S23 and subsequent reviewed contract versions rather than letting backend or authoring agents invent incompatible extensions. Promote a new public profile or artifact only after its declared feature matrix, compatibility rules, and relevant evidence are complete; an unchanged legacy fallback remains a supported outcome wherever the matrix says so.

## Agent source projections and unchanged semantics

Native authoring targets LLM agents. Human readability and editor conventions
are optional projections, not admission gates. This changes the authoring
objective without changing any existing observable behavior or released reader.
FA1 is an optional bounded prefix projection into the current structured AST;
its checked printer reparses and compares exact AST/FSM bytes. A valid existing
module may exceed FA1-specific projection limits and refuse that route while
remaining valid through its existing entry point.

Large semantic vocabularies lower through one reviewed core. Typed abstraction
expansions must preserve evaluation/trap order, capabilities, aliasing and exact
resource observations under the chosen profile. Dictionary/tokenizer/model
identity is distinct from semantic operation identity. A new cost model or wire
alphabet requires an explicit version and compatible old readers. Short strings
and arbitrary bit-code words do not prove model competence.

Action identity captures one bounded canonical payload before validation and hashes that same payload. It does not supply authorization, runner isolation, or an atomic snapshot of every possible concurrently mutating caller object. The structural contract composer checks exact content pins and a whole proposed catalog; invariant prose is not a theorem or executable predicate.

## Graph streaming and native reuse

fluxr-scalar-graph-v1 checks graph topology, exported scalar arity, finite constants,
source/semantic library identities and bounded canonical metadata. Every row and
node executes in declared order through the existing FluxR evaluator, including
unused nodes. Output publication is chunk-atomic; failing rows/calls remain in
receipts. Shared tensor reservations account output and scratch payload; retained
outputs can backpressure later chunks. Instruction/cell/frame limits remain
per-call and graph/chunk limits are separately explicit. The worker/offline route
allocates and hashes receipts, so it is not a real-time callback API.

An unchanged runtime can load an external FluxR mix/softclip effect package and execute it in the scalar graph. Model examples include preprocessing, imported analytic checkpoint parameters, neural prediction and clustering. These examples do not qualify a generic ONNX importer, physical GPU training or production native-plugin sandbox. Full ML training and audio latency requirements remain open.

The optional Vulkan live pipeline cache is private to one logical device. Hits
require exact canonical FSM bytes, shader words and full reflection identity;
key/entry limits and deterministic LRU eviction are explicit. Submitted jobs
retain native owners until fence completion. Default construction remains
uncached, and VKF1 bytes/approximate arithmetic meaning stay unchanged. Retained
key-byte accounting excludes opaque driver memory; quarantine remains bounded
per executor rather than globally. See source docs/native-stream-v1.md and
docs/vulkan-pipeline-reuse-v1.md for API, failure, migration and rollback details.

## Stateful checkpoints and development resources

The FSC1 contract in `docs/native-checkpoint-v1.md` adds bounded data-only stateful CPU worker/offline export/import. It retains exact finite state bits, graph/library/source/numerical/stream pins, complete attempt/counter history and remaining budgets. Expected external digest and byte cap are mandatory; malformed/stale/impossible histories refuse before construction. Failed preparation preserves old stream state; output lifetime charges remain. Fresh resume is a local fork, not authentication, anti-rollback or distributed exactly-once. It does not qualify general ML checkpoints or audio callbacks.

[resource-safety-v1.md](resource-safety-v1.md) governs trusted development admission, watchdogs and cleanup separately from runtime tensor quotas and hostile-candidate isolation. Aggregate resource headroom is measured before expensive work; scheduling limits do not reduce semantic or product acceptance obligations.

## Linux typed execution and prepared entry

The optional `typed-jit` route now admits Linux **GNU x86_64** for the existing
legacy native Plan's checked signed-i64, bool and supported flat-record shapes.
Its preparation caps remain 512 expanded operations, inline depth 8, 32 lanes
and four i64 fields per admitted record. Generated code plus System V unwind
metadata fit one 16 KiB owned mapping. This does not admit new language types,
redefine the S05/S23 semantic core, change released encodings, or complete
u64/strict-f32/general-collection/native-target coverage. The retained f32 scan
kernel has separate `PrecompiledScan` attribution. Unsupported shapes retain
the compatible portable path; Windows and other platforms require their own
execution evidence. Optional backend features remain optional.

Linux publication rejects an inaccessible personality query or
`READ_IMPLIES_EXEC`, constructs a private writable mapping, and completes an
RW-to-RX transition before an entry pointer escapes. Internally emitted,
address-bound System V CFI is registered through libgcc_s; its immutable bytes
share the code owner's allocation, are deregistered before unmapping, and
remain alive through borrowed calls. All error paths retain owned-memory
cleanup. These are audited trusted-host ownership and platform assumptions,
not a native-code sandbox, hard allocation quota, arbitrary unwinder guarantee,
or proof of third-party compiler termination. Synchronous compilation remains
outside real-time callback and publication critical sections. R11/R12/R13 and
arbitrary-candidate isolation gates remain independently open.

`Module::prepare_call` returns a `PreparedCall` borrowing that exact immutable
module, its export, code and prepared backend owners. No caller-supplied module,
export index or executable address can rebind it. Retaining an earlier handle
retains its original revision behavior; Rust lifetimes prohibit use after the
module owner dies. Warm calls retain arity/type, admission, checked arithmetic,
first-error, logical fuel/cell/frame and documented identity/COW obligations.
An unsuccessful speculative typed invocation replays the unchanged pure
portable evaluation once. Physical duplicated work remains a measured cost,
not erased logical work. Actual `ExecutionRoute` values distinguish typed
machine-code success, retained Windows scalar success, precompiled scan and
portable handling; preparation or compilation alone is never a native hit.

The bounded must-initialization analysis operates on the already admitted
native Plan. It intersects reaching-path facts, includes the function-entry
edge and loop backedges, and removes a dynamic local guard only where every
reaching path establishes initialization. Uncertain loads/updates retain the
legacy guard and replay semantics. Zero-trip loops, branch-only stores,
first-iteration loads before later stores, and record updates are required
negative witnesses. No undefined local value or new strict-source acceptance
rule is introduced. S15's proof/certificate and independent semantic coverage
requirements remain broader than this tested optimization; neither a hash nor
agreement with the shared production VM proves the entire compiler correct.

## Native interoperability

Explicit Rust interoperability retains ordinary Rust types, ownership, lifetimes, arithmetic, traits, unsafe code, FFI, panic strategy and optimization. The checked core does not extend VM fuel or sandbox guarantees to trusted Rust. Native build scripts and macros execute trusted code. Native hot paths may run directly while the application retains its runtime; injected checked code reaches only typed services registered by the application. See [native-rust-v1.md](native-rust-v1.md).
