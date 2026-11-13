# Evaluation integrity and operating boundary

This patch implements a bounded, resumable **provider collector** and a standalone
**typed transcript grader**. It disables generated Rust replay through the old
in-process oracle. It does **not** implement a hostile-code sandbox, enable live
model trials, or complete every acceptance criterion of plan task A23.

## What can run now

```sh
python3 -m unittest discover -s tools/eval/tests -v
cargo build --locked --release --lib
python3 tools/bench_heldout_tokens.py seal --evidence /new/evidence-directory
python3 tools/bench_heldout_tokens.py replay --evidence /new/evidence-directory
```

The final command runs the original seven source and seven edit fixture pairs.
`isolation.py` checks exact SHA256 values for `benchmarks/cases.toml` and
`benchmarks/token-replay.rs` from commit
`72e27cacd1df7286b8a6490c4d31f943c660d833`. Caller-supplied cases must equal the
complete frozen `heldout` table. The former `override` argument is now rejected
before any compiler or binary can start. Mutating the supplied cases while leaving
`override=None` is rejected too. Updating frozen corpus hashes requires a reviewed
change; a freshly computed seal alone cannot bless modified executable fixtures.
The admitted case dictionary and harness bytes are private verified copies. The
compiler reads the copied harness, avoiding a verify-then-reread race with another
agent editing the checkout.

This is a trusted repository regression route. Its compiler, dependency library,
Python evaluator and checkout must be trusted. Hashes establish identity, not
authenticity or confinement. Other repository benchmark tools are outside this
patch and must not be used as arbitrary generated-code evaluators.

The corpus seal also includes every Python source in `tools/eval/`. Old seals must
remain historical evidence; create a new evidence directory after applying these
patches. Do not overwrite an old run or relabel its evidence as newly collected.

## Collector lifecycle

The existing `tools/bench_heldout_live.py` command keeps its pilot and repair
arguments and adds `--resume`. An explicit future provider run requires the
experiment's ordinary authorization and frozen cost/retry policy. Tests use only
local fake providers; installing the patch does not call a model.

Before invoking any provider, `run.json` records the complete schedule, prompt
hashes, settings, provider identity and corpus-seal digest. Each arm writes its
prompt and `intent.json` before spawning. `status.json` is written atomically on
normal completion, launch failure, timeout, output overflow, protocol failure or
controller failure. A cancelled run records its current arm and writes
`cancelled_before_launch` for every remaining unstarted arm, with no provider
process and zero incurred tokens for those uninvoked arms. Independent arms
continue after ordinary failures.

`summary.json` maintains the identity:

`scheduled = complete + explicit_failure + pending`

Here **complete means collector-eligible response**, not a behaviorally correct
program. `candidate_execution_authorized` is always false. An attempt must have
a zero process exit, exactly one completed turn with valid nonnegative integer
usage, exactly one nonempty answer, no tool action and no malformed/provider-error
event. A terminal completion/failure seals the attempt: subsequent answers,
restarts and other semantic events reject. No semantic tail events are allowed.
For compatibility with existing offline event exports, explicit thread/turn-start
events are optional; if present they must precede work and occur at most once.
Complete-only item exports remain compatible. Once an item start is observed,
updates must refer to its active ID and unchanged type, and completion must close
that ID before the turn can count as eligible. Duplicate starts, updates after
completion, rebinding item types, missing item payloads and unfinished starts
refuse. `tool_items` records observed tool phases, including a started tool with
no completion; it is not a count of finished tool invocations. This prevents a
partial stream from hiding a forbidden tool attempt. `unfinished_item_ids` retains
the incomplete lifecycle evidence while usage remains accounted for.
Empty responses, duplicate keys/completions/items, truncated JSON, invalid
UTF-8, unknown event kinds and type-confused token counts fail closed.

Usage observations are retained even for failed attempts. Multiple observations
are reported as ambiguous instead of being treated as a reliable bill. Missing
usage remains unknown, never zero. No automatic retry occurs. Initial and repair
attempt directories remain separate so episode-level cost can include both.

With `--resume`, an identical sealed schedule reuses completed attempts and checks
their artifact hashes without invoking a provider. A directory with intent but
no terminal record becomes `interrupted`; it is not automatically repeated because
it may already have incurred cost. Even if its partial log contains a valid usage
record, accounting remains `unknown_after_interruption`; the observed record is
retained without asserting that it covers the complete provider lifetime.
Cancelled-before-launch arms remain terminal on resume and are not invoked later.
Changed settings, prompts, seals or raw artifacts
block reuse. Retry policy changes require a new run directory, manifest and attempt
identity. Concurrent writers to one run directory are not supported: assign one
controller lease per evidence directory; parallel controllers use separate runs.

### Bounds and portability

