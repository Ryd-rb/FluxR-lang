"""Lower bounded ordered effects into typed FluxR."""
from dataclasses import dataclass
import struct


BINARY = {"&": "&&", "|": "||", "=": "==", "!=": "==", "<": "<", "<=": "<=",
          "+": "+", "-": "-", "*": "*", "/": "/", "%": "%"}
MAX_EXPRESSION_TEXT = 32 * 1024
MAX_LOWER_STEPS = 32_768


@dataclass(frozen=True)
class Expr:
    text: str
    kind: str
    uses: frozenset[str] = frozenset()

    def __post_init__(self):
        if len(self.text) > MAX_EXPRESSION_TEXT:
            raise ValueError("expanded expression exceeds the byte limit")


def expr(op, kind, *args):
    text = " ".join(arg.text for arg in args)
    return Expr(f"({op} {text})", kind, frozenset().union(*(arg.uses for arg in args)))


def unsigned(value):
    if not 0 <= value < 1 << 64:
        raise ValueError("unsigned literal is outside u64")
    return Expr(f"{value}u", "u64")


def signed(value):
    if not -(1 << 63) <= value < 1 << 63:
        raise ValueError("signed literal is outside i64")
    return Expr(str(value), "i64")


def truth(value):
    if value.kind == "bool":
        return value
    if value.kind == "u64":
        return expr("!", "bool", expr("==", "bool", value, unsigned(0)))
    raise ValueError("guard needs a Boolean or unsigned integer")


