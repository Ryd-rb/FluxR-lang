# AC1 compact authoring context

AC1 is the opt-in `fluxr-ac1-v1` context projection for autonomous agents using
[AH1 typed body drafts](agent-authoring-v1.md). It removes five repeated component
digests and repeated binding labels from the model-facing packet, retains one
full draft digest, and offers explicit signature selection. The actual
host-retained `BodyContext` keeps the complete AH1 binding. Query projection
does not change FA1 source, the existing AH1 packet, checked body admission, FSM
artifacts, runtime behavior or publication authority.

The optimization concerns retrieved context tokens. Source tokens still have
the unchanged **at most 10% of equivalent Rust** target. Packet savings cannot
substitute for that source target, model competence, accepted task throughput or
complete coding-episode measurement. The new fixtures are authored development
cases; they are not held-out tasks or live-agent trials.

## Opt-in API and exact selectors

```rust
use fluxr::structured::authoring::{
    AuthoringDraft, BodyTarget, Cancellation, QueryLimits, SignatureSelection,
};
# fn example(compiler_sha256: [u8; 32]) -> Result<(), Box<dyn std::error::Error>> {
let cancel = Cancellation::default();
let target = BodyTarget::function("calc");
let source = "fa1 v2 fn step 1 x int int ret add x 3 pub calc 1 x int int ret x";
let draft = AuthoringDraft::open_fa1(
    source, 1, compiler_sha256, &[target.clone()], &cancel,
)?;
let callables = [BodyTarget::function("step")];
let context = draft.query_compact(
    draft.binding(),
    &target,
    SignatureSelection::Explicit {
        records: &[], enums: &[], callables: &callables,
    },
    QueryLimits::default(),
    &cancel,
)?;
// Retain this exact ticket and its request/response association in the host.
// Give context.packet() to the model under the negotiated AC1 profile.
let next = draft.fill_body(&context, "fa1 body 1 ret call step 1 mul x 2", &cancel)?;
let module = next.admit(next.binding(), &cancel)?;
# let _ = module;
# Ok(()) }
```

`query` continues to select the released AH1 packet. `query_compact` explicitly
selects AC1. `context.packet_profile()` reports the packet profile, while
`context.binding().profile` remains `fluxr-ah1-v1`, the unchanged authoring-state
identity domain. There is no automatic profile negotiation or fallback.

`SignatureSelection::All` retrieves the complete signature inventory, subject
to the output cap. `Explicit { records, enums, callables }` requests exact nominal
and callable identities. Empty explicit lists select no optional signatures.
Parameters and locals of the target and module inputs are always mandatory.
Selected optional items retain their source declaration order, independent of
the order of selectors. Other declarations' bodies are never returned.

Callables use the exact `(receiver, name)` identity already used by AH1.
`BodyTarget::function("calc")` and `BodyTarget::method("Point", "calc")` are
distinct. A receiver literally named `none` is a real receiver. Optional
`resolve_name` shorthand still rejects ambiguous names. Names must meet the
existing ASCII identifier rules and 64-byte bound. Unknown or duplicate
selectors fail before any header/item budget can hide them.

Explicit selection is a retrieval request, not a dependency analysis. The host
or model must account for the information and token cost used to choose it.
Omitting a signature does not make its semantics irrelevant, prove a helper
suitable, forbid the fill from referencing it, or authorize skipping tests.
The complete existing checker remains authoritative. Unknown dependencies and
unproven effects/resources retain their inherited conservative validation gates.

## Packet grammar and completeness

The ASCII prefix packet has this shape; bracketed text here denotes a value,
not literal packet syntax:

```text
ac1 analysis_only
binding host_ticket
draft [64 lowercase hexadecimal digits]
target function [name]
expected [FA1 type]
effects unproven
resources unproven
capabilities unproven
requires whole_module_check invocation_limits
param [name] [type]
local [name] [type]
input [name] [type]
rec [name] [field count] [name type pairs]
enum [name] [variant count] [name payload-type-or-none pairs]
fn [name] [parameter count] [name type pairs] [result type]
items [visible] [selected] [total]
selection all
context selected_complete
admission none
```

