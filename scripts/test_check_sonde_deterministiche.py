"""Sonde del gate delle sonde deterministiche.

Il gate tiene in piedi la correzione di `copertura.variazione-fra-corse`. Se
sbagliasse direbbe «le sonde ci sono» su un albero da cui sono state tolte, e
la copertura tornerebbe a cambiare fra due corse senza che nulla lo dica.

Le sonde del ramo `esegui` non lanciano `cargo`: quella parte e' provata
alimentando il lettore con uscite di harness costruite apposta, che e' dove il
gate puo' sbagliare in silenzio — un test assente, `#[ignore]`, fallito od
omonimo letti tutti come verdi. Lanciare la suite vera aggiungerebbe minuti
senza provare nulla di piu': il gate la esegue davvero quando gira.
"""

from __future__ import annotations

import json
import unittest
from unittest import mock

from scripts import check_assurance_n1_prove as n1
from scripts import check_sonde_deterministiche as gate


def gruppo(**extra):
    base = {
        "ramo": "un ramo che dipendeva dallo scheduling",
        "perche_dipendeva_dallo_scheduling": "perche' perdeva una corsa",
        "seam": "una seam test-only",
        "crate": "plenora-io-model",
        "configurazione": "default",
        "bersaglio": "lib",
        "test": ["modulo::tests::una_sonda"],
        "che_cosa_provano": "una proprieta', non una riga",
    }
    base.update(extra)
    return base


def uscita(*righe: str) -> str:
    return "\n".join(f"test {r}" for r in righe) + "\n"


class SondeStruttura(unittest.TestCase):
    def test_un_gruppo_completo_passa(self) -> None:
        """La controprova positiva: senza, «sempre rosso» sarebbe una difesa."""
        self.assertEqual(gate.struttura([gruppo()]), [])

    def test_un_ramo_senza_sonda_e_rosso(self) -> None:
        """Il caso per cui il registro esiste: il ramo torna alla fortuna."""
        errori = gate.struttura([gruppo(test=[])])
        self.assertTrue(any("nessuna sonda" in e for e in errori), errori)

    def test_una_ragione_vuota_e_rossa(self) -> None:
        """Una sonda senza la ragione per cui esiste e' una sonda che qualcuno
        togliera'."""
        for campo in ("perche_dipendeva_dallo_scheduling", "seam", "che_cosa_provano"):
            with self.subTest(campo=campo):
                errori = gate.struttura([gruppo(**{campo: ""})])
                self.assertTrue(any(campo in e for e in errori), errori)

    def test_campi_mancanti_sono_rossi(self) -> None:
        parziale = gruppo()
        del parziale["seam"]
        errori = gate.struttura([parziale])
        self.assertTrue(any("campi mancanti" in e for e in errori), errori)

    def test_un_ramo_duplicato_e_rosso(self) -> None:
        errori = gate.struttura([gruppo(), gruppo()])
        self.assertTrue(any("duplicata" in e for e in errori), errori)

    def test_una_configurazione_o_un_bersaglio_inventati_sono_rossi(self) -> None:
        for chiave, valore in (("configurazione", "quasi-tutte"), ("bersaglio", "esempi")):
            with self.subTest(chiave=chiave):
                errori = gate.struttura([gruppo(**{chiave: valore})])
                self.assertTrue(any(chiave in e for e in errori), errori)

    def test_identificatori_ripetuti_sono_rossi(self) -> None:
        errori = gate.struttura([gruppo(test=["m::t", "m::t"])])
        self.assertTrue(any("ripetuti" in e for e in errori), errori)


class SondeEsecuzione(unittest.TestCase):
    def esegui_con(self, testo: str, *gruppi, uscita_del_processo: int = 0):
        """Alimenta il **runner condiviso** con un'uscita di harness costruita.

        Il runner vive in `check_assurance_n1_prove` e lo usano tre gate: e' li'
        che le sue sonde stanno, comprese quelle sull'exit code. Qui si prova
        cio' che questo gate ne fa.
        """
        finto = mock.Mock(returncode=uscita_del_processo, stdout=testo, stderr="")
        with mock.patch.object(n1.subprocess, "run", return_value=finto) as corsa:
            return gate.esegui(list(gruppi) or [gruppo()]), corsa

    def test_una_sonda_eseguita_e_passata_va_bene(self) -> None:
        errori, _ = self.esegui_con(uscita("modulo::tests::una_sonda ... ok"))
        self.assertEqual(errori, [], errori)

    def test_una_sonda_assente_e_rossa(self) -> None:
        """Il caso che questo gate esiste per cogliere: la sonda e' stata
        tolta, e con essa la riproducibilita' della misura."""
        errori, _ = self.esegui_con(uscita("modulo::tests::un_altro ... ok"))
        self.assertTrue(any("torna a dipendere dallo scheduling" in e for e in errori), errori)

    def test_una_sonda_ignorata_e_rossa(self) -> None:
        errori, _ = self.esegui_con(uscita("modulo::tests::una_sonda ... ignored"))
        self.assertTrue(any("`#[ignore]`" in e for e in errori), errori)

    def test_una_sonda_fallita_e_rossa(self) -> None:
        errori, _ = self.esegui_con(uscita("modulo::tests::una_sonda ... FAILED"))
        self.assertTrue(any("non passa" in e for e in errori), errori)

    def test_un_elenco_vuoto_non_e_un_verde(self) -> None:
        errori, _ = self.esegui_con("")
        self.assertTrue(any("silenzio non va letto come un verde" in e for e in errori), errori)

    def test_un_harness_che_fallisce_e_rosso(self) -> None:
        """Le sonde stampate `ok` non certificano una corsa che esce con 17."""
        errori, _ = self.esegui_con(
            uscita("modulo::tests::una_sonda ... ok"), uscita_del_processo=17
        )
        self.assertTrue(any("esce con 17" in e for e in errori), errori)

    def test_una_terna_si_misura_una_volta_sola(self) -> None:
        """Ripetere la misura non la rende piu' vera, e allunga il gate."""
        _, corsa = self.esegui_con(
            uscita("modulo::tests::una_sonda ... ok", "modulo::tests::altra ... ok"),
            gruppo(),
            gruppo(ramo="un altro ramo", test=["modulo::tests::altra"]),
        )
        self.assertEqual(corsa.call_count, 1)

    def test_due_crate_si_misurano_separatamente(self) -> None:
        _, corsa = self.esegui_con(
            uscita("modulo::tests::una_sonda ... ok"),
            gruppo(),
            gruppo(ramo="un ramo altrove", crate="plenora-io-core"),
        )
        self.assertEqual(corsa.call_count, 2)


class SondeRegistroReale(unittest.TestCase):
    def registro(self) -> list[dict]:
        return json.loads(gate.REGISTRO.read_text(encoding="utf-8"))["gruppi"]

    def test_il_registro_reale_e_ben_formato(self) -> None:
        self.assertEqual(gate.struttura(self.registro()), [])

    def test_ogni_ramo_dichiara_almeno_una_sonda(self) -> None:
        senza = [v["ramo"] for v in self.registro() if not v["test"]]
        self.assertEqual(senza, [])


if __name__ == "__main__":
    unittest.main()
