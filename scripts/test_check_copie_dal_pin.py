"""Le controprove del confronto fra le copie e il checkout fissato."""

from __future__ import annotations

import json
import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import check_copie_dal_pin as gate  # noqa: E402

PIN = "a" * 40


def prepara(radice: pathlib.Path, copia: bytes, originale: bytes) -> tuple[pathlib.Path, pathlib.Path]:
    copie = radice / "copie"
    contratti = radice / "contratti"
    (contratti / "schemas").mkdir(parents=True)
    copie.mkdir()
    (contratti / "schemas" / "x.json").write_bytes(originale)
    (copie / "x.json").write_bytes(copia)
    (copie / "provenienza.json").write_text(
        json.dumps({"revisione": PIN, "file": {"x.json": "schemas/x.json"}}), encoding="utf-8"
    )
    return copie, contratti


class LeCopie(unittest.TestCase):
    def test_identiche_passano(self) -> None:
        with tempfile.TemporaryDirectory() as t:
            copie, contratti = prepara(pathlib.Path(t), b"{}\n", b"{}\n")
            self.assertEqual(gate.verifica(copie, contratti, PIN), [])

    def test_un_byte_diverso_e_rosso(self) -> None:
        with tempfile.TemporaryDirectory() as t:
            copie, contratti = prepara(pathlib.Path(t), b"{}\r\n", b"{}\n")
            errori = gate.verifica(copie, contratti, PIN)
            self.assertTrue(any("differisce" in e for e in errori), errori)

    def test_un_pin_diverso_e_rosso(self) -> None:
        with tempfile.TemporaryDirectory() as t:
            copie, contratti = prepara(pathlib.Path(t), b"{}\n", b"{}\n")
            errori = gate.verifica(copie, contratti, "b" * 40)
            self.assertTrue(any("il pin" in e for e in errori), errori)

    def test_un_file_non_elencato_e_rosso(self) -> None:
        with tempfile.TemporaryDirectory() as t:
            copie, contratti = prepara(pathlib.Path(t), b"{}\n", b"{}\n")
            (copie / "intruso.json").write_bytes(b"{}")
            errori = gate.verifica(copie, contratti, PIN)
            self.assertTrue(any("intruso" in e for e in errori), errori)

    def test_un_originale_mancante_e_rosso(self) -> None:
        with tempfile.TemporaryDirectory() as t:
            copie, contratti = prepara(pathlib.Path(t), b"{}\n", b"{}\n")
            (contratti / "schemas" / "x.json").unlink()
            errori = gate.verifica(copie, contratti, PIN)
            self.assertTrue(any("non esiste" in e for e in errori), errori)

    def test_le_copie_reali_combaciano_col_checkout(self) -> None:
        contratti = gate.RADICE / ".plenora-contracts"
        if not contratti.is_dir():
            self.skipTest("il checkout fissato esiste solo nel job profilo-pubblico")
        pin = json.loads(gate.PIN.read_text(encoding="utf-8"))["contracts_source"]["revision"]
        self.assertEqual(gate.verifica(gate.COPIE, contratti, pin), [])


if __name__ == "__main__":
    unittest.main()
