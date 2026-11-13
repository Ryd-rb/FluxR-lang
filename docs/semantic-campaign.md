# Bounded semantic differential campaigns

`tools/semantic_campaign.py` generates seeded signed-integer/Boolean programs
and compares portable and prepared execution with independently computed
results and exact first-trap expectations. It provides replay, bounded reduction
and mutation-detection checks. `examples/semantic_campaign_driver.rs` is a
test-only bridge to the existing public Rust APIs; it computes observations,
never expected answers. The campaign uses existing runtime and compiler
interfaces without production test hooks.

The campaign is a preparatory scalar slice of S25. The broader model,
release-wide coverage and product qualification requirements remain open.
Generated results, replay corpora and reduction outputs belong outside the
source tree. The committed fixtures are reusable test inputs, not receipts from
a previous run.

## Supported programs and observations

The generator uses a typed, bounded test AST and emits the released compact v2
source format. Each program has one function, scalar parameters and locals,
and a final top-level return. Statements may assign locals and contain nested
branches. The declared subset is:

| Area | Included behavior |
| --- | --- |
| Signed integers | Exact `i64` values, checked addition, subtraction, multiplication, division and remainder; signed equality and ordering |
| Booleans | Exact Boolean values, equality, negation and lazy `and`/`or` |
| Control and locals | Assignments, nested `if` statements, taken and untaken branches, and dynamic uninitialized-local probes |
| Boundaries | Ordered scalar arguments, deliberately invalid invocation probes and per-call instruction/cell/frame limits |
| Observations | Exact scalar result or complete first error text, compared separately for portable and prepared execution |

Signed division truncates toward zero, and the remainder has the dividend's
sign when nonzero. Overflow, division by zero and the signed minimum divided or
remaindered by minus one are checked. Short-circuiting determines whether a
right operand is evaluated and charged. The model independently follows the
declared entry, operand, control-flow and result-admission order, including
which resource exhaustion or semantic error occurs first.

Generated functions have no declared host inputs. Valid calls therefore use an
empty input slice; deliberate input-count probes test the invocation boundary.
There are no loops, function calls, recursion, early nested returns, host
effects, unsigned integers, floating-point values, collections, nominal values
or ownership/COW observations in this campaign. Integer bitwise/shift rules
from other FluxR representations are outside this subset.

The public runtime exposes `Error` as a complete display string. Accordingly,
an **exact first trap** is equality of the entire first error string, with no
substring or error-category matching. The runtime supplies no trap site or
structured error-code field. Model-local event witnesses describe the oracle's
reasoning and must not be presented as runtime-observed source locations.

Each call is checked against the oracle independently for both production
routes. Agreement between portable and prepared execution cannot by itself
pass a case. A compile/prepare failure for a supported valid program is a
failure, not an expected runtime trap. Unsupported model input is rejected
before claiming semantic coverage; the model never falls back to a production
evaluator for an answer.

## Build the observation bridge

Use the pinned toolchain, locked dependencies and one build job. Keep Cargo
artifacts and outputs outside the checkout, and apply the shared
[resource coordinator](resource-safety-v1.md) for sustained work.

```sh
FLUXR_SEMANTIC_WORK="$(mktemp -d "${TMPDIR:-/tmp}/fluxr-semantic.XXXXXX")"
export CARGO_TARGET_DIR="$FLUXR_SEMANTIC_WORK/cargo"
cargo build --release --locked --features typed-jit \
  --example semantic_campaign_driver --jobs 1
```

On Windows, the example executable has the `.exe` suffix. The supported
typed-JIT platforms are described in [prepared execution](prepared-execution.md).
The Python process collector currently requires POSIX; maintained process
qualification runs on Linux. Corpus generation and pure model tests are
platform neutral. The Rust bridge itself can be built and tested on the
runtime's other supported platforms, but that does not qualify this Python
collector there.

Omit `--features typed-jit` to build a portable bridge. Prepared execution may
then use its portable fallback; this remains a valid comparison of the two
public API paths but supplies no native-execution evidence.

The bridge calls `Module::compile_compact`, `run_portable_with`, `prepare_call`
and `run_with_route`. It keeps separate portable/prepared workspaces and reuses
each across a program's ordered calls, including failures followed by successes.
This checks workspace recovery as well as isolated call outcomes. It records
the actual prepared route for every call. `--require-native` requires at least
one successful, oracle-matching native observation; compiling with a feature or
obtaining a prepared handle does not satisfy that requirement.

## Generate and run

Generate a retained corpus, including the default frozen fixtures, without
executing the implementation:

```sh
python3 -B tools/semantic_campaign.py generate \
  --seed 243 --programs 64 --calls-per-program 8 --max-depth 4 \
  --output "$FLUXR_SEMANTIC_WORK/corpus.json"
```

Run a new generated campaign with the committed frozen fixtures:

```sh
python3 -B tools/semantic_campaign.py run \
  --driver "$CARGO_TARGET_DIR/release/examples/semantic_campaign_driver" \
  --seed 243 --programs 64 --calls-per-program 8 --max-depth 4 \
  --require-native --output "$FLUXR_SEMANTIC_WORK/campaign.json"
```

