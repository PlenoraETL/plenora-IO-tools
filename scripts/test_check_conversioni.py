"""Sonde del gate che deriva la copertura della matrice cross-format.

Il gate afferma che le conversioni dichiarate coprono le classi che contano. Se
sbagliasse, il registro direbbe «coperto» di un insieme che lascia fuori un
driver, una classe di CRS o una forma di rifiuto, e nessuno se ne accorgerebbe:
una copertura sbagliata non fallisce, tace.

Le sonde muovono i modi in cui potrebbe diventare verde senza meritarlo, e il
primo di tutti e' quello che il gate esiste per chiudere: **contare un rifiuto
come copertura**. Un rifiuto prova che il driver non e' stato attraversato, e
accettarlo direbbe che un formato e' stato convertito proprio dove la
conversione non e' avvenuta.
"""

from __future__ import annotations

import copy
import importlib.util
import json
import pathlib
import sys
import unittest

RADICE = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(RADICE / "scripts"))


def carica():
    percorso = RADICE / "scripts" / "check-conversioni.py"
    spec = importlib.util.spec_from_file_location("check_conversioni", percorso)
    modulo = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(modulo)
    return modulo


class SondeDelGate(unittest.TestCase):
    def setUp(self) -> None:
        self.gate = carica()
        self.registro = json.loads(self.gate.REGISTRO.read_text(encoding="utf-8"))

    def copertura(self, registro: dict) -> list[str]:
        _, motivi = self.gate.copertura(registro)
        return motivi

    # --- la controprova positiva ------------------------------------------

    def test_il_registro_reale_e_verde(self) -> None:
        """Senza, «sempre rosso» sarebbe una difesa.

        Gira sul registro vero e sulla suite vera: un gate rosso anche
        sull'insieme buono non direbbe niente, e sarebbe il primo a essere
        disattivato.
        """
        _, errori = self.gate.verifica()
        self.assertEqual(errori, [], errori)

    def test_il_riassunto_deriva_i_conteggi_e_non_li_legge(self) -> None:
        """Il registro non dichiara quanti casi ha, e non deve.

        Un «sedici» scritto accanto alle conversioni inviterebbe a tenerlo
        fermo: a un caso irrealizzabile se ne sostituirebbe uno qualunque per
        non muovere la cifra. Qui il numero e' derivato, e cambiarlo non
        richiede di aggiornare niente.
        """
        riassunto, _ = self.gate.copertura(self.registro)
        self.assertEqual(riassunto["conversioni"], len(self.registro["conversioni"]))
        testo = json.dumps(self.registro, ensure_ascii=False)
        self.assertNotIn('"totale"', testo)
        self.assertNotIn('"riuscite":', testo)

    # --- i modi di diventare verde senza merito ---------------------------

    def test_un_rifiuto_non_copre_l_estremo(self) -> None:
        """Il cuore del gate: un driver coperto dal solo rifiuto resta scoperto.

        La conversione riuscita che porta `xls` come sorgente diventa un
        rifiuto. Il driver compare ancora nel registro, e il conteggio delle
        conversioni non cala: se il gate contasse le righe invece degli esiti,
        resterebbe verde.
        """
        registro = copy.deepcopy(self.registro)
        for conversione in registro["conversioni"]:
            if conversione["sorgente"] == "xls" and conversione["esito_atteso"] == "successo":
                conversione["esito_atteso"] = "rifiuto_capability"
        motivi = self.copertura(registro)
        self.assertTrue(
            any("«xls»" in motivo and "sorgente" in motivo for motivo in motivi),
            motivi,
        )

    def test_un_driver_mai_bersaglio_di_un_successo_e_scoperto(self) -> None:
        registro = copy.deepcopy(self.registro)
        registro["conversioni"] = [
            c for c in registro["conversioni"] if c["destinazione"] != "kml"
        ]
        motivi = self.copertura(registro)
        self.assertTrue(
            any("«kml»" in motivo and "destinazione" in motivo for motivo in motivi),
            motivi,
        )

    def test_una_classe_di_crs_scoperta_e_rossa(self) -> None:
        """Le tre classi si comportano in modo diverso al confine."""
        registro = copy.deepcopy(self.registro)
        registro["conversioni"] = [
            c
            for c in registro["conversioni"]
            if registro["driver"][c["sorgente"]]["crs_handling"] != "None"
        ]
        motivi = self.copertura(registro)
        self.assertTrue(any("crs_handling: None" in motivo for motivo in motivi), motivi)

    def test_una_classe_di_fedelta_scoperta_e_rossa(self) -> None:
        registro = copy.deepcopy(self.registro)
        registro["conversioni"] = [
            c
            for c in registro["conversioni"]
            if registro["driver"][c["destinazione"]]["fidelity_class"] != "Approximating"
        ]
        motivi = self.copertura(registro)
        self.assertTrue(
            any("fidelity_class: Approximating" in motivo for motivo in motivi), motivi
        )

    def test_senza_origine_multi_layer_e_rosso(self) -> None:
        registro = copy.deepcopy(self.registro)
        for nome in registro["driver"]:
            registro["driver"][nome]["multi_layer"] = False
        motivi = self.copertura(registro)
        self.assertTrue(any("multi-layer" in motivo for motivo in motivi), motivi)

    def test_ogni_forma_di_rifiuto_va_provata(self) -> None:
        """Tre autorita' rifiutano, e ciascuna ha il proprio caso."""
        for forma in sorted(self.gate.RIFIUTI):
            with self.subTest(forma=forma):
                registro = copy.deepcopy(self.registro)
                registro["conversioni"] = [
                    c for c in registro["conversioni"] if c["esito_atteso"] != forma
                ]
                motivi = self.copertura(registro)
                self.assertTrue(any(forma in motivo for motivo in motivi), motivi)

    def test_un_elenco_vuoto_non_supera_niente(self) -> None:
        registro = copy.deepcopy(self.registro)
        registro["conversioni"] = []
        errori = self.gate._conversioni_ben_formate(registro)
        self.assertTrue(any("assente o vuoto" in errore for errore in errori), errori)

    def test_un_test_dichiarato_e_non_scritto_e_rosso(self) -> None:
        """Un caso che nomina una prova inesistente conta nella copertura e non
        prova niente."""
        registro = copy.deepcopy(self.registro)
        registro["conversioni"][0]["test"] = "conversioni::una_prova_che_nessuno_ha_scritto"
        errori = self.gate._ogni_conversione_ha_il_proprio_test(registro)
        self.assertTrue(any("non definisce" in errore for errore in errori), errori)

    def test_una_fixture_inventata_e_rossa(self) -> None:
        """Il gate delle fixture guarda l'albero, questo guarda il registro.

        Un caso che nominasse un file inventato passerebbe il primo -- che dei
        file inesistenti non sa niente -- e conterebbe nella copertura senza
        poter essere eseguito.
        """
        registro = copy.deepcopy(self.registro)
        registro["conversioni"][0]["fixture"] = "canonico_inventato.parquet"
        errori = self.gate._ogni_fixture_e_dichiarata(registro)
        self.assertTrue(any("non e' dichiarata" in errore for errore in errori), errori)

    def test_un_esito_fuori_vocabolario_e_rosso(self) -> None:
        registro = copy.deepcopy(self.registro)
        registro["conversioni"][0]["esito_atteso"] = "quasi"
        errori = self.gate._conversioni_ben_formate(registro)
        self.assertTrue(any("non ammesso" in errore for errore in errori), errori)

    def test_un_driver_con_un_campo_in_piu_e_rosso(self) -> None:
        """I nomi sono quelli del `FormatDescriptor`, e uno in piu' e' un campo
        che nessun descrittore confronta."""
        registro = copy.deepcopy(self.registro)
        registro["driver"]["csv"]["inventato"] = True
        errori = self.gate._driver_ben_formati(registro)
        self.assertTrue(any("non previsti" in errore for errore in errori), errori)


if __name__ == "__main__":
    unittest.main()
