"""I modelli, e che cosa fanno di una busta che non e' quella attesa.

Il principio che le sonde qui sotto muovono e' uno solo: **fail-closed**. Un
campo obbligatorio che manca e' un errore e non un `None`, perche' un `None`
trasforma un'incompatibilita' di versione in dati sbagliati piu' avanti, dove
nessuno la riconosce piu'.
"""

from __future__ import annotations

import json
import unittest
from pathlib import Path

from plenora_io import Catalog, Driver, ProtocolError, Version

RADICE = Path(__file__).resolve().parents[3]
CONTRATTO = RADICE / "release" / "cli-protocol-v2.json"


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

    @unittest.skipUnless(CONTRATTO.is_file(), "il contratto sta nel repository")
    def test_il_driver_espone_i_campi_che_il_protocollo_dichiara(self) -> None:
        attesi = {c for c, sempre in self.struttura(".drivers[]").items() if sempre}
        self.assertEqual(set(Driver.OBBLIGATORI), attesi)

    @unittest.skipUnless(CONTRATTO.is_file(), "il contratto sta nel repository")
    def test_il_catalogo_espone_i_campi_che_il_protocollo_dichiara(self) -> None:
        attesi = {c for c, sempre in self.struttura("").items() if sempre}
        self.assertEqual(set(Catalog.OBBLIGATORI), attesi)


if __name__ == "__main__":
    unittest.main()
