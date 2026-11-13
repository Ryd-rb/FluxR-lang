#!/usr/bin/env python3
"""FluxR 1 source compiler and unified-runtime application builder.

Every application is linked to the same Rust-implemented Runtime. Native Rust
extensions are explicit host interoperability; they never select a VM-free
execution profile. Historical native source is accepted only by import_native.
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass
import hashlib
import json
import math
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import tomllib
import time

HEADER = b"fluxr 1\n"
LEGACY_NATIVE_HEADER = b"fluxr-native 1\n"
LANGUAGE_VERSION = 1
NATIVE_DOMAIN = b"fluxr-native-bindings-v1\0"
WHITESPACE = b" \t\r\n\v\f"
ROOT = Path(__file__).resolve().parents[1]
MAX_SOURCE = 8 * 1024 * 1024
MAX_OUTPUT = 16 * 1024 * 1024
MAX_PROJECT = 64 * 1024 * 1024
MAX_FILES = 1024
MAX_NODES = 100_000
MAX_DEPTH = 128
MAX_COUNT = 10_000
MAX_DECIMAL_FLOAT_BYTES = 256
IDENT = re.compile(r"(?:r#)?[A-Za-z_][A-Za-z_0-9]*\Z")
PATH = re.compile(r"(?:::)?(?:r#)?[A-Za-z_][A-Za-z_0-9]*(?:::(?:r#)?[A-Za-z_][A-Za-z_0-9]*)*\Z")
INTEGER = re.compile(r"-?(?:0|[1-9][0-9]*)(?:[iu](?:8|16|32|64|128|size))?\Z")
TYPE = re.compile(r"[A-Za-z_0-9:#<>, &'\[\]();!*+\-]+\Z")
CHECKED_DECIMAL_FLOAT = re.compile(r"-?[0-9]+(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?f\Z")
BINARY = {"add": "+", "sub": "-", "mul": "*", "div": "/", "rem": "%",
          "eq": "==", "ne": "!=", "lt": "<", "le": "<=", "gt": ">", "ge": ">=",
          "and": "&&", "or": "||", "bitand": "&", "bitor": "|", "xor": "^",
          "shl": "<<", "shr": ">>", "range": "..", "range-inclusive": "..="}


class NativeError(ValueError):
    pass


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def read_regular(path: Path, limit: int = MAX_SOURCE) -> bytes:
    """No symlinks, devices, FIFOs or unbounded allocation."""
    fd = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_NONBLOCK", 0))
    with os.fdopen(fd, "rb") as stream:
        if not stat.S_ISREG(os.fstat(stream.fileno()).st_mode):
            raise NativeError(f"not a regular file: {path}")
        data = stream.read(limit + 1)
    if len(data) > limit:
        raise NativeError(f"file exceeds {limit} bytes: {path}")
    return data


def no_duplicates(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise NativeError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


class Parser:
    def __init__(self, source: bytes):
        if not isinstance(source, bytes) or len(source) > MAX_SOURCE:
            raise NativeError("native source exceeds byte limit")
        try:
            source.decode("utf-8")
        except UnicodeDecodeError as error:
            raise NativeError("source is not UTF-8") from error
        if not source.startswith(HEADER):
            raise NativeError("expected exact fluxr 1 header; migrate historical source explicitly")
        self.source, self.pos, self.nodes, self.depth = source, len(HEADER), 0, 0
        self.raw_bytes = 0

    def fail(self, message):
        raise NativeError(f"FluxR byte {self.pos}: {message}")

    def space(self):
        while self.pos < len(self.source) and self.source[self.pos] in WHITESPACE:
            self.pos += 1

    def token(self):
        self.space()
        start = self.pos
        if start == len(self.source):
            self.fail("unexpected end of source")
        if self.source[start] == 34:
            self.pos += 1
            escaped = False
            while self.pos < len(self.source):
                c = self.source[self.pos]
                self.pos += 1
                if c == 34 and not escaped:
                    break
                escaped = c == 92 and not escaped
            else:
                self.fail("unterminated JSON string")
            if self.pos < len(self.source) and self.source[self.pos] not in WHITESPACE:
                self.fail("quoted token must end at whitespace")
            try:
                text = json.loads(self.source[start:self.pos])
                text.encode("utf-8")
            except (ValueError, UnicodeError) as error:
                self.fail(f"invalid string: {error}")
            return text
        while self.pos < len(self.source) and self.source[self.pos] not in WHITESPACE:
            self.pos += 1
        return self.source[start:self.pos].decode("utf-8")

    def count(self, maximum=MAX_COUNT):
        token = self.token()
        if not re.fullmatch(r"0|[1-9][0-9]{0,7}", token):
            self.fail("invalid count")
        value = int(token)
        if value > maximum:
            self.fail("count exceeds limit")
        return value

    def identifier(self):
        value = self.token()
        if not IDENT.fullmatch(value):
            self.fail("invalid identifier")
        return value

    def path(self):
        value = self.token()
        if not PATH.fullmatch(value):
            self.fail("invalid path; use an explicit rust fragment for qualified generic paths")
        return value

    def ty(self):
        value = self.token()
        if value == "unit":
            return "()"
        if not TYPE.fullmatch(value) or len(value) > 4096:
            self.fail("invalid type fragment; use an explicit rust item for arbitrary Rust syntax")
        # Types are compiler-checked fragments. Forbid statement/item delimiters;
        # semicolons are only useful within array types, so validate their depth.
        stack = []
        for char in value:
            if char in "[(<":
                stack.append(char)
            elif char in "]>)":
                if char == ">" and stack and stack[-1] != "<":
                    self.fail("unbalanced type")
                if not stack or stack.pop() != {"]": "[", ">": "<", ")": "("}[char]:
                    self.fail("unbalanced type")
            elif char == ";" and (not stack or stack[-1] != "["):
                self.fail("semicolon outside array type")
        if stack:
            self.fail("unbalanced type")
        return value

    def raw(self):
        size = self.count(MAX_SOURCE)
        if self.source[self.pos:self.pos + 2] == b"\r\n":
            self.pos += 2
        elif self.source[self.pos:self.pos + 1] == b"\n":
            self.pos += 1
        else:
            self.fail("rust byte count must be immediately followed by newline")
        end = self.pos + size
        if end > len(self.source):
            self.fail("truncated rust fragment")
        data = self.source[self.pos:end]
        self.pos = end
        try:
            result = data.decode("utf-8")
        except UnicodeDecodeError:
            self.fail("rust byte length splits UTF-8")
        self.raw_bytes += size
        return result

    def enter(self):
        self.nodes += 1
        self.depth += 1
        if self.nodes > MAX_NODES or self.depth > MAX_DEPTH:
            self.fail("syntax node/depth limit exceeded")

    def expr(self):
        self.enter()
        try:
            op = self.token()
            if op == "rust":
                return "(" + self.raw() + ")"
            if op == "path":
                return self.path()
            if op == "int":
                value = self.token()
                if not INTEGER.fullmatch(value) or len(value) > 80:
                    self.fail("invalid integer literal")
                return value
            if op == "str":
                # Rust and JSON share most escapes, but Rust has no \\uXXXX.
                value = self.token()
                return '"' + ''.join({"\\": "\\\\", '"': '\\"', "\n": "\\n", "\r": "\\r", "\t": "\\t", "\0": "\\0"}.get(c,
                    f"\\u{{{ord(c):x}}}" if ord(c) < 32 or ord(c) == 127 else c) for c in value) + '"'
            if op == "bool":
                value = self.token()
                if value not in {"true", "false"}:
                    self.fail("invalid boolean")
                return value
            if op == "unit":
                return "()"
            if op == "call":
                name = self.path()
                return name + "(" + ", ".join(self.expr() for _ in range(self.count())) + ")"
            if op == "method":
                receiver, name = self.expr(), self.identifier()
                return f"({receiver}).{name}(" + ", ".join(self.expr() for _ in range(self.count())) + ")"
            if op == "macro":
                name = self.path()
                return name + "!(" + ", ".join(self.expr() for _ in range(self.count())) + ")"
            if op in {"ref", "mutref", "deref", "not", "neg"}:
                return "(" + {"ref": "&", "mutref": "&mut ", "deref": "*", "not": "!", "neg": "-"}[op] + "(" + self.expr() + "))"
            if op in {"try", "await"}:
                return "(" + self.expr() + ")" + {"try": "?", "await": ".await"}[op]
            if op == "cast":
                value, ty = self.expr(), self.ty()
                return f"(({value}) as {ty})"
            if op == "field":
                value, field = self.expr(), self.identifier()
                return f"({value}).{field}"
            if op == "index":
                value, index = self.expr(), self.expr()
                return f"({value})[{index}]"
            if op in {"array", "tuple"}:
                values = [self.expr() for _ in range(self.count())]
                if op == "array":
                    return "[" + ", ".join(values) + "]"
                return "(" + ", ".join(values) + ("," if len(values) == 1 else "") + ")"
            if op == "bin":
                kind = self.token()
                if kind not in BINARY:
                    self.fail("unknown binary operator")
                left, right = self.expr(), self.expr()
                return f"(({left}) {BINARY[kind]} ({right}))"
            if op == "if":
                cond, yes, no = self.expr(), self.expr(), self.expr()
                return f"(if {cond} {{ {yes} }} else {{ {no} }})"
            if op == "block":
                body, tail = self.body(), self.expr()
                return "{\n" + body + tail + "\n}"
            if op in {"move", "closure"}:
                names = [self.identifier() for _ in range(self.count())]
                if len(set(names)) != len(names):
                    self.fail("duplicate closure parameter")
                return ("move " if op == "move" else "") + "|" + ", ".join(names) + "| " + self.expr()
            if op in {"unsafe", "async", "async-move"}:
                return {"unsafe": "unsafe", "async": "async", "async-move": "async move"}[op] + " { " + self.expr() + " }"
            self.fail(f"unknown expression {op!r}")
        finally:
            self.depth -= 1

    def statement(self):
        self.enter()
        try:
            op = self.token()
            if op == "rust":
                return self.raw() + "\n"
            if op == "let":
                name, value = self.identifier(), self.expr()
                return f"let {name} = {value};\n"
            if op in {"var", "let-type"}:
                name, ty, value = self.identifier(), self.ty(), self.expr()
                return f"let {'mut ' if op == 'var' else ''}{name}: {ty} = {value};\n"
            if op == "set":
                left, right = self.expr(), self.expr()
                return f"{left} = {right};\n"
            if op in {"do", "return", "tail"}:
                value = self.expr()
                return ("return " if op == "return" else "") + value + ("\n" if op == "tail" else ";\n")
            if op in {"break", "continue"}:
                return op + ";\n"
            if op == "for":
                name, sequence, body = self.identifier(), self.expr(), self.body()
                return f"for {name} in {sequence} {{\n{body}}}\n"
            if op == "while":
                cond, body = self.expr(), self.body()
                return f"while {cond} {{\n{body}}}\n"
            if op == "if-stmt":
                cond, yes, no = self.expr(), self.body(), self.body()
                return f"if {cond} {{\n{yes}}} else {{\n{no}}}\n"
            self.fail(f"unknown statement {op!r}")
        finally:
            self.depth -= 1

    def body(self):
        return "".join(self.statement() for _ in range(self.count()))

    def native_item(self, op):
        if op == "rust":
            return self.raw()
        if op == "native-use":
            return "use " + self.path() + ";\n"
        if op not in {"native-fn", "native-pubfn", "native-asyncfn", "native-pubasyncfn"}:
            self.fail(f"unknown native extension {op!r}")
        kind = op.removeprefix("native-")
        name = self.identifier()
        params = [(self.identifier(), self.ty()) for _ in range(self.count())]
        if len({p[0] for p in params}) != len(params):
            self.fail("duplicate function parameter")
        result, body = self.ty(), self.body()
        qualifier = ("pub " if kind.startswith("pub") else "") + ("async " if "async" in kind else "")
        return qualifier + f"fn {name}(" + ", ".join(n + ": " + t for n, t in params) + f") -> {result} {{\n{body}}}\n"


@dataclass
class Document:
    source: bytes
    checked: bytes
    native: bytes
    native_digest: str
    native_spans: list[tuple[int, int]]
    checked_exports: list[str]


class SourceParser(Parser):
    """One document tokenizer; native extensions and checked declarations share it.

    This syntactic admission does not replace the Rust Runtime's semantic
    checker. The identical source is embedded and admitted there before main.
    Native spans use the same framed digest as the Runtime loader.
    Decimal floats are checked for spelling and byte length only: Python never
    converts them through binary64. Projection-only emit/export may retain a
    semantically overflowing literal; build runs the Rust semantic compiler
    before compiling the application, and embedded startup checks it again.
    """

    def word(self):
        self.space()
        start = self.pos
        while self.pos < len(self.source) and self.source[self.pos] not in WHITESPACE:
            self.pos += 1
        token = self.source[start:self.pos]
        if not token or not token.isascii():
            self.fail("checked tokens must be nonempty ASCII")
        return token.decode("ascii")

    def checked_count(self, maximum=32768):
        word = self.word()
        if not re.fullmatch(r"0|[1-9][0-9]{0,7}", word) or int(word) > maximum:
            self.fail("invalid checked count")
        return int(word)

    def checked_name(self):
        name = self.word()
        if len(name) > 64 or not re.fullmatch(r"[A-Za-z_][A-Za-z_0-9]*", name):
            self.fail("invalid checked identifier")
        return name

    def checked_type(self, depth=0, first=None):
        if depth >= 32:
            self.fail("checked type depth limit")
        token = self.word() if first is None else first
        if token in {"unit", "bool", "int", "uint", "float"}:
            return
        if token in {"record", "enum"}:
            self.checked_name()
        elif token == "list":
            self.checked_type(depth + 1)
        else:
            self.fail("invalid checked type")

    def checked_bindings(self):
        for _ in range(self.checked_count(256)):
            self.checked_name()
            self.checked_type()

    def checked_expr(self):
        self.enter()
        try:
            op = self.word()
            if op in {"add", "sub", "mul", "div", "rem", "eq", "lt", "le", "and", "or", "at"}:
                self.checked_expr(); self.checked_expr()
            elif op in {"not", "abs", "tof", "i64tof", "ftoi64", "sqrt", "empty", "last", "minidx"}:
                self.checked_expr()
            elif op in {"local", "input"}:
                self.checked_name()
            elif op in {"new", "call"}:
                self.checked_name()
                for _ in range(self.checked_count()): self.checked_expr()
            elif op in {"field", "is", "payload"}:
                self.checked_expr(); self.checked_name()
            elif op in {"tag", "wrap"}:
                self.checked_name(); self.checked_name()
                if op == "wrap": self.checked_expr()
            elif op == "list":
                self.checked_type()
                for _ in range(self.checked_count()): self.checked_expr()
            elif op == "m":
                self.checked_expr(); self.checked_name()
                for _ in range(self.checked_count()): self.checked_expr()
            elif op == "host":
                self.checked_name(); self.checked_type()
                for _ in range(self.checked_count(256)):
                    self.checked_type(); self.checked_expr()
            elif op not in {"unit", "true", "false"}:
                if op.endswith('f') and op[0] in '-0123456789' and not op.startswith('0x'):
                    # word() admits ASCII, so character and byte lengths agree.
                    # Rust owns rounding, signed-zero bits and finite overflow;
                    # preserve the original token for its semantic admission.
                    if len(op) > MAX_DECIMAL_FLOAT_BYTES:
                        self.fail("decimal float literal byte limit")
                    if not CHECKED_DECIMAL_FLOAT.fullmatch(op):
                        self.fail("invalid checked expression atom")
                elif not re.fullmatch(r"-?[0-9]+u?|0x[0-9A-Fa-f]+f|[A-Za-z_][A-Za-z_0-9]*", op):
                    self.fail("invalid checked expression atom")
        finally:
            self.depth -= 1

    def checked_block(self):
        self.enter()
        try:
            for _ in range(self.checked_count()):
                op = self.word()
                if op in {"set", "push"}:
                    self.checked_name(); self.checked_expr()
                elif op in {"bump", "scale", "sub", "div", "rem"}:
                    self.checked_name()
                    # Match the expanded Binary + Local syntax depth/charges.
                    # Rust remains the authoritative semantic and resource gate.
                    self.enter()
                    try:
                        self.enter()
                        self.depth -= 1
                        self.checked_expr()
                    finally:
                        self.depth -= 1
                elif op in {"next", "prev"}:
                    self.checked_name()
                    # Signed literal-one updates have the expanded Binary,
                    # Local and I64 leaves even though no RHS text is present.
                    self.enter()
                    try:
                        for _ in range(2):
                            self.enter()
                            self.depth -= 1
                    finally:
                        self.depth -= 1
                elif op == "put":
                    self.checked_name(); self.checked_name(); self.checked_expr()
                elif op == "pop":
                    self.checked_name()
                elif op == "if":
                    self.checked_expr(); self.checked_block(); self.checked_block()
                elif op == "while":
                    self.checked_expr(); self.checked_block()
                elif op == "ret":
                    self.checked_expr()
                elif op not in {"break", "continue"}:
                    self.fail("invalid checked statement")
        finally:
            self.depth -= 1

    def checked_item(self, op):
        export = None
        if op == "inputs":
            self.checked_bindings()
        elif op == "rec":
            self.checked_name(); self.checked_bindings()
        elif op == "enum":
            self.checked_name()
            for _ in range(self.checked_count(256)):
                self.checked_name(); ty = self.word()
                if ty != "none": self.checked_type(first=ty)
        elif op in {"fn", "pub", "method"}:
            if op == "method": self.checked_name()
            name = self.checked_name()
            if op == "pub": export = name
            self.checked_bindings(); self.checked_type()
            kind = self.word()
            if kind == "ret": self.checked_expr()
            elif kind == "do": self.checked_bindings(); self.checked_block()
            else: self.fail("checked function requires ret or do body")
        else:
            self.fail(f"unknown FluxR declaration {op!r}")
        return export

    def document(self):
        checked, extensions, spans, exports = [], [], [], []
        digest = hashlib.sha256(NATIVE_DOMAIN)
        while True:
            self.space()
            if self.pos == len(self.source): break
            self.nodes += 1
            if self.nodes > MAX_NODES: self.fail("source node limit")
            start = self.pos
            op = self.word()
            if op == "rust" or op.startswith("native-"):
                extensions.append(self.native_item(op))
                span = self.source[start:self.pos]
                digest.update(len(span).to_bytes(8, "little")); digest.update(span)
                spans.append((start, self.pos))
            else:
                export = self.checked_item(op)
                checked.append(self.source[start:self.pos])
                if export is not None: exports.append(export)
        native = "\n".join(extensions).encode()
        if len(native) > MAX_OUTPUT: self.fail("generated native items exceed limit")
        return Document(self.source, b"\n".join(checked), native, digest.hexdigest(), spans, exports)


def parse_source(source: bytes) -> Document:
    return SourceParser(source).document()


def import_native(source: bytes) -> bytes:
    """Explicitly migrate v15's source encoding without retaining its runtime fork."""
    if not source.startswith(LEGACY_NATIVE_HEADER):
        raise NativeError("expected historical fluxr-native 1 input")
    parser = Parser(HEADER + source[len(LEGACY_NATIVE_HEADER):])
    parts = [HEADER]
    while True:
        parser.space()
        if parser.pos == len(parser.source): break
        start = parser.pos
        op = parser.token()
        if op == "use":
            parser.path()
            parts.append(b"native-" + parser.source[start:parser.pos] + b"\n")
        elif op == "rust":
            parser.raw()
            parts.append(parser.source[start:parser.pos] + b"\n")
        elif op in {"fn", "pubfn", "asyncfn", "pubasyncfn"}:
            parser.native_item("native-" + op)
            parts.append(b"native-" + parser.source[start:parser.pos] + b"\n")
        else:
            parser.fail("unknown historical native item")
    result = b"".join(parts)
    parse_source(result)
    return result


