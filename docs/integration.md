# Integration guide

This directory is a complete Cargo project. Extract it, or review its files on a
branch of the destination repository. Pack **25** implements **FluxR language 1**;
the crate version and released codec identifiers have their own versioning.

This pack combines checked scalar `u64` native execution, bounded flat-list
property generators and deterministic shrinking, independent semantic campaigns,
deterministic source packaging, and current-language TextMate highlighting.
It retains exact JIT region metering, arithmetic-update notation, empirical
performance distributions, the executable cookbook and checked/native testing
interfaces. These implementation slices leave the broader runtime, testing,
editor and release acceptance contracts open. Its fixed public comparison is
`ryd-rb/fluxr-lang`, default branch `main`, commit
`72e27cacd1df7286b8a6490c4d31f943c660d833`. No private-repository synchronization
or remote publication is part of this source delivery.

The current implementation uses one `fluxr 1` source format, one
`fluxr-project-v1` project manifest, and the mandatory Rust-implemented runtime.
The [runtime guide](unified-runtime-v1.md) describes the public API and executable
behavior. [Current status](current-status.md) separates implemented capabilities
from qualification that remains open.

## Contents and ownership

| Path | Integration role |
| --- | --- |
| `src/`, `crates/fluxr-entry/` | Runtime, compiler, CLI and executable entry macro |
| `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml` | Cargo configuration, locked dependencies and Rust toolchain |
| `tests/`, `tools/*tests/`, `tools/test_*.py` | Behavior tests and reproducible qualification tools |
| `tools/` | Canonical frontend, compatibility imports, agent tooling and measurement tools |
| `examples/`, `benchmarks/`, `web/`, `syntaxes/` | Examples, comparison inputs and supported integrations |
| `docs/` | Current interfaces, implementation contracts and acceptance requirements |
| `third_party/multicore-reference/` | Exact pinned source and conformance harness used by the challenge tools |
| `LICENSE`, nested notices and licenses | Redistribution terms for the implementation and retained inputs |
| `.github/workflows/ci.yml` | Portable, release-feature and all-feature compilation gates |
| `tools/release_pack.py`, `docs/release-pack.md` | Deterministic source packaging, validation and exact extraction |

Keep the entire Cargo source layout together: application builds and standalone
exports bundle `src/`, `crates/fluxr-entry/`, both Cargo files and the license.
Keep the frozen vocabulary and its notice under `tools/fr1/`. The conformance
tool checks the exact inventory of `third_party/multicore-reference/` against
`benchmarks/multicore-reference.json`; removing or adding a file in that pinned
directory invalidates the fixture. Store local outputs elsewhere.

## Deterministic source packaging

The [packaging guide](release-pack.md) defines the clean payload policy, safe
unique paths, canonical file modes, source digests, fixed ZIP metadata, admission
limits and exact extraction. The dedicated packaging gate uses only the Python
standard library:

```sh
python3 -B tools/run_python_suite.py tests test_release_pack.py
python3 -B tools/release_pack.py create . /absolute/new/fluxr-v25.zip
python3 -B tools/release_pack.py validate /absolute/new/fluxr-v25.zip \
  --source . --pack-version 25
python3 -B tools/release_pack.py extract /absolute/new/fluxr-v25.zip \
  /absolute/new/extracted --source . --pack-version 25
```

Create the output parent outside the source tree first. Both archive and
extraction paths must be new. The archive is published only after repeated-byte
equality and validation succeed. Extraction verifies filesystem modes on POSIX;
other platforms explicitly report ZIP metadata only. The `release-pack` suite is included in the exact-checkout
verifier's `python`, `debug` and `full` profiles. Preserve licenses, dependency
locks and pinned functional fixtures. Generated outputs, recovery material and
dispatch prompts do not belong in the integration archive. Packaging success
does not satisfy the Rust, native-helper, platform or product acceptance gates.

## Integrate into an existing repository

Create a review branch from the destination's intended base, compare this source
tree with the destination, and apply the reviewed additions, replacements and
removals. Preserve destination-specific configuration and unrelated work. Review
the Cargo files, CI configuration and external dependency requirements together.
Do not treat a directory overlay as a complete merge: files removed from the
desired source tree must also be reconciled in the destination.

After the import, review `git status --short`, `git diff --stat` and
`git diff --check`. Commit the candidate before using `tools/verify_checkout.py`;
that verifier intentionally requires a clean Git checkout and an output directory
outside it. The extracted archive itself needs no Git repository for ordinary
builds, direct tests or source exports.

## Toolchain and first build

