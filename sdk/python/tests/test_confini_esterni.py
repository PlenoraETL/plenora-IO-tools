"""Ogni punto in cui puo' nascere un'eccezione esterna, e la sua traduzione.

Seconda revisione di #44. PYTHON-SDK-1.0 §6: nessuna eccezione esterna
attraversa il confine pubblico, e nessuna resta nella catena. Le prove sono di
due forme:

* **una per punto**, con il guasto provocato esattamente dove nasce -- il
  footer IPC che non si chiude, `hasattr` che solleva, il JSON troppo profondo,
  la consegna di `read_table()` che non si legge, il `Popen` che rifiuta un
  NUL, il manifesto che non e' UTF-8;
* **parametriche**, sui metodi pubblici e sui modelli: ogni argomento sostituito
  da valori ostili, ogni campo di ogni busta sostituito da tipi sbagliati. Cio'
  che esce e' un `PlenoraError` con gli assi giusti, o un risultato; mai
  un'eccezione di Python, di pyarrow o del sistema.

L'inventario dei punti, con il modulo e la traduzione, e' `PUNTI` qui sotto:
una riga per ciascuno, e una prova che lo esercita.
"""

from __future__ import annotations

import copy
import functools
import json
import os
import subprocess
import sys
import tempfile
import traceback
import unittest
from datetime import timedelta
from pathlib import Path
from unittest import mock

from plenora_io import (
    FrozenDict,
    FrozenList,
    Capabilities,
    Catalog,
    Client,
    CommandFailed,
    ConvertResult,
    Inspect,
    InvalidArgumentError,
    Layers,
    Limits,
    LocalIoError,
    ManifestError,
    OptionalDependencyError,
    PlenoraError,
    ProtocolError,
    Validation,
    Version,
    WriteResult,
)
from plenora_io import arrow as adattatore
from plenora_io import discovery
from plenora_io.errors import _copia_json, failure_from_envelope
from plenora_io.process import Runner

from test_contratto_sdk import capacita_sane, pa, scrittura_sana, serve_pyarrow
from test_convert import conversione_sana
from test_models import catalogo_sano, consegna_sana, inspect_sano, layers_sano, validazione_sana

SEGRETO = "SEGRETO-9c1e"

#: L'inventario: dove nasce un'eccezione esterna, e in che cosa si traduce.
#: Una riga per punto. Un punto conta come provato solo quando una sua prova e'
#: **eseguita e riuscita** (non quando e' definita, e non quando salta): con
#: `PLENORA_INVENTARIO_STRETTO=1` -- lo impostano i job della CI che hanno
#: pyarrow -- `tearDownModule` fallisce se un punto resta senza prova eseguita.
PUNTI: dict[str, tuple[str, str]] = {
    "process.Popen": ("ValueError/TypeError/OSError prima di partire", "InvalidArgumentError o ProtocolError, effetto none"),
    "process.communicate": ("OSError, ValueError, TimeoutExpired", "ProtocolError, unknown se scrive"),
    "process.decode": ("UnicodeDecodeError dei flussi", "ProtocolError, unknown se scrive"),
    "errors.carica_json": ("JSONDecodeError, RecursionError, ValueError", "ProtocolError, unknown se scrive"),
    "errors.copia_json": ("RecursionError su un documento profondo", "ProtocolError"),
    "models.from_json": ("qualunque tipo sbagliato in un campo", "ProtocolError, unknown se scrive"),
    "client.capabilities": ("documento malformato prima dell'arricchimento", "ProtocolError"),
    "client.argomenti": ("__fspath__, __str__, NUL, tipi sbagliati", "InvalidArgumentError, effetto none"),
    "client.read_table": ("consegna assente o illeggibile dopo la scrittura", "ProtocolError, unknown"),
    "arrow.scrivi_ipc": ("hasattr, from_stream, batch, footer, chiusura", "InvalidArgumentError o LocalIoError"),
    "arrow.cartella": ("fspath, mkdtemp, rmtree", "InvalidArgumentError, LocalIoError, CleanupError"),
    "discovery.binario": ("__fspath__, is_file, resolve", "InvalidArgumentError o LocalIoError"),
    "discovery.manifesto": ("OSError, UnicodeDecodeError, JSON", "ManifestError, catena tagliata"),
    "discovery.which": ("shutil.which che solleva", "LocalIoError"),
    "argomenti.sottoclassi": ("__str__, __format__, __int__, Limits e timedelta derivati", "copie esatte, nessuna chiamata dopo il confine"),
    "argomenti.grandezze": ("10**1000, 10**50000 in interi, limiti e timeout", "InvalidArgumentError prima di ogni conversione"),
    "process.exit": ("__exit__ del context manager di Popen", "ProtocolError, unknown se scrive"),
    "arrow.import": ("import di pyarrow e lettura di __version__", "OptionalDependencyError"),
    "version.metadati": ("PackageNotFoundError di importlib.metadata", "PackageMetadataError"),
    "confine.rete": ("qualunque eccezione non tradotta in un metodo pubblico", "UnexpectedError, none o unknown"),
    "messaggi.senza_dati": ("ricerche nei modelli, scoperta, profilo", "nessun nome, percorso o valore nel messaggio"),
}

PROVATI: set[str] = set()


def prova(punto: str):
    """Registra `punto` dell'inventario quando il test e' eseguito e riesce.

    Prima lo registrava alla definizione: un test saltato -- senza pyarrow,
    per esempio -- lasciava il punto «provato» senza aver provato niente.
    """
    assert punto in PUNTI, punto

    def decora(funzione):
        @functools.wraps(funzione)
        def eseguita(caso, *argomenti, **opzioni):
            esito = funzione(caso, *argomenti, **opzioni)
            # Un `subTest` fallito non solleva fuori dal test: il test torna
            # normalmente e unittest registra il fallimento dopo. Si registra
            # il punto solo se l'esito del test e' ancora un successo; se
            # unittest non espone l'esito, non si registra (chiuso, non aperto).
            esito_del_caso = getattr(caso, "_outcome", None)
            if esito_del_caso is not None and getattr(esito_del_caso, "success", False):
                PROVATI.add(punto)
            return esito

        return eseguita

    return decora


def tearDownModule() -> None:  # noqa: N802 - il nome lo impone unittest
    if os.environ.get("PLENORA_INVENTARIO_STRETTO") != "1":
        return
    mancanti = sorted(set(PUNTI) - PROVATI)
    if mancanti:
        raise AssertionError(
            f"punti dell'inventario senza una prova eseguita e riuscita: {mancanti}"
        )


def tradotto(caso: unittest.TestCase, errore: BaseException, effetto: str | None = None) -> None:
    """Un `PlenoraError` senza dati nel messaggio, nel traceback o nella catena."""
    caso.assertIsInstance(errore, PlenoraError, f"{type(errore).__name__} attraversa il confine")
    caso.assertNotIn(SEGRETO, str(errore))
    testo = "".join(traceback.format_exception(type(errore), errore, errore.__traceback__))
    caso.assertNotIn(SEGRETO, testo)
    caso.assertIsNone(errore.__cause__)
    caso.assertTrue(errore.__suppress_context__ or errore.__context__ is None,
                    "la catena implicita non e' tagliata")
    caso.assertEqual(errore.retry, {"kind": "never"})
    if effetto is not None:
        caso.assertEqual(errore.remote_effect, effetto)


class PopenFinto:
    """Un `Popen` che rende i flussi dati, e conta le partenze."""

    def __init__(self, stdout=b"", stderr=b"", codice=0):
        self.stdout, self.stderr, self.returncode = stdout, stderr, codice
        self.pid = 4242
        self.partenze = 0

    def __call__(self, argv, **opzioni):
        self.partenze += 1
        return self

    def __enter__(self):
        return self

    def __exit__(self, *eccezione):
        return False

    def communicate(self, timeout=None):
        return self.stdout, self.stderr

    def kill(self):
        pass

    def send_signal(self, segnale):
        pass


SCRIVE = ["write", f"/dati/{SEGRETO}.arrow", "/uscita/o.gpkg", "--to", "gpkg"]


# --- 1. l'adattatore Arrow -------------------------------------------------


@serve_pyarrow
class LAdattatore(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp())

    @prova("arrow.scrivi_ipc")
    def test_un_footer_che_non_si_scrive_e_local_io_error(self) -> None:
        """Prima: `with uscita, scrittore` fuori dai try, e l'errore della
        chiusura -- disco pieno sul footer -- usciva come eccezione di pyarrow."""
        vero = pa.ipc.new_file

        class Scrittore:
            def __init__(self, *argomenti):
                self._vero = vero(*argomenti)

            def write_batch(self, batch):
                self._vero.write_batch(batch)

            def close(self):
                raise OSError(28, f"No space left on device: {SEGRETO}")

            def __enter__(self):
                return self

            def __exit__(self, *eccezione):
                self.close()

        with mock.patch.object(pa.ipc, "new_file", Scrittore):
            with self.assertRaises(PlenoraError) as preso:
                adattatore.scrivi_ipc(pa.table({"x": [1, 2]}), self.tmp)
        self.assertIsInstance(preso.exception, LocalIoError)
        tradotto(self, preso.exception, "none")

    @prova("arrow.scrivi_ipc")
    def test_una_chiusura_fallita_non_copre_l_errore_gia_tradotto(self) -> None:
        schema = pa.schema([("x", pa.int64())])

        def batch():
            yield pa.record_batch([pa.array([1])], schema=schema)
            raise ValueError(SEGRETO)

        vero = pa.ipc.new_file

        class Scrittore:
            def __init__(self, *argomenti):
                self._vero = vero(*argomenti)

            def write_batch(self, batch):
                self._vero.write_batch(batch)

            def close(self):
                self._vero.close()
                raise OSError(SEGRETO)

            def __enter__(self):
                return self

            def __exit__(self, *eccezione):
                self.close()

        lettore = pa.RecordBatchReader.from_batches(schema, batch())
        with mock.patch.object(pa.ipc, "new_file", Scrittore):
            with self.assertRaises(PlenoraError) as preso:
                adattatore.scrivi_ipc(lettore, self.tmp)
        self.assertIsInstance(preso.exception, InvalidArgumentError,
                              "resta l'errore della sorgente, non quello della chiusura")
        tradotto(self, preso.exception, "none")

    @prova("arrow.scrivi_ipc")
    def test_hasattr_che_solleva_e_invalid_argument(self) -> None:
        """Prima: `hasattr` lascia passare tutto cio' che non e' `AttributeError`."""

        class Ostile:
            def __getattr__(self, nome):
                raise RuntimeError(SEGRETO)

        with self.assertRaises(PlenoraError) as preso:
            adattatore.scrivi_ipc(Ostile(), self.tmp)
        self.assertIsInstance(preso.exception, InvalidArgumentError)
        tradotto(self, preso.exception, "none")

    @prova("arrow.cartella")
    def test_una_temp_dir_ostile_e_invalid_argument(self) -> None:
        class Rotto:
            def __fspath__(self):
                raise RuntimeError(SEGRETO)

        for valore in (Rotto(), 12, f"a\x00{SEGRETO}"):
            with self.subTest(valore=type(valore).__name__):
                with self.assertRaises(PlenoraError) as preso:
                    with adattatore.cartella_temporanea(valore):
                        pass
                self.assertIsInstance(preso.exception, InvalidArgumentError)
                tradotto(self, preso.exception, "none")


# --- 2. capabilities -------------------------------------------------------


class RunnerFinto:
    def __init__(self, documento):
        self.documento = documento
        self.chiamate = []

    def run(self, argv):
        # Il documento com'e', senza `deepcopy`: quello profondo non si copia.
        self.chiamate.append(list(argv))
        return self.documento


def cliente_con(documento) -> Client:
    cliente = Client.__new__(Client)
    cliente._runner = RunnerFinto(documento)
    return cliente


class LeCapacita(unittest.TestCase):
    @prova("client.capabilities")
    def test_un_documento_malformato_e_protocol_error_prima_dell_arricchimento(self) -> None:
        """Prima: `"operations": 1` o `"id": []` davano `TypeError` dentro
        `con_superficie_python`, che girava prima della validazione."""
        casi = {
            "operations intero": dict(capacita_sane(), operations=1),
            "id lista": dict(capacita_sane(), operations=[dict(capacita_sane()["operations"][0], id=[])]),
            "id oggetto": dict(capacita_sane(), operations=[dict(capacita_sane()["operations"][0], id={})]),
            "interfaces stringa": dict(capacita_sane(), interfaces="cli"),
            "surfaces stringa": dict(capacita_sane(), operations=[dict(capacita_sane()["operations"][0], surfaces="cli")]),
        }
        for nome, documento in casi.items():
            with self.subTest(nome):
                with self.assertRaises(PlenoraError) as preso:
                    cliente_con(documento).capabilities()
                self.assertIsInstance(preso.exception, ProtocolError)
                tradotto(self, preso.exception, "none")

    @prova("errors.copia_json")
    def test_un_documento_profondo_e_protocol_error(self) -> None:
        # Oltre il limite di ricorsione di **questo** interprete: 600 livelli
        # bastavano su 3.11 e non su 3.12, dove i frame costano meno.
        profondo: object = 0
        for _ in range(sys.getrecursionlimit() + 100):
            profondo = [profondo]
        with self.assertRaises(PlenoraError) as preso:
            _copia_json(profondo, "prova")
        self.assertIsInstance(preso.exception, ProtocolError)
        tradotto(self, preso.exception, "none")
        documento = dict(capacita_sane(), extra=profondo)
        with self.assertRaises(PlenoraError) as preso:
            cliente_con(documento).capabilities()
        self.assertIsInstance(preso.exception, ProtocolError)
        tradotto(self, preso.exception, "none")


# --- 3. la decodifica del JSON ----------------------------------------------


class IlJson(unittest.TestCase):
    @prova("errors.carica_json")
    def test_ogni_guasto_del_parser_e_protocol_error_e_resta_unknown(self) -> None:
        """Prima: solo `JSONDecodeError` era intercettato; `RecursionError`
        (annidamento) e `ValueError` (intero oltre il limite di cifre)
        uscivano com'erano, e dopo una scrittura si perdeva `unknown`."""
        casi = {
            "annidamento": b"[" * 200_000 + b"]" * 200_000,
            "intero lungo": b'{"status": "ok", "n": ' + b"7" * 50_000 + b"}",
            "sintassi": b"{" + SEGRETO.encode(),
        }
        for nome, stdout in casi.items():
            with self.subTest(nome):
                finto = PopenFinto(stdout=stdout)
                with mock.patch.object(subprocess, "Popen", finto):
                    with self.assertRaises(PlenoraError) as preso:
                        Runner(Path("plenora-io")).run(SCRIVE)
                self.assertIsInstance(preso.exception, ProtocolError)
                tradotto(self, preso.exception, "unknown")


# --- 4. read_table ---------------------------------------------------------


@serve_pyarrow
class LaLetturaInTabella(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp())

    def cliente(self, consegna) -> Client:
        cliente = Client.__new__(Client)

        def chiama(argv, modello):
            # Il comando `read` riuscito: scrive la consegna dove `--output` dice.
            uscita = argv[argv.index("--output") + 1]
            Path(uscita).write_bytes(b"non e' IPC " + SEGRETO.encode())
            documento = validazione_sana() if consegna is None else validazione_sana(
                delivered=consegna_sana())
            return modello.from_json(documento)

        cliente._chiama = chiama
        return cliente

    @prova("client.read_table")
    def test_una_consegna_non_dichiarata_e_unknown(self) -> None:
        """Prima: `ProtocolError` con effetto `none`, e il file era scritto."""
        with self.assertRaises(PlenoraError) as preso:
            self.cliente(None).read_table("s.geojson", temp_dir=self.tmp)
        self.assertIsInstance(preso.exception, ProtocolError)
        tradotto(self, preso.exception, "unknown")

    @prova("client.read_table")
    def test_una_consegna_illeggibile_e_unknown(self) -> None:
        with self.assertRaises(PlenoraError) as preso:
            self.cliente(True).read_table("s.geojson", temp_dir=self.tmp)
        self.assertIsInstance(preso.exception, ProtocolError)
        tradotto(self, preso.exception, "unknown")

    @prova("client.read_table")
    def test_resta_unknown_anche_con_la_pulizia_fallita(self) -> None:
        def non_cancella(percorso, *argomenti, **opzioni):
            raise PermissionError(SEGRETO)

        with mock.patch.object(adattatore.shutil, "rmtree", non_cancella):
            with self.assertRaises(PlenoraError) as preso:
                self.cliente(True).read_table("s.geojson", temp_dir=self.tmp)
        self.assertIsInstance(preso.exception, ProtocolError)
        self.assertTrue(preso.exception.cleanup_failed)
        tradotto(self, preso.exception, "unknown")


