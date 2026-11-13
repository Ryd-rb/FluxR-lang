# Rust-to-FluxR cookbook

This cookbook contains **41 complete Rust/FluxR pairs** spanning checked scalar,
control-flow, collection, nominal-type, binary32 and resource-safety behavior.
Start with an ordinary Rust implementation, then inspect its `.fluxr` migration.
The `.compact` file is the supported compact-v2 representation used to execute
the identical program against older public versions. It is a compatibility
fixture, not a different algorithm or a second application source tree.

`catalog.json` lists every pair. The Rust integration test and benchmark import
the exact listed Rust file; token accounting measures that same file. Shared
Value adapters, generators, kernel preparation and measurement infrastructure
are kept separately in `support.rs` and counted separately. Rust kernels use
checked integer arithmetic where the FluxR operation can trap. Rust itself does
not acquire the VM's logical fuel, cell or frame limits.

## Run and test

Build with the pinned toolchain and locked dependencies, keeping all targets and
outputs outside this tree. Follow the root integration and resource-safety guide.

```sh
cargo test --locked --test cookbook --jobs 1
cargo test --release --locked --features typed-jit --test cookbook --jobs 1
cargo build --release --locked --features typed-jit --example bench_cookbook --jobs 1
```

The `tests.json` manifest is a `fluxr-tests-v1` suite for the native FluxR test
runner. It contains one hand-authored expectation for each pair and additional
explicit overflow, wrong-tag, bounds, fuel, cell and frame failures. The test
runner's documentation describes filtering, listing and result output.

```sh
"$CARGO_TARGET_DIR/release/fluxr-cli" test-suite examples/cookbook/tests.json \
  --mode compare --format json --test-threads 1
```

`compare` checks the portable and prepared executions against each declared
outcome. Retain its JSON output outside the source tree when recording a run.

For a direct executable example:

```sh
"$CARGO_TARGET_DIR/release/fluxr-cli" run examples/cookbook/gcd.fluxr run   '[{"u64":48},{"u64":18}]'
```

The result value is `{"u64":6}`. Another useful starting point is `list_sum`,
which consumes a local COW copy of its list from the back. Its Rust reference
intentionally iterates in reverse: checked intermediate overflow makes fold
order observable.

## Coverage

