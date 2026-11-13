# AH1 typed body drafts

AH1 adds immutable, revision-bound **whole-function-body holes** for autonomous
agents editing an already checked FA1 program. A host can retrieve a small typed
context, ask for one `fa1 body ...` replacement, validate it against the existing
signatures, and admit the complete candidate after every selected body is filled.
It reuses the existing parser and type checker. FA1 source, compact source, FSM
artifacts, collaboration targets, runtime behavior and publication APIs retain
their released meanings.

An `AuthoringDraft` has no execution method and exposes no `Module`. Opening a
draft erases every selected body's AST; an unresolved body has no executable
placeholder. The exact original FA1 text remains separately available as source
provenance. `admit` refuses while any hole remains, then runs the normal full
module checker, including the aggregate instruction limit. Passing an individual
fill is distinct from final executable admission.

```rust
use fluxr::structured::authoring::{
    AuthoringDraft, BodyTarget, Cancellation, QueryLimits,
};
# fn example(compiler_binary_sha256: [u8; 32]) -> Result<(), Box<dyn std::error::Error>> {
let cancellation = Cancellation::default();
let target = BodyTarget::function("calc");
let source = "fa1 v2 pub calc 2 x int y int int ret add x y";
let draft = AuthoringDraft::open_fa1(
    source, 17, compiler_binary_sha256, &[target.clone()], &cancellation,
)?;
let context = draft.query(
    draft.binding(), &target, QueryLimits::default(), &cancellation,
)?;
// Give context.packet() to the agent; retain the actual context ticket in the host.
let next = draft.fill_body(
    &context, "fa1 body 1 ret rem add mul x y 17 997", &cancellation,
)?;
let executable = next.admit(next.binding(), &cancellation)?;
# let _ = executable;
# Ok(()) }
```

## Binding and contextual identity

Every successful context response binds the AH1 profile and structured compiler contract, the
host-supplied compiler digest, exact original source bytes, caller's buffer
version, original checked canonical semantics, original declaration index,
current partial sketch and its generation/unresolved-hole set. Raw-source and
semantic identities are separate: changing only whitespace or CRLF still changes
the source/index binding. Filling a body increments the immutable draft generation
and invalidates every previous context ticket, including tickets for another hole.
The caller must requery the new draft. Separate immutable forks remain usable with
their own exact tickets.

The buffer/source IDs always describe the immutable original text. Fills are
structural AST transactions: whitespace-only differences between equivalent FA1
fill bodies canonicalize to the same sketch and draft identity. AH1 does not
retain a raw fill-text history or provide edit source-span fidelity. The host's
episode/tool ledger must retain every exact request, failure and token cost.
This is distinct from full S24 lossless buffer recovery and C04 source-span gates.

The compiler digest must be nonzero. The trusted host should supply the exact
compiler binary/build identity and retain its provenance. AH1 compares this field
but does not authenticate it. A digest does not grant publication authority or
prove that an execution or review occurred. The runnable example computes the
digest of its actual executing binary for its own binding.

Targets use an exact `(receiver, name)` pair. `BodyTarget::function("x")` selects
an ordinary function, and `BodyTarget::method("Point", "x")` selects that method.
`resolve_name("x")` is optional shorthand and refuses multiple matching names.
The packet spells `function x` and `method Point x` distinctly; a record literally
named `none` cannot collide with an absent receiver. Declaration indices are local
to their bound source and never treated as globally stable IDs.

## Model-facing context and uncertainty

The returned `BodyContext` is a host-retained ticket with private fields and no
deserializer. Its packet is plain ASCII prefix text beginning `ah1 analysis_only`.
It reports the full binding, explicit target, expected return type, explicit
`effects unproven`, `resources unproven`, `capabilities unproven` obligations,
and the requirement for full module checking
plus runtime invocation limits. It contains parameter/local/input bindings,
nominal schema signatures and callable signatures in deterministic declaration
order. Parameters and locals come first. It never retrieves other declarations'
bodies. This is bounded signature retrieval, not a minimal-dependency algorithm
or a proof that a helper is semantically appropriate.

`context signature_complete` means the available signature inventory fit in the
requested budget. It does not claim complete semantic analysis, static cost
proof, termination, effects certification, or executable admission.
`context truncated` and visible/total item counts explicitly report omission.
Neither an empty diagnostic set nor a completed context query can authorize
execution. A host may request another bounded context or use a larger allowed
budget; it must keep omitted information classified as unknown.

The full source, edit, complete query packet, host/profile setup, retrieval and
repairs all count in agent-token evaluation. Six hashes and exact provenance
have a real token cost. This implementation does not claim that such a packet
meets the ≤10% equivalent-Rust source-token target or improves model competence.

