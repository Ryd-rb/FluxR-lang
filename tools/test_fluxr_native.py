"""Canonical FluxR compiler, import adapter and mandatory Runtime admission tests.

Real Rust behavior, VM injection and AOT execution are separately qualified by
actual executable tests. These tests cover frontend structure and failure policy.
"""
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import tomllib
import unittest

import fluxr as n
import fluxr_native as compatibility


def native_text(text):
    return n.parse_source(n.HEADER + text.encode()).native.decode()


class Frontend(unittest.TestCase):
    def test_compatibility_name_has_one_implementation(self):
        self.assertIs(compatibility.emit, n.emit)
        self.assertIs(compatibility.load_project, n.load_project)
        self.assertEqual(n.HEADER, b'fluxr 1\n')

    def test_raw_payload_preserves_bytes_and_cannot_remove_runtime(self):
        for data in [b'', b'fn main() {}', b'// no final newline', '// € λ\r\nfn main() {}\n'.encode()]:
            self.assertEqual(n.parse_source(n.wrap_rust(data)).native, data)
        output = n.emit(n.wrap_rust(b'fn main() {}'))
        self.assertIn(b'::fluxr::program!', output)
        self.assertIn(b'::fluxr::runtime::EmbeddedSource', output)
        self.assertIn(b'binary;', output)

    def test_legacy_native_is_explicit_import_only(self):
        legacy = n.LEGACY_NATIVE_HEADER + b'use std::sync::Arc\nfn main 0 unit 0\n'
        with self.assertRaises(n.NativeError): n.emit(legacy)
        migrated = n.import_native(legacy)
        self.assertTrue(migrated.startswith(n.HEADER))
        self.assertIn(b'native-use std::sync::Arc', migrated)
        self.assertIn(b'native-fn main', migrated)
        self.assertIn(b'::fluxr::program!', n.emit(migrated))

    def test_arbitrary_native_method_and_function_lowering(self):
        output = native_text('native-pubfn bump 1 x "&std::sync::atomic::AtomicUsize" usize 1 '
                             'return method path x fetch_add 2 int 1 path std::sync::atomic::Ordering::AcqRel')
        self.assertIn('pub fn bump(x: &std::sync::atomic::AtomicUsize)', output)
        self.assertIn('(x).fetch_add(1, std::sync::atomic::Ordering::AcqRel)', output)
        self.assertIn('.user_defined_method(7)', native_text('native-fn f 0 unit 1 do method path value user_defined_method 1 int 7'))

    def test_native_expression_vocabulary(self):
        cases = {
            'int -42i64': '-42i64', 'bool true': 'true', 'unit': '()',
            'call std::thread::yield_now 0': 'std::thread::yield_now()',
            'method ref path x len 0': '((&(x))).len()', 'ref path x': '(&(x))',
            'mutref path x': '(&mut (x))', 'deref path x': '(*(x))',
            'not bool false': '(!(false))', 'neg int 2': '(-(2))',
            'try call f 0': '(f())?', 'await path x': '(x).await',
            'cast int 1 u64': '((1) as u64)', 'field path x y': '(x).y',
            'index path x int 0': '(x)[0]', 'array 2 int 1 int 2': '[1, 2]',
            'tuple 1 int 3': '(3,)', 'tuple 0': '()',
            'if bool true int 1 int 2': '(if true { 1 } else { 2 })',
            'block 1 let x int 3 path x': '{\nlet x = 3;\nx\n}',
            'closure 1 x path x': '|x| x', 'move 0 int 2': 'move || 2',
            'unsafe int 1': 'unsafe { 1 }', 'async int 2': 'async { 2 }',
            'async-move int 2': 'async move { 2 }',
            'macro assert_eq 2 int 2 int 2': 'assert_eq!(2, 2)',
            'rust 5\n1 + 2': '(1 + 2)',
        }
        for source, expected in cases.items():
            with self.subTest(source=source):
                parser = n.Parser(n.HEADER + source.encode())
                self.assertEqual(parser.expr(), expected)
                parser.space(); self.assertEqual(parser.pos, len(parser.source))
        for name, symbol in n.BINARY.items():
            self.assertEqual(n.Parser(n.HEADER + f'bin {name} int 1 int 2'.encode()).expr(), f'((1) {symbol} (2))')

    def test_native_statements(self):
        output = native_text('native-fn f 0 unit 9 let x int 1 var y u64 int 0 let-type z u64 int 3 '
                             'set path y path z do call g 0 for i bin range int 0 int 2 1 do path i '
                             'while bool false 1 break if-stmt bool true 1 do path x 1 continue tail unit')
        for expected in ['let x = 1;', 'let mut y: u64 = 0;', 'let z: u64 = 3;', 'y = z;', 'g();',
                         'for i in ((0) .. (2))', 'while false', 'break;', 'else', 'continue;', '()\n}']:
            self.assertIn(expected, output)

    def test_rust_string_escaping(self):
        value = '\x01\b\f\0\n\r\t"\\€'
        result = n.Parser(n.HEADER + ('str ' + json.dumps(value)).encode()).expr()
        self.assertEqual(result, '"\\u{1}\\u{8}\\u{c}\\0\\n\\r\\t\\"\\\\€"')
        self.assertEqual(n.rust_string(value), result)

    def test_checked_and_native_share_source_and_framed_digest(self):
        source = n.HEADER + b'pub answer 0 int ret 42\n native-fn main 0 unit 0 \npub ok 0 bool ret true\n'
        document = n.parse_source(source)
        self.assertEqual(document.checked_exports, ['answer', 'ok'])
        self.assertEqual(document.checked, b'pub answer 0 int ret 42\npub ok 0 bool ret true')
        span = b'native-fn main 0 unit 0'
        expected = hashlib.sha256(n.NATIVE_DOMAIN + len(span).to_bytes(8, 'little') + span).hexdigest()
        self.assertEqual(document.native_digest, expected)
        self.assertIn(document.native_digest.encode(), n.emit(source))

    def test_shared_rust_python_span_corpus(self):
        rows = json.loads((Path(__file__).parent / 'unified_tests/native_spans.json').read_text())
        for row in rows:
            with self.subTest(name=row['name']):
                document = n.parse_source(row['source'].encode())
                self.assertEqual(document.native_digest, row['native_digest'])
                self.assertEqual([list(span) for span in document.native_spans], row['native_spans'])
                self.assertEqual(document.checked.decode(), row['checked'])

    def test_checked_entry_uses_same_runtime(self):
        output = n.emit(n.HEADER + b'pub main 0 int ret add 40 2\n')
        self.assertIn(b'run_embedded_entry("main.fluxr", "main")', output)
        self.assertIn(b'::fluxr::program!', output)
        with self.assertRaises(n.NativeError): n.emit(n.HEADER + b'pub other 0 int ret 42')

    def test_checked_syntax_and_host_boundary(self):
        source = (n.HEADER + b'inputs 1 state uint\nrec Pair 2 a int b int\nenum Option 2 None none Some int\n'
                  b'fn value 1 x int int ret x\npub bridge 1 x int int ret host exchange int 1 int x\n'
                  b'pub main 0 int do 1 total int 3 set total 0 while lt total 4 1 set total add total 1 ret total\n')
        document = n.parse_source(source)
        self.assertEqual(document.checked_exports, ['bridge', 'main'])
        self.assertFalse(document.native)

    def test_checked_decimal_fixture_pairs_preserve_text_native_spans_and_charges(self):
        native = b'native-fn spelling 0 "&\'static str" 1 tail str "1f -0f"'
        expected_digest = hashlib.sha256(
            n.NATIVE_DOMAIN + len(native).to_bytes(8, 'little') + native).hexdigest()
        fixtures = n.ROOT/'benchmarks/unified-floats-v1'
        for name in ['affine', 'threshold', 'constants', 'loop', 'special']:
            states = []
            for arm in ['before', 'after']:
                source = (fixtures/f'{name}.{arm}.fluxr').read_bytes()
                combined = source + native
                parser = n.SourceParser(combined)
                document = parser.document()
                self.assertEqual(document.checked, source[len(n.HEADER):].strip())
                self.assertEqual(document.checked_exports, ['run'])
                self.assertEqual(document.native_spans, [(len(source), len(combined))])
                self.assertEqual(document.native_digest, expected_digest)
                states.append((parser.nodes, parser.depth))
            self.assertEqual(*states, name)

    def test_checked_decimal_spelling_preserves_signed_scientific_text_and_names(self):
        for literal in ['0f', '-0f', '0.5f', '-2.5f', '0001.0E+00f', '1e-45f',
                        '-1e-100f', '3.4028235e38f', '1.000000059604644775390626f']:
            checked = f'pub main 0 float ret {literal}'.encode()
            source = n.HEADER + checked
            with self.subTest(literal=literal):
                self.assertEqual(n.parse_source(source).checked, checked)
                emitted = n.emit(source)
                self.assertIn(literal.encode(), emitted)
                self.assertIn(b'::fluxr::program!', emitted)
        for name in ['nanf', 'inff', 'NaNf', 'Infinityf', 'f']:
            checked = f'pub main 1 {name} float float ret {name}'.encode()
            self.assertEqual(n.parse_source(n.HEADER + checked).checked, checked)

    def test_checked_decimal_syntax_and_byte_limit_do_not_change_raw_bit_reader(self):
        boundary = '0.' + '0' * 253 + 'f'
        self.assertEqual(len(boundary), 256)
        n.parse_source(n.HEADER + f'pub main 0 float ret {boundary}'.encode())
        with self.assertRaisesRegex(n.NativeError, 'decimal float literal byte limit'):
            n.parse_source(n.HEADER + ('pub main 0 float ret 0.' + '0' * 254 + 'f').encode())
        for literal in ['1.f', '.5f', '-.5f', '+1f', '1_0f', '1ef', '1e+f', '1e-f',
                        '1.2.3f', '1e1.5f', '--1f', '-inff', '-NaNf', '1ff', '0xzzf', '1\u00a0f']:
            with self.subTest(literal=literal), self.assertRaises(n.NativeError):
                n.parse_source(n.HEADER + f'pub main 0 float ret {literal}'.encode())
        # The new decimal bound does not amend retained raw-bit spellings.
        for raw in ['0x80000000f', '0x7fa00001f', '0x' + '0' * 300 + '1f']:
            checked = f'pub main 0 float ret {raw}'.encode()
            self.assertEqual(n.parse_source(n.HEADER + checked).checked, checked)

    def test_checked_decimal_literals_have_identical_node_and_depth_admission(self):
        for decimal, raw in [('1f', '0x3f800000f'), ('-0f', '0x80000000f'), ('1e-45f', '0x1f')]:
            for nodes in [0, n.MAX_NODES - 1, n.MAX_NODES]:
                for depth in [0, n.MAX_DEPTH - 1, n.MAX_DEPTH]:
                    outcomes = []
                    for literal in [decimal, raw]:
                        parser = n.SourceParser(n.HEADER + literal.encode())
                        parser.nodes, parser.depth = nodes, depth
                        try:
                            parser.checked_expr()
                            outcome = 'accepted'
                        except n.NativeError as error:
                            outcome = str(error).partition(': ')[2]
                        outcomes.append((outcome, parser.nodes, parser.depth))
                    self.assertEqual(*outcomes, (decimal, nodes, depth))

    def test_projection_keeps_semantic_overflow_for_authoritative_rust_admission(self):
        # These tests cover projection, not successful compilation or execution.
        # The real Rust/CLI/application gates must reject the preserved source.
        for literal in ['1e39f', '-1e39f', '1e9999f']:
            source = n.HEADER + f'pub main 0 float ret {literal}'.encode()
            self.assertEqual(n.parse_source(source).source, source)
            emitted = n.emit(source)
            self.assertIn(literal.encode(), emitted)
            self.assertIn(b'::fluxr::program!', emitted)
            self.assertIn(b'run_embedded_entry("main.fluxr", "main")', emitted)

    def test_checked_updates_preserve_expanded_frontend_depth_and_node_charges(self):
        for short, expanded in [('bump x add 1 2', 'set x add local x add 1 2'),
                                ('scale host 3', 'set host mul local host 3')]:
            name = 'host' if short.startswith('scale') else 'x'
            states = []
            for update in [short, expanded]:
                source = n.HEADER + f'pub calc 1 {name} int int do 0 2 {update} ret local {name}'.encode()
                parser = n.SourceParser(source)
                document = parser.document()
                self.assertEqual(document.checked_exports, ['calc'])
                self.assertEqual(document.checked, source[len(n.HEADER):])
                states.append((parser.nodes, parser.depth))
            self.assertEqual(*states)
        for amount_depth in [0, n.MAX_DEPTH - 4, n.MAX_DEPTH, n.MAX_DEPTH + 1]:
            outcomes = []
            for update in ['bump x ', 'set x add x ']:
                source = n.HEADER + ('pub calc 1 x int int do 0 1 ' + update
                                     + 'add 1 ' * amount_depth + '1').encode()
                try:
                    n.parse_source(source)
                    outcomes.append('accepted')
                except n.NativeError as error:
                    # Diagnostics include source offsets, which shorter source changes.
                    outcomes.append(str(error).partition(': ')[2])
            self.assertEqual(*outcomes)

    def test_checked_updates_keep_native_spans_and_reject_incomplete_forms(self):
        native = b'native-fn helper 0 unit 0'
        source = n.HEADER + b'pub main 0 int do 1 total int 4 set total 1 bump total 2 scale total 3 ret total\n' + native
        document = n.parse_source(source)
        self.assertEqual(document.native_spans, [(source.index(native), len(source))])
        self.assertEqual(document.native_digest, hashlib.sha256(
            n.NATIVE_DOMAIN + len(native).to_bytes(8, 'little') + native).hexdigest())
        for update in ['bump', 'bump x', 'scale x', 'bump 1 2', 'scale x ?', 'bump x 1 extra']:
            with self.subTest(update=update), self.assertRaises(n.NativeError):
                n.parse_source(n.HEADER + ('pub f 1 x int int do 0 1 ' + update).encode())

    def test_signed_unit_steps_match_expanded_frontend_charges_and_failures(self):
        for short, expanded in [('next next', 'set next add local next 1'),
                                ('prev prev', 'set prev sub local prev 1')]:
            for nodes in [0, n.MAX_NODES - 4, n.MAX_NODES - 3, n.MAX_NODES]:
                for depth in [0, n.MAX_DEPTH - 3, n.MAX_DEPTH - 2, n.MAX_DEPTH]:
                    outcomes = []
                    for statement in [short, expanded]:
                        parser = n.SourceParser(n.HEADER + ('1 ' + statement).encode())
                        parser.nodes, parser.depth = nodes, depth
                        try:
                            parser.checked_block()
                            outcome = 'accepted'
                        except n.NativeError as error:
                            outcome = str(error).partition(': ')[2]
                        outcomes.append((outcome, parser.nodes, parser.depth))
                    self.assertEqual(*outcomes, (short, nodes, depth))

    def test_signed_steps_preserve_native_bytes_and_keyword_names(self):
        native = 'native-fn greeting 0 "&\'static str" 1 tail str "héllo β"'.encode()
        before = n.HEADER + b'pub main 1 next int int do 0 3 bump next 1 set next sub next 1 ret next\n' + native
        after = n.HEADER + b'pub main 1 next int int do 0 3 next next prev next ret next\n' + native
        expected_digest = hashlib.sha256(
            n.NATIVE_DOMAIN + len(native).to_bytes(8, 'little') + native).hexdigest()
        for source in [before, after]:
            document = n.parse_source(source)
            self.assertEqual(document.checked_exports, ['main'])
            self.assertEqual(document.native_spans, [(source.index(native), len(source))])
            self.assertEqual(document.native_digest, expected_digest)
        for statement in ['next', 'prev', 'next 1', 'prev ?', 'next x 1', 'prev x extra']:
            with self.subTest(statement=statement), self.assertRaises(n.NativeError):
                n.parse_source(n.HEADER + ('pub f 1 x int int do 0 1 ' + statement).encode())

    def test_arithmetic_updates_preserve_expanded_frontend_bounds(self):
        for op in ['sub', 'div', 'rem']:
            short = f'{op} {op} add 1 2'
            expanded = f'set {op} {op} local {op} add 1 2'
            for nodes in [0, n.MAX_NODES - 6, n.MAX_NODES - 4, n.MAX_NODES]:
                for depth in [0, n.MAX_DEPTH - 4, n.MAX_DEPTH - 2, n.MAX_DEPTH]:
                    outcomes = []
                    for statement in [short, expanded]:
                        parser = n.SourceParser(n.HEADER + ('1 ' + statement).encode())
                        parser.nodes, parser.depth = nodes, depth
                        try:
                            parser.checked_block()
                            outcome = 'accepted'
                        except n.NativeError as error:
                            outcome = str(error).partition(': ')[2]
                        outcomes.append((outcome, parser.nodes, parser.depth))
                    self.assertEqual(*outcomes, (op, nodes, depth))

    def test_arithmetic_updates_preserve_native_spans_and_source_bytes(self):
        native = 'native-fn greeting 0 "&\'static str" 1 tail str "héllo β"'.encode()
        expected_digest = hashlib.sha256(
            n.NATIVE_DOMAIN + len(native).to_bytes(8, 'little') + native).hexdigest()
        for op in ['sub', 'div', 'rem']:
            for update in [f'{op} {op} 2', f'set {op} {op} local {op} 2']:
                source = n.HEADER + f'pub calc 1 {op} int int do 0 2 {update} ret local {op}\n'.encode() + native
                document = n.parse_source(source)
                self.assertEqual(document.source, source)
                self.assertEqual(document.checked_exports, ['calc'])
                self.assertEqual(document.native_spans, [(source.index(native), len(source))])
                self.assertEqual(document.native_digest, expected_digest)
                self.assertIn(n.rust_string(source.decode()).encode(), n.emit(source))
            for update in [op, f'{op} x', f'{op} 1 2', f'{op} x ?', f'{op} x 2 extra']:
                with self.subTest(update=update), self.assertRaises(n.NativeError):
                    n.parse_source(n.HEADER + ('pub f 1 x int int do 0 1 ' + update).encode())

    def test_crate_prelude_preserves_attributes_strings_and_docs(self):
        raw = b'//! crate docs\n#![doc = r###"[quoted] no_std"###]\n#![allow(unused)]\nfn main() {}'
        output = n.emit(n.wrap_rust(raw))
        self.assertTrue(output.startswith(b'//! crate docs\n#![doc'))
        self.assertLess(output.index(b'#![allow'), output.index(b'::fluxr::program!'))
        self.assertIn(b'fn main() {}', output)
        for attr in [b'#![no_std]', b'#![no_main]', b'#![cfg_attr(any(), no_main)]', b'#![cfg_attr(all(), no_std)]']:
            with self.subTest(attr=attr), self.assertRaisesRegex(n.NativeError, 'no_main/no_std'):
                n.emit(n.wrap_rust(attr + b'\nfn main() {}'))

    def test_invalid_grammar_bounds_and_headers(self):
        for source in [b'', b'fluxr-native 1\n', b'fluxr 2\n', b'fluxr 1\r\n', n.HEADER+b'\xff']:
            with self.subTest(source=source), self.assertRaises(n.NativeError): n.parse_source(source)
        bad = ['native-fn', 'native-fn x -1 unit 0', 'native-fn x 10001 unit 0',
               'native-fn x 0 unit 1 do bool yes', 'native-fn x 0 unit 1 do int 01',
               'native-fn x 0 unit 1 do int 2.5', 'native-fn x 0 unit 1 do path a();',
               'native-fn x 0 unit 1 do bin unknown int 1 int 1', 'native-fn x 0 unit 1 wrong',
               'rust 5\nabc', 'rust 2 abc', 'rust 1\n€', 'rust -1\na',
               'native-fn f 2 x u64 x u64 unit 0', 'native-fn f 0 "u64;evil" 0',
               'native-fn f 0 "Vec<u8" 0', 'native-fn f 0 unit 1 do str "x"y',
               'native-fn f 0 unit 1 do str "\\ud800"', 'pub bad 0 int wrong 42',
               'pub bad 0 int ret host f int 999999999', '"pub" bad 0 int ret 1']
        for source in bad:
            with self.subTest(source=source), self.assertRaises(n.NativeError): n.parse_source(n.HEADER+source.encode())
        with self.assertRaises(n.NativeError): n.parse_source(n.HEADER+b' '*n.MAX_SOURCE)
        with self.assertRaises(n.NativeError): n.parse_source(n.HEADER+b'native-fn f 0 unit 1 do '+b'ref '*(n.MAX_DEPTH+1)+b'unit')