class Lower:
    def __init__(self, parameter, fields):
        self.parameter = parameter
        self.env = {}
        self.types = {parameter["name"]: parameter["ty"]["record"]}
        self.locals = []
        self.helpers = []
        self.body = []
        self.steps = 0
        for alias, binding in fields:
            name = parameter["name"]
            kind = binding["ty"]
            if isinstance(kind, dict):
                kind = f'[{kind["list"]}]'
            self.env[alias] = Expr(f'{name}.{binding["name"]}', kind, frozenset({name}))

    def local(self, kind):
        name = f"v{len(self.locals)}"
        self.types[name] = kind
        self.locals.append(f"{name}:{kind}")
        return Expr(name, kind, frozenset({name}))

    def choose(self, test, yes, no):
        if yes.kind != no.kind:
            raise ValueError("conditional result types differ")
        test = truth(test)
        uses = sorted(test.uses | yes.uses | no.uses)
        params = " ".join(f"{name}:{self.types[name]}" for name in uses)
        if not hasattr(self, "helper_names"):
            self.helper_names = set()
        used = self.helper_names
        number = len(self.helpers)
        name = f"ordered_select_{getattr(self, 'name', 'plan')}_{number}"
        while name in used:
            number += 1
            name = f"ordered_select_{getattr(self, 'name', 'plan')}_{number}"
        used.add(name)
        self.helpers.append(
            f"(fn {name} ({params}) {yes.kind} () "
            f"(when {test.text} (ret {yes.text})) (ret {no.text}))"
        )
        return expr(f"call {name}", yes.kind, *(Expr(key, self.types[key], frozenset({key})) for key in uses))

    def charge(self):
        self.steps += 1
        if self.steps > MAX_LOWER_STEPS:
            raise ValueError("expanded function exceeds the expression budget")

    def expression(self, tree, expected=None):
        self.charge()
        op, *args = tree
        if op == "var":
            return self.env[args[0]]
        if op == "f32bits":
            return Expr(f"0x{args[0]:08x}f", "f32")
        if op == "const":
            value = args[0]
            if type(value) is bool:
                return Expr(str(value).lower(), "bool")
            if isinstance(value, int):
                return signed(value) if expected == "i64" else unsigned(value)
            bits, = struct.unpack("<I", struct.pack("<f", value))
            return Expr(f"0x{bits:08x}f", "f32")
        if op == "neg" and args[0][0] == "const" and type(args[0][1]) is int:
            return signed(-args[0][1])
        if op == "if":
            return self.choose(self.expression(args[0]), self.expression(args[1], expected), self.expression(args[2], expected))
        if op == "not":
            return expr("!", "bool", truth(self.expression(args[0])))
        if op == "call":
            _, name = args[0]
            values = [self.expression(node) for node in args[1]]
            if name == "abs" and len(values) == 1 and values[0].kind == "f32":
                return expr("abs", "f32", *values)
            if name == "u32" and len(values) == 1 and values[0].kind == "u64":
                return expr("%", "u64", values[0], unsigned(1 << 32))
            raise ValueError(f"unsupported effect expression {tree}")
        if args[0][0] == "tuple":
            if args[1][0] != "const" or op not in {"<", "<=", ">", ">="}:
                raise ValueError("tuple guard needs a constant scalar bound")
            rows = args[0][1]
            if not rows:
                raise ValueError("empty tuple guard")
            result, *rest = [self.expression((op, row, args[1])) for row in rows]
            for part in rest:
                result = expr("&&", "bool", result, part)
            return result
        if op == "index":
            left = self.expression(args[0])
            right = self.expression(args[1], "u64")
            if left.kind in {"[bool]", "[u64]", "[i64]"} and right.kind == "u64":
                return expr("at", left.kind[1:-1], left, right)
            raise ValueError("index needs a supported list and u64 index")
        if op in {"<", "<=", ">", ">=", "=", "!="}:
            expected = None
        literal = lambda node: node[0] == "const" and type(node[1]) is int
        if literal(args[0]) and not literal(args[1]):
            right = self.expression(args[1], expected)
            left = self.expression(args[0], "i64" if right.kind == "i64" else expected)
        else:
            left = self.expression(args[0], expected)
            right = self.expression(args[1], "i64" if left.kind == "i64" else expected)
        if op in {"&", "|"}:
            return expr(BINARY[op], "bool", truth(left), truth(right))
        if (left.kind, right.kind) == ("u64", "f32"):
            left = expr("tof", "f32", left)
        if (left.kind, right.kind) == ("f32", "u64"):
            right = expr("tof", "f32", right)
        if left.kind != right.kind:
            raise ValueError(f"type mismatch in {tree}")
        if op in {">", ">="}:
            return expr("<" if op == ">" else "<=", "bool", right, left)
        if op == "!=":
            return expr("!", "bool", expr("==", "bool", left, right))
        return expr(BINARY[op], left.kind if op in {"+", "-", "*", "/", "%"} else "bool", left, right)

    def function(self, export, program, abi, order):
        tags = {}
        pairs = {}
        for line in abi.splitlines()[1:]:
            alias, *args = line.split()
            if len(args) == 1:
                tags[alias] = int(args[0])
            else:
                change, rank, mode = args
                if mode not in {"set", "flip"}:
                    raise ValueError(f"invalid ordered effect mode {mode}")
                pairs[alias] = int(change), int(rank), mode
        name = order["name"]
        output = self.local(f"[#{name}]")
        self.body.append(f"(= {output.text} (list #{name}))")

        def push(tag, *values):
            variant = order["variants"][tag]
            payload = " ".join(value.text for value in values)
            if isinstance(variant["payload"], dict) and "record" in variant["payload"]:
                payload = f'(new {variant["payload"]["record"]} {payload})'
            suffix = f" {payload}" if payload else ""
            return f'(push {output.text} (tag {name}.{variant["name"]}{suffix}))'

        def when(test, stmt):
            self.body.append(f"(when {truth(test).text} {stmt})")

        for op, *args in program:
            if op == "guard":
                when(expr("!", "bool", truth(self.expression(args[0]))), f"(ret {output.text})")
            elif op == "set":
                alias, tree = args
                value = self.expression(tree)
                slot = self.local(value.kind)
                self.body.append(f"(= {slot.text} {value.text})")
                self.env[alias] = slot
            elif op == "emit":
                tag, test, payload = args
                values = () if payload is None else (self.expression(payload),)
                when(self.expression(test), push(tags[tag], *values))
            elif op == "diff":
                flag, rank, indices, default, rows = args
                change, priority, mode = pairs[flag]
                for index, (target, desired_tree, *overlay) in zip(indices, rows, strict=True):
                    key = unsigned(index)
                    want = self.local("bool")
                    self.body.append(f"(= {want.text} {truth(self.expression(target)).text})")
                    effective = expr("at", "bool", self.env[flag], key)
                    if overlay:
                        effective = expr("||", "bool", effective, truth(self.expression(overlay[0])))
                    changed = expr("!", "bool", expr("==", "bool", effective, want))
                    when(changed, push(change, key, want) if mode == "set" else push(change, key))
                    desired = self.expression(desired_tree)
                    if default is not None:
                        desired = self.choose(want, desired, unsigned(default))
                    old = expr("at", "u64", self.env[rank], key)
                    desired_slot = self.local("u64")
                    self.body.append(f"(= {desired_slot.text} {desired.text})")
                    when(expr("!", "bool", expr("==", "bool", old, desired_slot)),
                         push(priority, key, desired_slot))
            else:
                raise ValueError(f"unsupported ordered effect {op}")
        self.body.append(f"(ret {output.text})")
        signature = f'{self.parameter["name"]}:{self.parameter["ty"]["record"]}'
        return "\n".join([*self.helpers,
                          f"(pub {export} ({signature}) [#{name}] ({' '.join(self.locals)})",
                          *["  " + line for line in self.body], ")"]) + "\n"
