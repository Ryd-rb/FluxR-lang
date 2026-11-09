"""FR1 transport and nominal binding to the FluxR structured compiler."""
import gzip
import hashlib
from pathlib import Path
import re

from collections_codec import Bits, decode_words, encode_words, pack, unpack
from collections_lower import lower_collections, ty
from effects import Lower
from ordered_codec import MAX_BYTES, decode_abi, decode_code, encode_abi, encode_code, eval_expr


VERSION = "fr1"
MAX_PACKET_BYTES = 32 * 1024
ALPHABET_SHA256 = "ad5ec04db8fd979a5a8c14c8329284fe27d7253173c268c076253ea212418ecd"
IDENTIFIER = re.compile(r"[A-Za-z_][A-Za-z0-9_]{0,63}", re.ASCII)


def alphabet():
    data = gzip.decompress(Path(__file__).with_name("o200k-fr1.txt.gz").read_bytes())
    if hashlib.sha256(data).hexdigest() != ALPHABET_SHA256:
        raise ValueError("FR1 alphabet identity mismatch")
    return data.decode("utf-8").splitlines()


def encode(planner, order_abi, order_code, names, collection_code, collection_abi, words):
    data = Bits()
    for block in (order_abi, order_code):
        data.put_uint(len(block))
        data.put(int.from_bytes(block), len(block) * 8)
    data.put_uint(collection_code.size)
    data.put(collection_code.data, collection_code.size)
    data.put(collection_abi.data, collection_abi.size)
    return " ".join([VERSION, planner, *names]) + "\n" + encode_words(data, words, 16) + "\n"


def decode(source, words):
    if len(source.encode("utf-8")) > MAX_PACKET_BYTES:
        raise ValueError("source envelope exceeds the byte limit")
    header, body, tail = source.split("\n")
    version, planner, *names = header.split()
    exports = [planner, *names]
    if version != VERSION or tail or not body.startswith(" "):
        raise ValueError("unknown source envelope")
    if len(exports) > 16 or len(set(exports)) != len(exports) or not all(map(IDENTIFIER.fullmatch, exports)):
        raise ValueError("invalid or duplicate export name")
    if len(body.split()) > 1024:
        raise ValueError("source envelope exceeds the word limit")
    data = decode_words(body, {word: index for index, word in enumerate(words)}, 16)
    blocks = []
    for _ in range(2):
        size = data.get_uint()
        if not 0 < size <= MAX_BYTES:
            raise ValueError("ordered block exceeds the byte limit")
        blocks.append(data.get(size * 8).to_bytes(size, "big"))
    size = data.get_uint()
    code = Bits(data.get(size), size, prefix=True)
    collection, binding = unpack(names, code, data, code_padding=0, schema_padding=15)
    inputs, outputs = decode_abi(blocks[0])
    planner_code = decode_code(inputs, outputs, blocks[1])
    rebuilt_names, rebuilt_code, rebuilt_binding = pack(collection, binding, prefix=True)
    canonical = encode(planner, encode_abi(inputs, outputs), encode_code(inputs, outputs, planner_code),
                       rebuilt_names, rebuilt_code, rebuilt_binding, words)
    if source != canonical:
        raise ValueError("noncanonical FR1 source envelope")
    return planner, inputs, outputs, planner_code, collection, binding


