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
    fn il_costruttore_storico_resta_coperto() {
        // Nel codice di test la via legacy resta lecita: e' la copertura della
        // via che si sta smantellando.
        let _ = PlenoraIoError::Contract("piano non valido".to_owned());
    }
}
"""

# Un crate non ancora migrato, con una chiamata legacy censita.
DRIVER_DA_MIGRARE = """use plenora_io_model::PlenoraIoError;

fn err(messaggio: String) -> PlenoraIoError {
    PlenoraIoError::Unsupported(messaggio)
}
"""


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

    def test_il_codice_di_test_resta_lecito(self) -> None:
        """La via legacy nei test è coperta apposta.

        Vietarla toglierebbe la copertura alla via che si sta smantellando,
        proprio mentre la si smantella.
        """
        errori, per_crate = gate.verifica(self.albero())
        self.assertEqual(errori, [])
        # Il modulo di test del crate migrato contiene una chiamata legacy, e
        # non conta: se contasse, l'albero conforme non passerebbe.
        self.assertNotIn("plenora-io-model", per_crate)

    def test_l_attrezzaggio_e_escluso(self) -> None:
        con_bench = {
            "crates/plenora-bench/src/main.rs": DRIVER_DA_MIGRARE,
        }
        errori, per_crate = gate.verifica(self.albero(con_bench))
        self.assertEqual(errori, [])
        self.assertNotIn("plenora-bench", per_crate)


if __name__ == "__main__":
    unittest.main()
