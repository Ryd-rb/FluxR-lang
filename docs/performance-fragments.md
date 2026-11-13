# Fragment latency and percentile comparison

`examples/bench_fragments.rs` and `tools/fragment_performance.py` provide a
bounded, reusable experiment for **60 checked fragment cases**. They add
individual invocation observations and complete empirical distribution
comparisons to the existing matched Rust/JIT benchmark. They do not replace
`bench_jit`, its released `fluxr-jit-performance-v12` schema, the independent
semantic fixtures, or the integration gates.

The new report, capture and comparison schemas have independent `v1` identifiers.
They do not change language, crate, source-codec or artifact versions. Generated
reports and process logs belong outside the source and implementation archive.

The current workload catalog is `checked-fragments-v3`. The report schema remains
`fluxr-fragment-performance-v1`: catalog identity is independent of the report
encoding, language and artifacts. The reader still accepts the exact earlier
41-case catalog (`checked-fragments-v1`, also the default for reports without a
catalog field) and the complete 55-case `checked-fragments-v2` catalog. It rejects
partial or unknown catalogs and comparisons between different catalogs. Use the
identical current probe on both implementation trees to compare all 60 cases.

## Coverage and execution routes

Every case has a source identity, deterministic 64-row input/limit/oracle
identity, artifact identity, independent Rust-calculated expected values or
explicit expected errors, and observed route counts. Artifacts are decoded and
re-encoded before execution. Current-source builds also compile each canonical
`fluxr 1` representation and check its artifact equality. Canonical spelling has
its own byte count and SHA-256: parsing unlike spellings is not a matched-input
compile comparison.

| Fragment group | Measured behavior |
| --- | --- |
| Signed scalar and inputs | Zero through five arguments, split host inputs and arguments, checked add/subtract/multiply/divide/remainder, small fixed boundaries and generic boundary controls |
| Boolean and control flow | Signed comparison returning Boolean, Boolean identity/equality, mixed Boolean/signed branches, split Boolean host inputs and signed arguments, a five-lane generic boundary control, `and`/`or`/`not`, skipped arithmetic traps, early return, loop assignments, break/continue, acyclic calls and recursive calls |
| Dynamic loop metering | Nested loops, parity-dependent branches with remainder, acyclic helpers called from a loop, a looping callee, and repeated record updates; zero-trip inputs and retained record ownership remain explicit |
| Ownership and nominal values | Unchanged records, record update/construction, nested fields, nominal methods, unchanged input graphs, record/list copy-on-write |
| Unsigned and binary32 | `u64` arithmetic, strict binary32 multiply/add/absolute value/square root, signed and unsigned conversions, exact result bits; explicit NaN, infinity and signed-range saturation cases for `ftoi64` |
| Collections and variants | List literal construction, empty and nonempty inputs, indexing/update, push/pop/last/empty, nested lists, enum tags, discrimination and payloads |
| Specialized binary32 scan | Empty, eight-element and 1,024-element lists; increasing data with a planted minimum, fully descending data, and mixed signed/exceptional data with varying minima, subnormals, infinities, signed zeros, NaNs and ties. `PrecompiledScan` remains distinct from generated JIT code |
| Failure and admission | Signed and unsigned checked overflow, division traps, uninitialized-before-arithmetic failure, empty-list last/pop, variant payload mismatch, bounds, fuel/cells/frames, wrong types and wrong arity |

The three long-scan distributions prevent tuning only for increasing data with
one planted minimum. Descending data changes the selected minimum at every
position and independently expects index 1,023. Mixed data includes positive and
negative finite values, subnormals, both zeros, positive infinity, and quiet and
signaling NaNs of both signs. Its expected result is either the deliberately
planted first NaN or the first planted equal minimum at a deterministic varying
position. Those expectations do not call the runtime scan or its ordering helper.
The normal canonical/artifact checks, per-input identities, logical limits and
separate timing phases apply unchanged to all three distributions.

