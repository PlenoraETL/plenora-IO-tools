"""Cio' che l'SDK valida e' cio' che conserva, e solo tipi JSON esatti.

Due classi di difetto, trovate rileggendo la validazione della busta:

* **aliasing** -- la busta conservava il dizionario `retry` del chiamante.
  Validata con `remote_effect: unknown` e `retry: {kind: never}`, bastava
  cambiare dopo quel dizionario in `{kind: safe}` perche' `retryable`
  diventasse vero senza nessuna nuova validazione. Lo stesso valeva per i
  modelli, che condividevano elenchi e oggetti col documento d'origine;
* **sottoclassi** -- i controlli usavano `isinstance`. Una sottoclasse di `str`
  con `__eq__`/`__hash__` riscritti superava `in EFFETTI_REMOTI` fingendosi
  `none`; una di `dict` con `get` riscritto rispondeva a `kind` con un valore
  che non aveva.

Le sonde non eseguono un binario e girano ovunque.
"""

from __future__ import annotations

import copy
import dataclasses
import json
import pickle
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

from plenora_io import ErrorEnvelope, Field, ManifestError, ProtocolError
from plenora_io.discovery import Manifest, leggi_manifesto
from plenora_io.errors import carica_json, failure_from_envelope
from plenora_io.models import FormatDescriptor


class StrBugiarda(str):
    """Una stringa che si dichiara uguale a `none` qualunque cosa contenga."""

    def __eq__(self, altro: object) -> bool:
        return altro == "none" or str.__eq__(self, altro)

    def __hash__(self) -> int:
        return hash("none")


class DictBugiardo(dict):
    """Un dizionario che a `get("kind")` risponde `never`."""

    def get(self, chiave, predefinito=None):
        return "never" if chiave == "kind" else super().get(chiave, predefinito)


def errore(**campi) -> dict:
    base = {
        "code": "X",
        "category": "internal",
        "phase": "read",
        "remote_effect": "unknown",
        "retry": {"kind": "never"},
        "message": "m",
    }
    base.update(campi)
    return base


def busta(**campi) -> ErrorEnvelope:
    return ErrorEnvelope.from_json({"status": "error", "error": errore(**campi)})


class LaBustaNonCondivideNiente(unittest.TestCase):
    def test_cambiare_il_retry_del_chiamante_non_cambia_la_busta(self) -> None:
        """Il difetto riprodotto: prima `retryable` diventava vero."""
        retry = {"kind": "never"}
        documento = {"status": "error", "error": errore(retry=retry)}
        fallito = failure_from_envelope(documento, 5, ["x"])
        retry["kind"] = "safe"
        documento["error"]["retry"] = {"kind": "safe"}
        self.assertFalse(fallito.retryable)
        self.assertEqual(fallito.envelope.retry["kind"], "never")

    def test_anche_la_costruzione_diretta_copia(self) -> None:
        retry = {"kind": "never"}
        costruita = ErrorEnvelope(
            code="X",
            category="io",
            phase="read",
            remote_effect="unknown",
            retry=retry,
            message="m",
        )
        retry["kind"] = "safe"
        self.assertEqual(costruita.retry["kind"], "never")
        self.assertIsNot(costruita.retry, retry)

    def test_cambiare_la_copia_pubblica_non_cambia_le_decisioni(self) -> None:
        """`retry` e' una copia ordinaria; le decisioni stanno altrove."""
        fallito = failure_from_envelope(
            {"status": "error", "error": errore(retry={"kind": "never"})}, 5, ["x"]
        )
        fallito.envelope.retry["kind"] = "safe"
        fallito.envelope.retry["delay_ms"] = 1
        self.assertFalse(fallito.retryable)
        self.assertIsNone(fallito.retry_after_ms)
        self.assertTrue(fallito.must_assume_remote_committed)

    def test_la_diagnostica_resta_un_documento_uguale_all_originale(self) -> None:
        # Niente tuple al posto degli elenchi: il confronto con il documento
        # d'origine resta vero, e la copia non e' l'originale.
        documento = {"contract": "c", "examples": [{"column": "a"}]}
        letta = busta(details={"row_diagnostics": documento})
        self.assertEqual(letta.row_diagnostics, documento)
        self.assertIsNot(letta.row_diagnostics["examples"], documento["examples"])
        documento["examples"][0]["column"] = "cambiata"
        self.assertEqual(letta.row_diagnostics["examples"][0]["column"], "a")


