"""Sonde del lettore JSON dei documenti di controllo."""

import io
import json
import unittest

try:
    from . import strict_json
except ImportError:
    import strict_json


class StrictJsonTests(unittest.TestCase):
    def assert_duplicate(self, text):
        for read in (lambda: strict_json.loads(text),
                     lambda: strict_json.load(io.StringIO(text))):
            with self.subTest(read=read):
                with self.assertRaises(json.JSONDecodeError) as caught:
                    read()
                self.assertEqual(caught.exception.msg, "chiave JSON duplicata")
                self.assertEqual(caught.exception.doc, "")
                self.assertEqual(str(caught.exception),
                                 "chiave JSON duplicata: line 1 column 1 (char 0)")

    def test_surface_duplicate(self):
        self.assert_duplicate('{"segreto": "dato-uno", "segreto": "dato-due"}')

    def test_nested_duplicate(self):
        self.assert_duplicate('{"esterno": {"segreto": 1, "segreto": 2}}')

    def test_duplicate_in_array(self):
        self.assert_duplicate('[{"segreto": 1, "segreto": 2}]')

    def test_valid_document(self):
        text = '{"oggetti": [{"a": 1}, {"a": 2}], "vuoto": {}, "ok": true}'
        expected = {"oggetti": [{"a": 1}, {"a": 2}], "vuoto": {}, "ok": True}
        self.assertEqual(strict_json.loads(text), expected)
        self.assertEqual(strict_json.loads(text.encode("utf-8")), expected)
        self.assertEqual(strict_json.load(io.StringIO(text)), expected)


if __name__ == "__main__":
    unittest.main()
