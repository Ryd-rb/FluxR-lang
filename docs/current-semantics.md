# Checked semantics and implementation contracts

This guide describes the checked compiler, portable VM, compatibility readers
and local coordination primitives delivered in this source tree. The
[FluxR 1 guide](unified-runtime-v1.md) defines the current application language
and runtime API. The [requirements](requirements.json) describe the complete
implementation program; [current status](current-status.md) records its limits.
Protocol labels such as FSM v2, FXH v1 and FA1 identify compatibility formats,
not separate current application languages.

## Values, evaluation and resource charging

The checked type system includes unit, booleans, signed and unsigned 64-bit
integers, binary32, nominal records/enums and lists. Checked integer operations
preserve overflow and signed division/remainder errors. Source printing,
comparison, caching and migration preserve raw float bits, including signed zero
and NaN payloads where the relevant operation does not specify normalization.
Native Rust declarations retain Rust semantics; their behavior must not be
inferred from these checked VM rules.

A call first checks input and argument counts. It visits inputs in declaration
order, then arguments in parameter order. Each value visit checks depth, debits
one logical cell, then checks the tag. Admitted parents determine the order of
record fields, variant payloads and list elements. Frame capacity is checked
before frame/slot cells are charged. Instruction dispatch and operand evaluation
consume fuel in execution order. A successful return is followed by result
admission and its cell charges. A result-admission error is therefore possible
after the body has computed a value.

The logical budget counts visits, not distinct allocations. A value shared under
several roots or at different depths is charged at every occurrence. Equal
shared and expanded trees have the same logical cost. The maximum admitted type
depth is 31 when the root is depth zero; deeper declared types and recursive type
definitions are rejected. Recursive functions and nominal methods have separate
runtime frame limits and remain supported by the portable evaluator.

Flat signed and binary32 lists may use a bulk visitor. For a nonempty list it
checks child depth and inspects only the affordable prefix. A bad tag within
that prefix wins over later cell exhaustion. An unaffordable element produces a
cell error before its tag is read. Empty lists visit no child depth. Nested
lists retain their actual depth and order. With a fixed interface and cell cap
`C`, admission inspects at most `C` value tags and attempts at most one further
depth/cell check. This bound concerns already-constructed host values; it does
not bound their prior construction or arbitrary native work.

Workspace reuse must remain valid after success, an arithmetic trap, a malformed
value and each exhausted budget. Portable completion and failure clear retained
operand, local and frame values. Lowering and specialization may preserve the
same logical debits without performing the same physical allocations.

## Ordered control flow, calls and recursion

Checked boolean AND evaluates its right operand only when the left is true;
OR evaluates it only when the left is false. Untaken branches do not evaluate
arithmetic or perform value writes. A taken break charges its forward jump and
skips the next guard. Continue charges its backedge and skips the remaining body.
For nested loops, an inner break can still reach the outer update; an outer
continue skips both the inner loop and that update. These distinctions determine
which fuel, bounds, arithmetic or payload error is observed first.

Call arguments are evaluated before dispatch. Dispatch consumes fuel before
callee frame admission; frame depth is checked before charging the frame and
slots. Recursive calls retain caller values across the callee and can trap on
checked arithmetic while unwinding. A source-level edit must preserve these
orders across portable, decoded, injected and eligible native routes.

The executable semantic fixtures specify exact resource boundaries. Their
numbers are properties of the named programs, not general per-feature costs:

| Fixture | Successful boundary contract |
| --- | --- |
| Signed recursive fold with nonnegative count `n` | `12n + 6` instruction units, `3n + 6` cells, `n + 1` frames, provided arithmetic succeeds |
| Compound record/variant procedure chain | 24 instructions, 36 cells and three frames with populated variants; an empty snapshot variant saves six cells |
| Signed countdown returning a record | Nonpositive count: 6 instructions, 13 cells, one frame and original allocation; positive `n`: `21 + 13(n - 1)` instructions, 18 cells, two frames |
| Nested list update | 18 instructions, 32 cells, two frames for the shared two-element-row fixture |
| Guarded variant payload update | 18 instructions, 24 cells, two frames for its shared two-element payload |
| Indexed nominal method call | 24 instructions, 31 cells, two frames; projected receiver: 23 instructions and 34 cells |
| Recursive nominal self call / nested nominal call | Self: 83 instructions, 63 cells, five frames; nested: 104 instructions, 69 cells, six frames |

