"""Sonde del censimento dei fallback (INFRA-4).

Il gate ha tre doveri, e vanno provati tutti e tre: contare le chiamate vere,
**non** contare commenti e stringhe, e accorgersi di un crate che nessuno ha
registrato.

Il secondo e' quello per cui questo file esiste. Il gate testuale che lo
precedeva contava il testo, e il 2026-08-21 un commento che spiegava perche' in
quel punto *non* si usasse `unwrap_or(...)` ha fatto salire il contatore. Una
sonda che prova solo il conteggio non avrebbe visto niente.

Le sonde girano su un albero finto: mutare i file veri lascerebbe il repository
sporco se un test si interrompe a meta'.
"""

from __future__ import annotations

import shutil
import tempfile
import unittest
from pathlib import Path

from scripts import check_assurance_fallbacks as gate

# Due chiamate vere, e nient'altro.
DUE_CHIAMATE = """pub fn primo(valore: Option<u64>) -> u64 {
    valore.unwrap_or(0)
}

pub fn secondo(valore: Option<u64>) -> u64 {
    valore.unwrap_or_else(|| 1)
}
"""

# Le stesse due chiamate, piu' un commento e una stringa che le nominano.
# Il gate testuale ne contava quattro.
DUE_CHIAMATE_E_DUE_ESCHE = '''pub fn primo(valore: Option<u64>) -> u64 {
    // Qui si potrebbe usare `unwrap_or(0)`, e infatti lo si usa.
    valore.unwrap_or(0)
}

pub fn secondo(valore: Option<u64>) -> u64 {
    let _spiegazione = "evitato un unwrap_or_else(|| ...) inutile";
    valore.unwrap_or_else(|| 1)
}
'''


class SondeFallback(unittest.TestCase):
    def albero(self, contenuto: str, crate: str = "driver-finto") -> Path:
        radice = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, radice, True)
        sorgente = radice / "crates" / crate / "src" / "lib.rs"
        sorgente.parent.mkdir(parents=True, exist_ok=True)
        sorgente.write_text(contenuto, encoding="utf-8")
        (radice / "crates" / crate / "Cargo.toml").write_text(
            '[package]\nname = "driver-finto"\n', encoding="utf-8"
        )
        return radice

    def con_radice(self, radice: Path):
        originale = gate.ROOT
        gate.ROOT = radice
        self.addCleanup(lambda: setattr(gate, "ROOT", originale))

    # --- primo dovere: contare le chiamate vere -----------------------------

    def test_conta_le_chiamate_vere(self) -> None:
        self.con_radice(self.albero(DUE_CHIAMATE))
        self.assertEqual(gate.conta("driver-finto"), 2)

    def test_il_registro_conforme_passa(self) -> None:
        self.con_radice(self.albero(DUE_CHIAMATE))
        self.assertEqual(gate.verifica({"driver-finto": 2}, 2), [])

    def test_una_chiamata_in_piu_e_rossa(self) -> None:
        """È la regressione che il registro esiste per prendere."""
        self.con_radice(
            self.albero(DUE_CHIAMATE + "\npub fn terzo(v: Option<u64>) -> u64 { v.unwrap_or(2) }\n")
        )
        errori = gate.verifica({"driver-finto": 2}, 2)
        self.assertTrue(
            any("registrati=2, trovati=3" in voce for voce in errori),
            f"la chiamata in piu' non e' stata intercettata: {errori}",
        )

    def test_una_chiamata_in_meno_e_rossa(self) -> None:
        """Anche in calo: un registro che non scende tiene in vita una ragione
        che nessuno rilegge, ed e' il verso in cui H-01 si sgonfia da solo."""
        self.con_radice(self.albero(DUE_CHIAMATE))
        errori = gate.verifica({"driver-finto": 3}, 3)
        self.assertTrue(
            any("registrati=3, trovati=2" in voce for voce in errori),
            f"il calo non e' stato intercettato: {errori}",
        )

    # --- secondo dovere: NON contare commenti e stringhe --------------------

    def test_commenti_e_stringhe_non_contano(self) -> None:
        """Il difetto che INFRA-4 chiude.

        Il gate testuale contava quattro occorrenze in questo file: due
        chiamate, un commento e una stringa. Il commento diceva l'opposto di
        quello che il contatore concludeva.
        """
        self.con_radice(self.albero(DUE_CHIAMATE_E_DUE_ESCHE))
        self.assertEqual(
            gate.conta("driver-finto"),
            2,
            "un commento o una stringa che nominano `unwrap_or` non sono chiamate",
        )
        self.assertEqual(gate.verifica({"driver-finto": 2}, 2), [])

    def test_una_chiamata_dentro_un_commento_non_sblocca_un_calo(self) -> None:
        """Il caso davvero insidioso.

        Se il conteggio scende di uno e qualcuno aggiunge un commento che nomina
        `unwrap_or`, un gate testuale torna verde senza che il codice sia
        cambiato. Con lo spoglio lessicale resta rosso.
        """
        solo_una = """pub fn primo(valore: Option<u64>) -> u64 {
    // Qui il fallback e' stato tolto: restava `unwrap_or(0)`, ora c'e' un match.
    match valore {
        Some(v) => v,
        None => 0,
    }
}

pub fn secondo(valore: Option<u64>) -> u64 {
    valore.unwrap_or_else(|| 1)
}
"""
        self.con_radice(self.albero(solo_una))
        errori = gate.verifica({"driver-finto": 2}, 2)
        self.assertTrue(
            any("registrati=2, trovati=1" in voce for voce in errori),
            f"il commento ha mascherato il calo: {errori}",
        )

    # --- terzo dovere: un crate non registrato ------------------------------

    def test_un_crate_non_registrato_e_rosso(self) -> None:
        """Il difetto che il gate testuale aveva davvero.

        `driver-common` non era nel suo elenco, quindi non veniva contato: un
        crate intero fuori dal registro, senza che nulla lo dicesse.
        """
        self.con_radice(self.albero(DUE_CHIAMATE))
        errori = gate.verifica({}, 0)
        self.assertTrue(
            any("presente ma non registrato" in voce for voce in errori),
            f"il crate non registrato non e' stato intercettato: {errori}",
        )

    def test_una_voce_senza_crate_e_rossa(self) -> None:
        """Il verso opposto: una riga che sopravvive al proprio crate."""
        self.con_radice(self.albero(DUE_CHIAMATE))
        errori = gate.verifica({"driver-finto": 2, "driver-sparito": 1}, 3)
        self.assertTrue(
            any("il crate non esiste" in voce for voce in errori),
            f"la voce fantasma non e' stata intercettata: {errori}",
        )

    # --- il totale ----------------------------------------------------------

    def test_il_totale_incoerente_e_rosso(self) -> None:
        """Il totale non e' ridondante: e' la somma che qualcuno ha guardato.

        Due voci che si spostano in direzioni opposte lasciano fermi i controlli
        per crate, e solo il totale — o la loro assenza — lo direbbe.
        """
        self.con_radice(self.albero(DUE_CHIAMATE))
        errori = gate.verifica({"driver-finto": 2}, 99)
        self.assertTrue(
            any("totale fallback del workspace inatteso: 2" in voce for voce in errori),
            f"il totale incoerente non e' stato intercettato: {errori}",
        )


if __name__ == "__main__":
    unittest.main()
