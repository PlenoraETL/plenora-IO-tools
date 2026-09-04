#!/usr/bin/env python3
"""Le sonde del gate delle buste.

Un gate senza sonde e' un gate di cui ci si fida perche' non lo si e' mai visto
diventare rosso. Qui si muove ciascuno dei modi in cui manifesto e binario
possono divergere, e si guarda che il gate lo nomini -- e che nomini **quello**,
non un altro: un rosso che dice la cosa sbagliata manda a cercare nel posto
sbagliato, ed e' costato mezz'ora la settimana scorsa.

Le sonde non eseguono il binario: quello lo fa il gate, come passo del
checkpoint. Qui si esercitano le funzioni pure su documenti sintetici, piu' due
verifiche sull'albero vero -- che la matrice sia coerente con le fixture e che
il manifesto sia ben formato -- che non costano una compilazione.
"""

from __future__ import annotations

import json
import unittest

from scripts import check_buste_v2 as gate


class SondeDellaForma(unittest.TestCase):
    def test_gli_elementi_di_un_array_collassano(self) -> None:
        """Il contratto parla del tipo dell'elemento, non della sua posizione."""
        forma = gate.forma({"layers": [{"a": 1}, {"a": 2, "b": "x"}]})
        self.assertEqual(
            set(forma),
            {".layers", ".layers[]", ".layers[].a", ".layers[].b"},
        )

    def test_un_booleano_non_e_un_intero(self) -> None:
        """In Python `True` **e'** un `int`, e senza cura il tipo sarebbe quello."""
        forma = gate.forma({"truncated": True, "rows": 1})
        self.assertEqual(forma[".truncated"], {"boolean"})
        self.assertEqual(forma[".rows"], {"integer"})

    def test_il_nullo_e_un_tipo_e_non_un_assente(self) -> None:
        """`definition: null` e' un valore dichiarato, non un campo mancante."""
        self.assertEqual(gate.forma({"definition": None})[".definition"], {"null"})

    def test_i_tipi_si_uniscono_fra_gli_elementi(self) -> None:
        forma = gate.forma({"v": [None, "x"]})
        self.assertEqual(forma[".v[]"], {"null", "string"})


class SondeDelRaggruppamento(unittest.TestCase):
    def osservazione(self, caso, documento, attesa="c", flusso="stdout"):
        return {
            "caso": caso,
            "attesa": attesa,
            "exit": 0,
            "flusso": flusso,
            "documento": documento,
        }

    def test_in_tutte_e_l_intersezione(self) -> None:
        per_busta, problemi = gate.raggruppa(
            [
                self.osservazione("uno", {"contract": "c", "a": 1, "b": 2}),
                self.osservazione("due", {"contract": "c", "a": 1}),
            ]
        )
        self.assertEqual(problemi, [])
        stato = per_busta["c"]
        self.assertEqual(set(stato["osservati"]), {".contract", ".a", ".b"})
        self.assertEqual(stato["in_tutte"], {".contract", ".a"})

    def test_un_caso_senza_json_e_nominato(self) -> None:
        _, problemi = gate.raggruppa([self.osservazione("muto", None)])
        self.assertEqual(len(problemi), 1)
        self.assertIn("muto", problemi[0])
        self.assertIn("non ha prodotto JSON", problemi[0])

    def test_una_busta_diversa_da_quella_attesa_e_nominata(self) -> None:
        """Il buco che questo controllo chiude.

        Una fixture cancellata fa uscire una busta d'errore al posto di quella
        attesa. Senza questo controllo il gate resterebbe verde -- la busta
        d'errore e' dichiarata come le altre, quindi tutti i suoi percorsi
        tornerebbero -- e il caso sparirebbe dalla matrice in silenzio.
        """
        _, problemi = gate.raggruppa(
            [
                self.osservazione(
                    "sparita",
                    {
                        "contract": "plenora-io-error-v1",
                        "error": {"message": "file non trovato"},
                    },
                    attesa="plenora-io-read-v2",
                )
            ]
        )
        self.assertEqual(len(problemi), 1)
        self.assertIn("sparita", problemi[0])
        self.assertIn("plenora-io-read-v2", problemi[0])
        # Il messaggio dell'errore arriva fino al rosso: senza, chi legge
        # saprebbe che il caso e' fallito e non perche'.
        self.assertIn("file non trovato", problemi[0])


