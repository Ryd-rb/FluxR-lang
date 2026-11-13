# Resource safety contract v1

`tools/resource_guard.py` provides a Linux coordinator for trusted development
commands. It admits one expensive process tree at a time and starts a live
resource watchdog before accepting the first job. It is separate from FluxR
runtime execution bounds and from arbitrary-candidate compilation/execution
isolation. It does not establish grading authority, hermeticity, authorization,
semantic independence, or completed domain qualification.

## Admission and reserves

The default policy is recorded in `policy.json` at initialization. A job must
declare anticipated additional disk bytes, memory bytes, and inodes. Admission
requires each observed available amount minus that additional budget to be
strictly greater than the corresponding stop reserve. Zero, negative, nonfinite,
unknown, and malformed policy values are rejected; budgets may be zero but must
be finite and nonnegative. Timeouts are finite positive seconds.

| Resource | Stop reserve | Emergency reserve | Behavior |
| --- | ---: | ---: | --- |
| Disk available to this process | 8 GiB | 4 GiB | TERM at stop reserve; KILL immediately at emergency reserve |
| Available memory | 1.5 GiB | 0.75 GiB | Same; use the minimum of host availability and accessible cgroup limits |
| Available inodes | 100,000 | 50,000 | Same |
| Sampling interval | 0.5 seconds | At most 1 second configurable | Already running before spawn |
| TERM grace | 2 seconds | Zero at emergency reserve | Configurable up to 5 seconds |
| Combined stdout/stderr per job | 32 MiB | No additional spill | Overflow terminates the job and cannot pass |
| Concurrent expensive jobs | 1 | 1 | One server lock and one serialized queue |

Crossing an emergency reserve during an existing TERM grace also triggers
immediate KILL. A final resource sample rejects an otherwise successful command
that crossed a reserve between periodic samples. Sampling failure and audit-log
failure terminate active commands before attempting best-effort error logging.
These thresholds are safeguards, not estimates of an operating system's exact
failure point; reserve margins must exceed the admitted aggregate growth rate
between samples. Unknown external consumption is not a guarantee of unlimited
capacity. Separate project quotas and inaccessible cgroup constraints are
explicitly reported as unknown.

For cgroup v2, every readable ancestor of the current hierarchy is observed. An
unlimited root does not mask a tighter descendant constraint. `statvfs` provides
the process's disk/inode availability. The coordinator records whether cgroup
files were readable, rather than assuming host RAM is the effective quota.

## Workspace and scheduling

Use one coordinator for all agents. Parallel research, editing, and review may
continue within the shared budget; route acquisition, extraction, source preparation,
compilation, native qualification, and required executions through this queue.
Compiler job limits are independent of the queue limit and must be supplied
explicitly, for example `CARGO_BUILD_JOBS=2`. Do not create duplicate per-agent
Cargo targets. Fresh or independent builds use distinct targets sequentially.
Keep each copied runtime or source checkout in its own target directory, including
generated application projects. FluxR emits both `rlib` and `cdylib` products;
reusing one target across source copies can overwrite an unhashed library while
another copy retains a valid Cargo fingerprint. Share the candidate target among
agents, but isolate generated-application and comparison-source targets.

Keep the coordinator state, toolchains, Cargo caches, targets, temporary files,
and evidence outside verified checkouts. Before expensive phases, retain a
durable recovery checkpoint containing the exact source snapshot and checksums,
toolchain/configuration identities, completed and unfinished work, actual check
results, and the next executable command. Store it outside disposable workspace
and build storage. The guard's operational state does not replace these source
checkpoints.

The following example assumes a task root containing the candidate checkout and
separate `state`, `disposable`, and `evidence` directories. Use absolute paths in
actual receipts. The guard does not install or select a Rust toolchain itself.