# --- 5. gli argomenti e il Popen ----------------------------------------------


class FspathRotto:
    def __fspath__(self):
        raise RuntimeError(SEGRETO)


class StrRotto:
    def __str__(self):
        raise RuntimeError(SEGRETO)

    __format__ = __repr__ = lambda self, *a: (_ for _ in ()).throw(RuntimeError(SEGRETO))


class BoolRotto:
    def __bool__(self):
        raise RuntimeError(SEGRETO)


class MappaRotta(dict):
    def items(self):
        raise RuntimeError(SEGRETO)


class ArrowRotto:
    def __getattr__(self, nome):
        raise RuntimeError(SEGRETO)


NUL = f"a\x00{SEGRETO}"


def _limiti_grezzi(**campi):
    """Un `Limits` con campi non normalizzati, costruito senza `__init__`."""
    from dataclasses import fields

    grezzo = object.__new__(Limits)
    valori = object.__getattribute__(grezzo, "__dict__")
    for campo in fields(Limits):
        valori[campo.name] = None
    valori.update(campi)
    return grezzo


OSTILI = {
    "percorso": [FspathRotto(), 12, NUL, b"/dati/x", object()],
    "testo": [StrRotto(), 12, NUL, b"x", object()],
    "intero": ["1", 1.5, True, StrRotto()],
    "opzioni": [1, ["k"], MappaRotta(k="v"), {"k": StrRotto()}, {1: "v"},
                {f"k\x00{SEGRETO}": "v"}, {"k": NUL}, {"a=b": "c"}],
    # `Limits` con campi ostili non si costruisce piu' (si normalizza nel
    # costruttore): qui si fabbrica scavalcando `__init__`, come farebbe un
    # `__new__` altrui, per provare che il client rinormalizza.
    "limiti": [1, "x", _limiti_grezzi(max_rows="10"), _limiti_grezzi(max_rows=True),
               _limiti_grezzi(deadline=5), _limiti_grezzi(max_rows=StrRotto())],
    "booleano": [BoolRotto(), "si", 1],
}

#: Per ogni metodo pubblico: la chiamata sana, e il genere di ciascun argomento.
METODI = {
    "inspect": ({"source": "s.geojson"},
                {"source": "percorso", "assume_crs": "testo", "options": "opzioni"}),
    "layers": ({"source": "s.geojson"},
               {"source": "percorso", "assume_crs": "testo", "options": "opzioni"}),
    "validate": ({"source": "s.geojson"},
                 {"source": "percorso", "layer": "intero", "limit": "intero",
                  "assume_crs": "testo", "options": "opzioni", "limits": "limiti"}),
    "read": ({"source": "s.geojson", "output": "o.arrow"},
             {"source": "percorso", "output": "percorso", "layer": "intero",
              "assume_crs": "testo", "options": "opzioni", "limits": "limiti"}),
    "write": ({"source": "s.arrow", "destination": "o.gpkg", "format": "gpkg"},
              {"source": "percorso", "destination": "percorso", "format": "testo",
               "layer": "intero", "assume_crs": "testo", "read_options": "opzioni",
               "write_options": "opzioni", "options": "opzioni", "durable": "booleano",
               "limits": "limiti"}),
    "convert": ({"source": "s.geojson", "target": "o.gpkg", "source_format": "geojson",
                 "target_format": "gpkg"},
                {"source": "percorso", "target": "percorso", "source_format": "testo",
                 "target_format": "testo", "layer": "intero", "assume_crs": "testo",
                 "read_options": "opzioni", "write_options": "opzioni", "options": "opzioni",
                 "durable": "booleano", "limits": "limiti"}),
    "require_profile": ({"profile": "base"}, {"profile": "testo"}),
}


class GliArgomenti(unittest.TestCase):
    """Ogni argomento di ogni metodo pubblico, sostituito da valori ostili."""

    def cliente(self) -> tuple[Client, PopenFinto]:
        cliente = Client.__new__(Client)
        cliente._runner = Runner(Path("plenora-io"))
        cliente._manifest = None
        return cliente, PopenFinto(stdout=b"{}")

    @prova("client.argomenti")
    def test_nessun_argomento_ostile_attraversa_il_confine(self) -> None:
        """Prima: `__fspath__` che solleva passava, e un NUL arrivava al
        `Popen`, che solleva `ValueError`; un `__str__` rotto, una chiave
        con `=` -- che la CLI avrebbe diviso altrove -- o un booleano ostile
        avevano la stessa sorte."""
        provati = 0
        for metodo, (sano, generi) in METODI.items():
            for argomento, genere in generi.items():
                for ostile in OSTILI[genere]:
                    with self.subTest(metodo=metodo, argomento=argomento, valore=type(ostile).__name__):
                        cliente, finto = self.cliente()
                        chiamata = dict(sano, **{argomento: ostile})
                        with mock.patch.object(subprocess, "Popen", finto):
                            with self.assertRaises(PlenoraError) as preso:
                                getattr(cliente, metodo)(**chiamata)
                        attese = (InvalidArgumentError,)
                        if pa is None and metodo == "write" and argomento == "source":
                            # Senza pyarrow un oggetto che non e' un percorso si
                            # rifiuta come `unsupported`, prima di guardarlo.
                            attese = (InvalidArgumentError, OptionalDependencyError)
                        self.assertIsInstance(preso.exception, attese)
                        self.assertEqual(finto.partenze, 0, "il processo non parte")
                        tradotto(self, preso.exception, "none")
                        provati += 1
        self.assertGreater(provati, 150)

    @prova("client.argomenti")
    def test_un_oggetto_arrow_ostile_e_invalid_argument(self) -> None:
        cliente, finto = self.cliente()
        with mock.patch.object(subprocess, "Popen", finto):
            with self.assertRaises(PlenoraError) as preso:
                cliente.write(ArrowRotto(), "o.gpkg", format="gpkg")
        self.assertEqual(finto.partenze, 0)
        tradotto(self, preso.exception, "none")

    @prova("client.argomenti")
    def test_il_costruttore_rifiuta_un_timeout_ostile(self) -> None:
        for ostile in ("5", True, float("nan"), -1, StrRotto()):
            with self.subTest(valore=repr(ostile) if not isinstance(ostile, StrRotto) else "StrRotto"):
                with mock.patch.object(discovery, "trova_binario", lambda b: Path("plenora-io")), \
                        mock.patch("plenora_io.client.leggi_manifesto", lambda p: None):
                    with self.assertRaises(PlenoraError) as preso:
                        Client(timeout=ostile)
                tradotto(self, preso.exception, "none")

    @prova("process.Popen")
    def test_un_nul_nel_popen_vero_e_invalid_argument_senza_effetto(self) -> None:
        """Prima: `Popen` intercettava solo `OSError`, e il `ValueError` del NUL
        passava. Qui il `Popen` e' quello vero, perche' e' lui a sollevare."""
        for argv in (["write", NUL, "o.gpkg"], ["convert", "a", "b", "--opt", NUL]):
            with self.subTest(argv=argv[0]):
                with self.assertRaises(PlenoraError) as preso:
                    Runner(Path("plenora-io")).run(argv)
                self.assertIsInstance(preso.exception, InvalidArgumentError)
                tradotto(self, preso.exception, "none")

    @prova("process.Popen")
    def test_un_argomento_non_testo_nel_popen_e_invalid_argument(self) -> None:
        with self.assertRaises(PlenoraError) as preso:
            Runner(Path("plenora-io")).run(["write", 12, "o.gpkg"])  # type: ignore[list-item]
        self.assertIsInstance(preso.exception, InvalidArgumentError)
        tradotto(self, preso.exception, "none")


# --- 6. la scoperta e il manifesto -------------------------------------------


class LaScoperta(unittest.TestCase):
    def setUp(self) -> None:
        self.radice = Path(tempfile.mkdtemp())
        (self.radice / "bin").mkdir()
        self.binario = self.radice / "bin" / "plenora-io"
        self.binario.write_bytes(b"")

    def manifesto(self, contenuto: bytes) -> None:
        (self.radice / "MANIFEST.json").write_bytes(contenuto)

    @prova("discovery.manifesto")
    def test_un_manifesto_non_utf8_e_manifest_error(self) -> None:
        """Prima: `UnicodeDecodeError` non era intercettato."""
        self.manifesto(b"\xff\xfe" + SEGRETO.encode())
        with self.assertRaises(PlenoraError) as preso:
            discovery.leggi_manifesto(self.binario)
        self.assertIsInstance(preso.exception, ManifestError)
        tradotto(self, preso.exception, "none")

    @prova("discovery.manifesto")
    def test_un_manifesto_non_json_taglia_la_catena(self) -> None:
        """Prima: `from errore`, e il testo del parser restava nella catena."""
        for contenuto in (b"{" + SEGRETO.encode(), b"[" * 200_000, b'{"n": ' + b"9" * 50_000 + b"}"):
            with self.subTest(inizio=contenuto[:3]):
                self.manifesto(contenuto)
                with self.assertRaises(PlenoraError) as preso:
                    discovery.leggi_manifesto(self.binario)
                self.assertIsInstance(preso.exception, ManifestError)
                tradotto(self, preso.exception, "none")

    @prova("discovery.manifesto")
    def test_un_manifesto_che_non_si_legge_taglia_la_catena(self) -> None:
        self.manifesto(b"{}")

        def non_legge(*argomenti, **opzioni):
            raise PermissionError(13, SEGRETO)

        with mock.patch.object(Path, "read_text", non_legge):
            with self.assertRaises(PlenoraError) as preso:
                discovery.leggi_manifesto(self.binario)
        self.assertIsInstance(preso.exception, ManifestError)
        tradotto(self, preso.exception, "none")

    @prova("discovery.manifesto")
    def test_un_manifesto_con_tipi_sbagliati_taglia_la_catena(self) -> None:
        documento = {"nome": "x", "versione": "1", "piattaforma": "p", "profilo": "base",
                     "canale": "c", "non_release": False, "altro": [[[[1e400]]]]}
        self.manifesto(json.dumps(documento).replace("Infinity", "1e400").encode())
        with self.assertRaises(PlenoraError) as preso:
            discovery.leggi_manifesto(self.binario)
        self.assertIsInstance(preso.exception, ManifestError)
        tradotto(self, preso.exception, "none")

    @prova("discovery.binario")
    def test_un_binario_ostile_e_invalid_argument(self) -> None:
        for ostile in (FspathRotto(), 12, object()):
            with self.subTest(valore=type(ostile).__name__):
                with self.assertRaises(PlenoraError) as preso:
                    discovery.trova_binario(ostile)
                self.assertIsInstance(preso.exception, InvalidArgumentError)
                tradotto(self, preso.exception, "none")

    @prova("discovery.binario")
    def test_un_candidato_che_non_si_esamina_e_local_io_error(self) -> None:
        """Saltarlo farebbe scegliere il candidato successivo: un binario
        diverso da quello indicato, in silenzio."""

        def non_esamina(self, *argomenti, **opzioni):
            raise PermissionError(13, SEGRETO)

        with mock.patch.object(Path, "is_file", non_esamina):
            with self.assertRaises(PlenoraError) as preso:
                discovery.trova_binario(self.binario)
        self.assertIsInstance(preso.exception, LocalIoError)
        tradotto(self, preso.exception, "none")


# --- 7. i modelli, campo per campo ---------------------------------------------


VALORI_SBAGLIATI = [None, True, 0, -1, 10**30, 1.5, "", "x", [], {}, [None], {"k": None}]


def cammini(documento, prefisso=()):
    """Ogni cammino verso un valore del documento, oggetti ed elenchi compresi."""
    yield prefisso
    if isinstance(documento, dict):
        for chiave, valore in documento.items():
            yield from cammini(valore, prefisso + (chiave,))
    elif isinstance(documento, list):
        for indice, valore in enumerate(documento):
            yield from cammini(valore, prefisso + (indice,))


def sostituito(documento, cammino, valore):
    copia = copy.deepcopy(documento)
    if not cammino:
        return valore
    contenitore = copia
    for passo in cammino[:-1]:
        contenitore = contenitore[passo]
    if valore is ...:
        if isinstance(contenitore, dict):
            del contenitore[cammino[-1]]
        else:
            contenitore.pop(cammino[-1])
    else:
        contenitore[cammino[-1]] = valore
    return copia


def risultato_di(busta: dict) -> dict:
    """Il `result` della busta di convert, che i test portano ancora piatta."""
    return {k: v for k, v in busta.items() if k not in ("status", "protocol_version", "contract")}


MODELLI = {
    "Version": (Version, lambda: {"component_version": "4.2.0", "cli_protocol_version": 2}),
    "Catalog": (Catalog, catalogo_sano),
    "Inspect": (Inspect, inspect_sano),
    "Layers": (Layers, layers_sano),
    "Validation": (Validation, validazione_sana),
    "ConvertResult": (ConvertResult, conversione_sana),
    "WriteResult": (WriteResult, scrittura_sana),
    "Capabilities": (Capabilities, capacita_sane),
}


class IModelli(unittest.TestCase):
    """Ogni campo di ogni busta, sostituito o tolto: un risultato o `ProtocolError`."""

    @prova("models.from_json")
    def test_ogni_campo_sbagliato_e_protocol_error(self) -> None:
        provati = 0
        for nome, (modello, sano) in MODELLI.items():
            documento = sano()
            try:
                modello.from_json(copy.deepcopy(documento))
            except ProtocolError:
                self.skipTest(f"il campione di {nome} non e' piu' sano: aggiornarlo")
            for cammino in cammini(documento):
                for valore in VALORI_SBAGLIATI + ([...] if cammino else []):
                    with self.subTest(modello=nome, cammino=cammino, valore=repr(valore)[:20]):
                        try:
                            modello.from_json(sostituito(documento, cammino, valore))
                        except ProtocolError:
                            pass
                        except Exception as altro:  # noqa: BLE001 - e' la prova
                            self.fail(f"{type(altro).__name__} attraversa il confine")
                        provati += 1
        self.assertGreater(provati, 1000)

    @prova("models.from_json")
    def test_ogni_campo_sbagliato_di_capabilities_dal_metodo_pubblico(self) -> None:
        documento = capacita_sane()
        for cammino in cammini(documento):
            for valore in VALORI_SBAGLIATI + ([...] if cammino else []):
                with self.subTest(cammino=cammino, valore=repr(valore)[:20]):
                    try:
                        cliente_con(sostituito(documento, cammino, valore)).capabilities()
                    except PlenoraError as errore:
                        self.assertIsInstance(errore, ProtocolError)
                        tradotto(self, errore, "none")
                    except Exception as altro:  # noqa: BLE001
                        self.fail(f"{type(altro).__name__} attraversa il confine")

    @prova("models.from_json")
    def test_ogni_campo_sbagliato_della_busta_d_errore(self) -> None:
        sana = {
            "status": "error",
            "protocol_version": 2,
            "error": {"code": "X", "category": "io", "phase": "write",
                      "remote_effect": "none", "retry": {"kind": "never"}, "message": "m"},
        }
        for cammino in cammini(sana):
            for valore in VALORI_SBAGLIATI + ([...] if cammino else []):
                with self.subTest(cammino=cammino, valore=repr(valore)[:20]):
                    try:
                        esito = failure_from_envelope(sostituito(sana, cammino, valore), 4, SCRIVE)
                        self.assertIsInstance(esito, CommandFailed)
                    except ProtocolError:
                        pass
                    except Exception as altro:  # noqa: BLE001
                        self.fail(f"{type(altro).__name__} attraversa il confine")

    @prova("process.decode")
    @prova("process.communicate")
    def test_il_processo_e_gia_coperto_da_test_confini_sdk(self) -> None:
        """`IlProcesso` in `test_confini_sdk.py` esercita flussi non UTF-8,
        pipe rotte, timeout e seconda attesa: qui si verifica che ci sia."""
        import test_confini_sdk

        nomi = set(dir(test_confini_sdk.IlProcesso))
        for atteso in ("test_un_flusso_che_non_e_utf8_e_un_protocol_error",
                       "test_un_guasto_delle_pipe_e_un_protocol_error",
                       "test_la_seconda_attesa_dopo_il_kill_non_sostituisce_l_errore"):
            self.assertIn(atteso, nomi)


