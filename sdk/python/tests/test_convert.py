"""`convert()`: le fixture controllate e il prodotto vero.

Le sonde sui modelli costruiscono buste sintetiche, perche' devono descrivere
combinazioni che una conversione vera non produce a comando -- una perdita
troncata, un esito di pubblicazione diverso. Quelle sul client eseguono il
binario, perche' un modello che si decodifica non dice che il comando lo
consegni.
"""

from __future__ import annotations

import os
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

from plenora_io import (
    Client,
    CommandFailed,
    ConflictError,
    ConvertResult,
    Limits,
    LossReport,
    ProtocolError,
)
from plenora_io.discovery import NOME, VARIABILE

RADICE = Path(__file__).resolve().parents[3]
CANONICHE = RADICE / "crates" / "plenora-io-cli" / "tests" / "fixtures" / "canoniche"


def omesse_sane():
    return {
        "categorie_omesse": 0,
        "ragioni_omesse": 0,
        "esempi_omessi": 0,
        "omesse_per_byte": 0,
    }


def fedelta_sana(**modifiche):
    documento = {
        "level": "exact",
        "reasons": [],
        "troncato": False,
        "omesse": omesse_sane(),
        "omesse_esatte": True,
    }
    documento.update(modifiche)
    return documento


def perdita_sana(**modifiche):
    documento = {
        "lossless": False,
        "counts": [{"categoria": "crs_id_not_preserved_absent", "conteggio": 1}],
        "esempi": [
            {
                "category": "crs_id_not_preserved_absent",
                "context": "representation=crs_id value_bytes=9",
                "field_index": 0,
                "layer_index": 0,
            }
        ],
        "troncato": False,
        "omesse": omesse_sane(),
        "omesse_esatte": True,
    }
    documento.update(modifiche)
    return documento


def conversione_sana(**modifiche):
    documento = {
        "status": "ok",
        "protocol_version": 2,
        "contract": "plenora-io-convert-v2",
        "from": "geojson",
        "to": "csv",
        "layers": [{"name": "canonico", "rows": 5, "batches": 1}],
        "total_rows": 5,
        "bytes_written": 599,
        "publish_outcome": "published",
        "read_fidelity": fedelta_sana(),
        "write_fidelity": fedelta_sana(),
        "conversion_fidelity": fedelta_sana(),
        "read_loss": perdita_sana(lossless=True, counts=[], esempi=[]),
        "write_loss": perdita_sana(),
    }
    documento.update(modifiche)
    return documento


class LaBustaDiConvert(unittest.TestCase):
    def test_letta_intera(self) -> None:
        esito = ConvertResult.from_json(conversione_sana())
        self.assertEqual(esito.from_, "geojson")
        self.assertEqual(esito.to, "csv")
        self.assertEqual(esito.total_rows, 5)
        self.assertTrue(esito.published)

    def test_from_e_l_unico_campo_rinominato(self) -> None:
        """`from` e' una parola chiave e non puo' essere un attributo.

        Il nome del wire resta leggibile in `raw`, cosi' chi conosce il
        contratto non deve imparare la deviazione per ritrovare il campo.
        """
        esito = ConvertResult.from_json(conversione_sana())
        self.assertEqual(esito.raw["from"], esito.from_)
        self.assertEqual(set(ConvertResult.RINOMINATI), {"from"})
        self.assertFalse(hasattr(esito, "from"))

    def test_pubblicata_non_e_sinonimo_di_riuscita(self) -> None:
        """Una conversione puo' riuscire e non pubblicare: e' `publish_outcome`
        a dirlo, e leggere il successo dal solo ritorno perderebbe la
        differenza."""
        esito = ConvertResult.from_json(conversione_sana(publish_outcome="skipped"))
        self.assertFalse(esito.published)
        self.assertEqual(esito.publish_outcome, "skipped")

    def test_senza_perdita_vuol_dire_da_entrambi_i_lati(self) -> None:
        pulita = conversione_sana(
            read_loss=perdita_sana(lossless=True, counts=[], esempi=[]),
            write_loss=perdita_sana(lossless=True, counts=[], esempi=[]),
        )
        self.assertTrue(ConvertResult.from_json(pulita).lossless)
        # Basta un lato per non esserlo.
        self.assertFalse(ConvertResult.from_json(conversione_sana()).lossless)

    def test_ogni_campo_obbligatorio_mancante_e_un_errore(self) -> None:
        for campo in ConvertResult.OBBLIGATORI:
            with self.subTest(campo=campo):
                documento = conversione_sana()
                del documento[campo]
                with self.assertRaises(ProtocolError):
                    ConvertResult.from_json(documento)

    def test_un_layer_che_non_c_e_solleva_ed_elenca(self) -> None:
        esito = ConvertResult.from_json(conversione_sana())
        with self.assertRaises(KeyError) as preso:
            esito.layer("altro")
        self.assertIn("canonico", str(preso.exception))


