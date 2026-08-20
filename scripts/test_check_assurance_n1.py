"""Sonde di ASSURANCE-N1.

La proprieta' che conta e' la **separazione delle due modalita'**: `--integrita`
puo' essere verde mentre il debito e' pieno, e non deve poter essere letto come
«la copertura negativa e' a posto».

E' la ragione per cui esistono due comandi invece di uno con una soglia: un
verde che significa due cose diverse a seconda di chi lo legge e' la forma di
falso verde che questa serie di checkpoint ha incontrato cinque volte.
"""

from __future__ import annotations

import unittest

from scripts import check_assurance_n1 as gate

APERTO = {
    "gruppo": "driver-x::f",
    "file": "scripts/check_assurance_n1.py",
    "righe": 3,
    "raggiunto_da_replay": 0,
    "disposizione": "test_tabellare",
    "nota": "ramo mai eseguito",
}
CHIUSO = {
    "gruppo": "driver-x::g",
    "file": "scripts/check_assurance_n1.py",
    "righe": 2,
    "raggiunto_da_replay": 0,
    "disposizione": "strutturale",
    "nota": "markup",
}


class SondeN1(unittest.TestCase):
    # --- integrita' del registro -------------------------------------------

    def test_un_registro_completo_e_integro(self) -> None:
        self.assertEqual(gate.integrita([APERTO, CHIUSO]), [])

    def test_una_disposizione_non_ammessa_e_rossa(self) -> None:
        voce = dict(APERTO, disposizione="vedremo")
        errori = gate.integrita([voce])
        self.assertTrue(any("non ammessa" in e for e in errori), errori)

    def test_una_disposizione_senza_nota_e_rossa(self) -> None:
        """Una casella riempita senza ragione non e' una disposizione."""
        voce = dict(APERTO, nota="")
        errori = gate.integrita([voce])
        self.assertTrue(any("senza nota" in e for e in errori), errori)

    def test_un_gruppo_duplicato_e_rosso(self) -> None:
        errori = gate.integrita([APERTO, dict(APERTO)])
        self.assertTrue(any("duplicata" in e for e in errori), errori)

    def test_un_file_sparito_e_rosso(self) -> None:
        """Una voce che sopravvive al proprio file tiene in vita un debito
        che non esiste piu', e gonfia il residuo."""
        voce = dict(APERTO, file="crates/driver-inesistente/src/lib.rs")
        errori = gate.integrita([voce])
        self.assertTrue(any("non esiste piu'" in e for e in errori), errori)

    def test_un_campo_mancante_e_rosso(self) -> None:
        voce = {k: v for k, v in APERTO.items() if k != "righe"}
        errori = gate.integrita([voce])
        self.assertTrue(any("campi mancanti" in e for e in errori), errori)

    # --- la separazione, che e' il punto ------------------------------------

    def test_il_debito_conta_solo_le_disposizioni_aperte(self) -> None:
        self.assertEqual([v["gruppo"] for v in gate.debito([APERTO, CHIUSO])], ["driver-x::f"])

    def test_un_registro_integro_puo_avere_debito_pieno(self) -> None:
        """La proprieta' decisiva.

        Un registro coerente **non** significa che i rami siano coperti. Se le
        due modalita' fossero una sola, questo caso darebbe verde.
        """
        gruppi = [APERTO, dict(APERTO, gruppo="driver-x::h")]
        self.assertEqual(gate.integrita(gruppi), [], "il registro e' coerente")
        self.assertEqual(len(gate.debito(gruppi)), 2, "e il debito e' pieno")

    def test_solo_disposizioni_chiuse_azzerano_il_debito(self) -> None:
        gruppi = [CHIUSO, dict(CHIUSO, gruppo="driver-x::i", disposizione="difensivo")]
        self.assertEqual(gate.integrita(gruppi), [])
        self.assertEqual(gate.debito(gruppi), [])


if __name__ == "__main__":
    unittest.main()
