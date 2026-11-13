# Testing checked and native FluxR programs

`fluxr-cli test-suite` executes checked source from a bounded JSON catalog. Native
Rust items continue to use Cargo's test harness through `fluxr-cli build
--test`. The two routes share the existing checked runtime; neither changes
the language grammar, artifact encoding or JIT implementation. The released
`fluxr-cli test SOURCE REPLAY` command keeps its rule/timeline replay behavior;
checked catalogs use the distinct `test-suite` command.

## Run the cookbook

Build the CLI with locked dependencies and an external target directory, as in
the [integration guide](integration.md). Then run:

```sh
"$CARGO_TARGET_DIR/release/fluxr-cli" test-suite examples/cookbook/tests.json
"$CARGO_TARGET_DIR/release/fluxr-cli" test-suite examples/cookbook/tests.json \
  --mode compare --test-threads 1 --format json
"$CARGO_TARGET_DIR/release/fluxr-cli" test-suite examples/cookbook/tests.json \
  checked_affine::example --exact
"$CARGO_TARGET_DIR/release/fluxr-cli" test-suite examples/cookbook/tests.json --list
```

The default mode uses the prepared-call API, which may choose a native route or
portable fallback. A prepared handle does not prove native execution. Compare
mode executes both portable and prepared calls and compares exact values or
error text. Reports identify the actual selected routes.

## Catalog and assertions

The catalog has `format: "fluxr-tests-v1"` and a `tests` array. Each test has a
unique `name`, a catalog-relative `source`, an `export`, and `expect`. Optional
`arguments` and `inputs` arrays default to empty. Values use the existing
runtime JSON encoding: `"unit"`, `{"bool": true}`, `{"i64": 7}`, `{"u64": 7}`,
or `{"f32": 1065353216}` for the exact binary32 bits of 1.0. Records, lists and
variants retain their existing ordered Value encoding; see the
[runtime guide](unified-runtime-v1.md).

For example, this retained cookbook case calls the complete
`checked_affine.fluxr` program beside its catalog:

```json
{
  "format": "fluxr-tests-v1",
  "tests": [{
    "name": "checked_affine::example",
    "source": "checked_affine.fluxr",
    "export": "run",
    "arguments": [{"i64": 7}, {"i64": 3}, {"i64": 2}],
    "expect": {"value": {"i64": 23}}
  }]
}
```

Exactly one expectation is required:

| Expectation | Passing condition |
| --- | --- |
| `{"value": VALUE}` | The checked call returns exactly that Value, including floating-point bits. A Boolean assertion export uses `{"value":{"bool":true}}`. |
| `{"error": "substring"}` | The selected export exists, compiles, and returns a runtime error containing the nonempty substring. |
| `{"compile_error": "substring"}` | Parsing or checked compilation fails with the nonempty substring. `export` may be omitted. |

Source I/O errors, missing exports, unsupported native spans and harness errors
cannot satisfy an expected runtime/compilation error. A successful call where
an error was expected fails. Use a specific diagnostic substring; broad text
cannot establish which invariant failed. Regressions that require exact trap
order should also compare exact errors through the Rust API, as the cookbook
does.

Tests may set `limits` with all three existing fields, `instructions`, `cells`
and `frames`. Defaults are 50,000 instructions, 16,384 cells and 64 frames.
Zero is valid for intentional exhaustion tests. A test may set a nonempty
`ignore` reason. Ignored cases remain visible and never count as passed.

## Selection, repetition and parallel execution

| Option | Meaning |
| --- | --- |
| Positional `FILTER` | Select names containing this string. |
| `--exact` | Match the full name; also applies to skip filters. |
| `--skip FILTER` | Exclude matching names; may be repeated. |
| `--list` | List selected cases without compilation or execution. All catalog/source admission checks still run. |
| `--ignored` | Select and run only explicitly ignored tests. |
| `--include-ignored` | Run ordinary and ignored tests; incompatible with `--ignored`. |
| `--fail-fast` | Stop scheduling after an observed failure. Already-started parallel work completes. |
| `--repeat N` | Execute runtime cases 1–100 times. Compilation is cached once per selected source. |
| `--test-threads N` | Use 1–8 workers; default 1. Each test owns its workspaces. |
| `--mode portable`, `prepared`, `compare` | Select the execution comparison described above. |
| `--format text`, `json` | Human-readable output or `fluxr-test-results-v1` JSON on stdout. |
| `--allow-empty` | Explicitly allow an empty selection; otherwise it is an error. |

Reports follow catalog order even when execution is parallel. Parallel
fail-fast does not guarantee which other cases have already started. Reports
separate passed, failed, ignored, filtered and not-run counts. Listing is not a
passing test run. Failure or harness error returns a nonzero process status.
There are no automatic success-producing retries.

## Deterministic property tests

An optional `property` object declares a `seed` (u64), `cases` (1–1024),
`generators` and optional `shrink` Boolean. Property exports must return Boolean
and expect `true`. A property replaces fixed arguments; fixed host snapshot
inputs remain available. Generators must match the export's parameter types,
including the declared element type of an empty list. The supported generators
are:

| Generator | Domain |
| --- | --- |
| `{"kind":"bool"}` | Both Boolean values |
| `{"kind":"i64","min":-100,"max":100}` | Inclusive signed range |
| `{"kind":"u64","min":0,"max":100}` | Inclusive unsigned range |
| `{"kind":"bool_list","min_len":0,"max_len":16}` | Flat Boolean lists with inclusive length bounds |
| `{"kind":"i64_list","min_len":0,"max_len":16,"min":-100,"max":100}` | Flat signed lists with inclusive length and element bounds |
| `{"kind":"u64_list","min_len":0,"max_len":16,"min":0,"max":100}` | Flat unsigned lists with inclusive length and element bounds |

List lengths must satisfy `0 <= min_len <= max_len <= 4096`; zero-length and
fixed-length domains are valid. Integer element ranges use the same inclusive
domains as the scalar generators, including the full i64/u64 ranges. Lists are
homogeneous and flat; nested-list or arbitrary composite generators are not
supported. The catalog format remains `fluxr-tests-v1`.

SplitMix64 gives deterministic input generation from the catalog seed. Existing
scalar-only catalogs retain their seed sequences: every scalar argument consumes
one draw. Each list consumes one length draw, even for a fixed or empty length
domain, followed by one draw per element in list order. Arguments are generated
in catalog order; each repetition restarts the seed. The failing report retains
seed, case index, repetition, original argument values, reduced argument values
and shrink evaluation count. Replaying the same catalog reproduces these inputs
independently of worker count. Copy either reported argument vector into a fixed
case to retain an exact regression independent of future generator changes.

Shrinking visits arguments in declaration order and uses at most **128 candidate
evaluations in total** for a failing case. For each list, it first tries deleting
contiguous chunks, starting with the largest deletion allowed by `min_len` and
trying offsets from left to right. After an unsuccessful pass it halves the
chunk size; after an accepted deletion it restarts with the largest allowed
chunk. It then visits retained elements from left to right, using the existing
scalar reductions: `true` toward `false`, signed integers toward zero clamped
to their declared range, and unsigned integers toward their minimum, with
midpoint attempts for integers. Scalar arguments retain the same reductions.

A candidate is accepted only when it preserves the original failure signature.
Runtime-error failures retain exact error text, so a different trap cannot
replace them; backend disagreements retain the existing mismatch category.
Successful candidates are always rejected. Every accepted value stays within its
declared length and element domains. The report's `shrink_evaluations` counts
the attempts actually executed, and an exhausted budget retains the last
accepted counterexample. Compare mode reserves both backend calls for every
attempt. Shrinking is a bounded reduction, not a proof of a globally minimal
counterexample. It does not implement arbitrary recursive strategies, external
state machines or an entire third-party Rust property-testing ecosystem.
Hand-picked edge cases and independent Rust references remain necessary, and
broader I19 acceptance remains open.

## Admission and safety boundaries

Catalogs reject unknown fields and duplicate JSON keys at every nesting level.
Source paths must be safe relative paths, with no traversal or symlinks below
the catalog directory. Files must be regular files. Sources are read once and
retained for the run. This is trusted local testing, not a concurrently mutated
filesystem or hostile-code sandbox.

| Bound | Maximum |
| --- | ---: |
| Catalog bytes | 8 MiB |
| Individual source / total distinct sources | 8 MiB / 32 MiB |
| Tests | 4,096 |
| Arguments / input values | 64 each |
| Generated list length | 4,096 |
| Generated argument cells per property case | 262,144 |
| Nested test-value depth / visited cells | 32 / 262,144 |
| Per-call instructions / cells / frames | 1,000,000 / 262,144 / 256 |
| Selected invocation reservation | 65,536 backend calls |
| Generated argument cells reserved across the selected run | 4,194,304 |
| Selected instruction reservation | 1,000,000,000 |

Generated argument admission reserves the declared maximum sizes before any
generation: each scalar costs one cell and each list costs `1 + max_len` cells.
Their sum must not exceed 262,144, even if a particular seed would produce
shorter lists. This reservation is separate from the existing fixed-input and
fixed/expected-value checks, and from the per-call VM cell limit. A catalog can
therefore intentionally exercise runtime cell exhaustion without bypassing the
generator's allocation cap. Catalog validation covers every case, including
those later ignored or filtered.

Execution also reserves generated argument cell work across the selected run.
For each selected, nonignored property, multiply its maximum generated argument
cells by `(cases + shrink_allowance) * repeat * backend_count`, where
`shrink_allowance` is 128 when shrinking is enabled and zero otherwise, and
`backend_count` is two in compare mode and one otherwise. The sum must not exceed
4,194,304; fixed cases contribute zero to this additional reservation. This
conservative bound limits cumulative list work and retained property
counterexamples. Its maximum equals the existing 65,536-call limit times the
64-scalar-argument limit, so it does not newly reject scalar-only catalogs that
satisfy the existing reservations.

Invocation and instruction reservations include repeats, both comparison
backends and the maximum possible shrink work. The existing compiler limits
still apply. Checked catalogs neither execute Rust spans nor grant host services.
Use the application/native route
for those integrations. VM fuel cannot interrupt trusted native I/O. For long
qualification runs, apply the external [resource coordinator](resource-safety-v1.md).

The Rust library exposes `fluxr::testing::load`, `run`, immutable suite catalog
inspection, options and structured reports. The CLI adds no shell execution.
The framework's regression suite is `cargo test --locked --test testing`;
the cookbook's independent Rust oracles are in `tests/cookbook.rs`.

## Independent semantic campaigns

`tools/semantic_campaign.py` provides a separate bounded differential campaign
for generated signed-integer/Boolean programs, assignments and branches.
Expected results and exact first traps come from its independent model and
hand-pinned fixtures, rather than agreement between two production routes.
The test-only `semantic_campaign_driver` example reports portable and prepared
observations through the existing Rust API, including the actual prepared
execution route. The campaign uses existing compiler and runtime interfaces.

The campaign includes explicit `generate`, `run`, `replay`, `reduce` and
`mutations` commands. Its case identity binds the program, ordered calls,
limits, source and generator provenance; replay validates retained content
before recomputing expectations. Reduction is bounded and retains the selected
backend and discrepancy class. Mutation checks require passing controls and
the exact frozen faulty outcomes on both routes. See [the campaign guide](semantic-campaign.md) for
commands, limits and exclusions. Generated reports and replay files belong
outside the source tree; committed frozen fixtures remain test inputs.

## Native Cargo tests

Append ordinary libtest arguments after the literal `--` separator:

```sh
"$CARGO_TARGET_DIR/release/fluxr-cli" build project/fluxr.json \
  "$FLUXR_WORK/native-project" --target-dir "$FLUXR_WORK/native-target" \
  --logs "$FLUXR_WORK/native-build" --test -- \
  checked_runtime_isolated --exact --test-threads 1 --nocapture
