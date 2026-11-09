"""Canonical ordered effects packet specimen. All slots and tags are positional."""

MAX_BYTES = 1024
MAX_DEPTH = 48
OPS = ('not', 'neg', 'if', '&', '|', '=', '!=', '<', '<=', '>', '>=', '+', '-', '*', '%',
       'index', 'thresholds', 'tuple', 'abs', 'min', 'f32', 'u32', 'phase', 'negative', 'bool')
ARITY = {'not': 1, 'neg': 1, 'if': 3, 'abs': 1, 'min': 2, 'f32': 1, 'u32': 1, 'phase': 4}
CALLS = ('abs', 'min', 'f32', 'u32', 'phase')
RELATIONS = ('=', '!=', '<', '<=', '>', '>=')
TYPES = ('none', 'b', 'u', 'i', 'f', 'list', 'product')
NUMBERS = ('u', 'i', 'f')


def assert_that(condition, message):
    if not condition:
        raise ValueError(message)


def width(size):
    assert_that(size > 0, 'empty ordinal domain')
    return (size - 1).bit_length()


class Writer:
    def __init__(self, magic):
        self.bits = f'{magic:08b}'

    def put(self, number, bits):
        assert_that(type(number) is int and 0 <= number < 1 << bits, 'value outside bit width')
        if bits:
            self.bits += f'{number:0{bits}b}'

    def uint(self, value):
        assert_that(type(value) is int and 0 <= value < 2**64, 'unsigned value outside range')
        while value >= 8:
            self.put(8 | (value & 7), 4)
            value >>= 3
        self.put(value, 4)

    def finish(self):
        pad = -len(self.bits) % 8
        result = (int(self.bits, 2) << pad).to_bytes((len(self.bits)+pad)//8, 'big')
        assert_that(len(result) <= MAX_BYTES, 'packet exceeds byte limit')
        return result


class Reader:
    def __init__(self, data, magic):
        assert_that(0 < len(data) <= MAX_BYTES, 'packet exceeds byte limit')
        self.bits = ''.join(f'{x:08b}' for x in data)
        self.pos = 0
        assert_that(self.take(8) == magic, 'unknown packet version')

    def take(self, bits):
        end = self.pos + bits
        assert_that(end <= len(self.bits), 'truncated packet')
        result = int(self.bits[self.pos:end] or '0', 2)
        self.pos = end
        return result

    def uint(self):
        result = 0
        for shift in range(0, 66, 3):
            group = self.take(4)
            result |= (group & 7) << shift
            if group < 8:
                assert_that((shift == 0 or group != 0) and result < 2**64, 'noncanonical unsigned value')
                return result
        raise ValueError('unterminated unsigned value')

    def finish(self):
        tail = self.bits[self.pos:]
        assert_that(len(tail) < 8 and '1' not in tail, 'noncanonical trailing bits')


def write_type(w, shape):
    kind = shape[0] if isinstance(shape, tuple) else shape
    w.put(TYPES.index(kind), 3)
    if kind == 'list':
        write_type(w, shape[1])
    if kind == 'product':
        w.uint(len(shape)-1)
        for element in shape[1:]:
            write_type(w, element)


def read_type(r, depth=0):
    assert_that(depth < 8, 'type depth exceeded')
    code = r.take(3)
    assert_that(code < len(TYPES), 'unknown type')
    kind = TYPES[code]
    if kind == 'list':
        return (kind, read_type(r, depth+1))
    if kind == 'product':
        size = r.uint()
        assert_that(0 < size <= 16, 'product arity outside bounds')
        return (kind, *(read_type(r, depth+1) for _ in range(size)))
    return kind


def encode_abi(inputs, outputs):
    w = Writer(0xA1)
    w.uint(len(inputs))
    for shape in inputs:
        write_type(w, shape)
    w.uint(len(outputs))
    for tag, shape in sorted(outputs.items()):
        w.uint(tag)
        write_type(w, shape)
    return w.finish()


def decode_abi(data):
    r = Reader(data, 0xA1)
    size = r.uint()
    assert_that(size <= 64, 'too many input slots')
    inputs = tuple(read_type(r) for _ in range(size))
    size = r.uint()
    assert_that(size <= 64, 'too many output tags')
    outputs = {}
    for _ in range(size):
        tag = r.uint()
        assert_that(not outputs or tag > next(reversed(outputs)), 'output tags must be strictly sorted')
        outputs[tag] = read_type(r)
    r.finish()
    return inputs, outputs


def typed_expr(node, slots):
    op, *args = node
    if op == 'var':
        assert_that(0 <= args[0] < len(slots), 'invalid slot reference')
        return slots[args[0]]
    if op in ('uint', 'negative', 'f32bits', 'bool'):
        return {'uint': 'u', 'negative': 'i', 'f32bits': 'f', 'bool': 'b'}[op]
    if op == 'thresholds':
        _, refs, bound = args
        values = [typed_expr(('var', ref), slots) for ref in refs]
        bound_type = typed_expr(bound, slots)
        bounds = bound_type[1:] if isinstance(bound_type, tuple) else (bound_type,) * len(values)
        assert_that(len(values) == len(bounds) and all(t in NUMBERS for t in (*values, *bounds)),
                    'thresholds require equally sized numeric operands')
        return 'b'
    types = tuple(typed_expr(expr, slots) for expr in args)
    if op == 'tuple':
        return ('product', *types)
    if op == 'if':
        assert_that(types[0] in (*NUMBERS, 'b') and types[1] == types[2], 'conditional type mismatch')
        return types[1]
    if op == 'index':
        source, index = types
        assert_that(isinstance(source, tuple) and source[0] == 'list' and index == 'u', 'invalid list index')
        return source[1]
    if op in ('not', '&', '|'):
        assert_that(all(t in (*NUMBERS, 'b') for t in types), 'invalid truth operand')
        return 'b'
    if op in RELATIONS:
        assert_that(all(t in NUMBERS for t in types) or (op in ('=', '!=') and types == ('b','b')),
                    'invalid scalar comparison')
        return 'b'
    assert_that(all(t in NUMBERS for t in types), 'numeric operand required')
    if op == 'phase':
        assert_that(types == ('u','u','u','u'), 'phase requires unsigned operands')
        return 'b'
    if op == 'u32':
        assert_that(types == ('u',), 'u32 truncation requires unsigned operand')
        return 'u'
    if op == 'f32':
        return 'f'
    if op == 'neg' and types[0] == 'u':
        return 'i'
    return 'f' if 'f' in types else 'i' if 'i' in types else 'u'


def write_expr(w, node, slots):
    op, *args = node
    if op == 'var':
        w.put(0, 1)
        w.put(args[0], width(slots))
    elif op == 'uint':
        w.put(2, 2)
        w.uint(args[0])
    elif op == 'f32bits':
        w.put(6, 3)
        w.put(args[0], 32)
    else:
        w.put(7, 3)
        w.put(OPS.index(op), 5)
        if op == 'negative':
            w.uint(args[0])
        elif op == 'bool':
            w.put(int(args[0]), 1)
        elif op == 'thresholds':
            cmp, refs, bound = args
            w.put(RELATIONS.index(cmp), 3)
            w.uint(len(refs))
            for ref in refs:
                w.put(ref, width(slots))
            write_expr(w, bound, slots)
        else:
            if op == 'tuple':
                w.uint(len(args))
            for arg in args:
                write_expr(w, arg, slots)


def read_expr(r, slots, depth=0):
    assert_that(depth < MAX_DEPTH, 'expression depth exceeded')
    if not r.take(1):
        index = r.take(width(slots))
        assert_that(index < slots, 'invalid slot ordinal')
        return ('var', index)
    if not r.take(1):
        return ('uint', r.uint())
    if not r.take(1):
        return ('f32bits', r.take(32))
    code = r.take(5)
    assert_that(code < len(OPS), 'unknown expression opcode')
    op = OPS[code]
    if op == 'negative':
        magnitude = r.uint()
        assert_that(0 < magnitude <= 2**63, 'signed magnitude outside range')
        return (op, magnitude)
    if op == 'bool':
        return (op, bool(r.take(1)))
    if op == 'thresholds':
        compare = r.take(3)
        size = r.uint()
        assert_that(compare < len(RELATIONS) and 1 < size <= 16, 'invalid grouped comparison')
        refs = tuple(r.take(width(slots)) for _ in range(size))
        return (op, RELATIONS[compare], refs, read_expr(r, slots, depth+1))
    size = r.uint() if op == 'tuple' else ARITY.get(op, 2)
    assert_that(0 < size <= 16, 'expression arity outside bounds')
    return (op, *(read_expr(r, slots, depth+1) for _ in range(size)))


def eval_expr(node):
    op, *args = node
    if op in ('var', 'f32bits'):
        return node
    if op in ('uint', 'negative', 'bool'):
        x = args[0]
        return ('const', -x if op == 'negative' else x)
    if op == 'thresholds':
        return (op, args[0], args[1], eval_expr(args[2]))
    operands = tuple(map(eval_expr, args))
    if op == 'tuple':
        return (op, operands)
    if op in CALLS:
        return ('call', ('var', op), operands)
    return (op, *operands)


def checked_truth(expr, slots):
    assert_that(typed_expr(expr, slots) in (*NUMBERS, 'b'), 'guard requires scalar truth operand')


def encode_code(inputs, outputs, code):
    w = Writer(0xC1)
    slots = list(inputs)
    tags = tuple(sorted(outputs))
    w.uint(len(code))
    for op, *args in code:
        w.put(('guard','set','emit','diff').index(op), 2)
        if op in ('guard','set'):
            write_expr(w, args[0], len(slots))
            if op == 'set':
                slots.append(typed_expr(args[0], slots))
        elif op == 'emit':
            tag, guard, payload = args
            w.put(tags.index(tag), width(len(tags)))
            write_expr(w, guard, len(slots))
            if payload is not None:
                write_expr(w, payload, len(slots))
        else:
            flag, priority, flagtag, ranktag, mode, default, rows = args
            for slot in (flag, priority):
                w.put(slot, width(len(slots)))
            for tag in (flagtag, ranktag):
                w.put(tags.index(tag), width(len(tags)))
            w.put(int(mode == 'set'), 1)
            w.put(int(default is not None), 1)
            if default is not None:
                w.uint(default)
            w.uint(len(rows))
            for i, test, rank, overlay in rows:
                w.uint(i)
                write_expr(w, test, len(slots))
                write_expr(w, rank, len(slots))
                w.put(int(overlay is not None), 1)
                if overlay is not None:
                    write_expr(w, overlay, len(slots))
    return w.finish()


def decode_code(inputs, outputs, data):
    r = Reader(data, 0xC1)
    slots = list(inputs)
    tags = tuple(sorted(outputs))
    code = []
    count = r.uint()
    assert_that(count <= 128, 'too many instructions')

    def expr():
        return read_expr(r, len(slots))

    def tag():
        index = r.take(width(len(tags)))
        assert_that(index < len(tags), 'invalid effect tag')
        return tags[index]

    for _ in range(count):
        op = ('guard','set','emit','diff')[r.take(2)]
        if op == 'guard':
            guard = expr()
            checked_truth(guard, slots)
            code.append((op, guard))
        elif op == 'set':
            payload = expr()
            slots.append(typed_expr(payload, slots))
            code.append((op, payload))
        elif op == 'emit':
            effect = tag()
            guard = expr()
            checked_truth(guard, slots)
            payload = None if outputs[effect] == 'none' else expr()
            assert_that(payload is None or typed_expr(payload, slots) == outputs[effect], 'effect payload type mismatch')
            code.append((op, effect, guard, payload))
        else:
            flag, priority = (r.take(width(len(slots))) for _ in range(2))
            assert_that(typed_expr(('var',flag), slots) == ('list','b') and
                        typed_expr(('var',priority), slots) == ('list','u'), 'diff slots require Bool and U64 lists')
            flagtag, ranktag = tag(), tag()
            mode = 'set' if r.take(1) else 'flip'
            default = r.uint() if r.take(1) else None
            assert_that(outputs[flagtag] == (('product','u','b') if mode == 'set' else 'u') and
                        outputs[ranktag] == ('product','u','u'), 'diff tag shapes disagree with mode')
            size = r.uint()
            assert_that(0 < size <= 128, 'diff row count outside bounds')
            rows = []
            for _ in range(size):
                i, test, rank = r.uint(), expr(), expr()
                overlay = expr() if r.take(1) else None
                checked_truth(test, slots)
                assert_that(typed_expr(rank, slots) == 'u', 'diff rank requires U64')
                if overlay is not None:
                    checked_truth(overlay, slots)
                rows.append((i, test, rank, overlay))
            code.append((op, flag, priority, flagtag, ranktag, mode, default, tuple(rows)))
    r.finish()
    return tuple(code)
