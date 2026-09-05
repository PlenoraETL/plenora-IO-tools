"""I modelli, e che cosa fanno di una busta che non e' quella attesa.

Il principio che le sonde qui sotto muovono e' uno solo: **fail-closed**. Un
campo obbligatorio che manca e' un errore e non un `None`, perche' un `None`
trasforma un'incompatibilita' di versione in dati sbagliati piu' avanti, dove
nessuno la riconosce piu'.
"""

from __future__ import annotations

import json
import unittest
from plenora_io import (
    Catalog,
    CrsResolution,
    Driver,
    Fidelity,
    FidelityReason,
    Field,
    FormatDescriptor,
    Geometry,
    Inspect,
    Layer,
    Layers,
    LayerSummary,
    Omissions,
    ProtocolError,
    Validation,
    Version,
)

from _repository import CONTRATTO, serve_il_repository


def driver_sano(**modifiche):
    documento = {campo: "x" for campo in Driver.OBBLIGATORI}
    documento.update(
        {
            "id": "geojson",
            "available": True,
            "required_feature": None,
            "direction": "bidirectional",
            "multi_layer": False,
            "multi_file": False,
            "hostile_input_hardened": True,
            "spec_version_supported": None,
            "descriptor_version": 9,
            "driver_version": 6,
            "semantic_version": 1,
            "format_options": [],
            "write_capabilities": {"attributes": "all"},
        }
    )
    documento.update(modifiche)
    return documento


def catalogo_sano(**modifiche):
    documento = {
        "status": "ok",
        "protocol_version": 2,
        "contract": "plenora-io-catalog-v2",
        "determinism": "byte_for_byte",
        "drivers": [driver_sano(), driver_sano(id="csv", available=False)],
    }
    documento.update(modifiche)
    return documento


class LaBustaDiBootstrap(unittest.TestCase):
    def test_i_due_campi(self) -> None:
        versione = Version.from_json({"status": "ok", "version": "2.0.0"})
        self.assertEqual(versione.version, "2.0.0")
        self.assertEqual(versione.status, "ok")

    def test_un_campo_mancante_e_un_errore(self) -> None:
        with self.assertRaises(ProtocolError) as preso:
            Version.from_json({"status": "ok"})
        self.assertIn("version", str(preso.exception))

    def test_un_campo_in_piu_e_un_errore(self) -> None:
        """Qui, e **solo** qui, un campo in piu' non si ignora.

        Lo schema di bootstrap e' chiuso perche' si legge prima della
        negoziazione: chi lo consuma non ha una versione su cui appoggiarsi per
        capire che cosa sia cambiato, quindi deve accorgersene subito invece di
        scoprirlo quando il campo nuovo gli serviva.
        """
        with self.assertRaises(ProtocolError) as preso:
            Version.from_json(
                {"status": "ok", "version": "2.0.0", "protocol_version": 2}
            )
        self.assertIn("protocol_version", str(preso.exception))
        self.assertIn("chiuso", str(preso.exception))


class IlCatalogo(unittest.TestCase):
    def test_letto_intero(self) -> None:
        catalogo = Catalog.from_json(catalogo_sano())
        self.assertEqual(catalogo.contract, "plenora-io-catalog-v2")
        self.assertEqual(len(catalogo.drivers), 2)
        self.assertEqual([d.id for d in catalogo.available], ["geojson"])

    def test_un_campo_di_primo_livello_mancante_e_un_errore(self) -> None:
        for campo in Catalog.OBBLIGATORI:
            with self.subTest(campo=campo):
                documento = catalogo_sano()
                del documento[campo]
                with self.assertRaises(ProtocolError):
                    Catalog.from_json(documento)

    def test_un_campo_di_driver_mancante_e_un_errore(self) -> None:
        for campo in Driver.OBBLIGATORI:
            with self.subTest(campo=campo):
                documento = driver_sano()
                del documento[campo]
                with self.assertRaises(ProtocolError) as preso:
                    Driver.from_json(documento)
                self.assertIn(campo, str(preso.exception))

    def test_un_campo_in_piu_arriva_comunque_a_chi_legge(self) -> None:
        """Le regole di compatibilita' del protocollo consentono
        `add_optional_field`, e un campo aggiunto deve poter essere letto prima
        che l'SDK lo modelli: senza `raw`, chi lo usa dovrebbe aspettare una
        nostra release per leggere qualcosa che il prodotto gli manda gia'."""
        driver = Driver.from_json(driver_sano(campo_nuovo="valore"))
        self.assertEqual(driver.raw["campo_nuovo"], "valore")

    def test_drivers_non_elenco(self) -> None:
        with self.assertRaises(ProtocolError) as preso:
            Catalog.from_json(catalogo_sano(drivers={}))
        self.assertIn("non un elenco", str(preso.exception))

    def test_un_id_che_non_c_e_solleva_invece_di_tornare_none(self) -> None:
        """Un `None` restituito diventa un `AttributeError` tre righe piu' in
        la', dove il nome sbagliato non si vede piu'."""
        catalogo = Catalog.from_json(catalogo_sano())
        with self.assertRaises(KeyError) as preso:
            catalogo.driver("gpkg")
        self.assertIn("csv", str(preso.exception))

    def test_le_due_direzioni_derivate(self) -> None:
        self.assertTrue(Driver.from_json(driver_sano()).writable)
        solo_lettura = Driver.from_json(driver_sano(direction="read_only"))
        self.assertFalse(solo_lettura.writable)
        self.assertTrue(solo_lettura.readable)



