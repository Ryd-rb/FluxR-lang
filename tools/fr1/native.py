"""Emit native Rust for the checked structured subset."""
import hashlib
import json
from collections import Counter


SCALARS = {"unit": "()", "bool": "bool", "u64": "u64", "i64": "i64", "f32": "f32"}
RELATIONS = {"eq": "==", "lt": "<", "le": "<=", "and": "&&", "or": "||"}
ARITHMETIC = {"add": "+", "sub": "-", "mul": "*", "div": "/", "rem": "%"}


def ungroup(code):
    if not code.startswith("(") or not code.endswith(")"):
        return code
    depth = 0
    for index, character in enumerate(code):
        if character == "(":
            depth += 1
        elif character == ")":
            depth -= 1
            if depth == 0:
                return code[1:-1] if index == len(code) - 1 else code
    return code


class Emitter:
    def __init__(self, source, bounded_loops=False):
        if source["inputs"]:
            raise ValueError("native subset does not accept host inputs")
        self.records = {item["name"]: item["fields"] for item in source["records"]}
        self.enums = {item["name"]: item["variants"] for item in source["enums"]}
        self.functions = {item["name"]: item for item in source["functions"] if not item["receiver"]}
        self.types = set()
        self.functions_used = {}
        self.needs_f32 = False
        self.needs_argmin = False
        self.bounded_loops = bounded_loops

    def borrows(self, ty):
        if isinstance(ty, str):
            return False
        if "list" in ty:
            return True
        if "record" in ty:
            return any(self.borrows(field["ty"]) for field in self.records[ty["record"]])
        return any(v["payload"] is not None and self.borrows(v["payload"])
                   for v in self.enums[ty["enum"]])

    def kind(self, ty, borrowed=False):
        if isinstance(ty, str):
            return SCALARS[ty]
        if "list" in ty:
            item = self.kind(ty["list"], borrowed)
            return f"&'a [{item}]" if borrowed else f"Vec<{item}>"
        category, name = next(iter(ty.items()))
        self.types.add((category, name))
        if self.borrows(ty):
            if not borrowed:
                raise ValueError("native subset cannot return a borrowed record or variant")
            return f"Native{name}<'a>"
        return f"Native{name}"

    def expression(self, node, env):
        op = node["op"]
        if op == "local":
            return f"v_{node['name']}", env[node["name"]]
        if op == "f32":
            return f"f32::from_bits(0x{node['bits']:08x})", "f32"
        if op == "bool":
            return str(node["value"]).lower(), "bool"
        if op in {"u64", "i64"}:
            return f"{node['value']}{op}", op
        if op == "unit":
            return "()", "unit"
        if op == "field":
            record, ty = self.expression(node["record"], env)
            name = node["name"]
            field, = [field for field in self.records[ty["record"]] if field["name"] == name]
            return f"{record}.f_{name}", field["ty"]
        if op == "list":
            items = [ungroup(self.expression(item, env)[0]) for item in node["items"]]
            return f"vec![{', '.join(items)}]", {"list": node["element"]}
        if op == "record":
            name = node["name"]
            ty = {"record": name}
            record = self.kind(ty)
            fields = [f"f_{field['name']}: {ungroup(self.expression(item, env)[0])}"
                      for field, item in zip(self.records[name], node["fields"], strict=True)]
            return f"{record} {{ {', '.join(fields)} }}", ty
        if op == "variant":
            ty = {"enum": node["name"]}
            variants = self.enums[node["name"]]
            tag = next(i for i, variant in enumerate(variants) if variant["name"] == node["variant"])
            variant = f"{self.kind(ty)}::V{tag}"
            if node["payload"] is not None:
                payload, _ = self.expression(node["payload"], env)
                variant += f"({ungroup(payload)})"
            return variant, ty
        if op == "index":
            values, ty = self.expression(node["list"], env)
            index, _ = self.expression(node["index"], env)
            return f"(*{values}.get(usize::try_from({ungroup(index)}).ok()?)?)", ty["list"]
        if op == "call":
            name = node["name"]
            function = self.functions[name]
            self.functions_used.setdefault(name, None)
            args = [ungroup(self.expression(arg, env)[0]) for arg in node["args"]]
            if self.bounded_loops:
                args.append("_v_backedges")
            return f"native_{name}({', '.join(args)})?", function["result"]
        if op == "unary":
            value, _ = self.expression(node["value"], env)
            return {
                "not": (f"!({value})", "bool"),
                "abs_f32": (f"f32::from_bits(({value}).to_bits() & 0x7fff_ffff)", "f32"),
                "u64_to_f32": (f"({value} as f32)", "f32"),
            }[node["kind"]]
        if op == "binary":
            kind = node["kind"]
            left, ty = self.expression(node["left"], env)
            right, _ = self.expression(node["right"], env)
            if kind in RELATIONS:
                return f"({left} {RELATIONS[kind]} {right})", "bool"
            if ty in {"u64", "i64"} and kind in ARITHMETIC:
                return f"({left}).checked_{kind}({ungroup(right)})?", ty
            if ty == "f32" and kind in ARITHMETIC and kind != "rem":
                self.needs_f32 = True
                return f"native_canonical_f32({left} {ARITHMETIC[kind]} {right})", ty
        if op == "arg_min_f32":
            values, _ = self.expression(node["list"], env)
            self.needs_argmin = True
            return f"native_arg_min_f32({values})", "u64"
        raise ValueError(f"unsupported native expression {op}")

    def statements(self, body, env, indent, tail=False):
        def line(code):
            return indent + code

        for index, node in enumerate(body):
            op = node["op"]
            if op in {"return", "push", "set"}:
                value, _ = self.expression(node["value"], env)
                value = ungroup(value)
                if op == "return":
                    if tail and index == len(body) - 1:
                        yield line(f"Some({value})")
                    else:
                        yield line(f"return Some({value});")
                elif op == "push":
                    yield line(f"v_{node['local']}.push({value});")
                else:
                    yield line(f"v_{node['local']} = {value};")
            elif op == "if":
                test, _ = self.expression(node["test"], env)
                yield line(f"if {ungroup(test)} {{")
                yield from self.statements(node["then"], env, indent + "    ")
                if node["otherwise"]:
                    yield line("} else {")
                    yield from self.statements(node["otherwise"], env, indent + "    ")
                yield line("}")
            elif op == "while" and self.bounded_loops:
                test, _ = self.expression(node["test"], env)
                yield line(f"while {ungroup(test)} {{")
                yield line("    *_v_backedges = _v_backedges.checked_sub(1)?;")
                yield from self.statements(node["body"], env, indent + "    ")
                yield line("}")
            else:
                raise ValueError(f"unsupported native statement {op}")

    def function(self, name):
        source = self.functions[name]
        params = source["params"]
        locals_ = source["locals"]
        env = {binding["name"]: binding["ty"] for binding in params + locals_}
        writes = Counter()

        def count_writes(body, repeating=False):
            for statement in body:
                op = statement["op"]
                if op in {"set", "push"}:
                    writes[statement["local"]] += 2 if repeating or op == "push" else 1
                if op == "if":
                    count_writes(statement["then"], repeating)
                    count_writes(statement["otherwise"], repeating)
                if op == "while":
                    count_writes(statement["body"], True)

        count_writes(source["body"])
        args = [f"{'mut ' if writes[arg['name']] else ''}v_{arg['name']}: {self.kind(arg['ty'], True)}"
                for arg in params]
        if self.bounded_loops:
            args.append("_v_backedges: &mut usize")
        arguments = ", ".join(args).replace("&'a ", "&").replace("<'a>", "<'_>")
        signature = f"fn native_{name}({arguments}) -> Option<{self.kind(source['result'])}>"
        locals_decl = []
        for local in locals_:
            variable = local["name"]
            mutable = "mut " if writes[variable] > 1 else ""
            locals_decl.append(f"    let {mutable}v_{variable}: {self.kind(local['ty'])};")
        self.functions_used[name] = "\n".join([
            signature + " {", *locals_decl, *self.statements(source["body"], env, "    ", tail=True), "}",
        ])

    def definitions(self):
        rendered = {}
        while self.types - rendered.keys():
            for category, name in sorted(self.types - rendered.keys()):
                borrows = self.borrows({category: name})
                suffix = "<'a>" if borrows else ""
                if category == "record":
                    fields = [f"    pub(crate) f_{field['name']}: {self.kind(field['ty'], borrows)},"
                              for field in self.records[name]]
                    lines = ["#[derive(Clone, Copy)]", f"pub(crate) struct Native{name}{suffix} {{", *fields, "}"]
                else:
                    variants = []
                    for tag, variant in enumerate(self.enums[name]):
                        payload = variant["payload"]
                        item = f"    V{tag}"
                        if payload is not None:
                            item += f"({self.kind(payload, borrows)})"
                        variants.append(item + ",")
                    lines = ["#[derive(Clone, Copy)]", f"enum Native{name}{suffix} {{", *variants, "}"]
                rendered[category, name] = "\n".join(lines)
        return rendered.values()


