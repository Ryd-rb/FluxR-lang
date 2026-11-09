# FluxR usage guide

Run the commands below from the repository root.

## Migration and artifact formats

Timeline v2 supports translated tracks in one source row. The row
`+ 0 128 1 1 1 0:0 500:500 1000:0` defines 128 consecutive object IDs
starting at 0, all with field 1 and linear interpolation. Each successive
track adds 1 to each of the first track's values. Ordinary rows are also valid.
The v1 reader and LSP encoding remain supported.

```sh
cargo build --bin fluxr-cli
target/debug/fluxr-cli migrate-timeline old.flux new.flux
target/debug/fluxr-cli migrate-structured old.fluxm new.flux
target/debug/fluxr-cli migrate-rule old.rule.json new.flux
```

Migration creates a new file after the generated source compiles to the same
canonical artifact. Timeline migration accepts v1 and v2 text, timeline JSON,
and LSP bytecode. Structured migration accepts JSON, compact source, and FSM
bytecode. Rule migration accepts rule text, `.rule.json`, and FXD bytecode.
Structured and rule output preserve the original semantic version.

```sh
target/debug/fluxr-cli --receipt --replay samples.json migrate-rule old.rule.json new.flux
python -B tools/fr1/compile.py old.fr1 new.flux --migrate --interface interface.json --compiler target/debug/fluxr-cli
```

`--receipt` prints versioned JSON with SHA-256 hashes of the exact input,
generated source, and shared canonical artifact. FR1 migration also binds
the exact host interface and frozen alphabet. Each command refuses an existing
destination. FR1's compilation and `--emit-source` options remain available.

`--replay` accepts a JSON array of 1–256 probes, up to 256 KiB. Use
`[object, field, time_ms]` rows for timelines and positional input arrays for
rules. Structured and FR1 probes use objects with `export`, `inputs`,
`arguments`, and optional `limits`. Values use the serialized host `Value`
representation, such as `{"i64":7}` and `{"f32":2147483648}`. Omitted limits
use the VM defaults. Explicit limits may lower them. Replay compares results
and exact errors through the old and migrated programs and records the probe
file hash and case count in the receipt.

Use FXH v1 to round-trip checked rule and structured input through a shared
migration IR. FR1 uses its frozen alphabet and nominal interface before it
enters the structured checker.

```sh
target/debug/fluxr-cli --receipt migrate-rule-hir old.rule.json program.fxh
target/debug/fluxr-cli --receipt migrate-structured-hir old.fluxm module.fxh
python -B tools/fr1/compile.py old.fr1 packet.fxh --migrate-hir --interface interface.json --compiler target/debug/fluxr-cli
target/debug/fluxr-cli restore-hir program.fxh restored.fxr
```

The FXH receipt binds the exact input, FXH output and restored artifact.
`restore-hir` recreates canonical FXD or FSM bytes. A rule's original FXD
preserves its execution contract. The Rust `Hir::source` API exposes the
shared structured representation where available and refuses rules that
require integer bitwise and or raw binary32 transport.

## Benchmarks

Run `python -B tools/bench_tokens.py` with Python 3.11+ and `tiktoken` installed.
It counts exact UTF-8 text with LF line endings using `o200k_base`. Whole-file
input and output have the same count when the payload is identical. For a case
with distinct replacement or edit payloads, add an `output` table with paired
`fluxr` and `rust` paths. List host bindings, generated native source, and
shared task context separately. Counts exclude provider framing and tool metadata.

Run `cargo run --locked --release --example bench_volume` for matched checked
volume interpolation in Rust, the structured VM, and ahead-of-time compiled FluxR.
Run `cargo run --locked --release --example bench` for the timeline benchmark.
Register new source pairs and corresponding runtime commands in
[`benchmarks/cases.toml`](../benchmarks/cases.toml) when a new language case warrants
measurement. Use the same semantics, inputs, target, and build profile.

## Run the browser demo

The browser build uses the Rust `wasm32-unknown-unknown` target and
`wasm-bindgen-cli` 0.2.130, which matches the library dependency.

```powershell
cargo test --locked
cargo run --locked -- test web/well.fluxr web/well.replay.json
cargo build --locked --release --lib --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir web/pkg target/wasm32-unknown-unknown/release/fluxr.wasm
python -m http.server 58843 --bind 127.0.0.1 --directory web
```

