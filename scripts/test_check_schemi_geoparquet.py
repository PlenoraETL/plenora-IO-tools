"""Sonde del gate che tiene l'autorita' fuori dal codice.

Il gate afferma che la conformita' `GeoParquet` e' decisa dagli schemi
ufficiali. Se sbagliasse, l'invariante `lotto.s10` direbbe «validato contro
l'autorita'» mentre l'autorita' siamo noi -- ed e' esattamente il difetto che la
prima stesura aveva: derivava il perimetro dal modulo che doveva controllare.

Le sonde muovono i modi in cui potrebbe diventare verde senza meritarlo: uno
schema modificato in casa, un draft o un `$id` che non combaciano, un `$ref` che
punta fuori dagli schemi fissati, un elenco del codice che diverge da quello
dello schema, e la dipendenza che si riprende i resolver.
"""

from __future__ import annotations

import copy
import json
import unittest
from unittest import mock

from scripts import check_schemi_geoparquet as gate


class SondeDelGate(unittest.TestCase):
    def documenti(self) -> dict:
        documenti, errori = gate.schemi_fissati()
        self.assertEqual(errori, [], errori)
        return documenti

    # --- i file fissati sono quelli -----------------------------------

    def test_gli_schemi_reali_combaciano_col_lock(self) -> None:
        """La controprova positiva: senza, «sempre rosso» sarebbe una difesa."""
        documenti, errori = gate.schemi_fissati()
        self.assertEqual(errori, [], errori)
        self.assertEqual(len(documenti), 4)
        self.assertEqual(
            sorted(documenti),
            ["geoparquet-1.0.0", "geoparquet-1.1.0", "projjson-0.5", "projjson-0.7"],
        )

    def test_uno_schema_modificato_in_casa_e_rosso(self) -> None:
        """Un'autorita' che dice cio' che vogliamo noi non e' un'autorita'."""
        vero = gate.LOCK.read_bytes()
        registro = json.loads(vero)
        registro["schemi"][0]["sha256"] = "0" * 64
        _, errori = gate.schemi_fissati(registro)
        self.assertTrue(any("sha256" in e for e in errori), errori)

    def test_una_dimensione_diversa_dal_lock_e_rossa(self) -> None:
        registro = json.loads(gate.LOCK.read_text(encoding="utf-8"))
        registro["schemi"][0]["byte"] = 7
        _, errori = gate.schemi_fissati(registro)
        self.assertTrue(any("byte" in e for e in errori), errori)

    def test_un_draft_diverso_e_rosso(self) -> None:
        """Il validatore e' compilato per Draft 7: un altro draft si legge con
        regole che non sono le sue."""
        registro = json.loads(gate.LOCK.read_text(encoding="utf-8"))
        registro["schemi"][0]["draft"] = "http://json-schema.org/draft-04/schema#"
        _, errori = gate.schemi_fissati(registro)
        self.assertTrue(any("draft" in e for e in errori), errori)

    def test_un_id_projjson_diverso_e_rosso(self) -> None:
        """E' con quell'URI che il registro in memoria lo indicizza: se non
        combacia, il `$ref` non si risolve e la compilazione fallisce."""
        registro = json.loads(gate.LOCK.read_text(encoding="utf-8"))
        for voce in registro["schemi"]:
            if voce["famiglia"] == "projjson":
                voce["id"] = "https://esempio.invalido/projjson.json"
                break
        _, errori = gate.schemi_fissati(registro)
        self.assertTrue(any("`$id`" in e for e in errori), errori)

    # --- i `$ref` puntano dentro --------------------------------------

    def test_i_ref_reali_sono_risolti_dagli_schemi_fissati(self) -> None:
        self.assertEqual(gate.ref_risolti(self.documenti()), [])

    def test_un_ref_che_punta_fuori_e_rosso(self) -> None:
        documenti = copy.deepcopy(self.documenti())
        colonna = gate.colonna_dello_schema(documenti["geoparquet-1.1.0"])
        colonna["properties"]["crs"]["oneOf"][0]["$ref"] = "https://altrove.invalido/x.json"
        errori = gate.ref_risolti(documenti)
        self.assertTrue(any("altrove.invalido" in e for e in errori), errori)

    # --- il codice non si riscrive la specifica ------------------------

    def test_gli_elenchi_reali_vengono_dallo_schema(self) -> None:
        documenti = self.documenti()
        dallo_schema = gate.elenchi_dallo_schema(documenti)
        # Gli elenchi non sono vuoti: due elenchi vuoti coinciderebbero, e il
        # confronto direbbe «uguali» senza aver confrontato niente.
        for nome, valori in dallo_schema.items():
            self.assertTrue(valori, f"l'elenco «{nome}» estratto e' vuoto")
        self.assertEqual(len(dallo_schema["nomi_di_tipo"]), 7)
        self.assertEqual(len(dallo_schema["codifiche_native"]), 6)
        self.assertEqual(sorted(dallo_schema["suffissi"]), ["", " Z"])
        self.assertEqual(gate.elenchi_coincidono(documenti), [])

    def test_un_elenco_del_codice_che_diverge_e_rosso(self) -> None:
        """Il caso per cui il gate esiste: il codice che si riscrive la
        specifica, come faceva ammettendo `" M"` e `" ZM"`."""
        divergente = gate.elenchi_del_codice()
        divergente["suffissi"] = ["", " Z", " M", " ZM"]
        with mock.patch.object(gate, "elenchi_del_codice", return_value=divergente):
            errori = gate.elenchi_coincidono(self.documenti())
        self.assertTrue(any("suffissi" in e for e in errori), errori)

    # --- la dipendenza non si riprende i resolver ----------------------

    def test_la_dipendenza_reale_e_fissata_e_senza_resolver(self) -> None:
        self.assertEqual(gate.supply_chain(), [])

    def test_senza_default_features_false_e_rosso(self) -> None:
        manifesto = (gate.ROOT / "Cargo.toml").read_text(encoding="utf-8")
        riacceso = manifesto.replace(
            'jsonschema = { version = "=0.51.0", default-features = false }',
            'jsonschema = "=0.51.0"',
        )
        errori = gate.supply_chain(riacceso)
        self.assertTrue(any("default-features" in e for e in errori), errori)

    def test_una_feature_di_resolver_riaccesa_e_rossa(self) -> None:
        manifesto = (gate.ROOT / "Cargo.toml").read_text(encoding="utf-8")
        riacceso = manifesto.replace(
            'jsonschema = { version = "=0.51.0", default-features = false }',
            'jsonschema = { version = "=0.51.0", default-features = false, '
            'features = ["resolve-http"] }',
        )
        errori = gate.supply_chain(riacceso)
        self.assertTrue(any("resolve-http" in e for e in errori), errori)

    def test_una_versione_non_esatta_e_rossa(self) -> None:
        manifesto = (gate.ROOT / "Cargo.toml").read_text(encoding="utf-8")
        molle = manifesto.replace('"=0.51.0"', '"0.51"')
        errori = gate.supply_chain(molle)
        self.assertTrue(any("versione esatta" in e for e in errori), errori)

    # --- la closure del driver ----------------------------------------

    def test_la_closure_reale_e_pulita(self) -> None:
        self.assertEqual(gate.closure_del_driver(), [])

    def test_una_crate_di_rete_nella_closure_e_rossa(self) -> None:
        """Non basta guardare l'intero `Cargo.lock`: un'altra crate del
        workspace potrebbe dipendere da `reqwest` senza che questo driver lo
        faccia, e sarebbe legittimo."""
        import subprocess

        finto = subprocess.CompletedProcess(
            [], 0, stdout="driver-geoparquet v1.0.1\nreqwest v0.12.0\n", stderr=""
        )
        with mock.patch("subprocess.run", return_value=finto):
            errori = gate.closure_del_driver()
        self.assertTrue(any("reqwest" in e for e in errori), errori)

    # --- il perimetro dichiarato nel catalogo ---------------------------

    def test_il_perimetro_reale_e_coerente(self) -> None:
        self.assertEqual(gate.versione_dichiarata(), "1.1.0")
        self.assertEqual(gate.perimetro_dichiarato(self.documenti()), [])

    def test_un_perimetro_diverso_da_quello_degli_schemi_e_rosso(self) -> None:
        """`spec_version_supported` e' un'affermazione pubblica: chi legge il
        catalogo decide su di essa, e un perimetro dichiarato diverso da quello
        applicato e' peggio di nessun perimetro."""
        with mock.patch.object(gate, "versione_dichiarata", return_value="2.0.0"):
            errori = gate.perimetro_dichiarato(self.documenti())
        self.assertTrue(any("2.0.0" in e and "1.1.0" in e for e in errori), errori)

    def test_un_perimetro_non_dichiarato_e_rosso(self) -> None:
        with mock.patch.object(gate, "versione_dichiarata", return_value=None):
            errori = gate.perimetro_dichiarato(self.documenti())
        self.assertTrue(any("non dichiara" in e for e in errori), errori)

    # --- il censimento della closure ----------------------------------

    def test_il_censimento_reale_nomina_la_closure(self) -> None:
        self.assertEqual(gate.censimento_della_closure(), [])
        osservate = gate.closure_dal_lock()
        self.assertIsNotNone(osservate)
        # Non e' vuota: due insiemi vuoti coinciderebbero, e il confronto
        # direbbe «uguali» senza aver confrontato niente.
        self.assertGreater(len(osservate), 100)
        self.assertIn("jsonschema", osservate)

    def test_una_crate_entrata_senza_censimento_e_rossa(self) -> None:
        """Una dipendenza che entra senza essere censita entra senza che
        nessuno ne abbia guardato la licenza."""
        osservate = gate.closure_dal_lock() | {"crate-mai-censita"}
        errori = gate.censimento_della_closure(osservate)
        self.assertTrue(any("crate-mai-censita" in e for e in errori), errori)

    def test_una_crate_censita_e_sparita_e_rossa(self) -> None:
        """Un censimento che nomina cio' che non c'e' piu' e' un elenco che
        nessuno rilegge."""
        osservate = gate.closure_dal_lock() - {"jsonschema"}
        errori = gate.censimento_della_closure(osservate)
        self.assertTrue(any("jsonschema" in e for e in errori), errori)

    def test_il_censimento_registra_la_variazione_e_le_licenze(self) -> None:
        """Non soltanto un numero: quali crate e con quale licenza."""
        registro = json.loads(gate.CENSIMENTO.read_text(encoding="utf-8"))
        variazione = registro["variazione_s10"]
        self.assertEqual(len(variazione["elenco"]), variazione["nuove"])
        self.assertGreater(variazione["nuove"], 0)
        for voce in variazione["elenco"]:
            self.assertTrue(voce["crate"])
            self.assertTrue(voce["versione"])
            self.assertTrue(voce["licenza"], voce["crate"])
        nomi = {v["crate"] for v in variazione["elenco"]}
        self.assertIn("jsonschema", nomi)
        # E ogni crate della closure porta una licenza dichiarata.
        for voce in registro["crate"]:
            self.assertTrue(voce["licenza"], voce["crate"])

    # --- conformita' e compatibilita' restano separate -----------------

    def test_la_via_storica_non_entra_nella_prova_di_conformita(self) -> None:
        """Mescolarle direbbe che la via di compatibilita' e' conforme, e non lo
        e'. Sono due elenchi, e nessuna prova sta in tutti e due."""
        conformita = set(gate.PROVE_DI_CONFORMITA)
        compatibilita = set(gate.PROVE_DI_COMPATIBILITA)
        self.assertEqual(conformita & compatibilita, set())
        self.assertTrue(conformita and compatibilita)
        for prova in conformita:
            self.assertNotIn("storic", prova)
            self.assertNotIn("opt_in", prova)


if __name__ == "__main__":
    unittest.main()