class IlRapportoDiPerdita(unittest.TestCase):
    def test_conteggi_ed_esempi(self) -> None:
        perdita = LossReport.from_json(perdita_sana())
        self.assertFalse(perdita.lossless)
        self.assertEqual(perdita.categories, ["crs_id_not_preserved_absent"])
        self.assertEqual(perdita.count("crs_id_not_preserved_absent"), 1)
        self.assertEqual(perdita.esempi[0].field_index, 0)

    def test_una_categoria_assente_vale_zero(self) -> None:
        """Zero e non `KeyError`: una categoria che non c'e' vuol dire che
        quella perdita non si e' verificata, ed e' una risposta."""
        self.assertEqual(LossReport.from_json(perdita_sana()).count("altro"), 0)

    def test_lossless_non_e_l_unica_informazione(self) -> None:
        """Quando e' falso, `counts` dice quanto e `esempi` dice dove: un
        consumatore che si fermasse al booleano saprebbe che qualcosa e' andato
        perso e non che cosa."""
        perdita = LossReport.from_json(perdita_sana())
        self.assertFalse(perdita.lossless)
        self.assertTrue(perdita.counts)
        self.assertTrue(perdita.esempi)

    def test_una_perdita_troncata_lo_dichiara(self) -> None:
        """L'elenco che si legge **non e' tutto**, e un consumatore che lo
        trascurasse concluderebbe dall'assenza di una categoria che quella
        perdita non c'e'."""
        perdita = LossReport.from_json(
            perdita_sana(
                troncato=True,
                omesse={**omesse_sane(), "categorie_omesse": 3},
                omesse_esatte=True,
            )
        )
        self.assertTrue(perdita.troncato)
        self.assertTrue(perdita.omesse.any)
        self.assertEqual(perdita.omesse.categorie_omesse, 3)

    def test_counts_non_elenco(self) -> None:
        with self.assertRaises(ProtocolError) as preso:
            LossReport.from_json(perdita_sana(counts={}))
        self.assertIn("non un elenco", str(preso.exception))


