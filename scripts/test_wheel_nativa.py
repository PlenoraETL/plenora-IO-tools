"""La verifica della wheel rifiuta omissioni e alterazioni delle licenze."""
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import unittest
from unittest.mock import patch
import zipfile
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))

spec = importlib.util.spec_from_file_location("wheel_nativa", Path(__file__).with_name("prepara-wheel-nativa.py"))
wheel = importlib.util.module_from_spec(spec)
spec.loader.exec_module(wheel)


class LicenzeDellaWheel(unittest.TestCase):
    def test_omissioni_e_alterazioni_non_passano(self):
        avviso = b"Copyright autore\nMIT License\n"
        crate = [{"nome": "pyo3", "versione": "1", "avvisi": [
            {"file": "LICENSE", "sha256": hashlib.sha256(avviso).hexdigest()}]}]
        prefisso = "plenora_io/LICENSES/crate-rust/"
        completo = {"plenora_io/LICENSE": (wheel.RADICE / "LICENSE").read_bytes(),
                    prefisso + "CRATE.json": json.dumps({"pacchetti": crate}).encode(),
                    prefisso + "pyo3-1/LICENSE": avviso}
        casi = [("completo", completo, False)]
        for nome in completo:
            casi.append(("manca " + nome, {k: v for k, v in completo.items() if k != nome}, True))
            casi.append(("alterato " + nome, {**completo, nome: b"{}" if nome.endswith('.json') else b"alterato"}, True))
        casi.append(("elenco vuoto", {**completo, prefisso + "CRATE.json": b'{"pacchetti": []}'}, True))
        with patch.object(wheel.avvisi_crate, "_dipendenze_rust"), \
             patch.object(wheel.avvisi_crate, "avvisi", return_value=(crate, [], set())):
            for nome, contenuto, fallisce in casi:
                with self.subTest(caso=nome):
                    dati = io.BytesIO()
                    with zipfile.ZipFile(dati, 'w') as z:
                        for file, testo in contenuto.items():
                            z.writestr(file, testo)
                    with zipfile.ZipFile(dati) as z:
                        self.assertEqual(bool(wheel.verifica_licenze(z)), fallisce)
