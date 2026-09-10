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
import re
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

    @staticmethod
    def _manifesto_con(dichiarazione: str) -> str:
        """Il manifesto vero, con la sola riga di `jsonschema` sostituita.

        Le tre sonde qui sotto scrivevano il numero di versione a mano, preso
        dal manifesto del giorno in cui furono scritte. Il 2026-09-09, alzando
        il pin a `=0.55.1`, la sostituzione ha smesso di mordere: il manifesto
        «mutato» tornava identico all'originale, il gate non trovava niente da
        segnalare e le sonde fallivano dicendo che il gate non vede una
        divergenza che nessuno gli aveva messo davanti.

        Una sonda che conosce il numero di versione misura anche quello, e non
        e' cio' che deve misurare.
        """
        manifesto = (gate.ROOT / "Cargo.toml").read_text(encoding="utf-8")
        nuovo, quante = re.subn(
            r"^jsonschema\s*=.+$", dichiarazione, manifesto, count=1, flags=re.M
        )
        assert quante == 1, "la riga di `jsonschema` non e' nel manifesto"
        return nuovo

    def test_senza_default_features_false_e_rosso(self) -> None:
        errori = gate.supply_chain(self._manifesto_con('jsonschema = "=9.9.9"'))
        self.assertTrue(any("default-features" in e for e in errori), errori)

    def test_una_feature_di_resolver_riaccesa_e_rossa(self) -> None:
        errori = gate.supply_chain(
            self._manifesto_con(
                'jsonschema = { version = "=9.9.9", default-features = false, '
                'features = ["resolve-http"] }'
            )
        )
        self.assertTrue(any("resolve-http" in e for e in errori), errori)

    def test_una_versione_non_esatta_e_rossa(self) -> None:
        errori = gate.supply_chain(
            self._manifesto_con(
                'jsonschema = { version = "9.9", default-features = false }'
            )
        )
        self.assertTrue(any("versione esatta" in e for e in errori), errori)

    def test_la_riga_sostituita_e_davvero_quella_reale(self) -> None:
        """La controprova delle tre sopra: senza, passerebbero anche se
        `_manifesto_con` restituisse un manifesto inventato."""
        vero = (gate.ROOT / "Cargo.toml").read_text(encoding="utf-8")
        sostituito = self._manifesto_con(
            'jsonschema = { version = "=9.9.9", default-features = false }'
        )
        self.assertNotEqual(vero, sostituito)
        self.assertEqual(gate.supply_chain(sostituito), [])

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

    @staticmethod
    def _closure() -> set[tuple[str, str]]:
        osservate = gate.closure_con_versioni()
        assert osservate is not None, "la closure non si ricava dal lock"
        return osservate

    @staticmethod
    def _coppia(osservate: set[tuple[str, str]], nome: str) -> tuple[str, str]:
        """La coppia di quel nome, presa dalla closure invece che scritta a mano.

        Scriverla a mano legherebbe la sonda al numero di versione del giorno,
        ed e' l'errore che il 2026-09-09 ha reso rosse tre sonde della supply
        chain quando il pin di `jsonschema` e' passato da 0.51.0 a 0.55.1.
        """
        trovate = [c for c in osservate if c[0] == nome]
        assert len(trovate) == 1, f"{nome}: {trovate}"
        return trovate[0]

    def test_il_censimento_reale_nomina_la_closure(self) -> None:
        self.assertEqual(gate.censimento_della_closure(), [])
        osservate = self._closure()
        # Non e' vuota: due insiemi vuoti coinciderebbero, e il confronto
        # direbbe «uguali» senza aver confrontato niente.
        self.assertGreater(len(osservate), 100)
        self.assertIn("jsonschema", {n for n, _ in osservate})
        # Ogni voce porta una versione: senza, il confronto per coppie
        # degenererebbe in un confronto per nomi senza dirlo.
        for nome, versione in osservate:
            self.assertTrue(nome, osservate)
            self.assertRegex(versione, r"^\d", f"{nome}: «{versione}»")

    def test_una_crate_entrata_senza_censimento_e_rossa(self) -> None:
        """Una dipendenza che entra senza essere censita entra senza che
        nessuno ne abbia guardato la licenza."""
        osservate = self._closure() | {("crate-mai-censita", "1.0.0")}
        errori = gate.censimento_della_closure(osservate)
        self.assertTrue(any("crate-mai-censita" in e for e in errori), errori)

    def test_una_crate_censita_e_sparita_e_rossa(self) -> None:
        """Un censimento che nomina cio' che non c'e' piu' e' un elenco che
        nessuno rilegge."""
        osservate = self._closure()
        errori = gate.censimento_della_closure(
            osservate - {self._coppia(osservate, "jsonschema")}
        )
        self.assertTrue(any("jsonschema" in e for e in errori), errori)

    # --- il perimetro: quale, e dichiarato ------------------------------

    def test_il_perimetro_e_quello_distribuito(self) -> None:
        """I bersagli sono quelli che la matrice dichiara distribuiti.

        Non tutti quelli che Cargo conosce: `--target all` comprende redox,
        haiku, wasm, android e le architetture i686 e aarch64 di Windows, che
        non spediamo. Sono due domande diverse, e il censimento risponde a
        quella che il suo titolo promette.
        """
        matrice = json.loads(
            (gate.ROOT / "assurance/registries/distribuzione-matrice.json").read_text(
                encoding="utf-8"
            )
        )
        distribuite = {p["id"] for p in matrice["piattaforme"]}
        non_distribuite = {p["id"] for p in matrice["piattaforme_non_distribuite"]}
        self.assertEqual(len(gate.TARGET_DISTRIBUITI), len(distribuite), distribuite)
        # `macos-aarch64` e' fra le non distribuite: nessun bersaglio apple.
        self.assertIn("macos-aarch64", non_distribuite)
        self.assertFalse(
            [t for t in gate.TARGET_DISTRIBUITI if "apple" in t],
            gate.TARGET_DISTRIBUITI,
        )

    def test_il_perimetro_esteso_e_piu_largo_e_dichiarato_tale(self) -> None:
        """Il registro non presenta i due perimetri come equivalenti."""
        self.assertEqual(gate.perimetro_esteso_misurato(), [])
        registro = json.loads(gate.CENSIMENTO.read_text(encoding="utf-8"))
        self.assertGreater(
            registro["perimetro_esteso"]["coppie"], len(registro["crate"])
        )
        self.assertTrue(registro["perimetro_esteso"]["non_e_equivalente"])

    def test_un_perimetro_esteso_ricordato_male_e_rosso(self) -> None:
        registro = json.loads(gate.CENSIMENTO.read_text(encoding="utf-8"))
        registro["perimetro_esteso"]["coppie"] = 1
        errori = gate.perimetro_esteso_misurato(registro)
        self.assertTrue(any("perimetro esteso" in e for e in errori), errori)

    # --- prodotto e build restano distinguibili -------------------------

    def test_le_origini_reali_sono_misurate(self) -> None:
        self.assertEqual(gate.origini_misurate(), [])
        registro = json.loads(gate.CENSIMENTO.read_text(encoding="utf-8"))
        origini = {v["origine"] for v in registro["crate"]}
        self.assertEqual(origini, {"prodotto", "build"})
        # Entrambe popolate: se una fosse vuota il campo distinguerebbe nulla.
        for attesa in ("prodotto", "build"):
            self.assertTrue(
                [v for v in registro["crate"] if v["origine"] == attesa], attesa
            )

    def test_una_dipendenza_di_build_spacciata_per_prodotto_e_rossa(self) -> None:
        """La distinzione serve a chi guarda una licenza: una crate che finisce
        nel binario e una che serve solo a costruirlo non pongono la stessa
        domanda."""
        registro = json.loads(gate.CENSIMENTO.read_text(encoding="utf-8"))
        vittima = next(v for v in registro["crate"] if v["origine"] == "build")
        vittima["origine"] = "prodotto"
        errori = gate.origini_misurate(registro)
        self.assertTrue(any(vittima["crate"] in e for e in errori), errori)
        self.assertTrue(any("build" in e and "prodotto" in e for e in errori), errori)

    def test_una_versione_sbagliata_col_nome_giusto_e_rossa(self) -> None:
        """Il caso per cui il confronto e' passato dai nomi alle coppie.

        Fino al 2026-09-09 il gate confrontava i soli nomi, e ventiquattro voci
        del censimento portavano una versione vecchia restando verdi. La licenza
        censita e' quella della versione dichiarata: una versione sbagliata
        censisce la licenza di un'altra crate.
        """
        osservate = self._closure()
        vera = self._coppia(osservate, "jsonschema")
        falsata = (osservate - {vera}) | {(vera[0], "0.0.1")}
        errori = gate.censimento_della_closure(falsata)
        self.assertTrue(any("jsonschema" in e for e in errori), errori)
        # Nomina entrambi i valori: chi legge nel registro della CI non ha il
        # censimento sott'occhio.
        self.assertTrue(
            any(vera[1] in e and "0.0.1" in e for e in errori),
            errori,
        )
        # E non lo racconta come una crate entrata o uscita, che sarebbe una
        # diagnosi diversa dal fatto.
        self.assertFalse(
            any("non nel censimento" in e or "non piu' nella closure" in e for e in errori),
            errori,
        )

    def test_una_crate_in_due_versioni_va_censita_due_volte(self) -> None:
        """Se una crate e' raggiungibile in due versioni, il censimento deve
        dirle entrambe: sono due licenze da guardare, non una."""
        osservate = self._closure()
        vera = self._coppia(osservate, "jsonschema")
        errori = gate.censimento_della_closure(osservate | {(vera[0], "0.0.1")})
        self.assertTrue(any("jsonschema" in e for e in errori), errori)

    def test_la_closure_distingue_due_versioni_dello_stesso_nome(self) -> None:
        """L'attraversamento risolve `nome versione`, non solo `nome`.

        `Cargo.lock` scrive la versione in una voce di `dependencies` soltanto
        quando il nome da solo sarebbe ambiguo. Indicizzando per nome, come
        faceva prima, la seconda voce del pacchetto sovrascriveva la prima e la
        distinzione spariva gia' nell'attraversamento.
        """
        lock = (gate.ROOT / "Cargo.lock").read_text(encoding="utf-8")
        nomi_del_lock: dict[str, set[str]] = {}
        for blocco in lock.split("[[package]]")[1:]:
            nome = re.search(r'^name = "([^"]+)"', blocco, re.M)
            versione = re.search(r'^version = "([^"]+)"', blocco, re.M)
            if nome and versione:
                nomi_del_lock.setdefault(nome.group(1), set()).add(versione.group(1))
        doppi = {n for n, v in nomi_del_lock.items() if len(v) > 1}
        self.assertTrue(doppi, "il lock non ha nomi in piu' versioni: sonda muta")

        # Delle crate in piu' versioni, questa closure deve portare esattamente
        # le versioni che l'attraversamento raggiunge -- e ognuna deve essere una
        # di quelle che il lock dichiara per quel nome, non una inventata.
        osservate = self._closure()
        for nome, versione in osservate:
            if nome in doppi:
                self.assertIn(versione, nomi_del_lock[nome], nome)

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
