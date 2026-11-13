# Local agent pipeline prototype

The `tools/agent_pipeline` package provides working primitives for coordinating
trusted workers on one Linux host. It separates immutable action identity,
verified evidence storage, fenced job ownership, and advisory conflict planning.
No function executes a candidate, pushes a branch, merges a change, certifies a
release, or authenticates a worker. A production distributed controller remains
separate work, with replicated consensus, authenticated principals, hermetic
runners, admission quotas, retention and recovery engineering.

Run the behavioral tests from the repository root:

```sh
python3 -m unittest discover -s tests -p 'test_agent*.py' -v
PYTHONPATH=tools python3 -m agent_pipeline --help
```

## Action identity

An action has an explicit argument vector, working directory, environment,
declared input digests, output paths, complete source snapshot digest, toolchain,
runner, platform, target, profile and policy. The v1 key hashes all of these with
a versioned domain. The complete source snapshot deliberately sacrifices some
cache reuse until a trusted producer can establish a finer dependency closure.
The snapshot must cover file bytes, executable modes and relevant link metadata;
an arbitrary string chosen by an agent does not establish a complete snapshot.
The API validates shape and digest syntax, not that mapping to actual files.

The working directory is `.` or a normalized path relative to that snapshot.
All input/output paths are snapshot-relative, case-sensitive POSIX names.
Drive-qualified and drive-relative Windows spellings such as `C:/x` and `C:x`
reject before hashing. The
eventual runner must reject escaping symlinks and account for toolchain files,
environment, generated inputs, response files and undeclared filesystem access.
Do not reuse a result just because two untrusted actors supply the same key.

`fluxr-action-v1` uses UTF-8 JSON with sorted object keys, no whitespace, unchanged
Unicode strings and Python JSON escaping. Floats, duplicate keys, non-finite
values, integers outside signed 64-bit, deep nesting and oversized messages are
rejected. V5 measures exact escaped UTF-8 size before serialization, so the 4 MiB
budget also bounds allocation for API callers that supply in-memory values.
This restricted encoding is **not** advertised as RFC 8785. A client
in another language must pass the same canonical-byte fixtures before sharing
identities. An action key does not serve as a source node identity. Enqueue takes an owned
JSON-value snapshot before hashing and storing the packet; a caller mutating its
original dictionary cannot change the recorded action after the hash is computed.

## Fences and ownership

Register dependencies before their consumer. Task packets are immutable and
enqueue is idempotent only for identical packets. A claim becomes eligible when
all prerequisite jobs succeeded. Optional exclusive scopes protect shared
publication resources. Workers should use isolated worktrees: do not lock the
whole codebase or place all ordinary source reads under exclusive ownership.

Each claim has a monotonically increasing token. Renewal and completion check
the task, owner, token, exact immutable action/snapshot and unexpired lease in a
short SQLite transaction. A stale worker cannot publish its result after
reconciliation and reassignment. Expiry alone does not permit takeover: a
trusted controller must retain the abandoned branch, dirty worktree and useful
evidence and supply a reconciliation object before requeueing. The queue never
resets or removes worker files.

The clock floor persists even when a requested operation is rejected, provided
its database commit succeeds. A backward
wall-clock adjustment cannot revive a lease already observed expired. This
does not make a wall clock reliable for a distributed lease: it is a single-host
contract with a trusted controller. Forward clock jumps may invalidate work
early. The database must be on a local filesystem with working SQLite locking;
network filesystems and multi-region use are unsupported. A storage error is a
failed authority operation, not an accepted result. The implementation rolls
back a still-active transaction after failed commit so it cannot retain the
writer lock. Operators must stop acceptance and investigate storage failures;
this prototype does not implement authority recovery from a rolled-back backup,
clock failure, lost durable writes or disk corruption.

Lease fields and owner names are not credentials. The filesystem and controller
API must be inaccessible to untrusted workers. A later network service must
derive the owner from authentication and enforce authority at every publication
boundary; it cannot trust an owner field supplied by a client.

## Bounded dispatch and recovery