def wrap_rust(source: bytes) -> bytes:
    """Declare foreign Rust items inside the one FluxR source version."""
    source.decode("utf-8")
    result = HEADER + b"rust " + str(len(source)).encode() + b"\n" + source
    if len(result) > MAX_SOURCE: raise NativeError("wrapped source exceeds limit")
    return result


def rust_string(value: str) -> str:
    return '"' + ''.join({"\\": "\\\\", '"': '\\"', "\n": "\\n", "\r": "\\r", "\t": "\\t", "\0": "\\0"}.get(
        c, f"\\u{{{ord(c):x}}}" if ord(c) < 32 or ord(c) == 127 else c) for c in value) + '"'


def split_inner_attributes(source: bytes) -> tuple[bytes, bytes]:
    """Preserve crate attributes, while token-tree instrumentation owns main.

    Only the leading Rust prelude is scanned. Strings (including raw strings),
    nested comments and bracketed cfg_attr payloads cannot hide entry overrides.
    Rust still parses and checks all actual items and attributes.
    """
    text = source.decode(); pos = 0; prefix_end = 0

    def comment(i):
        if text.startswith("//", i):
            end = text.find("\n", i)
            return len(text) if end < 0 else end + 1
        if text.startswith("/*", i):
            depth = 1; i += 2
            while depth and i < len(text):
                if text.startswith("/*", i): depth += 1; i += 2
                elif text.startswith("*/", i): depth -= 1; i += 2
                else: i += 1
            if depth: raise NativeError("unterminated Rust comment")
            return i
        return None

    def literal(i):
        raw = re.match(r'(?:br|cr|r)(#{0,255})"', text[i:])
        if raw:
            end = text.find('"' + raw[1], i + len(raw[0]))
            if end < 0: raise NativeError("unterminated raw Rust attribute string")
            return end + 1 + len(raw[1])
        opening = i + 1 if text[i:i + 2] in {'b"', 'c"', "b'"} else i
        if text[opening:opening + 1] not in {'"', "'"}: return None
        quote = text[opening]
        if quote == "'" and not re.match(r"'(?:[^'\\]|\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|.))'", text[opening:]):
            return None  # Rust lifetime, not a character literal.
        j = opening + 1
        while j < len(text):
            if text[j] == "\\": j += 2
            elif text[j] == quote: return j + 1
            else: j += 1
        raise NativeError("unterminated Rust attribute string")

    while True:
        while pos < len(text):
            if text[pos].isspace(): pos += 1; continue
            # An outer doc comment belongs to the following native item.
            if text.startswith("///", pos) and not text.startswith("////", pos): break
            if text.startswith("/**", pos) and not text.startswith("/***", pos): break
            end = comment(pos)
            if end is None: break
            pos = end
        if not text.startswith("#![", pos): break
        pos += 3; depth = 1
        while pos < len(text) and depth:
            end = comment(pos)
            if end is not None: pos = end; continue
            end = literal(pos)
            if end is not None: pos = end; continue
            identifier = re.match(r"(?:r#)?[A-Za-z_][A-Za-z_0-9]*", text[pos:])
            if identifier:
                if identifier[0].removeprefix("r#") in {"no_main", "no_std"}:
                    raise NativeError("no_main/no_std bypass unified Runtime startup and are unsupported")
                pos += len(identifier[0]); continue
            if text[pos] == "[": depth += 1
            elif text[pos] == "]": depth -= 1
            pos += 1
        if depth: raise NativeError("unterminated crate attribute")
        prefix_end = pos
    if text.startswith("#!", pos): raise NativeError("unsupported crate prelude")
    # Preserve leading inner documentation even when no explicit attribute exists.
    if prefix_end == 0 and ("//!" in text[:pos] or "/*!" in text[:pos]): prefix_end = pos
    return text[:prefix_end].encode(), text[prefix_end:].encode()