Use Rust **1.99.1**, as pinned by `rust-toolchain.toml` and CI, and Python **3.11
or newer**; CI uses Python 3.12. The frontend uses standard-library `tomllib`.
Git is needed for exact-checkout verification. Cargo dependencies must be
available from the registry or a local cache. Optional platform qualification
has additional prerequisites described by its own guide.

Install `requirements-dev.txt` into a Python virtual environment for the full
Python test suite and token-accounting tools. It pins `tiktoken==0.14.0` and
`tokenizers==0.23.3`, matching the measurement protocols. The tokenizer tests
also need the official `o200k_base` data, either already cached or retrievable
on first use. The six-tokenizer measurement panel has its own exact asset pins
in [wordcode token accounting](wordcode-token-accounting.md).

```sh
python3 -m pip install -r requirements-dev.txt
```

Start with one Cargo build job and one expensive process tree. Increase build
concurrency only after resource measurements show adequate headroom.
The project builder and native qualification drivers default to one build job.
The following POSIX commands run from the extracted project root. The work
directory is deliberately outside the source tree. Use fresh output directories
for each qualification attempt.

```sh
FLUXR_WORK="$(mktemp -d "${TMPDIR:-/tmp}/fluxr-integration.XXXXXX")"
export CARGO_TARGET_DIR="$FLUXR_WORK/cargo"

cargo build --release --locked --bin fluxr-cli --jobs 1
"$CARGO_TARGET_DIR/release/fluxr-cli" build \
  examples/unified/fluxr.json "$FLUXR_WORK/hello-project" \
  --target-dir "$FLUXR_WORK/hello-target" --logs "$FLUXR_WORK/hello-build"
"$FLUXR_WORK/hello-target/release/fluxr-hello"
```

The hello executable should print `{"i64":42}`. Windows binaries use the `.exe`
suffix; use equivalent PowerShell paths and temporary-directory creation there.
Do not place a Cargo target inside an emitted project. For concurrent or lengthy
build and benchmark work, apply the resource admission rules in
[agent operations](agent-operations.md) and
[resource safety](resource-safety-v1.md).

## Required behavior checks

On Linux, prepare the real typed-JIT wordcode worker before running the complete
Python suites. `ActualWorkerTests` otherwise skip the process-promotion gate;
discovery or a portable-only worker cannot establish native execution.
Process-tree, resource-guard, frontend/export shell and challenge tooling have
Linux-specific qualification. The exact-checkout verifier records those
platform boundaries explicitly on other operating systems.

```sh
cargo build --release --locked --features typed-jit \
  --example wordcode_worker --example semantic_campaign_driver --jobs 1
export FLUXR_WORDCODE_WORKER="$CARGO_TARGET_DIR/release/examples/wordcode_worker"
export FLUXR_SEMANTIC_DRIVER="$CARGO_TARGET_DIR/release/examples/semantic_campaign_driver"
python3 -B tools/run_python_suite.py tests 'test_*.py'
python3 -B tools/run_python_suite.py tools 'test_*.py'
python3 -B tools/run_python_suite.py tools/eval/tests 'test_*.py'
python3 -B tools/check_requirements.py
python3 -B tools/native_rust_tests/audit_importer.py \
  --reference third_party/multicore-reference \
  --output "$FLUXR_WORK/importer-audit"
```

The configured semantic bridge also enables the Python suite's real-driver
run/replay and mutation checks. On a supported typed-JIT platform, set
`FLUXR_SEMANTIC_REQUIRE_NATIVE=1` before that suite to require an observed native
success. An absent bridge setting produces explicit skips and cannot qualify
that execution gate.

Run the semantic campaign's independent comparison and mutation checks using
the bridge built above. These outputs must be new paths outside the
source tree. `--require-native` requires an observed native route on a supported
typed-JIT platform; a prepared handle alone is insufficient.

```sh
python3 -B tools/semantic_campaign.py run \
  --driver "$CARGO_TARGET_DIR/release/examples/semantic_campaign_driver" \
  --seed 243 --programs 64 --calls-per-program 8 --max-depth 4 \
  --require-native --output "$FLUXR_WORK/semantic-campaign.json"
python3 -B tools/semantic_campaign.py mutations \
  --driver "$CARGO_TARGET_DIR/release/examples/semantic_campaign_driver" \
  --output "$FLUXR_WORK/semantic-mutations.json"
```

See [semantic campaigns](semantic-campaign.md) for portable-only execution,
exact first-trap observations, frozen fixtures, replay, reduction and the
coverage boundary. These finite checks leave the full S25 model and
release-wide coverage requirements open.