`claim_next(owner, ttl_seconds=300, after_sequence=0, scan_limit=128)` selects and
claims in the same short write transaction. Ready tasks are considered in their
original enqueue order. The index on `(state, enqueue_sequence)` avoids scanning
completed history. Each call considers at most `scan_limit` queued rows and
checks dependency states plus the currently held exclusive scopes. Dispatch and
state transitions read control metadata without loading immutable task packets.
V5 keeps active scope rows in SQLite, indexed by namespace and name. Exact,
ancestor and descendant queries replace reconstruction of the entire active set
on every scoped poll. Scope acquisition and release commit with the task fence,
so concurrent processes never rely on a stale controller-local index. These
changes remove two observed sources of unnecessary work under the writer lock;
`scan_limit` still bounds candidate count, not total writer-lock duration or
database size. An expired scoped attempt still owns
its scope until explicit reconciliation; automatic takeover would allow its
still-running process to interfere.

The result contains `lease`, `next_after` and `examined`. Continue a pass with the
returned `next_after`; when it is `None`, begin the next pass at zero. A result
without a lease does **not** prove global idleness: earlier blocked tasks may
have become eligible and later windows may contain work. Cursors grant no
authority and may be independently maintained by concurrent dispatchers. Each
successful claim is fenced inside the transaction, so two pollers cannot obtain
the same attempt. Sustained overloaded workloads still require admission limits,
priorities and fairness policy; this API does not promise starvation freedom.

```python
after = 0
result = queue.claim_next("trusted-runner-1", after_sequence=after, scan_limit=128)
if result.lease is not None:
    submit_to_authorized_runner(result.lease)  # Application-owned execution.
after = result.next_after if result.next_after is not None else 0
```

V6 adds `claim_ready()` with the same arguments and result shape. It uses a
partial index over queued tasks with zero pending dependencies, so a large
blocked DAG prefix does not consume every poller's scan budget. Use the CLI
`claim-next OWNER --ready-only` to select this policy. Its `examined` and cursor
describe only dependency-ready candidates; legacy `claim_next()` keeps its
original all-queued window semantics. Readiness is advisory until the transaction
also rechecks actual dependency states and scopes and commits the new fence.

Each enqueue records the number of unfinished parents. The first successful
completion decrements its immediate children's counts in the same transaction.
Failed attempts, retries and lease expiration do not release a dependency;
idempotent duplicate completion does not decrement twice. A success transition
with an inconsistent negative counter rolls back. Work is proportional to the
completed task's direct fanout, not the full queue, while the ready-index lookup
avoids scanning tasks waiting on that fanout. Large fanout still costs database
work at completion, and scope-blocked ready tasks still require bounded scans.
Restart each pass at zero because an earlier task can become ready during a pass.

V8 adds an independently versioned, opt-in durable cursor policy. Explicitly
activate it with `enable_fair_dispatch()` and use `claim_fair()` to share bounded
sweep progress across controllers and restarts. Its fixed enqueue ceiling
prevents continuous later arrivals from extending a sweep; its cursor, active
scopes and lease commit together. Legacy dispatch methods remain unchanged.
Read [agent-fair-dispatch.md](agent-fair-dispatch.md) for the conditional logical
fairness bound, schema-5 migration, exact deferral fields and recovery limits.
There is no wall-time, worker-share or distributed fairness guarantee.

Failed jobs intentionally block their dependents. After retaining the previous
receipt and deciding the failure is retryable, a trusted controller can call
`retry_failed(task, expected_token, expected_result_digest, reconciliation_digest,
request_id="unique-controller-request")`. This requeues the **same immutable
action**, increases its fence and records both the old receipt and reconciliation
object in the event history. A changed source, command, toolchain or policy needs
a new task ID. Successful jobs cannot be reopened this way because downstream
work may already depend on their accepted result.

The request ID and payload digest commit atomically with the retry. Identical
replay returns the historical retry token, even after a later attempt has
started, without changing current authority. Rebinding the ID to another payload
is an error. Receipts from earlier attempts remain in CAS and the event history;
no failed evidence or worker directory is deleted. Before retry, V5 verifies both
the prior failed receipt and every artifact referenced by that receipt; retaining
only a receipt with missing failure logs is insufficient. The controller is responsible
for bounded retry policy and for handling deterministic failures rather than
repeatedly dispatching them.

## Queue schema and upgrade