The fixtures cover exact and one-short fuel, cell and frame limits, overlapping
errors, deep recursion, empty collections and a successful subsequent call in
the same workspace. Large logical budgets do not grant new native capabilities.

## Copy-on-write values and nominal methods

Host inputs are read-only snapshots. Returning an unchanged record preserves
its allocation. A first write to a shared record copies the complete record and
charges the required cells; further writes to the private copy do not copy it
again. Lists follow the same ownership discipline. A first push on a shared row
charges its existing length plus the new element; a later push to the private
row charges only the new element. A local pop does not alter the original host
snapshot. Result construction and admission retain their own costs.

`empty` and `last` on a direct local list (including a list parameter) borrow
that local without temporarily cloning the list wrapper. Their internal paired
instructions keep both original slots, jump indices and logical charges. Load
fuel is charged before checking initialization; list-operation fuel is charged
before an empty-list `last` error. A successful `last` clones only the selected
element, retaining its nested allocation identity. Inputs, field projections and
computed list operands use their existing paths. Resource failures and later
copy-on-write mutations must remain equivalent to the unfused instructions.

Variant extraction requires the expected nominal tag and payload shape. A tag
guard may bypass extraction. Forced extraction fails for an absent payload or
for a different populated tag even when both tags carry the same underlying
list type. A failed write, later bounds check or result-admission failure must
leave the host allocation unchanged.

Method lookup uses the nominal receiver, not only field layout or method text.
A `Twin` with the same fields as `Point` is not interchangeable with it. A method
requires a correctly typed `self`; duplicate nominal methods, exported methods,
wrong receivers, argument mismatches and incompatible returns are rejected.
Explicit and dotted calls lower to the same checked program and portable bytes.

A method call evaluates the receiver before the first argument, then arguments
in order. An indexed receiver can therefore fail bounds before a division in
argument one, which can fail before payload extraction in argument two. In the
ordered-call fixture these traps become reachable at three, six and eight
instructions; the ninth dispatches the call. A projected receiver saves the
index instruction but has a different boundary-admission cost.

Recursive nominal methods retain this order. A `Point.fold` can recurse on its
receiver and call a distinct `Twin.fold` at the base. Each written receiver
detaches independently; a no-write base path returns the original allocation.
Runtime frame budgets bound recursion. The migration CLI's replay frame cap is
64 and can refuse a deep probe even though a separately budgeted runtime call
is valid. Native preparation and AOT generation may decline these exports.

## Binary32 and scan rules

The `i64tof`, `ftoi64` and `sqrt` operators have explicit bit-level tests.
Integer-to-float conversion must preserve binary32 rounding at integer ties and
large midpoints; conversion via an intermediate representation must not
introduce double rounding. `ftoi64` truncates toward zero, saturates at the
signed bounds and returns zero for NaN. Square root preserves the specified
signed-zero, subnormal and infinity results. A negative nonzero operand or any
NaN yields canonical NaN bits `0x7fc00000` for square root.

Each unary export in the binary32 boundary fixture needs three instructions;
its composed export needs five. Each needs four cells and one frame. Exact-bit
and competing-error checks are required even when an optimizing backend chooses
the portable route for these operations.

`minidx` accepts a binary32 list and returns an unsigned index. It charges operand
evaluation and scan dispatch, then the whole list length before comparing any
elements. A NaN at index zero remains selected. Later NaNs never replace the
selected value. Strict less-than preserves the first tie, including either
order of signed zeros. Empty input returns zero. Return fuel and result-cell
charging occur after the scan.

