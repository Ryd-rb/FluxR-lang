#!/usr/bin/env python3
"""Bounded, independently modelled scalar semantics campaigns.

This tool never imports the compiler, lowered plans, or portable evaluator as an
oracle. A small typed AST has its own checked integer interpreter and explicit
resource-event contract. The separately built example observes both public
execution paths. Reports, replays and reductions must live outside this tree.
See docs/semantic-campaign.md for the finite scope and trust boundaries.
"""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
import math
import os
from pathlib import Path
import re
import sys
import tempfile
import time

try:
    from eval.runner import run_bounded
except ModuleNotFoundError:  # Importing as tools.semantic_campaign in a test.
    from tools.eval.runner import run_bounded


ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "tests/fixtures/semantic-campaign/cases.json"
I64_MIN, I64_MAX = -(1 << 63), (1 << 63) - 1
MASK64 = (1 << 64) - 1
GENERATOR = "splitmix64-scalar-v1"
ORACLE_VERSION = "bounded-scalar-oracle-v1"
CORPUS_SCHEMA = "fluxr-semantic-corpus-v1"
REPORT_SCHEMA = "fluxr-semantic-campaign-v1"
MAX_BYTES = 64 * 1024 * 1024
MAX_NODES, MAX_EXPR_DEPTH, MAX_BLOCK_DEPTH = 256, 8, 4
MAX_CASES, MAX_CALLS = 512, 16384
MAX_REPORT_EVENTS = 32768
DEFAULT_LIMITS = {"instructions": 10000, "cells": 1024, "frames": 8}
TRAPS = {
    "arithmetic": "invalid or overflowing integer arithmetic",
    "uninitialized": "read of uninitialized local",
    "instructions": "module instruction budget exhausted",
    "cells": "module cell budget exhausted",
    "frames": "module frame limit exceeded",
    "arity": "host argument or input count differs from interface",
    "int": "host value differs from I64",
    "bool": "host value differs from Bool",
}
OPS = {"add": "+", "sub": "-", "mul": "*", "div": "/", "rem": "%",
       "eq": "==", "lt": "<", "le": "<=", "and": "&&", "or": "||"}
ROUTES = {"Portable", "TypedJit", "NativeScalar", "PrecompiledScan"}
RESERVED = {"true", "false", "pub", "fn", "ret", "if", "else", "while",
            "inputs", "i64", "bool", "unit", "v2"}


class CampaignError(ValueError):
    """Invalid bounded input or an incomplete infrastructure operation."""


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"),
                      ensure_ascii=True, allow_nan=False).encode("ascii")


def digest(value):
    return hashlib.sha256(canonical(value)).hexdigest()


def file_digest(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(65536), b""):
            h.update(block)
    return h.hexdigest()


def strict_json(raw):
    if len(raw) > MAX_BYTES:
        raise CampaignError("JSON input exceeds 64 MiB")

    def unique(pairs):
        out = {}
        for key, value in pairs:
            if key in out:
                raise CampaignError("duplicate JSON field: " + key)
            out[key] = value
        return out

    def bad_constant(value):
        raise CampaignError("nonfinite JSON number: " + value)

    def decimal(value):
        number = float(value)
        if not math.isfinite(number):
            raise CampaignError("nonfinite JSON number: " + value)
        return number

    try:
        return json.loads(raw, object_pairs_hook=unique, parse_constant=bad_constant,
                          parse_float=decimal)
    except (ValueError, RecursionError, UnicodeError) as exc:
        raise CampaignError("invalid bounded JSON: " + str(exc)) from exc


def read_json(path):
    with Path(path).open("rb") as stream:
        return strict_json(stream.read(MAX_BYTES + 1))


def fields(value, required, optional=()):
    if not isinstance(value, dict) or set(value) - set(required) - set(optional) \
            or set(required) - set(value):
        raise CampaignError("object fields differ; required: " + ",".join(required))


def integer(value, low, high, name):
    if type(value) is not int or not low <= value <= high:
        raise CampaignError(f"{name} must be an integer in {low}..{high}")
    return value


def scalar_type(value):
    if isinstance(value, dict) and len(value) == 1:
        if "i64" in value and type(value["i64"]) is int \
                and I64_MIN <= value["i64"] <= I64_MAX:
            return "int"
        if "bool" in value and type(value["bool"]) is bool:
            return "bool"
    raise CampaignError("expected an exact tagged i64 or Boolean scalar")


def validate_outcome(value):
    if isinstance(value, dict) and set(value) == {"value"}:
        scalar_type(value["value"])
    elif isinstance(value, dict) and set(value) == {"trap"} \
            and isinstance(value["trap"], str) and 0 < len(value["trap"]) <= 1024:
        pass
    else:
        raise CampaignError("invalid result/trap outcome")


def compare_outcome(expected, observed):
    """Strict tagged comparison: Python's True == 1 is never sufficient."""
    try:
        validate_outcome(expected)
        validate_outcome(observed)
    except CampaignError:
        return False
    return canonical(expected) == canonical(observed)


def declarations(program):
    return {item["name"]: item["type"] for item in program["params"] + program["locals"]}


def expr_type(expr, types):
    op = expr["op"]
    if op in ("int", "bool"):
        return op
    if op == "var":
        return types[expr["name"]]
    return "bool" if op in ("eq", "lt", "le", "and", "or", "not") else "int"


