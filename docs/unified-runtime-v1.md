# FluxR 1: compiled applications and one runtime

FluxR has one current source language, identified by the exact first line
`fluxr 1`. The implementation pack's number is independent: this pack implements
language version 1. The runtime is implemented in Rust. Compiling an application
retains that runtime; it does not turn the application into a VM-free program.

## One source and build path

`tools/fluxr.py` is the shared source/project compiler. `fluxr-cli build` invokes
it. The old `tools/fluxr_native.py` is a compatibility/import entry to this
implementation. Current projects use `fluxr-project-v1` and `.fluxr` source files.

The current source accepts checked declarations (`inputs`, `rec`, `enum`, `fn`,
`pub`, `method`) and explicit native interoperability declarations
(`native-use`, `native-fn`, `native-pubfn`, `native-asyncfn`,
`native-pubasyncfn`, and length-delimited `rust`). They belong to one document.
Native declarations are foreign Rust code checked by rustc. They do not silently
inherit the checked VM's arithmetic or execution-budget semantics.

This checked program exports a function taking one signed integer:

```text
fluxr 1
pub answer 1 x int int ret add x 1
```

The first `int` is the parameter type; the second is the result type. Counts make
declaration and expression boundaries explicit. Signed and unsigned checked
integer operations preserve the VM's overflow and trap rules in every eligible
execution tier.

| Current source type | Rust runtime type | Control value example |
| --- | --- | --- |
| `unit` | `Type::Unit` | `"unit"` |
| `bool` | `Type::Bool` | `{"bool":true}` |
| `int` | `Type::I64` | `{"i64":42}` |
| `uint` | `Type::U64` | `{"u64":42}` |
| `float` | `Type::F32` | `{"f32":1065353216}` for the bits of 1.0 |
| `list int` | `Type::List(Box::new(Type::I64))` | `{"list":[{"i64":1}]}` |

Checked records and enums are also available. Native services currently exchange
scalar/list values or application-defined integer handles. A record name does
not establish a Rust memory layout, so nominal records/enums are rejected at this
service ABI until their schemas are explicitly bound.

## Compact binary32 literals

Current source accepts finite decimal `float` literals with an `f` suffix,
including `1f`, `0.5f`, `-0f` and `1e-8f`. The grammar is an optional minus,
one or more decimal digits, an optional decimal point followed by one or more
digits, and an optional `e` or `E` exponent with an optional sign and one or more
digits. A decimal literal is limited to 256 bytes including its suffix, before
numeric conversion; this accommodates an exact decimal spelling of every finite
binary32 value. There is no implicit conversion from an integer literal to `float`.

Decimal text rounds directly to binary32; it does not first round through
binary64. Negative zero is preserved. Underflow follows binary32 rounding,
including signed zero, while overflow to infinity is rejected. Use the retained
`0xHEXf` raw-bit spelling when exact bits are the source of truth, including
infinities and every NaN sign/payload. A decimal literal and its corresponding
raw-bit literal produce the identical checked AST, canonical artifact and
execution charges. Both consume one expression node and obey the existing
source, node, depth and output bounds.

`Module::unified_text()` chooses a shorter decimal spelling only after checking
an exact bit round trip. It keeps raw-bit notation when decimal text is no
shorter, and always keeps it for infinities and NaNs. This is a deterministic
byte-length choice; tokenizer savings are measured separately. FA1 and WK1
retain their released raw-bit readers and printers, and names such as `nanf`
and `inff` remain identifiers. Native Rust literal syntax is unchanged. This
additive spelling does not change the language, artifact or dictionary version.

The project frontend admits the same bounded decimal grammar without converting
its value in Python. Rust owns direct binary32 rounding and overflow rejection.
`build` performs that semantic check before compiling the application. Source
`emit`/`export` operations are structural projections; emitted programs still
admit the exact embedded source through the runtime before user `main`.

The five complete source pairs and matched Rust controls in
`benchmarks/unified-floats-v1.toml` cover affine arithmetic, comparisons, lists,
loops, signed zero, subnormals and retained special values. Both source arms
already use the same update/step spellings, so earlier shorthand savings are
not counted again. Execute semantic tests and token accounting independently:

```sh
cargo test --locked --test unified_float_literals
python3 -B tools/bench_update_tokens.py --catalog benchmarks/unified-floats-v1.toml \
  --output /absolute/external/float-tokens.json
```

The counter reports full-source and JSON-wire tokens using pinned `o200k_base`
and `cl100k_base`, plus separate protocol and validation-host costs. These finite
static pairs do not establish the whole-language Rust-token ratio or live-agent
success. The source lifecycle probe includes raw finite and special-value
corpora by default; enable `FLUXR_SOURCE_DECIMAL=1` for the additional current
decimal-grammar corpus. Compare identical corpus settings and probe bytes.