For a direct input/argument scan of length `n` with `s` export slots, the two
borrowed shapes require one frame, `n + s + 3` cells and `n + 3` fuel. Full scan
fuel is required even when the first element is NaN. Computed lists and
uninitialized locals remain subject to their ordinary evaluation errors. The
portable and prepared scan share the comparison loop; a native guard decline
replays the portable route to choose the correct failure.

## Revision publication and checked dependencies

The structured `ModuleSlot` API grants inputs by exact name/type and receives
values in declaration order. A compatible update retains ordered input bindings, export signatures,
record layouts and enum layouts. Revision and interface checks precede the
optional probe. A failed probe preserves the active revision, encoded module,
working result and host-owned values. A successful replacement publishes a new
immutable revision; retained calls and snapshots continue using their original
module. The current runtime additionally validates native item digests and
source-bound AOT identities as specified in the runtime guide.

A method body edit can leave a caller's own declaration revision unchanged while
changing which method body that caller reaches in its snapshot. Same-named
methods on different nominal receivers have independent revision authority. A
batch containing one invalid edit rejects as a unit. Stale edits, stale
behavioral dependencies and activation prepared against a different snapshot
must not publish.

Each immutable compiler snapshot retains declaration digests, environment
identity, reverse call edges and registered-target lookup indexes.
`impact_since` traverses both graph generations so additions, removals, remapped
targets and changed edit contracts remain observable without hashing every
transitive target closure. `affected_targets` preserves deterministic order.
Behavioral pins include resolved direct/method callees and recursive closures;
a same-batch or previously stale dependency cannot silently authorize an edit.
This graph service covers admitted structured programs. External files,
negative lookups, build options, module interfaces and test discovery need
separately qualified dependency producers before selective CI is justified.

A preparation cache retains at most one selected checked code object per
declaration, with instructions bounded by the module limit. It belongs to the
specific preparation environment. AST equality includes raw binary32 bits.
Different declarations/bodies cannot reuse another entry's code; an uncached
valid proposal still compiles when selected. Whole-module instruction limits are
checked after composition. Empty or completely rejected batches avoid needless
source cloning and checker construction, but several nonempty paths still do
work proportional to the full module.

## Source projections and compatibility codecs

Canonical current source starts with `fluxr 1`. Compatibility encoders and
readers retain their own public contracts:

| Representation | Implementation contract |
| --- | --- |
| Compact structured source | Typed branches, returns, loops and edits lower to the checked source model; canonical printing retains explicit forms |
| FA1 | Fixed-arity prefix expressions and bounded counts; `Source::parse_agent`, `Module::compile_agent`, `Module::agent_text` and `Stmt::parse_agent_body` expose the projection |
| FSM | Checked portable source envelope; decoding recompiles and validates before admission |
| FR1 | Frozen packet alphabet and exact nominal interface; checked lowering retains effect order and raw-bit identities |
| WK1 | Dictionary/version-bound compression and explicit expansion; see [wordcode](wordcode.md) |
| FN1 | Explicit native-source import through the shared project builder; see [native interoperability](native-rust-v1.md) |

The FA1 printer reparses its result and checks AST and exact FSM equality.
Declaration order, float bits, evaluation/trap order, resource accounting and
collaboration identities remain unchanged. Output beyond the permitted size
refuses before return or file creation.

Compact v1/v2 accept `= tail` for a no-local function. A tail is an expression or
`(? test yes no)` with recursive tails. A terminal edit statement can use the
same conditional. These expand to ordinary typed branches/returns with lazy
branch execution, identical charging and identical artifacts. Grouped bindings
such as `x y:i64` work in inputs, fields, parameters and locals. Names expand in
order and the full expanded spelling is charged to the source budget before
types are copied. Canonical printing retains explicit annotations.