class SondeDelConfronto(unittest.TestCase):
    """I cinque modi di divergere, uno per sonda.

    La controprova positiva sta in `test_due_dichiarazioni_coincidenti`: senza,
    «diventa rosso» sarebbe vero anche di un confronto rosso sempre.
    """

    def stato(self, osservati: dict[str, set], in_tutte: set):
        return {"osservati": osservati, "in_tutte": in_tutte}

    def test_due_dichiarazioni_coincidenti(self) -> None:
        problemi = gate.confronta(
            "b",
            {".a": {"tipi": {"integer"}, "sempre": True}},
            self.stato({".a": {"integer"}}, {".a"}),
        )
        self.assertEqual(problemi, [])

    def test_un_percorso_emesso_e_non_dichiarato(self) -> None:
        problemi = gate.confronta("b", {}, self.stato({".a": {"integer"}}, {".a"}))
        self.assertEqual(len(problemi), 1)
        self.assertIn("non lo dichiara", problemi[0])

    def test_un_percorso_dichiarato_e_mai_emesso(self) -> None:
        problemi = gate.confronta(
            "b", {".a": {"tipi": {"integer"}, "sempre": True}}, self.stato({}, set())
        )
        self.assertEqual(len(problemi), 1)
        self.assertIn("nessun caso della matrice lo produce", problemi[0])

    def test_un_tipo_emesso_e_non_dichiarato(self) -> None:
        problemi = gate.confronta(
            "b",
            {".a": {"tipi": {"string"}, "sempre": True}},
            self.stato({".a": {"string", "null"}}, {".a"}),
        )
        self.assertEqual(len(problemi), 1)
        self.assertIn("['null']", problemi[0])

    def test_un_tipo_dichiarato_e_mai_emesso(self) -> None:
        problemi = gate.confronta(
            "b",
            {".a": {"tipi": {"string", "null"}, "sempre": True}},
            self.stato({".a": {"string"}}, {".a"}),
        )
        self.assertEqual(len(problemi), 1)
        self.assertIn("nessun caso lo produce con quel tipo", problemi[0])

    def test_sempre_discorde_nei_due_sensi(self) -> None:
        """Dichiarare opzionale cio' che c'e' sempre e' piu' debole del vero.

        Non e' falso, ed e' proprio per questo che il gate lo rifiuta: la
        dichiarazione piu' debole passerebbe in silenzio il giorno in cui il
        campo diventasse davvero condizionale.
        """
        troppo_forte = gate.confronta(
            "b",
            {".a": {"tipi": {"integer"}, "sempre": True}},
            self.stato({".a": {"integer"}}, set()),
        )
        self.assertEqual(len(troppo_forte), 1)
        self.assertIn("sempre=True", troppo_forte[0])

        troppo_debole = gate.confronta(
            "b",
            {".a": {"tipi": {"integer"}, "sempre": False}},
            self.stato({".a": {"integer"}}, {".a"}),
        )
        self.assertEqual(len(troppo_debole), 1)
        self.assertIn("sempre=False", troppo_debole[0])