def validate_program(program):
    fields(program, ("params", "locals", "result", "body"))
    types = {}
    for key in ("params", "locals"):
        items = program[key]
        if not isinstance(items, list) or len(items) > 8:
            raise CampaignError("at most eight parameters and eight locals are admitted")
        for item in items:
            fields(item, ("name", "type"))
            name = item["name"]
            if not isinstance(name, str) or not re.fullmatch(r"[a-z][a-z0-9_]{0,15}", name) \
                    or name in RESERVED or name in types or item["type"] not in ("int", "bool"):
                raise CampaignError("invalid or duplicate scalar declaration")
            types[name] = item["type"]
    if program["result"] not in ("int", "bool"):
        raise CampaignError("result type must be int or bool")
    nodes = 0

    def count():
        nonlocal nodes
        nodes += 1
        if nodes > MAX_NODES:
            raise CampaignError("program exceeds 256 AST nodes")

    def expression(expr, depth=0):
        count()
        if depth > MAX_EXPR_DEPTH or not isinstance(expr, dict):
            raise CampaignError("expression nesting exceeds eight or is malformed")
        op = expr.get("op")
        if op in ("int", "bool"):
            fields(expr, ("op", "value"))
            if op == "int":
                integer(expr["value"], I64_MIN, I64_MAX, "literal")
            elif type(expr["value"]) is not bool:
                raise CampaignError("Boolean literal must be a Boolean")
            return op
        if op == "var":
            fields(expr, ("op", "name"))
            if not isinstance(expr["name"], str) or expr["name"] not in types:
                raise CampaignError("undeclared variable")
            return types[expr["name"]]
        if op == "not":
            fields(expr, ("op", "value"))
            if expression(expr["value"], depth + 1) != "bool":
                raise CampaignError("not needs a Boolean operand")
            return "bool"
        if not isinstance(op, str) or op not in OPS:
            raise CampaignError("unknown scalar expression")
        fields(expr, ("op", "left", "right"))
        left, right = expression(expr["left"], depth + 1), expression(expr["right"], depth + 1)
        if left != right or (op in ("and", "or") and left != "bool") \
                or (op not in ("eq", "and", "or") and left != "int"):
            raise CampaignError("binary expression operand types differ")
        return expr_type(expr, types)

    def block(items, depth=0):
        if not isinstance(items, list) or len(items) > 64 or depth > MAX_BLOCK_DEPTH:
            raise CampaignError("statement list exceeds its bounds")
        if depth == 0 and (not items or not isinstance(items[-1], dict)
                           or items[-1].get("op") != "return"):
            raise CampaignError("program needs one final top-level return")
        for index, stmt in enumerate(items):
            count()
            if not isinstance(stmt, dict):
                raise CampaignError("invalid statement")
            op = stmt.get("op")
            if op == "set":
                fields(stmt, ("op", "name", "value"))
                if not isinstance(stmt["name"], str) or stmt["name"] not in types \
                        or expression(stmt["value"]) != types[stmt["name"]]:
                    raise CampaignError("assignment type or name differs")
            elif op == "if":
                fields(stmt, ("op", "condition", "then", "else"))
                if expression(stmt["condition"]) != "bool":
                    raise CampaignError("branch condition must be Boolean")
                block(stmt["then"], depth + 1)
                block(stmt["else"], depth + 1)
            elif op == "return":
                fields(stmt, ("op", "value"))
                if depth or index != len(items) - 1 \
                        or expression(stmt["value"]) != program["result"]:
                    raise CampaignError("only one final top-level return is admitted")
            else:
                raise CampaignError("unknown scalar statement")

    block(program["body"])


def validate_call(call):
    fields(call, ("inputs", "arguments", "limits"))
    for name in ("inputs", "arguments"):
        if not isinstance(call[name], list) or len(call[name]) > 16:
            raise CampaignError("at most 16 scalar boundary lanes are admitted")
        for value in call[name]:
            scalar_type(value)
    fields(call["limits"], ("instructions", "cells", "frames"))
    for key, maximum in (("instructions", 1000000), ("cells", 65536), ("frames", 64)):
        integer(call["limits"][key], 0, maximum, key)


def validate_cases(cases):
    if not isinstance(cases, list) or not 1 <= len(cases) <= MAX_CASES:
        raise CampaignError("campaign requires 1..512 cases")
    ids, calls = set(), 0
    for case in cases:
        fields(case, ("id", "program", "calls"), ("expected",))
        name = case["id"]
        if not isinstance(name, str) or not re.fullmatch(r"[A-Za-z0-9_.-]{1,128}", name) or name in ids:
            raise CampaignError("case ids must be unique bounded identifiers")
        ids.add(name)
        validate_program(case["program"])
        if not isinstance(case["calls"], list) or not 1 <= len(case["calls"]) <= 512:
            raise CampaignError("case requires 1..512 calls")
        calls += len(case["calls"])
        if calls > MAX_CALLS:
            raise CampaignError("campaign exceeds 16384 calls")
        for call in case["calls"]:
            validate_call(call)
        if "expected" in case:
            if not isinstance(case["expected"], list) or len(case["expected"]) != len(case["calls"]):
                raise CampaignError("fixture expected count differs")
            for value in case["expected"]:
                validate_outcome(value)


def emit_source(program):
    validate_program(program)

    def expression(expr):
        op = expr["op"]
        if op == "int":
            return str(expr["value"])
        if op == "bool":
            return "true" if expr["value"] else "false"
        if op == "var":
            return expr["name"]
        if op == "not":
            return "(! " + expression(expr["value"]) + ")"
        return "(" + OPS[op] + " " + expression(expr["left"]) + " " + expression(expr["right"]) + ")"

    def block(items):
        parts = []
        for stmt in items:
            if stmt["op"] == "set":
                parts.append("(= " + stmt["name"] + " " + expression(stmt["value"]) + ")")
            elif stmt["op"] == "return":
                parts.append("(ret " + expression(stmt["value"]) + ")")
            else:
                parts.append("(if " + expression(stmt["condition"]) + " (" + block(stmt["then"]) + ") (" + block(stmt["else"]) + "))")
        return " ".join(parts)

    def decl(items):
        return " ".join(item["name"] + ":" + ("i64" if item["type"] == "int" else "bool") for item in items)

    source = "v2 (pub calc (" + decl(program["params"]) + ") " + ("i64" if program["result"] == "int" else "bool") + " (" + decl(program["locals"]) + ") " + block(program["body"]) + ")\n"
    if len(source.encode()) > 16384:
        raise CampaignError("emitted source exceeds driver source bound")
    return source


class _Trap(Exception):
    def __init__(self, kind, site):
        self.kind, self.site = kind, site