The explicit-bound `scan` form expands to the checked indexed loop. Its bound is
reevaluated each iteration; continue skips the trailing increment. It does not
infer a list length, change public interfaces or introduce a different artifact.

FR1 semantic edits target exact tags and literal bits. An edit matching one
tag's negative zero does not alter another tag, positive zero or a NaN payload.
Collection edits preserve the distinction between arithmetic overflow while
computing an index and a later bounds failure. Counts in one pinned tokenizer
are not a universal token price: retain full-stream and cross-tokenizer checks,
including syntax/API context and repairs.

## Portable artifacts and migration

FSM begins with `FSM\0`, a little-endian source version and canonical source.
Decoding rechecks the supported version, UTF-8, syntax, types and resource
bounds. Ill-formed declarations, wrong nominal receivers, invalid payloads,
recursive/deep types, truncated envelopes and oversized source refuse before
slot publication. Rejection preserves the active revision and all host values.
Compatibility fixtures can use older source spellings even when their filename
has the `.fluxr` extension; the document header selects the reader.

FXH v1 uses `FXH\0`, a version byte, a kind byte and a little-endian `u32` payload
length. Kind 0 contains canonical compact structured source. Kind 1 contains a
little-endian `u16` FXD length, checked FXD bytes and the canonical compact rule
projection when that projection is representable. Decoding validates lengths,
rechecks the origin, regenerates the canonical payload and compares the whole
envelope. Checked HIR retains the compact text produced from checked compiler
input; reparsing must recover the same AST.

Migration recompiles generated source and compares canonical artifacts before
exclusive output creation. Receipts can bind exact input/output/artifact bytes;
bounded replay compares exact values and errors for structured calls, rules and
timeline samples. FR1 migration also binds its nominal interface and alphabet.
Failure leaves a missing destination missing and preserves an existing file.
An artifact's checksum identifies its bytes; it does not prove runtime safety or
the correctness of an unexecuted migration.

The retained readers cover timeline v1/v2 text, v1 JSON and LSP v1; rule v1–v4
postfix, `.rule.json` and FXD; structured JSON, compact source and FSM; and FR1
packets. Restore emits canonical FXD or FSM for their existing execution and
metering paths. Exact source spelling can have a distinct input hash while
mapping to the same canonical artifact.

## Legacy rules through checked HIR

Rule select is eager: operands are evaluated and stored in postfix order before
selection, so an unchosen arithmetic arm can still trap. A failed guard returns
zero at its original position. The checked projection preserves that behavior
and signed traps; it must not replace rule select with a lazy structured branch.
The original FXD remains authoritative for its legacy ABI and diagnostics.

`Hir::prepare_rule_projection()` prepares an explicit portable checked route with
instruction, cell and frame limits. Its rule-only compiler allows signed bitwise
AND while ordinary checked boolean AND keeps short-circuit semantics. The same
opt-in route handles all six released raw binary32 operators. Raw operations
read the low 32 bits of signed rule slots; arithmetic returns zero-extended
result bits, including signed zero, infinities and canonical NaN `0x7fc00000`.
Ordered comparisons return signed zero or one. A two-input raw operation or
signed AND uses ten VM instructions, eight cells and one frame.

The general structured source projection cannot represent raw binary32 transport
or rule-only signed AND. FXH therefore omits the compact projection for those
origins while retaining their checked FXD. A representable origin requires its
exact canonical projection. Missing/extra projections, changed guards, trailing
whitespace and invalid UTF-8 are rejected. A projection of 256 KiB reaches the
byte bound and is still subject to canonical comparison; one extra byte is a
byte-budget error.

FXD readers validate magic/version, counts, operand lengths, input indices,
opcodes and stack shape. Version-gated raw arithmetic remains gated. The boundary
fixtures include the 16-input/256-operation limits, early guard return, overflow
and division errors. The legacy rule ABI has no per-call VM limits; the prepared
checked route adds explicit limits without changing legacy telemetry or restored
wire bytes.

