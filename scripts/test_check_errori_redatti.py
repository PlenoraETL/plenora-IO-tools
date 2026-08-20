"""Sonde del censimento dei costruttori d'errore legacy (S9).

Il gate ha due doveri opposti, e vanno provati **entrambi**: accorgersi di una
chiamata legacy aggiunta a un crate già migrato, e accorgersi di una voce di
censimento che è sopravvissuta al proprio codice. Un gate provato in una
direzione sola è severo o rumoroso, e non si sa quale delle due finché non
serve.

Le sonde girano su un albero finto: mutare i file veri lascerebbe il repository
sporco se un test si interrompe a metà.
"""

from __future__ import annotations

import shutil
import tempfile
import unittest
from pathlib import Path

from scripts import check_errori_redatti as gate

# Un crate «migrato»: usa solo il costruttore redatto.
MODELLO_MIGRATO = """use crate::error::{PlenoraIoError, PublicMessage};

fn quota_superata() -> PlenoraIoError {
    PlenoraIoError::limite_redatto(&PublicMessage::Curated("quota superata"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_quota_superata_si_costruisce() {
        // Dopo la rimozione della via legacy il codice di test usa la via
        // redatta come tutto il resto: non c'e' piu' una via vecchia da
        // coprire.
        let _ = quota_superata();
    }
}
"""

# Un crate non ancora migrato, con una chiamata legacy censita.
DRIVER_DA_MIGRARE = """use plenora_io_model::PlenoraIoError;

fn err(messaggio: String) -> PlenoraIoError {
    PlenoraIoError::Unsupported(messaggio)
}
"""


def doctest_di_modulo(attributi: str, *righe: str) -> str:
    """Un blocco doctest `//!` con gli attributi dati, davanti al driver finto.

    Costruito riga per riga invece che con un letterale multilinea: dentro un
    letterale i tre backtick e le sequenze di escape si mescolano male, e la
    fixture di un test deve essere leggibile a colpo d'occhio.
    """
    blocco = ["//! ```" + attributi]
    blocco.extend("//! " + riga for riga in righe)
    blocco.append("//! ```")
    return "\n".join(blocco) + "\n" + DRIVER_DA_MIGRARE


CHIAMATA_VIETATA = "let _ = plenora_io_model::PlenoraIoError::Crs(String::new());"


