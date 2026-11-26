"""Project frontend preserves checked UTF-8 literals; Rust owns semantic admission."""
import importlib.util
import json
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("fluxr_utf8_frontend", ROOT / "tools/fluxr.py")
FRONTEND = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = FRONTEND
SPEC.loader.exec_module(FRONTEND)

class Utf8Frontend(unittest.TestCase):
    def test_unicode_spaces_escapes_and_native_spans(self):
        checked = 'pub bytes 0 list uint ret utf8 ' + json.dumps('café 🦀\n " \\', ensure_ascii=False)
        native = 'fn main() { let _ = "é"; }'.encode()
        source = b'fluxr 1\n' + checked.encode() + b'\nrust ' + str(len(native)).encode() + b'\n' + native
        document = FRONTEND.parse_source(source)
        self.assertEqual(document.checked, checked.encode())
        self.assertEqual(len(document.native_spans), 1)

    def test_rejects_invalid_strings_and_nonstring(self):
        for expression in ['utf8', 'utf8 bare', 'utf8 "bad\\x00"', 'utf8 "\\ud800"', 'utf8 "unterminated', 'utf8 "one"tail', 'utf8 "raw\nnewline"']:
            with self.subTest(expression=expression), self.assertRaises(FRONTEND.NativeError):
                FRONTEND.parse_source(('fluxr 1\npub bytes 0 list uint ret ' + expression).encode())

    def test_byte_limit_and_expanded_expression_work(self):
        def parse(expression):
            parser = FRONTEND.SourceParser(('fluxr 1\npub bytes 0 list uint ret ' + expression).encode())
            parser.document()
            return parser.nodes
        self.assertEqual(parse('utf8 "éA"'), parse('list uint 3 195u 169u 65u'))
        parse('utf8 "' + 'a' * 32768 + '"')
        with self.assertRaises(FRONTEND.NativeError): parse('utf8 "' + 'a' * 32769 + '"')
        with self.assertRaises(FRONTEND.NativeError): parse('utf8 "' + 'é' * 16385 + '"')

if __name__ == '__main__':
    unittest.main()