def emit(program, exports, *, bounded_loops=False):
    if program[:6] != b"FSM\x00\x02\x00":
        raise ValueError("native generation requires a compiled V2 structured module")
    emitter = Emitter(json.loads(program[6:]), bounded_loops)
    for export in exports:
        if not emitter.functions[export]["export"]:
            raise ValueError(f"{export} is not an export")
        emitter.functions_used[export] = None
    while pending := [name for name, code in emitter.functions_used.items() if code is None]:
        for name in pending:
            emitter.function(name)
    digest = hashlib.sha256(program).digest()
    identity = ", ".join(f"0x{byte:02x}" for byte in digest)
    helpers = []
    if emitter.needs_f32:
        helpers.append("""fn native_canonical_f32(value: f32) -> f32 {
    if value.is_nan() { f32::from_bits(0x7fc0_0000) } else { value }
}""")

    if emitter.needs_argmin:
        helpers.append("""fn native_arg_min_f32(values: &[f32]) -> u64 {
    let mut best = None;
    for (index, &value) in values.iter().enumerate() {
        if best.is_none_or(|(_, old)| value < old) {
            best = Some((index, value));
        }
    }
    best.map_or(values.len(), |(index, _)| index) as u64
}""")
    return "\n\n".join([
        "// Generated from checked FluxR structured source. Edit the source program.",
        f"const NATIVE_PROGRAM_SHA256: [u8; 32] = [{identity}];",
        *emitter.definitions(), *emitter.functions_used.values(), *helpers,
    ]) + "\n"