class _Meter:
    def __init__(self, limits):
        self.limits = limits
        self.fuel = self.cells = 0
        self.events, self.total = [], 0

    def event(self, event, site, **extra):
        self.total += 1
        if len(self.events) < 256:
            self.events.append({"event": event, "site": site, **extra})

    def charge(self, kind, amount, site):
        used = self.fuel if kind == "instructions" else self.cells
        self.event(kind, site, amount=amount)
        if used + amount > self.limits[kind]:
            raise _Trap(kind, site)
        if kind == "instructions":
            self.fuel += amount
        else:
            self.cells += amount


def oracle(program, call):
    """Evaluate independent checked mathematics and first observable trap order.

    The simple-binary atomic debit, lazy Boolean jumps, and branch forward jump
    are explicit public metering contracts, not extracted VM instructions.
    A witness site belongs to this AST only; Error exposes no runtime location.
    """
    validate_program(program)
    validate_call(call)
    meter, env = _Meter(call["limits"]), {}
    types = declarations(program)

    def atom(expr, site):
        if expr["op"] == "var":
            if expr["name"] not in env:
                meter.event("uninitialized", site)
                raise _Trap("uninitialized", site)
            return env[expr["name"]]
        return expr["value"]

    def expression(expr, site):
        op = expr["op"]
        if op in ("int", "bool", "var"):
            meter.charge("instructions", 1, site)
            return atom(expr, site)
        if op == "not":
            value = expression(expr["value"], site + "/value")
            meter.charge("instructions", 1, site)
            return not value
        if op in ("and", "or"):
            left = expression(expr["left"], site + "/left")
            meter.charge("instructions", 1, site + "/branch")
            short = (not left) if op == "and" else left
            meter.event("short_circuit", site, taken=bool(short))
            if short:
                meter.charge("instructions", 1, site + "/constant")
                return left
            value = expression(expr["right"], site + "/right")
            meter.charge("instructions", 1, site + "/jump")
            return value
        left_expr, right_expr = expr["left"], expr["right"]
        if left_expr["op"] in ("int", "bool", "var") and right_expr["op"] in ("int", "bool", "var"):
            meter.charge("instructions", 3, site + "/atomic_binary")
            left, right = atom(left_expr, site + "/left"), atom(right_expr, site + "/right")
        else:
            left = expression(left_expr, site + "/left")
            right = expression(right_expr, site + "/right")
            meter.charge("instructions", 1, site)
        if op == "eq":
            return left == right
        if op == "lt":
            return left < right
        if op == "le":
            return left <= right
        if op in ("div", "rem"):
            if right == 0 or (left == I64_MIN and right == -1):
                meter.event("arithmetic", site)
                raise _Trap("arithmetic", site)
            quotient = abs(left) // abs(right)
            if (left < 0) != (right < 0):
                quotient = -quotient
            value = quotient if op == "div" else left - quotient * right
        elif op == "add":
            value = left + right
        elif op == "sub":
            value = left - right
        else:
            value = left * right
        if not I64_MIN <= value <= I64_MAX:
            meter.event("arithmetic", site)
            raise _Trap("arithmetic", site)
        return value

    def block(items, site):
        for index, stmt in enumerate(items):
            here = site + "/" + str(index)
            if stmt["op"] == "set":
                value = expression(stmt["value"], here + "/value")
                meter.charge("instructions", 1, here + "/store")
                env[stmt["name"]] = value
            elif stmt["op"] == "return":
                value = expression(stmt["value"], here + "/value")
                meter.charge("instructions", 1, here + "/return")
                return value
            else:
                condition = expression(stmt["condition"], here + "/condition")
                meter.charge("instructions", 1, here + "/branch")
                arm = "then" if condition else "else"
                meter.event("branch", here, arm=arm)
                block(stmt[arm], here + "/" + arm)
                if condition:
                    meter.charge("instructions", 1, here + "/jump")
        return None

    try:
        meter.event("arity", "boundary")
        if call["inputs"] or len(call["arguments"]) != len(program["params"]):
            raise _Trap("arity", "boundary")
        for index, (decl, arg) in enumerate(zip(program["params"], call["arguments"])):
            site = "arguments/" + str(index)
            meter.charge("cells", 1, site)
            if scalar_type(arg) != decl["type"]:
                meter.event("type", site)
                raise _Trap(decl["type"], site)
            env[decl["name"]] = next(iter(arg.values()))
        meter.event("frames", "frame")
        if call["limits"]["frames"] == 0:
            raise _Trap("frames", "frame")
        meter.charge("cells", len(program["params"]) + len(program["locals"]) + 1, "frame")
        value = block(program["body"], "body")
        meter.charge("cells", 1, "result")
        tag = "i64" if program["result"] == "int" else "bool"
        outcome = {"value": {tag: value}}
        first_trap = None
    except _Trap as trap:
        outcome = {"trap": TRAPS[trap.kind]}
        first_trap = {"kind": trap.kind, "site": trap.site}
    return {"outcome": outcome, "witness": {"first_trap": first_trap,
            "instructions_used": meter.fuel, "cells_used": meter.cells,
            "events": meter.events, "event_count": meter.total,
            "trace_truncated": meter.total > len(meter.events)}}


class SplitMix64:
    """Specified integer PRNG: seed replay does not depend on random.py."""
    def __init__(self, seed):
        self.state = integer(seed, 0, MASK64, "seed")

    def next(self):
        self.state = (self.state + 0x9E3779B97F4A7C15) & MASK64
        value = self.state
        value = ((value ^ (value >> 30)) * 0xBF58476D1CE4E5B9) & MASK64
        value = ((value ^ (value >> 27)) * 0x94D049BB133111EB) & MASK64
        return value ^ (value >> 31)

    def choose(self, items):
        return items[self.next() % len(items)]


EDGES = [I64_MIN, I64_MIN + 1, -3037000500, -97, -3, -2, -1, 0, 1, 2, 3,
         97, 3037000500, I64_MAX - 1, I64_MAX]