Run the native gates before approving the integrated candidate:

```sh
cargo test --locked --all-targets --jobs 1
cargo test --release --locked --all-targets \
  --features typed-jit,host-transport --jobs 1
cargo check --locked --all-targets --all-features --jobs 1

python3 -B tools/qualify_unified_cli.py \
  --binary "$CARGO_TARGET_DIR/release/fluxr-cli" \
  --output "$FLUXR_WORK/cli-checks"
```

The executable integration gate below also requires PostgreSQL 18. Set
`PG_BIN` to the absolute directory containing its `initdb`, `postgres`,
`pg_isready` and `psql` binaries, and `PG_SHARE` to the matching directory
containing `postgres.bki` and the configuration templates. Use the environment
provided by your installation or local package extraction. For locally
extracted Linux packages, also export the required library directories through
`LD_LIBRARY_PATH`, preserving any existing entries. For example, when
`FLUXR_PG_LIBRARY_PATH` contains those directories:

```sh
: "${FLUXR_PG_LIBRARY_PATH:?Set the extracted PostgreSQL library directories}"
export LD_LIBRARY_PATH="$FLUXR_PG_LIBRARY_PATH${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
```

Keep an already configured loader environment unchanged. Run the gate as a
non-root user permitted to create a private PostgreSQL cluster and bind loopback
TCP and filesystem Unix sockets. The gate starts and stops its own database;
it does not use an existing PostgreSQL service.

```sh
: "${PG_BIN:?Set PG_BIN to the PostgreSQL 18 binary directory}"
: "${PG_SHARE:?Set PG_SHARE to the matching PostgreSQL 18 share directory}"
export PG_BIN PG_SHARE
"$PG_BIN/postgres" --version
python3 -B tools/unified_tests/run_integration.py \
  --cli "$CARGO_TARGET_DIR/release/fluxr-cli" \
  --work "$FLUXR_WORK/integration-work" \
  --output "$FLUXR_WORK/integration-results" \
  --target-dir "$FLUXR_WORK/integration-target"
```

Native feature support is platform-specific. The all-features command is a
compilation check, and a successful compile does not establish execution on
every backend. The repository CI runs portable and release-feature checks on
Linux, Windows and macOS, with the all-features compilation check on Linux.

The all-targets gates include signed/tagged-scalar/generic JIT ownership, strict
scan, compact updates, signed steps, finite decimal float literals,
exact straight-line JIT region metering, arithmetic-update shorthand, and
fragment/source-oracle regressions. Dedicated unsigned checks use independent
Rust edge oracles, per-call typed-JIT/AOT witnesses, generated AOT fixtures and
exact first-trap/resource comparisons:

```sh
cargo test --locked --test jit_unsigned --jobs 1
cargo test --release --locked --features typed-jit --test jit_unsigned --jobs 1
```

The first command executes the compiled AOT fixtures and portable controls; the
second also requires successful typed-JIT routes on supported targets. Existing
canonical artifacts and AOT binding identities retain their contracts. Passing
these scalar tests does not close strict-f32, collection or full R10 acceptance.
The dedicated release probe retains the existing fragment catalogs:

```sh
cargo run --release --locked --features typed-jit --example bench_unsigned_jit -- \
  --iterations 100000 --samples 5
```

It measures the six successful unsigned operations against independent checked
Rust controls using identical full-width input pairs. It separates source
compilation, preparation, pure Rust kernels, Rust host admission and prepared
JIT calls, and requires a native witness for every JIT invocation. Capture its
JSON stdout outside the source tree. Raw batch samples do not establish a speed
claim, population tails or broader R10 acceptance. Follow
[fragment measurement](performance-fragments.md)
for bounded distribution capture and comparison with an independently retained
baseline. Run `tools/bench_update_tokens.py --output /absolute/new/tokens.json`
for the original update pairs; add `--catalog benchmarks/unified-steps-v1.toml`
with a different output path for signed-step pairs against already compact
inputs. Use `--catalog benchmarks/unified-floats-v1.toml` with another output
path for exact raw-bit/decimal float source pairs. Use
`--catalog benchmarks/unified-arithmetic-v1.toml` for checked subtraction,
division and remainder updates against already compact current source. Keep raw samples and reports
outside this tree. Passing a finite measurement does not complete the broad
performance or language-safety acceptance goals.

In a committed clean checkout, `tools/verify_checkout.py --profile full --jobs 1
--output /absolute/new/results` provides bounded exact-checkout verification.
Set `CARGO_TARGET_DIR` to a path outside that checkout. It records its selected
profile; dedicated CLI, executable, native/database and challenge qualifiers
must still run when those capabilities are in the integration scope.

