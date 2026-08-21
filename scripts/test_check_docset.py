"""Sonde del gate del docset.

Il gate ha sette doveri, e ciascuno puo' fallire in un modo diverso. Quello che
queste sonde fissano non e' che il docset attuale passi — lo si vede eseguendo
il gate — ma che **ogni controllo diventi rosso quando deve**.

L'eccezione piu' delicata e' che il gate stesso legge i documenti: li verifica,
non ne dipende. La sonda la fissa, cosi' estenderla ad altri script richiede di
scriverlo.
"""

from __future__ import annotations

import json
import unittest

from scripts import check_docset as gate


class SondePerimetro(unittest.TestCase):
    def test_l_allowlist_ha_sette_voci(self) -> None:
        self.assertEqual(len(gate.AMMESSI), 7)
        self.assertEqual(len(gate.CANONICI), 4)
        self.assertEqual(len(gate.OPERATIVI), 3)

    def test_ogni_file_operativo_dichiara_la_propria_ragione(self) -> None:
        """Un'eccezione senza ragione e' un'eccezione permanente senza dirlo."""
        for percorso, ragione in gate.OPERATIVI.items():
            self.assertTrue(ragione.strip(), percorso)

    def test_i_canonici_e_gli_operativi_non_si_sovrappongono(self) -> None:
        self.assertEqual(set(gate.CANONICI) & set(gate.OPERATIVI), set())


class SondeControlli(unittest.TestCase):
    """Ogni controllo, sul repository reale, e' verde."""

    def test_tutti_i_controlli_passano(self) -> None:
        for nome, controllo in gate.CONTROLLI:
            with self.subTest(controllo=nome):
                self.assertEqual(controllo(), [], nome)


class SondeCoerenzaStato(unittest.TestCase):
    """`docs/RELEASE.md` riporta i numeri della fonte strutturata."""

    def stato(self) -> dict:
        return json.loads(gate.STATO.read_text(encoding="utf-8"))

    def test_i_numeri_estratti_compaiono_nel_documento(self) -> None:
        numeri = gate._numeri(self.stato())
        testo = (gate.ROOT / "docs" / "RELEASE.md").read_text(encoding="utf-8")
        compatto = testo.replace(" ", "")
        for nome, valore in numeri.items():
            with self.subTest(numero=nome):
                self.assertTrue(
                    valore in testo or valore.replace(" ", "") in compatto,
                    f"{nome} = {valore}",
                )

    def test_un_numero_divergente_sarebbe_rosso(self) -> None:
        """La sonda decisiva: se il confronto fosse vacuo, due verita'
        potrebbero divergere senza che nulla lo dicesse."""
        stato = self.stato()
        stato["ultima_misura"]["fuzz"]["replay_input"] = 999999
        numeri = gate._numeri(stato)
        testo = (gate.ROOT / "docs" / "RELEASE.md").read_text(encoding="utf-8")
        self.assertNotIn(numeri["input di replay"], testo)

    def test_release_authorized_e_false_in_entrambi(self) -> None:
        self.assertIs(self.stato()["release_authorized"], False)
        testo = (gate.ROOT / "docs" / "RELEASE.md").read_text(encoding="utf-8")
        self.assertIn("release_authorized: false", testo)


class SondeEccezione(unittest.TestCase):
    def test_solo_il_gate_del_docset_puo_leggere_i_documenti(self) -> None:
        """L'eccezione e' ristretta a un file, e va tenuta tale.

        Il gate legge i documenti per **verificarli** — collegamenti, numeri,
        raggiungibilita' — che e' l'opposto di dipendere dalla prosa. Se domani
        un altro script leggesse un documento, quella sarebbe la dipendenza che
        la regola vieta.
        """
        self.assertEqual(
            gate.VALIDATORI,
            {"scripts/check_docset.py", "scripts/test_check_docset.py"},
            "l'eccezione si e' allargata oltre il validatore e la sua sonda",
        )

    def test_i_validatori_esistono(self) -> None:
        for relativo in gate.VALIDATORI:
            self.assertTrue((gate.ROOT / relativo).is_file(), relativo)

    def test_un_lettore_qualunque_non_e_ammesso(self) -> None:
        """La sonda decisiva: l'eccezione e' un insieme chiuso.

        Se `VALIDATORI` fosse ignorato e il controllo passasse tutto, questa
        resterebbe verde soltanto perche' non verifica nulla — quindi verifica
        il verso opposto: un nome che non e' nell'insieme non e' ammesso.
        """
        self.assertNotIn("scripts/check_release_contract.py", gate.VALIDATORI)
        self.assertNotIn("scripts/check_assurance_n1.py", gate.VALIDATORI)


if __name__ == "__main__":
    unittest.main()
