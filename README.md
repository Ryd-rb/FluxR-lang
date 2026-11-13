# FluxR

FluxR is a compact language for agent-authored programs embedded in Rust
applications. **FluxR 1** is the current source language. Its compiler, portable
VM and eligible native execution tiers share checked semantics. Every compiled
application includes the Rust runtime, so checked code can be installed and
replaced while the application runs.

This directory is the complete **v25 implementation pack**. Start with the
[integration guide](docs/integration.md) for importing it into a repository,
building, and running the integration gates. The
[requirements](docs/requirements.json) preserve the implementation contracts;
[current status](docs/current-status.md) distinguishes implemented capabilities
from checks and product goals that remain open.

## Build and run

Use the pinned Rust toolchain and Cargo lockfile. Python 3.11 or newer is needed
by the project builder. Keep Cargo artifacts outside the source directory:

```sh
export CARGO_TARGET_DIR="$(pwd)/../fluxr-target"
cargo build --release --locked --bin fluxr-cli
"$CARGO_TARGET_DIR/release/fluxr-cli" build examples/unified/fluxr.json \
  ../fluxr-hello-project --target-dir ../fluxr-hello-target --logs ../fluxr-hello-build
../fluxr-hello-target/release/fluxr-hello
```

The checked hello application prints `{"i64":42}`. Emission and log directories
must be new paths; the application target must be outside both source and
emission directories. Fetch the locked dependencies before using offline build
mode. The builder's `--online` option permits dependency retrieval when needed.
The emitted executable runs without Python, Cargo or rustc.

The Python command `python3 -B tools/fluxr.py build` provides the same builder.
It includes the exact runtime, validates the source catalog, binds eligible AOT
exports to their checked source identity, and enters the application. Exports
outside the AOT subset use the VM and report that execution route.

A checked file can also run directly. Save this as `answer.fluxr`:

```text
fluxr 1
pub answer 1 x int int ret add x 1
```

```sh
"$CARGO_TARGET_DIR/release/fluxr-cli" run answer.fluxr answer '[{"i64":41}]'
```

The result identifies its revision, value and execution route. The
[language and runtime guide](docs/unified-runtime-v1.md) specifies project
manifests, Value encodings, native services, live control and limits.

## Native interoperability and live updates

A source document can contain checked declarations, explicit native declarations
and length-delimited Rust items. Native code can use Rust crates, ownership,
threads, atomics, networking and databases. It retains Rust's semantics and trust
requirements. Typed services registered by the application are callable from
checked FluxR, including code installed after startup.

Each installed module has an immutable checked revision. Calls retain their
selected revision; a successful replacement affects future lookups. Publication
requires the expected revision and compatible checked interfaces. Invalid,
stale, incompatible or unbound native changes leave the working module intact.
AOT bindings belong to exact source/export identities and cannot silently remain
active after replacement.

Use the Rust API directly or enable an authenticated loopback endpoint with
`FLUXR_CONTROL_ADDR` and `FLUXR_CONTROL_TOKEN`. The CLI exposes `install`, `call`,
`inspect`, `list` and `remove`, plus a bounded `serve` protocol over inherited
standard input/output. The runtime also operates without a network listener.

Trusted callbacks own their I/O deadlines, thread limits and cancellation
policy. Checked fuel cannot interrupt arbitrary native code or roll back its
external effects. Injected checked code can call granted services; introducing
new Rust code, dependencies or changed native item bytes requires rebuilding.
The service ABI supports owned scalar/list values and application-defined
integer handles; nominal records and enums require explicit schema binding.

## Source layout

| Path | Purpose |
| --- | --- |
| `src/`, `crates/fluxr-entry/` | Runtime, checked compiler, CLI, execution tiers and application entry macro |
| `tools/` | Canonical project builder, compatibility tools, source exporter, verification and agent tooling |
| `tests/`, `tools/unified_tests/` | Behavior, failure, concurrency, control and executable integration checks |
| `examples/unified/` | Minimal checked application and native-host examples |
| `examples/cookbook/` | Paired Rust/FluxR examples, compatible compact sources and checked test catalog |
| `examples/multicore-server/` | Attributed native interoperability and conformance fixture |
| `examples/fixtures/`, `benchmarks/` | Semantic oracles, compatibility fixtures and matched measurement inputs |
| `docs/` | Current interfaces, semantic contracts, integration instructions and implementation requirements |
| `web/`, `syntaxes/` | Browser example and syntax support |

## Semantic and compatibility contracts

The [checked semantic guide](docs/current-semantics.md) describes evaluation and
trap order, resource charging, binary32 behavior, copy-on-write values, nominal
methods, portable artifacts and compiler dependency services.
[Prepared execution](docs/prepared-execution.md) explains the native subsets,
ownership, fallbacks and measurement boundaries.

