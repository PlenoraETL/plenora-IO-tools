"""Le identità del wire sono obbligatorie su entrambi i backend."""
import json
import os
import sys
import unittest
from pathlib import Path
from unittest import mock
from plenora_io import ProtocolError, BinaryNotFound
from plenora_io._process import Runner, NativeRunner, Completed
from plenora_io._discovery import trova_binario


class WireIdentity(unittest.TestCase):
    CONTRACTS = {
        "catalog": "plenora-io-catalog-v2", "capabilities": "plenora-capabilities-v2",
        "inspect": "plenora-io-inspect-v1", "layers": "plenora-io-layers-v1",
        "read": "plenora-io-read-result-v2", "write": "plenora-io-write-result-v2",
        "convert": "plenora-io-convert-v1", "--version": "plenora-io-version-v2",
    }

    def test_every_envelope_requires_exact_identity(self):
        for command, contract in self.CONTRACTS.items():
            for exit_code in (0, 5):
                valid = {"status": "ok" if exit_code == 0 else "error", "protocol_version": 2,
                         "contract": contract if exit_code == 0 else "plenora-error-v1", "result": {}}
                completed = lambda value: Completed(argv=[command], exit_code=exit_code,
                                                     stdout=json.dumps(value), stderr="")
                self.assertEqual(Runner._decode(completed(valid), "stdout"), valid)
                for key, values in {"protocol_version": [None, True, 1, 99, "2"],
                                    "contract": [None, 2, "SECRET", "plenora-io-catalog-v1"]}.items():
                    for value in values:
                        with self.subTest(command=command, exit=exit_code, key=key, value=value):
                            bad = {**valid, key: value}
                            with self.assertRaises(ProtocolError) as raised:
                                Runner._decode(completed(bad), "stdout")
                            self.assertNotIn("SECRET", str(raised.exception))
                    bad = dict(valid)
                    del bad[key]
                    with self.assertRaises(ProtocolError):
                        Runner._decode(completed(bad), "stdout")

    def test_native_violation_after_write_retains_unknown(self):
        native = mock.Mock()
        native.esegui.return_value = (0, json.dumps({"status": "ok", "protocol_version": 99,
                                                   "contract": "plenora-io-write-result-v2", "result": {}}), False)
        with self.assertRaises(ProtocolError) as raised:
            NativeRunner(native).run(["write", "in", "out", "--to", "ipc"])
        self.assertEqual(raised.exception.remote_effect, "unknown")

    def test_explicit_and_environment_never_fall_back(self):
        for value in ("", "does-not-exist-r5"):
            with mock.patch.dict(os.environ, {"PLENORA_IO_BIN": value}), \
                    mock.patch("plenora_io._discovery._cerca_nel_path") as fallback:
                with self.assertRaises(BinaryNotFound):
                    trova_binario()
                fallback.assert_not_called()
        with mock.patch.dict(os.environ, {"PLENORA_IO_BIN": sys.executable}):
            with self.assertRaises(BinaryNotFound):
                trova_binario(Path("does-not-exist-r5"))


    def test_layer_local_error_is_an_envelope_too(self):
        from plenora_io import LayerSummary
        error = {"protocol_version": 2, "contract": "plenora-error-v1", "status": "error",
                 "error": {"code": "UNSUPPORTED", "category": "unsupported", "phase": "read",
                           "remote_effect": "none", "retry": {"kind": "never"}, "message": "rifiutato"}}
        layer = {"id": 0, "name": "x", "field_count": 0, "geometry_crs": None,
                 "status": {"readable": False, "error": error}}
        self.assertFalse(LayerSummary.from_json(layer).status["readable"])
        error["protocol_version"] = 99
        with self.assertRaises(ProtocolError):
            LayerSummary.from_json(layer)

    def test_read_status_is_derived_without_changing_raw(self):
        from plenora_io import Validation
        from test_models import validazione_sana
        value = validazione_sana()
        value["layer"].pop("status", None)
        result = Validation.from_json(value)
        self.assertTrue(result.layer.status["readable"])
        self.assertNotIn("status", result.layer.raw)
