"""Opt-in v2 contract identities: exact artifacts, interfaces, and documentation.

V1 meaning and bytes remain unchanged. Only the explicit documentation map is
non-normative in v2; every legacy descriptor/invariant remains interface content.
This module prepares immutable local candidates, never authorizes publication.
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass
import heapq
import json
from pathlib import Path
import sys
from types import MappingProxyType

from . import contracts as v1
from .identity import canonical_bytes, content_digest, load_json_bytes, require_digest

SCHEMA = "fluxr-contract-fragment-v2"
CATALOG_SCHEMA = "fluxr-contract-catalog-v2"
MAX_FRAGMENT_BYTES = v1.MAX_FRAGMENT_BYTES
MAX_TOTAL_BYTES = v1.MAX_TOTAL_BYTES
MAX_FRAGMENTS = v1.MAX_FRAGMENTS
MAX_EDGES = v1.MAX_EDGES
MAX_EXPORTS = v1.MAX_EXPORTS


def _digest(domain, payload):
    return content_digest(domain.encode("ascii") + b"\0" + payload)


@dataclass(frozen=True)
class _Fragment:
    key: str
    digest: str
    interface: str
    documentation: str
    payload: bytes
    requires: tuple[tuple[str, str], ...]
    exports: tuple[str, ...]


def _fragment(value):
    # Own once before validating or deriving any identity. Only this owned
    # snapshot contributes to all three identities.
    payload = canonical_bytes(value)
    if len(payload) > MAX_FRAGMENT_BYTES:
        raise ValueError("contract fragment exceeds byte budget")
    value = load_json_bytes(payload)
    fields = {"schema", "key", "requires", "exports", "invariants", "documentation"}
    if type(value) is not dict or set(value) != fields:
        raise ValueError("unexpected or missing v2 contract fragment fields")
    if value["schema"] != SCHEMA:
        raise ValueError("unsupported v2 contract fragment schema")
    documentation = value["documentation"]
    if type(documentation) is not dict:
        raise ValueError("documentation must map bounded names to strings")
    for key, text in documentation.items():
        v1._name(key)
        if type(text) is not str:
            raise ValueError("documentation entries must be strings")
    # Reuse the released name/descriptor/invariant validator. This does not
    # reinterpret fields called doc, description, default or examples inside
    # descriptors; those fields remain normative and are hashed in full.
    legacy = {key: item for key, item in value.items() if key != "documentation"}
    legacy["schema"] = v1.SCHEMA
    checked = v1._fragment(legacy)
    interface = {key: item for key, item in value.items() if key != "documentation"}
    document = {"key": value["key"], "documentation": documentation}
    return _Fragment(checked.key, _digest("fluxr/contract-fragment/v2", payload),
                     _digest("fluxr/contract-interface/v2", canonical_bytes(interface)),
                     _digest("fluxr/contract-documentation/v2", canonical_bytes(document)),
                     payload, checked.requires, checked.exports)


def fragment_digest(value):
    """Exact full fragment identity; required by guarded edits and review."""
    return _fragment(value).digest


def interface_digest(value):
    """Identity for v2 requirements; includes transitive interface pins."""
    return _fragment(value).interface


def fragment_identities(value):
    item = _fragment(value)
    return {"fragment": item.digest, "interface": item.interface,
            "documentation": item.documentation}


class Catalog:
    """A bounded frozen registry with exact and interface-only revisions.

    Dependency pins compare interface IDs. Edit guards always compare complete
    fragment IDs, including documentation; stale documentation cannot overwrite
    concurrent work. Equal interfaces do not prove descriptor meaning, trusted
    dependency completeness, authorization, or safe compiled-artifact reuse.
    """

    __slots__ = ("_fragments", "_manifest", "_revision", "_interface_revision")

    def __init__(self, fragments):
        by_key, symbols = {}, {}
        total = edges = exports = 0
        for count, value in enumerate(fragments, 1):
            if count > MAX_FRAGMENTS:
                raise ValueError("too many contract fragments")
            item = _fragment(value)
            total += len(item.payload)
            edges += len(item.requires)
            exports += len(item.exports)
            if total > MAX_TOTAL_BYTES or edges > MAX_EDGES or exports > MAX_EXPORTS:
                raise ValueError("contract catalog exceeds aggregate budget")
            if item.key in by_key:
                raise ValueError(f"duplicate contract key: {item.key}")
            by_key[item.key] = item
            for symbol in item.exports:
                if symbol in symbols:
                    raise ValueError(f"export collision: {symbol}")
                symbols[symbol] = item.key
        for item in by_key.values():
            for key, expected in item.requires:
                actual = by_key.get(key)
                if actual is None or actual.interface != expected:
                    raise ValueError(f"missing or stale interface pin: {item.key} -> {key}")
        exports = dict(sorted(symbols.items()))
        interfaces = {key: item.interface for key, item in sorted(by_key.items())}
        interface_manifest = {"schema": "fluxr-contract-interface-catalog-v2",
                              "interfaces": interfaces, "exports": exports}
        self._interface_revision = _digest("fluxr/contract-interface-catalog/v2",
                                           canonical_bytes(interface_manifest))
        manifest = {"schema": CATALOG_SCHEMA,
                    "fragments": {key: {"fragment": item.digest, "interface": item.interface,
                                         "documentation": item.documentation}
                                  for key, item in sorted(by_key.items())},
                    "exports": exports, "interface_revision": self._interface_revision}
        self._manifest = canonical_bytes(manifest)
        self._revision = _digest("fluxr/contract-catalog/v2", self._manifest)
        self._fragments = MappingProxyType(by_key)

    @property
    def revision(self):
        return self._revision

    @property
    def interface_revision(self):
        return self._interface_revision

    def manifest(self):
        return {**load_json_bytes(self._manifest), "revision": self._revision}

    def fragment(self, key):
        return load_json_bytes(self._fragments[v1._name(key)].payload)

    def compose(self, changes):
        """Prepare a complete candidate with v1-shaped exact artifact guards."""
        replacements, seen = {}, set()
        total = 0
        for count, change in enumerate(changes, 1):
            if count > MAX_FRAGMENTS:
                raise ValueError("too many contract edits")
            encoded = canonical_bytes(change)
            total += len(encoded)
            if total > MAX_TOTAL_BYTES:
                raise ValueError("contract edits exceed aggregate budget")
            change = load_json_bytes(encoded)
            if type(change) is not dict or set(change) != {"key", "expected", "replacement"}:
                raise ValueError("unexpected or missing contract edit fields")
            key = v1._name(change["key"])
            if key in seen:
                raise ValueError("duplicate contract edit target")
            seen.add(key)
            expected = change["expected"]
            if expected is not None:
                require_digest(expected)
            actual = self._fragments.get(key)
            if (actual.digest if actual else None) != expected:
                raise ValueError(f"stale contract edit: {key}")
            replacement = change["replacement"]
            if replacement is None:
                if actual is None:
                    raise ValueError("cannot delete an absent contract")
                replacements[key] = None
            else:
                item = _fragment(replacement)
                if item.key != key:
                    raise ValueError("replacement key differs from edit target")
                replacements[key] = item
        candidate = dict(self._fragments)
        for key, value in replacements.items():
            if value is None:
                del candidate[key]
            else:
                candidate[key] = value
        return Catalog(load_json_bytes(item.payload) for item in candidate.values())

    def delta(self, newer):
        """Deterministic exact review delta; not a compatibility certificate."""
        if type(newer) is not Catalog:
            raise ValueError("delta requires a v2 catalog")
        before, after = self._fragments, newer._fragments
        shared = before.keys() & after.keys()
        return {"schema": "fluxr-contract-review-delta-v2",
                "before": self.revision, "after": newer.revision,
                "before_interface": self.interface_revision,
                "after_interface": newer.interface_revision,
                "added": sorted(after.keys() - before.keys()),
                "removed": sorted(before.keys() - after.keys()),
                "interface_changed": sorted(key for key in shared
                                            if before[key].interface != after[key].interface),
                "documentation_only": sorted(key for key in shared
                                             if before[key].digest != after[key].digest
                                             and before[key].interface == after[key].interface)}


class Migration:
    """Retained canonical v1 payloads, v2 candidate, and detached receipt.

    Input file formatting was never retained by v1 Catalog. This migration is
    lossless for its canonical artifact bytes and identities, not original
    source-file whitespace. The caller must retain source files independently.
    """

    __slots__ = ("_original", "_catalog", "_receipt")

    def __init__(self, original, catalog, receipt):
        self._original = MappingProxyType(dict(original))
        self._catalog = catalog
        self._receipt = canonical_bytes(receipt)

    @property
    def catalog(self):
        return self._catalog

    def receipt(self):
        return load_json_bytes(self._receipt)

    def original_fragment(self, key):
        return load_json_bytes(self._original[v1._name(key)])

    def restore(self):
        result = v1.Catalog(load_json_bytes(raw) for raw in self._original.values())
        if result.revision != self.receipt()["source_catalog"]:
            raise ValueError("migration restore identity mismatch")
        return result


def migrate_v1(legacy):
    """Explicit, conservative, reversible canonical v1 -> v2 migration.

    All descriptors/invariants remain normative. No field is automatically
    promoted to documentation. Required exact v1 pins become exact v2 interface
    pins in dependency order. A changed version is never disguised as v1.
    """
    if type(legacy) is not v1.Catalog:
        raise ValueError("migration requires a validated v1 catalog")
    originals = {key: canonical_bytes(legacy.fragment(key))
                 for key in legacy.manifest()["fragments"]}
    values = {key: load_json_bytes(raw) for key, raw in originals.items()}
    remaining = {key: len(value["requires"]) for key, value in values.items()}
    dependents = {key: [] for key in values}
    for key, value in values.items():
        for provider in value["requires"]:
            dependents[provider].append(key)
    ready = [key for key, count in remaining.items() if not count]
    heapq.heapify(ready)
    migrated, interfaces, mapping = {}, {}, {}
    while ready:
        key = heapq.heappop(ready)
        value = values[key]
        replacement = {**value, "schema": SCHEMA, "documentation": {},
                       "requires": {provider: interfaces[provider]
                                    for provider in sorted(value["requires"])}}
        item = _fragment(replacement)
        migrated[key], interfaces[key] = replacement, item.interface
        mapping[key] = {"v1_fragment": v1.fragment_digest(value),
                        "v2_fragment": item.digest, "v2_interface": item.interface}
        for dependent in dependents[key]:
            remaining[dependent] -= 1
            if not remaining[dependent]:
                heapq.heappush(ready, dependent)
    if len(migrated) != len(values):
        raise ValueError("cyclic fragments cannot migrate; combine recursive contracts explicitly")
    catalog = Catalog(migrated.values())
    receipt = {"schema": "fluxr-contract-migration-v1-to-v2",
               "source_catalog": legacy.revision, "result_catalog": catalog.revision,
               "result_interface_catalog": catalog.interface_revision,
               "mapping": dict(sorted(mapping.items())),
               "rule": "retain all v1 normative fields; replace dependency pins; empty documentation"}
    return Migration(originals, catalog, receipt)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fragments", nargs="+", type=Path)
    args = parser.parse_args(argv)
    try:
        catalog = Catalog(v1._read(path) for path in args.fragments)
    except (ValueError, OSError) as error:
        parser.exit(1, f"v2 contract composition refused: {error}\n")
    json.dump(catalog.manifest(), sys.stdout, sort_keys=True, separators=(",", ":"))
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