class Projects(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(); self.root = Path(self.temp.name)
        self.source = self.root/'source'; self.source.mkdir()
        (self.source/'main.fluxr').write_bytes(n.wrap_rust(b'fn main() {}\n'))
        (self.source/'Cargo.toml').write_text('[package]\nname="fixture"\nversion="0.1.0"\nedition="2024"\n')
        (self.source/'Cargo.lock').write_text('version=4\n[[package]]\nname="fixture"\nversion="0.1.0"\n')
        self.config = {'format':'fluxr-project-v1','rustflags':[],'files':[
            dict(source='main.fluxr',output='src/main.rs',mode='source',executable=False),
            dict(source='Cargo.toml',output='Cargo.toml',mode='copy',executable=False),
            dict(source='Cargo.lock',output='Cargo.lock',mode='copy',executable=False)]}
        self.manifest=self.source/'fluxr.json'
    def tearDown(self): self.temp.cleanup()
    def save(self): self.manifest.write_text(json.dumps(self.config))

    def test_export_has_mandatory_runtime_and_exact_source_receipt(self):
        self.save(); out=self.root/'out'; project=n.export_project(self.manifest,out)
        self.assertIn(b'::fluxr::program!',(out/'src/main.rs').read_bytes())
        self.assertTrue((out/'.fluxr-runtime/src/runtime.rs').is_file())
        self.assertTrue((out/'.fluxr-runtime/crates/fluxr-entry/src/lib.rs').is_file())
        cargo=tomllib.loads((out/'Cargo.toml').read_text())
        self.assertEqual(cargo['dependencies']['fluxr']['path'],'.fluxr-runtime')
        packages=tomllib.loads((out/'Cargo.lock').read_text())['package']
        self.assertIn('fluxr',next(r for r in packages if r['name']=='fixture')['dependencies'])
        self.assertTrue(project.receipt['runtime_required'])
        self.assertEqual(project.receipt['files'][0]['output_sha256'],n.sha((out/'src/main.rs').read_bytes()))
        with self.assertRaises(n.NativeError): n.export_project(self.manifest,out)

    def test_copy_cannot_disguise_custom_extension_cargo_entry(self):
        (self.source/'Cargo.toml').write_text('[package]\nname="fixture"\nversion="0.1.0"\n[[bin]]\nname="fixture"\npath="entry.txt"\n')
        self.config['files'][0].update(output='entry.txt',mode='copy');self.save()
        with self.assertRaisesRegex(n.NativeError,'canonical FluxR source'): n.load_project(self.manifest)

    def test_cargo_derives_custom_binary_build_and_auto_targets(self):
        cargo={'package':{'name':'fixture','build':'tools/generate.rs'},'bin':[{'name':'custom','path':'app/start.rs'}]}
        paths={'tools/generate.rs','app/start.rs','src/bin/extra.rs','src/helper.rs'}
        self.assertEqual(n.cargo_targets(cargo,paths),{'tools/generate.rs':'build-script','app/start.rs':'binary','src/bin/extra.rs':'binary','src/helper.rs':'module'})
        self.assertEqual(n.cargo_targets(cargo,paths,testing=True)['app/start.rs'],'test')
        cargo['package']['build']='app/start.rs'
        with self.assertRaisesRegex(n.NativeError,'incompatible'): n.cargo_targets(cargo,paths)

    def test_build_script_is_only_derived_host_tool(self):
        source=n.wrap_rust(b'fn main() {}')
        self.assertEqual(n.emit(source,role='build-script'),b'fn main() {}')
        with self.assertRaises(n.NativeError): n.emit(n.HEADER+b'pub main 0 int ret 1',role='build-script')
        self.config['files'][0]['role']='build-script';self.save()
        with self.assertRaises(n.NativeError): n.load_project(self.manifest)

    def test_reserved_runtime_and_workspace_overrides_refuse(self):
        for cargo in [dict(package={'name':'fixture'},dependencies={'fluxr':{}}),
                      dict(package={'name':'fixture'},workspace={}),
                      dict(package={'name':'fixture'},target={'cfg(unix)':{'dependencies':{'fluxr':{}}}})]:
            with self.subTest(cargo=cargo),self.assertRaises(n.NativeError): n.cargo_targets(cargo,{'src/main.rs'})

    def test_path_policy(self):
        for path in ['', '../x','/tmp/x','a/../b','a//b','a/./b','a\\b','C:/x','a\0b','a.','a /x']:
            with self.subTest(path=path),self.assertRaises(n.NativeError): n.relpath(path)

    def test_symlinks_and_nonregular_sources_refuse(self):
        (self.source/'link').symlink_to(self.source/'main.fluxr')
        (self.source/'linkdir').symlink_to(self.source,target_is_directory=True)
        for name in ['link','linkdir/main.fluxr','.']:
            self.config['files'][0]['source']=name;self.save()
            with self.subTest(name=name),self.assertRaises((n.NativeError,OSError)):n.load_project(self.manifest)

    def test_invalid_manifest_and_output_collisions(self):
        original=json.dumps(self.config)
        edits=[lambda c:c['files'][0].update(mode='copy'),lambda c:c['files'][0].update(mode='native'),
               lambda c:c['files'][0].update(mode=[]),lambda c:c['files'][0].update(executable=1),
               lambda c:c.update(format='fluxr-native-project-v1'),lambda c:c.update(unknown=True),
               lambda c:c['files'].append(dict(c['files'][0])),lambda c:c.update(rustflags=['\x1f']),
               lambda c:c['files'][0].update(output='.fluxr-runtime/src/main.rs'),
               lambda c:c['files'][0].update(output='fluxr-receipt.json')]
        for edit in edits:
            self.config=json.loads(original);edit(self.config);self.save()
            with self.assertRaises(n.NativeError): n.load_project(self.manifest)
        self.manifest.write_text('{"format":1,"format":2}')
        with self.assertRaises(n.NativeError):n.load_project(self.manifest)

    def test_missing_lock_and_invalid_build_options_refuse_before_output(self):
        self.config['files'].pop();self.save()
        for extra in [[],['--jobs','0'],['--timeout','nan'],['--no-run']]:
            output=self.root/'out'
            self.assertEqual(n.main(['build',str(self.manifest),str(output),'--target-dir',str(self.root/'target'),'--logs',str(self.root/'logs'),*extra]),2)
            self.assertFalse(output.exists())

    def test_emit_cli_refuses_overwrite(self):
        output=self.root/'result.rs'
        self.assertEqual(n.main(['emit',str(self.source/'main.fluxr'),str(output)]),0)
        before=output.read_bytes()
        self.assertEqual(n.main(['emit',str(self.source/'main.fluxr'),str(output)]),2)
        self.assertEqual(output.read_bytes(),before)

    def test_process_timeout_is_recorded_and_killed(self):
        result=n.run_logged([sys.executable,'-c','import time;time.sleep(5)'],self.root,self.root/'logs',.05)
        self.assertTrue(result['timed_out']);self.assertNotEqual(result['exit_code'],0)


if __name__=='__main__':unittest.main()