## Bounds, cancellation and ownership

Input source and replacement bodies are limited to 262,144 bytes before parsing.
The checked base and FA1 parser retain their existing declaration, node, depth,
type, binding and canonical-byte limits. A draft selects 1–512 distinct existing
bodies. Duplicate, missing or malformed target identities refuse.

Queries accept `max_bytes` from 1–65,536 and `max_items` from 0–4,096; defaults are
16,384 bytes and 128 items. The returned packet always fits the byte limit; a
limit too small for its mandatory binding/obligation header refuses with
`OutputLimit`. Zero items returns an explicitly truncated packet if the header
fits. Header and per-item formatting check every fragment before allocation and
append. A signature uses at most the packet's remaining text allowance; an
oversized item is discarded as a whole. After truncation or an item cap, later
signature formatters do not run. The bounded signature inventory is still counted
with cancellation checks so `total_items` remains exact. The packet and one
pending item are separately owned buffers: each requests at most `max_bytes`,
and their combined requested text capacity is at most twice that cap during an
append, before allocator/reallocation overhead and fixed metadata. This is not a
measured heap/RSS ceiling. A refused fallible text allocation returns
`ResourceLimit`; it is not labeled ordinary requested context truncation.
No global query cache is retained.

Error classes distinguish invalid requests, parsing, typing, stale binding,
unknown or ambiguous targets, unresolved or already-filled holes, cancellation,
output limits and resource limits. Detail text is at most 240 UTF-8 bytes. Failed
opening, querying, filling or admission does not modify an existing draft.
Cloning a draft shares its original bytes and AST; a successful fill creates a
new checked body sketch. Retention of multiple generations is the host's
responsibility and must be included in its resource budget.

Cancellation tokens are terminal and request-local. Clones share cancellation;
a fresh request uses a fresh token. Operations check cancellation before work,
between bounded phases/signature items and before returning a successful result.
The existing parser, checker and sketch-rebinding calls are synchronous phases;
cancellation does not interrupt an individual phase mid-stack. Their input/work bounds still apply,
but no hard cancellation-latency guarantee is claimed. There is no global lock
through which a stalled old request can block a newer independent draft.

## Execution and evaluation

```sh
cargo test --locked --test agent_authoring
cargo run --locked --release --example agent_authoring_demo
python3 tools/bench_tokens.py --manifest benchmarks/agent-authoring-v1.toml
```

Run these through the shared resource admission/watchdog described in
`resource-safety-v1.md`; fresh build routes use separate sequential targets.
The example executes a genuine filled FluxR function, compares the exact FSM
bytes with a directly compiled program, and compares its result with checked Rust
arithmetic. It emits all source/edit/context strings and 31 timing rounds for
open, query, fill, final admission, direct full compilation, runtime invocation,
equivalent Rust and the complete reviewed authoring episode. Small, medium and
large signature inventories report context truncation explicitly. Byte sizes are
payload sizes; they are not peak heap/RSS measurements. Runtime and preparation
costs remain separate, and repeated round samples are retained.

The separate `benchmarks/agent-authoring-v1.toml` catalog adds token measurement
without changing the frozen executable-replay catalog. It does not authorize
arbitrary model-generated compilation or execution. The external independent
oracle remains distinct from author tests. Differential tests check exact
artifacts, trap/budget outcomes and shared-list copy-on-write; final whole-module
budget refusal is tested even after individually valid fills.

## Qualification boundary and rollback

This slice contributes to S24's typed-hole/revision/cancellation/uncertainty gates
and C04's immutable-query separation. Those complete tasks remain open. AH1
requires a checked base and does not recover unrelated malformed declarations,
preserve arbitrary invalid UTF-8, infer expression holes, supply source-span
sidecars, provide constrained model decoding, or implement a production
incremental compiler/cache. Per-body filling uses the existing `BodyChecker`;
final admission remains a complete compile. General native effects/capabilities,
semantic rebase, publication authority and hostile compilation/execution
isolation require their separate contracts.

To roll back, stop authoring producers, retain all draft/query/fill outcomes and
switch callers back to the existing checked-source/collaboration APIs. No reader,
artifact, database or runtime migration is needed. A completed AH1 program is
ordinary checked structured FluxR, and its execution rules are unchanged.

AH1 refusal diagnostics contain a bounded error kind and detail. The host must
retain the failed request and its exact binding; the error value itself has no
revision sidecar. Full S24 revision-bound diagnostic/source-span acceptance
remains open.