```

The frontend preserves argument boundaries and forwards the suffix only to the
final `cargo test` invocation. It rejects a suffix without `--test` and rejects
execution arguments with `--no-run`. Native tests retain Rust assertions,
`Result` returns, `#[should_panic]`, ignored tests, output controls, filtering
and test threads. The generated runtime-admission test is a separate test;
tests needing a runtime must initialize their own fixture and must not rely on
another test's execution order.

`tools/unified_tests/run_native_tests.py` builds a real emitted application test
fixture and verifies these behaviors, including an intentional failing test's
nonzero status. Its outputs are external. The builder, locks, runtime embedding
and standalone-export contracts remain in force.

## Design references and scope

The selection/ignore/output conventions follow the official
[rustc test harness documentation](https://doc.rust-lang.org/rustc/tests/index.html).
Deterministic generated cases and reduction are informed by the
[Proptest introduction](https://proptest-rs.github.io/proptest/intro.html).
Explicit not-run results and conservative parallel fail-fast follow the
distinctions described by [cargo-nextest](https://nexte.st/docs/running/).
The implementation uses existing FluxR/standard-library dependencies; it does
not embed or claim full parity with those independent frameworks. Native
process isolation, async test adapters, doctest extraction, arbitrary strategy
composition and third-party reporter formats are separate capabilities.
