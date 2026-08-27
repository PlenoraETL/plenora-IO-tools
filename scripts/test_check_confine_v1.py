#!/usr/bin/env python3
"""Sonde di `check_confine_v1.py`.

Un gate verde sul repository sano dice che oggi e' verde, non che domani
diventerebbe rosso. Ogni proprieta' affermata dal gate ha qui una sonda che la
viola e pretende il rosso.
"""

from __future__ import annotations

import unittest

from scripts import check_confine_v1 as gate


class IlRepositorySano(unittest.TestCase):
    def test_il_gate_passa_su_cio_che_c_e(self):
        self.assertEqual(gate.verifica(), [])

    def test_i_chiamanti_reali_sono_i_due_dichiarati(self):
        # Se ne comparisse un terzo, il gate lo direbbe: qui si controlla che
        # non ne manchi uno dei due, che renderebbe la sonda sopra verde per la
        # ragione sbagliata.
        self.assertEqual(
            set(gate.chiamanti()), {gate.ADATTATORE, gate.PROPRIETARIO}
        )


class UnSecondoLettoreDellIdentita(unittest.TestCase):
    def test_un_chiamante_estraneo_e_rosso(self):
        trovati = {gate.ADATTATORE: 1, gate.PROPRIETARIO: 2, "driver-dxf/src/lib.rs": 1}
        errori = gate.verifica(trovati)
        self.assertTrue(any("driver-dxf/src/lib.rs" in e for e in errori), errori)

    def test_l_adattatore_che_smette_di_chiamarla_e_rosso(self):
        errori = gate.verifica({gate.PROPRIETARIO: 2})
        self.assertTrue(any(gate.ADATTATORE in e for e in errori), errori)

    def test_il_proprietario_non_e_un_estraneo(self):
        # La privatezza del campo lo confina gia' li': pretendere zero chiamate
        # nel modulo che lo possiede sarebbe una regola che non protegge niente
        # e costringerebbe a girarci intorno.
        self.assertEqual(gate.verifica({gate.ADATTATORE: 1, gate.PROPRIETARIO: 2}), [])


class IlCodiceDiProva(unittest.TestCase):
    def test_le_sonde_della_redazione_non_contano_come_chiamanti(self):
        # `driver.rs` chiama `detail_v1()` nelle due sonde della redazione: e'
        # cio' che quelle sonde verificano. Se contassero, il gate sarebbe rosso
        # sul repository sano e la disciplina si imparerebbe aggirandolo.
        driver = gate.CRATES / "plenora-io-core/src/driver.rs"
        intero = driver.read_text(encoding="utf-8")
        self.assertIn("detail_v1", intero, "le sonde della redazione la chiamano")
        self.assertNotIn(
            "detail_v1", gate.codice_di_produzione(driver), "ma non in produzione"
        )


class IlDeriveCheNonDeveTornare(unittest.TestCase):
    def test_oggi_non_deriva_serialize(self):
        derive = gate._derive_della_valutazione()
        self.assertIsNotNone(derive)
        self.assertNotIn("Serialize", derive)

    def test_la_regex_lo_troverebbe_se_tornasse(self):
        # La sonda vale quanto la sua capacita' di vedere: si verifica che
        # l'espressione riconosca davvero un derive con `Serialize`, se no
        # `test_oggi_non_deriva_serialize` passerebbe anche su una regex rotta.
        finto = "#[derive(Clone, Debug, Serialize)]\npub struct FidelityAssessment {\n"
        trovato = gate.DERIVE_DELLA_VALUTAZIONE.search(finto)
        self.assertIsNotNone(trovato)
        self.assertIn("Serialize", trovato.group(1))


if __name__ == "__main__":
    unittest.main()