def fedelta_sana(**modifiche):
    documento = {
        "level": "conditional",
        "reasons": [{"code": "format_constraint", "detail": "il formato limita"}],
        "troncato": False,
        "omesse": {
            "categorie_omesse": 0,
            "ragioni_omesse": 0,
            "esempi_omessi": 0,
            "omesse_per_byte": 0,
        },
        "omesse_esatte": True,
    }
    documento.update(modifiche)
    return documento


def geometria_sana(**modifiche):
    documento = {
        "name": "geometry",
        "kind": "Geographic",
        "crs": "OGC:CRS84",
        "crs_resolution": {
            "id": "OGC:CRS84",
            "kind": "geographic",
            "status": "resolved",
            "axis_order": "longitude_latitude",
            "definition": None,
            "definition_format": None,
        },
    }
    documento.update(modifiche)
    return documento


def strato_sano(**modifiche):
    documento = {
        "id": 0,
        "name": "canonico",
        "fields": [
            {"name": "geometry", "type": "Binary", "nullable": True, "geometry": True},
            {"name": "codice", "type": "Utf8", "nullable": True, "geometry": False},
        ],
        "geometry": geometria_sana(),
    }
    documento.update(modifiche)
    return documento


def descrittore_sano(**modifiche):
    documento = {campo: "x" for campo in FormatDescriptor.OBBLIGATORI}
    documento.update(
        {
            "id": "geojson",
            "direction": "bidirectional",
            "multi_layer": False,
            "multi_file": False,
            "hostile_input_hardened": True,
            "spec_version_supported": None,
            "descriptor_version": 9,
            "driver_version": 6,
            "semantic_version": 1,
            "format_options": [],
            "write_capabilities": {"attributes": "all"},
        }
    )
    documento.update(modifiche)
    return documento


def inspect_sano(**modifiche):
    documento = {
        "status": "ok",
        "protocol_version": 2,
        "contract": "plenora-io-inspect-v2",
        "format": descrittore_sano(),
        "fidelity": fedelta_sana(),
        "layers": [strato_sano()],
    }
    documento.update(modifiche)
    return documento


def layers_sano(**modifiche):
    documento = {
        "status": "ok",
        "protocol_version": 2,
        "contract": "plenora-io-layers-v2",
        "format": "gpkg",
        "fidelity": fedelta_sana(),
        "layers": [
            {"id": 0, "name": "principale", "field_count": 9, "geometry_crs": "EPSG:3003"}
        ],
    }
    documento.update(modifiche)
    return documento


class LaSezioneDiFedelta(unittest.TestCase):
    def test_letta_intera(self) -> None:
        fedelta = Fidelity.from_json(fedelta_sana())
        self.assertEqual(fedelta.level, "conditional")
        self.assertEqual(len(fedelta.reasons), 1)
        self.assertFalse(fedelta.omesse.any)

    def test_esatta_vuol_dire_esatta_e_non_troncata(self) -> None:
        """Una sezione troncata non puo' dirsi esatta: le ragioni che mancano
        non si sono viste, e concludere dall'assenza di una ragione che quella
        perdita non c'e' e' l'errore che `troncato` esiste per impedire."""
        self.assertTrue(Fidelity.from_json(fedelta_sana(level="exact")).exact)
        self.assertFalse(
            Fidelity.from_json(fedelta_sana(level="exact", troncato=True)).exact
        )
        self.assertFalse(Fidelity.from_json(fedelta_sana()).exact)

    def test_le_quattro_omissioni_restano_separate(self) -> None:
        """Portano a decisioni diverse: chi ha perso categorie non conosce tutti
        i tipi di perdita, chi ha perso esempi li conosce e non ne ha campioni."""
        for causa in Omissions.OBBLIGATORI:
            with self.subTest(causa=causa):
                omesse = dict(fedelta_sana()["omesse"])
                omesse[causa] = 3
                fedelta = Fidelity.from_json(fedelta_sana(omesse=omesse))
                self.assertTrue(fedelta.omesse.any)
                self.assertEqual(getattr(fedelta.omesse, causa), 3)

    def test_una_ragione_localizzata_porta_i_due_indici(self) -> None:
        """Vengono insieme o non vengono: una ragione che nomina un campo nomina
        anche il layer in cui sta."""
        generica = FidelityReason.from_json({"code": "c", "detail": "d"})
        self.assertFalse(generica.localized)
        self.assertIsNone(generica.field_index)

        precisa = FidelityReason.from_json(
            {"code": "c", "detail": "d", "field_index": 2, "layer_index": 0}
        )
        self.assertTrue(precisa.localized)
        self.assertEqual((precisa.field_index, precisa.layer_index), (2, 0))

    def test_un_campo_mancante_e_un_errore(self) -> None:
        for campo in Fidelity.OBBLIGATORI:
            with self.subTest(campo=campo):
                documento = fedelta_sana()
                del documento[campo]
                with self.assertRaises(ProtocolError):
                    Fidelity.from_json(documento)

    def test_reasons_non_elenco(self) -> None:
        with self.assertRaises(ProtocolError) as preso:
            Fidelity.from_json(fedelta_sana(reasons={}))
        self.assertIn("non un elenco", str(preso.exception))