class PuntiInventariati(unittest.TestCase):
    def test_ogni_punto_dell_inventario_ha_una_prova_definita(self) -> None:
        """Il verso statico: ogni punto ha almeno un test che lo dichiara. Che
        sia stato **eseguito** lo dice `tearDownModule`."""
        import inspect

        dichiarati = set()
        for valore in globals().values():
            if isinstance(valore, type) and issubclass(valore, unittest.TestCase):
                for nome, metodo in vars(valore).items():
                    if nome.startswith("test_"):
                        sorgente = inspect.getsource(metodo)
                        for punto in PUNTI:
                            if f'@prova("{punto}")' in sorgente:
                                dichiarati.add(punto)
        self.assertEqual(set(PUNTI) - dichiarati, set())


# --- 8. la normalizzazione degli argomenti -------------------------------------


class Contatore:
    chiamate = 0


class StrOstile(str):
    """Una `str` con i metodi ridefiniti: nessuno deve essere chiamato."""

    def __str__(self):
        Contatore.chiamate += 1
        raise RuntimeError(SEGRETO)

    def __format__(self, specifica):
        Contatore.chiamate += 1
        raise RuntimeError(SEGRETO)

    def __contains__(self, altro):
        Contatore.chiamate += 1
        raise RuntimeError(SEGRETO)

    def __eq__(self, altro):
        Contatore.chiamate += 1
        raise RuntimeError(SEGRETO)

    __hash__ = str.__hash__


class IntOstile(int):
    def __int__(self):
        Contatore.chiamate += 1
        raise RuntimeError(SEGRETO)

    def __index__(self):
        Contatore.chiamate += 1
        raise RuntimeError(SEGRETO)

    def __repr__(self):
        Contatore.chiamate += 1
        raise RuntimeError(SEGRETO)

    __str__ = __format__ = __repr__


class DurataOstile(timedelta):
    def total_seconds(self):
        Contatore.chiamate += 1
        raise RuntimeError(SEGRETO)


class LimitsOstile(Limits):
    def to_argv(self):
        Contatore.chiamate += 1
        raise RuntimeError(SEGRETO)


class PopenCheRegistra(PopenFinto):
    def __init__(self):
        super().__init__(stdout=b"{}")
        self.argv = None

    def __call__(self, argv, **opzioni):
        self.argv = list(argv)
        return super().__call__(argv, **opzioni)


class LaNormalizzazione(unittest.TestCase):
    def setUp(self) -> None:
        Contatore.chiamate = 0
        self.cliente = Client.__new__(Client)
        self.cliente._runner = Runner(Path("plenora-io"))
        self.cliente._manifest = None

    @prova("argomenti.sottoclassi")
    def test_le_sottoclassi_diventano_copie_esatte_senza_essere_chiamate(self) -> None:
        """Prima: `f"{chiave}={valore}"` chiamava `__format__`, `str(layer)`
        `__str__`, `limits.to_argv()` il metodo della sottoclasse."""
        finto = PopenCheRegistra()
        with mock.patch.object(subprocess, "Popen", finto):
            with self.assertRaises(ProtocolError):  # `{}` non e' una busta: va bene
                self.cliente.convert(
                    StrOstile("s.geojson"),
                    StrOstile("o.gpkg"),
                    source_format=StrOstile("geojson"),
                    target_format=StrOstile("gpkg"),
                    layer=IntOstile(2),
                    assume_crs=StrOstile("EPSG:4326"),
                    options={StrOstile("k"): StrOstile("v")},
                    limits=LimitsOstile(max_rows=IntOstile(5), deadline=DurataOstile(seconds=3)),
                )
        self.assertEqual(Contatore.chiamate, 0, "codice del chiamante eseguito dopo il confine")
        self.assertIsNotNone(finto.argv)
        for elemento in finto.argv[1:]:
            self.assertIs(type(elemento), str)
        self.assertEqual(
            finto.argv[1:],
            ["convert", "s.geojson", "o.gpkg", "--from", "geojson", "--to", "gpkg",
             "--assume-crs", "EPSG:4326", "--layer", "2", "--opt", "k=v",
             "--deadline-ms", "3000", "--max-rows", "5"],
        )

    @prova("argomenti.sottoclassi")
    def test_una_durata_lunga_arriva_esatta(self) -> None:
        """Prima: `int(total_seconds() * 1000)` in virgola mobile."""
        durata = timedelta(days=2 * 10**8, milliseconds=1)
        argv = Limits(deadline=durata).to_argv()
        self.assertEqual(argv, ["--deadline-ms", str((2 * 10**8 * 86_400) * 1000 + 1)])

    @prova("argomenti.grandezze")
    def test_le_grandezze_si_rifiutano_prima_delle_conversioni(self) -> None:
        """`str(10**50000)` solleva `ValueError`, `math.isfinite(10**1000)`
        `OverflowError`: si rifiuta prima, con un errore tipizzato."""
        casi = {
            "layer": lambda n: self.cliente.validate("s.geojson", layer=n),
            "limit": lambda n: self.cliente.validate("s.geojson", limit=n),
            "max_rows": lambda n: self.cliente.validate("s.geojson", limits=Limits(max_rows=n)),
        }
        for nome, chiama in casi.items():
            for numero in (10**1000, 10**50000, -(10**1000), 1 << 64):
                with self.subTest(argomento=nome, cifre=len(str(numero)) if numero < 10**4000 else "molte"):
                    finto = PopenFinto(stdout=b"{}")
                    with mock.patch.object(subprocess, "Popen", finto):
                        with self.assertRaises(PlenoraError) as preso:
                            chiama(numero)
                    self.assertIsInstance(preso.exception, InvalidArgumentError)
                    self.assertEqual(finto.partenze, 0)
                    tradotto(self, preso.exception, "none")
        for numero in (10**1000, 10**50000, 1 << 64):
            with self.subTest(timeout="enorme"):
                with self.assertRaises(PlenoraError) as preso:
                    Runner(Path("plenora-io"), timeout=numero)
                self.assertIsInstance(preso.exception, InvalidArgumentError)
                tradotto(self, preso.exception, "none")


# --- 9. la rete ---------------------------------------------------------------


def _avvolto(oggetto) -> bool:
    from plenora_io.confine import SEGNO

    return bool(getattr(oggetto, SEGNO, False))


class LaRete(unittest.TestCase):
    @staticmethod
    def _callable_delle_classi():
        """Ogni callable che una classe dell'SDK definisce, senza filtri propri.

        Non ripete il filtro della rete: elenca **tutto** -- metodi speciali,
        privati, getter, setter e deleter -- e la prova confronta con le sole
        eccezioni dichiarate, `NON_AVVOLTI` e i nomi privati (`_x`), che non
        sono superficie pubblica.
        """
        import plenora_io

        visti = set()
        for nome in plenora_io.__all__:
            valore = getattr(plenora_io, nome)
            if not isinstance(valore, type):
                continue
            for classe in valore.__mro__:
                if not classe.__module__.startswith("plenora_io") or classe in visti:
                    continue
                visti.add(classe)
                for attributo, interno in vars(classe).items():
                    if isinstance(interno, (classmethod, staticmethod)):
                        yield classe, attributo, "", interno.__func__
                    elif isinstance(interno, property):
                        for parte in ("fget", "fset", "fdel"):
                            funzione = getattr(interno, parte)
                            if funzione is not None:
                                yield classe, attributo, parte, funzione
                    elif callable(interno) and not isinstance(interno, type):
                        yield classe, attributo, "", interno

    @prova("confine.rete")
    def test_ogni_metodo_pubblico_e_avvolto(self) -> None:
        """Per introspezione: un callable nuovo senza rete e' rosso.

        Anche i metodi speciali (`__hash__`, `__repr__`, `__eq__`...), i
        setter e i deleter delle proprieta', le classi d'eccezione. Gli esclusi
        sono esattamente `NON_AVVOLTI` e i nomi privati.
        """
        import plenora_io
        from plenora_io.confine import NON_AVVOLTI, NON_AVVOLTI_DEFINITI

        scoperti = 0
        esclusi_visti = set()
        for classe, attributo, parte, funzione in self._callable_delle_classi():
            privato = attributo.startswith("_") and not (
                attributo.startswith("__") and attributo.endswith("__")
            )
            if attributo in NON_AVVOLTI:
                # Un nome escluso che una classe dell'SDK **definisce**: deve
                # stare nell'elenco chiuso, o l'esclusione si sarebbe aggiunta
                # in silenzio.
                esclusi_visti.add((classe.__name__, attributo))
                continue
            if privato:
                continue
            with self.subTest(metodo=f"{classe.__name__}.{attributo}{'.' + parte if parte else ''}"):
                self.assertTrue(_avvolto(funzione))
                scoperti += 1
        for nome in plenora_io.__all__:
            valore = getattr(plenora_io, nome)
            if callable(valore) and not isinstance(valore, type):
                with self.subTest(funzione=nome):
                    self.assertTrue(_avvolto(valore))
                    scoperti += 1
        self.assertGreater(scoperti, 150)
        self.assertEqual(esclusi_visti, set(NON_AVVOLTI_DEFINITI))

    @prova("confine.rete")
    def test_nessun_metodo_pubblico_rende_un_generatore(self) -> None:
        """Un generatore uscirebbe dalla rete prima di eseguire il corpo."""
        import inspect

        for classe, attributo, parte, funzione in self._callable_delle_classi():
            originale = inspect.unwrap(funzione)
            with self.subTest(metodo=f"{classe.__name__}.{attributo}"):
                self.assertFalse(inspect.isgeneratorfunction(originale))
                self.assertFalse(inspect.isasyncgenfunction(originale))
                self.assertFalse(inspect.iscoroutinefunction(originale))

    @prova("confine.rete")
    def test_nessun_metodo_si_aggiunge_alle_istanze(self) -> None:
        """I metodi aggiunti dinamicamente sfuggirebbero alla rete: non ce ne sono."""
        import dataclasses

        campioni = [
            Catalog.from_json(catalogo_sano()),
            Inspect.from_json(inspect_sano()),
            Layers.from_json(layers_sano()),
            Validation.from_json(validazione_sana()),
            ConvertResult.from_json(conversione_sana()),
            WriteResult.from_json(scrittura_sana()),
            Capabilities.from_json(capacita_sane()),
            Limits(max_rows=1),
        ]

        def visita(oggetto):
            for valore in object.__getattribute__(oggetto, "__dict__").values():
                self.assertFalse(callable(valore) and not dataclasses.is_dataclass(valore))
                if dataclasses.is_dataclass(valore):
                    visita(valore)
                elif type(valore) is FrozenList or type(valore) is list:
                    # Dalla 5.0.0 gli elenchi dei modelli sono `FrozenList`: la
                    # visita deve scendere anche li', o i driver di un
                    # catalogo resterebbero fuori.
                    for elemento in valore:
                        if dataclasses.is_dataclass(elemento):
                            visita(elemento)
                    visitati.append(type(oggetto).__name__)

        visitati: list[str] = []
        for campione in campioni:
            with self.subTest(modello=type(campione).__name__):
                visita(campione)
        self.assertIn("Catalog", visitati, "la visita non e' scesa negli elenchi del catalogo")

    @prova("confine.rete")
    def test_i_metodi_speciali_dei_modelli_non_escono_dalla_gerarchia(self) -> None:
        """Prima: `hash(Version(...))` dava `TypeError` (il campo `raw` e' un
        `dict`), e `repr` ed `==` eseguivano il codice dei valori."""
        import copy as copia
        import pickle

        versione = Version.from_json({"component_version": "4.2.0", "cli_protocol_version": 2})
        uguale = Version.from_json({"component_version": "4.2.0", "cli_protocol_version": 2})
        self.assertEqual(hash(versione), hash(uguale))
        self.assertEqual(versione, uguale)
        catalogo = Catalog.from_json(catalogo_sano())
        self.assertEqual(hash(catalogo), hash(copia.deepcopy(catalogo)))
        self.assertEqual(pickle.loads(pickle.dumps(catalogo)), catalogo)
        with self.assertRaises(PlenoraError) as preso:
            versione.component_version = SEGRETO
        self.assertIsInstance(preso.exception, InvalidArgumentError)
        tradotto(self, preso.exception, "none")
        with self.assertRaises(PlenoraError):
            del versione.component_version

    @prova("confine.rete")
    def test_un_modello_costruito_a_mano_non_tiene_oggetti_del_chiamante(self) -> None:
        Contatore.chiamate = 0
        versione = Version(component_version=StrOstile("4.2.0"), cli_protocol_version=IntOstile(2))
        self.assertIs(type(versione.component_version), str)
        self.assertIs(type(versione.cli_protocol_version), int)
        repr(versione)
        hash(versione)
        self.assertEqual(versione, Version(component_version="4.2.0", cli_protocol_version=2))
        self.assertEqual(Contatore.chiamate, 0)

        class Qualunque:
            def __repr__(self):
                raise RuntimeError(SEGRETO)

        for valore in (Qualunque(), [Qualunque()], {"k": Qualunque()}, {1: "v"}):
            with self.subTest(valore=type(valore).__name__):
                with self.assertRaises(PlenoraError) as preso:
                    Version(component_version="x", cli_protocol_version=2, raw={"a": valore})
                self.assertIsInstance(preso.exception, InvalidArgumentError)
                tradotto(self, preso.exception, "none")

    @prova("confine.rete")
    def test_un_eccezione_non_prevista_prima_dell_avvio_e_none(self) -> None:
        with mock.patch("plenora_io.models._copia_json", side_effect=RuntimeError(SEGRETO)):
            with self.assertRaises(PlenoraError) as preso:
                Catalog.from_json(catalogo_sano())
        self.assertEqual(type(preso.exception).__name__, "UnexpectedError")
        self.assertEqual(preso.exception.category, "internal")
        tradotto(self, preso.exception, "none")

    @prova("confine.rete")
    def test_un_eccezione_non_prevista_dopo_l_avvio_di_una_scrittura_e_unknown(self) -> None:
        cliente = Client.__new__(Client)
        cliente._runner = Runner(Path("plenora-io"))
        finto = PopenFinto(stdout=b"{}")
        with mock.patch.object(subprocess, "Popen", finto), \
                mock.patch("plenora_io.process._testo", side_effect=RuntimeError(SEGRETO)):
            with self.assertRaises(PlenoraError) as preso:
                cliente.write("s.arrow", "o.gpkg", format="gpkg")
        self.assertEqual(finto.partenze, 1)
        self.assertEqual(type(preso.exception).__name__, "UnexpectedError")
        tradotto(self, preso.exception, "unknown")

    @prova("confine.rete")
    def test_dopo_l_avvio_di_una_lettura_resta_none(self) -> None:
        cliente = Client.__new__(Client)
        cliente._runner = Runner(Path("plenora-io"))
        finto = PopenFinto(stdout=b"{}")
        with mock.patch.object(subprocess, "Popen", finto), \
                mock.patch("plenora_io.process._testo", side_effect=RuntimeError(SEGRETO)):
            with self.assertRaises(PlenoraError) as preso:
                cliente.inspect("s.geojson")
        tradotto(self, preso.exception, "none")

    @prova("confine.rete")
    def test_keyboardinterrupt_passa(self) -> None:
        with mock.patch("plenora_io.models._copia_json", side_effect=KeyboardInterrupt):
            with self.assertRaises(KeyboardInterrupt):
                Catalog.from_json(catalogo_sano())


class PercorsoCheCambia:
    """Un `os.PathLike` che rende un valore diverso a ogni lettura."""

    def __init__(self):
        self.letture = 0

    def __fspath__(self):
        self.letture += 1
        return f"s{self.letture}.geojson"


class MappaCheConta(dict):
    def __init__(self, *argomenti, **opzioni):
        super().__init__(*argomenti, **opzioni)
        self.letture = 0

    def items(self):
        self.letture += 1
        return super().items()


