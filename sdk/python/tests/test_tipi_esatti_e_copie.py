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

import json
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

    def test_retry_e_diagnostica_sono_di_sola_lettura(self) -> None:
        documento = {"contract": "c", "examples": [{"column": "a"}]}
        letta = busta(details={"row_diagnostics": documento})
        with self.assertRaises(TypeError):
            letta.retry["kind"] = "safe"  # type: ignore[index]
        with self.assertRaises(TypeError):
            letta.row_diagnostics["contract"] = "altro"  # type: ignore[index]
        with self.assertRaises(TypeError):
            letta.row_diagnostics["examples"][0]["column"] = "b"  # type: ignore[index]
        documento["examples"][0]["column"] = "cambiata"
        self.assertEqual(letta.row_diagnostics["examples"][0]["column"], "a")
        # Il confronto con un dizionario resta quello di prima.
        self.assertEqual(letta.retry, {"kind": "never"})


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