def generate_cases(seed, count, max_depth=4, calls_per_case=8):
    integer(count, 1, 256, "program count")
    integer(max_depth, 0, 6, "max depth")
    integer(calls_per_case, 1, 32, "calls per program")
    random = SplitMix64(seed)
    cases = []
    for index in range(count):
        params = [{"name": name, "type": typ} for name, typ in
                  (("x", "int"), ("y", "int"), ("z", "int"), ("p", "bool"), ("q", "bool"))]
        locals_ = [{"name": name, "type": typ} for name, typ in
                   (("a", "int"), ("b", "int"), ("c", "bool"))]
        types = {item["name"]: item["type"] for item in params + locals_}
        remaining = [110]

        def expression(typ, depth):
            remaining[0] -= 1
            names = [name for name, actual in types.items() if actual == typ]
            if depth == 0 or remaining[0] < 16 or random.next() % 4 == 0:
                if random.next() % 3:
                    return {"op": "var", "name": random.choose(names)}
                return {"op": typ, "value": random.choose(EDGES) if typ == "int" else bool(random.next() & 1)}
            if typ == "int":
                op, child = random.choose(["add", "sub", "mul", "div", "rem"]), "int"
            else:
                op = random.choose(["eq", "lt", "le", "and", "or", "not"])
                child = "bool" if op in ("and", "or", "not") else random.choose(["int", "bool"]) if op == "eq" else "int"
                if op == "not":
                    return {"op": op, "value": expression("bool", depth - 1)}
            return {"op": op, "left": expression(child, depth - 1), "right": expression(child, depth - 1)}

        body = []
        # Most programs initialize locals; every fifth intentionally tests the
        # dynamic uninitialized-read contract, including join paths.
        for item in locals_:
            if index % 5 or random.next() & 1:
                value = {"op": "var", "name": random.choose([p["name"] for p in params if p["type"] == item["type"]])}
                body.append({"op": "set", "name": item["name"], "value": value})
        if index % 3 != 0:
            then = [{"op": "set", "name": "a", "value": expression("int", min(max_depth, 2))}]
            otherwise = [{"op": "set", "name": "b", "value": expression("int", min(max_depth, 2))}]
            if index % 3 == 2:
                otherwise = [{"op": "if", "condition": {"op": "var", "name": "q"},
                              "then": otherwise, "else": [{"op": "set", "name": "a", "value": {"op": "int", "value": 1}}]}]
            body.append({"op": "if", "condition": expression("bool", min(max_depth, 2)),
                         "then": then, "else": otherwise})
        result_type = "int" if index % 2 == 0 else "bool"
        body.append({"op": "return", "value": expression(result_type, max_depth)})
        program = {"params": params, "locals": locals_, "result": result_type, "body": body}
        validate_program(program)
        calls = []
        for call_index in range(calls_per_case):
            args = []
            for param_index in range(3):
                if call_index < 4:
                    value = random.choose(EDGES)
                elif random.next() % 3:
                    value = int(random.next() % 199) - 99
                else:
                    value = random.next() - (1 << 63)
                args.append({"i64": value})
            args += [{"bool": bool(call_index & 1)}, {"bool": bool(call_index & 2)}]
            limits = dict(DEFAULT_LIMITS)
            if call_index >= 4 and call_index % 4 == 0:
                limits["instructions"] = random.next() % 40
            elif call_index >= 4 and call_index % 4 == 1:
                limits["cells"] = random.next() % 16
            elif call_index >= 4 and call_index % 4 == 2:
                limits["frames"] = random.next() & 1
            calls.append({"arguments": args, "inputs": [], "limits": limits})
        cases.append({"id": f"seed-{seed:016x}-{index:04d}", "program": program, "calls": calls})
    validate_cases(cases)
    return cases


def load_fixtures(path=FIXTURES):
    data = read_json(path)
    fields(data, ("schema", "cases"))
    if data["schema"] != "fluxr-semantic-fixtures-v1":
        raise CampaignError("fixture schema differs")
    validate_cases(data["cases"])
    for case in data["cases"]:
        if "expected" not in case:
            raise CampaignError("fixed fixture has no hand-derived expectations")
        for call, expected in zip(case["calls"], case["expected"]):
            if not compare_outcome(expected, oracle(case["program"], call)["outcome"]):
                raise CampaignError("independent oracle disagrees with frozen fixture " + case["id"])
    return data["cases"]


def seal(value):
    value = copy.deepcopy(value)
    value.pop("integrity_sha256", None)
    value["integrity_sha256"] = digest(value)
    return value


def check_integrity(value):
    if not isinstance(value, dict) or "integrity_sha256" not in value:
        raise CampaignError("missing replay integrity binding")
    actual = {key: item for key, item in value.items() if key != "integrity_sha256"}
    if value["integrity_sha256"] != digest(actual):
        raise CampaignError("replay integrity digest differs")


def make_corpus(cases, generator=None):
    validate_cases(cases)
    sources = []
    for case in cases:
        source = emit_source(case["program"])
        sources.append({"id": case["id"], "source": source,
                        "sha256": hashlib.sha256(source.encode()).hexdigest()})
    return seal({"schema": CORPUS_SCHEMA, "generator": generator,
                 "model_version": ORACLE_VERSION,
                 "cases": cases, "sources": sources})