The target can instead be `target method [receiver] [name]`; a method signature
is `method [receiver] [name] [parameter count] ... [result type]`.
`selection` is `all` or `explicit`. `context` is `selected_complete` or
`truncated`. Empty item groups produce no lines. Type, field, variant and
callable signature spellings are exactly those of the existing AH1 renderer.

`visible` counts whole rendered items. `selected` counts mandatory scope plus
requested optional signatures before truncation. `total` counts all mandatory
scope and the full available signature inventory. Therefore `total - selected`
is deliberate nonselection, and `selected - visible` is output omission.
`BodyContext` exposes these through `visible_items`, `selected_items`,
`total_items`, `unselected_items` and `truncated`.

`selected_complete` means all requested items fit. It may coexist with a large
unselected inventory. It does not assert complete semantic analysis, dependency
closure, effect/capability certification, termination or static resource proof.
Neither this status nor an empty diagnostic set admits a program. The packet
always retains unknown obligations and `admission none`.

## Revision authority and response routing

The full draft digest is a visible provenance link to the complete immutable
revision. It is not shortened, but it still grants no authorization and proves
no execution. The private ticket retains every component: AH1 profile, compiler
contract/digest, original source bytes, buffer version, original checked semantic
base, original declaration index, partial sketch, generation and unresolved-hole
binding. `fill_body` compares that full binding and target/index exactly as before.

A model's reply **must be paired with its originating host-retained ticket**.
Do not attach an old body-only reply to the newest query ticket. Maintain the
association across retries, handoffs and asynchronous responses. The packet is
not a serialized ticket and has no deserializer or fill-by-packet operation.
Copying a digest, forging packet text or editing the public binding cannot create
a ticket. AH1's full source/compiler/generation checks remain in force even when
those fields are absent from model-visible text.

Whitespace-only original-source changes still produce distinct revision bindings.
Every successful fill creates a new immutable generation and invalidates old
tickets for that new draft, including tickets for other holes. Old forks retain
their own valid tickets and behavior. Failure does not change the draft. A compact
projection preserves these rules; it does not implement semantic rebase or
cross-process authorization. Hosts must retain exact requests and failed fills
because bounded errors themselves have no revision sidecar.

## Output, work, memory and cancellation bounds

Output accepts 1–65,536 bytes and 0–4,096 items; defaults remain 16,384 bytes and
128 items. An item cap of zero can succeed only when mandatory current scope is
empty. If any mandatory parameter, local or module input cannot fit, the entire
AC1 query returns `OutputLimit`. It never returns an incomplete current scope as
`selected_complete`.

The bounded formatter checks every fragment before allocation/append. Optional
signatures that do not fit are discarded whole. After the first truncated
selected item, later selected signature formatters do not run, but bounded
inventory scanning and cancellation checks continue so counts remain exact.
A conservative 160-byte reserve protects the fixed trailer; a budget that
could fit a differently packed packet may still refuse under this rule.
Fallible allocation failure returns `ResourceLimit`, distinct from requested
output truncation. Header-too-small refusal is `OutputLimit`.

There can be at most 4,096 explicit selectors in total, counted before allocating
query text. Validation uses borrowed slices and bounded source scans without
copying selectors or building an unbounded index. Duplicate checks and membership
checks are scans, not a new incremental dependency index. The existing checked
source declaration, binding, nesting, node and byte limits continue to bound
their work. Cancellation is checked before work, throughout selector/inventory
loops and before returning success. Existing synchronous parser/checker/rebinding
phases remain bounded but are not interrupted mid-stack. No hard cancellation
latency or real-time callback qualification is claimed.

The packet and one pending signature use separately owned text buffers, each
requesting at most the output byte cap. Their combined requested text allowance
is at most twice that cap, excluding allocator/reallocation overhead and fixed
metadata. The returned ticket also clones its bounded binding, target and type.
These are implementation bounds, not measured RSS/heap ceilings. The caller owns
and budgets retained drafts, tickets, exact transcripts and outstanding work.
There is no global query cache or shared publication lock in this API.

## Reproducible execution and token accounting