| Pair | Topics |
| --- | --- |
| [unit](unit.fluxr) / [Rust](unit.rs) | unit, zero arguments |
| [checked_affine](checked_affine.fluxr) / [Rust](checked_affine.rs) | i64, checked arithmetic, overflow |
| [unsigned_affine](unsigned_affine.fluxr) / [Rust](unsigned_affine.rs) | u64, checked arithmetic, overflow |
| [clamp](clamp.fluxr) / [Rust](clamp.rs) | if, comparisons, early return |
| [guarded_division](guarded_division.fluxr) / [Rust](guarded_division.rs) | bool, short circuit, division, traps |
| [gcd](gcd.fluxr) / [Rust](gcd.rs) | u64, euclidean algorithm, while, rem |
| [factorial](factorial.fluxr) / [Rust](factorial.rs) | u64, while, checked mul |
| [fibonacci](fibonacci.fluxr) / [Rust](fibonacci.rs) | i64, recurrence, locals, overflow |
| [power](power.fluxr) / [Rust](power.rs) | i64, bounded exponent, checked mul |
| [triangular](triangular.fluxr) / [Rust](triangular.rs) | i64, while, compact steps |
| [loop_exits](loop_exits.fluxr) / [Rust](loop_exits.rs) | break, continue, while, control order |
| [helper_calls](helper_calls.fluxr) / [Rust](helper_calls.rs) | private function, calls, checked arithmetic |
| [recursive_sum](recursive_sum.fluxr) / [Rust](recursive_sum.rs) | recursion, frames, base case |
| [list_sum](list_sum.fluxr) / [Rust](list_sum.rs) | list, copy on write, checked fold |
| [list_count](list_count.fluxr) / [Rust](list_count.rs) | list, empty, pop, copy on write |
| [list_reverse](list_reverse.fluxr) / [Rust](list_reverse.rs) | list, allocation, push, pop, ordering |
| [list_contains](list_contains.fluxr) / [Rust](list_contains.rs) | list, search, early return, bool |
| [list_last](list_last.fluxr) / [Rust](list_last.rs) | list, guard, short circuit |
| [list_push_pop](list_push_pop.fluxr) / [Rust](list_push_pop.rs) | list, push, pop, copy on write, snapshot |
| [list_index](list_index.fluxr) / [Rust](list_index.rs) | list, bounds, unsigned index |
| [record_point](record_point.fluxr) / [Rust](record_point.rs) | record, construction, checked add |
| [record_update](record_update.fluxr) / [Rust](record_update.rs) | record, field, copy on write, checked add |
| [nested_record](nested_record.fluxr) / [Rust](nested_record.rs) | record, nesting, projections |
| [method_shift](method_shift.fluxr) / [Rust](method_shift.rs) | record, nominal method, checked add |
| [enum_option](enum_option.fluxr) / [Rust](enum_option.rs) | enum, optional payload, guard |
| [enum_map](enum_map.fluxr) / [Rust](enum_map.rs) | enum, construct payload, checked map |
| [float_affine](float_affine.fluxr) / [Rust](float_affine.rs) | f32, strict binary32, arithmetic, nan |
| [float_sqrt](float_sqrt.fluxr) / [Rust](float_sqrt.rs) | f32, sqrt, canonical nan, signed zero |
| [float_to_int](float_to_int.fluxr) / [Rust](float_to_int.rs) | f32, i64, saturating conversion, nan |
| [int_to_float](int_to_float.fluxr) / [Rust](int_to_float.rs) | i64, f32, direct rounding |
| [uint_to_float](uint_to_float.fluxr) / [Rust](uint_to_float.rs) | u64, f32, direct rounding |
| [float_minimum](float_minimum.fluxr) / [Rust](float_minimum.rs) | f32, list, minidx, tie, nan, signed zero |
| [input_adjust](input_adjust.fluxr) / [Rust](input_adjust.rs) | inputs, snapshots, bool, checked arithmetic |
| [five_arguments](five_arguments.fluxr) / [Rust](five_arguments.rs) | i64, five arguments, generic boundary |
| [uninitialized_order](uninitialized_order.fluxr) / [Rust](uninitialized_order.rs) | trap, uninitialized, first error |
| [tagged_payload](tagged_payload.fluxr) / [Rust](tagged_payload.rs) | enum, exact tag, payload, trap |
| [unsigned_division](unsigned_division.fluxr) / [Rust](unsigned_division.rs) | u64, checked div, division by zero |
| [enum_list](enum_list.fluxr) / [Rust](enum_list.rs) | enum, list, short circuit, payload |
| [nested_list](nested_list.fluxr) / [Rust](nested_list.rs) | list, nesting, last, ownership |
| [float_absolute](float_absolute.fluxr) / [Rust](float_absolute.rs) | f32, abs, raw bits, nan payload |
| [boolean_logic](boolean_logic.fluxr) / [Rust](boolean_logic.rs) | bool, not, and, or, comparison |

## Behavioral obligations

Every pair runs against hand-picked boundary values and deterministic seeded
inputs where applicable. The shared test harness compares the independently
executed Rust result with portable, automatic and prepared FluxR execution.
It repeats this check through current source, current canonical printing,
compact source, compact printing, released FA1 source and artifact decode.
All representations must retain the same canonical artifact bytes.

Tests retain the host input snapshots across calls, check COW updates, exercise
malformed boundary values and compare ordered low-budget failures across
execution routes. Reused workspaces execute an independent successful program
after failures, including for the deliberately always-failing
`uninitialized_order` example. Low-budget route comparison checks preservation;
it does not claim that a Rust kernel independently reproduces VM metering.