class ControIlBinarioVero(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        indicato = os.environ.get(VARIABILE)
        cls.binario = indicato or shutil.which(NOME)
        if cls.binario is None:
            costruito = RADICE / "target" / "debug" / NOME
            cls.binario = str(costruito) if costruito.is_file() else None

    def setUp(self) -> None:
        if self.binario is None:
            self.skipTest("nessun binario plenora-io da esercitare")
        self._temporanea = tempfile.TemporaryDirectory(prefix="plenora-sdk-conv-")
        self.tmp = Path(self._temporanea.name)
        self.cliente = Client(binary=self.binario)

    def tearDown(self) -> None:
        self._temporanea.cleanup()

    def test_una_conversione_pubblica_e_conta(self) -> None:
        uscita = self.tmp / "da-geojson.csv"
        esito = self.cliente.convert(CANONICHE / "canonico.geojson", uscita)

        self.assertEqual(esito.contract, "plenora-io-convert-v2")
        self.assertEqual(esito.from_, "geojson")
        self.assertEqual(esito.to, "csv")
        self.assertTrue(esito.published)
        self.assertTrue(uscita.exists(), "la destinazione esiste davvero")
        self.assertEqual(uscita.stat().st_size, esito.bytes_written)
        self.assertEqual(
            esito.total_rows,
            sum(strato.rows for strato in esito.layers),
            "il totale e' la somma dei layer",
        )

    def test_la_perdita_arriva_strutturata(self) -> None:
        """GeoJSON verso CSV perde l'identificatore del CRS, e il rapporto lo
        dice con la categoria e un esempio invece che con un booleano."""
        esito = self.cliente.convert(
            CANONICHE / "canonico.geojson", self.tmp / "perdita.csv"
        )
        self.assertFalse(esito.lossless)
        self.assertTrue(esito.write_loss.categories)
        for esempio in esito.write_loss.esempi:
            self.assertIn(esempio.category, esito.write_loss.categories)
            self.assertGreaterEqual(esempio.field_index, 0)

    def test_le_tre_fedelta_rispondono_a_domande_diverse(self) -> None:
        """`conversion_fidelity` non e' piu' forte di nessuna delle due: la
        coppia promette al piu' quanto promette il piu' debole dei due formati."""
        esito = self.cliente.convert(
            CANONICHE / "canonico.geojson", self.tmp / "tre.csv"
        )
        livelli = {"exact": 0, "conditional": 1, "approximating": 2, "lossy": 3}
        self.assertGreaterEqual(
            livelli[esito.conversion_fidelity.level],
            max(
                livelli[esito.read_fidelity.level],
                livelli[esito.write_fidelity.level],
            ),
        )

    def test_una_destinazione_esistente_e_un_conflitto(self) -> None:
        uscita = self.tmp / "occupata.csv"
        uscita.write_text("gia' qui", encoding="utf-8")
        with self.assertRaises(ConflictError) as preso:
            self.cliente.convert(CANONICHE / "canonico.geojson", uscita)
        self.assertEqual(preso.exception.envelope.category, "conflict")
        self.assertEqual(
            uscita.read_text(encoding="utf-8"),
            "gia' qui",
            "il file di prima non e' stato toccato",
        )

    def test_le_opzioni_di_lettura_e_scrittura_vanno_a_driver_diversi(self) -> None:
        """La stessa chiave puo' esistere per entrambi con significati diversi.

        `delimiter` vale per il CSV che si legge e per quello che si scrive, e
        un unico dizionario costringerebbe a indovinare a chi vada. Due
        conversioni, una per lato, perche' CSV verso CSV incontra un vincolo
        che non riguarda le opzioni: il formato pretende una dichiarazione
        preventiva dei tipi geometrici, che un CSV letto da WKT non porta.
        """
        sorgente = self.tmp / "punto-e-virgola.csv"
        sorgente.write_text("geometry;nome\nPOINT (1 2);uno\n", encoding="utf-8")

        # Le opzioni di **lettura**: senza, il delimitatore sarebbe la virgola
        # e il file non si leggerebbe come tabella.
        letto = self.cliente.convert(
            sorgente,
            self.tmp / "da-csv.geojson",
            # GeoJSON impone il proprio CRS: assumerne un altro e' un rifiuto
            # tipizzato, non un'approssimazione silenziosa.
            assume_crs="OGC:CRS84",
            read_options={"delimiter": ";", "wkt_column": "geometry"},
        )
        self.assertTrue(letto.published)
        self.assertEqual(letto.total_rows, 1)

        # Le opzioni di **scrittura**: la stessa chiave, all'altro driver.
        uscita = self.tmp / "a-punto-e-virgola.csv"
        scritto = self.cliente.convert(
            CANONICHE / "canonico.geojson", uscita, write_options={"delimiter": ";"}
        )
        self.assertTrue(scritto.published)
        intestazione = uscita.read_text(encoding="utf-8").splitlines()[0]
        self.assertIn(";", intestazione)
        self.assertNotIn(",", intestazione)

    def test_durable_non_cambia_l_esito(self) -> None:
        """Costa in tempo e non in significato: la busta e' la stessa."""
        esito = self.cliente.convert(
            CANONICHE / "canonico.geojson", self.tmp / "durevole.csv", durable=True
        )
        self.assertTrue(esito.published)

    def test_un_tetto_superato_ferma_la_conversione(self) -> None:
        uscita = self.tmp / "mai-scritta.csv"
        with self.assertRaises(CommandFailed) as preso:
            self.cliente.convert(
                CANONICHE / "canonico.geojson", uscita, limits=Limits(max_rows=1)
            )
        self.assertEqual(preso.exception.envelope.category, "resource_limit")
        self.assertFalse(
            uscita.exists(),
            "la pubblicazione e' atomica: un tetto superato non lascia un file",
        )

    @unittest.skipIf(sys.platform == "win32", "l'inoltro del segnale non c'e'")
    def test_la_cancellazione_e_disponibile(self) -> None:
        """Che l'inoltro sia armato lo si puo' interrogare.

        La sonda non manda un segnale: farlo su una conversione che dura
        millisecondi sarebbe una corsa, e un test che vince a volte non e' un
        test. Che il segnale **arrivi** al prodotto lo provano le sonde della
        CLI, che hanno un processo vero da interrompere.
        """
        self.assertTrue(self.cliente.cancellable)


if __name__ == "__main__":
    unittest.main()
