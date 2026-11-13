# Persisted fair ready dispatch, profile 1

## Purpose and compatibility

`fluxr-fair-ready-v1` adds a shared bounded sweep to the trusted local work queue.
Controllers can restart, lose a response or hand work to another controller
without losing progress past an unrelated busy scope. Later arrivals cannot
extend the current sweep. Selection remains advisory until the same transaction
commits its lease fence and active scopes. A successful job receipt remains
separate from independent review, semantic acceptance and publication authority.

The opt-in `WorkQueue.enable_fair_dispatch()` operation migrates database schema
4 to schema 5. New databases still default to schema 4. `claim`, `claim_next` and
`claim_ready` keep their existing arguments, result shapes and decisions. Their
caller-managed cursors have not acquired new meanings. Existing action keys,
packets, leases, task fences, dependency counts and result receipts are unchanged.

```python
queue.enable_fair_dispatch()  # Trusted controller, explicit activation.
window = queue.claim_fair("worker-a", ttl_seconds=300, scan_limit=128)
if window.lease is not None:
    # Execute reviewed work outside every queue transaction, then retain evidence.
    ...
```

The CLI exposes the same operations as `enable-fair` and
`claim-fair OWNER --scan-limit N --ttl SECONDS`. The serialized result names
`fluxr-fair-dispatch-result-v1` and `fluxr-fair-ready-v1`. Callers observe the
epoch, fixed upper enqueue sequence, committed cursor, examined count, separate
scope/dependency deferrals and sweep completion. Callers cannot supply or reset
the durable cursor. Nothing in these fields is permission to execute arbitrary
code, omit validation or publish a candidate.

## Algorithm and exact progress claim

At the start of a nonempty sweep, the controller captures the largest enqueue
sequence of a currently dependency-ready queued task. That value is the sweep's
fixed ceiling. Each call retrieves at most `scan_limit + 1` control rows from the
existing partial ready index, with a strict lower cursor and an inclusive upper
ceiling. The extra row detects whether more candidates remain; at most
`scan_limit` candidates are examined. The range and every relevant lease,
dependency and active-scope check happen under one `BEGIN IMMEDIATE` transaction.

For each candidate, the controller checks the authoritative parent states and
the existing exact/ancestor/descendant scope conflict rules. At most one lease is
granted. The cursor advances over claimed and deferred candidates alike. Cursor
progress, the claim event, task fence and active scopes commit together. When no
candidate remains below the ceiling, the cursor reaches the ceiling. The next
call starts another sweep from zero with a new ceiling. Empty polls without any
dependency-ready candidate do not consume epochs. Epoch exhaustion rejects;
logical counters never wrap or reset.

The ceiling bounds a range of immutable enqueue identities. It does **not**
freeze eligibility or materialize a task snapshot. A parent may succeed while a
sweep is in progress. An earlier child that becomes ready behind the cursor is
considered in the next sweep. A retried or reconciled earlier task also waits
until the next sweep; repeated retry of a low-sequence task cannot keep later
candidates out of the current range. Later enqueues are outside that range.

Let `M` be the finite number of existing tasks at or below a sweep's ceiling,
including tasks that could become ready or be retried. Within a sweep each
candidate's immutable sequence can be examined at most once. Each successfully
committed nonempty call advances past at least one candidate, or a call with no
remaining candidates closes the sweep. Consequently, a sweep closes within
`M + 1` successfully committed fair-dispatch calls. This is a conservative
logical bound; the partial index usually skips most of those tasks.

A task that stays queued, dependency-ready and scope-unblocked from before its
turn is granted when the sweep reaches it. If its sequence is already behind
the cursor or beyond the ceiling, it is reconsidered in the next sweep. The
bound then includes the remaining current sweep and the finite prefix captured
for the next one. These claims assume trusted controllers continue making
successful `claim_fair` calls and no other policy repeatedly removes the task or
acquires its scope first. They are candidate-examination bounds, not bounds on
wall time, worker shares, service duration, model latency or accepted
requirements. Persistently held scopes, unfinished dependencies, unavailable
controllers, database lock contention and storage failure remain blockers.

`lease is None` and `sweep_complete` do not prove queue idleness. The result
does not contain a `queue_empty` assertion. Missing dependency information never
becomes a passing job. Readiness counts remain an index; actual parent states
are independently checked before a grant.

## Cost and concurrency boundaries

The queue retains three additional signed 64-bit counters. Each call keeps at
most 4,097 selected control rows (`scan_limit` is 1–4,096), never full action
packets. The initial ceiling and candidate range use the existing partial index.
Work is one bounded range lookup plus dependency and scope checks for the
examined candidates, and at most one claim. The existing scope-list size and
dependency fan-out costs remain; this change does not bound total queue storage
or remove the need for admission/backpressure outside the queue.