Open http://127.0.0.1:58843 and edit either representation. The other view
updates when the compiler accepts the edit. A rejected edit leaves the
running program available.

Use `Program::bind_track` in Rust or `Scene.bind_track` in the browser when sampling the same timeline channel repeatedly. Bindings share immutable keys and retain the program's period. Bind each accepted replacement program before switching the active scene.

## Decision rules

`r1 2 i0 i1 < 100 0 ?` accepts two signed 64-bit inputs and returns `100` when input 0 is smaller. The equivalent `.rule.json` is `[1,2,[[0,0],[0,1],[7],[1,100],[1,0],[11]]]`. Use `fluxr-cli test rule.flux cases.json` with cases such as `[[[2,3],100],[[3,2],0]]`. The browser `Rule` binding runs the same replay. `encode` writes a versioned `FXD` program to `.fxr`.

V1 tokens in binary opcode order are `iN`, integer, `+`, `-`, `*`, `/`, `%`, `<`, `=`, `m`, `M`, `?`. V2 adds `&` (bitwise and, opcode 12). V3 adds ordered binary32 `f<` and `f<=` (opcodes 13 and 14). These read the low 32 bits of each operand as IEEE 754, compare signed zeros as equal, and return 0 for any NaN operand. In V3, `f32:0.03` is authoring shorthand for the integer bit encoding of the rounded binary32 constant. Canonical text and typed programs use the integer encoding to preserve every bit.

V4 adds binary32 `f+`, `f-`, `f*` and `f/` (opcodes 16 to 19). Each operation rounds independently to binary32 and returns its zero-extended bits. NaN results use the fixed encoding `0x7fc00000`. Division by zero follows IEEE 754. V4 text supports `#` line comments.

Operators are postfix. `m` and `M` mean minimum and maximum. Comparisons produce 0 or 1. `?` takes condition, true value and false value in that order. Both values are computed before selection. V3 `guard` (opcode 15) consumes one value, returns zero immediately if it was zero, and otherwise continues without pushing a value. Arithmetic errors fail evaluation. Rules permit 16 inputs, 256 instructions and 32 stack values. Every program produces one value. Successful execution allocates no memory and performs no host I/O.

For bundled rules, `RuleProgram::rust_function` generates a typed function for the host build to compile and inline. The FluxR source stays authoritative and both execution paths use checked arithmetic. For rules supplied at runtime, `RuleProgram::prepare` retains the portable program and chooses an available executable backend. The optional `native-jit` feature enables experimental Windows x86-64 code generation for validated integer rules. Platforms without that backend use the portable interpreter.

## Structured modules

Structured modules are a separate versioned language for Rust embedding. The
source is a JSON syntax tree, and Rust producers can construct the same typed
`fluxr::structured::Source` directly. The compiler validates names, nominal
types, calls and control flow, then lowers the module to an in-memory instruction
sequence. The portable `FSM\0` envelope stores canonical source and decoders
revalidate it. Existing timeline and scalar rule formats have separate entry
points and semantics.

Version 1 has typed local slots, conditional branches, while loops, break and
continue, function calls and recursion, record construction and field updates,
nominal enums with payloads, and growable typed lists. A method declares a
record receiver and takes that record as its first `self` parameter. Values use
copy-on-write. A method can update its local receiver and return the next value
to its caller. Lists support push, pop, last, indexed reads and empty tests.
Without host-imposed bounds, two growable lists and a loop can represent the
left and right halves of an arbitrary Turing machine tape. The runtime always
runs under an explicit invocation budget.

Signed and unsigned 64-bit integer arithmetic is checked. Binary32 values cross
the host boundary as their IEEE bits. Comparisons follow ordered IEEE behavior,
and arithmetic canonicalizes NaNs. Boolean `and` and `or` short-circuit.
Instruction fuel, aggregate value-cell charges and call frames are independent
limits. Acyclic value schemas bound the nesting of records, enum payloads and
lists. Module source, declaration count and code size have separate validation
limits.

Structured module version 2 adds `arg_min_f32`. It returns the first index
whose value is strictly least under the same ordered comparison, or zero for
an empty list. The instruction charges one additional unit of fuel per value.
Version 1 source and encoded modules still decode with their original rules.

A host supplies typed snapshot inputs in declared order, and arguments and
results by value. The module has no ambient filesystem, network, clock or game
mutation access. A `ModuleSlot` validates declared inputs against the host's
grants. Replacement checks the expected active revision and the host-visible
interface, including reachable record and enum schemas. The host can run an
isolated candidate probe before activation. It commits returned state only
after a successful invocation. Hosts that share a slot across threads can
serialize replacement at their own transaction boundary.

