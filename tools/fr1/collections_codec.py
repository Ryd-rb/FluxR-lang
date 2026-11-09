"""A positional instruction packet for the typed collection expression subset."""
from dataclasses import dataclass
import ast
import re
import struct

from expressions import parse


BUILTINS = (
    "first", "first_eq", "fold", "indices", "map_n", "last_true", "last", "rank",
    "argmin", "add_u64", "cross", "octants", "reflect", "affine", "count_if",
    "mirror", "lattice", "min", "bind", "eq", "lt",
)
OPS = (
    "ref", "uint", "f32", "call", "tuple", "list", "if", "neg", "not", "index",
    "and", "or", "band", "bor", "-", "+", "*", "%", "<", "<=", "=", "!=",
    ">", ">=", "bits",
)
UNARY = {"neg", "not"}
SYMBOLS = {
    "and": "and", "or": "or", "band": "&", "bor": "|", "=": "==",
    "-": "-", "+": "+", "*": "*", "%": "%", "<": "<", "<=": "<=",
    "!=": "!=", ">": ">", ">=": ">=",
}
PYTHON = {
    ast.And: "and", ast.Or: "or", ast.BitOr: "bor", ast.BitAnd: "band",
    ast.Sub: "-", ast.Add: "+", ast.Mult: "*", ast.Mod: "%", ast.Lt: "<",
    ast.LtE: "<=", ast.Eq: "=", ast.NotEq: "!=", ast.Gt: ">", ast.GtE: ">=",
    ast.USub: "neg", ast.Not: "not",
}
SHAPES = "buf[("


@dataclass
class Bits:
    data: int = 0
    size: int = 0
    prefix: bool = False

    def put(self, value, width):
        if not 0 <= value < 1 << width:
            raise ValueError("value does not fit its instruction field")
        self.data = (self.data << width) | value
        self.size += width

    def get(self, width):
        if not 0 <= width <= self.size:
            raise ValueError("incomplete instruction")
        self.size -= width
        value = self.data >> self.size
        self.data &= (1 << self.size) - 1
        return value

    def put_uint(self, value):
        if not 0 <= value < 1 << 64:
            raise ValueError("expected a u64")
        while value > 7:
            self.put(8 | (value & 7), 4)
            value >>= 3
        self.put(value, 4)

    def get_uint(self):
        value = 0
        for offset in range(0, 64, 3):
            part = self.get(4)
            value |= (part & 7) << offset
            if part < 8:
                if (offset and part == 0) or value >= 1 << 64:
                    break
                return value
        raise ValueError("invalid u64 field")


def constant(value):
    if type(value) is float:
        return "f32", struct.unpack("<I", struct.pack("<f", value))[0]
    if type(value) is int:
        return "uint", value
    raise ValueError("expected numeric literal")


def python(node):
    match node:
        case ast.Name(id=name):
            return "ref", name
        case ast.Constant(value=value):
            return constant(value)
        case ast.Call(func=func, args=args, keywords=[]):
            return "call", python(func), tuple(map(python, args))
        case ast.Tuple(elts=items) | ast.List(elts=items):
            return ("tuple" if isinstance(node, ast.Tuple) else "list"), tuple(map(python, items))
        case ast.IfExp(test=test, body=yes, orelse=no):
            return "if", python(test), python(yes), python(no)
        case ast.UnaryOp(op=op, operand=value):
            return PYTHON[type(op)], python(value)
        case ast.Subscript(value=value, slice=index):
            return "index", python(value), python(index)
        case ast.BinOp(left=left, op=op, right=right) | ast.Compare(left=left, ops=[op], comparators=[right]):
            return PYTHON[type(op)], python(left), python(right)
        case ast.BoolOp(op=op, values=[first, *others]):
            result = python(first)
            for item in others:
                result = PYTHON[type(op)], result, python(item)
            return result
        case _:
            raise ValueError(f"unsupported collection expression {ast.dump(node)}")


def rank_expr(node):
    op, *args = node
    if op == "const":
        return constant(args[0])
    if op == "var":
        return "ref", args[0]
    if op == "call":
        return "call", rank_expr(args[0]), tuple(map(rank_expr, args[1]))
    if op == "tuple":
        return "tuple", tuple(map(rank_expr, args[0]))
    return {"&": "and", "|": "or"}.get(op, op), *map(rank_expr, args)


