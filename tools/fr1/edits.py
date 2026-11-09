"""Bounded semantic edits to a canonical FR1 packet."""
import struct

from packet import decode, encode
from collections_codec import pack
from collections_lower import DECLARATION
from ordered_codec import encode_abi, encode_code


def binary32(value):
    return struct.unpack("<I", struct.pack("<f", float(value)))[0]


def replace_float(tree, old, new):
    if not isinstance(tree, tuple):
        return tree, 0
    if tree == ("f32bits", old):
        return ("f32bits", new), 1
    items, changes = [], 0
    for item in tree:
        replaced, count = replace_float(item, old, new)
        items.append(replaced)
        changes += count
    return tuple(items), changes


def apply(envelope, words, commands):
    planner, inputs, outputs, code, collection, abi = decode(envelope, words)
    lines = commands.splitlines()
    if not 0 < len(lines) <= 32:
        raise ValueError("expected one to 32 semantic edits")
    for command in lines:
        section, edit = command.split(maxsplit=1)
        if section == "c":
            export, replacement = edit.split("=", 1)
            candidates = []
            found = 0
            for line in collection.splitlines():
                name, args, _ = DECLARATION.fullmatch(line).groups()
                if name == export:
                    head = name if args is None else f"{name}({args})"
                    line = f"{head}={replacement}"
                    found += 1
                candidates.append(line)
            if found != 1:
                raise ValueError("collection edit must identify a single declaration")
            collection = "\n".join(candidates) + "\n"
        elif section == "o":
            tag, prior, after = edit.split()
            tag = int(tag)
            prior, after = binary32(prior), binary32(after)
            found, updated = 0, []
            for operation in code:
                if operation[:2] == ("emit", tag):
                    operation, count = replace_float(operation, prior, after)
                    found += count
                updated.append(operation)
            if not found:
                raise ValueError("effect edit did not match a binary32 literal")
            code = tuple(updated)
        else:
            raise ValueError("unknown edit section")
    names, policy, bindings = pack(collection, abi, prefix=True)
    return encode(planner, encode_abi(inputs, outputs), encode_code(inputs, outputs, code),
                  names, policy, bindings, words)