Values compare exactly, including binary32 bits. `float_absolute` preserves NaN
payloads while clearing the sign bit; arithmetic and `float_sqrt` follow their
specified canonical-NaN behavior. `float_minimum` preserves the first tie and
first-position NaN. `float_to_int` demonstrates truncating, saturating conversion.

The corpus covers the checked constructs currently provided by the language.
Rust-owned threads, async, networking, databases and other services belong to the
explicit native interoperability examples and their separate integration gates.
The cookbook does not assert that those are checked-language primitives or that
finite examples establish whole-language safety or universal Rust-speed parity.

## Measurement protocol

`bench_cookbook` emits `fluxr-cookbook-performance-v1` JSON to standard output;
redirect it to a new file outside the source tree. It executes every row against
all applicable routes before measuring, then checks every individual sample.
Failures are emitted as failed cases and the process returns a nonzero status.
No failed baseline case is counted as passed.

The measured lanes are:

- `rust_kernel`: the exact imported Rust function with typed input preparation
  outside timing. Opaque inputs, dynamic harness dispatch and a typed result
  enum remain in the timed call. It excludes FluxR Value conversion and VM
  validation/metering.
- `rust_value_adapter`: the same reference plus the shared per-call Value
  input/output adapters. Collection conversion can allocate in this lane.
- `portable`: the checked portable entry with reusable workspace.
- `named`: automatic named export lookup; it has no per-call route attribution.
- `prepared`: the cached prepared entry with actual per-call execution-route
  witnesses, available on the candidate API.

Individual observations exclude post-call result checking, typed Rust-to-Value
conversion and result destruction. The `rust_value_adapter` lane's own per-call
Value conversions remain inside its timing. Batch observations include result
destruction and report **batch means**, not individual-call percentiles. Lanes
rotate their starting order and consume the same row sequence. Batch round `r`
starts at row `r * calls_per_batch`, wrapping at the corpus length; successive
rounds advance through the corpus instead of repeatedly timing its first batch.
The default batch settings visit every row of every current example. Smaller
settings measure only the rows reached by their declared schedule. The retained results contain raw
samples, every empirical integer percentile from p0 through p100, and p99.9,
using nearest rank. Timer, dynamic dispatch and empty-loop calibration are
reported separately and never subtracted.

Compilation, artifact decoding, cold preparation and first invocation use
separate lifecycle distributions. Compilation identifies the consumed source
format: current `fluxr 1` on the candidate; compact-v2 for the public adapter.
These different source-reader timings must not be relabeled as a like-for-like
parser improvement. Process startup and the Rust compiler's build costs remain
separate measurements.

Environment settings are bounded:

| Setting | Default | Accepted range |
| --- | ---: | ---: |
| `FLUXR_COOKBOOK_SAMPLES` | 1,024 | 64–16,384 |
| `FLUXR_COOKBOOK_ROUNDS` | 11 | 3–101 |
| `FLUXR_COOKBOOK_BATCH_CALLS` | 64 | 8–4,096 |
| `FLUXR_COOKBOOK_COMMON_ROUTES` | `0` on candidate; `1` on public adapter | `0` or `1`; public adapter requires `1` |

Warmup uses 128 calls per lane; lifecycle uses seven observations per case. Small
sample counts cannot establish population tail bounds. Keep matched compiler
settings, source/input hashes, hardware and execution limits, and compare all
percentiles without dropping regressions. The `fluxr_public_baseline` cfg adapts
only the probe to the older public API: no prepared API or native route is
invented when that baseline does not expose one.

Use `FLUXR_COOKBOOK_COMMON_ROUTES=1` on both arms of a public comparison so the
same four timed lanes follow the same rotation schedule. Use the default five
lanes for comparisons between incoming and candidate packs that both expose the
prepared API. Do not compare common-route and five-lane runs as matched results.
