"""Gli avvisi di licenza crate per crate: dal pacchetto, dal registro, o un errore."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import pathlib
import sys
import tempfile
import unittest
from unittest import mock

_QUI_SCRIPTS = str(pathlib.Path(__file__).resolve().parent)
if _QUI_SCRIPTS not in sys.path:
    # i gate caricati via spec importano strict_json dalla directory degli script
    sys.path.insert(0, _QUI_SCRIPTS)

QUI = pathlib.Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("avvisi_crate", QUI / "avvisi_crate.py")
avvisi_crate = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(avvisi_crate)

MIT = b"MIT License\r\n\r\nCopyright (c) 2020 Qualcuno\r\n"


def impronta(dati: bytes) -> str:
    return hashlib.sha256(dati.replace(b"\r\n", b"\n")).hexdigest()


class GliAvvisi(unittest.TestCase):
    def setUp(self) -> None:
        self.radice = pathlib.Path(tempfile.mkdtemp())
        self.testi = self.radice / "licenze-crate"
        self.testi.mkdir()
        self.registro = self.radice / "avvisi-crate.json"
        self.scrivi_registro([], [])
        for nome, valore in (("REGISTRO", self.registro), ("TESTI", self.testi)):
            patch = mock.patch.object(avvisi_crate, nome, valore)
            patch.start()
            self.addCleanup(patch.stop)

    def scrivi_registro(self, testi: list, senza: list) -> None:
        self.registro.write_text(
            json.dumps({"testi": testi, "senza_testo_upstream": senza}), encoding="utf-8"
        )

    def crate(self, nome: str, file: dict[str, bytes], licenza: str = "MIT", **extra) -> dict:
        cartella = self.radice / "src" / f"{nome}-1.0.0"
        cartella.mkdir(parents=True)
        (cartella / "Cargo.toml").write_text("[package]\n", encoding="utf-8")
        for nome_file, dati in file.items():
            (cartella / nome_file).write_bytes(dati)
        return {"nome": nome, "versione": "1.0.0", "licenza": licenza, "nostro": False,
                "cartella": str(cartella), "licenza_file": "", **extra}

    def test_i_file_del_pacchetto_si_copiano_tutti_notice_compreso(self) -> None:
        voce = self.crate("a", {"LICENSE-MIT": MIT, "NOTICE": b"n", "README.md": b"r"},
                          "MIT OR Apache-2.0")
        uscita = self.radice / "uscita"
        risultato, errori, _ = avvisi_crate.avvisi([voce], uscita)
        self.assertEqual(errori, [])
        self.assertEqual([a["file"] for a in risultato[0]["avvisi"]], ["LICENSE-MIT", "NOTICE"])
        self.assertEqual((uscita / "a-1.0.0" / "LICENSE-MIT").read_bytes(), MIT)
        self.assertFalse((uscita / "a-1.0.0" / "README.md").exists())

    def test_un_crate_senza_avviso_e_un_errore(self) -> None:
        """Prima: il solo testo per identificatore, e nessun gate se ne accorgeva."""
        _, errori, _ = avvisi_crate.avvisi([self.crate("b", {})])
        self.assertEqual(len(errori), 1)
        self.assertIn("nessun avviso", errori[0])

    def test_un_crate_senza_avviso_nel_pacchetto_usa_il_registro_verificato(self) -> None:
        (self.testi / "c-1.0.0").mkdir()
        (self.testi / "c-1.0.0" / "LICENSE").write_bytes(MIT)
        self.scrivi_registro([{"crate": "c", "versione": "1.0.0", "file": "LICENSE",
                               "sha256": impronta(MIT), "origine": "https://x"}], [])
        risultato, errori, usate = avvisi_crate.avvisi([self.crate("c", {})])
        self.assertEqual(errori, [])
        self.assertEqual(risultato[0]["avvisi"][0]["fonte"], "https://x")
        self.assertEqual(usate, {("c", "1.0.0")})

    def test_un_testo_del_registro_cambiato_e_un_errore(self) -> None:
        (self.testi / "c-1.0.0").mkdir()
        (self.testi / "c-1.0.0" / "LICENSE").write_bytes(MIT + b"x")
        self.scrivi_registro([{"crate": "c", "versione": "1.0.0", "file": "LICENSE",
                               "sha256": impronta(MIT), "origine": "https://x"}], [])
        _, errori, _ = avvisi_crate.avvisi([self.crate("c", {})])
        self.assertTrue(any("impronta" in e for e in errori))

    def test_una_voce_del_registro_per_un_pacchetto_che_ha_i_suoi_testi_e_obsoleta(self) -> None:
        self.scrivi_registro([{"crate": "d", "versione": "1.0.0", "file": "LICENSE",
                               "sha256": impronta(MIT), "origine": "https://x"}], [])
        _, errori, _ = avvisi_crate.avvisi([self.crate("d", {"LICENSE": MIT})])
        self.assertTrue(any("obsoleta" in e for e in errori))

    def test_senza_testo_upstream_porta_un_avviso_generato(self) -> None:
        self.scrivi_registro([], [{"crate": "e", "versione": "1.0.0", "licenza": "MIT",
                                   "autori": "Qualcuno", "perche": "nessun testo"}])
        uscita = self.radice / "uscita"
        risultato, errori, _ = avvisi_crate.avvisi([self.crate("e", {})], uscita)
        self.assertEqual(errori, [])
        generato = (uscita / "e-1.0.0" / avvisi_crate.AVVISO_GENERATO).read_text(encoding="utf-8")
        self.assertIn("Qualcuno", generato)
        self.assertIn("MIT", generato)

    def test_i_nostri_crate_non_portano_avvisi(self) -> None:
        voce = self.crate("nostro", {})
        voce["nostro"] = True
        risultato, errori, _ = avvisi_crate.avvisi([voce])
        self.assertEqual((risultato, errori), ([], []))


class LeVociDiCrateJson(unittest.TestCase):
    """`pacchetti_con_avvisi`, cio' che i costruttori scrivono nell'artefatto."""

    def setUp(self) -> None:
        GliAvvisi.setUp(self)

    scrivi_registro = GliAvvisi.scrivi_registro
    crate = GliAvvisi.crate

    def test_le_voci_portano_gli_avvisi_e_non_la_cartella(self) -> None:
        voce = self.crate("a", {"LICENSE": MIT})
        uscita = self.radice / "uscita"
        voci = avvisi_crate.pacchetti_con_avvisi([voce], uscita)
        self.assertNotIn("cartella", voci[0])
        self.assertEqual(voci[0]["avvisi"], [{"file": "LICENSE", "sha256": impronta(MIT), "fonte": "pacchetto"}])
        self.assertTrue((uscita / "a-1.0.0" / "LICENSE").is_file())

    def test_un_crate_senza_avviso_ferma_la_costruzione(self) -> None:
        with self.assertRaises(SystemExit):
            avvisi_crate.pacchetti_con_avvisi([self.crate("b", {})], self.radice / "uscita")


class IlRegistroVero(unittest.TestCase):
    def test_ogni_testo_versionato_ha_l_impronta_registrata(self) -> None:
        dati = avvisi_crate.registro()
        for voce in dati["testi"]:
            with self.subTest(crate=f"{voce['crate']}-{voce['versione']}/{voce['file']}"):
                testo = avvisi_crate.TESTI / f"{voce['crate']}-{voce['versione']}" / voce["file"]
                self.assertEqual(impronta(testo.read_bytes()), voce["sha256"])
                dati_grezzi = testo.read_bytes()
                blob = hashlib.sha1(b"blob %d\0" % len(dati_grezzi) + dati_grezzi).hexdigest()
                self.assertEqual(blob, voce["blob_git"])


if __name__ == "__main__":
    unittest.main()