@serve_pyarrow
class LeCopieNormalizzate(unittest.TestCase):
    @prova("argomenti.sottoclassi")
    def test_read_table_legge_ogni_argomento_una_volta_sola(self) -> None:
        """Prima: `read_table()` normalizzava, scartava il risultato, e
        `read()` rileggeva `__fspath__` e `items()` degli originali."""
        cliente = Client.__new__(Client)
        visti = []

        def chiama(argv, modello):
            visti.append(list(argv))
            raise ProtocolError("fine della prova")

        cliente._chiama = chiama
        sorgente = PercorsoCheCambia()
        opzioni = MappaCheConta(k="v")
        # `chiama` e' codice della prova: il suo `ProtocolError` e' esterno, e
        # la rete lo traduce (settimo giro). Qui conta che si fermi li'.
        with self.assertRaises(PlenoraError):
            cliente.read_table(sorgente, options=opzioni, temp_dir=tempfile.mkdtemp())
        self.assertEqual(sorgente.letture, 1)
        self.assertEqual(opzioni.letture, 1)
        self.assertEqual(visti[0][:2], ["read", "s1.geojson"])


# --- 10. i punti specifici del terzo giro --------------------------------------


class PopenCheNonEsce(PopenFinto):
    def __exit__(self, *eccezione):
        raise OSError(SEGRETO)


class IPuntiSpecifici(unittest.TestCase):
    @prova("process.exit")
    def test_l_uscita_dal_with_di_popen_e_tradotta(self) -> None:
        """Prima: `__exit__` stava fuori dalla traduzione."""
        for argv, effetto in ((SCRIVE, "unknown"), (["inspect", "s.geojson"], "none")):
            with self.subTest(comando=argv[0]):
                with mock.patch.object(subprocess, "Popen", PopenCheNonEsce(stdout=b"{}")):
                    with self.assertRaises(PlenoraError) as preso:
                        Runner(Path("plenora-io")).run(argv)
                self.assertIsInstance(preso.exception, ProtocolError)
                tradotto(self, preso.exception, effetto)

    @prova("arrow.import")
    def test_un_import_di_pyarrow_che_solleva_e_optional_dependency(self) -> None:
        import builtins

        vero = builtins.__import__

        def importa(nome, *argomenti, **opzioni):
            if nome == "pyarrow" or nome.startswith("pyarrow."):
                raise OSError(SEGRETO)
            return vero(nome, *argomenti, **opzioni)

        with mock.patch.dict(sys.modules, {k: v for k, v in sys.modules.items() if not k.startswith("pyarrow")}, clear=True), \
                mock.patch.object(builtins, "__import__", importa):
            with self.assertRaises(PlenoraError) as preso:
                adattatore.pyarrow()
        self.assertIsInstance(preso.exception, OptionalDependencyError)
        tradotto(self, preso.exception, "none")

    @prova("arrow.import")
    def test_una_versione_di_pyarrow_che_solleva_e_optional_dependency(self) -> None:
        import types

        class Modulo(types.ModuleType):
            def __getattr__(self, nome):
                raise RuntimeError(SEGRETO)

        finto = Modulo("pyarrow")
        finto.ipc = types.ModuleType("pyarrow.ipc")
        with mock.patch.dict(sys.modules, {"pyarrow": finto, "pyarrow.ipc": finto.ipc}):
            with self.assertRaises(PlenoraError) as preso:
                adattatore.pyarrow()
        self.assertIsInstance(preso.exception, OptionalDependencyError)
        tradotto(self, preso.exception, "none")

    @prova("arrow.cartella")
    def test_mkdtemp_con_un_percorso_non_codificabile_e_local_io_error(self) -> None:
        with mock.patch.object(adattatore.tempfile, "mkdtemp",
                               side_effect=UnicodeEncodeError("utf-8", SEGRETO, 0, 1, "x")):
            with self.assertRaises(PlenoraError) as preso:
                with adattatore.cartella_temporanea(None):
                    pass
        self.assertIsInstance(preso.exception, LocalIoError)
        tradotto(self, preso.exception, "none")
        with self.assertRaises(PlenoraError) as preso:
            adattatore.cartella_temporanea("\udcff" + SEGRETO)
        self.assertIsInstance(preso.exception, InvalidArgumentError)
        tradotto(self, preso.exception, "none")

    @prova("discovery.which")
    def test_shutil_which_che_solleva_e_local_io_error(self) -> None:
        with mock.patch.dict(os.environ, {}, clear=False), \
                mock.patch.object(discovery, "_albero_accanto_al_pacchetto", return_value=None), \
                mock.patch.object(discovery.shutil, "which", side_effect=ValueError(SEGRETO)):
            os.environ.pop(discovery.VARIABILE, None)
            with self.assertRaises(PlenoraError) as preso:
                discovery.trova_binario(None)
        self.assertIsInstance(preso.exception, LocalIoError)
        tradotto(self, preso.exception, "none")

    @prova("version.metadati")
    def test_version_senza_metadati_e_un_errore_plenora(self) -> None:
        import importlib.metadata

        import plenora_io

        with mock.patch.object(importlib.metadata, "version",
                               side_effect=importlib.metadata.PackageNotFoundError(SEGRETO)):
            with self.assertRaises(PlenoraError) as preso:
                plenora_io.version()
        self.assertEqual(type(preso.exception).__name__, "PackageMetadataError")
        tradotto(self, preso.exception, "none")


class LImmutabilita(unittest.TestCase):
    """Quinto giro: i contenitori dei modelli, l'elenco chiuso, le chiavi."""

    @prova("confine.rete")
    def test_i_contenitori_annidati_non_si_modificano(self) -> None:
        """Prima: `versione.raw["x"] = 1` e `catalogo.drivers.clear()`
        cambiavano il modello e il suo `hash` dopo la costruzione."""
        versione = Version.from_json({"component_version": "4.2.0", "cli_protocol_version": 2})
        catalogo = Catalog.from_json(catalogo_sano())
        prima = (hash(versione), hash(catalogo), repr(catalogo))
        tentativi = (
            lambda: versione.raw.__setitem__("x", 1),
            lambda: versione.raw.update(x=1),
            lambda: versione.raw.pop("component_version"),
            lambda: catalogo.drivers.clear(),
            lambda: catalogo.drivers.append(catalogo.drivers[0]),
            lambda: catalogo.drivers.__setitem__(0, None),
            lambda: catalogo.drivers[0].recognised_suffixes.append(".z"),
            lambda: catalogo.drivers[0].format_options.sort(),
            lambda: catalogo.raw["drivers"].pop(),
        )
        for numero, tentativo in enumerate(tentativi):
            with self.subTest(tentativo=numero):
                with self.assertRaises(PlenoraError) as preso:
                    tentativo()
                self.assertIsInstance(preso.exception, InvalidArgumentError)
                tradotto(self, preso.exception, "none")
        self.assertEqual((hash(versione), hash(catalogo), repr(catalogo)), prima)
        # Si leggono come un `Mapping` e una `Sequence`, si confrontano con un
        # documento e si serializzano con `json_default`.
        from collections.abc import Mapping, Sequence

        import plenora_io

        self.assertIsInstance(versione.raw, Mapping)
        self.assertIsInstance(catalogo.drivers, Sequence)
        self.assertEqual(
            json.loads(json.dumps(versione.raw, default=plenora_io.json_default)), versione.raw
        )
        self.assertEqual(catalogo.drivers[0].recognised_suffixes, list(catalogo.drivers[0].recognised_suffixes))

    @prova("confine.rete")
    def test_solo_le_classi_dell_elenco_chiuso_sono_modelli(self) -> None:
        """Prima: una dataclass in un modulo `plenora_io...` passava come
        modello senza normalizzazione, anche `process.Completed`."""
        import dataclasses
        import types

        import plenora_io
        from plenora_io import confine
        from plenora_io.process import Completed

        esportate = {
            nome for nome in plenora_io.__all__
            if isinstance(getattr(plenora_io, nome), type)
            and dataclasses.is_dataclass(getattr(plenora_io, nome))
        }
        self.assertEqual(esportate, set(confine.NOMI_DEI_MODELLI))
        self.assertEqual({c.__name__ for c in confine.MODELLI}, esportate)

        interno = Completed(argv=["x"], exit_code=0, stdout="", stderr="")
        modulo = types.ModuleType("plenora_io_plugin")
        sys.modules["plenora_io_plugin"] = modulo
        self.addCleanup(sys.modules.pop, "plenora_io_plugin", None)
        exec(
            "import dataclasses\n"
            "@dataclasses.dataclass\n"
            "class Finto:\n"
            "    x: object = None\n",
            modulo.__dict__,
        )
        estraneo = modulo.Finto()
        for valore in (interno, estraneo):
            with self.subTest(tipo=type(valore).__name__):
                with self.assertRaises(PlenoraError) as preso:
                    Version(component_version="1", cli_protocol_version=2, raw={"v": valore})
                self.assertIsInstance(preso.exception, InvalidArgumentError)

    @prova("confine.rete")
    def test_un_modello_annidato_si_rinormalizza(self) -> None:
        """Un modello dell'SDK dentro un altro si rinormalizza comunque: lo
        stato che un'alterazione fuori contratto gli ha dato non passa."""
        import dataclasses

        from plenora_io import FrozenList

        catalogo = Catalog.from_json(catalogo_sano())
        driver = catalogo.drivers[0]
        object.__setattr__(driver, "raw", {"x": [1]})  # fuori contratto, apposta
        self.assertIs(type(driver.raw), dict)
        dataclasses.replace(catalogo)
        self.assertIs(type(driver.raw["x"]), FrozenList)

    @prova("confine.rete")
    def test_due_chiavi_che_normalizzate_coincidono_sono_un_errore(self) -> None:
        """Prima: due sottoclassi di `str` con lo stesso contenuto diventavano
        la stessa chiave, e una copriva l'altra in silenzio."""

        class Chiave(str):
            # Un'altra chiave per il `dict` (hash e uguaglianza propri), con lo
            # stesso contenuto di "k".
            def __hash__(self):
                return 1

            def __eq__(self, altro):
                return self is altro

        with self.assertRaises(PlenoraError) as preso:
            Version(component_version="1", cli_protocol_version=2,
                    raw={"k": 1, Chiave("k"): 2})
        self.assertIsInstance(preso.exception, InvalidArgumentError)
        tradotto(self, preso.exception, "none")

    @prova("messaggi.senza_dati")
    def test_profile_error_non_formatta_il_valore_originale(self) -> None:
        """Prima: il messaggio si costruiva da `actual` originale, e una `str`
        con `__format__` ostile entrava nel messaggio."""
        from plenora_io import ProfileError

        Contatore.chiamate = 0
        errore = ProfileError(StrOstile("filegdb"), StrOstile("base"))
        self.assertEqual(Contatore.chiamate, 0)
        self.assertIsNone(errore.required)
        self.assertIsNone(errore.actual)
        self.assertIn("fuori vocabolario", str(errore))

    @prova("argomenti.sottoclassi")
    def test_le_ricerche_normalizzano_l_argomento_prima_di_confrontarlo(self) -> None:
        Contatore.chiamate = 0
        catalogo = Catalog.from_json(catalogo_sano())
        capacita = Capabilities.from_json(capacita_sane())
        esito = Inspect.from_json(inspect_sano())
        for chiama in (
            lambda: catalogo.driver(StrOstile("non-c-e")),
            lambda: capacita.operation(StrOstile("non-c-e")),
            lambda: esito.layer(StrOstile("non-c-e")),
            lambda: esito.layers[0].field(StrOstile("non-c-e")),
        ):
            with self.assertRaises(KeyError):
                chiama()
        self.assertEqual(Contatore.chiamate, 0)
        with self.assertRaises(PlenoraError) as preso:
            catalogo.driver(12)
        self.assertIsInstance(preso.exception, InvalidArgumentError)

    @prova("messaggi.senza_dati")
    def test_gli_errori_costruiti_a_mano_non_formattano_valori_altrui(self) -> None:
        from plenora_io import BinaryNotFound, CleanupError, LocalIoError

        Contatore.chiamate = 0
        self.assertEqual(str(LocalIoError(StrOstile("m"))), "m")
        self.assertEqual(str(LocalIoError(object())), "(messaggio non testuale)")
        BinaryNotFound([StrOstile("posto"), object()])
        self.assertEqual(Contatore.chiamate, 0)
        with self.assertRaises(PlenoraError):
            CleanupError("m", remote_effect=StrOstile("committed"))


class LaMatriceDellImmutabilita(unittest.TestCase):
    """Sesto giro: ogni via di modifica, per ogni contenitore e per i modelli."""

    MUTAZIONI_DELL_ELENCO = (
        ("__setitem__", lambda x: x.__setitem__(0, 1)),
        ("item", lambda x: x.__setitem__(slice(0, 1), [1])),
        ("__delitem__", lambda x: x.__delitem__(0)),
        ("__iadd__", lambda x: x.__iadd__([1])),
        ("__imul__", lambda x: x.__imul__(2)),
        ("append", lambda x: x.append(1)),
        ("extend", lambda x: x.extend([1])),
        ("insert", lambda x: x.insert(0, 1)),
        ("remove", lambda x: x.remove(1)),
        ("pop", lambda x: x.pop()),
        ("clear", lambda x: x.clear()),
        ("sort", lambda x: x.sort()),
        ("reverse", lambda x: x.reverse()),
        ("setattr", lambda x: setattr(x, "_dati", ())),
        ("delattr", lambda x: delattr(x, "_dati")),
    )
    MUTAZIONI_DEL_DIZIONARIO = (
        ("__setitem__", lambda x: x.__setitem__("k", 1)),
        ("__delitem__", lambda x: x.__delitem__("k")),
        ("__ior__", lambda x: x.__ior__({"z": 1})),
        ("clear", lambda x: x.clear()),
        ("pop", lambda x: x.pop("k")),
        ("popitem", lambda x: x.popitem()),
        ("setdefault", lambda x: x.setdefault("z", 1)),
        ("update", lambda x: x.update(z=1)),
        ("setattr", lambda x: setattr(x, "_dati", {})),
        ("delattr", lambda x: delattr(x, "_dati")),
    )

    def contenitori(self):
        return FrozenList([1, [2], {"a": 3}]), FrozenDict({"k": [1], "j": {"x": 2}})

    @prova("confine.rete")
    def test_ogni_metodo_di_modifica_e_un_errore_e_niente_cambia(self) -> None:
        elenco, dizionario = self.contenitori()
        prima = (hash(elenco), hash(dizionario), repr(elenco), repr(dizionario))
        for oggetto, mutazioni in ((elenco, self.MUTAZIONI_DELL_ELENCO),
                                   (dizionario, self.MUTAZIONI_DEL_DIZIONARIO)):
            for nome, mutazione in mutazioni:
                with self.subTest(contenitore=type(oggetto).__name__, via=nome):
                    with self.assertRaises(PlenoraError) as preso:
                        mutazione(oggetto)
                    self.assertIsInstance(preso.exception, InvalidArgumentError)
        self.assertEqual((hash(elenco), hash(dizionario), repr(elenco), repr(dizionario)), prima)

    @prova("confine.rete")
    def test_reinizializzare_non_modifica(self) -> None:
        """Prima: `drivers.__init__([])` e `raw.__init__({...})` svuotavano o
        riscrivevano i contenitori (ereditavano `__init__` da `list` e `dict`)."""
        elenco, dizionario = self.contenitori()
        prima = (hash(elenco), hash(dizionario))
        for chiamata in (lambda: elenco.__init__([]), lambda: dizionario.__init__({"x": 1})):
            try:
                chiamata()
            except PlenoraError:
                pass
        self.assertEqual((hash(elenco), hash(dizionario)), prima)
        self.assertEqual(elenco, [1, [2], {"a": 3}])
        self.assertEqual(dizionario, {"k": [1], "j": {"x": 2}})
        versione = Version.from_json({"component_version": "4.2.0", "cli_protocol_version": 2})
        with self.assertRaises(PlenoraError) as preso:
            versione.__init__(component_version="9", cli_protocol_version=2)
        self.assertIsInstance(preso.exception, InvalidArgumentError)
        self.assertEqual(versione.component_version, "4.2.0")

    @prova("argomenti.sottoclassi")
    def test_la_costruzione_diretta_normalizza_le_foglie(self) -> None:
        """Prima: `FrozenList([oggetto])` conservava l'oggetto del chiamante."""

        class Mutevole:
            def __init__(self):
                self.stato = 1

            def __hash__(self):
                return self.stato

        for costruisci in (lambda v: FrozenList([v]), lambda v: FrozenDict({"k": v})):
            with self.assertRaises(PlenoraError) as preso:
                costruisci(Mutevole())
            self.assertIsInstance(preso.exception, InvalidArgumentError)
        Contatore.chiamate = 0
        elenco = FrozenList([StrOstile("a"), [IntOstile(2)]])
        dizionario = FrozenDict({StrOstile("k"): StrOstile("v")})
        self.assertIs(type(elenco[0]), str)
        self.assertIs(type(elenco[1]), FrozenList)
        self.assertIs(type(elenco[1][0]), int)
        self.assertIs(type(dizionario["k"]), str)
        hash(elenco), hash(dizionario), repr(elenco), elenco == ["a", [2]]
        self.assertEqual(Contatore.chiamate, 0)

    @prova("confine.rete")
    def test_copie_e_pickle_ricostruiscono_contenitori_immutabili(self) -> None:
        import copy as copia
        import pickle

        elenco, dizionario = self.contenitori()
        catalogo = Catalog.from_json(catalogo_sano())
        strade = [("copy", copia.copy), ("deepcopy", copia.deepcopy)] + [
            (f"pickle {protocollo}",
             lambda x, p=protocollo: pickle.loads(pickle.dumps(x, protocol=p)))
            for protocollo in range(pickle.HIGHEST_PROTOCOL + 1)
        ]

        def tutti_immutabili(valore):
            tipo = type(valore)
            self.assertIsNot(tipo, list)
            self.assertIsNot(tipo, dict)
            if tipo is FrozenList:
                for elemento in valore:
                    tutti_immutabili(elemento)
            elif tipo is FrozenDict:
                for elemento in valore.values():
                    tutti_immutabili(elemento)

        for nome, strada in strade:
            with self.subTest(strada=nome):
                for originale in (elenco, dizionario):
                    ricostruito = strada(originale)
                    self.assertIs(type(ricostruito), type(originale))
                    self.assertEqual(ricostruito, originale)
                    self.assertEqual(hash(ricostruito), hash(originale))
                    tutti_immutabili(ricostruito)
                    with self.assertRaises(PlenoraError):
                        ricostruito.clear()
                copiato = strada(catalogo)
                self.assertEqual(copiato, catalogo)
                self.assertIs(type(copiato.drivers), FrozenList)
                self.assertIs(type(copiato.raw), FrozenDict)
                with self.assertRaises(PlenoraError):
                    copiato.drivers.append(None)