Use `--no-fixtures` only when intentionally selecting generated cases alone;
both `generate` and `run` include the dedicated fixed corpus by default. Omit
`--require-native` on a portable-only bridge. Results retain the selection and observed routes so that
the narrower run stays visible. The `--timeout` budget covers the driver work
across all batches in an operation; `run`, `replay` and `mutations` default to
30 seconds and accept at most 300 seconds. Corpus admission and oracle work
have separate finite node/call/byte limits, rather than a real-time guarantee.

Generated corpora retain the seed, generator version `splitmix64-scalar-v1`,
generation bounds, frozen-fixture digest, full typed AST, ordered calls, exact
scalar values, limits, emitted source and source digest. Every corpus binds
`model_version: "bounded-scalar-oracle-v1"`. Fixed/API-constructed corpora may
have null generator provenance rather than claiming a seeded origin.
Execution reports also bind the Python campaign bytes as `tool_sha256`, identify
the bridge executable by digest, and record its build/target metadata and
actual routes. These identifiers support
reproduction and integrity checks; they are not producer authentication or
semantic proofs. Source generation, compilation/preparation and execution
remain distinct steps. A generated corpus alone is not an executed test.

Oracle witnesses retain at most 256 detailed events per call and 32,768 events
across a report. Each call still retains its complete outcome, `first_trap` and
`event_count`, with `trace_truncated` explicitly marking omitted detail. Those
limits bound diagnostic size without replacing an omitted trace with a claim
of complete observation.

## Replay retained cases

Both a standalone corpus and a campaign report are replay inputs:

```sh
python3 -B tools/semantic_campaign.py replay \
  --driver "$CARGO_TARGET_DIR/release/examples/semantic_campaign_driver" \
  --input "$FLUXR_SEMANTIC_WORK/corpus.json" \
  --require-native --output "$FLUXR_SEMANTIC_WORK/replay.json"
```

Replay validates the complete canonical corpus, supported model version and
source/provenance integrity, then computes fresh expectations and executes the
supplied bridge. Non-null generator metadata has an exact field set, supported
version, bounded seed/program/call/depth values and a valid optional fixture
digest; malformed or unknown metadata refuses.
Stored outcomes cannot replace fresh oracle evaluation. Retained ASTs and calls
permit replay without relying on the current generator to recreate the old
program. The new report identifies the bridge actually used, allowing a case
to be checked after an implementation change without misattributing old
execution to the new executable.

Malformed, oversized, unsupported, inconsistent or tampered replay input must
refuse rather than become an empty passing campaign. The standard regression
suite includes deterministic generation, frozen results and boundary/trap
expectations; replay-specific checks also exercise integrity refusal.

## Reduce a discrepancy

Select a failing case and zero-based call index from an execution report.
Reduction targets runtime-call discrepancies; compilation/preparation failures
remain report failures but do not provide a reducible call observation.

```sh
python3 -B tools/semantic_campaign.py reduce \
  --driver "$CARGO_TARGET_DIR/release/examples/semantic_campaign_driver" \
  --input "$FLUXR_SEMANTIC_WORK/campaign.json" \
  --case CASE_ID --call 0 --max-attempts 64 --timeout 60 \
  --output "$FLUXR_SEMANTIC_WORK/reduced.json"
```

The reducer first reproduces the selected discrepancy. It retains the entire
ordered invocation prefix throughout the search so that a failure depending on
workspace reuse is not silently converted into an isolated-call result. Prefix
minimization is outside this reducer's scope. Candidate reductions must be
valid, strictly smaller and retain the selected backend, mismatch class, exact
trap texts and scalar result tags. Arguments and program expressions may
shrink while that signature is preserved. Expectations are recomputed for every
candidate; stored expected values are never patched to fit production behavior.

The attempt bound counts candidate executions and excludes the single initial
reproduction: at most `1 + max_attempts` case evaluations run. The default is
64 candidate attempts and 60 seconds; accepted maxima are 1,024 attempts and
300 seconds. The output records the retained case, attempt count and whether
the search ended at its budget. An incomplete search remains explicit, and the
retained candidate must still reproduce the selected class. A greedy bounded
result is not a globally minimal counterexample or a proof that no smaller
program exists. A case that does not reproduce, or a driver/protocol failure,
cannot be reported as a successful reduction.

## Qualify mutation detection

```sh
python3 -B tools/semantic_campaign.py mutations \
  --driver "$CARGO_TARGET_DIR/release/examples/semantic_campaign_driver" \
  --output "$FLUXR_SEMANTIC_WORK/mutations.json"
```

Mutation sentinels execute controlled faulty compact programs against the
original frozen expectations. Each has both a frozen control outcome and a
frozen expected faulty outcome. Both production paths must match the control
and produce the exact represented faulty observation; an unrelated trap,
compiler failure or unavailable bridge cannot masquerade as a detected
semantic defect. They exercise the complete comparison path using the real
portable and prepared APIs, without patching production source or binaries.