class LaBustaDiInspect(unittest.TestCase):
    def test_letta_intera(self) -> None:
        esito = Inspect.from_json(inspect_sano())
        self.assertEqual(esito.format.id, "geojson")
        self.assertEqual(esito.layers[0].name, "canonico")
        self.assertEqual(esito.layers[0].geometry.crs_resolution.status, "resolved")

    def test_gli_attributi_sono_i_campi_meno_la_geometria(self) -> None:
        strato = Inspect.from_json(inspect_sano()).layers[0]
        self.assertEqual([c.name for c in strato.attributes], ["codice"])
        self.assertEqual(len(strato.fields), 2)

    def test_un_campo_che_non_c_e_solleva_ed_elenca_quelli_che_ci_sono(self) -> None:
        strato = Inspect.from_json(inspect_sano()).layers[0]
        with self.assertRaises(KeyError) as preso:
            strato.field("inesistente")
        self.assertIn("codice", str(preso.exception))

    def test_un_layer_che_non_c_e_solleva(self) -> None:
        esito = Inspect.from_json(inspect_sano())
        with self.assertRaises(KeyError) as preso:
            esito.layer("altro")
        self.assertIn("canonico", str(preso.exception))

    def test_ogni_campo_obbligatorio_mancante_e_un_errore(self) -> None:
        for campo in Inspect.OBBLIGATORI:
            with self.subTest(campo=campo):
                documento = inspect_sano()
                del documento[campo]
                with self.assertRaises(ProtocolError):
                    Inspect.from_json(documento)

    def test_l_errore_dice_dove_manca_il_campo(self) -> None:
        """Un campo mancante in fondo all'albero non deve produrre un messaggio
        che parla della busta intera: chi lo legge deve sapere dove guardare."""
        documento = inspect_sano()
        del documento["layers"][0]["geometry"]["crs_resolution"]["axis_order"]
        with self.assertRaises(ProtocolError) as preso:
            Inspect.from_json(documento)
        self.assertIn("crs_resolution", str(preso.exception))
        self.assertIn("axis_order", str(preso.exception))

    def test_un_campo_in_piu_resta_leggibile(self) -> None:
        esito = Inspect.from_json(inspect_sano(campo_futuro=1))
        self.assertEqual(esito.raw["campo_futuro"], 1)


class LaBustaDiLayers(unittest.TestCase):
    def test_letta_intera(self) -> None:
        esito = Layers.from_json(layers_sano())
        self.assertEqual(esito.format, "gpkg")
        self.assertEqual(esito.layers[0].field_count, 9)

    def test_il_riassunto_non_promette_uno_schema(self) -> None:
        """`LayerSummary` non e' un `Layer` incompleto: un modello che
        promettesse `fields` vuoti farebbe credere a un layer senza colonne."""
        riassunto = Layers.from_json(layers_sano()).layers[0]
        self.assertIsInstance(riassunto, LayerSummary)
        self.assertFalse(hasattr(riassunto, "fields"))

    def test_format_e_una_stringa_qui_e_un_oggetto_in_inspect(self) -> None:
        """I due campi hanno lo stesso nome e tipi diversi, ed e' il wire a
        volerlo: modellarli uguali avrebbe richiesto di inventare un descrittore
        che questa busta non porta."""
        self.assertIsInstance(Layers.from_json(layers_sano()).format, str)
        self.assertIsInstance(
            Inspect.from_json(inspect_sano()).format, FormatDescriptor
        )

    def test_ogni_campo_obbligatorio_mancante_e_un_errore(self) -> None:
        for campo in Layers.OBBLIGATORI:
            with self.subTest(campo=campo):
                documento = layers_sano()
                del documento[campo]
                with self.assertRaises(ProtocolError):
                    Layers.from_json(documento)