class LePassateSistematiche(unittest.TestCase):
    """Sesto giro: identita' dei tipi, ricerche, errori costruiti a mano."""

    @prova("argomenti.sottoclassi")
    def test_una_metaclasse_che_mente_non_passa_dal_costruttore_diretto(self) -> None:
        """Prima: `tipo in (bool, str, int, float)` chiedeva alla metaclasse se
        il tipo fosse `str`, e il `Camaleonte` restava nel campo."""
        from test_tipi_esatti_e_copie import Camaleonte

        valore = Camaleonte("4.2.0")
        versione = Version(component_version=valore, cli_protocol_version=2)
        self.assertIs(type(versione.component_version), str)
        valore.valore = "altro"
        self.assertEqual(versione.component_version, "4.2.0")

    @prova("argomenti.sottoclassi")
    def test_ogni_ricerca_normalizza_prima_di_confrontare(self) -> None:
        """Prima: `LossReport.count()` confrontava l'argomento originale."""
        from plenora_io import LossReport

        from test_convert import perdita_sana

        Contatore.chiamate = 0
        perdita = LossReport.from_json(perdita_sana())

        class SempreUguale(str):
            def __eq__(self, altro):
                Contatore.chiamate += 1
                return True

            __hash__ = str.__hash__

        self.assertEqual(perdita.count(SempreUguale("non-c-e")), 0)
        for chiama in (
            lambda: Layers.from_json(layers_sano()).layer(StrOstile("non-c-e")),
            lambda: ConvertResult.from_json(conversione_sana()).layer(StrOstile("non-c-e")),
        ):
            with self.assertRaises(KeyError):
                chiama()
        self.assertEqual(Contatore.chiamate, 0)
        with self.assertRaises(PlenoraError):
            perdita.count(12)

    @prova("messaggi.senza_dati")
    def test_command_failed_non_formatta_l_exit_code_originale(self) -> None:
        """Prima: il messaggio usava `exit_code` originale, e il suo
        `__format__` entrava nel testo dell'eccezione."""
        from plenora_io import CommandFailed, ErrorEnvelope

        class CodiceOstile(int):
            def __format__(self, specifica):
                return SEGRETO

            __str__ = __repr__ = lambda self: SEGRETO

        busta = ErrorEnvelope(code="X", category="io", phase="write", remote_effect="none",
                              retry={"kind": "never"}, message="m")
        errore = CommandFailed(busta, CodiceOstile(4), ["write"])
        self.assertNotIn(SEGRETO, str(errore))
        self.assertNotIn(SEGRETO, repr(errore.args))
        self.assertIs(type(errore.exit_code), int)
        self.assertIn("uscito con 4", str(errore))

    @prova("messaggi.senza_dati")
    def test_binary_not_found_conserva_solo_i_posti_noti(self) -> None:
        """Prima: `searched` conservava e riportava testo arbitrario."""
        from plenora_io import BinaryNotFound

        errore = BinaryNotFound([f"COLONNA={SEGRETO}", "il `PATH`", StrOstile("il `PATH`")])
        self.assertEqual(errore.searched, ["?", "il `PATH`", "?"])
        self.assertNotIn(SEGRETO, str(errore))
        self.assertNotIn(SEGRETO, repr(vars(errore)))


class IMessaggi(unittest.TestCase):
    @prova("messaggi.senza_dati")
    def test_le_ricerche_nei_modelli_non_nominano_niente(self) -> None:
        documento = dict(inspect_sano())
        esito = Inspect.from_json(documento)
        catalogo = Catalog.from_json(catalogo_sano())
        capacita = Capabilities.from_json(capacita_sane())
        for chiama in (
            lambda: esito.layer(SEGRETO),
            lambda: esito.layers[0].field(SEGRETO),
            lambda: catalogo.driver(SEGRETO),
            lambda: capacita.operation(SEGRETO),
            lambda: Layers.from_json(layers_sano()).layer(SEGRETO),
            lambda: ConvertResult.from_json(conversione_sana()).layer(SEGRETO),
        ):
            with self.assertRaises(KeyError) as preso:
                chiama()
            self.assertIsInstance(preso.exception, PlenoraError)
            tradotto(self, preso.exception, "none")
            for nome in ("canonico", "codice", "csv", "io.read"):
                self.assertNotIn(nome, str(preso.exception))

    @prova("messaggi.senza_dati")
    def test_la_scoperta_non_riporta_percorsi(self) -> None:
        radice = Path(tempfile.mkdtemp()) / SEGRETO
        (radice / "bin").mkdir(parents=True)
        binario = radice / "bin" / "plenora-io"
        binario.write_bytes(b"")
        (radice / "MANIFEST.json").write_bytes(b"{")
        with self.assertRaises(PlenoraError) as preso:
            discovery.leggi_manifesto(binario)
        tradotto(self, preso.exception, "none")
        with mock.patch.dict(os.environ, {discovery.VARIABILE: str(radice / "no"),
                                          "PATH": str(radice)}), \
                mock.patch.object(discovery, "_albero_accanto_al_pacchetto", return_value=None):
            with self.assertRaises(PlenoraError) as preso:
                discovery.trova_binario(radice / "nemmeno")
        tradotto(self, preso.exception, "none")

    @prova("messaggi.senza_dati")
    def test_gli_attributi_degli_errori_non_portano_dati(self) -> None:
        """Prima: `CommandFailed.argv` conservava la riga intera, e
        `ProfileError.required` il valore del chiamante."""
        busta = {
            "status": "error",
            "error": {"code": "X", "category": "io", "phase": "write",
                      "remote_effect": "none", "retry": {"kind": "never"}, "message": "m"},
        }
        errore = failure_from_envelope(busta, 4, ["write", f"/{SEGRETO}.arrow", "o.gpkg"])
        self.assertFalse(hasattr(errore, "argv"))
        self.assertEqual(errore.command, "write")
        self.assertNotIn(SEGRETO, repr(vars(errore)))
        cliente = Client.__new__(Client)
        cliente._manifest = None
        with self.assertRaises(PlenoraError) as preso:
            cliente.require_profile(SEGRETO)
        self.assertNotIn(SEGRETO, repr(vars(preso.exception)))

    @prova("messaggi.senza_dati")
    def test_un_primo_argomento_fuori_elenco_non_compare(self) -> None:
        """Prima: qualunque primo argomento alfanumerico era «sicuro»."""
        finto = PopenFinto(stdout=b"non e' JSON")
        with mock.patch.object(subprocess, "Popen", finto):
            with self.assertRaises(PlenoraError) as preso:
                Runner(Path("plenora-io")).run([SEGRETO.replace("-", "")])
        self.assertNotIn(SEGRETO.replace("-", ""), str(preso.exception))
        self.assertIn("`plenora-io ?`", str(preso.exception))
        busta = {
            "status": "error",
            "error": {"code": "X", "category": "io", "phase": "write",
                      "remote_effect": "none", "retry": {"kind": "never"}, "message": "m"},
        }
        errore = failure_from_envelope(busta, 4, ["SEGRETO9c1e"])
        self.assertNotIn("SEGRETO9c1e", str(errore))
        self.assertIsNone(errore.command)

    @prova("argomenti.grandezze")
    def test_limits_si_normalizza_nel_costruttore(self) -> None:
        """Prima: `Limits(max_rows=1 << 64).to_argv()` rendeva un intero fuori
        intervallo, e `to_argv` leggeva le proprieta' di una `timedelta`
        derivata."""
        for valore in (1 << 64, 10**1000, True, "5"):
            with self.subTest(valore=type(valore).__name__):
                with self.assertRaises(PlenoraError) as preso:
                    Limits(max_rows=valore)
                self.assertIsInstance(preso.exception, InvalidArgumentError)
                tradotto(self, preso.exception, "none")
        Contatore.chiamate = 0
        limiti = Limits(deadline=DurataOstile(seconds=2), max_rows=IntOstile(3))
        self.assertEqual(limiti.to_argv(), ["--deadline-ms", "2000", "--max-rows", "3"])
        self.assertIs(type(limiti.deadline), timedelta)
        self.assertEqual(Contatore.chiamate, 0)

    @prova("messaggi.senza_dati")
    def test_require_profile_non_riporta_il_valore(self) -> None:
        cliente = Client.__new__(Client)
        cliente._manifest = None
        with self.assertRaises(PlenoraError) as preso:
            cliente.require_profile(SEGRETO)
        tradotto(self, preso.exception, "none")


# --- 10. settimo giro di revisione ------------------------------------------


def _busta(**campi):
    from plenora_io import ErrorEnvelope

    valori = {
        "code": "IO-X", "category": "io", "phase": "execute", "remote_effect": "none",
        "retry": {"kind": "never"}, "message": "m",
    }
    valori.update(campi)
    return ErrorEnvelope(**valori)


class IndiceOstile:
    """Un `__index__` che solleva un'eccezione con un dato del chiamante."""

    def __init__(self, errore):
        self.errore = errore

    def __index__(self):
        raise self.errore


class IntCheConta(int):
    def __index__(self):
        Contatore.chiamate += 1
        return 0


class IlSettimoGiro(unittest.TestCase):
    """Settimo giro: `__post_init__`, costruttori dei contenitori, indici."""

    @prova("confine.rete")
    def test_post_init_non_riapre_la_mutabilita_della_busta(self) -> None:
        """Prima: `busta.__post_init__()` rimetteva `retry` come `dict`, e una
        seconda chiamata dopo `retry["kind"] = "safe"` riscriveva la decisione
        privata: un `CommandFailed` gia' costruito diventava ritentabile."""
        import dataclasses

        from plenora_io import ErrorEnvelope

        busta = _busta()
        errore = failure_from_envelope(
            {"error": {"code": "IO-X", "category": "io", "phase": "execute",
                       "remote_effect": "none", "retry": {"kind": "never"},
                       "message": "m"}}, 1, ["read"])
        for oggetto in (busta, errore.envelope):
            prima = hash(oggetto)
            with self.assertRaises(PlenoraError) as preso:
                oggetto.__post_init__()
            self.assertIsInstance(preso.exception, InvalidArgumentError)
            tradotto(self, preso.exception, "none")
            self.assertIs(type(oggetto.retry), FrozenDict)
            with self.assertRaises(PlenoraError):
                oggetto.retry["kind"] = "safe"
            with self.assertRaises(PlenoraError):
                oggetto.__post_init__()
            self.assertEqual(hash(oggetto), prima)
            self.assertEqual(oggetto.retry, {"kind": "never"})
        self.assertFalse(errore.retryable)
        self.assertEqual(errore.retry, {"kind": "never"})
        # Con la diagnostica riga per riga: anche quella resta immutabile.
        diagnostica = _busta(row_diagnostics={"righe": [{"colonna": "c"}]})
        self.assertIs(type(diagnostica.row_diagnostics), FrozenDict)
        with self.assertRaises(PlenoraError):
            diagnostica.__post_init__()
        self.assertIs(type(diagnostica.row_diagnostics), FrozenDict)
        self.assertIs(type(diagnostica.row_diagnostics["righe"]), FrozenList)
        hash(diagnostica)
        # `replace` ricostruisce, e quindi resta ammesso.
        sostituita = dataclasses.replace(busta, retry={"kind": "safe"})
        self.assertEqual(sostituita.retry, {"kind": "safe"})
        self.assertIs(type(sostituita), ErrorEnvelope)

    @prova("confine.rete")
    def test_nessun_post_init_di_un_modello_si_richiama(self) -> None:
        """La stessa classe su ogni modello che definisce `__post_init__`."""
        from plenora_io.confine import MODELLI

        istanze = {"ErrorEnvelope": _busta(), "Limits": Limits(max_rows=3)}
        con_post_init = sorted(c.__name__ for c in MODELLI if "__post_init__" in vars(c))
        self.assertEqual(con_post_init, sorted(istanze))
        for nome, istanza in istanze.items():
            with self.subTest(modello=nome):
                prima = (hash(istanza), repr(istanza))
                with self.assertRaises(PlenoraError) as preso:
                    istanza.__post_init__()
                self.assertIsInstance(preso.exception, InvalidArgumentError)
                self.assertEqual((hash(istanza), repr(istanza)), prima)

    @prova("messaggi.senza_dati")
    def test_i_costruttori_dei_contenitori_rifiutano_senza_dati(self) -> None:
        """Prima: `FrozenDict(**{chiave: 1})` usciva come `TypeError` di
        binding, con la chiave nel messaggio: `__new__` era fuori dalla rete."""
        tipizzate = (
            ("FrozenDict kw", lambda: FrozenDict(**{SEGRETO: 1})),
            ("FrozenList kw", lambda: FrozenList(**{SEGRETO: 1})),
            ("FrozenDict due", lambda: FrozenDict({}, {SEGRETO: 1})),
            ("FrozenList due", lambda: FrozenList([], [SEGRETO])),
            ("__new__ kw", lambda: FrozenList.__new__(FrozenList, **{SEGRETO: 1})),
        )
        # `__reduce__` non ha argomenti: un binding sbagliato e' della rete.
        di_rete = (
            ("__reduce__ kw", lambda: FrozenList([1]).__reduce__(**{SEGRETO: 1})),
            ("__reduce__ dict", lambda: FrozenDict({}).__reduce__(SEGRETO)),
        )
        for nome, chiamata in tipizzate + di_rete:
            with self.subTest(chiamata=nome):
                with self.assertRaises(PlenoraError) as preso:
                    chiamata()
                tradotto(self, preso.exception, "none")
                if (nome, chiamata) in tipizzate:
                    self.assertIsInstance(preso.exception, InvalidArgumentError)

    @prova("argomenti.sottoclassi")
    def test_slice_e_index_normalizzano_i_componenti(self) -> None:
        """Prima: i componenti di uno slice e gli argomenti di `index()` erano
        passati a `tuple` com'erano, e il loro `__index__` girava: un
        `InvalidArgumentError` con un dato del chiamante usciva intatto."""
        elenco = FrozenList([1, 2, 3, 2])
        for errore in (InvalidArgumentError(SEGRETO), ValueError(SEGRETO),
                       ProtocolError(SEGRETO)):
            ostile = IndiceOstile(errore)
            chiamate = (
                lambda o=ostile: elenco[o:],
                lambda o=ostile: elenco[:o],
                lambda o=ostile: elenco[::o],
                lambda o=ostile: elenco.index(2, o),
                lambda o=ostile: elenco.index(2, 0, o),
            )
            for posizione, chiamata in enumerate(chiamate):
                with self.subTest(errore=type(errore).__name__, via=posizione):
                    with self.assertRaises(PlenoraError) as preso:
                        chiamata()
                    self.assertIsInstance(preso.exception, InvalidArgumentError)
                    tradotto(self, preso.exception, "none")
        rifiutate = (
            lambda: elenco[::0],
            lambda: elenco[1.0:],
            lambda: elenco[True:],
            lambda: elenco.index(2, 0, 4, 5),
            lambda: elenco.index(2, **{SEGRETO: 1}),
            lambda: elenco.index(2, "0"),
        )
        for posizione, chiamata in enumerate(rifiutate):
            with self.subTest(rifiutata=posizione):
                with self.assertRaises(PlenoraError) as preso:
                    chiamata()
                self.assertIsInstance(preso.exception, InvalidArgumentError)
                tradotto(self, preso.exception, "none")
        # Un valore presente ma fuori dall'intervallo e' una ricerca fallita.
        with self.assertRaises(PlenoraError) as preso:
            elenco.index(1, 1)
        self.assertEqual(type(preso.exception).__name__, "ResultLookupError")
        tradotto(self, preso.exception, "none")
        # Una sottoclasse di `int`: il suo `__index__` non gira, vale il numero.
        Contatore.chiamate = 0
        self.assertEqual(elenco[IntCheConta(1):IntCheConta(3)], [2, 3])
        self.assertEqual(elenco[::IntCheConta(2)], [1, 3])
        self.assertEqual(elenco.index(2, IntCheConta(2)), 3)
        self.assertEqual(elenco.index(2, None, IntCheConta(2)), 1)
        self.assertEqual(elenco[None:None:-1], [2, 3, 2, 1])
        self.assertEqual(elenco[-10:10], [1, 2, 3, 2])
        self.assertEqual(Contatore.chiamate, 0)

    @prova("confine.rete")
    def test_un_errore_dell_sdk_sollevato_da_codice_esterno_si_traduce(self) -> None:
        """Un `PlenoraError` che nasce fuori dall'SDK -- un `__index__`, un
        `__fspath__`, un `side_effect` -- porta il testo di chi l'ha scritto:
        la rete lo traduce come ogni altra eccezione esterna."""
        from plenora_io.confine import _confinato

        def esterna():
            raise InvalidArgumentError(SEGRETO)

        with self.assertRaises(PlenoraError) as preso:
            _confinato(esterna, "prova")()
        self.assertEqual(type(preso.exception).__name__, "UnexpectedError")
        tradotto(self, preso.exception, "none")
        # Un errore dell'SDK nato nell'SDK passa com'e'.
        with self.assertRaises(PlenoraError) as preso:
            FrozenList([1])[5]
        self.assertEqual(type(preso.exception).__name__, "ResultLookupError")