The live example accepts JSON lines on standard input and reports its PID,
active revision, state and error on standard output. Run it with
`cargo run --example structured_live -- examples/structured-counter.fluxr`.
Send `{"op":"tick","count":6}` to run one transaction. An injection request
supplies `op: "inject"`, the `expected_revision`, and a JSON or compact
`source` string.

On Windows x64, `--features native-jit` also enables runtime compilation for
straight-line structured exports that use only signed 64-bit arithmetic.
A first successful invocation can prepare its checked body for future
calls. Hosts can prewarm an eligible body with `Module::prepare_native`.
Both startup and injected modules share this path. The VM handles unsupported
bodies, invalid arguments and limits below the native body's bound.

## Compact structured source

Use compact typed source when it uses fewer tokens than JSON. This function checks route expiry.

```text
v2
(pub route_expired (now:u64 since:u64) bool ()
  (ret (&& (<= since now) (<= 6000u (- now since)))))
```

A function declares parameters, result, locals and a sequence of statements.
Use rec for records, enum for enums, fn for private functions and pub for
exports. A method qualifies its name with the receiver type. Types in brackets
are lists, and # marks a nominal enum. Field access uses a dot. Unsigned
integers end in u, floats end in f, and 0x followed by binary32 bits and f
preserves every bit. Group branches and loop bodies in parentheses.

Source::parse_compact and Module::compile_compact use the existing checker and
VM. Stmt::parse_compact_body checks the edit syntax before typed composition.
The encode-structured CLI command emits the same portable envelope for either
released JSON or compact input. The project targets at most 10% of equivalent
Rust source tokens using reproducible, behavior-matched counts.

Compact collection forms lower to existing checked statements. A find returns
the first index that passes a predicate, or its stated bound. An indices form
collects matching indices in ascending order. Declare the index, row and
output locals with the usual types. The loop retains index errors, evaluation
order and metering. A when guards one statement. Ampersand and pipe chains
evaluate left to right and short-circuit.

Lazy gates express boolean decisions over consecutive facts named a through
p. A flat gate such as &abcde requires all its facts. A vertical bar accepts
the first true fact. Nested gates use the compact expression syntax.
Gate::rust_function generates inline Rust with lazy fact closures. Neither
an expensive fact nor an authority effect is evaluated past a short-circuit.
The emit-gate CLI command produces a fixed-name sample for local comparisons.

### FR1 packed structured source

FR1 is a bounded authoring format for an ordered-effect export and collection
exports. Its header names every export. Its payload carries structural types
and policy instructions using a frozen 16-bit word alphabet derived from
o200k_base. The host supplies a separate nominal interface containing only
record, variant and function declarations. Python 3.11+ reads the frozen
alphabet from disk. The authoring tool sends the decoded program through the
ordinary FluxR typed compiler, which emits the existing portable FSM format.

```sh
cargo build --bin fluxr-cli
python tools/fr1/compile.py ruler.fr1 ruler.fsm --interface interface.json --compiler target/debug/fluxr-cli
```

The interface file has `records`, `enums` and `functions` arrays matching the
corresponding fields of a structured source. Function entries contain
`name`, `export: true`, `params` and `result`. Use `--emit-source` to inspect the
generated compact FluxR. Use `--edits edits.txt --emit-packet edited.fr1`
to type-check and re-encode semantic edits. A line `c export=expression`
replaces a collection declaration's body with the same positional arguments.
A line `o tag old new` replaces matching binary32 literals in emit instructions
for the numeric tag, including guards and payloads. All lines are applied in
order. Files are published only after typed compilation succeeds. Gameplay can
load the compiled module without the alphabet or authoring tools.

For a straight-line export, `--emit-native path.rs --native-export NAME`
also emits typed Rust and the compiled module's SHA-256. Hosts must match that
identity, validate argument types and use `Module::native_ceiling` for budget
admission. The metered VM remains available for edited programs, smaller
budgets and native argument or arithmetic errors.
Pass `--native-loops` to emit a supported loop with a shared loop-body quota.
The host supplies that quota and checks `Module::native_loop_ceiling` with the
same value before invoking generated code. Quota exhaustion delegates the
original invocation to the metered VM.

