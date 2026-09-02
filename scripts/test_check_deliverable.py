"""Sonde sul controllo post-upload dei deliverable."""

from __future__ import annotations

import importlib.util
import json
import pathlib
import shutil
import sys
import tempfile
import unittest


RADICE = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(RADICE / "scripts"))


def carica(percorso: pathlib.Path):
    spec = importlib.util.spec_from_file_location("check_deliverable", percorso)
    modulo = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(modulo)
    return modulo


class SondeDeliverable(unittest.TestCase):
    VERSIONE = "2.0.0"
    REVISIONE = "a" * 40

    def setUp(self) -> None:
        self.gate = carica(RADICE / "scripts" / "check-deliverable.py")
        self.d = __import__("distribuzione")
        self.tmp = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        self.crea_serie()

    def crea_serie(self) -> None:
        for piattaforma in ("linux-x86_64", "windows-x86_64"):
            for profilo in ("base", "filegdb"):
                nome = self.d.nome_archivio(self.VERSIONE, piattaforma, profilo)
                archivio = self.tmp / f"{nome}.{self.d.contenitore(piattaforma)}"
                archivio.write_bytes(f"{piattaforma}/{profilo}".encode())
                digesto = self.d.sha256(archivio)
                (self.tmp / f"{archivio.name}.sha256").write_text(
                    f"{digesto}  {archivio.name}\n", encoding="utf-8"
                )
                (self.tmp / f"{archivio.name}.provenance.json").write_text(
                    json.dumps(
                        {
                            "artefatto": archivio.name,
                            "sha256": digesto,
                            "dimensione": archivio.stat().st_size,
                            "piattaforma": piattaforma,
                            "profilo": profilo,
                            "canale": "prova",
                            "non_release": True,
                            "revisione": self.REVISIONE,
                            "lock": self.d.sha256(self.gate.LOCK[piattaforma]),
                        }
                    ),
                    encoding="utf-8",
                )

    def errori(self) -> list[str]:
        return self.gate.verifica(self.tmp, self.VERSIONE, "prova", self.REVISIONE)

    def test_la_serie_completa_passa(self) -> None:
        self.assertEqual(self.errori(), [])

    def test_un_archivio_cambiato_dopo_l_upload_e_rosso(self) -> None:
        archivio = next(self.tmp.glob("*.tar.gz"))
        archivio.write_bytes(archivio.read_bytes() + b"manomesso")
        self.assertIn("diverso dai byte scaricati", " ".join(self.errori()))

    def test_la_provenance_deve_nominare_quegli_stessi_byte(self) -> None:
        percorso = next(self.tmp.glob("*.provenance.json"))
        prova = json.loads(percorso.read_text(encoding="utf-8"))
        prova["sha256"] = "0" * 64
        percorso.write_text(json.dumps(prova), encoding="utf-8")
        self.assertIn("provenance.sha256", " ".join(self.errori()))

    def test_la_revisione_del_deliverable_non_e_libera(self) -> None:
        percorso = next(self.tmp.glob("*.provenance.json"))
        prova = json.loads(percorso.read_text(encoding="utf-8"))
        prova["revisione"] = "b" * 40
        percorso.write_text(json.dumps(prova), encoding="utf-8")
        self.assertIn("provenance.revisione", " ".join(self.errori()))

    def test_un_sidecar_mancante_e_rosso(self) -> None:
        next(self.tmp.glob("*.sha256")).unlink()
        self.assertIn("file mancanti", " ".join(self.errori()))

    def test_un_file_extra_non_si_nasconde_nell_artifact(self) -> None:
        (self.tmp / "non-dichiarato.txt").write_text("x", encoding="utf-8")
        self.assertIn("file non dichiarati", " ".join(self.errori()))


if __name__ == "__main__":
    unittest.main()