# --- 11. ottavo giro di revisione -------------------------------------------


def _un_esemplare_per_classe():
    """Un'istanza costruita dall'SDK per ogni classe d'errore di `errors`."""
    from plenora_io import errors

    busta = _busta()
    speciali = {
        "BinaryNotFound": lambda c: c(["il `PATH`"]),
        "ProfileError": lambda c: c("base", None),
        "CleanupError": lambda c: c("m", remote_effect="committed"),
        "UnexpectedError": lambda c: c("m"),
    }
    for nome, classe in vars(errors).items():
        if not (type(classe) is type and issubclass(classe, PlenoraError)):
            continue
        if issubclass(classe, CommandFailed):
            yield nome, classe, (lambda c=classe: c(busta, 3, ["read"]))
        else:
            costruisci = speciali.get(nome, lambda c: c("m"))
            yield nome, classe, (lambda c=classe, f=costruisci: f(c))


class LOttavoGiro(unittest.TestCase):
    """Ottavo giro: la rete ricostruisce, e lo stato delle eccezioni e' fisso."""

    @prova("messaggi.senza_dati")
    def test_un_errore_dell_sdk_alterato_da_una_callback_non_porta_il_testo(self) -> None:
        """Prima: la rete certificava dove l'errore era nato, non il contenuto.
        Una callback prendeva un `ResultLookupError` dell'SDK, ne cambiava
        `args` e lo rilanciava: il testo usciva."""
        from plenora_io import ResultLookupError
        from plenora_io.confine import _confinato

        def callback():
            try:
                FrozenDict({})["assente"]
            except ResultLookupError as errore:
                errore.args = (SEGRETO,)
                errore.__notes__ = [SEGRETO]  # come `add_note`, anche su 3.10
                raise

        with self.assertRaises(PlenoraError) as preso:
            _confinato(callback, "prova")()
        errore = preso.exception
        self.assertIs(type(errore), ResultLookupError)
        self.assertIsInstance(errore, KeyError)
        tradotto(self, errore, "none")
        self.assertNotIn(SEGRETO, repr(errore))
        self.assertNotIn(SEGRETO, repr(errore.args))
        self.assertFalse(getattr(errore, "__notes__", None))
        self.assertEqual(str(errore), "chiave assente dal dizionario.")

    @prova("confine.rete")
    def test_la_rete_non_dipende_dai_percorsi(self) -> None:
        """Prima: un `_CARTELLA` diverso dal `co_filename` -- PyInstaller, un
        percorso relativo dopo un `chdir`, il bytecode altrove -- faceva di
        ogni errore dell'SDK un `UnexpectedError`, e `FrozenDict.get` smetteva
        di rendere il valore predefinito."""
        from plenora_io import ResultLookupError, confine

        cartelle = (
            os.path.join(tempfile.gettempdir(), "_MEI123456", "plenora_io"),
            "plenora_io",
            os.path.join("altrove", "__pycache__"),
        )
        for cartella in cartelle:
            with self.subTest(cartella=cartella), \
                    mock.patch.object(confine, "_CARTELLA", cartella, create=True):
                self.assertEqual(FrozenDict({}).get("assente", "DEFAULT"), "DEFAULT")
                self.assertNotIn("assente", FrozenDict({}).keys())
                with self.assertRaises(KeyError) as preso:
                    FrozenDict({})["assente"]
                self.assertIs(type(preso.exception), ResultLookupError)
                with self.assertRaises(IndexError):
                    FrozenList([])[3]
                with self.assertRaises(PlenoraError) as preso:
                    Version.from_json({"component_version": 1, "cli_protocol_version": 2})
                self.assertIs(type(preso.exception), ProtocolError)

    @prova("confine.rete")
    def test_un_file_che_finge_di_essere_dell_sdk_non_basta(self) -> None:
        """Codice del chiamante compilato con un nome di file dentro il
        pacchetto: prima il suo errore passava come nato nell'SDK."""
        from plenora_io import confine
        from plenora_io.confine import _confinato

        finto = os.path.join(os.path.dirname(os.path.abspath(confine.__file__)), "finto.py")
        spazio = {"InvalidArgumentError": InvalidArgumentError, "SEGRETO": SEGRETO}
        sorgente = "def f():\n    raise InvalidArgumentError(SEGRETO)\n"
        exec(compile(sorgente, finto, "exec"), spazio)  # noqa: S102 - il codice del chiamante
        with self.assertRaises(PlenoraError) as preso:
            _confinato(spazio["f"], "prova")()
        self.assertEqual(type(preso.exception).__name__, "UnexpectedError")
        tradotto(self, preso.exception, "none")

    @prova("confine.rete")
    def test_lo_stato_di_command_failed_si_fissa_una_volta(self) -> None:
        """Prima: `errore.envelope = altra` o un secondo `__init__` cambiavano
        `retryable` di un'eccezione gia' costruita."""
        errore = failure_from_envelope(
            {"error": {"code": "IO-X", "category": "io", "phase": "execute",
                       "remote_effect": "none", "retry": {"kind": "never"},
                       "message": "m"}}, 1, ["read"])
        busta = errore.envelope
        testo = str(errore)
        altra = _busta(retry={"kind": "safe"})
        mutazioni = (
            ("envelope", lambda: setattr(errore, "envelope", altra)),
            ("del envelope", lambda: delattr(errore, "envelope")),
            ("__init__", lambda: errore.__init__(altra, 0)),
            ("exit_code", lambda: setattr(errore, "exit_code", 0)),
            ("command", lambda: setattr(errore, "command", "write")),
            ("effetto", lambda: setattr(errore, "_effetto_remoto", "committed")),
            ("messaggio", lambda: setattr(errore, "_messaggio", SEGRETO)),
            ("__dict__", lambda: setattr(errore, "__dict__", {})),
            ("nuovo", lambda: setattr(errore, "nuovo", 1)),
        )
        for nome, mutazione in mutazioni:
            with self.subTest(via=nome):
                with self.assertRaises(PlenoraError) as preso:
                    mutazione()
                self.assertIsInstance(preso.exception, InvalidArgumentError)
                self.assertNotIn(SEGRETO, str(preso.exception))
        self.assertIs(errore.envelope, busta)
        self.assertFalse(errore.retryable)
        self.assertEqual(errore.exit_code, 1)
        # Gli attributi che imposta l'interprete restano liberi; `args` non e'
        # piu' la fonte del messaggio.
        errore.__cause__ = None
        errore.__context__ = None
        errore.__suppress_context__ = True
        errore.__traceback__ = None
        errore.__notes__ = ["nota"]
        errore.args = ("altro",)
        self.assertEqual(str(errore), testo)
        self.assertEqual(errore.message, "m")

    @prova("confine.rete")
    def test_ogni_eccezione_dell_sdk_e_sigillata(self) -> None:
        """La stessa classe su ogni eccezione di `errors`."""
        visti = 0
        for nome, classe, costruisci in _un_esemplare_per_classe():
            with self.subTest(classe=nome):
                errore = costruisci()
                self.assertIs(type(errore), classe)
                prima = (str(errore), errore.to_dict())
                for mutazione in (lambda e=errore: e.__init__("m"),
                                  lambda e=errore: setattr(e, "nuovo", 1),
                                  lambda e=errore: setattr(e, "_messaggio", SEGRETO),
                                  lambda e=errore: delattr(e, "_messaggio")):
                    with self.assertRaises(PlenoraError) as preso:
                        mutazione()
                    self.assertIsInstance(preso.exception, InvalidArgumentError)
                self.assertEqual((str(errore), errore.to_dict()), prima)
                visti += 1
        self.assertEqual(visti, 31)

    @prova("confine.rete")
    def test_gli_attributi_delle_eccezioni_sono_immutabili(self) -> None:
        from plenora_io import BinaryNotFound, ProfileError

        errore = BinaryNotFound(["il `PATH`", SEGRETO])
        self.assertIs(type(errore.searched), FrozenList)
        self.assertEqual(errore.searched, ["il `PATH`", "?"])
        with self.assertRaises(PlenoraError):
            errore.searched.append(SEGRETO)
        with self.assertRaises(PlenoraError):
            errore.searched = [SEGRETO]
        profilo = ProfileError("base", "filegdb")
        for campo in ("required", "actual"):
            with self.subTest(campo=campo), self.assertRaises(PlenoraError):
                setattr(profilo, campo, SEGRETO)
        self.assertEqual((profilo.required, profilo.actual), ("base", "filegdb"))

    @prova("confine.rete")
    def test_copia_e_pickle_delle_eccezioni(self) -> None:
        import copy as copia
        import pickle

        for nome, classe, costruisci in _un_esemplare_per_classe():
            errore = costruisci()
            for strada, funzione in (("copy", copia.copy), ("deepcopy", copia.deepcopy),
                                     ("pickle", lambda x: pickle.loads(pickle.dumps(x)))):
                with self.subTest(classe=nome, strada=strada):
                    copiato = funzione(errore)
                    self.assertIs(type(copiato), classe)
                    self.assertEqual(str(copiato), str(errore))
                    self.assertEqual(copiato.to_dict(), errore.to_dict())
                    with self.assertRaises(PlenoraError):
                        copiato.nuovo = 1


# --- 12. nono giro di revisione ---------------------------------------------


class IlNonoGiro(unittest.TestCase):
    """Nono giro: nomi di classe nei messaggi, contesto, codice valutato."""

    def _senza_segreto(self, chiamata) -> PlenoraError:
        with self.assertRaises(PlenoraError) as preso:
            chiamata()
        errore = preso.exception
        self.assertIsInstance(errore, InvalidArgumentError)
        tradotto(self, errore)
        return errore

    @prova("messaggi.senza_dati")
    def test_il_nome_di_una_classe_del_chiamante_non_entra_nei_messaggi(self) -> None:
        """Prima: `type(self).__name__` nei messaggi di sigillo, di
        immutabilita' e di normalizzazione portava il nome scelto dal
        chiamante per la propria sottoclasse."""
        Errore = type(SEGRETO, (InvalidArgumentError,), {})
        errore = Errore("m")
        self.assertIn("InvalidArgumentError", str(
            self._senza_segreto(lambda: setattr(errore, "nuovo", 1))))
        self._senza_segreto(lambda: delattr(errore, "_messaggio"))
        self._senza_segreto(lambda: errore.__init__("m"))

        Versione = type(SEGRETO, (Version,), {})
        versione = Versione(component_version="4.2.0", cli_protocol_version=2)
        self.assertIn("Version", str(
            self._senza_segreto(lambda: setattr(versione, "component_version", "x"))))
        self._senza_segreto(lambda: delattr(versione, "component_version"))
        self._senza_segreto(
            lambda: versione.__init__(component_version="9", cli_protocol_version=2))
        self._senza_segreto(
            lambda: Versione(component_version=object(), cli_protocol_version=2))

        Limiti = type(SEGRETO, (Limits,), {})
        limiti = Limiti(max_rows=3)
        self._senza_segreto(lambda: limiti.__post_init__())

    @prova("confine.rete")
    def test_un_errore_dell_sdk_dentro_un_except_del_chiamante_non_ha_contesto(self) -> None:
        """Prima: l'errore sollevato dalla rete dentro un `except` del
        chiamante prendeva come `__context__` l'eccezione che il chiamante
        stava gestendo; `from None` lo nascondeva soltanto."""
        from plenora_io import ResultLookupError

        casi = (
            ("ricostruito", lambda: FrozenList([])[3], ResultLookupError),
            ("ricostruito per nome", lambda: FrozenDict({})["k"], ResultLookupError),
            ("invalid", lambda: Limits(max_rows="x"), InvalidArgumentError),
            ("unexpected", lambda: Catalog.from_json(object()), PlenoraError),
        )
        for nome, chiamata, classe in casi:
            with self.subTest(caso=nome):
                preso = None
                try:
                    raise ValueError(SEGRETO)
                except ValueError:
                    try:
                        chiamata()
                    except PlenoraError as errore:
                        preso = errore
                self.assertIsInstance(preso, classe)
                self.assertIsNone(preso.__context__)
                self.assertIsNone(preso.__cause__)
                self.assertTrue(preso.__suppress_context__)
                testo = "".join(traceback.format_exception(type(preso), preso, preso.__traceback__))
                self.assertNotIn(SEGRETO, testo)
        # Con `mock.patch`, un `side_effect` che solleva fuori dall'SDK.
        try:
            raise ValueError(SEGRETO)
        except ValueError:
            with mock.patch("plenora_io.models._copia_json", side_effect=RuntimeError("x")),                     self.assertRaises(PlenoraError) as preso:
                Catalog.from_json(catalogo_sano())
        self.assertIsNone(preso.exception.__context__)

    @prova("confine.rete")
    def test_codice_valutato_coi_globali_dell_sdk_non_e_dell_sdk(self) -> None:
        """Prima: `eval` senza globali propri eredita quelli del frame che lo
        chiama -- la rete, o un modulo dell'SDK -- e l'errore che costruisce
        passava per costruito dall'SDK, con il suo testo."""
        from plenora_io.confine import _confinato

        sorgente = (
            "(_ for _ in ()).throw("
            "__import__('plenora_io').InvalidArgumentError('" + SEGRETO + "'))"
        )
        with self.assertRaises(PlenoraError) as preso:
            _confinato(functools.partial(eval, sorgente), "prova")()
        self.assertEqual(type(preso.exception).__name__, "UnexpectedError")
        tradotto(self, preso.exception, "none")

        # Dal metodo pubblico: un `__fspath__` che e' un `eval` parziale gira
        # con i globali del modulo dell'SDK che chiama `os.fspath`.
        class Percorso:
            __fspath__ = functools.partial(eval, sorgente, None)

        with self.assertRaises(PlenoraError) as preso:
            Client(binary=Percorso())
        tradotto(self, preso.exception, "none")