def emit(source: bytes, *, name="main.fluxr", role="binary", catalog=None, aot_initializer="") -> bytes:
    """Project syntax into runtime-bound Rust without claiming semantic validity.

    build separately invokes the Rust checked compiler before application
    compilation. Direct emit/export retain source bytes for runtime admission
    before user main, including structurally valid decimal overflow spellings.
    """
    document = parse_source(source)
    if role not in {"binary", "module", "test", "build-script"}: raise NativeError("invalid emission role")
    if role == "build-script":
        if document.checked: raise NativeError("foreign build scripts cannot contain checked application declarations")
        return document.native
    records = [(name, document)] if catalog is None else catalog
    prefix, native = split_inner_attributes(document.native)
    if not native.strip() and role == "binary":
        if "main" not in document.checked_exports:
            raise NativeError("checked executable requires an exported zero-argument main")
        native = ("fn main() -> Result<(), ::fluxr::runtime::RuntimeError> { "
                  "::fluxr::runtime::run_embedded_entry(" + rust_string(name) + ', "main") }\n').encode()
    catalog_text = ",\n".join("::fluxr::runtime::EmbeddedSource { name: " + rust_string(n) +
        ", source: " + rust_string(d.source.decode()) + ", native_digest: " + rust_string(d.native_digest) + " }" for n, d in records)
    initializer = aot_initializer or ""
    generated = prefix + b"\n::fluxr::program! {\n" + role.encode() + b";\n[\n" + catalog_text.encode() + b"\n];\n{" + initializer.encode() + b"};\n{\n" + native + b"\n}\n}\n"
    if len(generated) > MAX_OUTPUT: raise NativeError("runtime-bound emission exceeds limit")
    return generated