```sh
python3 tools/resource_guard.py --state /task/state init --root /task
python3 tools/resource_guard.py --state /task/state register /task/disposable/target \
  --purpose 'Cargo target for candidate tree and pinned toolchain'
python3 tools/resource_guard.py --state /task/state serve
```

Keep `serve` running in a separate supervised session. Only submit after its
`ready: true` response. Supply a JSON object of explicit environment overrides,
including workspace-local `CARGO_HOME`, `RUSTUP_HOME`, `CARGO_TARGET_DIR`, writable
`TMPDIR`, explicit `PATH`, `CARGO_BUILD_JOBS`, and any other build configuration.
The request retains these overrides verbatim; do not put authentication secrets
in job arguments or override files. The inherited process environment is not
claimed to be a hermetic dependency inventory.

```sh
python3 tools/resource_guard.py --state /task/state submit \
  --id candidate-debug --cwd /task/candidate \
  --disk-gib 10 --memory-gib 6 --inodes 200000 --timeout 1800 \
  --env-json /task/evidence/build-env.json -- \
  cargo test --locked --all-targets
python3 tools/resource_guard.py --state /task/state status
```

The explicit budget must cover the actual phase, including dependencies, logs,
archives, source reconstruction, and finalization. If several activities are
outside the queue, their aggregate peak must be included in the reserved budget
before admission. A denied phase remains unexecuted. An interrupted phase remains
incomplete. A zero child exit after output overflow, reserve crossing, background
writer termination, or a watchdog failure cannot produce `passed`.

## Process ownership and cleanup

Each job starts in its own process session. The coordinator is a Linux child
subreaper, tracks descendant PID/start-time generations, terminates the owned
group while the leader's generation is still present, and signals matching owned
descendants individually. PID reuse alone cannot authorize a signal. The output
drain and exceptional wait paths are bounded. A successful leader leaving live
background descendants is classified as terminated, and the descendants are
killed and reaped. Remaining survivors prohibit subsequent admission and retain
the unresolved running marker.

SIGTERM, SIGINT, and the CLI `stop` operation request supervised termination;
they do not silently abandon the active job. `serve` refuses a second server for
the same state and refuses restart when unresolved running markers remain. This
is trusted-process lifecycle management, not containment for malicious code that
attacks the coordinator, its files, PID visibility, or the kernel. Arbitrary
candidate compilation and execution still require independently qualified
isolation, protected grading authority, and immutable replay admission.

Disposable outputs must be registered as new empty directories before their
writers start. Registration records an ownership token and directory device/
inode identity; overlapping registrations, symlink paths, and protected roots
are rejected. Pruning requires the server stopped, no unresolved running jobs,
a matching marker and directory identity, and no nested `.git`, `bundle`,
`bundles`, `patches`, `evidence`, `deliverables`, or `upload` component. The entire
ledger and all evidence are retained.

```sh
python3 tools/resource_guard.py --state /task/state stop
# Observe state=stopped and no running job before pruning.
python3 tools/resource_guard.py --state /task/state prune /task/disposable/target
```

A refused prune is not permission to fall back to `rm -rf`. Some third-party
caches may contain conservatively protected directory names; preserve them or
review an explicit, separately recorded migration. Do not register source,
Git history, patches, bundles, inputs, evidence, or deliverables as disposable.
Older development ledgers without device/inode binding remain readable but are
ineligible for this version's prune operation until explicitly migrated. After
stopping the coordinator and verifying no unresolved writers, run:

```sh
python3 tools/resource_guard.py --state /task/state migrate-legacy /task/disposable/target
```

Migration requires an explicit stopped status and the same protected-content,
symlink, marker, and non-overlap checks. It retains the exact prior ledger bytes
under `ledger-history/<sha256>.json` and the original entry inside the new one.
The new fields bind only the directory's **current** device/inode at migration;
`historical_directory_identity` remains `unknown_not_asserted`. It does not
invent a historical identity or rewrite the original registration. Already-bound
entries cannot be rebound as legacy. A refused migration preserves the old ledger
and does not authorize deletion.

