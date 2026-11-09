"""Expression grammar used by FR1 collection scans."""
import re
import struct


def f32(value):
    return struct.unpack("<f", struct.pack("<f", value))[0]


NUMBER = re.compile(r"(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?")
NAME = re.compile(r"[a-zA-Z_]\w*")
LEX = re.compile(rf"{NUMBER.pattern}|{NAME.pattern}|>=|<=|!=|[?():\[\],!&|<>=+*/%\-]")
BP = {"|": 2, "&": 3, "=": 4, "!=": 4, "<": 4, ">": 4, "<=": 4, ">=": 4,
      "+": 5, "-": 5, "*": 6, "/": 6, "%": 6}


def guard_tokens(text):
    matches = list(LEX.finditer(text))
    for i, token in enumerate(matches):
        word = token.group()
        if i:
            previous = matches[i-1]
            before = previous.group()
            if previous.end() < token.start() and (before[-1].isalnum() or before[-1] in "_.)]") and (
                word[0].isalnum() or word[0] in "_.(!" and word != "!="
            ):
                yield "&"
        yield word


def parse(text, *, compact=False):
    tokens = LEX.findall(text)
    if "".join(tokens) != "".join(text.split()):
        raise ValueError(f"invalid expression {text!r}")
    if compact:
        tokens = list(guard_tokens(text))
    tokens.append("")
    cursor = 0

    def take(expected=None):
        nonlocal cursor
        token = tokens[cursor]
        if not token or (expected is not None and token != expected):
            raise ValueError(f"expected {expected} in {text}")
        cursor += 1
        return token

    def number(token):
        return int(token) if token.isdigit() else f32(float(token))

    def expr(level=1):
        token = take()
        if token == "(":
            left = expr()
            if compact and tokens[cursor] == ",":
                elements = [left]
                while tokens[cursor] == ",":
                    take(",")
                    elements.append(expr())
                left = ("tuple", elements)
            take(")")
        elif token in {"!", "-"}:
            left = ("not" if token == "!" else "neg", expr(7))
        elif token[0].isdigit() or token[0] == ".":
            left = ("const", number(token))
        elif token[0].isalpha() or token[0] == "_":
            left = ("var", token)
        else:
            raise ValueError(f"unexpected {token} in {text}")
        while True:
            op = tokens[cursor]
            if op == "[":
                take("[")
                left = ("index", left, expr())
                take("]")
            elif op == "(":
                take("(")
                args = [expr()]
                while tokens[cursor] == ",":
                    take(",")
                    args.append(expr())
                take(")")
                left = ("call", left, args)
            elif op == "?" and level == 1:
                take("?")
                yes = expr()
                take(":")
                left = ("if", left, yes, expr())
            elif BP.get(op, 0) >= level:
                left = (take(), left, expr(BP[op] + 1))
            else:
                return left

    tree = expr()
    if tokens[cursor]:
        raise ValueError(f"unexpected {tokens[cursor]} in {text}")
    return tree
