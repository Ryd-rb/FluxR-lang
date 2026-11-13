"""A single-host fenced work queue with short SQLite transactions.

Only trusted controllers may call this API. Workers use separate worktrees;
this queue neither executes code nor grants permission to integrate changes.
Job success is not independent review, semantic equivalence, or release
acceptance. SQLite on a shared network filesystem is expressly unsupported.
"""
from __future__ import annotations

from contextlib import contextmanager
from dataclasses import asdict, dataclass
import json
from pathlib import Path
import sqlite3
import time

from .identity import action_key, canonical_bytes, content_digest, load_json_bytes, require_digest
from .planning import resource
from .store import ContentStore


MAX_COUNTER = 2**63 - 1


def _identifier(value, name):
    if (type(value) is not str or not value or len(value) > 256
            or any(ord(c) < 32 or ord(c) == 127 for c in value)):
        raise ValueError(f"invalid {name}")
    value.encode("utf-8", errors="strict")
    return value


def _counter(value, name, minimum=0):
    if type(value) is not int or not minimum <= value <= MAX_COUNTER:
        raise ValueError(f"{name} must be an integer in [{minimum}, {MAX_COUNTER}]")
    return value


def _next_token(value):
    _counter(value, "fencing token")
    if value == MAX_COUNTER:
        raise ValueError("fencing token exhausted; authority must not wrap or reset")
    return value + 1


@dataclass(frozen=True)
class Lease:
    task: str
    owner: str
    token: int
    expires_ms: int
    action_key: str
    source_snapshot: str

    def __post_init__(self):
        _identifier(self.task, "task id")
        _identifier(self.owner, "owner")
        _counter(self.token, "fencing token", 1)
        _counter(self.expires_ms, "lease deadline")
        require_digest(self.action_key)
        require_digest(self.source_snapshot)

    def to_dict(self):
        return asdict(self)


@dataclass(frozen=True)
class DispatchResult:
    """One bounded dispatch window. A cursor is a hint, never an authority.

    Continue with next_after while it is not None, then restart at zero. Jobs
    blocked on dependencies or scopes must be reconsidered in the next pass.
    A missing lease alone does not mean that all queued work is blocked.
    """
    lease: Lease | None
    next_after: int | None
    examined: int

    def to_dict(self):
        return asdict(self)


@dataclass(frozen=True)
class FairDispatchResult:
    """One committed portion of the shared, bounded dependency-ready sweep.

    The sweep has a fixed enqueue-sequence ceiling, not frozen eligibility.
    ``sweep_complete`` never means queue idleness: scopes and dependencies may
    still block work, and new/previously visited jobs wait for the next sweep.
    The cursor is observation only; callers cannot supply or reset it.
    """
    lease: Lease | None
    epoch: int
    upper_sequence: int
    after_sequence: int
    examined: int
    deferred_dependencies: int
    deferred_scopes: int
    sweep_complete: bool

    def to_dict(self):
        return {"schema": "fluxr-fair-dispatch-result-v1", "policy": "fluxr-fair-ready-v1", **asdict(self)}


