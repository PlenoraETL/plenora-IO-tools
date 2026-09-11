#!/usr/bin/env python3
"""Sonde del gate sul dettaglio congelato.

Un gate che cerca l'**assenza** di qualcosa e' verde anche quando non guarda
niente: un percorso sbagliato, una regex che non aggancia piu', un soggetto
rinominato. Ogni sonda qui sotto ha la propria controprova.
"""

from __future__ import annotations

import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import check_dettaglio_congelato as gate  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parents[1]

STRUTTURA_SANA = """
/// La valutazione, senza `Serialize` per scelta.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FidelityAssessment {
    prime_v1: Vec<String>,
}

impl FidelityReason {
    pub fn detail_v1(&self) -> &str {
        self.dettaglio_v1.as_deref().unwrap_or(&self.detail)
    }
}
"""


class AlberoFinto:
    """Un albero minimo con la casa del dato e un altro modulo."""

    def __init__(self, radice: pathlib.Path) -> None:
        self.radice = radice
        (radice / gate.CASA).parent.mkdir(parents=True, exist_ok=True)
        (radice / "crates" / "plenora-io-cli" / "src").mkdir(parents=True, exist_ok=True)
        self.scrivi_casa(STRUTTURA_SANA)
        self.scrivi_altrove("pub fn niente() {}\n")

    def scrivi_casa(self, testo: str) -> None:
        (self.radice / gate.CASA).write_text(testo, encoding="utf-8")

    def scrivi_altrove(self, testo: str) -> None:
        (self.radice / "crates" / "plenora-io-cli" / "src" / "busta.rs").write_text(
            testo, encoding="utf-8"
        )


class SondeDelDerive(unittest.TestCase):
    def albero(self, temporanea: str) -> AlberoFinto:
        return AlberoFinto(pathlib.Path(temporanea))

    def test_senza_serialize_passa(self) -> None:
        """La controprova positiva: senza, «sempre rosso» sarebbe una difesa."""
        with tempfile.TemporaryDirectory() as temporanea:
            albero = self.albero(temporanea)
            self.assertEqual(gate.il_derive_non_e_tornato(albero.radice), [])

    def test_col_serialize_e_rosso(self) -> None:
        with tempfile.TemporaryDirectory() as temporanea:
            albero = self.albero(temporanea)
            albero.scrivi_casa(
                STRUTTURA_SANA.replace(
                    "#[derive(Clone, Debug, PartialEq, Eq)]",
                    "#[derive(Clone, Debug, PartialEq, Eq, Serialize)]",
                )
            )
            errori = gate.il_derive_non_e_tornato(albero.radice)
            self.assertTrue(any("Serialize" in e for e in errori), errori)

    def test_la_struttura_rinominata_e_rossa(self) -> None:
        """Un gate che non trova il soggetto non lo protegge, e deve dirlo.

        E' il modo in cui una difesa muore senza rumore: il nome cambia, la
        regex non aggancia piu', e il gate resta verde su niente.
        """
        with tempfile.TemporaryDirectory() as temporanea:
            albero = self.albero(temporanea)
            albero.scrivi_casa(
                STRUTTURA_SANA.replace("FidelityAssessment", "ValutazioneDiFedelta")
            )
            errori = gate.il_derive_non_e_tornato(albero.radice)
            self.assertTrue(any("non si trova" in e for e in errori), errori)

    def test_il_file_assente_e_rosso(self) -> None:
        with tempfile.TemporaryDirectory() as temporanea:
            errori = gate.il_derive_non_e_tornato(pathlib.Path(temporanea))
            self.assertTrue(any("assente" in e for e in errori), errori)


class SondeDellAccessore(unittest.TestCase):
    def albero(self, temporanea: str) -> AlberoFinto:
        return AlberoFinto(pathlib.Path(temporanea))

    def test_nessun_altro_lo_legge_passa(self) -> None:
        with tempfile.TemporaryDirectory() as temporanea:
            albero = self.albero(temporanea)
            self.assertEqual(gate.il_dettaglio_resta_interno(albero.radice), [])

    def test_una_chiamata_da_un_altro_modulo_e_rossa(self) -> None:
        with tempfile.TemporaryDirectory() as temporanea:
            albero = self.albero(temporanea)
            albero.scrivi_altrove("pub fn x(r: &R) -> &str { r.detail_v1() }\n")
            errori = gate.il_dettaglio_resta_interno(albero.radice)
            self.assertTrue(any("busta.rs" in e for e in errori), errori)

    def test_una_chiamata_in_un_modulo_di_prova_non_conta(self) -> None:
        """Le sonde della redazione **devono** poterlo leggere.

        E' cio' su cui verificano che quei nomi non escano: contarle renderebbe
        impossibile scrivere la prova che il divieto serve.
        """
        with tempfile.TemporaryDirectory() as temporanea:
            albero = self.albero(temporanea)
            albero.scrivi_altrove(
                "pub fn x() {}\n"
                "#[cfg(test)]\n"
                "mod sonde {\n"
                "    fn y(r: &R) -> &str { r.detail_v1() }\n"
                "}\n"
            )
            self.assertEqual(gate.il_dettaglio_resta_interno(albero.radice), [])

    def test_un_commento_che_lo_nomina_non_conta(self) -> None:
        """Spiegare la regola non e' violarla."""
        with tempfile.TemporaryDirectory() as temporanea:
            albero = self.albero(temporanea)
            albero.scrivi_altrove(
                "/// Non chiama `detail_v1()`, e questa riga spiega perche'.\n"
                "pub fn x() {}\n"
            )
            self.assertEqual(gate.il_dettaglio_resta_interno(albero.radice), [])

    def test_un_albero_vuoto_e_rosso(self) -> None:
        """Verde per assenza di domanda: la forma che questo gate deve evitare."""
        with tempfile.TemporaryDirectory() as temporanea:
            radice = pathlib.Path(temporanea)
            (radice / "crates").mkdir()
            errori = gate.il_dettaglio_resta_interno(radice)
            self.assertTrue(any("assenza di domanda" in e for e in errori), errori)


class SondeSullAlberoVero(unittest.TestCase):
    """Le due controprove sul repository, che sono il punto del gate."""

    def test_il_derive_non_c_e(self) -> None:
        self.assertEqual(gate.il_derive_non_e_tornato(ROOT), [])

    def test_l_accessore_e_interno(self) -> None:
        self.assertEqual(gate.il_dettaglio_resta_interno(ROOT), [])

    def test_il_dato_esiste_ancora(self) -> None:
        """Il gate ha un soggetto: quando non l'avra' piu', va rimosso.

        `dettaglio_v1` e `prime_v1` sono l'eredita' del protocollo congelato, e
        finche' ci sono il pericolo c'e'. Il giorno in cui saranno tolti, questa
        sonda diventera' rossa ed e' il segnale che anche il gate ha finito.
        """
        sorgente = (ROOT / gate.CASA).read_text(encoding="utf-8")
        self.assertIn("dettaglio_v1", sorgente)
        self.assertIn("prime_v1", sorgente)


if __name__ == "__main__":
    unittest.main()