| Sentinel | Represented fault at the frozen counterexample |
| --- | --- |
| `add_as_sub` | Addition emitted as subtraction |
| `inclusive_as_strict` | `<=` emitted as `<` at equality |
| `branch_inverted` | The opposite branch selected |
| `eager_and`, `eager_or` | A normally skipped trapping expression evaluated |
| `right_before_left` | Competing operand errors observed in the wrong order |
| `wrapping_add` | A wrapping boundary value returned instead of an overflow trap |
| `floor_division`, `floor_remainder` | Python-style floor results substituted for truncation-toward-zero results |

The wrapping and floor-result witnesses use literal replacements only at their
counterexamples. They are not general mutant implementations of arithmetic.

The observed detection concerns the represented fault effects in these
sentinels. It does not establish detection of every possible compiler defect or
close S25's broader alias, floating-point and recursive-frame mutation gates.
Ordinary harness tests additionally inject bad observations and malformed
protocol responses to verify that comparison, replay and reduction fail
closed. Those protocol tests are distinct from native semantic execution.

## Independence and fixtures

`tests/fixtures/semantic-campaign/cases.json` contains manually pinned typed
programs, calls and exact expected observations. It is maintained as an
input fixture rather than generated from a successful production run. Changes
to a frozen expectation require a semantic explanation and independent review;
a production discrepancy alone is not a reason to rewrite the oracle.

The Python model uses integer/Boolean operations and explicit ordered resource
events. It does not import the Rust parser, compiler IR, VM evaluator or JIT
helpers. The generator, model and emitter share the test AST/data schema and
admission validation; this is an explicit shared assumption. Frozen answer
fixtures test the model independently of random generation, and the Rust
bridge's tests retain literal compact programs with hand-pinned outcomes. The
Rust bridge necessarily shares production compilation
between its portable and prepared paths, which is why those paths are checked
against an external model rather than only against each other.

`tests/test_semantic_campaign.py` covers the model and fixtures;
`tests/test_semantic_campaign_cli.py` covers external outputs, process failures,
replay and reduction. `tests/test_semantic_campaign_native.py` checks a controlled
addition-to-subtraction source fault through the actual compiled bridge,
including a passing control, bounded reduction and replay of the discrepancy.
The request wrapper forwards real observations and computes no expectations.
Configure the bridge to enable the suites' real-driver
run/replay and mutation checks; without it, those checks report explicit skips.
The Rust example's own tests cover bounded protocol admission and real API
observations:

```sh
export FLUXR_SEMANTIC_DRIVER="$CARGO_TARGET_DIR/release/examples/semantic_campaign_driver"
python3 -B tools/run_python_suite.py tests 'test_semantic_campaign*.py'
cargo test --locked --example semantic_campaign_driver --jobs 1
cargo test --release --locked --features typed-jit \
  --example semantic_campaign_driver --jobs 1
```

Run the campaign and mutation commands above as well; unit tests and successful
compilation alone are not a native differential run. Existing cookbook,
compatibility and integration gates remain required for their respective
capabilities. The complete integration commands are in
[the integration guide](integration.md).

The exact-checkout verifier requires all three semantic suite files in its
Python-bearing profiles. An absent native helper is recorded as an explicit
coverage gap; a configured invalid helper fails. The Linux release CI job
builds the real bridge, requires observed native execution, and retains its
campaign and mutation outputs outside the source tree.

On a supported typed-JIT platform, `FLUXR_SEMANTIC_REQUIRE_NATIVE=1` also makes
the real-driver Python run require a successful native observation. The
deliberately faulty synthetic driver used for replay/reduction plumbing never
evaluates FluxR and is not counted as semantic-execution evidence.

## Resource and protocol boundary

The bridge reads one JSON request from standard input and writes one JSON
response to standard output. It owns no files and executes no native source
spans. Driver or protocol errors are distinct from language traps.

| Bridge bound | Maximum |
| --- | ---: |
| Request / response bytes | 4 MiB / 8 MiB |
| Programs / compact source bytes per program | 256 / 16 KiB |
| Calls per program / total calls | 512 / 8,192 |
| Arguments / inputs | 16 scalar lanes per slice |
| Per-call instructions / cells / frames | 1,000,000 / 65,536 / 64 |

The campaign admits its own generator/AST/replay bounds before invoking the
bridge and applies finite process deadlines and bounded output collection:

| Campaign bound | Maximum or range |
| --- | ---: |
| Generator seed | Unsigned 64-bit integer |
| Generated programs / calls per generated program | 1–256 / 1–32 |
| Requested generator expression depth | 0–6 |
| Statement/expression nodes / expression nesting / statement nesting | 256 / 8 / 4 |
| Parameters / locals | 8 each |
| Retained corpus cases / total calls | 512 / 16,384 |
| Replay input / serialized output | 64 MiB each |

Zero runtime limits are available for intentional exhaustion cases. Keep
process results in new external output files so that failure reports and replay
inputs are not accidentally overwritten or included in an implementation ZIP.
The harness is trusted local validation, with the external resource coordinator
for sustained work; it is not an arbitrary-code isolation boundary.
