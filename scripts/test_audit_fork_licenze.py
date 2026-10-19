"""Le licenze dei fork: l'audit fallisce se una licenza dichiarata manca o e' cambiata.

Il fork di gdal ha ridistribuito codice MIT senza `LICENSE.txt` dalla prima
vendorizzazione fino alla 4.1.1, con il registro che lo dichiarava: l'audit lo
riportava, ma nessuno lo leggeva, e nessun gate lo verificava. Queste prove
fissano i quattro casi su un fork finto, senza rete e senza toccare `vendor/`.
"""

from __future__ import annotations

try:
    from . import strict_json
except ImportError:
    import strict_json

import hashlib
import importlib.util
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

QUI = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(QUI))
_spec = importlib.util.spec_from_file_location("audit_fork", QUI / "audit-fork.py")
audit = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(audit)

TESTO = b"The MIT License (MIT)\r\n\r\nCopyright (c) 2014 Qualcuno\r\n"


def impronta(dati: bytes) -> str:
    return hashlib.sha256(dati.replace(b"\r\n", b"\n")).hexdigest()


class UnFork(unittest.TestCase):
    """Un repository git temporaneo con un fork di un file e la sua licenza."""

    def setUp(self) -> None:
        self.radice = pathlib.Path(tempfile.mkdtemp())
        self.vendor = self.radice / "vendor" / "finto"
        self.vendor.mkdir(parents=True)
        (self.vendor / "lib.rs").write_bytes(b"pub fn f() {}\n")
        (self.vendor / "LICENSE.txt").write_bytes(TESTO)
        for comando in (["init", "-q"], ["add", "-A"]):
            subprocess.run(["git", *comando], cwd=self.radice, check=True)
        # `insieme_versionato` chiede a git sotto la radice del repository:
        # qui la radice e' quella del repository finto.
        radice = mock.patch.object(audit.fork_comune, "ROOT", self.radice)
        radice.start()
        self.addCleanup(radice.stop)
        self.registro = {
            "file_licenza": "LICENSE.txt",
            "licenze": [{"file": "LICENSE.txt", "sha256": impronta(TESTO)}],
        }

    def rilievi(self) -> list[str]:
        return audit.rilievi_di_licenza("finto", self.registro, self.vendor)

    def test_una_licenza_presente_versionata_e_identica_non_e_un_rilievo(self) -> None:
        self.assertEqual(self.rilievi(), [])

    def test_i_fine_riga_non_contano(self) -> None:
        (self.vendor / "LICENSE.txt").write_bytes(TESTO.replace(b"\r\n", b"\n"))
        subprocess.run(["git", "add", "-A"], cwd=self.radice, check=True)
        self.assertEqual(self.rilievi(), [])

    def test_una_licenza_dichiarata_e_assente_e_un_rilievo(self) -> None:
        (self.vendor / "LICENSE.txt").unlink()
        subprocess.run(["git", "add", "-A"], cwd=self.radice, check=True)
        rilievi = self.rilievi()
        self.assertEqual(len(rilievi), 1)
        self.assertIn("senza la sua licenza", rilievi[0])

    def test_una_licenza_non_versionata_e_un_rilievo(self) -> None:
        subprocess.run(["git", "rm", "-q", "--cached", "vendor/finto/LICENSE.txt"],
                       cwd=self.radice, check=True)
        self.assertTrue(any("non e' versionato" in r for r in self.rilievi()))

    def test_un_testo_cambiato_e_un_rilievo(self) -> None:
        (self.vendor / "LICENSE.txt").write_bytes(TESTO + b"x")
        self.assertTrue(any("non e' quello registrato" in r for r in self.rilievi()))

    def test_una_licenza_dichiarata_senza_impronta_e_un_rilievo(self) -> None:
        self.registro["file_notice"] = "NOTICE.txt"
        self.assertTrue(any("`file_notice`" in r for r in self.rilievi()))
        del self.registro["file_notice"]
        self.registro["licenze"] = []
        self.assertTrue(any("non elenca `licenze`" in r for r in self.rilievi()))


class IRegistriVeri(unittest.TestCase):
    def test_i_quattro_fork_del_repository_non_hanno_rilievi_di_licenza(self) -> None:

        radice = QUI.parent
        for nome in audit.FORK:
            lock = strict_json.loads((radice / "scripts" / f"{nome}-fork-lock.json").read_text(encoding="utf-8"))
            registro = strict_json.loads(
                (radice / "assurance" / "registries" / f"vendor-{nome}-fork.json").read_text(
                    encoding="utf-8"
                )
            )
            with self.subTest(fork=nome):
                self.assertEqual(
                    audit.rilievi_di_licenza(nome, registro, radice / lock["vendor_path"]), []
                )


if __name__ == "__main__":
    unittest.main()