V6 uses queue schema 4. Opening a schema 1 database first derives schema 2:
it derives enqueue order from existing durable events, normalizes scope rows,
adds the bounded dispatch indexes and creates the idempotent-request table.
The schema 2→3 step validates existing scopes and populates the active-scope
index from every running attempt, including expired attempts. The 3→4 step
checks referential integrity, derives pending-dependency counts once from retained
parent states and builds a partial ready index. Active or terminal jobs with an
unfinished parent refuse migration. All steps occur in one transaction. Normal
schema 4 opens do not scan or rebuild the active-scope or dependency-count data.
Existing task packets, fences, leases, results and events are retained. A missing
enqueue event or invalid persisted scope rejects and rolls back migration.
An existing database with a missing authority table or persisted clock floor
rejects ordinary open. Silently creating empty dependencies can release children
before their parents succeed; missing scope data can release held ownership;
forgotten request IDs allow replay rebinding; a reset clock floor can revive an
expired lease. V6 checks every schema's required authority tables before changes.
A missing schema-4 pending-count column also refuses. Ordinary opening does not
detect arbitrary row corruption: recover a stopped controller from retained,
independently checked authority and evidence. V5 resource
names have a 4,096-byte UTF-8 and 128-component limit; a preexisting
scope exceeding these limits requires explicit controller reconciliation and a
reviewed migration, never silent truncation. A newer unknown schema rejects
before creating tables. V5 code rejects schema 4; downgrade is unsupported.

Stop controllers, preserve a consistent SQLite backup (including WAL state via
the SQLite backup API), then run the new version and its migration check before
resuming. Do not copy only the main database file while a WAL database is active.
SQLite still has one concurrent writer: parsing packets, storing CAS bytes and
executing work stay outside these short authority transactions. Multi-host
scheduling requires the separate C11 replicated authority implementation and
failure qualification.

For a consistent backup created by the SQLite backup API, rollback means stopping
all controllers, restoring the entire known database state, and reconciling any
attempts or external work created after that backup before admitting new claims.
Changing only the schema number is not a downgrade or a safe authority recovery.

`PYTHONPATH=tools python3 tools/bench_agent_dispatch.py` measures dispatch against
real queue fixtures at several active-scope counts. The same script can run
against an archived implementation by setting `PYTHONPATH` to its `tools`
directory. It checks FIFO leases and fenced completion before reporting timings,
and reports Python allocation separately. This benchmark excludes fixture setup
and completion time; it is not an end-to-end throughput or distributed scaling
measurement.

`python3 tools/bench_agent_ready.py` compares the original and indexed dispatch
policies against identical queue snapshots with blocked dependency prefixes. It
requires the same first eligible task and retains all blocked states. Setup and
SQLite backup time are excluded; SQLite virtual-machine instruction counts are
measured in separate passes so instrumentation does not distort wall time.

## Evidence and completion

The data store uses atomic creation, reads back exact hashes, rejects corrupt,
oversized, symlink and nonregular objects, and does not silently replace a
corrupted object. Failed temporary writes are cleaned up; every successful POSIX
put synchronizes the directory entry, including concurrent duplicate puts. These
are tested API behaviors under the filesystem's durability contract, not a
power-loss or storage-device qualification. A successful receipt binds the exact action and source
snapshot, zero exit status, complete checks and available evidence objects.
Identical completion retries return the same receipt without a second event.
Changed completion receipts and old fencing tokens reject. Completion snapshots
the receipt before CAS I/O and uses that same value for the database transition.
Mutating the caller's receipt during storage cannot make a failed receipt count as
a successful prerequisite.

The caller is responsible for truthful evidence. A digest is neither a signature
nor a proof that a command ran. `checks_complete` is supplied by a trusted
supervisor, never by candidate output. The local append-only API does not prevent
a database administrator from rewriting history. The store is for data; it does
not authorize loading native objects or running binaries. Source changes require
independent review and whole-candidate checks before any Git integration.

V5 adds `put_stream(chunks)` for bounded byte-chunk producers and `verify(digest)`
for SHA-256 validation without loading a whole blob. The stream accepts at most
16 MiB and 65,536 chunks, counting empty chunks; interrupted or oversized input
cleans its staging file. CLI `put` uses 64 KiB reads. Queue evidence retention
checks use `verify` outside the transaction. `get` remains available when the
caller actually needs the bytes. A duplicate Linux `put(data)` rechecks and
syncs the existing object without staging another copy; it still rejects
corruption. Other hosts use the original staged publication path; a read-only
file-descriptor fsync fast path requires separate platform qualification.

`store.verify` can serve as the artifact verifier for `assess_evidence`:

```python
decision = assess_evidence(requirements, envelopes, now_ms=now,
                           authenticate=trusted_attestation_verifier,
                           verify_artifact=store.verify)
```