## Universal computation and bounded execution

The supplied [tape simulator](../examples/fixtures/universal_tape.flux) implements
a deterministic single-tape machine with a finite alphabet, numbered nonhalting
states and zero as blank/halt. A state and scanned symbol choose a transition
row containing the write, next state and motion. Two growable lists hold the
left/right tape halves, with a separate head cell. Right motion pushes the write
onto the left half and takes the nearest right cell or blank; left motion is
symmetric. This representation preserves the simulated tape at each step.

Under abstract growable storage and repeated invocations this is a universal
simulator. Concrete calls remain limited by fuel, cells and frames. A caller can
request a finite transition count, retain the returned state/tape and resume.
A budget failure returns no intermediate state; retry starts from the prior
checkpoint. The independent checker uses absolute positions and sparse memory,
including halting, bounded nonhalting and multi-symbol machines.

## Local agent tooling invariants

The [pipeline guide](agent-pipeline.md), [concurrent verification](concurrent-verification.md)
and [fair dispatch](agent-fair-dispatch.md) specify the full APIs. These properties
also constrain integration:

- Canonical action/evidence JSON is budgeted and copied to owned containers
  before serialization or identity calculation. Type, depth, cycle, string and
  aggregate-byte errors refuse within bounds. Path labels have byte/component
  limits and reject drive-qualified paths. Validation and hashing use the same
  immutable payload.
- Active scopes are stored transactionally with ownership. Dispatch reads
  control metadata and indexed conflicts instead of all immutable packets.
  Pending-dependency counts and reverse indexes identify ready jobs. Only a
  successful current parent completion releases child edges in the same
  transaction; failure, cancellation, retry, stale tokens and duplicate
  completion cannot release them early or twice. Expiry requires explicit
  reconciliation and does not discard scope authority.
- Required authority tables, clock-floor state and schema-specific counters
  cannot be silently recreated when missing. Default schema 4 supports explicit
  migration to schema 5 for fair dispatch. Stop old controllers, make a
  consistent database/evidence backup and test migration on a copy. No in-place
  downgrade is provided; restore must not revive old worker authority.
- The content store accepts bounded streams and verifies existing bytes without
  a second full-blob allocation. Qualified duplicate insertion rechecks and
  synchronizes the existing object. A retry of a failed job requires the stored
  receipt and referenced failure artifacts. Hashes alone do not authenticate a
  producer; garbage-collection leases and distributed consensus are separate
  capabilities.
- Contract fragments have stable ownership keys, exact dependency digests,
  exports and invariant descriptions. Duplicate ownership, stale/missing
  requirements and guarded replacement conflicts reject before the complete
  proposed result is accepted. Independent edits commute only while their full
  preconditions hold. A retained consumer prevents provider deletion. Invariant
  prose is not executed, and structural validity is not publication authority.
- Evaluation bounds case count and aggregate ledger bytes, freezes each expected
  observation before advancing its iterable, and streams canonical identity.
  Provider items follow started/updated/completed lifecycles; unfinished,
  rebound and late events are rejected, and attempted tools remain visible.
  A complete-only compatibility adapter keeps its explicit contract.

The queue is a single-host SQLite implementation with one writer. Fan-out costs
work per affected edge; scope-blocked ready jobs can require scanning. It does
not provide multi-host filesystem deployment, production authentication,
hermetic execution, distributed publication or automatic semantic merge.

## Verification and measurement contracts

The checkout driver admits 1–32 independent commands with bounded per-command
logs, retains declared result order and serializes commands sharing a Cargo
output group. It supplies each child with a private temporary directory outside
the source tree. Source drift stops admission, cancels ordinary process groups
and invalidates the aggregate. Required missing suites refuse before checks.
Python discovery refuses zero tests and a suite with no successful test/subtest;
expected failures or skips alone cannot satisfy that gate. A successful subtest
with a skipped sibling remains eligible when the whole suite succeeds.

