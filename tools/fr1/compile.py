"""Compile FR1 or compact structured source, with optional native output."""
import argparse
from hashlib import sha256
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

from edits import apply
from native import emit
from packet import ALPHABET_SHA256, IDENTIFIER, MAX_PACKET_BYTES, alphabet, lower_source


MAX_INTERFACE_BYTES = 64 * 1024
MAX_EDIT_BYTES = 4 * 1024
MAX_LOWERED_BYTES = 256 * 1024
SCALARS = {"unit", "bool", "u64", "i64", "f32"}


def read(path, limit):
    with path.open("rb") as file:
        data = file.read(limit + 1)
    if len(data) > limit:
        raise ValueError(f"{path.name} exceeds its byte limit")
    return data.decode("utf-8")


def check_interface(schema):
    def name(value):
        if not isinstance(value, str) or not IDENTIFIER.fullmatch(value):
            raise ValueError("invalid nominal identifier")

    def shape(value, depth=0):
        if depth >= 8:
            raise ValueError("nominal type is too deep")
        if isinstance(value, str) and value in SCALARS:
            return
        if isinstance(value, dict) and len(value) == 1:
            kind, item = next(iter(value.items()))
            if kind in {"record", "enum"}:
                name(item)
                return
            if kind == "list":
                shape(item, depth + 1)
                return
        raise ValueError("invalid nominal type")

    def names(items):
        seen = set()
        for item in items:
            value = item["name"]
            name(value)
            if value in seen:
                raise ValueError(f"duplicate nominal identifier {value}")
            seen.add(value)

    names(schema["records"] + schema["enums"])
    names(schema["functions"])
    for record in schema["records"]:
        names(record["fields"])
        for field in record["fields"]:
            shape(field["ty"])
    for enum in schema["enums"]:
        names(enum["variants"])
        for variant in enum["variants"]:
            if variant["payload"] is not None:
                shape(variant["payload"])
    for function in schema["functions"]:
        if function["export"] is not True:
            raise ValueError("FR1 binding accepts only exported signatures")
        names(function["params"])
        for parameter in function["params"]:
            shape(parameter["ty"])
        shape(function["result"])


def publish(path, data):
    with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as file:
        temporary = Path(file.name)
        try:
            file.write(data)
            file.close()
            os.replace(temporary, path)
        finally:
            temporary.unlink(missing_ok=True)


def bind_packet(args):
    envelope = read(args.source, MAX_PACKET_BYTES)
    interface = read(args.interface, MAX_INTERFACE_BYTES)
    schema = json.loads(interface)
    check_interface(schema)
    words = alphabet()
    if args.edits:
        envelope = apply(envelope, words, read(args.edits, MAX_EDIT_BYTES))
    lowered = lower_source(envelope, words, schema).encode("utf-8")
    if len(lowered) > MAX_LOWERED_BYTES:
        raise ValueError("lowered source exceeds the typed compiler byte limit")
    return envelope, interface, lowered


def migrate_packet(args):
    envelope, interface, lowered = bind_packet(args)
    with tempfile.TemporaryDirectory() as temporary:
        source = Path(temporary) / "decoded.flux"
        source.write_bytes(lowered)
        command = [args.compiler.resolve(), "--receipt"]
        if args.replay:
            command.extend(("--replay", args.replay))
        migrate = "migrate-structured-hir" if args.migrate_hir else "migrate-structured"
        command.extend((migrate, source, args.output))
        result = subprocess.run(command, stdout=subprocess.PIPE, check=True, timeout=30)
    receipt = {
        "version": 1,
        "format": "fr1",
        "input_sha256": sha256(envelope.encode("utf-8")).hexdigest(),
        "interface_sha256": sha256(interface.encode("utf-8")).hexdigest(),
        "alphabet_sha256": ALPHABET_SHA256,
        "structured": json.loads(result.stdout),
    }
    sys.stdout.buffer.write(json.dumps(receipt, separators=(",", ":")).encode("utf-8") + b"\n")


def compile_packet(args):
    outputs = [path.resolve() for path in
               (args.output, args.emit_source, args.emit_packet, args.emit_native) if path]
    inputs = {path.resolve() for path in (args.source, args.interface, args.edits, args.compiler) if path}
    if len(outputs) != len(set(outputs)) or inputs.intersection(outputs):
        raise ValueError("compiler outputs must be distinct from one another and from the inputs")
    if args.migrate or args.migrate_hir:
        if args.compact or any((args.edits, args.emit_source, args.emit_packet,
                               args.emit_native, args.native_export, args.native_loops)):
            flag = "--migrate-hir" if args.migrate_hir else "--migrate"
            raise ValueError(f"{flag} requires FR1 input and cannot be combined with compile or edit outputs")
        migrate_packet(args)
        return
    if args.replay:
        raise ValueError("--replay requires --migrate or --migrate-hir")
    if bool(args.emit_native) != bool(args.native_export):
        raise ValueError("native output requires at least one --native-export")
    if args.native_loops and not args.emit_native:
        raise ValueError("--native-loops requires native output")
    envelope = None
    if args.compact:
        if args.edits or args.emit_packet:
            raise ValueError("FR1 edits and packet output require FR1 source")
        lowered = read(args.source, MAX_LOWERED_BYTES).encode("utf-8")
    else:
        envelope, _, lowered = bind_packet(args)
    with tempfile.TemporaryDirectory() as temporary:
        source = Path(temporary) / "decoded.flux"
        program = Path(temporary) / "decoded.fsm"
        source.write_bytes(lowered)
        subprocess.run([args.compiler.resolve(), "encode-structured", source, program],
                       check=True, timeout=30)
        compiled = program.read_bytes()
    native = (
        emit(compiled, args.native_export, bounded_loops=args.native_loops).encode("utf-8")
        if args.emit_native else None
    )
    if args.emit_source:
        publish(args.emit_source, lowered)
    if args.emit_packet:
        publish(args.emit_packet, envelope.encode("utf-8"))
    if args.emit_native:
        publish(args.emit_native, native)
    publish(args.output, compiled)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--interface", type=Path, help="nominal interface for FR1 source")
    mode.add_argument("--compact", action="store_true", help="compile compact structured source")
    parser.add_argument("--compiler", required=True, type=Path, help="path to fluxr-cli")
    parser.add_argument("--edits", type=Path)
    parser.add_argument("--emit-source", type=Path)
    parser.add_argument("--emit-packet", type=Path)
    parser.add_argument("--emit-native", type=Path)
    parser.add_argument("--native-export", action="append")
    parser.add_argument("--native-loops", action="store_true")
    migration = parser.add_mutually_exclusive_group()
    migration.add_argument("--migrate", action="store_true", help="write compact source and print its migration receipt")
    migration.add_argument("--migrate-hir", action="store_true", help="write checked FXH and print its migration receipt")
    parser.add_argument("--replay", type=Path, help="bounded structured replay for migration")
    args = parser.parse_args()
    try:
        compile_packet(args)
    except (ValueError, KeyError, IndexError, TypeError, SyntaxError, StopIteration,
            OverflowError, RecursionError, OSError, subprocess.SubprocessError) as error:
        source_kind = "Compact" if args.compact else "FR1"
        parser.exit(1, f"{source_kind} compilation failed ({type(error).__name__}): {error}\n")


if __name__ == "__main__":
    main()