def validate_corpus(corpus):
    fields(corpus, ("schema", "generator", "model_version", "cases", "sources", "integrity_sha256"))
    check_integrity(corpus)
    if corpus["schema"] != CORPUS_SCHEMA:
        raise CampaignError("unsupported corpus schema")
    if corpus["model_version"] != ORACLE_VERSION:
        raise CampaignError("unsupported independent oracle model version")
    generator = corpus["generator"]
    if generator is not None:
        fields(generator, ("version", "seed", "programs", "calls_per_program", "max_depth", "fixtures_sha256"))
        if generator["version"] != GENERATOR:
            raise CampaignError("unsupported seeded generator version")
        integer(generator["seed"], 0, MASK64, "seed")
        integer(generator["programs"], 1, 256, "program count")
        integer(generator["max_depth"], 0, 6, "max depth")
        integer(generator["calls_per_program"], 1, 32, "calls per program")
        pinned = generator["fixtures_sha256"]
        if pinned is not None and (not isinstance(pinned, str) or not re.fullmatch(r"[0-9a-f]{64}", pinned)):
            raise CampaignError("invalid fixture content identity")
    expected = make_corpus(corpus["cases"], corpus["generator"])
    if canonical(expected) != canonical(corpus):
        raise CampaignError("replay source or source digest differs from its typed AST")
    # Hand-pinned fixture expectations cannot silently override the model.
    for case in corpus["cases"]:
        for index, call in enumerate(case["calls"]):
            if "expected" in case and not compare_outcome(case["expected"][index],
                                                         oracle(case["program"], call)["outcome"]):
                raise CampaignError("replay fixture expectation differs from independent oracle")


def extract_corpus(document):
    check_integrity(document)
    if document.get("schema") == CORPUS_SCHEMA:
        corpus = document
    elif document.get("schema") == REPORT_SCHEMA:
        corpus = document.get("corpus")
    else:
        raise CampaignError("expected a semantic corpus or campaign report")
    validate_corpus(corpus)
    return corpus


def external_path(path, *, new=False):
    path = Path(path).expanduser().resolve()
    if path == ROOT or ROOT in path.parents:
        raise CampaignError("generated results must be outside the source tree")
    if new and (path.exists() or path.is_symlink()):
        raise CampaignError("output already exists; choose a new external path")
    if not path.parent.is_dir():
        raise CampaignError("output parent must already exist")
    return path


def write_new(path, value):
    path = external_path(path, new=True)
    payload = canonical(value) + b"\n"
    if len(payload) > MAX_BYTES:
        raise CampaignError("serialized report exceeds 64 MiB")
    # Exclusive creation protects both previous evidence and symlink targets.
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    descriptor = os.open(path, flags, 0o600)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
    except BaseException:
        path.unlink(missing_ok=True)
        raise


def _response(data, requested):
    fields(data, ("schema", "driver", "cases"))
    if data["schema"] != "fluxr-semantic-response-v1":
        raise CampaignError("driver response schema differs")
    fields(data["driver"], ("typed_jit", "os", "arch"))
    if type(data["driver"]["typed_jit"]) is not bool \
            or not all(isinstance(data["driver"][k], str) and 0 < len(data["driver"][k]) <= 64 for k in ("os", "arch")):
        raise CampaignError("invalid driver build metadata")
    if not isinstance(data["cases"], list) or len(data["cases"]) != len(requested):
        raise CampaignError("driver case count differs")
    for row, case in zip(data["cases"], requested):
        if not isinstance(row, dict) or row.get("id") != case["id"]:
            raise CampaignError("driver case order or identity differs")
        if "compile_error" in row or "prepare_error" in row:
            error = "compile_error" if "compile_error" in row else "prepare_error"
            fields(row, ("id", error))
            if not isinstance(row[error], str) or not 0 < len(row[error]) <= 4096:
                raise CampaignError("invalid compilation/preparation diagnostic")
            continue
        fields(row, ("id", "calls"))
        if not isinstance(row["calls"], list) or len(row["calls"]) != len(case["calls"]):
            raise CampaignError("driver call count differs")
        for call in row["calls"]:
            fields(call, ("portable", "prepared", "route"))
            validate_outcome(call["portable"])
            validate_outcome(call["prepared"])
            if not isinstance(call["route"], str) or call["route"] not in ROUTES:
                raise CampaignError("unknown prepared execution route")
            if call["route"] == "TypedJit" and not data["driver"]["typed_jit"]:
                raise CampaignError("TypedJit route contradicts driver feature metadata")
            if "trap" in call["prepared"] and call["route"] != "Portable":
                raise CampaignError("native route cannot attest a portable-replayed trap")
    return data


def _observe(cases, driver, timeout):
    if isinstance(timeout, bool) or not isinstance(timeout, (float, int)) \
            or not math.isfinite(timeout) or not 0 < timeout <= 300:
        raise CampaignError("driver timeout must be finite and in (0, 300] seconds")
    driver = Path(driver).expanduser().resolve()
    if not driver.is_file() or not os.access(driver, os.X_OK):
        raise CampaignError("driver must be an explicitly selected executable file")
    before = file_digest(driver)
    observations, metadata = [], None
    # The reused collector has a stricter 2 MiB stdin limit than the bridge.
    # Split on bytes as well as row/call counts; preserve case/call order.
    batches, batch, size, calls = [], [], 0, 0
    for case in cases:
        row = {"id": case["id"], "source": emit_source(case["program"]),
               "export": "calc", "calls": case["calls"]}
        row_bytes = len(canonical(row))
        if batch and (len(batch) == 128 or size + row_bytes > 1800000 or calls + len(row["calls"]) > 4096):
            batches.append(batch)
            batch, size, calls = [], 0, 0
        batch.append(row)
        size += row_bytes
        calls += len(row["calls"])
    if batch:
        batches.append(batch)
    temporary_root = Path(tempfile.gettempdir()).resolve()
    if temporary_root == ROOT or ROOT in temporary_root.parents:
        raise CampaignError("TMPDIR must be outside the source tree")
    deadline = time.monotonic() + timeout
    with tempfile.TemporaryDirectory(prefix="fluxr-semantic-", dir=temporary_root) as directory:
        work = Path(directory)
        for batch in batches:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise CampaignError("driver campaign timeout exhausted")
            request = {"schema": "fluxr-semantic-request-v1", "cases": batch}
            run = run_bounded([str(driver)], canonical(request), work,
                              work / "stdout.json", work / "stderr.txt",
                              timeout_seconds=remaining,
                              max_stdout_bytes=8 * 1024 * 1024, max_stderr_bytes=65536)
            if run.terminal != "exited" or run.exit_code != 0:
                detail = (work / "stderr.txt").read_text(errors="replace")[:2048]
                raise CampaignError(f"driver incomplete: {run.terminal}, exit={run.exit_code}; {detail}")
            data = _response(read_json(work / "stdout.json"), batch)
            if metadata is not None and metadata != data["driver"]:
                raise CampaignError("driver build metadata changed during campaign")
            metadata = data["driver"]
            observations.extend(data["cases"])
    if file_digest(driver) != before:
        raise CampaignError("driver executable changed during campaign")
    return {"sha256": before, **metadata}, observations


