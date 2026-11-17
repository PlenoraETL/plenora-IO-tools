"""Sonde della forma <superficie>:<percorso>:<verifiche> di costruisci-manifesto-adozione."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

_SPEC = importlib.util.spec_from_file_location(
    "costruisci_manifesto_adozione", Path(__file__).with_name("costruisci-manifesto-adozione.py"))
costruisci = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(costruisci)


class FormaDellaSpecifica(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.file = Path(self.temp.name) / "plenora-io-9.0.0-linux-x86_64-base.tar.gz"
        self.file.write_bytes(b"artefatto")

    def tearDown(self):
        self.temp.cleanup()

    def verifica(self, percorso):
        voce = costruisci.artefatto(f"cli:{percorso}:python3 scripts/a.py;python3 scripts/b.py", "9.0.0")
        self.assertEqual(voce["name"], self.file.name)
        self.assertEqual(voce["surface"], "cli")
        self.assertEqual(voce["verification"], ["python3 scripts/a.py", "python3 scripts/b.py"])

    def test_percorso_posix(self):
        self.verifica(self.file.as_posix())

    def test_percorso_windows_con_lettera_di_unita(self):
        """Il percorso assoluto Windows porta un ':' che non separa i campi."""
        percorso = str(self.file)
        if len(percorso) > 1 and percorso[1] == ":":
            self.verifica(percorso)
            self.verifica(percorso.replace("\\", "/"))
        else:
            self.verifica(self.file.as_posix())

    def test_forma_incompleta_rifiutata(self):
        with self.assertRaises(SystemExit):
            costruisci.artefatto(f"cli:{self.file.as_posix()}", "9.0.0")

    def test_superficie_sconosciuta_rifiutata(self):
        with self.assertRaises(SystemExit):
            costruisci.artefatto(f"altro:{self.file.as_posix()}:verifica", "9.0.0")


if __name__ == "__main__":
    unittest.main()