All design, model requests, compilation, tests, fixture computation, evidence
creation and artifact verification remain outside the writer transaction.
`complete` retains its existing outside-transaction CAS write and inside-
transaction state/reference transition. SQLite still admits only one writer.
This policy does not introduce sharding, authentication, distributed fencing,
network-filesystem support or fairness in acquiring the SQLite writer lock.

## Migration, failure, recovery and rollback

Activation inserts the counters and changes the schema in one transaction.
Interrupted migration either leaves the original schema 4 or the complete
schema 5. Opening schema 5 requires every counter, valid signed ranges,
consistent cursor/ceiling relations and referenced enqueue events. Missing or
invalid state rejects before ordinary schema initialization; it is never
recreated as zero. Re-activation is idempotent after validation. Older
controllers reject schema 5 before touching its tables.

Failure while writing the cursor or committing a claim rolls back both cursor
and claim. A process exit before commit has no accepted claim; a process exit
after commit can lose the response while leaving the lease and cursor durable.
The replacement controller advances the shared sweep. It must not assume the
lost-response worker stopped. Lease expiry retains active scopes until explicit
reconciliation, and stale fences remain rejected. The persisted clock floor
prevents a local backward clock step from reviving an expired lease. Sweep
progress uses logical sequence values rather than wall time.

Save an offline coherent pre-activation database backup before migrating.
Stopping use of `claim_fair` permits the legacy policies in this version to
continue on schema 5. No in-place database downgrade is provided. Returning to
older controller code requires a separately reconciled restore of a coherent
schema-4 checkpoint; do not rewrite the schema number or discard the counters.
A complete database/backup rollback cannot be detected by values inside that
same rolled-back database. External authority epochs and recovery protocols
remain an explicit deployment obligation; this local implementation makes no
rollback-safe distributed authority claim.

## Validation and measurements

`tests/test_agent_fair.py` is a required full-verification suite. It exercises
finite-sweep oracle agreement, hot scopes with continuing arrivals, retry and
dependency readiness behind the cursor, restart without a caller cursor,
independent processes, strict bounded scans without packet reads, injected
cursor/commit/migration failure, real process exits before/after commit,
clock rollback, corrupted state and epoch exhaustion. All existing queue,
dispatch, dependency-ready, store, verification and conflict tests remain
required.

`tools/bench_agent_fair.py` runs reviewed deterministic SHA-256 fixtures outside
queue transactions. It compares one shared CPU affinity budget with one CPU per
worker, keeping the fixture count fixed. All trials, worker exits, calls,
examined/deferred counts, queue writer waits and hold times, fixture completion
times and worker peak RSS are retained. Empirical p50/p95/p99 include sample
counts; they have no independent-workload confidence claim. Neighboring work and
filesystem contention are uncontrolled. These synthetic jobs are not coding
agents, and their receipts are not accepted user requirements. Tokens and source
publication durations are not measured by this fixture harness.
The raw records retain the exact small receipt and artifact UTF-8 bytes as well
as their digests, so cleanup of temporary benchmark databases does not leave
unresolvable evidence-object references. Result collection reads occur after
the claim-through-completion timer and remain in overall campaign wall time.

Run, from a reviewed checkout with writable temporary storage outside it:

```sh
python3 -m unittest discover -s tests -p 'test_agent_fair.py' -v
python3 tools/bench_agent_fair.py --workers 1 2 4 --jobs 96 --repeats 3 --temp-root "$TMPDIR"
```

This implementation is scoped to C02-AC08/11/12/13/14/15, C03-AC11/15,
C10-AC02/05/15 and C11 recovery obligations; it does not complete those tasks.
Fairness under adversarial task priorities, production
backpressure, authority rollback, multi-region partitions and end-to-end
autonomous-agent scale remain open.

## Transaction and deployment references

SQLite's [isolation specification](https://sqlite.org/isolation.html)
describes visibility between
connections, one serialized writer and `BEGIN IMMEDIATE`; the implementation
uses these mechanisms for the joint transition, not as a distributed theorem.
The [transaction specification](https://sqlite.org/lang_transaction.html)
sections 2.2–2.3 and 3
documents explicit write transactions and error/rollback handling. The
[WAL documentation](https://sqlite.org/wal.html), selected overview and sections
2–2.3, describes local-host shared memory, the single writer and checkpoint
costs. The sweep bound
above is a direct argument over this implementation and its immutable enqueue
order; the external sources do not prove this application-level scheduler.