def relpath(value):
    if not isinstance(value, str) or not value or "\\" in value or "\0" in value or ":" in value:
        raise NativeError("invalid project-relative path")
    parts = value.split("/")
    if any(p in {"", ".", ".."} or p.endswith((" ", ".")) for p in parts) or PurePosixPath(value).is_absolute():
        raise NativeError("invalid project-relative path")
    return Path(*parts)


def beneath(root: Path, relative: Path):
    current = root
    for piece in relative.parts:
        current = current / piece
        if current.is_symlink(): raise NativeError("project path traverses a symlink")
    return current


def runtime_root():
    bundled = Path(__file__).resolve().parent / "runtime"
    return bundled if bundled.is_dir() else ROOT


def runtime_snapshot():
    root = runtime_root()
    paths = [Path("Cargo.toml"), Path("Cargo.lock")]
    for directory in ("src", "crates/fluxr-entry"):
        base = root / directory
        if not base.is_dir(): raise NativeError("unified Runtime source is unavailable: " + directory)
        paths.extend(p.relative_to(root) for p in sorted(base.rglob("*")) if p.is_file())
    for name in ("LICENSE", "LICENSE-MIT"):
        if (root / name).is_file(): paths.append(Path(name))
    files = {}
    total = 0
    for relative in paths:
        if "target" in relative.parts or "__pycache__" in relative.parts: continue
        data = read_regular(beneath(root, relative))
        total += len(data)
        if total > MAX_PROJECT: raise NativeError("Runtime source bundle exceeds limit")
        files[relative.as_posix()] = data
    for required in ("src/runtime.rs", "crates/fluxr-entry/src/lib.rs"):
        if required not in files: raise NativeError("Runtime bundle lacks " + required)
    return files


