#!/usr/bin/env python3
"""Le sonde del registro del riuso."""

from __future__ import annotations

import contextlib
import copy
import io
import json
import unittest

from scripts import check_riuso_evidenze as gate


def registro() -> dict:
    return json.loads(gate.REGISTRO.read_text(encoding="utf-8"))


class SondeDelRegistroDelRiuso(unittest.TestCase):
    def test_il_registro_vero_regge(self) -> None:
        self.assertEqual(gate.verifica(registro()), [])

    def test_i_sei_tipi_ci_sono_tutti(self) -> None:
        tipi = {v["tipo"] for v in registro()["tipi_di_modifica"]}
        self.assertEqual(tipi, set(gate.TIPI))

    def test_togliere_un_tipo_e_rosso(self) -> None:
        # E' la via piu' breve al verde per una tabella: cancellare la riga che
        # non si vuole compilare.
        for tipo in sorted(gate.TIPI):
            with self.subTest(tipo=tipo):
                documento = copy.deepcopy(registro())
                documento["tipi_di_modifica"] = [
                    v for v in documento["tipi_di_modifica"] if v["tipo"] != tipo
                ]
                motivi = gate.verifica(documento)
                self.assertTrue(any(tipo in m for m in motivi), motivi)

    def test_un_tipo_inventato_e_rosso(self) -> None:
        documento = copy.deepcopy(registro())
        voce = copy.deepcopy(documento["tipi_di_modifica"][0])
        voce["tipo"] = "meteo"
        documento["tipi_di_modifica"].append(voce)
        motivi = gate.verifica(documento)
        self.assertTrue(any("meteo" in m for m in motivi), motivi)

    def test_ogni_campo_e_obbligatorio(self) -> None:
        for campo in gate.CAMPI:
            with self.subTest(campo=campo):
                documento = copy.deepcopy(registro())
                del documento["tipi_di_modifica"][0][campo]
                motivi = gate.verifica(documento)
                self.assertTrue(any(campo in m for m in motivi), motivi)

    def test_una_riga_senza_sorveglianti_e_rossa(self) -> None:
        # Una regola che nessuno verifica e' una buona intenzione.
        documento = copy.deepcopy(registro())
        documento["tipi_di_modifica"][0]["chi_se_ne_accorge"] = []
        motivi = gate.verifica(documento)
        self.assertTrue(any("buona intenzione" in m for m in motivi), motivi)

    def test_un_sorvegliante_che_non_esiste_e_rosso(self) -> None:
        """Il difetto piu' probabile di una tabella come questa.

        Non che la regola sia sbagliata: che un file venga rinominato e la riga
        resti indietro, cosi' il registro comincia a descrivere un mondo che
        non c'e' piu'.
        """
        documento = copy.deepcopy(registro())
        documento["tipi_di_modifica"][0]["chi_se_ne_accorge"] = [
            "scripts/check_mai_scritto.py"
        ]
        motivi = gate.verifica(documento)
        self.assertTrue(any("non e' un file" in m for m in motivi), motivi)

    def test_ogni_sorvegliante_nominato_esiste_davvero(self) -> None:
        # Sul registro **vero**, non su una copia: e' il controllo che tiene la
        # tabella agganciata al repository.
        for voce in registro()["tipi_di_modifica"]:
            for nome in voce["chi_se_ne_accorge"]:
                with self.subTest(sorvegliante=nome):
                    self.assertTrue((gate.ROOT / nome).is_file(), nome)

    def test_l_esito_dice_che_cosa_non_ha_verificato(self) -> None:
        # Un gate che dicesse solo «verificato» inviterebbe a credere che abbia
        # provato l'efficacia dei sorveglianti, che non prova.
        catturato = io.StringIO()
        with contextlib.redirect_stdout(catturato):
            self.assertEqual(gate.main(), 0)
        self.assertIn("lo provano le loro regressioni", catturato.getvalue())


if __name__ == "__main__":
    unittest.main()