def to_tree(node):
    op, *args = node
    if op == "var":
        return op, f"s{args[0]}"
    if op in {"const", "f32bits"}:
        return node
    if op == "thresholds":
        compare, refs, bound = args
        bounds = bound[1] if bound[0] == "tuple" else [bound] * len(refs)
        parts = [(compare, ("var", f"s{ref}"), to_tree(value))
                 for ref, value in zip(refs, bounds, strict=True)]
        tree, *tail = parts
        for part in tail:
            tree = "&", tree, part
        return tree
    if op == "call":
        _, name = args[0]
        operands = tuple(map(to_tree, args[1]))
        if name == "phase":
            tick, lane, period, lanes = operands
            if period[0] != "const" or lanes[0] != "const" or lanes[1] <= 0 or period[1] <= 0 or period[1] % lanes[1]:
                raise ValueError("phase needs constant positive period and lanes with no remainder")
            return "=", ("%", tick, period), ("*", ("%", lane, lanes), ("const", period[1] // lanes[1]))
        return op, args[0], operands
    return op, *map(to_tree, args)


def lower_plan(planner, inputs, outputs, code, schema, helper_names=None):
    plan, = [function for function in schema["functions"] if function["name"] == planner and function["export"]]
    parameter, = plan["params"]
    record, = [r for r in schema["records"] if r["name"] == parameter["ty"]["record"]]
    order, = [e for e in schema["enums"] if e["name"] == plan["result"]["list"]["enum"]]
    records = {record["name"]: record["fields"] for record in schema["records"]}
    primitive = {"bool": "b", "u64": "u", "f32": "f", "i64": "i", "unit": "none", None: "none"}

    def shape(ty):
        if isinstance(ty, dict) and "list" in ty:
            return "list", shape(ty["list"])
        if isinstance(ty, dict) and "record" in ty:
            return "product", *(shape(field["ty"]) for field in records[ty["record"]])
        return primitive[ty]

    if tuple(shape(field["ty"]) for field in record["fields"]) != inputs:
        raise ValueError("planner argument packet differs from released nominal shape")
    if outputs != {tag: shape(variant["payload"]) for tag, variant in enumerate(order["variants"])}:
        raise ValueError("planner effect packet differs from released nominal shape")
    lower = Lower(parameter, ((f"s{i}", field) for i, field in enumerate(record["fields"])))
    lower.helper_names = helper_names if helper_names is not None else {f["name"] for f in schema["functions"]}
    abi = [""]
    groups = {}
    program = []
    index = len(inputs)

    def tree(value):
        return to_tree(eval_expr(value))

    for number, (op, *args) in enumerate(code):
        if op == "set":
            program.append((op, f"s{index}", tree(args[0])))
            index += 1
        elif op == "guard":
            program.append((op, tree(args[0])))
        elif op == "emit":
            tag, test, payload = args
            key = f"tag{number}"
            abi.append(f"{key} {tag}")
            program.append((op, key, tree(test), None if payload is None else tree(payload)))
        elif op == "diff":
            flag, rank, flagtag, ranktag, mode, default, rows = args
            flag, rank = f"s{flag}", f"s{rank}"
            association = flagtag, ranktag, mode
            if groups.setdefault(flag, association) != association:
                raise ValueError("one flag bank cannot name conflicting effect variants")
            abi.append(f"{flag} {flagtag} {ranktag} {mode}")
            indices = [row[0] for row in rows]
            items = [(tree(test), tree(value)) + ((tree(overlay),) if overlay is not None else ())
                     for _, test, value, overlay in rows]
            program.append((op, flag, rank, indices, default, items))
        else:
            raise ValueError(f"unknown packet effect {op}")
    return lower.function(planner, program, "\n".join(abi), order)


def declarations(schema):
    lines = ["v2"]
    for record in schema["records"]:
        fields = " ".join(f'{field["name"]}:{ty(field["ty"])}' for field in record["fields"])
        lines.append(f'(rec {record["name"]} {fields})')
    for enum in schema["enums"]:
        tags = []
        for variant in enum["variants"]:
            payload = variant["payload"]
            tags.append(variant["name"] + (f":{ty(payload)}" if payload is not None else ""))
        lines.append(f'(enum {enum["name"]} {" ".join(tags)})')
    return "\n".join(lines) + "\n"


def lower_source(envelope, words, schema):
    planner, inputs, outputs, code, decoded, decoded_abi = decode(envelope, words)
    helper_names = {function["name"] for function in schema["functions"]}
    return (
        declarations(schema)
        + lower_plan(planner, inputs, outputs, code, schema, helper_names)
        + lower_collections(planner, decoded, decoded_abi, schema, helper_names)
    )
