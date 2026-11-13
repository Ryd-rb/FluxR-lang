"""Local CLI. No commands are executed and no Git refs are changed."""
from __future__ import annotations

import argparse
import json
from pathlib import Path

from .identity import MAX_JSON_BYTES, action_key, load_json_bytes
from .planning import plan_batches
from .queue import Lease, WorkQueue
from .store import ContentStore, STREAM_CHUNK_BYTES


def read(path, maximum=MAX_JSON_BYTES):
    with Path(path).open("rb") as stream:
        data = stream.read(maximum + 1)
    if len(data) > maximum:
        raise ValueError("input file exceeds byte limit")
    return data


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--store", default=".fluxr-agent/objects")
    parser.add_argument("--database", default=".fluxr-agent/queue.sqlite")
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("key", "plan", "put"):
        command = commands.add_parser(name)
        command.add_argument("file")
    enqueue = commands.add_parser("enqueue")
    enqueue.add_argument("task")
    enqueue.add_argument("packet", help="JSON with action, dependencies and exclusive_scopes")
    claim = commands.add_parser("claim")
    claim.add_argument("task")
    claim.add_argument("owner")
    claim.add_argument("--ttl", type=int, default=300)
    dispatch = commands.add_parser("claim-next", help="claim one job from a bounded dispatch window")
    dispatch.add_argument("owner")
    dispatch.add_argument("--ttl", type=int, default=300)
    dispatch.add_argument("--after", type=int, default=0)
    dispatch.add_argument("--scan-limit", type=int, default=128)
    dispatch.add_argument("--ready-only", action="store_true",
                          help="scan only the indexed dependency-ready candidate set")
    commands.add_parser("enable-fair", help="explicitly migrate to durable fair-dispatch schema 5")
    fair = commands.add_parser("claim-fair", help="advance the durable shared bounded ready sweep")
    fair.add_argument("owner")
    fair.add_argument("--ttl", type=int, default=300)
    fair.add_argument("--scan-limit", type=int, default=128)
    renew = commands.add_parser("renew")
    renew.add_argument("lease")
    renew.add_argument("--ttl", type=int, default=300)
    complete = commands.add_parser("complete")
    complete.add_argument("lease")
    complete.add_argument("receipt")
    reconcile = commands.add_parser("reconcile")
    reconcile.add_argument("task")
    reconcile.add_argument("token", type=int)
    reconcile.add_argument("evidence_digest")
    retry = commands.add_parser("retry-failed", help="requeue an explicitly reconciled failed attempt")
    retry.add_argument("task")
    retry.add_argument("token", type=int)
    retry.add_argument("result_digest")
    retry.add_argument("evidence_digest")
    retry.add_argument("request_id")
    for name in ("inspect", "events"):
        command = commands.add_parser(name)
        command.add_argument("task")
    args = parser.parse_args()
    queue = None
    try:
        if args.command == "key":
            result = {"action_key": action_key(load_json_bytes(read(args.file)))}
        elif args.command == "plan":
            result = {"batches": plan_batches(load_json_bytes(read(args.file))), "advisory_only": True}
        else:
            store = ContentStore(args.store)
            if args.command == "put":
                with Path(args.file).open("rb") as source:
                    result = {"digest": store.put_stream(iter(lambda: source.read(STREAM_CHUNK_BYTES), b""))}
            else:
                queue = WorkQueue(args.database, store)
                if args.command == "enqueue":
                    packet = load_json_bytes(read(args.packet))
                    if set(packet) != {"action", "dependencies", "exclusive_scopes"}:
                        raise ValueError("invalid packet fields")
                    result = {"action_key": queue.enqueue(args.task, packet["action"], dependencies=packet["dependencies"], exclusive_scopes=packet["exclusive_scopes"])}
                elif args.command == "claim":
                    result = queue.claim(args.task, args.owner, args.ttl).to_dict()
                elif args.command == "claim-next":
                    claim_next = queue.claim_ready if args.ready_only else queue.claim_next
                    result = claim_next(args.owner, args.ttl, after_sequence=args.after,
                                        scan_limit=args.scan_limit).to_dict()
                elif args.command == "enable-fair":
                    queue.enable_fair_dispatch()
                    result = {"schema": 5, "policy": "fluxr-fair-ready-v1"}
                elif args.command == "claim-fair":
                    result = queue.claim_fair(args.owner, args.ttl, scan_limit=args.scan_limit).to_dict()
                elif args.command == "renew":
                    result = queue.renew(Lease(**load_json_bytes(read(args.lease))), args.ttl).to_dict()
                elif args.command == "complete":
                    result = {"receipt_digest": queue.complete(Lease(**load_json_bytes(read(args.lease))), load_json_bytes(read(args.receipt)))}
                elif args.command == "reconcile":
                    queue.reconcile_expired(args.task, args.token, args.evidence_digest)
                    result = {"reconciled": args.task}
                elif args.command == "retry-failed":
                    result = {"requeued": args.task, "token": queue.retry_failed(
                        args.task, args.token, args.result_digest, args.evidence_digest,
                        request_id=args.request_id)}
                elif args.command == "inspect":
                    result = queue.inspect(args.task)
                else:
                    result = queue.events(args.task)
        print(json.dumps(result, sort_keys=True, indent=2))
    except (ValueError, OSError, TypeError) as error:
        parser.exit(2, f"agent-pipeline: {error}\n")
    finally:
        if queue is not None:
            queue.close()


if __name__ == "__main__":
    main()
