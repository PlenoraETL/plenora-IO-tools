"""Sonde del censimento `WkbLimits::default()` (INFRA-1).

Il censimento e' stato riscritto da `percorso:riga` a `percorso::funzione`
perche' la chiave per riga si accendeva sui **movimenti**: qualunque
dichiarazione aggiunta sopra un'occorrenza rendeva rosso un gate a codice
invariato, e insegnava a riallinearlo senza guardare.

Un gate del genere ha due obblighi opposti, e vanno provati **entrambi**:

* tollerare spostamenti e riformattazione — altrimenti torna il difetto che
  questa riscrittura chiude;
* diventare rosso quando compare una nuova occorrenza di produzione —
  altrimenti la tolleranza ha mangiato il gate.

Le sonde girano su un albero finto: mutare i file veri lascerebbe il
repository sporco se un test si interrompe a meta'.
"""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from scripts import check_wkb_limits_defaults as gate

# Un driver con una sola occorrenza di produzione, dentro una funzione
# nominata, piu' una nei test e una fuori da ogni funzione per esercitare i
# rami di classificazione.
DRIVER_CONFORME = """use plenora_io_model::WkbLimits;

impl GpkgDriver {
    fn legge(&self, dati: &[u8]) -> Result<Geometry> {
        decode_wkb(dati, self.limits())
    }

    #[doc(hidden)]
    pub fn __fuzz_gpkg_geometry(dati: &[u8]) -> bool {
        decode_wkb(dati, &WkbLimits::default()).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rilegge_cio_che_ha_scritto() {
        let letta = decode_wkb(&scritta, &WkbLimits::default()).unwrap();
        assert_eq!(letta, attesa);
    }
}
"""