def cargo_targets(cargo: dict, paths: set[str], *, testing=False):
    package = cargo.get("package")
    if not isinstance(package, dict) or not isinstance(package.get("name"), str):
        raise NativeError("a single explicit Cargo package is required")
    if any(k in cargo for k in ("workspace", "patch", "replace")):
        raise NativeError("workspaces and Cargo dependency overrides require explicit migration before unified building")
    if package["name"] in {"fluxr", "fluxr-entry"}: raise NativeError("reserved Runtime package name")
    dependencies = cargo.get("dependencies", {})
    if not isinstance(dependencies, dict) or "fluxr" in dependencies or "fluxr-entry" in dependencies:
        raise NativeError("Runtime dependencies are supplied by the unified builder")
    for section in ("build-dependencies", "dev-dependencies"):
        if "fluxr" in cargo.get(section, {}) or "fluxr-entry" in cargo.get(section, {}):
            raise NativeError("Runtime dependency aliases are reserved")
    for target in cargo.get("target", {}).values():
        if any("fluxr" in target.get(section, {}) or "fluxr-entry" in target.get(section, {}) for section in ("dependencies", "build-dependencies", "dev-dependencies")):
            raise NativeError("target-specific Runtime override is forbidden")
    roles = {}
    def add(path, role):
        path = relpath(path).as_posix()
        if path not in paths: raise NativeError("Cargo target is absent from the source manifest: " + path)
        if path in roles and roles[path] != role: raise NativeError("one source cannot serve incompatible Cargo target roles: " + path)
        roles[path] = role
    build = package.get("build", "build.rs" if "build.rs" in paths else False)
    if build is not False:
        if not isinstance(build, str): raise NativeError("Cargo build path must be a string or false")
        add(build, "build-script")
    binary_paths = set()
    explicit = cargo.get("bin", [])
    if not isinstance(explicit, list): raise NativeError("Cargo bin entries must be an array")
    for entry in explicit:
        if not isinstance(entry, dict) or not isinstance(entry.get("name"), str): raise NativeError("explicit binary needs a name")
        path = entry.get("path")
        if path is None:
            name = entry["name"]
            candidates = (["src/main.rs"] if name == package["name"] else []) + [f"src/bin/{name}.rs", f"src/bin/{name}/main.rs"]
            available = [p for p in candidates if p in paths]
            if len(available) != 1: raise NativeError("explicit binary requires an unambiguous path")
            path = available[0]
        binary_paths.add(relpath(path).as_posix())
        add(path, "test" if testing and entry.get("test", True) and entry.get("harness", True) else "binary")
    if package.get("autobins", True):
        for path in sorted(paths):
            parts = PurePosixPath(path).parts
            auto = path == "src/main.rs" or (len(parts) == 3 and parts[:2] == ("src", "bin") and path.endswith(".rs")) or (len(parts) == 4 and parts[:2] == ("src", "bin") and parts[-1] == "main.rs")
            if auto and path not in binary_paths:
                add(path, "test" if testing else "binary"); binary_paths.add(path)
    library = cargo.get("lib")
    if library is not None:
        if not isinstance(library, dict): raise NativeError("invalid Cargo library target")
        add(library.get("path", "src/lib.rs"), "test" if testing and library.get("test", True) and library.get("harness", True) else "module")
    elif "src/lib.rs" in paths and package.get("autolib", True):
        add("src/lib.rs", "test" if testing else "module")
    for kind, directory, default_role in (("test", "tests", "test"), ("bench", "benches", "test"), ("example", "examples", "binary")):
        explicit_paths = set()
        for entry in cargo.get(kind, []):
            if not isinstance(entry, dict) or not isinstance(entry.get("name"), str): raise NativeError("invalid explicit Cargo target")
            name = entry["name"]
            candidates = [f"{directory}/{name}.rs", f"{directory}/{name}/main.rs"]
            path = entry.get("path")
            if path is None:
                found = [p for p in candidates if p in paths]
                if len(found) != 1: raise NativeError("Cargo target needs an explicit unambiguous path")
                path = found[0]
            explicit_paths.add(relpath(path).as_posix())
            add(path, default_role if entry.get("harness", True) else "binary")
        if package.get({"test": "autotests", "bench": "autobenches", "example": "autoexamples"}[kind], True):
            for path in sorted(paths):
                parts = PurePosixPath(path).parts
                auto = (len(parts) == 2 and parts[0] == directory and path.endswith(".rs")) or (len(parts) == 3 and parts[0] == directory and parts[-1] == "main.rs")
                if auto and path not in explicit_paths: add(path, default_role)
    if not binary_paths and not any(role in {"module", "test"} for role in roles.values()):
        raise NativeError("project has no supported Cargo application or library target")
    for path in paths:
        if path.endswith(".rs") and path not in roles: roles[path] = "module"
    return roles


