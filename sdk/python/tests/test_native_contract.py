"""Boundary behavior of the native binding, independent of SDK validation."""
import json
import unittest

from _nativo import binario_nativo


class NativeBoundaryContract(unittest.TestCase):
    def setUp(self):
        if binario_nativo() is None:
            self.skipTest("serve il backend plenora-io")
        from plenora_io import _native
        self.native = _native

    def test_unrepresentable_timeout_is_rejected_before_dispatch(self):
        for timeout in (-1.0, float("nan"), float("inf")):
            with self.subTest(timeout=timeout):
                with self.assertRaisesRegex(RuntimeError, "timeout non rappresentabile"):
                    self.native.esegui(["catalog"], timeout=timeout)

    def test_catalog_returns_protocol_document_without_timeout(self):
        code, stdout, timed_out = self.native.esegui(["catalog"])
        self.assertEqual(code, 0)
        self.assertEqual(json.loads(stdout)["status"], "ok")
        self.assertIs(timed_out, False)

    def test_expired_deadline_is_reported_even_when_command_finishes(self):
        _, stdout, timed_out = self.native.esegui(["catalog"], timeout=0.0)
        self.assertIs(timed_out, True)
        self.assertIsInstance(json.loads(stdout), dict)

    def test_callback_exception_is_preserved_after_command_stops(self):
        error = ValueError("callback failure")

        def interrupted():
            raise error

        with self.assertRaises(ValueError) as caught:
            self.native.esegui(["catalog"], interrotto=interrupted)
        self.assertIs(caught.exception, error)


if __name__ == "__main__":
    unittest.main()