## Syntax highlighting

The TextMate grammar supports current `fluxr 1` source and the retained legacy
rule, timeline, compact-module and gate scopes. Run its positive and negative
fixtures with real TextMate tokenization and Oniguruma WebAssembly. Install the
pinned Node dependencies outside the source tree, then pass that directory to
the existing harness:

```sh
npm install --prefix "$FLUXR_WORK/highlighting" --no-audit --no-fund --ignore-scripts \
  --save-exact vscode-textmate@9.2.0 vscode-oniguruma@2.0.1
node tests/highlighting.cjs "$FLUXR_WORK/highlighting"
```

These are lexical scope checks; they do not replace the compiler or runtime
behavior gates above. [Highlighting support](highlighting.md) defines token
boundaries, quoted-token handling and the limits of regex highlighting for
length-delimited native Rust payloads. Compiler, runtime and checked/native
testing APIs are unchanged by this editor-support slice.

## Cookbook and test framework

The [cookbook](cookbook.md) retains paired Rust/current FluxR programs and
public-compatible compact fixtures. Its Rust tests are included by the
all-targets gates above. The [testing guide](testing.md) defines the bounded
checked-source catalog, deterministic scalar and flat bool/i64/u64 list property
checks, failure-preserving shrinking and native libtest route. Generator work
is bounded across repeats, comparison backends and shrink reservations; scalar
seed sequences and existing catalog readers retain their contracts.

```sh
"$CARGO_TARGET_DIR/release/fluxr-cli" test-suite examples/cookbook/tests.json \
  --mode compare --test-threads 1
python3 -B tools/unified_tests/run_native_tests.py \
  --cli "$CARGO_TARGET_DIR/release/fluxr-cli" \
  --work "$FLUXR_WORK/native-test-work" \
  --output "$FLUXR_WORK/native-test-results" \
  --target-dir "$FLUXR_WORK/native-test-target"
python3 -B tools/bench_cookbook_tokens.py \
  --output "$FLUXR_WORK/cookbook-tokens.json"
```

The test runner's public flags and the performance collector's exact invocation
are documented in those guides. Keep reports and generated test projects outside
the source tree. A failed or unexecuted case cannot count as a pass.

## Native interoperability and optional qualification

Use [the native protocol](../tools/native_rust_tests/PROTOCOL.md) for the paired
Rust/native-interoperability tests and live PostgreSQL setup. The runtime and
application remain responsible for their documented cancellation and trust
boundaries. Rust callbacks can be caught as errors only when unwinding is enabled;
`panic=abort` retains native process termination.

For matched native-kernel and runtime-dispatch measurements:

```sh
python3 -B tools/unified_tests/run_performance.py \
  --cli "$CARGO_TARGET_DIR/release/fluxr-cli" \
  --work "$FLUXR_WORK/performance-work" \
  --output "$FLUXR_WORK/performance-results" \
  --target-dir "$FLUXR_WORK/performance-target"
```

That fixture requires a supported little-endian ELF64 environment. It reports
separate preparation/startup and batch-mean distributions, including all
empirical percentiles. Its checked kernel uses signed integers in the bounded
domain `0..996`, preserving an actual admitted AOT route and matching the native
control without overflow. Unsigned execution has separate fragment coverage.
The default is 9 batch/preparation observations and 20 startup observations;
`--samples` accepts 5 through 31 batch/preparation observations. These small
empirical distributions do not establish population tails.
See
[the multicore example](../examples/multicore-server/README.md) for build,
standalone export, official conformance and live-injection commands using the
bundled reference. The official workload also needs its declared Go/PostgreSQL
and host prerequisites. Any challenge performance claim needs a separate
compliant measurement on matched hardware.

## Acceptance contracts

[requirements.json](requirements.json) is the canonical task and acceptance
catalog: 124 tasks, 940 current criteria and 12 qualification scopes. Common
defaults and evidence templates are factored once at the top of the file. An
omitted task/profile status means `planned`; an omitted criterion status means
`open`. A task's `target_paths` are implementation planning hints and may identify
files still to be created. They are not an installed-file inventory.

The core, concurrency, agent and domain documents explain the intended contracts.
Task IDs, dependency edges, scope closures and criterion IDs remain the stable
links for integration planning. A passing subset does not complete a full task
or qualification scope. Use fresh results from the actual integrated candidate
to close gates, and preserve open or unsupported outcomes accurately.