class LaBustaSiCopiaESiSerializza(unittest.TestCase):
    """`deepcopy`, `pickle`, `asdict` e `replace` funzionano come prima."""

    def busta_piena(self) -> ErrorEnvelope:
        return busta(
            remote_effect="partial",
            retry={"kind": "after", "delay_ms": 10},
            details={"row_diagnostics": {"examples": [{"column": "a"}]}},
        )

    def decisioni(self, letta: ErrorEnvelope) -> tuple:
        fallito = failure_from_envelope(
            {"status": "error", "error": errore(
                remote_effect=letta.remote_effect, retry=dict(letta.retry)
            )},
            5,
            ["x"],
        )
        return (letta._tipo_di_ritentativo, letta._ritardo_ms, letta._effetto_remoto,
                fallito.retryable, fallito.retry_after_ms,
                fallito.must_assume_remote_committed)

    def test_deepcopy(self) -> None:
        originale = self.busta_piena()
        copia = copy.deepcopy(originale)
        self.assertEqual(copia, originale)
        self.assertIsNot(copia.retry, originale.retry)
        self.assertEqual(self.decisioni(copia), self.decisioni(originale))

    def test_pickle_andata_e_ritorno(self) -> None:
        originale = self.busta_piena()
        tornata = pickle.loads(pickle.dumps(originale))
        self.assertEqual(tornata, originale)
        self.assertEqual(tornata._tipo_di_ritentativo, "after")
        self.assertEqual(tornata._ritardo_ms, 10)
        self.assertEqual(tornata._effetto_remoto, "partial")

    def test_asdict_rende_i_soli_campi_pubblici(self) -> None:
        self.assertEqual(
            dataclasses.asdict(self.busta_piena()),
            {
                "code": "X",
                "category": "internal",
                "phase": "read",
                "remote_effect": "partial",
                "retry": {"kind": "after", "delay_ms": 10},
                "message": "m",
                "row_diagnostics": {"examples": [{"column": "a"}]},
            },
        )

    def test_replace_ricostruisce_e_rivalida(self) -> None:
        originale = self.busta_piena()
        self.assertEqual(dataclasses.replace(originale), originale)
        cambiata = dataclasses.replace(originale, retry={"kind": "never"})
        self.assertEqual(cambiata._tipo_di_ritentativo, "never")
        self.assertIsNone(cambiata._ritardo_ms)
        with self.assertRaises(ProtocolError):
            dataclasses.replace(
                originale, remote_effect="unknown", retry={"kind": "safe"}
            )


class MetaBugiarda(type):
    """Una metaclasse che fa dire al tipo di essere `str`."""

    def __eq__(cls, altro: object) -> bool:
        return altro is str or type.__eq__(cls, altro)

    def __hash__(cls) -> int:
        return hash(str)


class Camaleonte(str, metaclass=MetaBugiarda):
    """Una stringa il cui valore di confronto cambia dopo la costruzione."""

    def __new__(cls, valore: str):
        istanza = super().__new__(cls, valore)
        istanza.valore = valore
        return istanza

    def __eq__(self, altro: object) -> bool:
        return self.valore == altro

    def __hash__(self) -> int:
        return hash(self.valore)