class SondeCensimento(unittest.TestCase):
    """Ogni sonda costruisce l'albero, lo verifica, poi lo muta."""

    def setUp(self) -> None:
        # Il censimento reale nomina file che l'albero finto non ha: lo
        # sostituisco con quello dell'albero, e ripristino dopo.
        self._legittime = dict(gate.LEGITTIME)
        self._attesi = dict(gate.ATTESI)
        gate.LEGITTIME.clear()
        gate.LEGITTIME[
            "crates/driver-gpkg/src/lib.rs::__fuzz_gpkg_geometry"
        ] = (1, "entry point del fuzzer, input gia' bounded dall'harness")
        gate.ATTESI.clear()
        gate.ATTESI.update({"test": 1, "attrezzaggio": 0, "produzione": 1})

    def tearDown(self) -> None:
        gate.LEGITTIME.clear()
        gate.LEGITTIME.update(self._legittime)
        gate.ATTESI.clear()
        gate.ATTESI.update(self._attesi)

    def albero(self, sostituzioni: dict[str, str] | None = None) -> Path:
        radice = Path(tempfile.mkdtemp())
        self.addCleanup(_rimuovi, radice)
        contenuti = {"crates/driver-gpkg/src/lib.rs": DRIVER_CONFORME}
        contenuti.update(sostituzioni or {})
        for relativo, testo in contenuti.items():
            percorso = radice / relativo
            percorso.parent.mkdir(parents=True, exist_ok=True)
            percorso.write_text(testo, encoding="utf-8")
        return radice

    # --- il gate riconosce l'albero conforme -------------------------------

    def test_l_albero_conforme_passa(self) -> None:
        errori, conteggi = gate.verifica(self.albero())
        self.assertEqual(errori, [])
        self.assertEqual(conteggi["produzione"], 1)
        self.assertEqual(conteggi["test"], 1)

    # --- obbligo 1: tollerare i movimenti ----------------------------------

    def test_lo_spostamento_verticale_non_accende_il_gate(self) -> None:
        """Il difetto che questa riscrittura chiude, in forma minima.

        Con la chiave per riga bastava questo per far diventare rosso il gate
        a codice identico.
        """
        spostato = "// preambolo\n" * 40 + DRIVER_CONFORME
        errori, _ = gate.verifica(
            self.albero({"crates/driver-gpkg/src/lib.rs": spostato})
        )
        self.assertEqual(errori, [], "uno spostamento verticale ha acceso il gate")

    def test_la_riformattazione_non_accende_il_gate(self) -> None:
        riformattato = DRIVER_CONFORME.replace(
            "        decode_wkb(dati, &WkbLimits::default()).is_ok()",
            "        decode_wkb(\n            dati,\n"
            "            &WkbLimits::default(),\n        )\n        .is_ok()",
        )
        self.assertNotEqual(riformattato, DRIVER_CONFORME)
        errori, _ = gate.verifica(
            self.albero({"crates/driver-gpkg/src/lib.rs": riformattato})
        )
        self.assertEqual(errori, [], "la riformattazione ha acceso il gate")

    def test_rinominare_il_file_intorno_non_conta(self) -> None:
        """Aggiungere un'altra funzione sopra non tocca la chiave."""
        con_funzione_nuova = DRIVER_CONFORME.replace(
            "impl GpkgDriver {",
            "fn aiuto_nuovo(x: u8) -> u8 {\n    x + 1\n}\n\nimpl GpkgDriver {",
        )
        errori, _ = gate.verifica(
            self.albero({"crates/driver-gpkg/src/lib.rs": con_funzione_nuova})
        )
        self.assertEqual(errori, [])

    # --- obbligo 2: accendersi su una nuova occorrenza ----------------------

    def test_una_occorrenza_in_una_funzione_nuova_e_rossa(self) -> None:
        con_residuo = DRIVER_CONFORME.replace(
            "    fn legge(&self, dati: &[u8]) -> Result<Geometry> {\n"
            "        decode_wkb(dati, self.limits())",
            "    fn legge(&self, dati: &[u8]) -> Result<Geometry> {\n"
            "        decode_wkb(dati, &WkbLimits::default())",
        )
        gate.ATTESI["produzione"] = 2
        errori, _ = gate.verifica(
            self.albero({"crates/driver-gpkg/src/lib.rs": con_residuo})
        )
        self.assertTrue(
            any("::legge" in messaggio and "non censito" in messaggio for messaggio in errori),
            f"una nuova occorrenza di produzione non e' stata intercettata: {errori}",
        )

    def test_una_seconda_occorrenza_nella_stessa_funzione_e_rossa(self) -> None:
        """Il caso che il conteggio per funzione esiste per intercettare."""
        raddoppiata = DRIVER_CONFORME.replace(
            "        decode_wkb(dati, &WkbLimits::default()).is_ok()",
            "        let _ = decode_wkb(dati, &WkbLimits::default());\n"
            "        decode_wkb(dati, &WkbLimits::default()).is_ok()",
        )
        gate.ATTESI["produzione"] = 2
        errori, _ = gate.verifica(
            self.albero({"crates/driver-gpkg/src/lib.rs": raddoppiata})
        )
        self.assertTrue(
            any("2 occorrenze, 1 censite" in messaggio for messaggio in errori),
            f"il raddoppio dentro una funzione censita non e' stato intercettato: {errori}",
        )

    def test_una_occorrenza_fuori_da_ogni_funzione_e_rossa(self) -> None:
        a_livello_di_modulo = (
            "use plenora_io_model::WkbLimits;\n"
            "static QUOTE: WkbLimits = WkbLimits::default();\n" + DRIVER_CONFORME
        )
        gate.ATTESI["produzione"] = 2
        errori, _ = gate.verifica(
            self.albero({"crates/driver-gpkg/src/lib.rs": a_livello_di_modulo})
        )
        self.assertTrue(
            any(gate.FUORI_DA_UNA_FUNZIONE in messaggio for messaggio in errori),
            f"un'occorrenza fuori da ogni funzione non e' stata intercettata: {errori}",
        )

    def test_una_voce_che_sopravvive_al_proprio_codice_e_rossa(self) -> None:
        """Il censimento non deve accumulare fantasmi."""
        senza = DRIVER_CONFORME.replace(
            "        decode_wkb(dati, &WkbLimits::default()).is_ok()",
            "        decode_wkb(dati, self.limits()).is_ok()",
        )
        gate.ATTESI["produzione"] = 0
        errori, _ = gate.verifica(
            self.albero({"crates/driver-gpkg/src/lib.rs": senza})
        )
        self.assertTrue(
            any("non piu' presente nel codice" in messaggio for messaggio in errori),
            f"una voce orfana non e' stata intercettata: {errori}",
        )

    # --- il commento non e' codice -----------------------------------------

    def test_la_motivazione_nel_commento_non_conta_come_occorrenza(self) -> None:
        con_commento = DRIVER_CONFORME.replace(
            "impl GpkgDriver {",
            "// Qui NON usiamo WkbLimits::default(): la quota viene dal\n"
            "// PipelineContext.\nimpl GpkgDriver {",
        )
        errori, conteggi = gate.verifica(
            self.albero({"crates/driver-gpkg/src/lib.rs": con_commento})
        )
        self.assertEqual(errori, [])
        self.assertEqual(conteggi["produzione"], 1)


def _rimuovi(radice: Path) -> None:
    import shutil

    shutil.rmtree(radice, ignore_errors=True)


if __name__ == "__main__":
    unittest.main()