class SondeCensimento(unittest.TestCase):
    def setUp(self) -> None:
        self._migrati = gate.MIGRATI
        self._da_migrare = dict(gate.DA_MIGRARE)
        gate.MIGRATI = ("plenora-io-model",)
        gate.DA_MIGRARE.clear()
        gate.DA_MIGRARE["crates/driver-finto/src/lib.rs::err"] = 1

    def tearDown(self) -> None:
        gate.MIGRATI = self._migrati
        gate.DA_MIGRARE.clear()
        gate.DA_MIGRARE.update(self._da_migrare)

    def albero(self, sostituzioni: dict[str, str] | None = None) -> Path:
        radice = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, radice, True)
        contenuti = {
            "crates/plenora-io-model/src/error.rs": MODELLO_MIGRATO,
            "crates/driver-finto/src/lib.rs": DRIVER_DA_MIGRARE,
        }
        contenuti.update(sostituzioni or {})
        for relativo, testo in contenuti.items():
            percorso = radice / relativo
            percorso.parent.mkdir(parents=True, exist_ok=True)
            percorso.write_text(testo, encoding="utf-8")
        return radice

    def test_l_albero_conforme_passa(self) -> None:
        errori, per_crate = gate.verifica(self.albero())
        self.assertEqual(errori, [])
        self.assertNotIn("plenora-io-model", per_crate)
        self.assertEqual(per_crate.get("driver-finto"), 1)

    # --- primo dovere: l'aggiunta illecita in un crate migrato --------------

    def test_una_chiamata_legacy_in_un_crate_migrato_e_rossa(self) -> None:
        """È la regressione che questo gate esiste per prendere.

        Il conteggio globale non la vedrebbe se altrove una chiamata sparisse
        nello stesso commit: è già successo in FZ-0.2, dove un contatore fermo
        sembrava una conferma mentre due voci si annullavano.
        """
        ricaduto = MODELLO_MIGRATO.replace(
            'PlenoraIoError::limite_redatto(&PublicMessage::Curated("quota superata"))',
            'PlenoraIoError::LimitExceeded("quota superata".to_owned())',
        )
        errori, _ = gate.verifica(
            self.albero({"crates/plenora-io-model/src/error.rs": ricaduto})
        )
        self.assertTrue(
            any("dichiarato migrato" in voce for voce in errori),
            f"la ricaduta non e' stata intercettata: {errori}",
        )

    def test_una_chiamata_in_piu_in_un_crate_non_migrato_e_rossa(self) -> None:
        """Una seconda chiamata dentro una funzione già censita."""
        raddoppiato = DRIVER_DA_MIGRARE.replace(
            "    PlenoraIoError::Unsupported(messaggio)",
            "    let _ = PlenoraIoError::Contract(messaggio.clone());\n"
            "    PlenoraIoError::Unsupported(messaggio)",
        )
        errori, _ = gate.verifica(
            self.albero({"crates/driver-finto/src/lib.rs": raddoppiato})
        )
        self.assertTrue(
            any("2 chiamate, 1 censite" in voce for voce in errori),
            f"il raddoppio non e' stato intercettato: {errori}",
        )

    def test_una_chiamata_in_una_funzione_nuova_e_rossa(self) -> None:
        con_nuova = DRIVER_DA_MIGRARE + """
fn altro_errore() -> PlenoraIoError {
    PlenoraIoError::Schema("schema non valido".to_owned())
}
"""
        errori, _ = gate.verifica(
            self.albero({"crates/driver-finto/src/lib.rs": con_nuova})
        )
        self.assertTrue(
            any("::altro_errore" in voce and "non censita" in voce for voce in errori),
            f"la funzione nuova non e' stata intercettata: {errori}",
        )

    # --- secondo dovere: la voce obsoleta -----------------------------------

    def test_una_voce_che_sopravvive_al_proprio_codice_e_rossa(self) -> None:
        """Il censimento non deve accumulare fantasmi.

        Una riga che resta dopo che il codice è stato migrato tiene in vita una
        ragione che nessuno rilegge, e fa sembrare il debito più grande di
        quello che è.
        """
        migrato = DRIVER_DA_MIGRARE.replace(
            "    PlenoraIoError::Unsupported(messaggio)",
            "    PlenoraIoError::limite_redatto(&messaggio)",
        )
        errori, _ = gate.verifica(
            self.albero({"crates/driver-finto/src/lib.rs": migrato})
        )
        self.assertTrue(
            any("non piu' presente nel codice" in voce for voce in errori),
            f"la voce fantasma non e' stata intercettata: {errori}",
        )

    # --- il perimetro è dichiarato, non implicito ---------------------------

    def test_il_codice_di_test_conta_come_la_produzione(self) -> None:
        """La regola si e' **rovesciata** alla chiusura di S9, ed e' voluto.

        Finche' la migrazione procedeva un crate per volta, la via legacy nei
        test era la copertura della via che si stava smantellando, e vietarla
        avrebbe tolto quella copertura proprio mentre serviva. Ora i
        costruttori non esistono: una chiamata in un test non e' copertura, e'
        codice che non compila.
        """
        con_legacy_nei_test = MODELLO_MIGRATO.replace(
            "        let _ = quota_superata();",
            '        let _ = PlenoraIoError::Contract("piano non valido".to_owned());',
        )
        errori, _ = gate.verifica(
            self.albero({"crates/plenora-io-model/src/error.rs": con_legacy_nei_test})
        )
        self.assertTrue(
            any("dichiarato migrato" in voce for voce in errori),
            f"la chiamata nel modulo di test non e' stata contata: {errori}",
        )

    def test_l_attrezzaggio_non_e_piu_escluso(self) -> None:
        """`plenora-bench` e `plenora-fuzz` erano fuori dal conteggio.

        Erano esclusi perche' non sono codice spedito. Ma «non spedito» non
        vuol dire «non compilato»: dopo la rimozione, una chiamata legacy la'
        dentro romperebbe la build, e un gate che non la vede darebbe verde su
        un albero rotto.
        """
        con_bench = {"crates/plenora-bench/src/main.rs": DRIVER_DA_MIGRARE}
        errori, per_crate = gate.verifica(self.albero(con_bench))
        self.assertEqual(per_crate.get("plenora-bench"), 1)
        self.assertTrue(
            any("plenora-bench" in voce and "non censita" in voce for voce in errori),
            f"l'attrezzaggio e' ancora escluso: {errori}",
        )

    def test_i_target_di_fuzz_sono_nel_perimetro(self) -> None:
        """`fuzz/` non e' sotto `crates/`, e fino a S9 non veniva guardato."""
        errori, per_crate = gate.verifica(
            self.albero({"fuzz/fuzz_targets/finto.rs": DRIVER_DA_MIGRARE})
        )
        self.assertEqual(per_crate.get("fuzz"), 1)
        self.assertTrue(
            any("fuzz/fuzz_targets/finto.rs" in voce for voce in errori),
            f"i target di fuzz restano invisibili: {errori}",
        )

    # --- la prova che la rimozione e' avvenuta, indipendente da rustdoc -----

    def test_una_definizione_legacy_ricomparsa_e_rossa(self) -> None:
        """La prova che i doctest `compile_fail` **non** possono dare.

        Un `compile_fail` passa se il blocco non compila per una ragione
        qualunque — un import sbagliato basta. Questo controllo legge il
        sorgente e non dipende da rustdoc: e' il motivo per cui esiste accanto
        ai doctest invece che al posto loro.
        """
        con_definizione = MODELLO_MIGRATO + """
impl PlenoraIoError {
    pub fn Contract(message: String) -> Self {
        Self::contratto_redatto(&PublicMessage::Curated("x"))
    }
}
"""
        errori, _ = gate.verifica(
            self.albero({"crates/plenora-io-model/src/error.rs": con_definizione})
        )
        self.assertTrue(
            any("e' tornata a esistere" in voce for voce in errori),
            f"la definizione ricomparsa non e' stata intercettata: {errori}",
        )

    # --- i doctest: codice che vive dentro un commento ----------------------

    def test_una_chiamata_in_un_doctest_e_rossa(self) -> None:
        """`spoglia` cancella i commenti, e un doctest e' un commento.

        Senza il controllo apposito il gate non lo vedrebbe, e un esempio nella
        documentazione pubblica e' la prima cosa che un consumatore copia.
        """
        con_doctest = doctest_di_modulo("", CHIAMATA_VIETATA)
        errori, _ = gate.verifica(
            self.albero({"crates/driver-finto/src/lib.rs": con_doctest})
        )
        self.assertTrue(
            any("doctest alla riga" in voce for voce in errori),
            f"il doctest non e' stato guardato: {errori}",
        )

    def test_un_blocco_compile_fail_non_conta(self) -> None:
        """L'esclusione e' **semantica**, non una allowlist.

        Un blocco marcato `compile_fail` e' per definizione la prova che quel
        codice non compila: contarlo come violazione significherebbe rossare
        proprio la prova che la via legacy non esiste piu'.
        """
        con_compile_fail = doctest_di_modulo("compile_fail", CHIAMATA_VIETATA)
        errori, _ = gate.verifica(
            self.albero({"crates/driver-finto/src/lib.rs": con_compile_fail})
        )
        self.assertEqual(errori, [], f"il compile_fail e' stato contato: {errori}")

    def test_un_blocco_ignore_non_conta(self) -> None:
        """`ignore` non viene compilato: stessa ragione, altro marcatore."""
        con_ignore = doctest_di_modulo("ignore", CHIAMATA_VIETATA)
        errori, _ = gate.verifica(
            self.albero({"crates/driver-finto/src/lib.rs": con_ignore})
        )
        self.assertEqual(errori, [], f"il blocco ignore e' stato contato: {errori}")


if __name__ == "__main__":
    unittest.main()