Run these only after shared resource admission and under the already-running
watchdog described in [resource safety](resource-safety-v1.md). Build outputs,
toolchain homes and temporary directories belong outside verified source trees;
independent full validations use distinct targets within measured resource capacity.

```sh
cargo test --locked --test agent_authoring --test agent_context_compact --test agent_context_oracle
cargo run --locked --release --example agent_context_demo > "$FLUXR_EVIDENCE/ac1-observations.json"
python3 tools/bench_context_compact.py --observations "$FLUXR_EVIDENCE/ac1-observations.json" > "$FLUXR_EVIDENCE/ac1-tokens.json"
```

The example runs four complete checked FluxR programs: arithmetic, arithmetic
failure cases, a checked helper call and shared-list copy on write. Equivalent
checked Rust fixtures include the complete helper and ownership code where
needed. Each AH1/all-AC1/focused-AC1 arm produces the same canonical executable
bytes as direct full compilation. An actual invalid Boolean body is rejected
before the correct fill, and its exact error/request strings are emitted.
Separate synthetic inventories contain 0, 31 and 127 unrelated helpers.
Runtime outputs and unchanged shared inputs are checked against Rust controls.
Independent manually constructed AST and behavioral oracles remain separate
from these author fixtures.

The executing example binary's full SHA-256 is its host-supplied compiler
identity. The observation includes complete source/fill/Rust strings, exact
packets, selectors and complete host bindings, output counters, artifact hashes,
actual errors and runtime observations. Query timing records 31 raw batches of
20 calls per case/arm. These are sequential same-process wall-clock samples;
reported percentiles describe batch averages, not individual-call tail latency.
Single preparation/fill/admission observations are labeled separately. Process
memory requires the enclosing coordinator's measurements; packet bytes alone
cannot establish memory improvement.

The separate `benchmarks/context-compact-v1.toml` catalog preserves the frozen
executable replay catalog and does not add arbitrary candidates to replay
admission. `tools/bench_context_compact.py` counts complete strings with pinned
tiktoken 0.14.0 `o200k_base` and `cl100k_base`, including whitespace, punctuation,
escaping and adjacency. It records literal and JSON-quoted strings. It separately
counts complete source, fill, Rust, packet, protocol/tutorial, host bindings,
recorded exact selector calls and retrieved documentation. Shared setup is
reported with its scope; no free dictionary, amortization or vocabulary training
is assumed.

Scripted message-body sums and joined synthetic transcripts include both a
failed attempt and repair, actual failure text and repeated context. A separate
scenario charges the complete host query record twice if a deployment exposes
it to the model. Those records are JSON observations of actual in-process
arguments, not a released transport or observed provider exchange. Count that
cost when it is visible; hiding it in an implementation record does not prove a
whole-episode saving. Exact record overhead can outweigh focused retrieval on
small scopes. The counter does not equate static sums with provider framing,
cached billing, live latency, accepted-task success or model competence.

No custom tokenizer, new trained vocabulary, model/embedding compatibility,
held-out evaluation or live-agent trial is claimed. Live trials still require
pinned model/tokenizer negotiation, complete tool/repair/review/handoff telemetry
and qualified isolation of arbitrary candidate compilation **and execution**.
The development process is not itself an agent benchmark. The ≤10%
equivalent-Rust source target remains binding; unsuccessful measurements are nonpasses.

## Qualification and rollback

This slice extends bounded localized retrieval and reduces repeated provenance
text without weakening the full binding. It does not finish S24, A09, A12, A19,
I22 or any full task/profile. Malformed-source recovery, expression holes,
source-span fidelity, contextual DAG edits, constrained generation, declaration
incrementality, semantic conflict/rebase, trustworthy caches and complete
dependency accounting retain their own acceptance criteria. Native ML, Vulkan,
audio and concurrent publication boundaries are unchanged.

To roll back use the `query` API while retaining each outstanding
response's original ticket. No reader, executable artifact, runtime or database
migration is needed. Stop issuing compact requests before removing their callers,
and drain or explicitly cancel outstanding work without rebinding responses.
Source-bound validation records, rather than this guide, establish which exact
candidate and context behavior were tested.