The implemented collector supports POSIX. On another platform it records
`unsupported_platform` before launching. Defaults are 2 MiB prompt, 8 MiB event
output, 2 MiB stderr and 1 MiB per JSONL event. Optional sealed settings
`max_event_bytes` and `max_stderr_bytes` set the output caps. Streaming collection
never retains more than the cap in the evidence file. A monotonic wall deadline
also covers descendants that inherit pipes after the parent exits.

The collector starts a separate process group and kills ordinary descendants on
timeout, cancellation, overflow and parent completion. This is cleanup for the
trusted provider controller, **not hostile-process isolation**: a hostile process
can create another session; CPU, address space, network, filesystem and secrets
are not isolated here. The provider process inherits its ordinary authentication
environment. Generated Rust is never executed in that environment.

## Trusted typed transcript grading

`grader.grade_transcript` consumes a supervisor-owned case ledger, bounded raw
bytes, a 256-bit run nonce and the externally observed process outcome. It launches
no code. Each response must be one JSONL object with exactly these fields:

```json
{
  "protocol": "fluxr-grade/1",
  "run_nonce": "64 lowercase hexadecimal characters",
  "case_id": "case-001",
  "observation": {
    "result": {"value": {"type": "i64", "value": 7}},
    "fuel": 10,
    "live_bytes": 8,
    "trace": []
  }
}
```

Cases occur exactly once in supervisor-selected order. Every expected observation
is compared outside candidate code. Early successful process exit, a printed
`PASS`, missing/extra/duplicate/out-of-order responses, wrong nonces, malformed or
truncated JSON, unknown fields and nonzero exits all fail. Return values, trap
identifiers, logical fuel, live-byte observation and ordered traces compare
exactly. Runtime adapters must define the meaning and measurement point of each
field before using it as an experimental oracle.

The experimental value encoding supports `i64`, `u64`, `bool`, `unit`, `f32_bits`,
`list` and `tuple`. Float words are eight lowercase hexadecimal characters, so NaN
payloads and signed zero survive the protocol. This encoding is a grader transport,
not a new FluxR source/IR format or a claim of complete language coverage. It does
not yet encode named schemas, records, variants or host handles. A future adapter
must use a versioned extension rather than silently coercing them.

Limits are 10,000 cases, 8 MiB transcript, 1 MiB per response, 32 nested value
levels and 65,536 nodes per checked tree. Empty ledgers are invalid; otherwise a
program that executed no cases could pass vacuously. The result includes ledger
and transcript digests and the number of compared cases. A passing finite suite
is test evidence, not proof. A run nonce prevents accidental cross-run mixing;
it does not authenticate truthful execution or hide the oracle.

## Remaining A23 integration work

`require_untrusted_sandbox()` deliberately always raises. There is no flag that
converts a caller's assertion into sandbox admission. Before replacing that guard:

1. Select and pin an OS boundary and trusted image/toolchain. Compile generated
   source inside the boundary too: `include!`, `include_str!`, `env!`, compiler
   plugins, build scripts and dependency resolution are compilation-time attacks.
2. Mount only declared inputs, keep expected results and grading credentials
   outside, deny network and host secrets, and enforce CPU, memory, process, disk,
   output and wall budgets using the OS rather than voluntary guest cooperation.
3. Implement a language adapter that returns the grader protocol for all frozen
   cases, including exact trap/trace/resource semantics. Trusted FluxR and Rust
   reference observations must be computed independently of the candidate.
4. Run benign but adversarial compile-time filesystem/environment, runtime file
   and network, process escape, noncooperative worker and output-flood probes on
   every supported platform. Confirm complete process-tree reclamation and lack
   of access to oracle files.
5. Bind source, compiler, image/policy, target, semantic profile and trusted grader
   identities in the attempt record. Permit execution only after independent
   review of this boundary. Keep the frozen-fixture regression route separate.

Only then can A23's compile/runtime isolation criteria close. The patch already
provides executable checks for accounting and transcript completeness; those
checks do not substitute for the remaining OS-boundary tests.

## Design references

- Python subprocess documentation: process groups, pipe caveats and timeout
  behavior: <https://docs.python.org/3/library/subprocess.html>.
- Python selectors documentation: nonblocking readiness multiplexing:
  <https://docs.python.org/3/library/selectors.html>.
- Python JSON documentation: permissive duplicate-name/nonfinite behavior needs
  explicit rejection for the grading protocol:
  <https://docs.python.org/3/library/json.html#standard-compliance-and-interoperability>.
- Original defect location: the pinned
  [replay wrapper](https://github.com/Ryd-rb/FluxR-lang/blob/72e27cacd1df7286b8a6490c4d31f943c660d833/tools/bench_heldout_tokens.py)
  puts candidate Rust in the same process as
  [the oracle](https://github.com/Ryd-rb/FluxR-lang/blob/72e27cacd1df7286b8a6490c4d31f943c660d833/benchmarks/token-replay.rs).
