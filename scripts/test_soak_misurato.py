"""Regressioni della misura del soak, senza campagne lunghe o attese reali."""

import hashlib
import json
import pathlib
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from scripts import soak_misurato as soak

FIXTURE = pathlib.Path(__file__).parent / "fixtures" / "soak"
ANNUNCIO = "=== dxf_reader: 3600s ===\n"
AVVIO = "#2 INITED cov: 123 ft: 234 corp: 1/1b exec/s: 0 rss: 20Mb\n"
FINE = "Done 780540 runs in 3630 second(s)\n"
# libFuzzer stampa letteralmente `second(s)`, non una flessione inglese.
LOG = ANNUNCIO + AVVIO + FINE
TEMPI = {"monotonic_s": 3644.7, "boottime_s": 3644.7,
         "parete_s": 3644.6, "cpu_figli_s": 280.3}


class SondeGiudizio(unittest.TestCase):
    def giudica(self, log=LOG, codice=0, **tempi):
        return soak.giudica("dxf_reader", 3600, codice, TEMPI | tempi, log)

    def test_cpu_bassa_conserva_originale_e_giudizio_corretto(self):
        originale = json.loads((FIXTURE / "referto-originale.json").read_text(encoding="utf-8"))
        corretto = json.loads((FIXTURE / "giudizio-corretto.json").read_text(encoding="utf-8"))
        for nome, digest in corretto["sha256_originali"].items():
            self.assertEqual(hashlib.sha256((FIXTURE / nome).read_bytes()).hexdigest(), digest)
        self.assertFalse(originale["durata_dimostrata"])
        # Il log completo storico non e' nella fixture: ricostruiamo SOLO il
        # riepilogo dai numeri del referto, senza presentarlo come log originale.
        riepilogo = originale["riepilogo_del_fuzzer"]
        log = ANNUNCIO + f"Done {riepilogo['runs']} runs in {riepilogo['secondi_dichiarati']} second(s)\n"
        nuovo = soak.giudica("dxf_reader", 3600, 0, originale["tempi_grezzi"], log)
        self.assertEqual(nuovo["durata_dimostrata"], corretto["giudizio_corretto_durata_dimostrata"])
        self.assertEqual(nuovo["esito"], "completa")
        self.assertAlmostEqual(nuovo["diagnostica_cpu"]["quota_cpu_intervallo"], 280.3 / 3644.7)

    def test_vm_congelata_sedici_ore_con_due_orologi_concordi(self):
        referto = self.giudica(parete_s=3644.7 + 16 * 3600)
        self.assertEqual(referto["derivati"]["boottime_meno_monotonic_s"], 0)
        self.assertFalse(referto["durata_dimostrata"])
        self.assertFalse(referto["finding"])
        self.assertEqual(referto["esito"], "misura_invalida")

    def test_rustup_assente_non_diventa_finding(self):
        referto = self.giudica("scripts/fuzz-smoke.sh: line 65: rustup: command not found\n", 1)
        self.assertFalse(referto["avvio_osservato"])
        self.assertIsNone(referto["finding"])
        self.assertFalse(referto["durata_dimostrata"])
        self.assertEqual(referto["esito"], "avvio_non_dimostrato")

    def test_crash_senza_done_e_un_finding_osservato(self):
        referto = self.giudica(ANNUNCIO + "==42== ERROR: libFuzzer: deadly signal\n", 1)
        self.assertTrue(referto["avvio_osservato"])
        self.assertTrue(referto["finding"])
        self.assertFalse(referto["durata_dimostrata"])
        self.assertEqual(referto["esito"], "finding")

    def test_exit_nonzero_dopo_avvio_non_basta_per_un_finding(self):
        for log in (ANNUNCIO + AVVIO, LOG):
            with self.subTest(log=log):
                referto = self.giudica(log, 137)
                self.assertEqual(referto["esito"], "interrotta")
                self.assertIsNone(referto["finding"])
                self.assertFalse(referto["durata_dimostrata"])

    def test_preparazione_lunga_non_completa_un_fuzzer_breve(self):
        referto = self.giudica(LOG.replace("3630", "30"))
        self.assertFalse(referto["durata_dimostrata"])

    def test_exit_zero_senza_riepilogo_non_completa(self):
        for log in ("", ANNUNCIO, ANNUNCIO + AVVIO, LOG + FINE):
            with self.subTest(log=log):
                self.assertFalse(self.giudica(log)["durata_dimostrata"])

    def test_orologi_assenti_non_finiti_o_incoerenti(self):
        for tempi in ({"parete_s": None}, {"boottime_s": float("nan")},
                      {"monotonic_s": -1}, {"parete_s": 3000},
                      {"boottime_s": 4500}, {"monotonic_s": 3599}):
            with self.subTest(tempi=tempi):
                self.assertFalse(self.giudica(**tempi)["durata_dimostrata"])

    def test_cpu_zero_non_invalida_la_durata(self):
        self.assertTrue(self.giudica(cpu_figli_s=0)["durata_dimostrata"])

    def test_log_di_un_altro_target_non_qualifica(self):
        self.assertFalse(self.giudica(LOG.replace("dxf_reader", "shp_reader"))["durata_dimostrata"])


class SondeRaccolta(unittest.TestCase):
    def test_raccoglie_exit_log_digest_e_orologi_senza_sovrascrivere(self):
        def figlio(comando, **kwargs):
            self.assertEqual(comando[-1], "dxf_reader")
            kwargs["stdout"].write(LOG.encode())
            return subprocess.CompletedProcess(comando, 137)

        prima = dict.fromkeys(soak.OROLOGI, 100.0)
        dopo = {k: prima[k] + v for k, v in TEMPI.items()}
        with (
            tempfile.TemporaryDirectory() as tmp,
            patch.object(soak, "campiona", side_effect=[prima, dopo, prima]),
            patch.object(soak.subprocess, "run", side_effect=figlio) as run,
        ):
            cartella = pathlib.Path(tmp) / "corsa"
            referto = soak.misura("dxf_reader", 3600, cartella)
            self.assertEqual(referto["codice_uscita_campagna"], 137)
            self.assertEqual(referto["log_sha256"], hashlib.sha256(LOG.encode()).hexdigest())
            scritto = (cartella / "referto.json").read_bytes()
            self.assertEqual(json.loads(scritto), referto)
            with self.assertRaises(FileExistsError):
                soak.misura("dxf_reader", 3600, cartella)
            self.assertEqual((cartella / "referto.json").read_bytes(), scritto)
            run.assert_called_once()

    def test_eseguibile_assente_lascia_un_referto_non_verde(self):
        with (
            tempfile.TemporaryDirectory() as tmp,
            patch.object(soak, "campiona", return_value=dict.fromkeys(soak.OROLOGI, 100.0)),
            patch.object(soak.subprocess, "run", side_effect=FileNotFoundError("bash assente")),
        ):
            referto = soak.misura("dxf_reader", 3600, pathlib.Path(tmp) / "corsa")
            self.assertIsNone(referto["codice_uscita_campagna"])
            self.assertEqual(referto["esito"], "avvio_non_dimostrato")
            self.assertIn("bash assente", referto["errore_avvio"])


if __name__ == "__main__":
    unittest.main()
