"""Lower FR1 collections using the supplied nominal interface."""
import ast
import math
import re
import struct

from collections_codec import python, read_abi
from effects import Expr, Lower, expr, unsigned, truth
from expressions import f32, parse


def octants(turn):
    if turn not in (-1, 1):
        raise ValueError("rotation sign must be ±1")
    d = f32(math.sqrt(0.5))
    a, b = 1., 0.
    x, z = d, turn*d
    out = []
    for _ in range(4):
        out += [(a or 0., b or 0.), (x, z)]
        a, b = -turn*b, turn*a
        x, z = -turn*z, turn*x
    return out


DECLARATION = re.compile(r"(\w+)(?:\(([\w,]*)\))?=(.*)")


def scalar(node):
    op, *args = node
    if op == "ref":
        return "var", args[0]
    if op == "uint":
        return "const", args[0]
    if op == "f32":
        return "const", struct.unpack("<f", struct.pack("<I", args[0]))[0]
    if op == "call":
        return "call", scalar(args[0]), list(map(scalar, args[1]))
    return {"and": "&", "or": "|", "band": "&", "bor": "|"}.get(op, op), *map(scalar, args)


def ty(kind):
    if isinstance(kind, str):
        return kind
    if "record" in kind:
        return kind["record"]
    if "enum" in kind:
        return "#" + kind["enum"]
    return f'[{ty(kind["list"])}]'