# --- 13. decimo giro di revisione -------------------------------------------


class LaProvenienzaEsplicita(unittest.TestCase):
    """Decimo giro: un errore e' dell'SDK solo se l'SDK l'ha costruito con la
    sua fabbrica privata; il costruttore pubblico, chiamato in qualunque modo,
    fa un errore esterno."""

    def _costruiti_durante_una_chiamata_dell_sdk(self):
        """Errori costruiti mentre l'SDK esegue `os.fspath` sul binario, per
        quattro vie: `map` e `partial` in C, `eval`, una lambda, un metodo."""
        salvati = []
        sorgente = "__import__('plenora_io').InvalidArgumentError('" + SEGRETO + "')"

        class InC:
            __fspath__ = functools.partial(
                next, map(salvati.append, map(InvalidArgumentError, [SEGRETO])))

        class ConEval:
            __fspath__ = functools.partial(eval, "_s.append(" + sorgente + ")", None)

        class ConLambda:
            __fspath__ = lambda self: salvati.append(InvalidArgumentError(SEGRETO))  # noqa: E731

        class ConMetodo:
            def __fspath__(self):
                salvati.append(InvalidArgumentError(SEGRETO))

        vie = {"map e partial": InC, "eval": ConEval, "lambda": ConLambda, "metodo": ConMetodo}
        trovati = {}
        for nome, classe in vie.items():
            prima = len(salvati)
            oggetto = classe()
            if classe is ConEval:
                # `eval` con `locals` = l'oggetto: il nome `_s` e' l'elenco.
                oggetto = type("ConEvalLocale", (dict,), {"__fspath__": ConEval.__fspath__})(
                    _s=salvati)
            with self.assertRaises(PlenoraError):
                Client(binary=oggetto)
            self.assertEqual(len(salvati), prima + 1, nome)
            trovati[nome] = salvati[-1]
        return trovati

    @prova("messaggi.senza_dati")
    def test_un_errore_costruito_da_una_callback_non_e_dell_sdk(self) -> None:
        """Prima: il frame osservato era quello dell'SDK che eseguiva la
        callback, e un errore costruito da `map` in C contava come dell'SDK."""
        from plenora_io import errors

        for nome, errore in self._costruiti_durante_una_chiamata_dell_sdk().items():
            with self.subTest(via=nome):
                self.assertIs(type(errore), InvalidArgumentError)
                self.assertFalse(errors.costruito_dall_sdk(errore))

                class Versione(Version):
                    def __getattribute__(self, nome, _errore=errore):
                        raise _errore

                versione = object.__new__(Versione)
                with self.assertRaises(PlenoraError) as preso:
                    repr(versione)
                self.assertEqual(type(preso.exception).__name__, "UnexpectedError")
                tradotto(self, preso.exception, "none")

    @prova("confine.rete")
    def test_il_costruttore_pubblico_fa_sempre_un_errore_esterno(self) -> None:
        from plenora_io import errors

        for nome, classe, costruisci in _un_esemplare_per_classe():
            with self.subTest(classe=nome):
                self.assertFalse(errors.costruito_dall_sdk(costruisci()))
        # La fabbrica privata, invece, marca la provenienza.
        self.assertTrue(errors.costruito_dall_sdk(
            errors._dallo_sdk(InvalidArgumentError, "elenco.immutabile")))

    def test_ogni_errore_dell_sdk_nasce_dalla_fabbrica(self) -> None:
        """Inventario statico: nei sorgenti dell'SDK nessuna chiamata diretta a
        una classe d'errore, fuori dalla fabbrica `_dallo_sdk`. Una chiamata
        diretta farebbe un errore esterno, che la rete renderebbe
        `UnexpectedError` al posto della classe giusta."""
        import ast

        from plenora_io import errors

        nomi = {
            nome for nome, classe in vars(errors).items()
            if type(classe) is type and issubclass(classe, PlenoraError)
        }
        cartella = Path(errors.__file__).parent
        dirette = []
        for file in sorted(cartella.glob("*.py")):
            albero = ast.parse(file.read_text(encoding="utf-8"))
            for nodo in ast.walk(albero):
                if not isinstance(nodo, ast.Call):
                    continue
                funzione = nodo.func
                nome = (funzione.id if isinstance(funzione, ast.Name)
                        else funzione.attr if isinstance(funzione, ast.Attribute) else None)
                if nome in nomi:
                    dirette.append(f"{file.name}:{nodo.lineno}")
        self.assertEqual(dirette, [])
        # Le chiamate dinamiche (`classe(...)`) non le vede l'AST: l'unica e'
        # in `failure_from_envelope`, e deve passare dalla fabbrica.
        busta = {"error": {"code": "IO-X", "category": "io", "phase": "execute",
                           "remote_effect": "none", "retry": {"kind": "never"},
                           "message": "m"}}
        self.assertTrue(errors.costruito_dall_sdk(errors._failure_from_envelope(busta, 1, ["read"])))
        self.assertFalse(errors.costruito_dall_sdk(failure_from_envelope(busta, 1, ["read"])))


# --- 14. undicesimo giro di revisione ---------------------------------------

#: Ogni funzione e classe pubblica -- raggiungibile senza nomi privati -- dei
#: moduli dell'SDK che non sta in `plenora_io.__all__`, con il perche' non
#: puo' dare un errore fidato con dati del chiamante. Un elenco **chiuso**: una
#: funzione pubblica nuova e' rossa finche' non entra qui, e la prova la
#: esercita con argomenti che portano `SEGRETO`.
INVENTARIO_PUBBLICO: dict[str, str] = {
    "arrow.pyarrow": "nessun argomento; messaggi fissi",
    "arrow.e_un_percorso": "rende un bool",
    "arrow.scrivi_ipc": "messaggi fissi; il percorso non entra nel testo",
    "arrow.leggi_ipc": "messaggi fissi; percorso e content type non entrano nel testo",
    "arrow.cartella_temporanea": "costruisce `CartellaTemporanea`",
    "arrow.CartellaTemporanea": "effetto dal vocabolario chiuso; messaggi fissi",
    "client.con_superficie_python": "decoratore dei metodi del client",
    "confine.registra_avvio": "non solleva errori dell'SDK",
    "confine.nome_sdk": "rende il nome di una classe dell'SDK, o `?`",
    "confine.e_un_modello": "rende un bool",
    "discovery.trova_binario": "posti dall'elenco chiuso; messaggi fissi",
    "discovery.leggi_manifesto": "messaggi fissi",
    "discovery.verifica_profilo": "profili dall'elenco chiuso",
    "errors.sottocomando": "rende un sottocomando dall'elenco chiuso, o None",
    "errors.classe_dell_sdk": "rende un bool",
    "errors.costruito_dall_sdk": "rende un bool",
    "errors.ricostruito": "copia lo stato dell'errore ricevuto, senza sostituzioni",
    "errors.carica_json": "il messaggio dice posizione e motivo, mai il testo",
    "errors.failure_from_envelope": "rende un errore esterno: la busta e' del chiamante",
    "models.tipi_json": "non solleva errori dell'SDK",
    "process.sigint_inoltrabile": "rende un bool",
    "process.comando": "rende un sottocomando dall'elenco chiuso",
    "process.scrive": "rende un bool",
    "process.Completed": "un esito del processo, senza errori",
}


class _StrSegreta:
    def __str__(self):
        return SEGRETO

    __repr__ = __str__

    def __fspath__(self):
        return SEGRETO


def _valori_con_segreto():
    return (
        SEGRETO, [SEGRETO], {SEGRETO: SEGRETO}, Path(SEGRETO), _StrSegreta(), None, 7,
        {"error": {"code": SEGRETO, "category": "io", "phase": "execute",
                   "remote_effect": "none", "retry": {"kind": "never"}, "message": SEGRETO}},
    )


def _pubblici_dei_moduli():
    """Ogni funzione e classe pubblica dei moduli pubblici, fuori da `__all__`."""
    import importlib
    import inspect
    import pkgutil

    import plenora_io

    for modulo in pkgutil.iter_modules(plenora_io.__path__):
        if modulo.name.startswith("_"):
            continue
        spazio = importlib.import_module(f"plenora_io.{modulo.name}")
        for nome, valore in vars(spazio).items():
            if nome.startswith("_") or nome in plenora_io.__all__:
                continue
            if not callable(valore) or getattr(valore, "__module__", None) != spazio.__name__:
                continue
            if inspect.isclass(valore) or inspect.isfunction(valore):
                yield f"{modulo.name}.{nome}", valore


class LaFiduciaSenzaVieIndirette(unittest.TestCase):
    """Undicesimo giro: nessuna funzione raggiungibile senza nomi privati da'
    un errore fidato con dati del chiamante."""

    def _nessun_dato_se_fidato(self, errore, dove) -> None:
        from plenora_io import errors

        if not isinstance(errore, PlenoraError) or not errors.costruito_dall_sdk(errore):
            return
        testo = " ".join((str(errore), errore.message, repr(errore.to_dict()), repr(errore.args)))
        self.assertNotIn(SEGRETO, testo, dove)

    @prova("messaggi.senza_dati")
    def test_failure_from_envelope_pubblica_fa_errori_esterni(self) -> None:
        """Prima: la busta del chiamante diventava un `IoError` fidato, e la
        rete lo faceva uscire con il suo messaggio."""
        from plenora_io import errors

        busta = {"error": {"code": "IO-X", "category": "io", "phase": "execute",
                           "remote_effect": "none", "retry": {"kind": "never"},
                           "message": SEGRETO}}
        errore = failure_from_envelope(busta, 1, ["read"])
        self.assertEqual(type(errore).__name__, "IoError")
        self.assertFalse(errors.costruito_dall_sdk(errore))

        class Versione(Version):
            def __getattribute__(self, nome, _errore=errore):
                raise _errore

        with self.assertRaises(PlenoraError) as preso:
            repr(object.__new__(Versione))
        self.assertEqual(type(preso.exception).__name__, "UnexpectedError")
        tradotto(self, preso.exception, "none")

    @prova("messaggi.senza_dati")
    def test_ricostruito_pubblico_non_sostituisce_lo_stato(self) -> None:
        """Prima: `ricostruito(originale, envelope=busta)` conservava la
        fiducia e sostituiva lo stato, decisioni comprese."""
        from plenora_io import errors

        fidato = errors._dallo_sdk(InvalidArgumentError, "elenco.immutabile")
        busta = _busta(message=SEGRETO, retry={"kind": "safe"})
        try:
            copia = errors.ricostruito(fidato, envelope=busta)
        except PlenoraError as errore:
            self.assertIsInstance(errore, InvalidArgumentError)
            self.assertNotIn(SEGRETO, str(errore))
        else:
            self.assertFalse(errors.costruito_dall_sdk(copia))
        copia = errors.ricostruito(fidato)
        self.assertTrue(errors.costruito_dall_sdk(copia))
        self.assertEqual(str(copia), str(fidato))

    def test_la_ricostruzione_privata_accetta_solo_lo_stato_dell_sdk(self) -> None:
        from plenora_io import errors

        fidato = errors._dallo_sdk_strutturato(CommandFailed, _busta(), 1, ["read"])
        for voce in ("envelope", "_messaggio", "exit_code", "command", "_dall_sdk"):
            with self.subTest(voce=voce), self.assertRaises(InvalidArgumentError):
                errors._ricostruito(fidato, **{voce: SEGRETO})

    @prova("confine.rete")
    def test_str_message_e_to_dict_vengono_dallo_stesso_stato(self) -> None:
        import copy as copia

        from plenora_io import errors

        for nome, classe, costruisci in _un_esemplare_per_classe():
            originale = costruisci()
            for strada, errore in (("costruito", originale),
                                   ("ricostruito", errors.ricostruito(originale)),
                                   ("copia", copia.copy(originale))):
                with self.subTest(classe=nome, strada=strada):
                    self.assertEqual(errore.to_dict()["message"], errore.message)
                    # `message` e' contenuto in `str()`, non uguale: per
                    # `CommandFailed` `str()` antepone sottocomando, uscita,
                    # categoria, fase e codice, come documenta il README
                    # («Gli errori si distinguono per categoria»); per gli
                    # altri errori i due testi coincidono.
                    self.assertIn(errore.message, str(errore))
                    if not isinstance(errore, CommandFailed):
                        self.assertEqual(errore.message, str(errore))
                    self.assertEqual(str(errore), str(originale))

    @prova("messaggi.senza_dati")
    def test_l_inventario_delle_funzioni_pubbliche_e_chiuso(self) -> None:
        """Ogni funzione e classe pubblica dei moduli dell'SDK fuori da
        `__all__` e' nell'inventario, e nessuna da' un errore fidato con un
        dato del chiamante: si chiama con ogni combinazione di argomenti che
        portano `SEGRETO` (fino a tre posizionali e i keyword-only)."""
        import inspect
        import itertools

        trovati = dict(_pubblici_dei_moduli())
        with self.subTest(elenco="chiuso"):
            self.assertEqual(sorted(trovati), sorted(INVENTARIO_PUBBLICO))
        valori = _valori_con_segreto()
        cartella = tempfile.mkdtemp()
        precedente = os.getcwd()
        os.chdir(cartella)
        try:
            for nome, funzione in sorted(trovati.items()):
                firma = inspect.signature(funzione)
                parametri = [
                    p for p in firma.parameters.values()
                    if p.kind in (p.POSITIONAL_ONLY, p.POSITIONAL_OR_KEYWORD, p.KEYWORD_ONLY)
                    and p.name != "self"
                ][:3]
                for combinazione in itertools.product(valori, repeat=len(parametri)):
                    posizionali = [v for p, v in zip(parametri, combinazione)
                                   if p.kind != p.KEYWORD_ONLY]
                    nominali = {p.name: v for p, v in zip(parametri, combinazione)
                                if p.kind == p.KEYWORD_ONLY}
                    with self.subTest(funzione=nome):
                        try:
                            risultato = funzione(*posizionali, **nominali)
                            if isinstance(risultato, arrow_cartella_tipo()):
                                with risultato:
                                    pass
                        except PlenoraError as errore:
                            self._nessun_dato_se_fidato(errore, nome)
                        except Exception:  # noqa: BLE001, S112 - non e' un errore dell'SDK
                            continue
                        else:
                            self._nessun_dato_se_fidato(risultato, nome)
        finally:
            os.chdir(precedente)


def arrow_cartella_tipo():
    return adattatore.CartellaTemporanea


# --- 15. dodicesimo giro di revisione ---------------------------------------

#: Le classi di `__all__` che l'inventario a combinazioni non chiama, e perche'.
NON_CHIAMATE_A_COMBINAZIONI: dict[str, str] = {
    "Client": "avvia il binario: i suoi argomenti sono provati da `GliArgomenti`",
    "Runner": "avvia il binario: `argv` e' provato da `test_confini_sdk`",
}


def _segreto_come(valore):
    """Un valore della stessa forma di `valore`, con `SEGRETO` dentro."""
    if isinstance(valore, tuple):
        return (SEGRETO,)
    if isinstance(valore, frozenset):
        return frozenset({SEGRETO})
    if isinstance(valore, dict):
        return {SEGRETO: SEGRETO}
    return SEGRETO


