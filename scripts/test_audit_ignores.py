"""Sonde dei flag `--ignore` derivati dal registro delle eccezioni.

Il rischio che queste sonde coprono non e' il formato del flag: e' che
un'eccezione entri **senza la sua condizione di chiusura**. Un'eccezione senza
chiusura non e' temporanea, e' permanente senza dirlo, e il flag che ne deriva
la rende invisibile in CI.
"""

from __future__ import annotations

import json
import unittest
import subprocess
from unittest.mock import patch

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


class SondeAudit(unittest.TestCase):
    registro = {"yanked_accettati": [{"crate": "zip@=8.6.0"}]}

    @staticmethod
    def report(nome="zip", versione="8.6.0"):
        return {"warnings": {"yanked": [
            {"package": {"name": nome, "version": versione}}
        ]}}

    def test_solo_la_versione_esatta_dichiarata(self):
        self.assertEqual(gate.verifica_yanked(self.report(), self.registro), [])
        for nome, versione in (("zip", "8.6.1"), ("altro", "8.6.0")):
            self.assertTrue(gate.verifica_yanked(self.report(nome, versione), self.registro))
        self.assertTrue(gate.verifica_yanked(self.report(), {}))
        with self.assertRaises(ValueError):
            gate.verifica_yanked(self.report(), {"yanked_accettati": [{"crate": "zip"}]})

    def corsa(self, reports, codici=(0, 0, 0, 0), stderr=""):
        risultati = []
        for i, report in enumerate(reports):
            risultati.extend([
                subprocess.CompletedProcess([], codici[i * 2]),
                subprocess.CompletedProcess([], codici[i * 2 + 1], report, stderr),
            ])
        with patch.object(gate.subprocess, "run", side_effect=risultati) as run:
            risultato = gate.esegui({**self.registro, "accettate": [COMPLETA]})
        for i, lock in enumerate(("Cargo.lock", "fuzz/Cargo.lock")):
            a, b = [c.args[0] for c in run.call_args_list[i * 2:i * 2 + 2]]
            self.assertEqual(a, ["cargo", "audit", "--file", lock,
                                "--ignore", COMPLETA["id"], "--deny", "warnings", "--no-yanked"])
            self.assertEqual(b, ["cargo", "audit", "--file", lock,
                                "--ignore", COMPLETA["id"], "--json"])
        return risultato

    def test_zip_ammesso_in_entrambi_i_lockfile(self):
        self.assertEqual(self.corsa([json.dumps(self.report())] * 2), 0)

    def test_altro_yanked_blocca_anche_se_solo_nel_fuzz(self):
        for indice in (0, 1):
            reports = [json.dumps(self.report())] * 2
            reports[indice] = json.dumps(self.report("altro"))
            self.assertEqual(self.corsa(reports), 1)

    def test_advisory_e_errori_operativi_restano_bloccanti(self):
        for indice in range(4):
            codici = [0] * 4
            codici[indice] = 1
            self.assertEqual(self.corsa([json.dumps(self.report())] * 2, codici), 1)
        self.assertEqual(self.corsa([json.dumps(self.report())] * 2,
                                   stderr="error: couldn't check if the package is yanked"), 1)

    def test_report_assente_o_invalido_blocca(self):
        for report in ("", "{}", "null", '{"warnings": null}'):
            self.assertEqual(self.corsa([json.dumps(self.report()), report]), 1)


if __name__ == "__main__":
    unittest.main()