class IlTipoSiConfrontaPerIdentita(unittest.TestCase):
    def test_una_metaclasse_che_finge_str_non_passa(self) -> None:
        """Il caso riprodotto: `retryable` passava da False a True.

        Il tipo del valore si diceva uguale a `str` a un dizionario di tipi; la
        copia teneva la foglia del chiamante, e cambiarne il valore dopo
        cambiava la decisione.
        """
        self.assertEqual({str: "string"}.get(Camaleonte), "string", "la premessa")
        for campi in (
            {"retry": {"kind": Camaleonte("never")}},
            {"remote_effect": Camaleonte("unknown")},
            {"code": Camaleonte("X")},
        ):
            with self.subTest(campi=campi):
                with self.assertRaises(ProtocolError):
                    busta(**campi)

    def test_anche_modelli_e_manifesto(self) -> None:
        sano = {"name": "a", "type": "Utf8", "nullable": True, "geometry": False}
        with self.assertRaises(ProtocolError):
            Field.from_json({**sano, "name": Camaleonte("a")})
        with self.assertRaises(ManifestError):
            Manifest.from_json({**MANIFESTO, "profilo": Camaleonte("base")})


class TuttoIlDocumentoETipizzato(unittest.TestCase):
    def test_i_rami_ignorati_sono_controllati_lo_stesso(self) -> None:
        for documento in (
            {"status": "error", "error": errore(details={"altro": object()})},
            {"status": "error", "error": errore(), "extra": [object()]},
            {"status": "error", "error": errore(), StrBugiarda("chiave"): 1},
            {"status": "error", "error": errore(details={StrBugiarda("k"): 1})},
        ):
            with self.subTest(documento=documento):
                with self.assertRaises(ProtocolError):
                    ErrorEnvelope.from_json(documento)

    def test_i_messaggi_non_portano_le_chiavi_del_documento(self) -> None:
        for campi in (
            {"details": {"row_diagnostics": {"PRIVATE_COLUMN": object()}}},
            {"details": {"row_diagnostics": {"PRIVATE_COLUMN": [float("inf")]}}},
            {"retry": {"kind": "never", "PRIVATE_COLUMN": 1}},
        ):
            with self.subTest(campi=campi):
                with self.assertRaises(ProtocolError) as preso:
                    busta(**campi)
                self.assertNotIn("PRIVATE_COLUMN", str(preso.exception))
        with self.assertRaises(ProtocolError) as preso:
            ErrorEnvelope.from_json({"PRIVATE_COLUMN": 1})
        self.assertNotIn("PRIVATE_COLUMN", str(preso.exception))


class SoloTipiJsonEsatti(unittest.TestCase):
    def test_una_stringa_che_finge_none_non_passa(self) -> None:
        bugiarda = StrBugiarda("unknown")
        self.assertIn(bugiarda, {"none"}, "la premessa: con `in` passava")
        with self.assertRaises(ProtocolError):
            busta(remote_effect=bugiarda)

    def test_un_dizionario_che_mente_su_kind_non_passa(self) -> None:
        bugiardo = DictBugiardo(kind="safe")
        self.assertEqual(bugiardo.get("kind"), "never", "la premessa")
        with self.assertRaises(ProtocolError):
            busta(retry=bugiardo)

    def test_sottoclassi_ovunque_nella_busta(self) -> None:
        for campi in (
            {"retry": {"kind": StrBugiarda("never")}},
            {"retry": {StrBugiarda("kind"): "never"}},
            {"code": StrBugiarda("X")},
            {"details": DictBugiardo()},
            {"details": {"row_diagnostics": DictBugiardo()}},
            {"details": {"row_diagnostics": {"examples": [DictBugiardo()]}}},
            {"remote_effect": "none", "retry": {"kind": "after", "delay_ms": True}},
        ):
            with self.subTest(campi=campi):
                with self.assertRaises(ProtocolError):
                    busta(**campi)

    def test_la_busta_esterna_sottoclasse_non_passa(self) -> None:
        with self.assertRaises(ProtocolError):
            ErrorEnvelope.from_json(DictBugiardo(status="error", error=errore()))

    def test_i_modelli_rifiutano_le_sottoclassi(self) -> None:
        sano = {"name": "a", "type": "Utf8", "nullable": True, "geometry": False}
        Field.from_json(sano)
        for documento in (
            DictBugiardo(sano),
            {**sano, "name": StrBugiarda("a")},
            {**sano, "extra": {"annidato": StrBugiarda("x")}},
            {**sano, "extra": float("nan")},
        ):
            with self.subTest(documento=documento):
                with self.assertRaises(ProtocolError):
                    Field.from_json(documento)