def encode_node(bits, node, locals_, functions):
    op, *args = node
    if op == "tuple" and all(item in (("uint", 0), ("uint", 1)) for item in args[0]):
        op = "bits"
    if not bits.prefix:
        bits.put(OPS.index(op), 5)
    elif op == "ref":
        bits.put(0, 1)
    elif op in {"uint", "call"}:
        bits.put(4 if op == "uint" else 5, 3)
    else:
        bits.put(3, 2)
        bits.put(OPS.index(op), 5)
    if op == "ref":
        name, = args
        for bank, names, width in [(0, locals_, 3), (1, functions, 4), (2, BUILTINS, 5)]:
            if name in names:
                if bits.prefix:
                    bits.put(0, 1) if bank == 0 else bits.put(bank + 1, 2)
                else:
                    bits.put(bank, 2)
                bits.put(names.index(name), width)
                return
        raise ValueError(f"unknown reference {name}")
    if op == "uint":
        bits.put_uint(args[0])
    elif op == "f32":
        bits.put(args[0], 32)
    elif op in {"tuple", "list", "call", "bits"}:
        items = args[-1]
        if op == "call":
            encode_node(bits, args[0], locals_, functions)
        bits.put(len(items), 3)
        for item in items:
            if op == "bits":
                bits.put(item[1], 1)
            else:
                encode_node(bits, item, locals_, functions)
    else:
        for arg in args:
            encode_node(bits, arg, locals_, functions)


def decode_node(bits, locals_, functions, depth=0):
    if depth >= 48:
        raise ValueError("collection expression is too deep")

    def child():
        return decode_node(bits, locals_, functions, depth + 1)

    if not bits.prefix:
        op = OPS[bits.get(5)]
    elif bits.get(1) == 0:
        op = "ref"
    elif bits.get(1) == 0:
        op = "call" if bits.get(1) else "uint"
    else:
        op = OPS[bits.get(5)]
    if op == "ref":
        bank = (1 + bits.get(1) if bits.get(1) else 0) if bits.prefix else bits.get(2)
        names, width = ((locals_, 3), (functions, 4), (BUILTINS, 5))[bank]
        return op, names[bits.get(width)]
    if op == "uint":
        return op, bits.get_uint()
    if op == "f32":
        return op, bits.get(32)
    if op == "bits":
        return "tuple", tuple(("uint", bits.get(1)) for _ in range(bits.get(3)))
    if op in {"tuple", "list", "call"}:
        callee = (child(),) if op == "call" else ()
        items = tuple(child() for _ in range(bits.get(3)))
        return op, *callee, items
    count = 1 if op in UNARY else 3 if op == "if" else 2
    return op, *(child() for _ in range(count))


def render(node, rank=False):
    op, *args = node
    if op in {"ref", "uint"}:
        return str(args[0])
    if op == "f32":
        value = struct.unpack("<f", struct.pack("<I", args[0]))[0]
        if not (-float("inf") < value < float("inf")):
            raise ValueError("collection literal requires a finite binary32 value")
        return repr(value)
    if op == "call":
        return f"{render(args[0], rank)}({','.join(render(item, rank) for item in args[1])})"
    if op in {"tuple", "list"}:
        values = [render(item, rank) for item in args[0]]
        if op == "list":
            return f"[{','.join(values)}]"
        comma = "," if len(values) == 1 else ""
        return f"({','.join(values)}{comma})"
    values = [render(arg, rank) for arg in args]
    if op == "if":
        return (f"({values[0]}?{values[1]}:{values[2]})" if rank
                else f"({values[1]} if {values[0]} else {values[2]})")
    if op in UNARY:
        prefix = "-" if op == "neg" else "!" if rank else "not "
        return f"({prefix}{values[0]})"
    if op == "index":
        return f"{values[0]}[{values[1]}]"
    operator = {"and": "&", "or": "|", "band": "&", "bor": "|", "=": "="}.get(op, SYMBOLS[op]) if rank else SYMBOLS[op]
    return f"({values[0]} {operator} {values[1]})"


def read_abi(text):
    result = {}
    for line in text.splitlines():
        name, *tail = line.split()
        arrow = tail.index(">")
        params, outputs = tail[:arrow], tail[arrow + 1:]
        output, = outputs
        if name in result:
            raise ValueError(f"duplicate typed export {name}")
        result[name] = (params, output)
    return result