class WorkQueue:
    def __init__(self, database, store: ContentStore, clock_ms=None):
        path = Path(database)
        if path.is_symlink():
            raise ValueError("database must not be a symlink")
        path.parent.mkdir(parents=True, exist_ok=True)
        self.store = store
        self.clock_ms = clock_ms if clock_ms is not None else (lambda: time.time_ns() // 1_000_000)
        self.connection = sqlite3.connect(path, timeout=10, isolation_level=None)
        self.connection.row_factory = sqlite3.Row
        try:
            # Reject a newer database before applying our schema. Initialization
            # and the one-way v1 migration are serialized with other openers.
            self._check_schema()
            self.connection.execute("PRAGMA foreign_keys=ON")
            self.connection.execute("PRAGMA journal_mode=WAL")
            self.connection.execute("PRAGMA synchronous=FULL")
            self.connection.execute("BEGIN IMMEDIATE")
            self._check_schema()
            statements = [
                "CREATE TABLE IF NOT EXISTS metadata (key TEXT PRIMARY KEY, value INTEGER NOT NULL)",
                "INSERT OR IGNORE INTO metadata VALUES ('schema', 1)",
                "INSERT OR IGNORE INTO metadata VALUES ('clock_floor_ms', 0)",
                """CREATE TABLE IF NOT EXISTS tasks (
                id TEXT PRIMARY KEY, action_key TEXT NOT NULL, source_snapshot TEXT NOT NULL,
                packet TEXT NOT NULL, state TEXT NOT NULL CHECK(state IN ('queued','running','succeeded','failed')),
                owner TEXT, token INTEGER NOT NULL DEFAULT 0, expires_ms INTEGER,
                result_digest TEXT
                )""",
                """CREATE TABLE IF NOT EXISTS dependencies (
                task TEXT NOT NULL REFERENCES tasks(id), dependency TEXT NOT NULL REFERENCES tasks(id),
                PRIMARY KEY(task, dependency)
                )""",
                """CREATE TABLE IF NOT EXISTS events (
                sequence INTEGER PRIMARY KEY AUTOINCREMENT, at_ms INTEGER NOT NULL,
                task TEXT NOT NULL, token INTEGER NOT NULL, kind TEXT NOT NULL, details TEXT NOT NULL
                )""",
                """CREATE TABLE IF NOT EXISTS scopes (
                    task TEXT NOT NULL REFERENCES tasks(id), scope TEXT NOT NULL,
                    PRIMARY KEY(task, scope))""",
                """CREATE TABLE IF NOT EXISTS requests (
                    id TEXT PRIMARY KEY, payload_digest TEXT NOT NULL, resulting_token INTEGER NOT NULL)""",
                """CREATE TABLE IF NOT EXISTS active_scopes (
                    task TEXT NOT NULL REFERENCES tasks(id), scope TEXT NOT NULL,
                    namespace TEXT NOT NULL, name TEXT NOT NULL,
                    PRIMARY KEY(task, scope))""",
                "CREATE INDEX IF NOT EXISTS active_scopes_by_name ON active_scopes(namespace, name)",
                "CREATE INDEX IF NOT EXISTS tasks_by_state ON tasks(state, id)",
                "CREATE INDEX IF NOT EXISTS events_by_kind_sequence ON events(kind, sequence, task)",
                "CREATE INDEX IF NOT EXISTS events_by_task_sequence ON events(task, sequence)",
                "CREATE INDEX IF NOT EXISTS dependencies_by_dependency ON dependencies(dependency, task)",
            ]
            for statement in statements:
                self.connection.execute(statement)
            if self.connection.execute("SELECT value FROM metadata WHERE key='schema'").fetchone()[0] == 1:
                self.connection.execute("ALTER TABLE tasks ADD COLUMN enqueue_sequence INTEGER")
                self.connection.execute("""UPDATE tasks SET enqueue_sequence=(
                    SELECT min(sequence) FROM events e WHERE e.task=tasks.id AND e.kind='enqueued')""")
                if self.connection.execute("SELECT 1 FROM tasks WHERE enqueue_sequence IS NULL LIMIT 1").fetchone():
                    raise ValueError("missing enqueue event in v1 queue migration")
                for row in self.connection.execute("SELECT id, packet FROM tasks"):
                    scopes = json.loads(row["packet"])["exclusive_scopes"]
                    if type(scopes) is not list or len(scopes) != len(set(scopes)):
                        raise ValueError("invalid scopes in v1 queue migration")
                    for scope in scopes:
                        resource(scope)
                        self.connection.execute("INSERT INTO scopes VALUES (?,?)", (row["id"], scope))
                self.connection.execute("UPDATE metadata SET value=2 WHERE key='schema'")
            if self.connection.execute("SELECT value FROM metadata WHERE key='schema'").fetchone()[0] == 2:
                # Preserve scopes of expired attempts too. Expiration does not
                # prove the old worker stopped; explicit reconciliation does.
                for row in self.connection.execute("""SELECT s.task, s.scope, t.state FROM scopes s
                        JOIN tasks t ON t.id=s.task"""):
                    resource(row["scope"])
                    if row["state"] != "running":
                        continue
                    namespace, name = self._scope_parts(row["scope"])
                    self.connection.execute("INSERT INTO active_scopes VALUES (?,?,?,?)",
                                            (row["task"], row["scope"], namespace, name))
                self.connection.execute("UPDATE metadata SET value=3 WHERE key='schema'")
            if self.connection.execute("SELECT value FROM metadata WHERE key='schema'").fetchone()[0] == 3:
                # Succeeded parents are terminal: retries cannot reopen them.
                # Keep a transactional pending count so a blocked DAG prefix
                # does not have to be visited by every dependency-ready poll.
                if self.connection.execute("PRAGMA foreign_key_check").fetchone():
                    raise ValueError("dangling queue authority references; authority recovery required")
                self.connection.execute("""ALTER TABLE tasks ADD COLUMN pending_dependencies
                    INTEGER NOT NULL DEFAULT 0 CHECK(pending_dependencies>=0)""")
                self.connection.execute("""UPDATE tasks SET pending_dependencies=(
                    SELECT count(*) FROM dependencies d JOIN tasks parent ON parent.id=d.dependency
                    WHERE d.task=tasks.id AND parent.state!='succeeded')""")
                if self.connection.execute("""SELECT 1 FROM tasks
                        WHERE state!='queued' AND pending_dependencies!=0 LIMIT 1""").fetchone():
                    raise ValueError("active or terminal task has unfinished dependencies; authority recovery required")
                self.connection.execute("UPDATE metadata SET value=4 WHERE key='schema'")
            self.connection.execute("CREATE INDEX IF NOT EXISTS tasks_by_state_sequence ON tasks(state, enqueue_sequence)")
            self.connection.execute("""CREATE INDEX IF NOT EXISTS tasks_dependency_ready_by_sequence
                ON tasks(enqueue_sequence) WHERE state='queued' AND pending_dependencies=0""")
            self.connection.execute("COMMIT")
        except BaseException:
            if self.connection.in_transaction:
                self.connection.execute("ROLLBACK")
            self.close()
            raise

    def _check_schema(self):
        tables = {row[0] for row in self.connection.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")}
        if not tables:
            return  # A genuinely empty database may initialize new authority.
        if "metadata" not in tables:
            raise ValueError("queue metadata table is missing; authority recovery required")
        row = self.connection.execute("SELECT value FROM metadata WHERE key='schema'").fetchone()
        if row is None or type(row[0]) is not int or row[0] not in (1, 2, 3, 4, 5):
            raise ValueError("unsupported queue schema")
        version = row[0]
        required = {"metadata", "tasks", "dependencies", "events"}
        if version >= 2:
            required.update(("scopes", "requests"))
        if version >= 3:
            required.add("active_scopes")
        missing = required - tables
        if missing:
            # IF NOT EXISTS is only initialization/migration, never recovery.
            # Recreating missing dependencies can release a child before its
            # parent succeeds; recreating scopes releases held work; forgetting
            # requests permits a previously consumed id to be rebound.
            raise ValueError("queue authority tables are missing: " + ", ".join(sorted(missing))
                             + "; authority recovery required")
        floor = self.connection.execute("SELECT value FROM metadata WHERE key='clock_floor_ms'").fetchone()
        if floor is None:
            raise ValueError("queue clock floor is missing; authority recovery required")
        _counter(floor[0], "persisted clock floor")
        if version >= 4:
            columns = {row[1] for row in self.connection.execute("PRAGMA table_info(tasks)")}
            if "pending_dependencies" not in columns:
                raise ValueError("queue dependency count is missing; authority recovery required")
        if version >= 5:
            self._fair_state()

    def _fair_state(self):
        rows = dict(self.connection.execute("""SELECT key,value FROM metadata
            WHERE key IN ('fair_epoch','fair_after','fair_upper')"""))
        if set(rows) != {"fair_epoch", "fair_after", "fair_upper"}:
            raise ValueError("queue fair-dispatch state is missing; authority recovery required")
        epoch, after, upper = (rows[name] for name in ("fair_epoch", "fair_after", "fair_upper"))
        for value in (epoch, after, upper):
            _counter(value, "persisted fair-dispatch counter")
        if after > upper or (epoch == 0 and (after != 0 or upper != 0)) or (epoch != 0 and upper == 0):
            raise ValueError("inconsistent fair-dispatch state; authority recovery required")
        # These are event identities, not free integers. Detect truncated or
        # partial authority restores before a corrupt cursor can skip work.
        for sequence in {after, upper} - {0}:
            row = self.connection.execute("SELECT kind FROM events WHERE sequence=?", (sequence,)).fetchone()
            if row is None or row[0] != "enqueued":
                raise ValueError("missing fair-dispatch enqueue event; authority recovery required")
        return epoch, after, upper

    def enable_fair_dispatch(self):
        """Explicitly migrate schema 4 to 5 without changing legacy policies.

        A schema-5 queue is refused by older controllers. This migration has no
        downgrade operation and never reconstructs missing schema-5 state.
        Keep an offline, coherent database backup before activating the policy;
        restoring a live authority database needs external epoch reconciliation.
        """
        with self._transaction():
            self._check_schema()
            version = self.connection.execute("SELECT value FROM metadata WHERE key='schema'").fetchone()[0]
            if version == 5:
                return
            if version != 4:
                raise ValueError("fair dispatch requires a schema-4 queue")
            if self.connection.execute("SELECT 1 FROM metadata WHERE key GLOB 'fair_*' LIMIT 1").fetchone():
                raise ValueError("unexpected fair-dispatch migration state; authority recovery required")
            self.connection.executemany("INSERT INTO metadata VALUES (?,0)",
                                        [(name,) for name in ("fair_epoch", "fair_after", "fair_upper")])
            self.connection.execute("UPDATE metadata SET value=5 WHERE key='schema'")

    def close(self):
        self.connection.close()

    @contextmanager
    def _transaction(self):
        self.connection.execute("BEGIN IMMEDIATE")
        saved = False
        try:
            # Persist observed time even when the requested mutation is
            # rejected. Rolling this floor back could revive an expired lease
            # after a wall-clock regression. The operation itself is atomic in
            # a savepoint; no I/O or worker execution happens under this lock.
            self._observe_clock()
            self.connection.execute("SAVEPOINT operation")
            saved = True
            yield
        except BaseException:
            try:
                if saved and self.connection.in_transaction:
                    self.connection.execute("ROLLBACK TO operation")
                    self.connection.execute("RELEASE operation")
                    self.connection.execute("COMMIT")
            finally:
                if self.connection.in_transaction:
                    self.connection.execute("ROLLBACK")
            raise
        else:
            try:
                self.connection.execute("RELEASE operation")
                self.connection.execute("COMMIT")
            finally:
                # A failed COMMIT can leave a transaction active. Never retain
                # the writer lock after a call has returned an error.
                if self.connection.in_transaction:
                    self.connection.execute("ROLLBACK")

    def _observe_clock(self):
        observed = self.clock_ms()
        _counter(observed, "clock milliseconds")
        floor = self.connection.execute("SELECT value FROM metadata WHERE key='clock_floor_ms'").fetchone()[0]
        now = max(floor, observed)
        self.connection.execute("UPDATE metadata SET value=? WHERE key='clock_floor_ms'", (now,))
        return now

    def _now(self):
        return self.connection.execute("SELECT value FROM metadata WHERE key='clock_floor_ms'").fetchone()[0]

    def _event(self, now, task, token, kind, details):
        return self.connection.execute("INSERT INTO events(at_ms,task,token,kind,details) VALUES (?,?,?,?,?)",
                                       (now, task, token, kind, canonical_bytes(details).decode())).lastrowid

    def _row(self, task, *, include_packet=False):
        # Immutable packets can be several MiB. State transitions and dispatch
        # need only control metadata, especially while the writer lock is held.
        columns = ("*" if include_packet else
                   "id,action_key,source_snapshot,state,owner,token,expires_ms,result_digest,enqueue_sequence")
        row = self.connection.execute(f"SELECT {columns} FROM tasks WHERE id=?", (task,)).fetchone()
        if row is None:
            raise ValueError("unknown task")
        return row

    def enqueue(self, task: str, spec: dict, *, dependencies=(), exclusive_scopes=()):
        _identifier(task, "task id")
        # Own one immutable-value snapshot throughout validation, hashing and
        # storage. Caller-owned dictionaries may be reused by other threads.
        spec = load_json_bytes(canonical_bytes(spec))
        key = action_key(spec)
        dependencies = list(dependencies)
        exclusive_scopes = list(exclusive_scopes)
        if any(not isinstance(d, str) or not d or d == task for d in dependencies):
            raise ValueError("invalid task dependency")
        if len(dependencies) != len(set(dependencies)) or len(exclusive_scopes) != len(set(exclusive_scopes)):
            raise ValueError("duplicate dependency or scope")
        for name in exclusive_scopes:
            resource(name)
        packet = canonical_bytes({"action": spec, "dependencies": sorted(dependencies),
                                  "exclusive_scopes": sorted(exclusive_scopes)}).decode()
        with self._transaction():
            now = self._now()
            existing = self.connection.execute("SELECT packet FROM tasks WHERE id=?", (task,)).fetchone()
            if existing:
                if existing[0] != packet:
                    raise ValueError("task id already binds different immutable inputs")
                return key
            # Dependencies must already exist. Immutable insertion order makes
            # cycles impossible without allowing later dependency rewrites.
            pending = sum(self._row(dependency)["state"] != "succeeded" for dependency in dependencies)
            self.connection.execute("""INSERT INTO tasks(id,action_key,source_snapshot,packet,state,pending_dependencies)
                VALUES (?,?,?,?,'queued',?)""", (task, key, spec["source_snapshot"], packet, pending))
            self.connection.executemany("INSERT INTO dependencies VALUES (?,?)", [(task, d) for d in dependencies])
            self.connection.executemany("INSERT INTO scopes VALUES (?,?)", [(task, s) for s in exclusive_scopes])
            sequence = self._event(now, task, 0, "enqueued", {"action_key": key})
            self.connection.execute("UPDATE tasks SET enqueue_sequence=? WHERE id=?", (sequence, task))
        return key

    @staticmethod
    def _ttl(seconds):
        if type(seconds) is not int or not 1 <= seconds <= 86400:
            raise ValueError("lease TTL must be integer seconds in [1, 86400]")
        return seconds * 1000

    def claim(self, task: str, owner: str, ttl_seconds: int = 300) -> Lease:
        ttl = self._ttl(ttl_seconds)
        _identifier(task, "task id")
        _identifier(owner, "owner")
        with self._transaction():
            now = self._now()
            row = self._row(task)
            if row["state"] != "queued":
                raise ValueError("task is not queued; expired work needs explicit reconciliation")
            missing = self.connection.execute("""SELECT d.dependency FROM dependencies d
                JOIN tasks t ON t.id=d.dependency WHERE d.task=? AND t.state!='succeeded'""", (task,)).fetchall()
            if missing:
                raise ValueError("dependencies have not succeeded")
            scopes = self._scopes(task)
            if scopes and self._active_scope_conflict(scopes):
                raise ValueError("exclusive scope held by active or unreconciled work")
            return self._claim_row(row, owner, ttl, now, scopes)

    def _scopes(self, task):
        return [r[0] for r in self.connection.execute("SELECT scope FROM scopes WHERE task=?", (task,))]

    @staticmethod
    def _scope_parts(scope):
        return ("*", "") if scope == "*" else tuple(scope.split(":", 1))

    def _active_scope_conflict(self, scopes):
        """Indexed exact/ancestor/descendant probes, independent of active count.

        Different resource namespaces still conservatively overlap. The table
        contains running attempts only and changes atomically with their fence;
        no controller-local index can drift after another process commits.
        """
        for scope in scopes:
            namespace, name = self._scope_parts(scope)
            if namespace == "*":
                if self.connection.execute("SELECT 1 FROM active_scopes LIMIT 1").fetchone():
                    return True
                continue
            other = tuple(n for n in ("*", "path", "symbol", "contract") if n != namespace)
            if self.connection.execute("""SELECT 1 FROM active_scopes
                    WHERE namespace IN (?,?,?) LIMIT 1""", other).fetchone():
                return True
            if self.connection.execute("""SELECT 1 FROM active_scopes
                    WHERE namespace=? AND name=? LIMIT 1""", (namespace, name)).fetchone():
                return True
            if namespace == "path":
                # '/' and '0' are adjacent in SQLite BINARY collation. This
                # half-open range finds descendants without LIKE wildcards or
                # a full active-set scan, including '%' and '_' in file names.
                if self.connection.execute("""SELECT 1 FROM active_scopes
                        WHERE namespace='path' AND name>=? AND name<? LIMIT 1""",
                        (name + "/", name + "0")).fetchone():
                    return True
                ancestors = [name[:i] for i, char in enumerate(name) if char == "/"]
                if ancestors:
                    placeholders = ",".join("?" for _ in ancestors)
                    if self.connection.execute(f"""SELECT 1 FROM active_scopes
                            WHERE namespace='path' AND name IN ({placeholders}) LIMIT 1""",
                            ancestors).fetchone():
                        return True
        return False

    def _claim_row(self, row, owner, ttl, now, scopes):
        token = _next_token(row["token"])
        expiry = _counter(now + ttl, "lease deadline")
        self.connection.execute("UPDATE tasks SET state='running',owner=?,token=?,expires_ms=? WHERE id=?",
                                (owner, token, expiry, row["id"]))
        self.connection.executemany("INSERT INTO active_scopes VALUES (?,?,?,?)",
                                    [(row["id"], scope, *self._scope_parts(scope)) for scope in scopes])
        self._event(now, row["id"], token, "claimed", {"owner": owner, "expires_ms": expiry})
        return Lease(row["id"], owner, token, expiry, row["action_key"], row["source_snapshot"])

    def claim_next(self, owner: str, ttl_seconds: int = 300, *, after_sequence: int = 0,
                   scan_limit: int = 128) -> DispatchResult:
        """Atomically claim one eligible job from a bounded, FIFO-ordered window.

        Unlike list-then-claim, selection and fencing share a transaction.
        Dependency and scope conflicts are skipped, never treated as success.
        Pollers continue at next_after, resetting to zero when it is None;
        nothing about the cursor grants ownership or proves global idleness.
        """
        return self._dispatch(owner, ttl_seconds, after_sequence, scan_limit, ready_only=False)

    def claim_ready(self, owner: str, ttl_seconds: int = 300, *, after_sequence: int = 0,
                    scan_limit: int = 128) -> DispatchResult:
        """Claim from an indexed window of dependency-ready queued tasks.

        The legacy claim_next cursor counts every queued candidate. This opt-in
        route counts only candidates with no pending dependency, preserving FIFO
        within that set. Scope conflicts still need a bounded scan. Restart at
        zero after every pass: an earlier child may become ready during a pass.
        A missing lease or exhausted cursor never certifies global quiescence.
        """
        return self._dispatch(owner, ttl_seconds, after_sequence, scan_limit, ready_only=True)

    def claim_fair(self, owner: str, ttl_seconds: int = 300, *, scan_limit: int = 128) -> FairDispatchResult:
        """Share durable sweep progress across trusted controller processes.

        Selection, cursor progress, active scopes and the new fence commit in
        one short transaction. Each call examines at most ``scan_limit`` ready
        candidates and grants at most one lease. Eligibility is rechecked with
        the same dependency/scope rules as claim_ready. Continuous arrivals
        cannot extend a sweep's fixed ceiling. See docs/agent-fair-dispatch.md
        for the conditional candidate-examination bound and crash limitations.
        """
        _identifier(owner, "owner")
        if type(scan_limit) is not int or not 1 <= scan_limit <= 4096:
            raise ValueError("scan_limit must be an integer in [1, 4096]")
        ttl = self._ttl(ttl_seconds)
        with self._transaction():
            if self.connection.execute("SELECT value FROM metadata WHERE key='schema'").fetchone()[0] != 5:
                raise ValueError("enable_fair_dispatch must explicitly activate queue schema 5")
            epoch, after, upper = self._fair_state()
            if after == upper:
                last = self.connection.execute("""SELECT enqueue_sequence FROM tasks
                    INDEXED BY tasks_dependency_ready_by_sequence
                    WHERE state='queued' AND pending_dependencies=0
                    ORDER BY enqueue_sequence DESC LIMIT 1""").fetchone()
                if last is None:
                    return FairDispatchResult(None, epoch, upper, after, 0, 0, 0, True)
                if epoch == MAX_COUNTER:
                    raise ValueError("fair-dispatch epoch exhausted; authority must not wrap or reset")
                epoch += 1
                after, upper = 0, last[0]
            rows = self.connection.execute("""SELECT id,token,action_key,source_snapshot,enqueue_sequence
                FROM tasks INDEXED BY tasks_dependency_ready_by_sequence
                WHERE state='queued' AND pending_dependencies=0 AND enqueue_sequence>? AND enqueue_sequence<=?
                ORDER BY enqueue_sequence LIMIT ?""", (after, upper, scan_limit + 1)).fetchall()
            examined, deferred_dependencies, deferred_scopes = 0, 0, 0
            lease = None
            for row in rows[:scan_limit]:
                examined += 1
                after = row["enqueue_sequence"]
                blocked = self.connection.execute("""SELECT 1 FROM dependencies d
                    JOIN tasks t ON t.id=d.dependency
                    WHERE d.task=? AND t.state!='succeeded' LIMIT 1""", (row["id"],)).fetchone()
                if blocked:
                    deferred_dependencies += 1
                    continue
                scopes = self._scopes(row["id"])
                if scopes and self._active_scope_conflict(scopes):
                    deferred_scopes += 1
                    continue
                lease = self._claim_row(row, owner, ttl, self._now(), scopes)
                break
            complete = examined == len(rows)
            if complete:
                after = upper
            self.connection.executemany("UPDATE metadata SET value=? WHERE key=?",
                [(epoch, "fair_epoch"), (after, "fair_after"), (upper, "fair_upper")])
            return FairDispatchResult(lease, epoch, upper, after, examined,
                                      deferred_dependencies, deferred_scopes, complete)

    def _dispatch(self, owner, ttl_seconds, after_sequence, scan_limit, *, ready_only):
        _identifier(owner, "owner")
        _counter(after_sequence, "dispatch cursor")
        if type(scan_limit) is not int or not 1 <= scan_limit <= 4096:
            raise ValueError("scan_limit must be an integer in [1, 4096]")
        ttl = self._ttl(ttl_seconds)
        with self._transaction():
            now = self._now()
            eligibility = "AND pending_dependencies=0" if ready_only else ""
            index = "INDEXED BY tasks_dependency_ready_by_sequence" if ready_only else ""
            rows = self.connection.execute(f"""SELECT id,token,action_key,source_snapshot,enqueue_sequence FROM tasks {index}
                WHERE state='queued' {eligibility} AND enqueue_sequence>?
                ORDER BY enqueue_sequence LIMIT ?""", (after_sequence, scan_limit + 1)).fetchall()
            for examined, row in enumerate(rows[:scan_limit], 1):
                blocked = self.connection.execute("""SELECT 1 FROM dependencies d
                    JOIN tasks t ON t.id=d.dependency
                    WHERE d.task=? AND t.state!='succeeded' LIMIT 1""", (row["id"],)).fetchone()
                if blocked:
                    continue
                scopes = self._scopes(row["id"])
                if scopes and self._active_scope_conflict(scopes):
                    continue
                lease = self._claim_row(row, owner, ttl, now, scopes)
                cursor = row["enqueue_sequence"] if examined < len(rows) else None
                return DispatchResult(lease, cursor, examined)
            cursor = rows[scan_limit - 1]["enqueue_sequence"] if len(rows) > scan_limit else None
            return DispatchResult(None, cursor, min(len(rows), scan_limit))

    @staticmethod
    def _check_lease(row, lease, now):
        if (row["state"] != "running" or row["owner"] != lease.owner or row["token"] != lease.token
                or row["action_key"] != lease.action_key or row["source_snapshot"] != lease.source_snapshot
                or row["expires_ms"] <= now):
            raise ValueError("stale, expired or mismatched fencing lease")

    def renew(self, lease: Lease, ttl_seconds: int = 300) -> Lease:
        ttl = self._ttl(ttl_seconds)
        with self._transaction():
            now = self._now()
            row = self._row(lease.task)
            self._check_lease(row, lease, now)
            expiry = _counter(max(row["expires_ms"], now + ttl), "lease deadline")
            self.connection.execute("UPDATE tasks SET expires_ms=? WHERE id=?", (expiry, lease.task))
            self._event(now, lease.task, lease.token, "renewed", {"expires_ms": expiry})
            return Lease(lease.task, lease.owner, lease.token, expiry, lease.action_key, lease.source_snapshot)

    def reconcile_expired(self, task: str, expected_token: int, reconciliation_digest: str):
        # The trusted controller records branch/worktree/evidence disposition.
        # The queue never deletes or resets an abandoned worker's files.
        _identifier(task, "task id")
        _counter(expected_token, "expected fencing token", 1)
        self.store.verify(require_digest(reconciliation_digest))
        with self._transaction():
            now = self._now()
            row = self._row(task)
            if row["state"] != "running" or row["token"] != expected_token or row["expires_ms"] > now:
                raise ValueError("reconciliation does not match expired attempt")
            token = _next_token(row["token"])
            self.connection.execute("UPDATE tasks SET state='queued',owner=NULL,token=?,expires_ms=NULL WHERE id=?", (token, task))
            self.connection.execute("DELETE FROM active_scopes WHERE task=?", (task,))
            self._event(now, task, token, "reconciled", {"previous_token": expected_token, "evidence": reconciliation_digest})

    def retry_failed(self, task: str, expected_token: int, expected_result_digest: str,
                     reconciliation_digest: str, *, request_id: str) -> int:
        """Requeue one failed immutable action, retaining its evidence and fence.

        Only the trusted controller may choose whether a failure is retryable.
        A changed source, toolchain, policy or command requires a new task. The
        durable request id handles an acknowledged retry whose response is lost;
        it may be replayed after later work, but cannot be rebound to new inputs.
        """
        _identifier(task, "task id")
        _identifier(request_id, "request id")
        _counter(expected_token, "expected fencing token", 1)
        previous_receipt = load_json_bytes(self.store.get(require_digest(expected_result_digest)))
        if (type(previous_receipt) is not dict or previous_receipt.get("schema") != "fluxr-job-receipt-v1"
                or previous_receipt.get("outcome") != "failed"):
            raise ValueError("retry requires a retained failed receipt")
        self._verify_artifacts(previous_receipt.get("evidence"))
        self.store.verify(require_digest(reconciliation_digest))
        payload = {"schema": "fluxr-retry-failed-v1", "task": task, "token": expected_token,
                   "result": expected_result_digest, "reconciliation": reconciliation_digest}
        payload_digest = content_digest(canonical_bytes(payload))
        with self._transaction():
            previous = self.connection.execute("SELECT * FROM requests WHERE id=?", (request_id,)).fetchone()
            if previous is not None:
                if previous["payload_digest"] != payload_digest:
                    raise ValueError("request id already binds different immutable inputs")
                return previous["resulting_token"]
            row = self._row(task)
            if (row["state"] != "failed" or row["token"] != expected_token
                    or row["result_digest"] != expected_result_digest
                    or previous_receipt.get("action_key") != row["action_key"]
                    or previous_receipt.get("source_snapshot") != row["source_snapshot"]):
                raise ValueError("retry does not match the current failed attempt")
            token = _next_token(row["token"])
            self.connection.execute("""UPDATE tasks SET state='queued',owner=NULL,token=?,
                expires_ms=NULL,result_digest=NULL WHERE id=?""", (token, task))
            self._event(self._now(), task, token, "retry_requested",
                        {"previous_token": expected_token, "previous_receipt": expected_result_digest,
                         "evidence": reconciliation_digest, "request_id": request_id})
            self.connection.execute("INSERT INTO requests VALUES (?,?,?)", (request_id, payload_digest, token))
            return token

    def _verify_artifacts(self, evidence):
        if not isinstance(evidence, list) or not evidence:
            raise ValueError("receipt needs evidence object digests")
        verified = set()
        for digest in evidence:
            require_digest(digest)
            if digest not in verified:
                self.store.verify(digest)
                verified.add(digest)

    def complete(self, lease: Lease, receipt: dict) -> str:
        receipt = load_json_bytes(canonical_bytes(receipt))
        fields = {"schema", "action_key", "source_snapshot", "outcome", "exit_code", "checks_complete", "evidence"}
        if not isinstance(receipt, dict) or set(receipt) != fields or receipt["schema"] != "fluxr-job-receipt-v1":
            raise ValueError("invalid receipt-v1 fields")
        if receipt["action_key"] != lease.action_key or receipt["source_snapshot"] != lease.source_snapshot:
            raise ValueError("receipt belongs to different immutable inputs")
        if receipt["outcome"] not in {"succeeded", "failed"}:
            raise ValueError("invalid outcome")
        if type(receipt["exit_code"]) is not int or type(receipt["checks_complete"]) is not bool:
            raise ValueError("invalid exit/check types")
        if receipt["outcome"] == "succeeded" and (receipt["exit_code"] != 0 or not receipt["checks_complete"]):
            raise ValueError("successful job requires zero exit and complete checks")
        self._verify_artifacts(receipt["evidence"])
        encoded = canonical_bytes(receipt)
        # Store outside the transaction: checks and I/O must not hold a queue
        # write lock. A lost lease can leave an unreachable CAS object, safely.
        digest = self.store.put(encoded)
        with self._transaction():
            now = self._now()
            row = self._row(lease.task)
            if (row["state"] in {"succeeded", "failed"} and row["owner"] == lease.owner
                    and row["token"] == lease.token and row["result_digest"] == digest):
                return digest  # Retry after a lost response, no duplicate event.
            self._check_lease(row, lease, now)
            self.connection.execute("UPDATE tasks SET state=?,result_digest=?,expires_ms=NULL WHERE id=?",
                                    (receipt["outcome"], digest, lease.task))
            if receipt["outcome"] == "succeeded":
                # This edge decrement and the terminal transition share the
                # transaction. Repeated completion returns above, so an edge
                # is released once even after a lost response. A negative count
                # rejects and rolls back the whole transition.
                self.connection.execute("""UPDATE tasks SET pending_dependencies=pending_dependencies-1
                    WHERE id IN (SELECT task FROM dependencies WHERE dependency=?)""", (lease.task,))
            self.connection.execute("DELETE FROM active_scopes WHERE task=?", (lease.task,))
            self._event(now, lease.task, lease.token, "completed", {"receipt": digest, "outcome": receipt["outcome"]})
        return digest

    def inspect(self, task: str):
        row = dict(self._row(task, include_packet=True))
        row["packet"] = json.loads(row["packet"])
        if row["result_digest"] is not None:
            self.store.verify(row["result_digest"])
        return row

    def events(self, task: str):
        return [dict(row) | {"details": json.loads(row["details"])} for row in
                self.connection.execute("SELECT * FROM events WHERE task=? ORDER BY sequence", (task,))]