class Function(Lower):
    def __init__(self, name, aliases, interface, records, private):
        self.name, self.records, self.private = name, records, private
        self.params = [(binding["name"], ty(binding["ty"])) for binding in interface["params"]]
        self.result = ty(interface["result"])
        self.types = dict(self.params)
        self.env = {alias: Expr(key, kind, frozenset({key}))
                    for alias, (key, kind) in zip(aliases, self.params, strict=True)}
        self.locals, self.helpers, self.body = [], [], []
        self.steps = 0

    def expression(self, tree):
        op, *args = tree
        if op == "call" and args[0][1] in self.private:
            self.charge()
            aliases, body = self.private[args[0][1]]
            actuals = [self.expression(value) for value in args[1]]
            saved = self.env
            self.env = dict(zip(aliases, actuals, strict=True))
            try:
                return self.expression(body)
            finally:
                self.env = saved
        return super().expression(tree)

    def scalar(self, node):
        return self.expression(scalar(python(node)))

    def fields(self, record):
        return [Expr(f"{record.text}.{name}", kind, record.uses)
                for name, kind in self.records[record.kind]]

    def call_signature(self, node, name, count):
        if not (isinstance(node, ast.Call) and isinstance(node.func, ast.Name)
                and node.func.id == name and len(node.args) == count and not node.keywords):
            raise ValueError(f"expected {name} with {count} arguments")
        return node.args

    def scan(self, values, count, statements):
        index = self.local("u64")
        return [
            f"(= {index.text} 0u)",
            f"(while (< {index.text} {count.text}) (",
            *[f"  {line}" for line in statements(index, f"(at {values.text} {index.text})")],
            f"  (= {index.text} (+ {index.text} 1u))",
            "))",
        ]

    def first_eq(self, node):
        rows, count, pattern = self.call_signature(node, "first_eq", 3)
        rows, count = self.scalar(rows), self.scalar(count)
        row, index = self.local(rows.kind[1:-1]), self.local("u64")
        fields = self.fields(row)
        bits = ast.literal_eval(pattern)
        tests = []
        for bit, field in zip(bits, fields, strict=True):
            if type(bit) is not int or bit not in {0, 1} or field.kind != "bool":
                raise ValueError("first_eq needs a Boolean row and bit pattern")
            tests.append(field.text if bit else f"(! {field.text})")
        guard = tests[0] if len(tests) == 1 else f"(& {' '.join(tests)})"
        return [f"(find {index.text} {count.text} {row.text} {rows.text} {guard})"]

    def count_if(self, node):
        values, count, predicate, initial = self.call_signature(node, "count_if", 4)
        limit, = self.call_signature(predicate, "lt", 1)
        values, count, initial = self.scalar(values), self.scalar(count), self.scalar(initial)
        limit = self.scalar(limit)
        total = self.local("u64")

        def visit(_index, value):
            item = Expr(value, values.kind[1:-1])
            bound = expr("tof", "f32", limit) if item.kind == "f32" and limit.kind == "u64" else limit
            return [f"(when (< {item.text} {bound.text}) (= {total.text} (+ {total.text} 1u)))"]

        return [f"(= {total.text} {initial.text})", *self.scan(values, count, visit), f"(ret {total.text})"]

    def indices(self, node):
        values, count, callback = self.call_signature(node, "indices", 3)
        function, bound = self.call_signature(callback, "bind", 2)
        if not isinstance(function, ast.Name) or function.id not in self.private:
            raise ValueError("indices needs a local typed predicate")
        values, count = self.scalar(values), self.scalar(count)
        index = self.local("u64")
        out = self.local("[u64]")
        saved = self.env
        self.env = self.env | {"item": Expr(f"(at {values.text} {index.text})", values.kind[1:-1])}
        predicate = self.expression(("call", ("var", function.id), [scalar(python(bound)), ("var", "item")]))
        self.env = saved
        return [f"(indices {out.text} {index.text} {count.text} {truth(predicate).text})"]

    def map_n(self, node):
        rows, count, mapper = self.call_signature(node, "map_n", 3)
        if not isinstance(mapper, ast.Name) or mapper.id != "last_true":
            raise ValueError("map_n kernel requires last_true")
        rows, count = self.scalar(rows), self.scalar(count)
        out, row, rank = self.local("[u64]"), self.local(rows.kind[1:-1]), self.local("u64")

        def visit(_index, value):
            fields = self.fields(row)
            if any(field.kind != "bool" for field in fields):
                raise ValueError("last_true needs all-Boolean rows")
            return [f"(= {row.text} {value})", f"(= {rank.text} 0u)",
                    *[f"(when {field.text} (= {rank.text} {i}u))" for i, field in enumerate(fields, 1)],
                    f"(push {out.text} {rank.text})"]

        return [f"(= {out.text} (list u64))", *self.scan(rows, count, visit), f"(ret {out.text})"]

    def lattice(self, node):
        if not isinstance(node, ast.IfExp) or not isinstance(node.body, ast.List) or node.body.elts:
            raise ValueError("expected an empty-list early guard")
        center_node, scales, vectors = self.call_signature(node.orelse, "lattice", 3)
        vectors, axis, condition = self.call_signature(vectors, "mirror", 3)
        turn, = self.call_signature(vectors, "octants", 1)
        axes = ast.literal_eval(axis)
        center = self.scalar(center_node)
        if axes not in (0, 1):
            raise ValueError("octants need a planar reflection axis")
        if not (isinstance(condition, ast.Compare) and isinstance(condition.left, ast.Subscript)):
            raise ValueError("expected a positional vector reflection predicate")
        if ast.dump(condition.left.value) != ast.dump(center_node):
            raise ValueError("reflection predicate needs a coordinate of the lattice center")
        coordinates = self.fields(center)
        if len(coordinates) != 2 or any(value.kind != "f32" for value in coordinates):
            raise ValueError("octants need two binary32 coordinates")
        index = ast.literal_eval(condition.left.slice)
        saved = self.env
        self.env = self.env | {"coordinate": coordinates[index]}
        reflection_test = ast.Compare(left=ast.Name(id="coordinate"), ops=condition.ops,
                                      comparators=condition.comparators)
        reflect = truth(self.scalar(reflection_test))
        self.env = saved
        sign = self.local("f32")
        products = []
        for scale in ast.literal_eval(scales):
            for vector in octants(ast.literal_eval(turn)):
                components = []
                for i, (center_field, direction) in enumerate(zip(coordinates, vector, strict=True)):
                    bits, = struct.unpack("<I", struct.pack("<f", direction))
                    component = f"0x{bits:08x}f"
                    if i == axes:
                        component = f"(* {sign.text} {component})"
                    components.append(f"(+ {center_field.text} (* {component} {float(scale)}f))")
                products.append(f"(new {center.kind} {' '.join(components)})")
        active = truth(self.scalar(node.test))
        return [f"(when {active.text} (ret (list {center.kind})))", f"(= {sign.text} 1f)",
                f"(when {reflect.text} (= {sign.text} -1f))",
                f"(ret (list {center.kind} {' '.join(products)}))"]

    def ranked(self, body):
        scan, row_names, predicate, key, reducers = body[5:-1].split(";")
        rows, count = [self.expression(parse(term)) for term in scan.split(",")]
        row = self.local(rows.kind[1:-1])
        self.env = self.env | dict(zip(row_names.split(), self.fields(row), strict=True))
        test = truth(self.expression(parse(predicate, compact=True)))
        kind, keys = parse(key, compact=True)
        if kind != "tuple" or len(keys) != 2:
            raise ValueError("rank expects a class and score")
        category, score = (self.expression(node) for node in keys)
        if category.kind != "bool" or score.kind != "f32":
            raise ValueError("rank expects a Boolean class and binary32 score")
        reducers = reducers.split(",")
        if len(reducers) != 2 or any(reduce not in {"min", "last"} for reduce in reducers):
            raise ValueError("rank needs two supported reducers")
        best = [self.local("u64") for _ in reducers]
        scores = [self.local("f32") if reducer == "min" else None for reducer in reducers]

        def visit(index, value):
            branches = []
            for selected, best_score in zip(best, scores, strict=True):
                commands = f"(= {selected.text} {index.text})"
                if best_score is not None:
                    commands = (
                        f"(if (|| (== {selected.text} {count.text}) (< {score.text} {best_score.text})) "
                        f"({commands} (= {best_score.text} {score.text})))"
                    )
                branches.append(commands)
            return [f"(= {row.text} {value})",
                    f"(when {test.text} (if {category.text} ({branches[1]}) ({branches[0]})))"]

        return [*[f"(= {selected.text} {count.text})" for selected in best],
                *[f"(= {old.text} 0f)" for old in scores if old is not None],
                *self.scan(rows, count, visit), f"(when (< {best[0].text} {count.text}) (ret {best[0].text}))",
                f"(ret {best[1].text})"]

    def lower(self, body):
        if body.startswith("rank(") and ";" in body:
            statements = self.ranked(body)
        else:
            node = ast.parse(body, mode="eval").body
            if isinstance(node, ast.IfExp) and isinstance(node.orelse, ast.Call):
                statements = self.lattice(node)
            elif isinstance(node, ast.Call) and isinstance(node.func, ast.Name) and node.func.id in {
                "first_eq", "count_if", "indices", "map_n",
            }:
                statements = getattr(self, node.func.id)(node)
            elif isinstance(node, ast.Name) and node.id == "argmin":
                (name, _), = self.params
                statements = [f"(ret (minidx {name}))"]
            else:
                statements = [f"(ret {self.scalar(node).text})"]
        params = " ".join(f"{name}:{kind}" for name, kind in self.params)
        return "\n".join([*self.helpers,
                          f"(pub {self.name} ({params}) {self.result} ({' '.join(self.locals)})",
                          *["  " + line for line in statements], ")"]) + "\n"