def merge_lockfiles(application: bytes, runtime: bytes, package_name: str) -> bytes:
    app, rt = tomllib.loads(application.decode()), tomllib.loads(runtime.decode())
    if app.get("version") != 4 or rt.get("version") != 4: raise NativeError("unified build requires Cargo lock format 4")
    packages = {}
    app_root = None
    for owner, lock in (("application", app), ("runtime", rt)):
        for row in lock.get("package", []):
            if not isinstance(row, dict) or not isinstance(row.get("name"), str) or not isinstance(row.get("version"), str):
                raise NativeError("invalid locked package")
            key = (row["name"], row["version"], row.get("source", ""))
            if key in packages and packages[key] != row: raise NativeError("conflicting lock entries for " + row["name"])
            row = dict(row)
            if owner == "application" and row["name"] == package_name and "source" not in row:
                if app_root is not None: raise NativeError("ambiguous application lock root")
                app_root = key
            packages[key] = row
    if app_root is None: raise NativeError("application package is absent from Cargo.lock")
    row = packages[app_root]
    row["dependencies"] = sorted(set(row.get("dependencies", [])) | {"fluxr"})
    lines = ["# Generated by the FluxR unified builder from the two pinned input locks.", "version = 4", ""]
    for key, row in sorted(packages.items()):
        lines.append("[[package]]")
        for field in ("name", "version", "source", "checksum"):
            if field in row: lines.append(field + " = " + json.dumps(row[field]))
        if "dependencies" in row:
            lines.append("dependencies = [")
            lines.extend(" " + json.dumps(value) + "," for value in row["dependencies"])
            lines.append("]")
        if set(row) - {"name", "version", "source", "checksum", "dependencies"}:
            raise NativeError("unsupported Cargo lock package metadata")
        lines.append("")
    if app.get("metadata") or rt.get("metadata"): raise NativeError("unsupported legacy Cargo lock metadata")
    return ("\n".join(lines) + "\n").encode()


@dataclass
class Project:
    files: dict[str, bytes]
    modes: dict[str, int]
    receipt: dict
    rustflags: list[str]
    catalog: list[tuple[str, Document]]


def load_project(manifest: Path, *, testing=False) -> Project:
    raw = read_regular(manifest, 1024 * 1024)
    config = json.loads(raw, object_pairs_hook=no_duplicates)
    if not isinstance(config, dict) or set(config) != {"format", "files", "rustflags"} or config["format"] != "fluxr-project-v1":
        raise NativeError("expected one canonical fluxr-project-v1 manifest; migrate historical manifests explicitly")
    entries, flags = config["files"], config["rustflags"]
    if not isinstance(entries, list) or not 1 <= len(entries) <= MAX_FILES: raise NativeError("invalid project file count")
    if not isinstance(flags, list) or len(flags) > 128 or any(not isinstance(f, str) or "\0" in f or "\x1f" in f for f in flags):
        raise NativeError("rustflags must be a bounded argument vector")
    inputs, rows, names, sources, modes, parsed = {}, [], set(), set(), {}, {}
    total = 0
    for entry in entries:
        if not isinstance(entry, dict) or set(entry) != {"source", "output", "mode", "executable"}: raise NativeError("invalid project file fields")
        source, output = relpath(entry["source"]), relpath(entry["output"])
        key = output.as_posix()
        if key.casefold() in names or key in {"fluxr-receipt.json", "fluxr-native-receipt.json"} or key.casefold().startswith(".fluxr-runtime/") or key.casefold() == ".fluxr-runtime":
            raise NativeError("duplicate/reserved output path")
        if source.as_posix() in sources: raise NativeError("duplicate source path")
        names.add(key.casefold()); sources.add(source.as_posix())
        if not isinstance(entry["mode"], str) or entry["mode"] not in {"source", "copy"} or type(entry["executable"]) is not bool:
            raise NativeError("invalid canonical file mode")
        if output.suffix == ".rs" and entry["mode"] != "source": raise NativeError("Rust application outputs must originate from canonical FluxR source")
        if entry["mode"] == "source" and output.suffix != ".rs": raise NativeError("FluxR source must emit a Rust source file")
        original = read_regular(beneath(manifest.parent.resolve(), source))
        inputs[key] = original
        modes[key] = 0o755 if entry["executable"] else 0o644
        total += len(original)
        if total > MAX_PROJECT: raise NativeError("project input exceeds byte budget")
        if entry["mode"] == "source": parsed[key] = parse_source(original)
        rows.append({**entry, "source_sha256": sha(original)})
    for key in inputs:
        if any(parent.as_posix().casefold() in names for parent in PurePosixPath(key).parents if str(parent) != "."):
            raise NativeError("project file/directory collision")
    if "Cargo.toml" not in inputs or "Cargo.lock" not in inputs: raise NativeError("unified project requires explicit Cargo.toml and Cargo.lock")
    cargo = tomllib.loads(inputs["Cargo.toml"].decode())
    roles = cargo_targets(cargo, set(inputs), testing=testing)
    for path, role in roles.items():
        if path not in parsed:
            raise NativeError("Cargo target must originate from canonical FluxR source: " + path)
    catalog = [(row["source"], parsed[row["output"]]) for row in rows if row["output"] in parsed and roles[row["output"]] != "build-script"]
    runtime = runtime_snapshot()
    files = {}
    for row in rows:
        key = row["output"]
        if key in parsed:
            role = roles[key]
            files[key] = emit(inputs[key], name=row["source"], role=role, catalog=catalog if role in {"binary", "test"} else [])
            row.update(role=role, native_digest=parsed[key].native_digest, checked_exports=parsed[key].checked_exports)
        else: files[key] = inputs[key]
    files["Cargo.toml"] = inputs["Cargo.toml"] + b'\n# Mandatory unified FluxR Runtime, supplied by the compiler.\n[dependencies.fluxr]\npath = ".fluxr-runtime"\nversion = "=0.1.0"\n'
    files["Cargo.lock"] = merge_lockfiles(inputs["Cargo.lock"], runtime["Cargo.lock"], cargo["package"]["name"])
    for row in rows:
        generated = files[row["output"]]
        row.update(output_sha256=sha(generated), output_bytes=len(generated))
    runtime_rows = []
    for key, data in sorted(runtime.items()):
        relative = ".fluxr-runtime/" + key
        files[relative], modes[relative] = data, 0o644
        runtime_rows.append({"path": relative, "bytes": len(data), "sha256": sha(data)})
    total = sum(len(data) for data in files.values())
    if total > MAX_PROJECT: raise NativeError("runtime-bound project exceeds byte budget")
    receipt = {"format": "fluxr-emission-v1", "language_version": 1, "runtime": "unified",
               "manifest_sha256": sha(raw), "files": rows, "runtime_files": runtime_rows,
               "compiler_sha256": sha(read_regular(Path(__file__))), "rustflags": flags,
               "runtime_required": True, "cargo_roles": roles,
               "semantics": "One Runtime; checked declarations use the same loader for bundled and injected code; foreign Rust retains native semantics."}
    receipt["execution"] = {"status": "source-export", "modules": [
        {"name": name, "exports": [{"name": export, "status": "pending-build"} for export in doc.checked_exports]}
        for name, doc in catalog]}
    return Project(files, modes, receipt, flags, catalog)