Float-to-signed conversion retains its existing saturating semantics: NaN maps
to zero and out-of-range values clamp to the signed endpoints. The new edge-case
oracles use literal binary32 bits and explicit expected integers, including the
largest binary32 value below positive 2^63. They do not call the runtime cast.
Boolean/signed cases require actual `TypedJit` execution on the supported Linux
GNU x86-64 candidate, including the five-lane generic control; feature selection
alone does not satisfy that witness.

The five dynamic-loop cases independently calculate results using bounded Rust
integer ranges and checked arithmetic. Their 64 input rows repeat zero through
twelve iterations. Every case requires actual `TypedJit` execution on Linux GNU
x86-64 when the `typed-jit` feature is enabled. A record loop with zero iterations must return the original `Arc`;
positive iterations must produce a different allocation while retaining the
input. These workloads exercise metering paths separately from the unchanged
budget-boundary and first-error regression suites. Default-budget success does
not prove correct behavior at an exhausted fuel, cell or frame limit.

The suite spans the implemented checked value families and major expression,
statement and admission families. It is a finite set of programs and inputs,
not exhaustive language qualification. Typed native services, runtime registry
lookup, installation/publication/control, injection and standalone export are
separate real-execution integration gates. GPU, native ML/audio/device paths,
operating-system scheduling, concurrency and live-agent coding tasks have their
own contracts and are not measured by this microbenchmark.

The ordinary build measures `portable`, `named` and `prepared` entry APIs.
Portable calls force the portable evaluator. Prepared calls record their actual
route on **every timed individual invocation**, as well as during validation.
Named calls have no per-call route API and remain labeled `Unattributed`; a
separate prepared witness is not presented as instrumentation of named calls.
Unsupported prepared shapes and native declines retain their observed portable
route. Eligible scalar, branch, loop, call and flat-record cases require real
`TypedJit` results on Linux GNU x86-64 with the feature enabled. Record
construction, recursion and other unsupported shapes remain visible fallbacks.

Exact values, first errors, fuel/cells/frames and input ownership are mandatory.
Unchanged-record returns must retain their original `Arc`; changed records and
lists must return a different allocation while preserving the input. Every
individual timed result is checked after the timer stops. Batch results are
consumed with `black_box`, after the same corpus has passed validation and
warmup. Failure assertions terminate the probe rather than producing a passing
partial report.

## Observation units

| Measurement | Default observations | Unit and included work |
| --- | ---: | --- |
| Independent semantic rows | 64 per case and route | Exact values/errors and ownership, outside timing |
| Warmup | 256 calls per route | Excluded from reported timing |
| Individual warm invocation | 10,000 per case and route | One timed public API invocation, including timer and harness dispatch/attribution; result checking and disposal excluded |
| Warm batch | 31 batches of 256 calls | Mean nanoseconds per call in that batch; includes result disposal |
| Compact compile, canonical compile, artifact decode | 11 each per case | One parse/check/compile or decode/validation operation |
| Cold preparation, cached preparation | 11 each per case | One public preparation operation; cold means a fresh module in an already warm process, including code publication when admitted |
| First call | 11 per case | One invocation with a new workspace after preparation |
| Module destruction | 11 per case | Dropping one prepared owner, including executable-page and unwind-registration teardown where present |
| Startup-only process | 11 per capture | Bounded spawn, output collection and process-group cleanup of the benchmark's startup-only mode |
| Timer calibration | 4,096 empty and 4,096 injected observations | Raw timer overhead and an explicitly labeled 200,000 ns spin pause every hundredth calibration call |

Routes rotate by input index or batch round. Each route receives the same
input, in the same deterministic corpus order. The empty-timer distribution is
reported without subtraction. The injected calibration pauses are not mixed
into program samples. The calibration also retains reconstructed 64-call means
to show how batch averaging smooths a rare pause. A calibration pass demonstrates
sensitivity to this injected event; it does not qualify deadlines or scheduler
behavior.

The retained `cold_prepare` field does not measure process-cold compiler
initialization: earlier cases and validation may already have initialized global
target state. `first_call` has a fresh workspace but follows completed preparation.
The startup-only process does not compile or prepare a workload. Keep these
boundaries when interpreting the existing schema.