def _sottoclasse_ostile(classe):
    """`classe` derivata, con nome `SEGRETO` e ogni attributo di classe non
    chiamabile delle classi dell'SDK nel suo MRO ridefinito a `SEGRETO`."""
    ridefiniti = {}
    for base in classe.__mro__:
        if not base.__module__.startswith("plenora_io"):
            continue
        for nome, valore in vars(base).items():
            if nome.startswith("__") or callable(valore) or isinstance(
                    valore, (property, classmethod, staticmethod)):
                continue
            ridefiniti.setdefault(nome, _segreto_come(valore))
    try:
        return type(SEGRETO, (classe,), ridefiniti)
    except Exception:  # noqa: BLE001 - una classe che non si deriva non ha il rischio
        return None


def _esemplari_dei_modelli():
    """Un'istanza per ogni modello che i documenti sani raggiungono."""
    import dataclasses

    from plenora_io import Capabilities, LossReport

    from test_convert import perdita_sana

    radici = [
        Catalog.from_json(catalogo_sano()), Inspect.from_json(inspect_sano()),
        Layers.from_json(layers_sano()), Validation.from_json(validazione_sana()),
        Version.from_json({"component_version": "4.2.0", "cli_protocol_version": 2}),
        Capabilities.from_json(capacita_sane()), LossReport.from_json(perdita_sana()),
        WriteResult.from_json(scrittura_sana()), ConvertResult.from_json(conversione_sana()),
        Limits(max_rows=3), _busta(),
    ]
    trovati = {}

    def visita(valore):
        if dataclasses.is_dataclass(valore) and not isinstance(valore, type):
            trovati.setdefault(type(valore), valore)
            for campo in dataclasses.fields(valore):
                visita(object.__getattribute__(valore, "__dict__").get(campo.name))
        elif isinstance(valore, (FrozenList, list, tuple)):
            for interno in valore:
                visita(interno)
        elif isinstance(valore, (FrozenDict, dict)):
            for interno in valore.values():
                visita(interno)

    for radice in radici:
        visita(radice)
    from plenora_io import Delivered, Manifest, errors

    from test_discovery import MANIFESTO_SANO

    trovati[Delivered] = Delivered.from_json(consegna_sana())
    trovati[Manifest] = Manifest.from_json(MANIFESTO_SANO)
    trovati[FrozenList] = FrozenList([1, [2], {"k": 3}])
    trovati[FrozenDict] = FrozenDict({"k": [1], "j": {"x": 2}})
    if pa is not None:
        from plenora_io import TableRead

        trovati[TableRead] = TableRead(
            table=pa.table({"x": [1, 2]}), result=Validation.from_json(validazione_sana()))
    # Gli errori, fidati: nati dalla fabbrica dell'SDK.
    for nome, classe, costruisci in _un_esemplare_per_classe():
        esemplare = costruisci()
        fidato = errors._ricostruito(esemplare)
        object.__getattribute__(fidato, "__dict__")[errors.PROVENIENZA] = True
        trovati[classe] = fidato
    return trovati


def _istanza_come(bersaglio, esemplare):
    """Un'istanza di `bersaglio` con lo stato di `esemplare`, o None."""
    if issubclass(bersaglio, BaseException):
        istanza = BaseException.__new__(bersaglio)
        BaseException.__init__(istanza, *esemplare.args)
    elif bersaglio is type(esemplare):
        return esemplare
    elif issubclass(bersaglio, (FrozenList, FrozenDict)):
        # Non si estendono: una sottoclasse non ha istanze.
        return None
    else:
        istanza = object.__new__(bersaglio)
    object.__getattribute__(istanza, "__dict__").update(
        object.__getattribute__(esemplare, "__dict__"))
    return istanza


class LoSchemaDellaClasseDellSdk(unittest.TestCase):
    """Dodicesimo giro: gli attributi di una sottoclasse non entrano mai in un
    errore fidato."""

    def _nessun_dato_se_fidato(self, errore, dove) -> None:
        LaFiduciaSenzaVieIndirette._nessun_dato_se_fidato(self, errore, dove)

    @prova("messaggi.senza_dati")
    def test_obbligatori_di_una_sottoclasse_non_entrano_nel_messaggio(self) -> None:
        """Prima: `from_json` leggeva `cls.OBBLIGATORI`, e i nomi mancanti
        entravano in un `ProtocolError` fidato."""
        from plenora_io import FormatDescriptor

        for classe in (Catalog, FormatDescriptor, Inspect, Layers):
            with self.subTest(classe=classe.__name__):
                Ostile = type("Ostile", (classe,), {"OBBLIGATORI": (SEGRETO,)})
                with self.assertRaises(PlenoraError) as preso:
                    Ostile.from_json({})
                self.assertIs(type(preso.exception), ProtocolError)
                tradotto(self, preso.exception)

    @prova("messaggi.senza_dati")
    def test_l_inventario_copre_i_metodi_delle_classi_esportate(self) -> None:
        """Ogni metodo pubblico, `__init__`, classmethod, staticmethod e
        proprieta' delle classi di `__all__` -- sulla classe e su una
        sottoclasse con ogni attributo di classe ridefinito a `SEGRETO` --
        chiamato con ogni combinazione di argomenti che portano `SEGRETO` (fino
        a tre, posizionali e keyword-only) non da' un errore fidato che lo
        contenga. `__init__` si chiama anche non legato, su un oggetto
        estraneo. Un'eccezione che non e' un `PlenoraError` e' un fallimento:
        un metodo pubblico passa dalla rete."""
        import inspect
        import itertools

        import plenora_io

        valori = _valori_con_segreto()
        esemplari = _esemplari_dei_modelli()
        Estraneo = _estraneo()
        chiamate = 0
        estranee: list[str] = []

        def chiama(etichetta, funzione, posizionali, nominali):
            nonlocal chiamate
            chiamate += 1
            try:
                risultato = funzione(*posizionali, **nominali)
            except PlenoraError as errore:
                self._nessun_dato_se_fidato(errore, etichetta)
            except Exception as errore:  # noqa: BLE001 - si contano, e sono rosse
                estranee.append(f"{etichetta}: {type(errore).__name__}")
            else:
                self._nessun_dato_se_fidato(risultato, etichetta)

        def combinazioni(funzione, salta=0):
            try:
                firma = inspect.signature(funzione)
            except (TypeError, ValueError):
                return
            parametri = [
                p for p in list(firma.parameters.values())[salta:]
                if p.kind in (p.POSITIONAL_ONLY, p.POSITIONAL_OR_KEYWORD, p.KEYWORD_ONLY)
            ][:3]
            for valori_scelti in itertools.product(valori, repeat=len(parametri)):
                posizionali = [v for p, v in zip(parametri, valori_scelti)
                               if p.kind != p.KEYWORD_ONLY]
                nominali = {p.name: v for p, v in zip(parametri, valori_scelti)
                            if p.kind == p.KEYWORD_ONLY}
                yield posizionali, nominali

        for nome in plenora_io.__all__:
            classe = getattr(plenora_io, nome)
            if not isinstance(classe, type) or nome in NON_CHIAMATE_A_COMBINAZIONI:
                continue
            ostile = _sottoclasse_ostile(classe)
            for bersaglio in (classe, ostile):
                if bersaglio is None:
                    continue
                istanza = None
                if classe in esemplari:
                    istanza = _istanza_come(bersaglio, esemplari[classe])
                # `__init__` non legato: su un oggetto estraneo e sull'istanza.
                for primo in (Estraneo(), istanza):
                    if primo is None:
                        continue
                    for posizionali, nominali in combinazioni(bersaglio.__init__, salta=1):
                        chiama(f"{nome}.__init__", bersaglio.__init__,
                               [primo, *posizionali], nominali)
                for base in classe.__mro__:
                    if not base.__module__.startswith("plenora_io"):
                        continue
                    for attributo, valore in vars(base).items():
                        if attributo.startswith("_"):
                            continue
                        etichetta = f"{nome}.{attributo}"
                        if isinstance(valore, (classmethod, staticmethod)):
                            funzione = getattr(bersaglio, attributo)
                        elif inspect.isfunction(valore) and istanza is not None:
                            funzione = getattr(istanza, attributo)
                        elif isinstance(valore, property) and istanza is not None:
                            chiama(etichetta, getattr, [istanza, attributo], {})
                            continue
                        else:
                            continue
                        for posizionali, nominali in combinazioni(funzione):
                            chiama(etichetta, funzione, posizionali, nominali)
        self.assertEqual(estranee, [])
        self.assertGreater(chiamate, 5000)


# --- 16. tredicesimo giro di revisione --------------------------------------

#: Un dato del chiamante che puo' essere un identificatore Python: il nome di
#: un campo o di un attributo.
SEGRETO_NOME = "SEGRETOcampo9c1e"


def _estraneo():
    """Lo scenario di Codex: una dataclass del chiamante con un campo il cui
    nome e' un dato, e una proprieta' `drivers` che scrive in quel campo."""
    import dataclasses

    spazio = {}
    exec(  # noqa: S102 - la dataclass del chiamante, con un nome di campo dato
        "import dataclasses\n"
        "@dataclasses.dataclass(init=False)\n"
        "class Estraneo:\n"
        f"    {SEGRETO_NOME}: object = None\n"
        "    @property\n"
        "    def drivers(self):\n"
        f"        return self.{SEGRETO_NOME}\n"
        "    @drivers.setter\n"
        "    def drivers(self, valore):\n"
        f"        object.__setattr__(self, '{SEGRETO_NOME}', valore)\n",
        spazio,
    )
    assert dataclasses.is_dataclass(spazio["Estraneo"])
    return spazio["Estraneo"]


class IlCatalogoDeiMessaggi(unittest.TestCase):
    """Tredicesimo giro: la fabbrica accetta solo chiavi e parametri dal
    vocabolario chiuso; i metodi rifiutano un `self` estraneo."""

    def _senza_dati(self, testo, dove) -> None:
        self.assertNotIn(SEGRETO, testo, dove)
        self.assertNotIn(SEGRETO_NOME, testo, dove)

    @prova("messaggi.senza_dati")
    def test_un_costruttore_chiamato_su_un_oggetto_estraneo_si_rifiuta(self) -> None:
        """Prima: `Catalog.__init__(Estraneo(), ...)` girava, e `_normalizza`
        metteva il nome del campo del chiamante in un errore fidato."""
        from plenora_io import errors

        Estraneo = _estraneo()
        with self.assertRaises(PlenoraError) as preso:
            Catalog.__init__(Estraneo(), determinism="x", drivers=object())
        self._senza_dati(str(preso.exception), "costruttore")
        self.assertIsInstance(preso.exception, InvalidArgumentError)
        tradotto(self, preso.exception, "none")
        self.assertEqual(str(preso.exception), errors.MESSAGGI["rete.self_estraneo"])
        for metodo in (Catalog.driver, Catalog.__repr__, Catalog.__eq__, Catalog.__hash__):
            with self.subTest(metodo=metodo.__name__), self.assertRaises(InvalidArgumentError):
                metodo(Estraneo(), *([None] if metodo in (Catalog.driver, Catalog.__eq__) else []))

    @prova("messaggi.senza_dati")
    def test_una_sottoclasse_con_campi_ridefiniti_non_entra_nei_messaggi(self) -> None:
        """La stessa via da `from_json` di una sottoclasse che ridefinisce un
        campo come proprieta' e i campi della dataclass."""
        import dataclasses

        from plenora_io import errors

        Estraneo = _estraneo()
        campo = dataclasses.field()
        campo.name = SEGRETO_NOME
        campo._field_type = dataclasses._FIELD  # come lo marca `@dataclass`
        Derivata = type("Derivata", (Catalog,), {
            "drivers": Estraneo.drivers,
            "__dataclass_fields__": {SEGRETO_NOME: campo, **Catalog.__dataclass_fields__},
        })
        self.assertEqual(dataclasses.fields(Derivata)[0].name, SEGRETO_NOME)
        documento = catalogo_sano()
        documento["drivers"] = [object()]
        for chiamata in (lambda: Derivata.from_json(documento),
                         lambda: Derivata(determinism="x", drivers=[object()]),
                         lambda: hash(Derivata(determinism="x", drivers=[])),
                         lambda: Derivata(determinism="x", drivers=[]) == Catalog.from_json(
                             catalogo_sano())):
            try:
                chiamata()
            except PlenoraError as errore:
                if errors.costruito_dall_sdk(errore):
                    self._senza_dati(str(errore), "derivata")

    @prova("messaggi.senza_dati")
    def test_ogni_chiave_con_parametri_ostili(self) -> None:
        """Per ogni chiave di `MESSAGGI`, ogni parametro del modello con ogni
        forma di dato del chiamante: il testo non lo contiene, e l'errore
        nasce comunque (`?` al posto del parametro)."""
        import string

        from plenora_io import errors

        class StrSegreta(str):
            pass

        ostili = (
            SEGRETO, SEGRETO_NOME, StrSegreta("catalog"), f"catalog.{SEGRETO_NOME}",
            [SEGRETO], (SEGRETO, "null"), {SEGRETO: 1}, _StrSegreta(), Path(SEGRETO),
            True, b"SEGRETO-9c1e", InvalidArgumentError(SEGRETO),
        )
        for chiave, modello in errors.MESSAGGI.items():
            campi = [c for _, c, _, _ in string.Formatter().parse(modello) if c]
            for ostile in ostili:
                with self.subTest(chiave=chiave, ostile=type(ostile).__name__):
                    errore = errors._dallo_sdk(
                        InvalidArgumentError, chiave, **{c: ostile for c in campi})
                    self.assertIs(type(errore), InvalidArgumentError)
                    self.assertTrue(errors.costruito_dall_sdk(errore))
                    self._senza_dati(str(errore), chiave)
                    self.assertEqual(str(errore).count("?") >= len(campi), True)
        # Una chiave fuori catalogo, o un parametro in piu', non e' un crash.
        errore = errors._dallo_sdk(InvalidArgumentError, SEGRETO, extra=SEGRETO)
        self.assertEqual(str(errore), errors.MESSAGGI["sdk.sconosciuto"])
        # I parametri del vocabolario si scrivono come sono.
        errore = errors._dallo_sdk(
            ProtocolError, "protocollo.tipo_campo",
            dove="catalog.drivers[]", campo="id", tipo="integer",
            ammessi=("una stringa", "null"))
        self.assertEqual(
            str(errore),
            "catalog.drivers[].id e' integer e non una stringa o null, come il "
            "protocollo v2 lo dichiara.")

    def test_ogni_chiamata_alla_fabbrica_usa_una_chiave_del_catalogo(self) -> None:
        """Inventario sull'AST: ogni `_dallo_sdk` nei sorgenti ha come chiave
        un letterale di `MESSAGGI`; ogni `_dallo_sdk_strutturato` costruisce
        `BinaryNotFound`, `ProfileError` o -- in `_failure_from_envelope` --
        la classe della busta."""
        import ast

        from plenora_io import errors

        cartella = Path(errors.__file__).parent
        difformi = []
        for file in sorted(cartella.glob("*.py")):
            albero = ast.parse(file.read_text(encoding="utf-8"))
            for funzione in ast.walk(albero):
                if not isinstance(funzione, ast.FunctionDef):
                    continue
                for nodo in ast.walk(funzione):
                    if not (isinstance(nodo, ast.Call) and isinstance(nodo.func, ast.Name)):
                        continue
                    dove = f"{file.name}:{nodo.lineno}"
                    if nodo.func.id == "_dallo_sdk":
                        chiave = nodo.args[1] if len(nodo.args) > 1 else None
                        if not (isinstance(chiave, ast.Constant)
                                and chiave.value in errors.MESSAGGI):
                            difformi.append(dove)
                    elif nodo.func.id == "_dallo_sdk_strutturato":
                        classe = nodo.args[0]
                        ammessa = isinstance(classe, ast.Name) and (
                            classe.id in ("BinaryNotFound", "ProfileError")
                            or (classe.id == "classe"
                                and funzione.name == "_failure_from_envelope"))
                        if not ammessa:
                            difformi.append(dove)
        self.assertEqual(difformi, [])


if __name__ == "__main__":
    unittest.main()