def export_project(manifest: Path, destination: Path, project: Project | None = None):
    project = load_project(manifest) if project is None else project
    if destination.exists() or destination.is_symlink(): raise NativeError("destination already exists")
    destination.parent.mkdir(parents=True, exist_ok=True)
    stage = Path(tempfile.mkdtemp(prefix=".fluxr-", dir=destination.parent))
    try:
        for relative, data in project.files.items():
            path = stage / relative; path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data); path.chmod(project.modes[relative])
        (stage / "fluxr-receipt.json").write_text(json.dumps(project.receipt, indent=2) + "\n")
        if destination.exists() or destination.is_symlink(): raise NativeError("destination appeared during export")
        stage.rename(destination)
    finally:
        if stage.exists(): shutil.rmtree(stage)
    return project


def run_logged(command, cwd: Path, output: Path, timeout: float, env=None):
    if not math.isfinite(timeout) or timeout <= 0: raise NativeError("timeout must be finite and positive")
    if output.is_symlink(): raise NativeError("log directory cannot be a symlink")
    output.mkdir(parents=True, exist_ok=True)
    start = time.monotonic(); timed_out = False
    with (output / "stdout.log").open("xb") as stdout, (output / "stderr.log").open("xb") as stderr:
        process = subprocess.Popen(command, cwd=cwd, env=env, stdout=stdout, stderr=stderr, start_new_session=True)
        try: code = process.wait(timeout=timeout)
        except BaseException as error:
            timed_out = isinstance(error, subprocess.TimeoutExpired)
            try: os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError: pass
            code = process.wait()
            if not timed_out: raise
    receipt = {"argv": command, "cwd": str(cwd), "exit_code": code, "timed_out": timed_out, "wall_seconds": time.monotonic() - start}
    (output / "command.json").write_text(json.dumps(receipt, indent=2) + "\n")
    return receipt


def compile_checked_bindings(project: Project, output: Path, target: Path, logs: Path, args, env):
    """Compile bundled checked exports through the existing Rust semantic backend.

    The same Runtime owns AOT and VM calls. Unsupported AOT operations receive an
    explicit VM status from the Rust compiler; no independent Python semantics or
    unchecked fallback participates in this decision.
    """
    checked = [(name, doc) for name, doc in project.catalog if doc.checked]
    report = {"status": "compiled", "compiler": None, "modules": []}
    if not checked:
        report["modules"] = [{"name": name, "exports": []} for name, _ in project.catalog]
        project.receipt["execution"] = report
        (output / "fluxr-receipt.json").write_text(json.dumps(project.receipt, indent=2) + "\n")
        return
    compiler = args.compiler.resolve() if args.compiler else target / ".fluxr-compiler" / "debug" / "fluxr-cli"
    if args.compiler is None:
        command = [args.cargo, "build", "--bin", "fluxr-cli", "--locked", "--jobs", str(args.jobs), "--target-dir", str(target / ".fluxr-compiler")]
        if not args.online: command.append("--offline")
        bootstrap_env = env.copy()
        # Application architecture tuning must not mutate the compiler's ABI.
        bootstrap_env.pop("CARGO_ENCODED_RUSTFLAGS", None)
        bootstrap = run_logged(command, output / ".fluxr-runtime", logs / "compiler-bootstrap", args.timeout, bootstrap_env)
        if bootstrap["exit_code"] != 0 or bootstrap["timed_out"]:
            raise NativeError("checked compiler bootstrap failed; see compiler-bootstrap logs")
    if not compiler.is_file() or not os.access(compiler, os.X_OK): raise NativeError("checked compiler executable is unavailable")
    report["compiler"] = {"path": str(compiler), "sha256": sha(read_regular(compiler, 128 * 1024 * 1024))}
    scratch = output / ".fluxr-aot"
    scratch.mkdir()
    initializers = []
    by_name = {}
    for index, (name, document) in enumerate(checked):
        source = scratch / f"source-{index}.fluxr"
        generated = scratch / f"binding-{index}.rs"
        source.write_bytes(document.source)
        result = run_logged([str(compiler), "internal-emit-aot", str(source), str(generated)], output, logs / f"aot-{index}", args.timeout, env)
        if result["exit_code"] != 0 or result["timed_out"]:
            raise NativeError(f"checked compilation failed for {name}; see aot-{index} logs")
        summary = json.loads(read_regular(logs / f"aot-{index}" / "stdout.log"), object_pairs_hook=no_duplicates)
        statuses = summary.get("exports")
        if summary.get("language_version") != LANGUAGE_VERSION or summary.get("source_sha256") != sha(document.source):
            raise NativeError("checked compiler receipt does not bind the admitted source")
        if not isinstance(statuses, list) or sorted(row.get("name") for row in statuses) != sorted(document.checked_exports):
            raise NativeError("checked compiler export inventory disagrees")
        if any(row.get("status") not in {"aot", "vm"} for row in statuses): raise NativeError("unknown checked execution tier")
        code = read_regular(generated, MAX_OUTPUT).decode()
        by_name[name] = {"name": name, "exports": statuses, "binding_sha256": sha(code.encode())}
        module = f"__fluxr_checked_binding_{index}"
        initializers.append("{ mod " + module + " {\n" + code + "\n}\n"
            "let runtime = ::fluxr::runtime::global();\n"
            "let binding = runtime.inspect(" + rust_string(name) + ").and_then(|info| " + module + "::bind(runtime, " + rust_string(name) + ", info.revision));\n"
            "if let Err(error) = binding { ::std::eprintln!(\"FluxR AOT binding: {}\", error); ::std::process::exit(70); }\n}")
    report["modules"] = [by_name.get(name, {"name": name, "exports": []}) for name, _ in project.catalog]
    initializer = "\n".join(initializers)
    documents = dict(project.catalog)
    for row in project.receipt["files"]:
        if row.get("role") in {"binary", "test"}:
            generated = emit(documents[row["source"]].source, name=row["source"], role=row["role"], catalog=project.catalog, aot_initializer=initializer)
            project.files[row["output"]] = generated
            (output / row["output"]).write_bytes(generated)
            row.update(output_sha256=sha(generated), output_bytes=len(generated))
    project.receipt["execution"] = report
    (output / "fluxr-receipt.json").write_text(json.dumps(project.receipt, indent=2) + "\n")


