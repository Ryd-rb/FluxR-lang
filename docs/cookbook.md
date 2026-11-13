# Executable Rust-to-FluxR cookbook

The [cookbook directory](../examples/cookbook/README.md) contains 41 complete
program pairs. Each starts with an ordinary Rust reference and migrates that
algorithm to current `fluxr 1` source. A retained compact-v2 representation
allows the same checked program to execute on the fixed public baseline.
These are independent, small programs: extract the relevant example rather
than importing a generated application or an additional runtime copy.

## What to read

| Topic | Starting examples |
| --- | --- |
| Basic values and arithmetic | `unit`, `checked_affine`, `unsigned_affine`, `boolean_logic` |
| Branches and short-circuiting | `clamp`, `guarded_division`, `loop_exits` |
| Numerical algorithms | `gcd`, `factorial`, `fibonacci`, `power`, `triangular` |
| Calls, recursion and argument layouts | `helper_calls`, `recursive_sum`, `five_arguments` |
| Lists and ownership | `list_sum`, `list_count`, `list_reverse`, `list_contains`, `list_last`, `list_push_pop`, `list_index`, `nested_list` |
| Nominal records and methods | `record_point`, `record_update`, `nested_record`, `method_shift` |
| Enumerations and payloads | `enum_option`, `enum_map`, `tagged_payload`, `enum_list` |
| Binary32 values and conversions | `float_affine`, `float_sqrt`, `float_to_int`, `int_to_float`, `uint_to_float`, `float_minimum`, `float_absolute` |
| Inputs and explicit failures | `input_adjust`, `uninitialized_order`, `unsigned_division`, plus resource-limited catalog cases |

The directory's catalog maps exact `.rs`, `.fluxr` and `.compact` filenames.
The Rust tests and benchmark import those same Rust files; token accounting
reads their complete bytes. Support adapters and test context remain separate.
This prevents a short counted reference from silently standing in for a
different executed implementation.

## Execute and validate

Use the pinned compiler, locked dependencies and external build directories
from the [integration guide](integration.md):

```sh
cargo test --locked --test cookbook --jobs 1
cargo test --release --locked --features typed-jit,host-transport \
  --test cookbook --jobs 1
"$CARGO_TARGET_DIR/release/fluxr-cli" test-suite examples/cookbook/tests.json \
  --mode compare --test-threads 1
"$CARGO_TARGET_DIR/release/fluxr-cli" run examples/cookbook/gcd.fluxr run \
  '[{"u64":48},{"u64":18}]'
```

The GCD result is `{"u64":6}`. The catalog supplies complete normal and
negative calls for the [checked test framework](testing.md). Its hand-authored
successful expectations are also checked against the Rust reference.

For each pair the Rust integration suite compares the independent reference
against portable, automatic and prepared execution, using edge values and
deterministic generated inputs. It executes current source, canonical current
printing, retained compact source, printed compact source, FA1 projection and
decoded artifacts. All representations must preserve canonical artifact bytes.
The suite also checks low instruction/cell/frame limits, malformed host values,
COW input preservation and successful workspace reuse after failures.

Integer references use checked arithmetic where the translated operation can
trap. List folds follow the exact same order. Floating comparisons use Value
bits, so NaN payload and signed-zero differences cannot disappear behind an
approximate assertion. VM budget comparisons use the portable evaluator as the
preservation reference; the Rust algorithm does not independently implement VM
metering. This is finite executable coverage, not a whole-language proof.

## Measure tokens

```sh
python3 -B tools/bench_cookbook_tokens.py \
  --output "$FLUXR_WORK/cookbook-tokens.json"
```

The counter requires the declared `tiktoken==0.14.0` and uses `o200k_base` and
`cl100k_base`. It counts each complete UTF-8 file once, including its final
newline, with no per-word sums or normalization. Reports retain source hashes,
tokenizer rank/regex identities, per-case and aggregate counts, JSON-wire costs,
current FluxR/Rust ratios and current FluxR/public-compatible-compact ratios.
Shared test adapters, manifests, framework and protocol/context are separate.

The current-versus-compact comparison measures equivalent source
representations. It does not establish a token reduction across implementation
revisions. The full project token goal of at most 0.10 of Rust, and live-agent
edit/retry efficiency, require their separate acceptance work.

## Measure runtime distributions

Build `bench_cookbook` in release mode with the required feature set. The
reusable `tools/cookbook_performance.py` collector executes a prebuilt benchmark
with bounded output/time and external results, and checks source, binary,
compiler, configuration and hardware identities. Its `capture --help` and
`compare --help` show the full command interface.

The [detailed measurement protocol](../examples/cookbook/README.md#measurement-protocol)
defines the exact lanes, observation units, settings and compatibility adapter.
It separates typed Rust kernels, Rust Value adapters, portable calls, named
calls and prepared calls. Individual-call and batch-mean distributions are
distinct. Every distribution retains raw samples, p0–p100 and p99.9 using
nearest rank. Compilation, artifact decoding, preparation, first call and
process startup are separately identified.

Successive batches advance through the input rows, wrapping at the corpus
length; each lane receives the same schedule. Default settings cover every
current row in both the individual and batch observations. The Rust Value
adapter's per-call conversions are timed; only conversion after the measured
call, result checking and result destruction are excluded from individual timing.

Only common, semantically matched execution lanes can be compared across
revisions. The public adapter has no fabricated prepared-call API or native
route attribution. Candidate current-source compilation and public compact
compilation use different readers and must not be presented as a parser-speed
comparison. Rust kernel measurements exclude VM Value validation and logical
metering; their boundary is explicit. Timer/dispatch calibration is reported
without subtraction. All measured regressions remain in comparisons.

When switching source snapshots with the same crate name, use separate external
targets or clear only the task-owned `fluxr` package artifacts after its writers
have stopped. Cargo's unhashed cdylib/rlib output can otherwise collide across
checkouts even when dependency artifacts remain reusable. Preserve each built
benchmark binary outside the target before switching, and bind captures to the
actual source and binary identities.

Single-process empirical observations describe the executed finite corpus.
They do not establish statistically qualified population tails or improvement
at every percentile. Independent repeated builds, paired process runs and
same-binary controls are required for small-change claims. The cookbook and
measurement harness provide examples, testing and measurement coverage; their
finite results do not qualify optimizer changes outside the measured cases.

## Native and wider platform topics

Checked source currently has no string/map/async/thread or ambient filesystem
primitive. Those topics use explicit Rust interoperability and typed native
services, as shown by the [unified runtime](unified-runtime-v1.md),
[native integration](native-rust-v1.md) and
[multicore example](../examples/multicore-server/README.md). The native-test
qualification driver builds and executes real Rust/libtest cases through that
same application builder. It does not turn native callbacks into fuel-bounded
checked code or establish a new sandbox.

Windows, ARM64, Wasm, physical GPUs, live PostgreSQL and audio-device execution
retain their separate integration gates. A portable cookbook pass cannot stand
in for any of those capabilities or complete the broad I19 product task.
