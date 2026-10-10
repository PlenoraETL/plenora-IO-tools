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
from plenora_io.errors import copia_json, failure_from_envelope
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
        def eseguita(*argomenti, **opzioni):
            esito = funzione(*argomenti, **opzioni)
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
            copia_json(profondo, "prova")
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

        def read(sorgente, uscita, **opzioni):
            Path(uscita).write_bytes(b"non e' IPC " + SEGRETO.encode())
            documento = validazione_sana() if consegna is None else validazione_sana(
                delivered=consegna_sana())
            return Validation.from_json(documento)

        cliente.read = read
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

OSTILI = {
    "percorso": [FspathRotto(), 12, NUL, b"/dati/x", object()],
    "testo": [StrRotto(), 12, NUL, b"x", object()],
    "intero": ["1", 1.5, True, StrRotto()],
    "opzioni": [1, ["k"], MappaRotta(k="v"), {"k": StrRotto()}, {1: "v"},
                {f"k\x00{SEGRETO}": "v"}, {"k": NUL}, {"a=b": "c"}],
    "limiti": [1, Limits(max_rows="10"), Limits(max_rows=True),
               Limits(deadline=5), Limits(max_rows=StrRotto())],
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
    @prova("confine.rete")
    def test_ogni_metodo_pubblico_e_avvolto(self) -> None:
        """Per introspezione: un metodo pubblico nuovo senza rete e' rosso."""
        import plenora_io

        scoperti = 0
        for nome in plenora_io.__all__:
            valore = getattr(plenora_io, nome)
            if isinstance(valore, type):
                if issubclass(valore, BaseException):
                    continue
                for classe in valore.__mro__:
                    if not classe.__module__.startswith("plenora_io"):
                        continue
                    for attributo, interno in vars(classe).items():
                        if attributo.startswith("_") and attributo != "__init__":
                            continue
                        if isinstance(interno, (classmethod, staticmethod)):
                            bersaglio = interno.__func__
                        elif isinstance(interno, property):
                            bersaglio = interno.fget
                        elif callable(interno) and not isinstance(interno, type):
                            bersaglio = interno
                        else:
                            continue
                        with self.subTest(metodo=f"{classe.__name__}.{attributo}"):
                            self.assertTrue(_avvolto(bersaglio))
                            scoperti += 1
            elif callable(valore):
                with self.subTest(funzione=nome):
                    self.assertTrue(_avvolto(valore))
                    scoperti += 1
        self.assertGreater(scoperti, 60)

    @prova("confine.rete")
    def test_un_eccezione_non_prevista_prima_dell_avvio_e_none(self) -> None:
        with mock.patch("plenora_io.models.copia_json", side_effect=RuntimeError(SEGRETO)):
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
        with mock.patch("plenora_io.models.copia_json", side_effect=KeyboardInterrupt):
            with self.assertRaises(KeyboardInterrupt):
                Catalog.from_json(catalogo_sano())


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
    def test_require_profile_non_riporta_il_valore(self) -> None:
        cliente = Client.__new__(Client)
        cliente._manifest = None
        with self.assertRaises(PlenoraError) as preso:
            cliente.require_profile(SEGRETO)
        tradotto(self, preso.exception, "none")


if __name__ == "__main__":
    unittest.main()