def resolve_pinned_lock(project: Project, output: Path, logs: Path, args, env):
    """Let Cargo project the merged lock onto the unified dependency feature graph.

    A Runtime dependency has fewer active optional dependencies than the Runtime
    workspace root. Cargo owns that feature resolution; every resolved package
    must still match a version, source and checksum in the two input locks.
    """
    before = project.files["Cargo.lock"]
    locked = tomllib.loads(before.decode())
    def identity(row): return row["name"], row["version"], row.get("source", "")
    permitted = {identity(row): row.get("checksum") for row in locked["package"]}
    command = [args.cargo, "tree", "--prefix", "none"]
    if not args.online: command.append("--offline")
    result = run_logged(command, output, logs / "lock-resolution", args.timeout, env)
    if result["exit_code"] != 0 or result["timed_out"]:
        raise NativeError("Cargo lock projection failed; see lock-resolution logs")
    after = read_regular(output / "Cargo.lock")
    resolved = tomllib.loads(after.decode())
    if resolved.get("version") != 4: raise NativeError("Cargo changed the required lock format")
    for row in resolved["package"]:
        key = identity(row)
        if key not in permitted or permitted[key] != row.get("checksum"):
            raise NativeError("Cargo attempted to change a pinned package: " + row["name"] + " " + row["version"])
    project.files["Cargo.lock"] = after
    for row in project.receipt["files"]:
        if row["output"] == "Cargo.lock": row.update(output_sha256=sha(after), output_bytes=len(after))
    project.receipt["lock_resolution"] = {"merged_sha256": sha(before), "resolved_sha256": sha(after),
        "merged_packages": len(permitted), "resolved_packages": len(resolved["package"]), "original_pins_preserved": True}
    (output / "fluxr-receipt.json").write_text(json.dumps(project.receipt, indent=2) + "\n")


def main(argv=None):
    argv = list(sys.argv[1:] if argv is None else argv)
    # The build frontend owns everything before the separator. Only Cargo's
    # test harness receives the suffix; compiler bootstrap and lock projection
    # keep their existing commands. Keep each argument intact (never a shell).
    test_arguments = []
    test_separator = argv[:1] == ["build"] and "--" in argv
    if test_separator:
        separator = argv.index("--")
        argv, test_arguments = argv[:separator], argv[separator + 1:]
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    for name in ("emit", "wrap", "import-native"):
        command = sub.add_parser(name); command.add_argument("source", type=Path); command.add_argument("output", type=Path)
    command = sub.add_parser("export"); command.add_argument("manifest", type=Path); command.add_argument("output", type=Path)
    command = sub.add_parser("build", epilog="Forward native libtest options with --test -- FILTER --exact --test-threads 1.")
    command.add_argument("manifest", type=Path); command.add_argument("output", type=Path)
    command.add_argument("--target-dir", required=True, type=Path); command.add_argument("--logs", required=True, type=Path)
    command.add_argument("--cargo", default="cargo"); command.add_argument("--jobs", type=int, default=1)
    command.add_argument("--timeout", type=float, default=600); command.add_argument("--online", action="store_true")
    command.add_argument("--test", action="store_true")
    command.add_argument("--no-run", action="store_true", help="compile test executables without running them")
    command.add_argument("--profile", choices=["dev", "release"], default="release", help="Cargo optimization profile; the language and Runtime are identical")
    command.add_argument("--message-format", choices=["human", "json"], default="human")
    command.add_argument("--compiler", type=Path, help="existing FluxR 1 compiler; otherwise bootstrap the exact bundled Rust source")
    args = parser.parse_args(argv)
    try:
        if args.command in {"emit", "wrap", "import-native"}:
            data = read_regular(args.source)
            result = {"emit": emit, "wrap": wrap_rust, "import-native": import_native}[args.command](data)
            with args.output.open("xb") as out: out.write(result)
            print(json.dumps({"language_version": 1, "source_sha256": sha(data), "output_sha256": sha(result), "bytes": len(result), "runtime_required": args.command == "emit"}))
        elif args.command == "export":
            print(json.dumps(export_project(args.manifest, args.output).receipt))
        else:
            if not 1 <= args.jobs <= 64 or not math.isfinite(args.timeout) or args.timeout <= 0: raise NativeError("invalid jobs/timeout")
            if args.no_run and not args.test: raise NativeError("--no-run requires --test")
            if test_separator and not args.test: raise NativeError("test arguments after -- require --test")
            if test_arguments and args.no_run: raise NativeError("test arguments cannot be used with --no-run; no test will execute")
            source_root, output, target = args.manifest.parent.resolve(), args.output.resolve(), args.target_dir.resolve()
            if target.is_relative_to(source_root) or target.is_relative_to(output) or source_root.is_relative_to(target) or output.is_relative_to(target):
                raise NativeError("Cargo artifacts must be separate from source/emission trees")
            project = load_project(args.manifest, testing=args.test)
            project = export_project(args.manifest, args.output, project)
            env = os.environ.copy()
            for key in list(env):
                if key == "RUSTFLAGS" or key.startswith("FLUXR_CONTROL_"): env.pop(key)
            env["CARGO_ENCODED_RUSTFLAGS"] = "\x1f".join(project.rustflags); env["CARGO_INCREMENTAL"] = "0"
            resolve_pinned_lock(project, output, args.logs, args, env)
            compile_checked_bindings(project, output, target, args.logs, args, env)
            command = [args.cargo, "test" if args.test else "build", "--locked", "--jobs", str(args.jobs), "--target-dir", str(target)]
            if args.profile == "release": command.append("--release")
            if args.no_run: command.append("--no-run")
            if args.message_format == "json": command.append("--message-format=json")
            if not args.online: command.append("--offline")
            if test_arguments: command.extend(["--", *test_arguments])
            receipt = run_logged(command, output, args.logs, args.timeout, env)
            receipt["emission"] = project.receipt
            receipt["claim"] = "Unified Runtime build command result; runtime/conformance/performance qualification is separate."
            (args.logs / "build.json").write_text(json.dumps(receipt, indent=2) + "\n")
            print(json.dumps(receipt))
            return 0 if receipt["exit_code"] == 0 and not receipt["timed_out"] else 1
        return 0
    except (NativeError, OSError, ValueError, KeyError, TypeError) as error:
        print(f"fluxr: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
