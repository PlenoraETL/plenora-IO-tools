"""Sonde dei flag `--ignore` derivati dal registro delle eccezioni.

Il rischio che queste sonde coprono non e' il formato del flag: e' che
un'eccezione entri **senza la sua condizione di chiusura**. Un'eccezione senza
chiusura non e' temporanea, e' permanente senza dirlo, e il flag che ne deriva
la rende invisibile in CI.
"""

from __future__ import annotations

import json
import unittest

from scripts import audit_ignores as gate

COMPLETA = {
    "id": "RUSTSEC-0000-0000",
    "crate": "finto",
    "accettata_dal": "2026-01-01",
    "motivo": "una ragione",
    "esposizione": "nessuna",
    "chiusura": "aggiornare la dipendenza",
    "trigger_di_riesame": "nuova release",
}


class SondeFlag(unittest.TestCase):
    def test_il_registro_reale_produce_i_flag(self) -> None:
        registro = json.loads(gate.REGISTRO.read_text(encoding="utf-8"))
        prodotti = gate.flag(registro)
        self.assertEqual(len(prodotti) % 2, 0, "i flag vanno a coppie")
        self.assertEqual(set(prodotti[::2]), {"--ignore"})
        for identita in prodotti[1::2]:
            self.assertTrue(identita.startswith("RUSTSEC-"), identita)

    def test_una_voce_completa_produce_il_suo_flag(self) -> None:
        self.assertEqual(
            gate.flag({"accettate": [COMPLETA]}),
            ["--ignore", "RUSTSEC-0000-0000"],
        )

    def test_un_registro_vuoto_non_ignora_nulla(self) -> None:
        """Nessun advisory ignorato per assenza di elenco: `cargo audit`
        continua a bloccare tutto."""
        self.assertEqual(gate.flag({"accettate": []}), [])

    def test_un_eccezione_senza_chiusura_e_rossa(self) -> None:
        senza = {k: v for k, v in COMPLETA.items() if k != "chiusura"}
        with self.assertRaises(ValueError) as caso:
            gate.flag({"accettate": [senza]})
        self.assertIn("chiusura", str(caso.exception))

    def test_un_eccezione_senza_trigger_e_rossa(self) -> None:
        senza = {k: v for k, v in COMPLETA.items() if k != "trigger_di_riesame"}
        with self.assertRaises(ValueError):
            gate.flag({"accettate": [senza]})

    def test_un_eccezione_senza_motivo_e_rossa(self) -> None:
        senza = {k: v for k, v in COMPLETA.items() if k != "motivo"}
        with self.assertRaises(ValueError):
            gate.flag({"accettate": [senza]})


if __name__ == "__main__":
    unittest.main()