These host-entry measurements include dispatch, argument admission and result
handling inside the called API. They are not isolated generated-kernel timings.
Individual invocation and batch mean distributions have different units and
disposal costs; do not subtract them or interchange their percentiles. Compiler
process time belongs to the external build record. The existing `bench_jit`
provides separately labeled checked Rust kernel/host controls; this new probe
does not invent a Rust or .NET speed comparison.

Startup capture now uses the bounded collector and its unit explicitly includes
collection and reaping. The earlier unbounded startup unit remains readable but
is not interchangeable with this unit. In-probe latency measurements exclude the
external Python launcher and collector.

## All empirical percentiles, including regressions

Every distribution retains raw observations in execution order, sample count,
observation unit, nearest-rank **p0 through p100**, and **p99.9**. P0 is the
minimum and p100 the maximum. The comparison also sorts both complete sample
sets and compares **every order statistic**, so a regression between two integer
percentile points cannot disappear. It reports improved/equal/regressed rank
counts, the largest delta and its one-based rank. The complete raw arrays remain
in the captures, so every individual comparison can be reconstructed without
repeating a dictionary for every observation.

`all_observed_percentiles_nonregressing` is true only if every empirical rank
is no larger in the candidate. The corresponding improvement field requires
every rank to be strictly smaller. These are exact descriptions of these
observations. A lower median with a worse tail cannot pass the all-rank check.
Zero timer observations remain zero, and division by a zero baseline produces
an unavailable ratio rather than infinity or an invented speedup.

The default individual samples leave only about ten observations above p99.9;
the maximum is one observation. Lifecycle and startup have eleven observations,
and warm batch distributions have 31. Reporting a quantile does not make it a
precise population estimate. Shared-host pauses, correlated calls, compiler
layout and one-build effects remain visible. Independent builds/layouts,
repeated runs and uncertainty appropriate to the intended claim are still
required. R02 and universal/all-language/all-percentile improvement goals remain
open. By default the tool returns success for a consistent comparison even when
it reports regressions; that exit code is not a performance gate. An opt-in exact
empirical gate is available with `compare --require-nonregression warm` or
`compare --require-nonregression all`. Both require every matched sorted rank to
be no larger in the candidate, with no rounding tolerance or discarded samples.
`warm` covers every compared warm route and both individual and batch units;
`all` also covers matched lifecycle phases and process startup. Explicitly
excluded unmatched phases remain outside the gate. Selecting a route pair limits
the warm comparisons to that pair, and is retained in the report.

The full comparison, all regressions and the gate's failed distributions are
written before a failed gate returns exit code **2**. Invalid report data or an
output failure returns **1**; a consistent default comparison or a passing
selected gate returns **0**. CLI usage errors also return **2**, without creating
a comparison. Equality satisfies nonregression. A passing gate describes these
observed distributions only, and cannot establish statistical significance,
population tails, a universal speedup or every-percentile improvement.