Enable `typed-jit` for generated native execution of eligible checked programs
on Linux GNU x86-64 and Windows x86-64. Checked scalar `u64` addition, subtraction,
multiplication and comparisons use unsigned native operations and preserve the
full 64-bit range. The Rust AOT emitter admits the same scalar operations.
Prepare an export once and reuse its `PreparedCall` for repeated calls. Acyclic branch and call graphs can reserve
their conservative fuel/cell ceiling at entry, removing repeated metering from
the generated kernel. Tight budgets still use the exact portable evaluator;
loops retain dynamic metering with exact straight-line region reservations.
Conditional successors and unconditional cycles retain checks; declined
reservations replay the portable evaluator for exact first errors. AOT kernel identities and portable artifact bytes
remain unchanged. Native execution is attributed per call, and performance
must be measured with preparation and host overhead reported separately.

Eligible fixed-bound signed calls with up to four combined input and argument
values use a scalar JIT entry. Boolean, signed and unsigned integer calls in that
same lane limit use a separate tagged scalar entry with strict per-lane and result
tags.
Scalar, tagged-scalar, small-buffer and wide-buffer entries own only their
required boundary metadata; generic dispatch does not allocate scalar scratch. The precompiled binary32 minimum-index scan
validates element tags while selecting its result in one pass. Both retain
exact boundary checks and portable replay on a declined invocation. Use the
[fragment measurement guide](docs/performance-fragments.md) for broad checked
language coverage, individual-call and batch distributions, every empirical
integer percentile and explicit cross-version regression reporting. The finite
corpus and a local benchmark do not establish universal tail improvement.

Current source accepts `bump x amount`, `scale x factor`, `sub x amount`,
`div x divisor` and `rem x divisor` for local arithmetic updates,
and `next x` / `prev x` for signed addition/subtraction of exactly one.
They expand to the ordinary checked assignment and arithmetic, preserving
canonical module bytes, source limits, evaluation order and execution charges.
The current-source printer selects these shorter forms; older source readers
retain their released syntax. Finite decimal binary32 literals such as `0.5f`
reduce ordinary float-source tokens while preserving exact rounded bits and
negative zero. Raw-bit literals remain available for every encoding, including
infinities and NaN payloads. The printer selects a shorter decimal form only
after an exact bit-level round trip. The language guide includes paired
source/Rust fixtures and pinned-tokenizer measurement commands.

FA1, compact source, FSM and WK1 remain supported source/artifact codecs. The
`import` command converts checked programs into current source. FN1 is an
explicit native import/delegation adapter. Compatibility encodings, dictionary
pins and observable behavior remain contracts; their names do not select
separate current languages or a runtime-free application build.

The authoring/context APIs, domain buffers, streams, stateful checkpoints,
Vulkan facilities and offline audio examples are retained with their documented
scope. Start with [agent authoring](docs/agent-authoring-v1.md),
[local agent coordination](docs/agent-pipeline.md),
[native domains](docs/native-domains-v1.md) and
[Vulkan execution](docs/vulkan-vkf1.md).

## Validation and project goals

The [cookbook](docs/cookbook.md) covers checked algorithms, collections, nominal
values, floating-point bits, inputs and explicit failure cases. Every pair uses
the same Rust reference file in its correctness checks and measurement harness.
Run checked-source tests with `fluxr-cli test-suite examples/cookbook/tests.json`.
The [testing guide](docs/testing.md) describes filtering, ignored cases,
resource limits, seeded scalar and flat bool/i64/u64 list properties,
bounded deterministic shrinking, replay and portable/prepared comparisons.
Native applications retain Cargo's test harness, including explicit test
arguments after `build ... --test --`.

The [semantic campaign](docs/semantic-campaign.md) generates bounded signed
integer/Boolean programs and branches, then compares portable and prepared
execution with an independent result and first-trap model. Dedicated frozen
fixtures, replayable corpora, bounded reduction and mutation-detection checks
exercise this declared scalar subset. The campaign uses a test-only Rust API
bridge and keeps generated results outside the source tree.

The [syntax-highlighting guide](docs/highlighting.md) describes current `fluxr 1`
TextMate scopes, native quoted tokens, decimal/raw-bit floats and arithmetic
updates, alongside preserved legacy scopes. It includes real TextMate/Oniguruma
fixtures and explains why a regex grammar cannot recover exact native UTF-8
byte-span boundaries or validate a program.

Run `cargo test --locked` with an external `CARGO_TARGET_DIR` for core behavior.
Use the integration guide for the required Python suites, current CLI checks,
compiled-application tests and optional feature/platform gates. The checkout
verifier requires a clean Git checkout; commit the reviewed integration before
using it. Qualification tools keep their outputs outside the source tree.

Use the [deterministic source-pack tool](docs/release-pack.md) to create,
validate and safely extract clean implementation ZIPs. Pack 25 binds the
selected source bytes and modes to a manifest and checks repeated archive byte
equality; packaging checks remain separate from runtime qualification.

The goals are performance close to equivalent optimized Rust and source tokens
at most 10% of equivalent Rust. Both require behavior-matched measurement with
host bindings, setup and context accounted for. Report startup, checking,
compilation, preparation, publication and warm execution separately. Native
capability, compilation, local conformance and microbenchmarks do not establish
universal performance parity, full domain qualification or an official challenge
score. See current status for the remaining integration and product gates.

FluxR is MIT licensed. Retain the root license and the notices accompanying
third-party tokenizer data and the multicore reference application.