The driver runs trusted source and inherits its environment. Endpoint source
checks are not hermetic execution or hostile-process isolation. Internal compiler
threads and test concurrency remain part of resource planning. An impact-query
result does not grant permission to omit unqualified test coverage.

`benchmarks/cases.toml` is an executable fixture admission boundary. Adding an
experimental token catalog through `bench_tokens.py --manifest` must not add an
executable replay case. Arbitrary generated Rust replay remains disabled until
the separately qualified compilation/execution sandbox and complete adapters
exist. Benchmark composition/repair protocols bind exact full drafts and
context; stale drafts or ambiguous edit spans reject.

For performance, freeze case identities, source, seeds/distributions, outputs,
checked semantics, toolchain, target features, optimization, instrumentation and
resource policy before tuning. Check full outputs outside timing loops and
consume results within them. Compare both an optimized Rust kernel and a host
adapter with equivalent validation, metering and effects. Separate decode,
validation, compilation, lookup, preparation, publication, first call and warm
execution. Per-call and batch-average percentiles are different measurements.

For token cost, compare equivalent behavior and library reuse, name the tokenizer
panel, and count authored source, generated code, bindings, shared grammar/API
context, initial prompts, edits, diagnostics and retries separately. Static
count reductions do not establish model understanding, task success or agent
throughput. A safety claim must identify its property, assumptions, checker,
target and remaining limits.

## Executable semantic references

These source-controlled oracles and tests retain the detailed cases without
requiring prior build logs:

| Area | Reference |
| --- | --- |
| Admission, scan and typed effects | [typed_effect_spec.rs](../examples/fixtures/typed_effect_spec.rs), [typed_effect_checks.rs](../examples/fixtures/typed_effect_checks.rs) |
| Compound and recursive calls | [compound_effect_checks.rs](../examples/fixtures/compound_effect_checks.rs), [recursive_effect_checks.rs](../examples/fixtures/recursive_effect_checks.rs) |
| Loop exits and nesting | [loop_exits_checks.rs](../examples/fixtures/loop_exits_checks.rs), [nested_loop_calls_checks.rs](../examples/fixtures/nested_loop_calls_checks.rs) |
| Binary32 | [binary32_boundary_checks.rs](../examples/fixtures/binary32_boundary_checks.rs) |
| Nested lists, variants and shared values | [list_boundary_checks.rs](../examples/fixtures/list_boundary_checks.rs), [variant_payload_checks.rs](../examples/fixtures/variant_payload_checks.rs), [shared_host_admission_checks.rs](../examples/fixtures/shared_host_admission_checks.rs) |
| Nominal calls, edits and recursive methods | [method_call_order_checks.rs](../examples/fixtures/method_call_order_checks.rs), [method_edit_isolation_checks.rs](../examples/fixtures/method_edit_isolation_checks.rs), [recursive_method_safety_checks.rs](../examples/fixtures/recursive_method_safety_checks.rs) |
| Portable methods and HIR replay | [method_artifact_safety_checks.rs](../examples/fixtures/method_artifact_safety_checks.rs), [method_hir_replay_checks.rs](../examples/fixtures/method_hir_replay_checks.rs), [nested_payload_hir_replay_checks.rs](../examples/fixtures/nested_payload_hir_replay_checks.rs) |
| Rule projection, raw bits and origin checks | [rule_projection_checks.rs](../examples/fixtures/rule_projection_checks.rs), [raw_rule_projection_checks.rs](../examples/fixtures/raw_rule_projection_checks.rs), [composed_rule_hir_checks.rs](../examples/fixtures/composed_rule_hir_checks.rs), [rule_origin_artifact_checks.rs](../examples/fixtures/rule_origin_artifact_checks.rs) |

Run the `bench_typed` and `bench_raw_rule_projection` example test targets for
these bounded semantic checks, then the relevant platform/native routes from
the integration guide. Test availability describes what can be verified; current
status states what was actually executed for this delivery.
