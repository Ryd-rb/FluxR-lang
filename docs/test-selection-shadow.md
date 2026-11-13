# Full-suite shadow comparison for test selection

`tools/agent_pipeline/shadow.py` adds an opt-in C05 observation comparator around
the existing immutable impact-selection and complete-ledger implementation. It
compares a full baseline suite, the full candidate suite, and the selected
candidate suite at their exact declared snapshots. It retains every expected
attempt and detects cases in which a selector misses a changed observation or
an execution order produces a different result.

This module does not enable test omission. Every result has
`activation_authorized: false` and `full_suite_required: true`. It cannot prove
that an extractor discovered all dependencies, authenticate a log, or isolate
candidate code. The final repository verifier continues to require its full
applicable suite. A real deployment needs independently owned catalogs, oracles,
trusted execution evidence and a shadow campaign for its specific extractor,
runner, platform, features and effects. Finite fixture agreement cannot satisfy
those production obligations.

## Independent policy and exact subject

The caller supplies a `fluxr-test-shadow-policy-v1` policy from outside candidate
control. It contains exact baseline/candidate snapshots, independent complete
catalogs mapping test names to action IDs, complete full-suite test orders,
and the expected attempt IDs for each of `baseline`, `full` and `selected`.
Attempt IDs are globally unique and bounded to 64 per lane per comparison. This
is an input/execution bound, not an agent-count or development-capacity limit.

The context binds the runner, toolchain, target/profile, oracle, graph analyzer,
selector and evidence policy as SHA-256 identities. The policy and trial are
frozen through the existing restricted canonical-JSON encoder before use;
unknown fields and malformed identities refuse. The context is a declared
binding. Its correctness and authority must come from trusted orchestration;
hashing supplied text does not authenticate a producer.

A `fluxr-test-shadow-trial-v1` supplies the original selection, each lane's shard
plan, and every run. Selection validation reuses the released selector protocol.
Each plan must match the independent catalog, snapshot and ordered test manifest.
Both full plans must contain every test in the supplied catalog and use one
qualified order; a self-hashed reduced catalog cannot disguise an omitted test.
The selected plan must match the original selection and, if it uses a separately
qualified isolation contract, its runner and policy identities must also match.

A run has `schema: fluxr-test-shadow-run-v1`, `attempt`, `context`, `results` and
`observations`. `results` uses the unchanged `fluxr-test-result-v1` rows with the
exact plan, action, shard, snapshot, terminal, exit code and evidence references.
`observations` maps every planned test to the digest of its complete normalized
observation under the declared oracle. That observation must include all
relevant values, traps and effects; this module cannot discover omitted fields.
A timestamp-bearing log is not an appropriate substitute for an observation
contract. Evidence log digests are retained separately and may differ by run.

## Comparison and failure behavior

`compare_trial(policy, trial)` returns one of three statuses:

| Status | Meaning |
| --- | --- |
| `agreement` | All declared runs are complete and stable; every selected result agrees with full execution, and every changed candidate observation/action or newly discovered test was selected. This is finite selection agreement. |
| `divergent` | A changed test was omitted or the selected execution differs from full execution in terminal, exit code or observation digest. |
| `inconclusive` | A required attempt/result/observation is missing or invalid, execution was interrupted, retries disagree, baseline tests failed, or no selected test actually executed. |

The report preserves full/selected ledgers, observed failures, missing attempts,
invalid records, unstable retry outcomes, changed tests, unsafe omissions,
selected/full divergences and removed tests. Every report binds policy/trial
content and exact subjects. Malformed plans or external policy are input errors;
malformed run content becomes an explicit inconclusive observation.

A failing candidate may have correctly selected every failing test. In that case
the report can say `agreement` while `candidate_validation_passed` is **false**.
That field records whether the stable complete full candidate suite passed, not
whether omission is authorized. Conversely a full candidate can pass assertions
while a changed observation was omitted; its comparison remains `divergent`.
Neither situation is relabeled as a passing optimization qualification.

Missing/skipped/not-run/unsupported/timeout/cancelled/infrastructure outcomes
never establish agreement. A failed first attempt followed by a passing retry is
unstable; neither a majority vote nor the last successful retry removes the
failure. If a declared retry is absent, the comparison is inconclusive. A caller
cannot prove that undeclared runs were never attempted: it must independently
retain the attempt schedule, supervise execution and retain original run bytes.

The implementation is bounded by the existing 4 MiB canonical input and graph/
case limits. It performs complete comparisons of supplied ledgers. No action is
executed, no queue is mutated, and no evidence/publication authority is granted.

## Reproduction and executed fault fixtures

```sh
python3 -m unittest discover -s tests -p test_agent_shadow.py -v
python3 tools/qualify_test_shadow.py --output shadow-campaign.json
```

The campaign executes only checked-in, fixed Python fixture functions. It accepts
no arbitrary command, generated source or model-produced candidate. It records
policy, graphs/plans, all run results, raw observations, exact code hashes,
interpreter/platform metadata and comparison durations. Each full and selected
fixture invocation actually computes and checks its observations.

The eighteen trials include complete and deliberately incomplete dependency
claims for seven classes: negative lookups, configuration, generated inputs,
schema shape, effects, external resource identity and test discovery. Each
complete graph selects its affected test; deleting the relevant fixture edge
makes the comparator identify the unsafe omission. These are modeled dependency
observations, not a claim that a production compiler captures real filesystem,
plugin or effect dependencies. A known-incomplete graph and a discovered test
missing from the graph take the existing conservative full-suite fallback.

A fixture with hidden setup state demonstrates selected/full divergence caused
by an omitted prerequisite. A separate invariant accepts every pair of three
independent edits but rejects the composed triple. The full candidate catches
that higher-order failure even though all pairwise checks passed. This is a
finite counterexample to pairwise-only qualification, not a semantic rebase
implementation or proof for arbitrary source changes.

The campaign reports nine selection agreements and nine intentionally detected
divergences. Most agreement cases deliberately contain failing candidate
assertions; their candidate-failure status remains in the ledgers. The ordinary
unit suite additionally attacks catalog truncation, stale context/snapshots,
missing shards/results/observations/attempts, unknown terminals, duplicate IDs,
unstable retries and bounds. No frozen arbitrary-code replay admission catalog
or legacy test-selection meaning is changed.

## Research and remaining work

The official [Bazel Test Encyclopedia](https://bazel.build/reference/test-encyclopedia),
read on 11 November 2026, specifies declared test dependencies, execution
environment, runner outcomes and explicit sharding support. It supports keeping
full ordered execution, resource context and complete results in the comparison.
This module does not implement Bazel's environment or claim its guarantees.

The [Build systems à la carte author publication page](https://www.microsoft.com/en-us/research/publication/build-systems-a-la-carte/)
identifies the April 2020 journal version and executable framework; only its
abstract and metadata were freshly read here. The [Rust incremental-query
chapter](https://rustc-dev-guide.rust-lang.org/queries/incremental-compilation-in-detail.html)
was read for dependency/result stability. The engineering inference is to keep
selection, scheduling, execution and qualification as separate identities and
checks; the papers/docs do not prove this prototype sound.

Next steps retain C05's full production gates: trusted dependency extraction,
negative/range/configuration/generated/effect/resource coverage, isolated
runner qualification, independent oracle authority, and a representative shadow
campaign against the full repository matrix. Unknown dependency coverage still
requires conservative validation. Rollback disables the optional comparator and
keeps the unchanged full-suite path and original immutable run evidence.