def mismatch_kind(expected, observed):
    if set(expected) != set(observed):
        return "outcome_kind"
    return "trap" if "trap" in expected else "value"


def _report(corpus, driver_info, observations, *, kind="run", require_native=False):
    expected_rows, mismatches, routes = [], [], {}
    comparisons = successes = 0
    remaining_events = MAX_REPORT_EVENTS
    for case, row in zip(corpus["cases"], observations):
        models = []
        for call in case["calls"]:
            model = oracle(case["program"], call)
            witness = model["witness"]
            witness["events"] = witness["events"][:remaining_events]
            remaining_events -= len(witness["events"])
            witness["trace_truncated"] = witness["event_count"] > len(witness["events"])
            models.append(model)
        expected_rows.append({"id": case["id"], "calls": models})
        if "calls" not in row:
            key = "compile_error" if "compile_error" in row else "prepare_error"
            mismatches.append({"case": case["id"], "call": None, "backend": "compiler",
                               "kind": "compile" if key == "compile_error" else "prepare",
                               "expected": "well-typed program admitted", "observed": row[key]})
            continue
        for index, (model, observed) in enumerate(zip(models, row["calls"])):
            expected = model["outcome"]
            routes[observed["route"]] = routes.get(observed["route"], 0) + 1
            for backend in ("portable", "prepared"):
                comparisons += 1
                if not compare_outcome(expected, observed[backend]):
                    mismatches.append({"case": case["id"], "call": index, "backend": backend,
                                       "kind": mismatch_kind(expected, observed[backend]),
                                       "expected": expected, "observed": observed[backend]})
            if observed["route"] in ("TypedJit", "NativeScalar") \
                    and "value" in expected and compare_outcome(expected, observed["prepared"]):
                successes += 1
    native_missing = require_native and successes == 0
    return seal({"schema": REPORT_SCHEMA, "kind": kind, "corpus": corpus,
                 "driver": driver_info, "tool_sha256": file_digest(__file__),
                 "observations": observations, "oracles": expected_rows,
                 "mismatches": mismatches,
                 "summary": {"status": "failed" if mismatches or native_missing else "passed",
                             "cases": len(corpus["cases"]),
                             "calls": sum(len(case["calls"]) for case in corpus["cases"]),
                             "comparisons": comparisons, "mismatches": len(mismatches),
                             "native_successes": successes, "native_required": require_native,
                             "native_requirement_met": not native_missing, "routes": routes}})


def run_cases(cases, driver, timeout=30, *, generator=None, kind="run", require_native=False):
    corpus = make_corpus(cases, generator)
    validate_corpus(corpus)
    driver_info, observations = _observe(cases, driver, timeout)
    return _report(corpus, driver_info, observations, kind=kind, require_native=require_native)


def replay(document, driver, timeout=30, require_native=False):
    corpus = extract_corpus(document)
    # Historical observations are evidence only. They never supply expectations.
    report = run_cases(corpus["cases"], driver, timeout, generator=corpus["generator"],
                       kind="replay", require_native=require_native)
    report["replay_of"] = document["integrity_sha256"]
    return seal(report)


def _walk_expr(expr, path):
    yield path, expr
    if expr["op"] == "not":
        yield from _walk_expr(expr["value"], path + ("value",))
    elif expr["op"] in OPS:
        yield from _walk_expr(expr["left"], path + ("left",))
        yield from _walk_expr(expr["right"], path + ("right",))


def _walk_block(block, path=("body",)):
    for index, stmt in enumerate(block):
        here = path + (index,)
        yield here, stmt
        if stmt["op"] == "if":
            yield from _walk_block(stmt["then"], here + ("then",))
            yield from _walk_block(stmt["else"], here + ("else",))


def _at(value, path):
    for component in path:
        value = value[component]
    return value


def _replace(value, path, replacement):
    value = copy.deepcopy(value)
    _at(value, path[:-1])[path[-1]] = copy.deepcopy(replacement)
    return value


def reduction_cost(case):
    """Strictly decreasing well-founded size, then canonical byte tie-break."""
    return (len(canonical(case)), canonical(case))


def _reductions(case):
    program = case["program"]
    # Every candidate retains the final return and static scalar types.
    for path, stmt in _walk_block(program["body"]):
        if stmt["op"] != "return":
            value = copy.deepcopy(case)
            del _at(value["program"], path[:-1])[path[-1]]
            yield value
        if stmt["op"] == "if":
            for arm in ("then", "else"):
                value = copy.deepcopy(case)
                target = _at(value["program"], path[:-1])
                target[path[-1]:path[-1] + 1] = copy.deepcopy(stmt[arm])
                yield value
        key = "condition" if stmt["op"] == "if" else "value"
        types = declarations(program)
        for expr_path, expr in _walk_expr(stmt[key], path + (key,)):
            typ = expr_type(expr, types)
            alternatives = [{"op": typ, "value": 0 if typ == "int" else False},
                            {"op": typ, "value": 1 if typ == "int" else True}]
            if typ == "int":
                alternatives.append({"op": "int", "value": -1})
            for child in ("left", "right", "value"):
                if isinstance(expr.get(child), dict) and expr_type(expr[child], types) == typ:
                    alternatives.append(expr[child])
            for alternative in alternatives:
                value = copy.deepcopy(case)
                value["program"] = _replace(program, expr_path, alternative)
                yield value
    # Reducing scalar arguments preserves call sequence and workspace setup.
    for index, call in enumerate(case["calls"]):
        for lane, arg in enumerate(call["arguments"]):
            for replacement in ([{"i64": 0}, {"i64": 1}, {"i64": -1}] if "i64" in arg else [{"bool": False}]):
                value = copy.deepcopy(case)
                value["calls"][index]["arguments"][lane] = replacement
                yield value


