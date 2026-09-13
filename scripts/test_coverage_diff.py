"""Sonde del filtro di perimetro della diagnostica differenziale.

Il perimetro decide **di che cosa parla** un numero, e due numeri con lo stesso
nome su insiemi diversi sono peggio di un numero solo. Qui si prova la sola
funzione che lo decide: `rilevante`.

# Perche' esiste una vista dedicata

Nasceva come **complemento di un'esclusione**: `plenora-io-tools` stava fuori
dallo scope «library coverage» perche' il pacchetto conteneva un binario, e
«fuori dalla soglia» era diventato «fuori da ogni misura».

Dalla 4.0.0 quel pacchetto porta anche la superficie Rust pubblica, ed e' una
libreria come le altre: entra nel denominatore della soglia, e l'esclusione non
c'e' piu'. La vista dedicata resta, ma cambia natura -- non e' piu' un
complemento disgiunto, e' **lo stesso profdata letto con un filtro**, per dire
quanto di quel pacchetto sia coperto senza che il numero si diluisca nei
quattordici crate.

La differenza non e' di parole: prima le due misure non potevano sovrapporsi, e
una sonda lo pretendeva. Ora si sovrappongono per costruzione, e pretendere il
contrario proverebbe un perimetro che la corsa non usa.
"""

from __future__ import annotations

import unittest

from scripts import coverage_diff as strumento


class SondePerimetro(unittest.TestCase):
    LIBRERIA = "crates/driver-shp/src/lib.rs"
    PUBBLICO = "crates/plenora-io-tools/src/lib.rs"
    BINARIO = "crates/plenora-io-tools/src/main.rs"
    BENCH = "crates/plenora-bench/src/main.rs"
    FUZZ = "crates/plenora-fuzz/src/lib.rs"

    def test_il_perimetro_di_libreria_tiene_fuori_l_attrezzaggio(self) -> None:
        """Dentro le librerie, fuori gli strumenti di misura.

        `plenora-io-tools` e' **dentro**: porta la superficie Rust pubblica, e
        tenerlo fuori toglierebbe dal denominatore proprio il codice che la
        soglia deve sorvegliare.
        """
        for percorso in (self.LIBRERIA, self.PUBBLICO, self.BINARIO):
            with self.subTest(percorso=percorso):
                self.assertTrue(strumento.rilevante(percorso))
        for percorso in (self.BENCH, self.FUZZ):
            with self.subTest(percorso=percorso):
                self.assertFalse(strumento.rilevante(percorso))

    def test_il_perimetro_dedicato_tiene_dentro_solo_la_crate_scelta(self) -> None:
        """La proprieta' che rende la vista dedicata una vista **su** qualcosa.

        Se il filtro lasciasse passare anche le altre librerie, il numero
        sarebbe quello di libreria con un altro nome, e sembrerebbe una
        conferma invece che un dettaglio.
        """
        for percorso in (self.PUBBLICO, self.BINARIO):
            with self.subTest(percorso=percorso):
                self.assertTrue(strumento.rilevante(percorso, solo=strumento.SOLO_CLI))
        for percorso in (self.LIBRERIA, self.BENCH, self.FUZZ):
            with self.subTest(percorso=percorso):
                self.assertFalse(strumento.rilevante(percorso, solo=strumento.SOLO_CLI))

    def test_la_vista_dedicata_e_un_sottoinsieme_di_quella_di_libreria(self) -> None:
        """La relazione fra i due perimetri, adesso.

        Prima erano **disgiunti** e una sonda lo pretendeva: il pacchetto stava
        fuori dalla soglia, quindi nessun file poteva stare in entrambi. Ora la
        vista dedicata guarda dentro il perimetro di libreria, e la proprieta'
        vera e' l'inclusione. Sommare i due numeri resta sbagliato -- si
        conterebbe due volte lo stesso codice -- ma per la ragione opposta di
        prima.
        """
        for percorso in (self.LIBRERIA, self.PUBBLICO, self.BINARIO, self.BENCH, self.FUZZ):
            with self.subTest(percorso=percorso):
                if strumento.rilevante(percorso, solo=strumento.SOLO_CLI):
                    self.assertTrue(
                        strumento.rilevante(percorso),
                        "la vista dedicata non puo' guardare fuori dalla soglia",
                    )

    def test_cio_che_non_e_una_crate_resta_fuori_da_entrambi(self) -> None:
        """Gli script e i file di supporto non sono codice del prodotto."""
        for percorso in ("scripts/coverage_diff.py", "fuzz/fuzz_targets/shp_reader.rs", "x"):
            with self.subTest(percorso=percorso):
                self.assertFalse(strumento.rilevante(percorso))
                self.assertFalse(strumento.rilevante(percorso, solo=strumento.SOLO_CLI))

    def test_la_crate_scelta_e_quella_che_il_checkpoint_misura(self) -> None:
        """Il nome sta in un posto solo.

        Due letterali -- uno qui e uno nel checkpoint -- divergerebbero senza
        che nessuno se ne accorga, e la sonda proverebbe un perimetro che la
        corsa non usa.
        """
        self.assertEqual(strumento.SOLO_CLI, "plenora-io-tools")
        self.assertNotIn(
            strumento.SOLO_CLI,
            strumento.ESCLUSI,
            "il pacchetto pubblico entra nel denominatore della soglia: "
            "escluderlo la falserebbe proprio sul codice nuovo",
        )
