#!/usr/bin/env python3
"""Prepare exact committed source inputs; never execute or authorize an action."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys

from agent_pipeline import ContentStore
from agent_pipeline.identity import MAX_JSON_BYTES, load_json_bytes
from agent_pipeline.snapshot import (bind_action, capture_git, materialize,
                                     verify_materialization, verify_snapshot)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--store", required=True, help="trusted local CAS, outside source checkouts")
    commands = parser.add_subparsers(dest="command", required=True)
    command = commands.add_parser("capture")
    command.add_argument("repository")
    command.add_argument("commit", help="exact full commit OID; refs and abbreviations refuse")
    for name in ("verify", "materialize", "verify-tree", "bind"):
        command = commands.add_parser(name)
        command.add_argument("snapshot", help="exact manifest SHA-256")
        if name in ("materialize", "verify-tree"):
            command.add_argument("directory")
        if name == "bind":
            command.add_argument("action", help="bounded fluxr-action-v1 JSON file")
    arguments = parser.parse_args()
    try:
        store = ContentStore(arguments.store)
        if arguments.command == "capture":
            result = capture_git(arguments.repository, arguments.commit, store)
        elif arguments.command == "verify":
            result = verify_snapshot(store, arguments.snapshot)
        elif arguments.command == "materialize":
            result = materialize(store, arguments.snapshot, arguments.directory)
        elif arguments.command == "verify-tree":
            result = verify_materialization(store, arguments.snapshot, arguments.directory)
        else:
            with Path(arguments.action).open("rb") as stream:
                action = load_json_bytes(stream.read(MAX_JSON_BYTES + 1))
            result = bind_action(store, arguments.snapshot, action)
        print(json.dumps(result, indent=2, sort_keys=True))
        return 0
    except (ValueError, OSError, TypeError) as error:
        print(f"source-snapshot: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
