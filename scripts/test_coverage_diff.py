"""Sonde del filtro di perimetro della diagnostica differenziale.

Il perimetro decide **di che cosa parla** un numero, e due numeri con lo stesso
nome su insiemi diversi sono peggio di un numero solo. Qui si prova la sola
funzione che lo decide: `rilevante`.

# Perche' esiste una seconda misura

Lo scope della soglia si chiama «library coverage» e tiene fuori le crate non
libreria, `plenora-io-cli` compresa. La scelta resta -- una soglia sulle
librerie e' una scelta difendibile -- ma «fuori dalla soglia» era diventato
«fuori da ogni misura», e il binario che gli utenti eseguono era l'unica cosa
che nessuno guardava.

Da qui la corsa dedicata: stesso strumento, perimetro dichiarato, nessuna
soglia. E da qui queste sonde: il filtro sceglie il perimetro, e un filtro che
non morde non fallisce -- include tutto in silenzio, e la misura dedicata
direbbe i numeri di quella di libreria con un altro nome.
"""

from __future__ import annotations

import unittest

from scripts import coverage_diff as strumento


class SondePerimetro(unittest.TestCase):
    LIBRERIA = "crates/driver-shp/src/lib.rs"
    CLI = "crates/plenora-io-cli/src/main.rs"
    BENCH = "crates/plenora-bench/src/main.rs"
    FUZZ = "crates/plenora-fuzz/src/lib.rs"

    def test_il_perimetro_di_libreria_tiene_fuori_le_crate_non_libreria(self) -> None:
        self.assertTrue(strumento.rilevante(self.LIBRERIA))
        for percorso in (self.CLI, self.BENCH, self.FUZZ):
            with self.subTest(percorso=percorso):
                self.assertFalse(strumento.rilevante(percorso))

    def test_il_perimetro_dedicato_tiene_dentro_solo_la_crate_scelta(self) -> None:
        """La proprieta' che rende la seconda misura una misura diversa.

        Se il filtro lasciasse passare anche le librerie, i due numeri
        parlerebbero dello stesso insieme con due nomi, e il secondo sembrerebbe
        una conferma del primo invece che un'altra cosa."""
        self.assertTrue(strumento.rilevante(self.CLI, solo=strumento.SOLO_CLI))
        for percorso in (self.LIBRERIA, self.BENCH, self.FUZZ):
            with self.subTest(percorso=percorso):
                self.assertFalse(strumento.rilevante(percorso, solo=strumento.SOLO_CLI))

    def test_i_due_perimetri_non_si_sovrappongono(self) -> None:
        """Sommare i due numeri darebbe un terzo che non e' ne' l'uno ne'
        l'altro; che gli insiemi siano disgiunti e' cio' che lo rende
        vero."""
        percorsi = (self.LIBRERIA, self.CLI, self.BENCH, self.FUZZ)
        for percorso in percorsi:
            with self.subTest(percorso=percorso):
                self.assertFalse(
                    strumento.rilevante(percorso)
                    and strumento.rilevante(percorso, solo=strumento.SOLO_CLI),
                    "nessun file puo' stare in entrambi i perimetri",
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
        corsa non usa."""
        self.assertEqual(strumento.SOLO_CLI, "plenora-io-cli")
        self.assertIn(strumento.SOLO_CLI, strumento.ESCLUSI)


if __name__ == "__main__":
    unittest.main()