def descrittore() -> dict:
    documento = {campo: "x" for campo in FormatDescriptor.OBBLIGATORI}
    documento.update(
        {
            "multi_layer": False,
            "multi_file": False,
            "hostile_input_hardened": True,
            "spec_version_supported": None,
            "descriptor_version": 1,
            "driver_version": 1,
            "semantic_version": 1,
            "format_options": [{"key": "k"}],
            "recognised_suffixes": [".x"],
            "write_capabilities": {"attributes": "all"},
        }
    )
    return documento


class IModelliNonCondividonoNiente(unittest.TestCase):
    def test_cambiare_il_documento_dopo_non_cambia_il_modello(self) -> None:
        documento = descrittore()
        modello = FormatDescriptor.from_json(documento)
        documento["format_options"][0]["key"] = "cambiata"
        documento["recognised_suffixes"].append(".y")
        documento["write_capabilities"]["attributes"] = "none"
        self.assertEqual(modello.format_options, [{"key": "k"}])
        self.assertEqual(modello.recognised_suffixes, [".x"])
        self.assertEqual(modello.write_capabilities, {"attributes": "all"})
        self.assertEqual(modello.raw["write_capabilities"], {"attributes": "all"})


class IlJsonDelFilo(unittest.TestCase):
    def test_chiavi_ripetute_e_costanti_non_json_sono_rifiutate(self) -> None:
        # `json.loads` terrebbe l'ultima chiave, e accetta NaN/Infinity.
        self.assertEqual(json.loads('{"kind": "never", "kind": "safe"}'), {"kind": "safe"})
        for testo in (
            '{"retry": {"kind": "never", "kind": "safe"}}',
            '{"a": NaN}',
            '{"a": Infinity}',
            '[-Infinity]',
            '{"a": 1e400}',
            '[-1e400]',
        ):
            with self.subTest(testo=testo):
                with self.assertRaises(ProtocolError):
                    carica_json(testo)
        self.assertEqual(carica_json('{"a": [1, 2.5, null]}'), {"a": [1, 2.5, None]})


MANIFESTO = {
    "nome": "plenora-io",
    "versione": "1.0.0",
    "piattaforma": "linux-x86_64",
    "profilo": "base",
    "canale": "candidate",
    "non_release": False,
    "revisione": "a" * 40,
}


class IlManifesto(unittest.TestCase):
    def test_non_release_che_non_e_booleano_non_dichiara_una_release(self) -> None:
        # `release = not documento["non_release"]` dava True per `null` e
        # False per la stringa "false".
        for valore in (None, "false", 0, StrBugiarda("x")):
            with self.subTest(valore=valore):
                with self.assertRaises(ManifestError):
                    Manifest.from_json({**MANIFESTO, "non_release": valore})

    def test_i_campi_testuali_sono_stringhe_esatte(self) -> None:
        for campo in ("nome", "versione", "piattaforma", "profilo", "canale"):
            for valore in (None, 1, StrBugiarda("base")):
                with self.subTest(campo=campo, valore=valore):
                    with self.assertRaises(ManifestError):
                        Manifest.from_json({**MANIFESTO, campo: valore})
        with self.assertRaises(ManifestError):
            Manifest.from_json({**MANIFESTO, "revisione": 7})
        self.assertIsNone(Manifest.from_json({**MANIFESTO, "revisione": None}).revision)
        self.assertTrue(Manifest.from_json(MANIFESTO).release)

    def test_un_manifesto_con_chiavi_ripetute_e_rotto(self) -> None:
        with TemporaryDirectory() as cartella:
            radice = Path(cartella)
            (radice / "bin").mkdir()
            binario = radice / "bin" / "plenora-io"
            binario.write_text("", encoding="utf-8")
            testo = json.dumps(MANIFESTO)[:-1] + ', "profilo": "filegdb"}'
            (radice / "MANIFEST.json").write_text(testo, encoding="utf-8")
            with self.assertRaises(ManifestError):
                leggi_manifesto(binario)


if __name__ == "__main__":
    unittest.main()