For small changes, retain a same-binary A/A control with the same schedule, then
repeat matched builds and process runs. Alternate paired execution order (for
example ABBA, then BAAB) and keep per-build/per-run comparisons instead of pooling
correlated invocations as independent trials. A real package rebuild must be
verified; changing a build label is insufficient. Record resulting binary hashes:
identical deterministic executables do not establish independent code layouts.
Timer quantization and the A/A spread can exceed a claimed nanosecond change.
Keep all scheduled observations, tails and failures even when they reverse a win.
The distinction between build, process and iteration variation follows the
primary research of Kalibera and Jones,
[Rigorous Benchmarking in Reasonable Time (ISMM 2013)](https://kar.kent.ac.uk/33611/45/p63-kaliber.pdf).
The empirical gate does not implement that paper's uncertainty estimates.

## Build and capture

Use the pinned toolchain, locked dependencies, one Cargo job, external targets
and the [resource coordinator](resource-safety-v1.md). Build and execute the
oracle checks before measurements:

```sh
cargo test --locked --release --features typed-jit --example bench_fragments --jobs 1
cargo build --locked --release --features typed-jit --example bench_fragments --jobs 1
python3 -B tools/run_python_suite.py tests test_fragment_performance.py
```

After all source edits and formatting are complete, capture the exact candidate:

```sh
python3 -B tools/fragment_performance.py capture \
  --source /absolute/source-tree \
  --binary /absolute/external-target/release/examples/bench_fragments \
  --identity exact-source-or-archive-identity --build-id actual-release-build-id \
  --features typed-jit --output /absolute/external-results/candidate.json
```

`--samples` accepts 1,024–16,384 individual observations, `--rounds` accepts
5–101 batches and `--batch-calls` accepts 32–4,096 calls per batch. Defaults above
are the normal comparison settings; changing a count produces a different
experiment and cannot be silently paired with the original. Output paths must
be new and outside both the selected source and the tool's own source tree. The
tool atomically reserves a private `.logs` directory and retains raw output and
terminal outcomes, including failed attempts. The POSIX collector caps the main
probe's stdout at 64 MiB, each stderr at 64 KiB and each startup stdout at 1 KiB.
Timeout, cancellation and output overflow terminate its ordinary process group;
none can publish a successful capture. Reads and serialized capture/comparison
writes are bounded to 64 MiB. The probe's bounded case/count configuration fits
this limit without discarded samples. Use the resource coordinator's outer
process-tree timeout/watchdog for live reserves, escaped-descendant tracking and
the shared heavy-work queue. The collector manages trusted local processes; it
does not provide a hostile-code sandbox. Non-POSIX capture is explicitly
unsupported; offline report checking and comparison remain available.

The 55- and 60-case catalogs require complete bounded main/startup outcomes.
Earlier 41-case captures remain readable without pretending that bounded
collection ran retroactively. Capture also checks that the binary honored the requested sample
counts and route schedule, and that host affinity and compiler flags stayed
unchanged alongside source and binary bytes.

Capture records the active rustc/Cargo versions, declared release/features,
relevant compiler environment overrides, executing host/CPU affinity, source
inventory hash, dependency lock hash, binary hash and embedded probe-source
hash. Source and binary identity must remain unchanged through measurement.
Build IDs identify actual builds; changing a label cannot create an independent
build replication. Retain the exact build command/configuration and its outcomes
externally alongside captures. Metadata and hashes bind observations; they do
not authenticate a producer or prove that an arbitrary supplied report ran.

For an incoming-pack comparison, add only the **identical probe file** to an
external copy of that exact input tree. Preserve all implementation files,
locks and toolchain pins. Build each tree sequentially with matching release
settings. Preserve each executable before a shared target can overwrite it.
When reusing one Cargo target across checkouts with identical package names and
preserved old file timestamps, invalidate the changed package before switching:
`cargo clean --package fluxr --release --target-dir /absolute/external-target`.
Run this only after the previous task-owned writers have exited, retain each
completed benchmark binary outside the target, and rebuild from the explicitly
selected manifest. Dependency caches may remain shared. Alternatively, use
separate targets sequentially within the measured resource budget. A fast
Cargo no-op or a source hash cannot prove that a stale library was rebuilt.
If Cargo refuses cleanup because ownership/cache metadata is absent, preserve
that refusal and use a fresh target; do not fabricate a marker or broadly delete
files to bypass the safeguard.
Capture both with the same settings, then compare:

```sh
python3 -B tools/fragment_performance.py compare \
  --baseline /absolute/external-results/incoming.json \
  --candidate /absolute/external-results/candidate.json \
  --output /absolute/external-results/comparison.json
```

Append `--require-nonregression warm` to use the exact warm-rank gate while
retaining the full report on failure. Broad candidate qualification still needs
the unchanged integration and semantic gates.

The comparator refuses unlike compiler/configuration/hardware, target/feature,
probe, input/limit/oracle, sample-count, observation-unit and route-schedule
identities. Dependency lock differences are retained explicitly as implementation
differences. Canonical compile phases with different source spellings remain in
their captures and are excluded from equivalent-input comparisons.

## Bounded public-baseline adapter

The public API predating `PreparedCall` and canonical `SourceDocument` supports
the same compact fixtures and the `portable`/`named` execution methods. To run
this exact probe against such an inspected source baseline, use the explicit
probe-only adapter:

```sh
cargo rustc --locked --release --features typed-jit --example bench_fragments --jobs 1 \
  -- --cfg fluxr_public_baseline --check-cfg 'cfg(fluxr_public_baseline)'
```

This compiler option applies to the benchmark example, not FluxR library code.
It does not implement a missing backend, replace source or alter dependencies.
The adapter reports the prepared-call/canonical API limits, and named route
attribution stays unavailable. On a baseline with a Windows-only JIT, a Linux
feature build does not become a Linux native execution result.

Use `capture --common-routes` for **both** the public baseline and current tree.
This times exactly the same two-route rotation. The current tree still checks
its prepared route during validation, but does not insert it into this timing
schedule. Use separate ordinary three-route captures for incoming/current
prepared-call comparisons; a two-route capture cannot be silently compared with
a three-route schedule.

Public `prepare_native` and current `prepare_call` have different return and
lookup obligations; their first calls also use different APIs. Their cold/cached
preparation and first-call distributions remain visible in captures and are not
compared as equivalent costs. Common compact compile, artifact decode, module
destruction and matched named/portable warm observations remain comparable within
their stated scopes. Exact public default-branch/commit acquisition, repository
diff review and platform qualification remain separate integration obligations.

## Source parsing and checked printing

`examples/bench_source.rs` separately measures eight default source inputs:
a small checked function, 64 checked update helpers, and 64 native declarations
using unquoted tokens, quoted JSON tokens, escaped/Unicode strings, or raw
byte-counted Rust items; two further 64-helper cases exercise finite binary32
literals and exact signed-zero, subnormal, infinity and NaN sign/payload bits.
All eight inputs use retained syntax, including raw-bit float spelling, so the
identical probe can run against the incoming implementation for matched source
API comparisons. Every input has the independent checked result
`run(1) = 18`; native spans, digest, checked AST, artifact equality and canonical
round trips are checked outside the timer. Native Rust is parsed for boundaries
in this experiment; it is neither compiled nor executed by this probe.

The optional `capture --decimal-literals` flag adds a ninth case with the new
decimal float spelling (the child receives `FLUXR_SOURCE_DECIMAL=1`). Its finite
helper AST and artifact must equal the raw-bit case. Use it only with an
implementation supporting that syntax, and keep the setting identical in both
comparison arms. The source-report reader retains the exact six-case legacy
catalog while accepting the complete eight- or nine-case catalogs. Comparisons
reject differing case lists, settings or probe identities; an older parser is
not credited with accepting the new spelling.

Each case has 32 warmups and, by default, 10,000 individual observations for each
of three APIs: document parsing, compilation of the already parsed checked
projection, and canonical printing of a compiled checked module. Native items
are excluded from the latter two projections. Printing includes the printer's
decode, print, reparse and AST-equality validation. The probe's additional
compile/encode round-trip check runs outside timing. Every returned value is checked and
disposed after timing. Empty-timer calibration is retained without subtraction.
These measurements exclude compiler process time, native Rust compilation,
startup and call execution. They use the separate `fluxr-source-performance-v1`
schema and do not redefine the invocation or lifecycle fields above.

```sh
cargo test --locked --release --features typed-jit --example bench_source --jobs 1
cargo build --locked --release --features typed-jit --example bench_source --jobs 1
python3 -B tools/run_python_suite.py tests test_source_performance.py
python3 -B tools/source_performance.py capture \
  --source /absolute/source-tree --binary /absolute/target/release/examples/bench_source \
  --identity exact-source-identity --build-id actual-release-build-id \
  --features typed-jit --output /absolute/external/source-candidate.json
python3 -B tools/source_performance.py compare \
  --baseline /absolute/external/source-baseline.json \
  --candidate /absolute/external/source-candidate.json \
  --output /absolute/external/source-comparison.json
```

The source tool retains raw observations and compares every empirical rank,
integer percentile and p99.9 with matched input, artifact, compiler, host and
sampling identities. A changed canonical spelling is explicitly reported for
printing the same checked artifact; it is not a matched-output byte-copy claim.
Inputs to parsing remain byte-identical. Follow the same external-output,
resource, repeated-build and A/A controls as for invocation measurements.
This finite parser/printing experiment cannot qualify all source shapes,
population tails, end-to-end application builds or native service performance.