def shape(bits, text, index=0):
    tag = text[index]
    bits.put(SHAPES.index(tag), 3)
    index += 1
    if tag == "[":
        index = shape(bits, text, index)
        if text[index] != "]":
            raise ValueError("list type needs a closing bracket")
        index += 1
    elif tag == "(":
        nested = Bits()
        count = 0
        while text[index] != ")":
            index = shape(nested, text, index)
            count += 1
        bits.put(count, 3)
        bits.put(nested.data, nested.size)
        index += 1
    return index


def read_shape(bits, depth=0):
    if depth >= 8:
        raise ValueError("collection type is too deep")
    tag = SHAPES[bits.get(3)]
    if tag == "[":
        return f"[{read_shape(bits, depth + 1)}]"
    if tag == "(":
        return "(" + "".join(read_shape(bits, depth + 1) for _ in range(bits.get(3))) + ")"
    return tag


def pack(source, abi, prefix=False):
    public = read_abi(abi)
    declarations = [re.fullmatch(r"(\w+)(?:\(([\w,]*)\))?=(.*)", line).groups()
                    for line in source.splitlines() if line]
    code, schema = Bits(prefix=prefix), Bits()
    code.put(len(declarations), 4)
    names = []
    functions = []
    for name, params, body in declarations:
        if name in functions or name in BUILTINS:
            raise ValueError(f"duplicate declaration {name}")
        arguments = params.split(",") if params else []
        code.put(int(name in public), 1)
        if name in public:
            names.append(name)
            types, result = public[name]
            schema.put(len(types), 3)
            for ty in [*types, result]:
                if shape(schema, ty) != len(ty):
                    raise ValueError("unexpected trailing type text")
        code.put(int(params is not None), 1)
        code.put(len(arguments), 3)
        ranked = body.startswith("rank(") and ";" in body
        code.put(int(ranked), 1)
        if ranked:
            range_, fields, guard, score, reducers = body[5:-1].split(";")
            fields = fields.split()
            code.put(len(fields), 3)
            for term in range_.split(","):
                encode_node(code, rank_expr(parse(term)), arguments, functions)
            for term in [guard, score]:
                encode_node(code, rank_expr(parse(term, compact=True)), arguments + fields, functions)
            reducers = reducers.split(",")
            code.put(len(reducers), 3)
            for reducer in reducers:
                code.put(BUILTINS.index(reducer), 5)
        else:
            encode_node(code, python(ast.parse(body, mode="eval").body), arguments, functions)
        functions.append(name)
    if set(names) != set(public):
        raise ValueError("typed exports and declarations differ")
    return names, code, schema


def unpack(names, code, schema, code_padding=14, schema_padding=14):
    exports = iter(names)
    functions, source, abi = [], [], []
    for number in range(code.get(4)):
        public = code.get(1)
        name = next(exports) if public else f"inner{number}"
        has_params = code.get(1)
        arguments = [f"a{i}" for i in range(code.get(3))]
        types = [read_shape(schema) for _ in range(schema.get(3))] if public else []
        if public:
            if has_params and len(arguments) != len(types):
                raise ValueError("function arguments differ from its encoded signature")
            abi.append(" ".join([name, *types, ">", read_shape(schema)]))
        if code.get(1):
            fields = [f"r{i}" for i in range(code.get(3))]
            bounds = [render(decode_node(code, arguments, functions), rank=True) for _ in range(2)]
            guard, score = [render(decode_node(code, arguments + fields, functions), rank=True) for _ in range(2)]
            reducers = [BUILTINS[code.get(5)] for _ in range(code.get(3))]
            body = f"rank({','.join(bounds)};{' '.join(fields)};{guard};{score};{','.join(reducers)})"
        else:
            node = decode_node(code, arguments, functions)
            if public and not has_params and types and not (node[0] == "ref" and node[1] in BUILTINS):
                raise ValueError("typed arguments require a function or an explicit builtin alias")
            body = render(node)
        arguments = f"({','.join(arguments)})" if has_params else ""
        source.append(f"{name}{arguments}={body}")
        functions.append(name)
    if (next(exports, None) is not None or code.data or schema.data
            or code.size > code_padding or schema.size > schema_padding):
        raise ValueError("unexpected trailing packet contents")
    return "\n".join(source) + "\n", "\n".join(abi) + "\n"


def encode_words(bits, alphabet, width):
    padding = (-bits.size) % width
    data = Bits(bits.data << padding, bits.size + padding)
    return "".join(alphabet[data.get(width)] for _ in range(data.size // width))


def decode_words(source, lookup, width, prefix=False):
    bits = Bits(prefix=prefix)
    for word in source.split():
        bits.put(lookup[" " + word], width)
    return bits