## Compact checked updates

Current source accepts shorter spellings for an ordinary checked local
assignment. Each occupies one statement in a counted block:

| Spelling | Exact expanded form |
| --- | --- |
| `bump name expression` | `set name add local name expression` |
| `scale name expression` | `set name mul local name expression` |
| `sub name expression` | `set name sub local name expression` |
| `div name expression` | `set name div local name expression` |
| `rem name expression` | `set name rem local name expression` |
| `next name` | `set name add local name 1` |
| `prev name` | `set name sub local name 1` |

For example, this function adds `y` to `x`, multiplies the result by three, and
returns it:

```text
fluxr 1
pub calc 2 x int y int int do 0 3 bump x y scale x 3 ret x
```

The target is read once before the right expression. These spellings expand into
the identical checked AST as their long forms: types, overflow and first-trap
order, binary32 results, logical fuel/cells/frames, and canonical FSM artifacts
remain unchanged. An uninitialized target still fails before an effectful right
expression runs. The parser charges the expanded Binary and Local nodes at their
original depths, so short source cannot evade node or nesting limits. These
forms do not introduce an atomic shared-memory operation or implicit coercion.

`sub`, `div` and `rem` use their existing expression words in statement position.
For example, `div x y` as a statement divides the local `x` by `y`, whereas
`ret div x y` returns the corresponding expression without assigning `x`.
Integer division and remainder retain zero-divisor and signed `MIN / -1` or
`MIN % -1` failures. Subtraction retains signed overflow and unsigned underflow.
Binary32 subtraction/division retain rounding, signed zero, infinity and
canonical-NaN behavior; binary32 remainder remains unsupported. These forms do
not reserve additional names, and targets named `sub`, `div` or `rem` remain
usable through the ordinary explicit `local` expression spelling.

`next` and `prev` use the ordinary signed `i64` literal `1`; they do not infer
an unsigned or binary32 step from the local's type. Signed overflow still traps.
The words remain legal local names, and a statement target is parsed in the same
name position as `set`. Subtracting one is never rewritten as adding negative
one, which can have different overflow behavior.

`Module::unified_text()` prints these spellings only when the assignment's exact
left operand is the same local being updated. Unit signed addition/subtraction
selects `next`/`prev`; other additions and multiplications select `bump`/`scale`.
Other subtractions, divisions and integer remainders select `sub`/`div`/`rem`.
It does not reorder operands, reassociate arithmetic, or shorten assignments
whose left expression is another local or a computed value. Long spellings remain
accepted. FA1 retains its released grammar and long-form printer; WK1 retains
its independently pinned dictionary and expansion contracts. Native Rust
statements are unchanged. Language, semantic, artifact, and dictionary versions
are independent of this additive current-source spelling.

The paired inputs and arithmetic Rust controls are in
`benchmarks/unified-updates-v1.toml`; additional signed steps are in
`benchmarks/unified-steps-v1.toml`. The latter compares with already compact
`bump`/`set` inputs, so earlier shorthand savings are not counted again.
`benchmarks/unified-arithmetic-v1.toml` adds five signed, unsigned, binary32,
variable-divisor and loop pairs for `sub`/`div`/`rem`; both arms already use the
current decimal and unit-step notation. The fixtures test canonical AST/FSM
identity, checked Rust arithmetic, bounded fuel/cell/frame sweeps, malformed
input, native spans and live revision replacement. The corresponding frontend
tests preserve expanded node/depth bounds and the original embedded source.
Run the semantic/budget regression tests and full-string counters separately:

```sh
cargo test --locked --test unified_updates --test unified_steps \
  --test unified_arithmetic_updates
python3 -B tools/bench_update_tokens.py --output /absolute/external/update-tokens.json
python3 -B tools/bench_update_tokens.py --catalog benchmarks/unified-steps-v1.toml \
  --output /absolute/external/step-tokens.json
python3 -B tools/bench_update_tokens.py --catalog benchmarks/unified-arithmetic-v1.toml \
  --output /absolute/external/arithmetic-tokens.json
```

The counter uses the pinned `tiktoken` version with `o200k_base` and `cl100k_base`,
records tokenizer and source identities, and separates authored code, JSON wire
encoding, protocol context, and shared validation/host code. It reports every
pair, including a regression if present. Rust controls match arithmetic kernel
behavior; they do not include the VM's metering or runtime plumbing. These static
comparisons establish neither the complete 0.10 Rust-token goal nor live-agent
success. The counter does not execute the semantic tests on the user's behalf.

## Compile, then start the executable

From this source checkout:

```sh
cargo build --release --locked --target-dir ../fluxr-cli-target --bin fluxr-cli
../fluxr-cli-target/release/fluxr-cli build examples/unified/fluxr.json /tmp/fluxr-hello-project \
  --target-dir /tmp/fluxr-hello-target --logs /tmp/fluxr-hello-build
/tmp/fluxr-hello-target/release/fluxr-hello
```

