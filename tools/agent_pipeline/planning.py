"""Conservative deterministic conflict planning for independently prepared work.

Read/write sets are *claims* unless produced by trusted compiler analysis. A
conflict-free batch still requires validation of its exact combined candidate.
"""
from __future__ import annotations

from .identity import canonical_bytes, load_json_bytes, relative_path

MAX_RESOURCE_BYTES = 4096
MAX_RESOURCE_COMPONENTS = 128


def resource(value: str) -> str:
    if value == "*":
        return value
    if isinstance(value, str) and len(value) > MAX_RESOURCE_BYTES:
        raise ValueError("resource exceeds byte or component limit")
    if not isinstance(value, str) or ":" not in value:
        raise ValueError("resource must use path:, symbol:, contract: or *")
    namespace, name = value.split(":", 1)
    if namespace not in {"path", "symbol", "contract"}:
        raise ValueError("unknown resource namespace")
    if (len(value.encode("utf-8")) > MAX_RESOURCE_BYTES
            or name.count("/") >= MAX_RESOURCE_COMPONENTS):
        raise ValueError("resource exceeds byte or component limit")
    relative_path(name)
    return value


def overlaps(left: str, right: str) -> bool:
    resource(left)
    resource(right)
    if "*" in (left, right):
        return True
    ln, lv = left.split(":", 1)
    rn, rv = right.split(":", 1)
    if ln != rn:
        # Namespace independence is not inferable. Different namespaces need
        # trusted mapping before they can share an automatic integration batch.
        return True
    return lv == rv or (ln == "path" and (lv.startswith(rv + "/") or rv.startswith(lv + "/")))


def scope_conflicts(left, right):
    return any(overlaps(a, b) for a in left for b in right)


def change_conflicts(left, right):
    return (scope_conflicts(left["writes"], right["writes"] + right["reads"])
            or scope_conflicts(right["writes"], left["reads"]))


class _ResourceIndex:
    """Index a union of resources without changing conservative overlap rules.

    Exact names and path ancestors replace pairwise scans of every prior
    change. Different namespaces still conflict. This is a scheduling index,
    not evidence that a resource declaration is complete.
    """
    def __init__(self):
        self.global_scope = False
        self.names = {}
        self.path_tree = {}

    def add(self, resources):
        for value in resources:
            if value == "*":
                self.global_scope = True
                continue
            namespace, name = value.split(":", 1)
            names = self.names.setdefault(namespace, set())
            if namespace == "path":
                node = self.path_tree
                for piece in name.split("/"):
                    if None in node:
                        break  # An existing ancestor already covers this path.
                    node = node.setdefault(piece, {})
                else:
                    # An ancestor replaces its descendants in this union.
                    node.clear()
                    node[None] = True
            else:
                names.add(name)

    def conflicts(self, resources):
        for value in resources:
            if self.global_scope:
                return True
            if value == "*":
                if self.names:
                    return True
                continue
            namespace, name = value.split(":", 1)
            if self.names.keys() - {namespace}:
                return True
            names = self.names.get(namespace, set())
            if name in names:
                return True
            if namespace == "path":
                node = self.path_tree
                for piece in name.split("/"):
                    if None in node:
                        return True
                    node = node.get(piece)
                    if node is None:
                        break
                else:
                    if node:
                        return True  # Exact name or any covered descendant.
        return False


def plan_batches(changes: list[dict]) -> list[list[str]]:
    """Stable greedy partition, not a maximal scheduler or merge implementation."""
    # Validate and plan one owned value snapshot. The input shares the CLI's
    # 4 MiB canonical JSON budget; later caller mutation cannot invalidate a
    # check or alter a previously indexed scope.
    changes = load_json_bytes(canonical_bytes(changes))
    if type(changes) is not list:
        raise ValueError("changes must be a list")
    ids = set()
    for change in changes:
        if type(change) is not dict or set(change) != {"id", "reads", "writes", "analysis_complete"}:
            raise ValueError("invalid change fields")
        if not isinstance(change["id"], str) or not change["id"] or change["id"] in ids:
            raise ValueError("duplicate or empty change id")
        ids.add(change["id"])
        for field in ("reads", "writes"):
            if not isinstance(change[field], list):
                raise ValueError("read/write sets must be lists")
            for name in change[field]:
                resource(name)
        if type(change["analysis_complete"]) is not bool:
            raise ValueError("analysis_complete must be boolean")
    batches, indexes = [], []
    for change in sorted(changes, key=lambda c: c["id"]):
        if not change["analysis_complete"]:
            batches.append([change])
            indexes.append(None)
            continue
        for batch, index in zip(batches, indexes):
            if index is None:
                continue
            reads, writes = index
            if (not writes.conflicts(change["reads"] + change["writes"])
                    and not reads.conflicts(change["writes"])):
                batch.append(change)
                reads.add(change["reads"])
                writes.add(change["writes"])
                break
        else:
            batches.append([change])
            reads, writes = _ResourceIndex(), _ResourceIndex()
            reads.add(change["reads"])
            writes.add(change["writes"])
            indexes.append((reads, writes))
    return [[c["id"] for c in batch] for batch in batches]