class SondeDellaMatrice(unittest.TestCase):
    def test_i_nomi_dei_casi_sono_unici(self) -> None:
        nomi = [caso["nome"] for caso in gate.MATRICE]
        self.assertEqual(len(nomi), len(set(nomi)))

    def test_ogni_caso_dice_perche_esiste(self) -> None:
        """Un caso senza ragione scritta e' un caso che nessuno sa se togliere."""
        for caso in gate.MATRICE:
            with self.subTest(caso=caso["nome"]):
                self.assertTrue(caso["perche"].strip())

    def test_ogni_fixture_citata_esiste(self) -> None:
        """Prima di eseguire: una fixture assente e' un caso che si spegne.

        Il gate se ne accorgerebbe comunque -- la busta attesa non arriverebbe
        -- ma qui costa una `stat` invece di una compilazione, e il rosso dice
        il nome del file invece del contratto sbagliato.
        """
        radici = {"canoniche": gate.CANONICHE, "ostili": gate.OSTILI}
        for caso in gate.MATRICE:
            for argomento in caso["argomenti"]:
                for nome, radice in radici.items():
                    marcatore = "{" + nome + "}/"
                    if argomento.startswith(marcatore):
                        percorso = radice / argomento[len(marcatore) :]
                        with self.subTest(caso=caso["nome"], fixture=percorso.name):
                            self.assertTrue(percorso.exists(), percorso)

    def test_ogni_busta_attesa_e_dichiarata_dal_manifesto(self) -> None:
        manifesto = json.loads(gate.CONTRATTO.read_text(encoding="utf-8"))
        dichiarate = {
            voce["contract"] for voce in manifesto["envelopes"].values()
        }
        dichiarate.add(manifesto["busta_degli_errori"]["contract"])
        dichiarate |= {
            f"senza-contratto:{nome}"
            for nome in manifesto.get("buste_senza_contratto", {})
        }
        for caso in gate.MATRICE:
            with self.subTest(caso=caso["nome"]):
                self.assertIn(caso["busta"], dichiarate)

    def test_ogni_busta_dichiarata_e_prodotta_da_un_caso(self) -> None:
        """Il verso inverso: una busta descritta e mai esercitata."""
        manifesto = json.loads(gate.CONTRATTO.read_text(encoding="utf-8"))
        dichiarate = {voce["contract"] for voce in manifesto["envelopes"].values()}
        dichiarate.add(manifesto["busta_degli_errori"]["contract"])
        dichiarate |= {
            f"senza-contratto:{nome}"
            for nome in manifesto.get("buste_senza_contratto", {})
        }
        prodotte = {caso["busta"] for caso in gate.MATRICE}
        self.assertEqual(dichiarate - prodotte, set())


class SondeDelManifesto(unittest.TestCase):
    def strutture(self):
        manifesto = json.loads(gate.CONTRATTO.read_text(encoding="utf-8"))
        for nome, voce in manifesto["envelopes"].items():
            yield nome, voce["struttura"]
        yield "busta_degli_errori", manifesto["busta_degli_errori"]["struttura"]
        for nome, voce in manifesto["buste_senza_contratto"].items():
            yield nome, voce["struttura"]

    def test_ogni_busta_ha_una_struttura(self) -> None:
        nomi = [nome for nome, _ in self.strutture()]
        self.assertEqual(len(nomi), 7, nomi)

    def test_nessun_percorso_e_orfano(self) -> None:
        """`.a.b` senza `.a` descriverebbe un campo dentro un padre non dichiarato."""
        for nome, struttura in self.strutture():
            for percorso in struttura:
                padre = percorso.rsplit(".", 1)[0] if "." in percorso[1:] else ""
                if percorso.endswith("[]") or percorso.endswith("{}"):
                    padre = percorso[:-2]
                if not padre:
                    continue
                with self.subTest(busta=nome, percorso=percorso):
                    self.assertIn(padre, struttura)

    def test_un_padre_che_contiene_dichiara_di_contenere(self) -> None:
        """Un percorso con figli dev'essere un oggetto o un array, non una foglia."""
        for nome, struttura in self.strutture():
            for percorso, voce in struttura.items():
                figli = [
                    altro
                    for altro in struttura
                    if altro.startswith(percorso + ".")
                    or altro in (percorso + "[]", percorso + "{}")
                ]
                if not figli:
                    continue
                with self.subTest(busta=nome, percorso=percorso):
                    self.assertTrue(
                        {"object", "array"} & set(voce["tipi"]),
                        f"{percorso} ha figli e dichiara {voce['tipi']}",
                    )

    def test_i_tipi_dichiarati_sono_del_vocabolario_json(self) -> None:
        ammessi = {"object", "array", "string", "integer", "number", "boolean", "null"}
        for nome, struttura in self.strutture():
            for percorso, voce in struttura.items():
                with self.subTest(busta=nome, percorso=percorso):
                    self.assertTrue(voce["tipi"], f"{percorso} senza tipi")
                    self.assertEqual(set(voce["tipi"]) - ammessi, set())
                    self.assertIsInstance(voce["sempre"], bool)


if __name__ == "__main__":
    unittest.main()