def failure_signature(report, case_id, call_index):
    def category(outcome):
        return ("trap:" + outcome["trap"] if "trap" in outcome
                else "value:" + scalar_type(outcome["value"]))
    return sorted((item["backend"], item["kind"], category(item["expected"]),
                   category(item["observed"])) for item in report["mismatches"]
                  if item["case"] == case_id and item["call"] == call_index)


def reduce_failure(document, driver, case_id, call_index, *, max_attempts=64, timeout=60):
    integer(max_attempts, 1, 1024, "reduction attempts")
    integer(call_index, 0, 511, "call index")
    if type(timeout) not in (int, float) or not math.isfinite(timeout) or not 0 < timeout <= 300:
        raise CampaignError("reduction timeout must be in (0, 300] seconds")
    corpus = extract_corpus(document)
    selected = [case for case in corpus["cases"] if case["id"] == case_id]
    if not selected or call_index >= len(selected[0]["calls"]):
        raise CampaignError("requested reduction case or call does not exist")
    original = copy.deepcopy(selected[0])
    original.pop("expected", None)
    original["calls"] = original["calls"][:call_index + 1]
    started = time.monotonic()
    # Preserve the entire invocation prefix: a workspace-state failure may
    # require earlier success/trap calls. Prefix minimization is out of scope.
    current_report = run_cases([original], driver, timeout)
    signature = failure_signature(current_report, case_id, call_index)
    if not signature:
        raise CampaignError("selected failure does not reproduce; reduction refused")
    current, attempts, accepted, complete = original, 0, 0, False
    seen = {digest(current)}
    while attempts < max_attempts:
        improved = False
        exhausted = False
        for candidate in _reductions(current):
            if time.monotonic() - started >= timeout:
                exhausted = True
                break
            if reduction_cost(candidate) >= reduction_cost(current):
                continue
            identity = digest(candidate)
            if identity in seen:
                continue
            seen.add(identity)
            try:
                validate_cases([candidate])
            except CampaignError:
                continue
            remaining = timeout - (time.monotonic() - started)
            if attempts >= max_attempts or remaining <= 0:
                exhausted = True
                break
            attempts += 1
            observed = run_cases([candidate], driver, min(30, remaining))
            if failure_signature(observed, case_id, call_index) == signature:
                current, current_report = candidate, observed
                accepted += 1
                improved = True
                break
        if exhausted:
            break
        if not improved:
            complete = True
            break
    current_report["kind"] = "reduction"
    current_report["reduction"] = {"attempts": attempts, "max_attempts": max_attempts,
        "accepted": accepted, "complete": complete, "budget_exhausted": not complete,
        "original_cost": reduction_cost(original)[0], "reduced_cost": reduction_cost(current)[0],
        "signature": signature, "case": case_id, "call": call_index,
        "prefix_calls_retained": call_index + 1, "elapsed_seconds": time.monotonic() - started,
        "globally_minimal": False, "input_integrity_sha256": document["integrity_sha256"]}
    return seal(current_report)


def mutation_witnesses():
    """Frozen, reviewed source-level fault witnesses, never runtime patches.

    Replacing a boundary result with a literal represents that fault only at
    this counterexample; it is not a general mutant interpreter qualification.
    """
    def lit(value):
        return {"op": "bool" if type(value) is bool else "int", "value": value}

    def binary(op, left, right):
        return {"op": op, "left": left, "right": right}

    def program(expr, result="int", locals_=None):
        return {"params": [], "locals": locals_ or [], "result": result,
                "body": [{"op": "return", "value": expr}]}

    def case(name, prog):
        return {"id": name, "program": prog,
                "calls": [{"inputs": [], "arguments": [], "limits": dict(DEFAULT_LIMITS)}]}

    trap = binary("eq", binary("div", lit(1), lit(0)), lit(0))
    uninitialized = {"op": "var", "name": "u"}
    local = [{"name": "u", "type": "int"}]
    branch = {"params": [], "locals": [{"name": "a", "type": "int"}], "result": "int",
              "body": [{"op": "if", "condition": lit(True),
                        "then": [{"op": "set", "name": "a", "value": lit(4)}],
                        "else": [{"op": "set", "name": "a", "value": lit(9)}]},
                       {"op": "return", "value": {"op": "var", "name": "a"}}]}
    wrong_branch = copy.deepcopy(branch)
    wrong_branch["body"][0]["condition"] = lit(False)
    specs = [
        ("add_as_sub", program(binary("add", lit(5), lit(3))),
         program(binary("sub", lit(5), lit(3))), {"value": {"i64": 8}}),
        ("inclusive_as_strict", program(binary("le", lit(4), lit(4)), "bool"),
         program(binary("lt", lit(4), lit(4)), "bool"), {"value": {"bool": True}}),
        ("branch_inverted", branch, wrong_branch, {"value": {"i64": 4}}),
        ("eager_and", program(binary("and", lit(False), trap), "bool"),
         program(binary("and", trap, lit(False)), "bool"), {"value": {"bool": False}}),
        ("eager_or", program(binary("or", lit(True), trap), "bool"),
         program(binary("or", trap, lit(True)), "bool"), {"value": {"bool": True}}),
        ("right_before_left", program(binary("add", uninitialized, binary("div", lit(1), lit(0))), locals_=local),
         program(binary("add", binary("div", lit(1), lit(0)), uninitialized), locals_=local),
         {"trap": TRAPS["uninitialized"]}),
        ("wrapping_add", program(binary("add", lit(I64_MAX), lit(1))),
         program(lit(I64_MIN)), {"trap": TRAPS["arithmetic"]}),
        ("floor_division", program(binary("div", lit(-7), lit(3))),
         program(lit(-3)), {"value": {"i64": -2}}),
        ("floor_remainder", program(binary("rem", lit(-7), lit(3))),
         program(lit(2)), {"value": {"i64": -1}}),
    ]
    mutant_outcomes = [{"value": {"i64": 2}}, {"value": {"bool": False}},
                       {"value": {"i64": 9}}, {"trap": TRAPS["arithmetic"]},
                       {"trap": TRAPS["arithmetic"]}, {"trap": TRAPS["arithmetic"]},
                       {"value": {"i64": I64_MIN}}, {"value": {"i64": -3}},
                       {"value": {"i64": 2}}]
    return [{"name": name, "control": case(name, original), "mutant": case(name, mutant),
             "expected": expected, "mutant_expected": wrong}
            for (name, original, mutant, expected), wrong in zip(specs, mutant_outcomes)]