Use `--compact` in place of `--interface` to compile compact structured source
through the same checked emitter. To regenerate the benchmark's native fixture,
build `fluxr-cli` and run the following command. On Windows, append `.exe` to the
compiler path.

```sh
cargo build --locked --release --bin fluxr-cli
python tools/fr1/compile.py examples/bench_volume.flux target/bench_volume.fsm --compact --compiler target/release/fluxr-cli --emit-native examples/fixtures/bench_volume_native.rs --native-export volume
```

## Concurrent structured authoring

A structured module can opt in through a separate manifest of stable target
IDs. Build a collaboration Snapshot from the structured Source and its Targets.
Names and receivers select declarations in the manifest. A Change carries an
author, declaration assumptions, and an atomic list of typed edits. JSON uses
the same representations as Rust. A host should authenticate and authorize a
change before admitting it. The composer does not assign authority to author
strings.

Call Snapshot::edit_revision when preparing an edit. Exclusive declarations
accept a replacement body against their current revision. Identical replacements
coalesce. Different valid bodies for the same target hold all affected changes.
Snapshot::revision pins reads of the effective declaration, and a declared
dependency changed by another candidate holds the dependent change. The min_f32
hook accepts finite, nonnegative binary32 limits against a stable contract pin
and folds them into its literal baseline. It adds no per-contributor VM work.
Late contributions commute with earlier ones. This initial hook accumulates
limits within a lineage. The host starts a new baseline to remove one.

Snapshot::prepare has no mutable shared state. It deduplicates proposals by
content hash, reports holds, and checks the composed module. A candidate can
contain both admitted and held proposals. The host keeps the proposal artifacts
and chooses when to activate the admitted snapshot. LiveProgram::activate
compares the pinned active program ID and records the host-supplied tick.
Prepare again from the new active snapshot if another activation won first.
The host owns its world state and tick boundary. Existing ModuleSlot and
structured module versions retain their interfaces and encodings.

UnitCase pins an immutable program, typed arguments and host inputs, VM limits,
and expected output. The runner uses a private workspace and enforces the
host's limit ceiling. TestRun forks host state and owns its own active program.
Host state cloning must isolate mutable effects. A deterministic runner can
reuse immutable inputs while each fork keeps independent state. This API leaves
proposal storage, external service isolation, release assertions, and authority
admission to the host.

## Local telemetry

Telemetry starts disabled. Set `FLUXR_TELEMETRY` to `all` or a comma-separated
list of feature names. Pass `--stats` to the CLI to emit a JSON snapshot on
stderr.

```powershell
$env:FLUXR_TELEMETRY = 'all'
cargo run --locked -- --stats test web/well.fluxr web/well.replay.json
```

The Rust API exposes `fluxr::telemetry::configure` and
`fluxr::telemetry::snapshot`. A long-running native host can set
`FLUXR_TELEMETRY_SNAPSHOT` to a writable path and call
`fluxr::telemetry::configure_from_env()` and
`fluxr::telemetry::start_snapshot_worker()` to write snapshots locally.
The optional `FLUXR_TELEMETRY_CONTROL` file accepts `off`, `all` or a list
of feature names for changing collection while the host runs. For browser
inspection, use `sceneEditor.telemetry.configure(selection)` and
`sceneEditor.telemetry.snapshot()`.

Feature names are `compile-text`, `compile-typed`, `encode`, `decode`,
`replay`, `sample`, `sample-batch`, `reload` and `rule-evaluate`.
Counters are observational and do not affect replay or compiled output.

## Architecture decisions

- The AI authoring paths share typed IDs, semantics, and diagnostics. Host schemas should supply only the relevant channels and capabilities. Humans can direct the AI and inspect the running result.
- The Rust host owns I/O and performance-critical kernels. FluxR code receives
  the inputs and capabilities that the host chooses to expose.
- Released source, behavior, portable programs, script-owned saves, and mod data keep their versioned readers and semantics. Fixes that change behavior use an opt-in version. Keep immutable conformance vectors for each released version. Prototype format and limits are experimental.
- Multiplayer mods declare capabilities. The server validates the package, permissions, and rules version. Participants run the same approved module with deterministic inputs. Host I/O and presentation need separate capabilities from authoritative rules.
- Use the same scenario runner in native and browser builds. Measure edit latency and compare execution with the same direct Rust workload before moving hot paths. Linux execution is a first-release gate.
