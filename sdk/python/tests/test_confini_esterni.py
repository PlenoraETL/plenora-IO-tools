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
import json
import subprocess
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
    PlenoraError,
    ProtocolError,
    Validation,
    Version,
    WriteResult,
)
from plenora_io import arrow as adattatore
from plenora_io import discovery
from plenora_io.errors import failure_from_envelope
from plenora_io.process import Runner

from test_contratto_sdk import capacita_sane, pa, scrittura_sana, serve_pyarrow
from test_convert import conversione_sana
from test_models import catalogo_sano, consegna_sana, inspect_sano, layers_sano, validazione_sana

SEGRETO = "SEGRETO-9c1e"

#: L'inventario: dove nasce un'eccezione esterna, e in che cosa si traduce.
#: Una riga per punto; `PuntiInventariati` verifica che ognuno abbia una prova.
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
}

PROVATI: set[str] = set()


def prova(punto: str):
    """Registra che il test esercita `punto` dell'inventario."""
    assert punto in PUNTI, punto
    PROVATI.add(punto)

    def decora(funzione):
        return funzione

    return decora


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
        profondo: object = 0
        for _ in range(600):
            profondo = [profondo]
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
                        self.assertIsInstance(preso.exception, InvalidArgumentError)
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
    def test_ogni_punto_dell_inventario_ha_una_prova(self) -> None:
        self.assertEqual(set(PUNTI) - PROVATI, set())


if __name__ == "__main__":
    unittest.main()