def mutation_checks(driver, timeout=30):
    specs = mutation_witnesses()
    controls, mutants = [item["control"] for item in specs], [item["mutant"] for item in specs]
    for item in specs:
        for side, expectation in (("control", "expected"), ("mutant", "mutant_expected")):
            if not compare_outcome(item[expectation], oracle(item[side]["program"], item[side]["calls"][0])["outcome"]):
                raise CampaignError("mutation witness disagrees with its frozen independent expectation")
    start = time.monotonic()
    control_info, control_rows = _observe(controls, driver, timeout)
    remaining = timeout - (time.monotonic() - start)
    if remaining <= 0:
        raise CampaignError("mutation deadline exhausted before mutant observations")
    mutant_info, mutant_rows = _observe(mutants, driver, remaining)
    if control_info != mutant_info:
        raise CampaignError("mutation driver identity changed between control and mutant")
    checks = []
    for item, control, mutant in zip(specs, control_rows, mutant_rows):
        baseline_passed = "calls" in control and all(compare_outcome(item["expected"], control["calls"][0][backend]) for backend in ("portable", "prepared"))
        # A refused/malformed mutant is not a detected semantic execution fault.
        detected = {backend: "calls" in mutant
                    and compare_outcome(item["mutant_expected"], mutant["calls"][0][backend])
                    and not compare_outcome(item["expected"], mutant["calls"][0][backend])
                    for backend in ("portable", "prepared")}
        checks.append({"name": item["name"], "expected": item["expected"],
                       "mutant_expected": item["mutant_expected"],
                       "control": control, "mutant": mutant, "baseline_passed": baseline_passed,
                       "detected": detected, "passed": baseline_passed and all(detected.values())})
    count = sum(item["passed"] for item in checks)
    return seal({"schema": "fluxr-semantic-mutations-v1", "driver": control_info,
                 "model_version": ORACLE_VERSION, "tool_sha256": file_digest(__file__),
                 "controls": make_corpus(controls), "mutants": make_corpus(mutants), "checks": checks,
                 "summary": {"status": "passed" if count == len(checks) else "failed",
                             "mutants": len(checks), "detected": count,
                             "scope": "controlled source-level counterexamples; no runtime/compiler mutation"}})


def _generation(args):
    cases = generate_cases(args.seed, args.programs, args.max_depth, args.calls_per_program)
    fixture_hash = None
    if not args.no_fixtures:
        cases = load_fixtures() + cases
        fixture_hash = file_digest(FIXTURES)
    return cases, {"version": GENERATOR, "seed": args.seed, "programs": args.programs,
                   "calls_per_program": args.calls_per_program, "max_depth": args.max_depth,
                   "fixtures_sha256": fixture_hash}


def build_parser():
    parser = argparse.ArgumentParser(description=__doc__)
    subs = parser.add_subparsers(dest="command", required=True)
    for command in ("generate", "run", "replay", "reduce", "mutations"):
        sub = subs.add_parser(command)
        sub.add_argument("--output", type=Path, required=True, help="new JSON file outside the source tree")
        if command in ("generate", "run"):
            sub.add_argument("--seed", type=int, default=243)
            sub.add_argument("--programs", type=int, default=64)
            sub.add_argument("--calls-per-program", type=int, default=8)
            sub.add_argument("--max-depth", type=int, default=4)
            sub.add_argument("--no-fixtures", action="store_true")
        if command != "generate":
            sub.add_argument("--driver", type=Path, required=True)
            sub.add_argument("--timeout", type=float, default=60 if command == "reduce" else 30,
                             help="whole operation deadline in seconds, maximum 300")
        if command in ("run", "replay"):
            sub.add_argument("--require-native", action="store_true")
        if command in ("replay", "reduce"):
            sub.add_argument("--input", type=Path, required=True)
        if command == "reduce":
            sub.add_argument("--case", required=True)
            sub.add_argument("--call", type=int, required=True)
            sub.add_argument("--max-attempts", type=int, default=64)
    return parser


def main(argv=None):
    args = build_parser().parse_args(argv)
    try:
        external_path(args.output, new=True)
        if args.command in ("generate", "run"):
            cases, generator = _generation(args)
            if args.command == "generate":
                result = make_corpus(cases, generator)
            else:
                result = run_cases(cases, args.driver, args.timeout, generator=generator,
                                   require_native=args.require_native)
        elif args.command == "replay":
            result = replay(read_json(args.input), args.driver, args.timeout, args.require_native)
        elif args.command == "reduce":
            result = reduce_failure(read_json(args.input), args.driver, args.case, args.call,
                                    max_attempts=args.max_attempts, timeout=args.timeout)
        else:
            result = mutation_checks(args.driver, args.timeout)
        write_new(args.output, result)
        summary = result.get("summary", {"status": "generated", "cases": len(result["cases"])}) if args.command == "generate" else result["summary"]
        print(json.dumps({"command": args.command, **summary}, sort_keys=True))
        if args.command in ("generate", "reduce"):
            return 0
        return 0 if summary["status"] == "passed" else 1
    except (CampaignError, OSError, RecursionError, OverflowError) as error:
        print("semantic campaign: " + str(error), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