The example's checked `main` computes 42. The executable prints `{"i64":42}`.
Use fresh output and log directories. Cargo artifacts must be outside source and
emitted project directories. See [integration](integration.md) for build and
resource controls.

The equivalent frontend command is:

```sh
python3 -B tools/fluxr.py build examples/unified/fluxr.json /tmp/fluxr-hello-project \
  --target-dir /tmp/fluxr-hello-target --logs /tmp/fluxr-hello-build
```

Without an explicit compiler binary, the builder bootstraps the exact runtime
source bundled with the emitted project. It invokes the shared checked compiler
for AOT eligibility and records source, compiler, generated code and dependency
identities. The Rust CLI reuses itself as the compiler by default.

The project manifest explicitly lists source and copied files. The checked hello
example is a complete minimal reference. Source entries produce `.rs` files;
Cargo configuration and lockfiles use `copy`. Application Rust source must enter
through the canonical source format, including lossless `rust` items. The builder
supplies the mandatory runtime dependency, merges its locked dependencies and
copies the runtime and entry macro into the emitted project.

The executable starts in this order:

1. The generated entry initializes the process runtime and its real checked core.
2. The bundled catalog is parsed and checked, its native item digests are verified,
   and its modules are admitted together.
3. Eligible generated AOT kernels bind to their exact checked export identity and
   installed revision. Unsupported exports retain a recorded VM route.
4. Application `main` runs. A checked-only project gets this entry automatically;
   an explicit Rust entry is instrumented by a token-tree procedural macro.

Ordinary execution does not launch Python, Cargo or rustc. Native Rust code
executes normally. Checked code enters through the same runtime used for
injection. The `fluxr::program!` macro preserves main's Rust attributes, return
type and crate paths; an async executor attribute can still transform async
main. Unsupported `no_std`, `no_main`, or entries hidden entirely in macros are
rejected. Native code is trusted; the builder is not a sandbox for malicious
Rust macros or build scripts.

`tools/export_multicore_submission.py` exports the challenge application together
with this runtime. A direct source emission still references the runtime entry
macro; omitting runtime linkage produces a compiler error instead of a VM-free
application.

## Run checked source without packaging an application

```sh
../fluxr-cli-target/release/fluxr-cli run answer.fluxr answer '[{"i64":41}]'
```

The response includes `ok`, a result containing `revision`, `value` and `route`,
and an error field. Optional inputs follow arguments:

```sh
../fluxr-cli-target/release/fluxr-cli run file.fluxr export '[{"i64":1}]' '[{"u64":100}]'
```

Inputs are values supplied for this call in declared order, not implicit access
to process state. A source with native Rust items requires its compiled binding;
the source runner cannot silently interpret or discard those items.

## Native services callable from injected FluxR

A trusted application registers a named service and an exact typed signature.
For example, its native initialization can register a real atomic counter:

```rust
use fluxr::runtime::{global, RuntimeError, Type, Value};
use std::sync::atomic::{AtomicU64, Ordering};
static COUNTER: AtomicU64 = AtomicU64::new(0);

fn register_counter() -> Result<(), RuntimeError> {
    global().register_host("counter_add", vec![Type::U64], Type::U64, |args| {
        let Value::U64(delta) = args[0] else { unreachable!("checked signature") };
        Ok(Value::U64(COUNTER.fetch_add(delta, Ordering::AcqRel)))
    })
}
```

After registration, this checked source invokes the service from a real FluxR
expression:

```text
fluxr 1
pub next 1 delta uint uint ret host counter_add uint 1 uint delta
```

The `host` form supplies the service name, result type, argument count, and each
argument's type/expression. The shared checker validates the expression and the
runtime validates the native signature before admission. The VM validates actual
arguments and the returned value. Callback errors become execution errors.
Native panics are caught only with `panic = "unwind"`; an abort profile terminates
the process. Publication locks are not held during the callback. A callback can
therefore call the runtime again without deadlocking on its own registry lock.

The bundled catalog may declare a service before application main registers it.
Calls fail until that exact binding exists. Registration cannot replace an
existing binding or contradict a bundled requirement. Newly injected sources
must already have their required services registered.

There is no finite handwritten allowlist of Rust methods in native source.
Threads, atomics, networking, databases, FFI, generics, traits, assembly and crate
APIs retain their native Rust semantics. An injected checked program gains the
services explicitly granted by its application. Injecting new arbitrary Rust,
new crate dependencies or changed native item bytes requires rebuilding the
application; it is not a hidden compiler inside the running VM.

## Live injection into a compiled process

Set a token of 32 through 1024 bytes in `FLUXR_CONTROL_TOKEN`. Start the compiled
application with a loopback address:

```sh
FLUXR_CONTROL_ADDR=127.0.0.1:49150 ./your-compiled-program
```

The runtime exists with or without this listener. There is no network listener by
default. With port zero, `FLUXR_CONTROL_FILE` can identify a new receipt path for
the selected address; the runtime creates it with restricted permissions. The
receipt contains the address and process ID, never the token.

From another process with the same token:

```sh
fluxr-cli install 127.0.0.1:49150 counter new counter.fluxr
fluxr-cli call 127.0.0.1:49150 counter next '[{"u64":5}]'
fluxr-cli inspect 127.0.0.1:49150 counter
fluxr-cli list 127.0.0.1:49150
fluxr-cli install 127.0.0.1:49150 counter 1 revised-counter.fluxr
fluxr-cli remove 127.0.0.1:49150 counter 2
```

`new` means the module must not currently exist. The initial revision is 1;
replacement requires the exact revision being replaced. Removal and recreation
keep a monotonic counter, so an old revision cannot become current again through
an ABA cycle. Replacing a source requires the same checked input/export and
record/enum interface. Native item changes do not become accepted merely because
their checked signatures match: their build-provided digest must still match.

One request uses one bounded newline-delimited JSON frame and one TCP connection:

```json
{"version":1,"token":"your-process-token","command":{"op":"call","name":"counter","export":"next","arguments":[{"u64":5}]}}
```

The same envelope works through `fluxr-cli serve` over inherited stdin/stdout.
An inherited pipe can use an empty token when the environment has no token.
Unknown fields, versions and malformed JSON are rejected. Ordinary malformed
stdio frames return errors and permit a following frame; oversized or incomplete
framing terminates the session. TCP admission is loopback-only with an explicit
token, bounded workers and read/write deadlines.

## Revisions, native tiers and performance

`Runtime::snapshot(name)` returns a handle that owns one immutable revision.
`snapshot.call(...)` uses it without repeating the module registry lookup.
`Runtime::call(...)` selects the currently active revision for each call. Neither
holds the registry's publication lock while executing checked or native code.

For repeated calls, `snapshot.prepare_export(export)` resolves the export and
execution tier once. Its `call_with(inputs, arguments, limits, &mut workspace)`
reuses a caller-owned `Workspace` and returns the value, revision and an
`ExecutionRoute` enum. This path avoids registry/export lookups and allocating a
route string on every call. The handle safely borrows its retained snapshot;
explicitly prepare a new handle when choosing to adopt a later revision.

Successful replacement affects future lookups. In-flight or deliberately retained
snapshots finish the old revision. This is not on-stack code replacement or
automatic application-state migration. New revisions discard obsolete AOT
bindings; a kernel cannot bind unless its exact checked source/export identity
and expected active revision agree.

The runtime reports actual routes such as `vm`, `aot`, `native_scalar`, `jit` and
`precompiled_scan`. These are execution tiers of the same checked program. AOT
and optional JIT support bounded subsets. For example, a host-call export stays
on the VM route rather than losing its effects through unsupported lowering.

Measure native kernel speed independently from process startup, and checked
execution independently from source validation, compilation, binding and update
publication. A Rust-backed native hot path can preserve its native instructions
while the executable also carries a VM. That does not establish zero overhead
for dynamic lookup, typed value marshaling or newly injected code. The inherited
Rust-speed and 10% source-token goals remain measurement targets, not blanket
properties of every program.

## Limits and compatibility

Canonical source has an 8 MiB byte ceiling; additional parser, compiler and
module resource bounds still apply. Control frames are at most 2 MiB. Runtime
configuration bounds active modules, retained name history, source bytes, native
services and per-call budgets. Native callbacks supply their own I/O deadlines,
thread limits and cancellation behavior. VM fuel cannot interrupt arbitrary
trusted Rust or roll back its external side effects.

Use explicit imports for old checked codecs:

```sh
fluxr-cli import fa1 old.fa1 current.fluxr
fluxr-cli import compact old-source.flx current.fluxr
fluxr-cli import fsm old-artifact.fsm current.fluxr
fluxr-cli import wk1 old.wk1 current.fluxr
python3 -B tools/fluxr.py import-native old.fn1 current.fluxr
```

Imports refuse to overwrite output. Historical dictionary pins and checked
behavior remain validated. Legacy encoders/readers and older domain APIs are
compatibility contracts; their protocol names are not alternative current source
languages. The [native interoperability guide](native-rust-v1.md) specifies current
native forms and explicit FN1 migration.

The multicore example is attributed interoperability with the pinned Rust
submission. Run official conformance and live injection against the final
combined and standalone-export binaries. A challenge win requires a separate
compliant matched-hardware load result; neither import fidelity, code generation
nor local conformance supplies that score.