def validazione_sana(**modifiche):
    documento = {
        "status": "ok",
        "protocol_version": 2,
        "contract": "plenora-io-read-v2",
        "format": "geojson",
        "layer": strato_sano(),
        "rows_read": 5,
        "batches": 1,
        "truncated": False,
        "fidelity": fedelta_sana(),
    }
    documento.update(modifiche)
    return documento


class LaBustaDiValidate(unittest.TestCase):
    def test_letta_intera(self) -> None:
        esito = Validation.from_json(validazione_sana())
        self.assertEqual(esito.rows_read, 5)
        self.assertEqual(esito.batches, 1)
        self.assertEqual(esito.layer.name, "canonico")
        self.assertTrue(esito.complete)

    def test_completo_e_il_contrario_di_troncato(self) -> None:
        """Il nome positivo e' quello che si scrive in un `if`: `if complete`
        invece di `if not truncated`, che si legge male e si nega peggio."""
        self.assertFalse(Validation.from_json(validazione_sana(truncated=True)).complete)

    def test_non_porta_righe(self) -> None:
        """La busta conta e non consegna: se un giorno portasse dati, questo
        modello li butterebbe via in silenzio e la sonda lo direbbe."""
        esito = Validation.from_json(validazione_sana())
        self.assertFalse(hasattr(esito, "rows"))
        self.assertFalse(hasattr(esito, "data"))
        self.assertNotIn("rows", esito.raw)

    def test_ogni_campo_obbligatorio_mancante_e_un_errore(self) -> None:
        for campo in Validation.OBBLIGATORI:
            with self.subTest(campo=campo):
                documento = validazione_sana()
                del documento[campo]
                with self.assertRaises(ProtocolError):
                    Validation.from_json(documento)


class IModelliSeguonoIlContratto(unittest.TestCase):
    """La controprova che il gate `check_sdk_python.py` fa da fuori.

    Vive anche qui perche' l'SDK dev'essere verificabile da chi lo installa,
    senza gli script di questo repository: `sdk/python` e' un pacchetto, e un
    pacchetto porta con se' le prove di cio' che afferma.
    """

    def struttura(self, prefisso: str) -> dict[str, bool]:
        manifesto = json.loads(CONTRATTO.read_text(encoding="utf-8"))
        struttura = manifesto["envelopes"]["catalog"]["struttura"]
        fuori = {}
        for percorso, voce in struttura.items():
            if not percorso.startswith(prefisso + "."):
                continue
            resto = percorso[len(prefisso) + 1 :]
            if "." in resto or "[]" in resto or "{}" in resto:
                continue
            fuori[resto] = bool(voce["sempre"])
        return fuori

    @serve_il_repository
    def test_il_driver_espone_i_campi_che_il_protocollo_dichiara(self) -> None:
        """`Driver` e' un `FormatDescriptor` piu' i due campi del catalogo.

        La somma delle due tuple e' cio' che il protocollo dichiara sotto
        `.drivers[]`: e' la stessa affermazione che il gate fa da fuori, e vive
        anche qui perche' l'SDK dev'essere verificabile da chi lo installa,
        senza gli script di questo repository.
        """
        attesi = {c for c, sempre in self.struttura(".drivers[]").items() if sempre}
        self.assertEqual(set(FormatDescriptor.OBBLIGATORI) | set(Driver.PROPRI), attesi)

    @serve_il_repository
    def test_il_descrittore_di_inspect_e_quello_del_catalogo(self) -> None:
        """I due campi del catalogo sono **esattamente** la differenza.

        Se un giorno `inspect` portasse un campo che il catalogo non ha, questo
        modello unico direbbe una cosa falsa: sarebbero due tipi diversi con lo
        stesso nome.
        """
        manifesto = json.loads(CONTRATTO.read_text(encoding="utf-8"))

        def immediati(busta: str, prefisso: str) -> set[str]:
            struttura = manifesto["envelopes"][busta]["struttura"]
            fuori = set()
            for percorso in struttura:
                if not percorso.startswith(prefisso + "."):
                    continue
                resto = percorso[len(prefisso) + 1 :]
                if "." not in resto and "[]" not in resto:
                    fuori.add(resto)
            return fuori

        del_catalogo = immediati("catalog", ".drivers[]")
        di_inspect = immediati("inspect", ".format")
        self.assertEqual(del_catalogo - di_inspect, set(Driver.PROPRI))
        self.assertEqual(di_inspect, set(FormatDescriptor.OBBLIGATORI))

    @serve_il_repository
    def test_il_catalogo_espone_i_campi_che_il_protocollo_dichiara(self) -> None:
        attesi = {c for c, sempre in self.struttura("").items() if sempre}
        self.assertEqual(set(Catalog.OBBLIGATORI), attesi)


if __name__ == "__main__":
    unittest.main()