## Evidence, recovery, and rollback

`requests/<job>.json` retains the exact consumed request bytes. The receipt binds
its SHA-256, complete command and working directory, budget, timeout, retained
policy path/hash, guard implementation hash, timestamps, admission/final
resources, exit, termination reason, elapsed seconds, log length/hash, tracked
process generations, and survivors. `policies/<sha256>.json` retains the exact
policy loaded by the coordinator. Editing `policy.json` while a daemon runs does
not relabel the policy that daemon actually used. Stop and restart deliberately
to load a new policy.

`audit.jsonl` includes watchdog readiness, heartbeat/resource samples, admissions,
spawns, terminations, and completion. `initial_resources.json`, `ledger.json`,
per-job logs, request bytes, policy bytes, and result receipts belong in the
recovery evidence set. A receipt binds bytes and observations; it does not prove
that command semantics were correct or that an independent oracle ran.

After a crash, keep the unresolved running marker, existing logs, and audit.
Verify that the exact task-owned process generations and writers are gone before
any reclamation. Archive the unresolved request and record it as interrupted;
never convert its absence of a completion receipt into a pass. Restore the verified
source snapshot into a new directory with fresh coordinator state when proof of
prior writer termination is unavailable; leave the unresolved old state protected.
This implementation deliberately
provides no force-resume or force-prune option that bypasses those obligations.

Replace or disable the coordinator only after stopping/reaping its owned work and
retaining the complete state/results. Its replacement must preserve the required
admission and cleanup guarantees. Do not erase failed results or weaken task/profile
gates. A previous guard implementation's
receipts remain attributed to its original bytes; they do not become evidence
for this version merely because the new code is present.

## Required tests and scope

`test_resource_guard.py` is a required Python verification suite. It is selected
by the `python`, `debug`, and `full` verification profiles and cannot disappear
without making the profile incomplete. Linux facilities are required; there are
no success-producing platform skips in this suite. Use an external writable
`TMPDIR`. A direct run can write an additional measurement file at a new external
path through `FLUXR_GUARD_TEST_EVIDENCE`.

```sh
TMPDIR=/task/disposable/tmp PYTHONDONTWRITEBYTECODE=1 \
  python3 tools/run_python_suite.py tests test_resource_guard.py
```

The suite exercises numerical admission, finite configuration, duplicate-ID and
server exclusion, real serialized jobs, timeouts, SIGTERM/stop behavior,
background descendants, log overflow, an injected reserve crossing with a
separate-session descendant, audit failure including failed error logging,
emergency escalation, endpoint reserve checks, liveness immediately before
spawn, effective cgroup ancestry, policy/request binding, PID generation checks,
and safe refusal of unowned/overlapping/replaced/protected disposable paths.
The legacy migration tests preserve byte-for-byte ledger history, bind only the
current directory, and reject active/unresolved writers, unstopped states,
symlinks, marker mismatches, overlaps, protected contents, and attempted rebinding.
Injected metrics exercise failure paths without exhausting physical resources.
These small trusted Python process fixtures do not qualify arbitrary-code
isolation, a native backend, an operating-system-wide hard quota, or any broader
FluxR profile.
## Terminal audit and result publication

The required `job_finished` audit write occurs before atomic publication of a
successful result. If this write fails, the result is `audit_error`, retaining
the actual exit code and `command_status_before_audit_error`; the coordinator
stops and leaves later queued jobs unexecuted. No transient passed result is
published before this required audit operation. Fault-injection tests exercise
this exact final-write failure.

An audit line with `result_publication: pending` is an observation of a terminal
command, not an authoritative result. A crash before result publication leaves
the running marker and an incomplete attempt. Result publication precedes
retiring that marker, so a crash between those operations requires explicit
reconciliation before restarting. This is not a cross-file transaction, durable
exactly-once guarantee, authentication mechanism or hostile-code sandbox.

