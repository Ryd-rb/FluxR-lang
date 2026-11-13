"""Immutable, independently authored design contracts with exact dependency pins.

This is a local composition primitive, not a FluxR type checker, compatibility
proof, authorization service, or publisher. It permits disjoint contract edits
without one manually edited registry. A trusted compiler/policy must still
check the meanings of descriptors and approve the assembled candidate.
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass
import json
import os
from pathlib import Path
import re
import stat
import sys
from types import MappingProxyType

from .identity import canonical_bytes, content_digest, load_json_bytes, require_digest

SCHEMA = "fluxr-contract-fragment-v1"
MAX_FRAGMENT_BYTES = 256 * 1024
MAX_TOTAL_BYTES = 16 * 1024 * 1024
MAX_FRAGMENTS = 4096
MAX_EDGES = 32768
MAX_EXPORTS = 32768
NAME = re.compile(r"[A-Za-z][A-Za-z0-9_.-]*(?:/[A-Za-z][A-Za-z0-9_.-]*)*\Z", re.ASCII)


def _name(value):
    if type(value) is not str or len(value) > 256 or not NAME.fullmatch(value):
        raise ValueError("contract and export keys require bounded namespace names")
    return value


@dataclass(frozen=True)
class _Fragment:
    key: str
    digest: str
    payload: bytes
    requires: tuple[tuple[str, str], ...]
    exports: tuple[str, ...]


def _fragment(value):
    # canonical_bytes owns the input and enforces a pre-serialization budget.
    payload = canonical_bytes(value)
    if len(payload) > MAX_FRAGMENT_BYTES:
        raise ValueError("contract fragment exceeds byte budget")
    value = load_json_bytes(payload)
    if type(value) is not dict or set(value) != {"schema", "key", "requires", "exports", "invariants"}:
        raise ValueError("unexpected or missing contract fragment fields")
    if value["schema"] != SCHEMA:
        raise ValueError("unsupported contract fragment schema")
    key = _name(value["key"])
    requires, exports, invariants = value["requires"], value["exports"], value["invariants"]
    if type(requires) is not dict or len(requires) > MAX_EDGES:
        raise ValueError("contract requires must be a bounded pin map")
    for dependency, revision in requires.items():
        _name(dependency)
        require_digest(revision)
        if dependency == key:
            raise ValueError("mutually recursive contracts belong in one fragment")
    if type(exports) is not dict or not exports or len(exports) > MAX_EXPORTS:
        raise ValueError("contract exports must be a nonempty bounded descriptor map")
    for name, descriptor in exports.items():
        _name(name)
        if type(descriptor) is not dict or not descriptor:
            raise ValueError("export descriptors must be nonempty objects")
    if type(invariants) is not dict:
        raise ValueError("invariants must map stable names to requirement descriptions")
    for name, requirement in invariants.items():
        _name(name)
        if type(requirement) is not str or not requirement.strip():
            raise ValueError("invariant requirements must be nonempty strings")
    return _Fragment(key, content_digest(b"fluxr/contract-fragment/v1\0" + payload), payload,
                     tuple(sorted(requires.items())), tuple(sorted(exports)))


def fragment_digest(value):
    """Identity of the entire descriptor, dependency pins and named invariants."""
    return _fragment(value).digest


class Catalog:
    """Frozen catalog; returned manifests/fragments are detached copies.

    The constructor validates exact pins after the complete set is read. Missing
    or incompatible imported contracts fail instead of resolving to a newest
    version. Recursive nominal types may be described within a single fragment.
    """

    __slots__ = ("_fragments", "_manifest", "_revision")

    def __init__(self, fragments):
        by_key, symbols = {}, {}
        total = edges = exports = 0
        for count, value in enumerate(fragments, 1):
            if count > MAX_FRAGMENTS:
                raise ValueError("too many contract fragments")
            fragment = _fragment(value)
            total += len(fragment.payload)
            edges += len(fragment.requires)
            exports += len(fragment.exports)
            if total > MAX_TOTAL_BYTES or edges > MAX_EDGES or exports > MAX_EXPORTS:
                raise ValueError("contract catalog exceeds aggregate budget")
            if fragment.key in by_key:
                raise ValueError(f"duplicate contract key: {fragment.key}")
            by_key[fragment.key] = fragment
            for symbol in fragment.exports:
                if symbol in symbols:
                    raise ValueError(f"export collision: {symbol}")
                symbols[symbol] = fragment.key
        for fragment in by_key.values():
            for key, expected in fragment.requires:
                actual = by_key.get(key)
                if actual is None or actual.digest != expected:
                    raise ValueError(f"missing or stale contract pin: {fragment.key} -> {key}")
        manifest = {"schema": "fluxr-contract-catalog-v1",
                    "fragments": {key: item.digest for key, item in sorted(by_key.items())},
                    "exports": dict(sorted(symbols.items()))}
        self._manifest = canonical_bytes(manifest)
        self._revision = content_digest(b"fluxr/contract-catalog/v1\0" + self._manifest)
        self._fragments = MappingProxyType(by_key)

    @property
    def revision(self):
        return self._revision

    def manifest(self):
        return {**load_json_bytes(self._manifest), "revision": self._revision}

    def fragment(self, key):
        return load_json_bytes(self._fragments[_name(key)].payload)

    def compose(self, changes):
        """Atomically prepare guarded add/replace/delete operations.

        Each operation has key, expected (null for insertion), replacement
        (null for deletion). Expected revisions compare against this catalog,
        never against a partially composed intermediate. Distinct independent
        edits commute; dependent edits require explicitly updated pins in the
        same batch. No mutable state or publication authority is changed.
        """
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
            key = _name(change["key"])
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
                fragment = _fragment(replacement)
                if fragment.key != key:
                    raise ValueError("replacement key differs from edit target")
                replacements[key] = fragment
        candidate = dict(self._fragments)
        for key, value in replacements.items():
            if value is None:
                del candidate[key]
            else:
                candidate[key] = value
        # Revalidate the composed set, including unchanged consumers' pins.
        return Catalog(load_json_bytes(item.payload) for item in candidate.values())


def _read(path):
    if Path(path).is_symlink():
        raise ValueError("contract input must not be a symlink")
    descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_NONBLOCK", 0))
    with os.fdopen(descriptor, "rb") as stream:
        if not stat.S_ISREG(os.fstat(stream.fileno()).st_mode):
            raise ValueError("contract input must be a regular file")
        raw = stream.read(MAX_FRAGMENT_BYTES + 1)
    if len(raw) > MAX_FRAGMENT_BYTES:
        raise ValueError("contract file exceeds byte budget")
    return load_json_bytes(raw)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fragments", nargs="+", type=Path)
    args = parser.parse_args(argv)
    try:
        catalog = Catalog(_read(path) for path in args.fragments)
    except (ValueError, OSError) as error:
        parser.exit(1, f"contract composition refused: {error}\n")
    json.dump(catalog.manifest(), sys.stdout, sort_keys=True, separators=(",", ":"))
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