def lower_collections(planner, decoded, abi, schema):
    public = read_abi(abi)
    exports = {f["name"]: f for f in schema["functions"] if f["export"]}
    records = {r["name"]: [(f["name"], ty(f["ty"])) for f in r["fields"]] for r in schema["records"]}
    scalar_shapes = {"bool": "b", "u64": "u", "f32": "f"}

    def shape(kind):
        if kind in scalar_shapes:
            return scalar_shapes[kind]
        if kind.startswith("["):
            return f"[{shape(kind[1:-1])}]"
        return "(" + "".join(shape(field_kind) for _, field_kind in records[kind]) + ")"

    private = {}
    output = []
    for line in decoded.splitlines():
        name, args, body = DECLARATION.fullmatch(line).groups()
        params = args.split(",") if args else []
        if name not in public:
            private[name] = params, scalar(python(ast.parse(body, mode="eval").body))
            continue
        interface = exports[name]
        if public[name] != ([shape(ty(p["ty"])) for p in interface["params"]],
                            shape(ty(interface["result"]))):
            raise ValueError(f"packet signature differs from released export {name}")
        if body == "argmin" and not params:
            params = [binding["name"] for binding in interface["params"]]
        function = Function(name, params, interface, records, private)
        output.append(function.lower(body))
    if set(public) != set(exports) - {planner}:
        raise ValueError("collection packet does not cover the released collection exports")
    return "".join(output)