This establishes retained byte integrity at the check, not authenticated
execution, a retention lease or atomic publication. The authenticator and
publication policy remain independent trusted components.

## Conflict planning

`plan_batches` partitions changes deterministically using read/write conflicts.
Shared reads can proceed together. Writes that overlap another read/write
resource conflict. Directory scopes overlap descendants; unrelated path names
do not. Different namespaces are conservatively conflicting because this
prototype cannot prove a mapping between symbols, paths and contracts. Incomplete
analysis gets a singleton batch. This is a stable greedy partition, not a claim
of maximal packing or a semantic merge algorithm. The implementation indexes
the accumulated read and write resources per batch instead of comparing every
new change with every existing peer. V5 represents path coverage with a
component trie and prunes redundant descendants when an ancestor is added;
it no longer materializes every full ancestor string. Resources are limited to
4,096 UTF-8 bytes and 128 components, including the namespace prefix in the byte
budget. Planning takes
an owned JSON-value copy before validation and uses the same 4 MiB canonical
message budget as the CLI; later caller mutation cannot change its resource
sets. This removes
quadratic pair comparisons for a single disjoint batch; many mutually
conflicting batches can still require quadratic placement attempts. Seeded
differential tests compare every returned batch against the original pairwise
reference algorithm, including wildcard and cross-namespace cases.

The `analysis_complete` flag is an advisory input, not an attestation. Before
automatic integration, a trusted compiler must derive dependencies and invariant
domains, a trusted remapper must bind edits to the current source/node index, and
the exact combined candidate must pass admission. Neither syntactic disjointness
nor an agent's read-set declaration proves semantic independence. Names are
case-sensitive POSIX labels, not canonical filesystem objects. Case-insensitive
mounts, Windows drive-relative paths/alternate streams, reserved names, Unicode
normalization aliases, symlinks and hard-link aliases require a trusted platform
resolver before these labels can justify filesystem exclusivity. The tested
Linux-local scope must not be promoted to portable path isolation.

## Minimal API example

```python
from agent_pipeline import ContentStore, WorkQueue

# `action` is a validated fluxr-action-v1 object with real input identities.
store = ContentStore(".fluxr-agent/objects")
queue = WorkQueue(".fluxr-agent/queue.sqlite", store)
try:
    queue.enqueue("compile-001", action)
    lease = queue.claim("compile-001", "trusted-runner-1", ttl_seconds=300)
    # Execute separately in the authorized runner. Retain all required evidence.
    # Complete with a trusted receipt only after every required check finishes.
finally:
    queue.close()
```

CLI commands `key`, `plan`, `put`, `enqueue`, `claim`, `claim-next`, `renew`,
`complete`, `reconcile`, `retry-failed`, `inspect` and `events` expose the same primitives. Input JSON is
bounded. There is intentionally no command that executes receipt-supplied code
or advances a repository branch.

## Research and implementation decisions

The separation of scheduling from rebuild validity follows the design question
in [Build Systems à la Carte](https://www.microsoft.com/en-us/research/publication/build-systems-la-carte/).
Fencing addresses the paused-client/expired-lease failure described in
[Martin Kleppmann's lock analysis](https://martin.kleppmann.com/2016/02/08/how-to-do-distributed-locking.html).
Conflict sets are a conservative engineering boundary; the database model in
[Coordination Avoidance in Database Systems](https://arxiv.org/abs/1402.2237)
does not establish that arbitrary code changes can merge safely without
coordination. These are design influences, not proofs of this implementation.


The queue transaction boundary follows SQLite's documented [transaction
semantics](https://www.sqlite.org/lang_transaction.html): a failed commit can
leave a transaction active, so the caller explicitly rolls it back before
returning failure. SQLite's [WAL documentation](https://www.sqlite.org/wal.html)
explains the single-writer and same-host constraints. These properties justify a
local prototype, not a multi-region coordination service.

V5's namespace/name equality, `IN` and range probes follow SQLite's documented
[index term rules](https://www.sqlite.org/optoverview.html#where_clause_analysis).
The active-scope table is a transactional materialized index of held scopes,
not a second authority or a best-effort controller cache. The canonical JSON
byte preflight preserves the documented [Python JSON escaping rules](https://docs.python.org/3/library/json.html#json.dump),
with regressions for exact byte budgets, control characters and multibyte Unicode.
