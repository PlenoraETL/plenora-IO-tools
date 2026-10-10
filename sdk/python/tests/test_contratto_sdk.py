"""Il pacchetto contro `plenora-python-sdk-v1`: version, capabilities, errori, Arrow.

# Che cosa verificano queste sonde

PYTHON-SDK-1.0 fissa cio' che ogni SDK Plenora espone, qualunque sia il
componente. Qui si prova cio' che quel contratto chiede e che le altre sonde non
coprivano:

* §2 -- `version()` rende i metadati installati;
* §6 -- i cinque assi stanno su **ogni** eccezione pubblica, con i valori di
  `error-v1`, e un errore di protocollo dopo una scrittura non dice `none`;
* §7 -- `capabilities()` rende il documento `capabilities-v2` tipizzato;
* §12 -- ogni metodo del dominio corrisponde a un'operazione annunciata;
* §3 -- l'adattatore Arrow facoltativo, con pyarrow e senza.

Le sonde dell'adattatore che hanno bisogno di pyarrow saltano dove pyarrow non
c'e' -- l'installazione dalla sdist, che non porta extra -- e girano nel job
`python-sdk` della CI, che installa la serie fissata.
"""

from __future__ import annotations

import inspect
import json
import os
import shutil
import sys
import tempfile
import unittest
from importlib import metadata
from pathlib import Path
from unittest import mock

import plenora_io
from plenora_io import (
    Capabilities,
    Client,
    CommandFailed,
    InvalidArgumentError,
    OptionalDependencyError,
    PlenoraError,
    ProtocolError,
)
from plenora_io.client import OPERAZIONI
from plenora_io.discovery import NOME, VARIABILE

from _repository import CANONICHE, RADICE, serve_le_fixture
from test_client import finto
from test_convert import fedelta_sana, perdita_sana
from test_models import consegna_sana, validazione_sana

try:
    import pyarrow as pa
    import pyarrow.ipc  # noqa: F401
except ImportError:  # pragma: no cover - dipende dall'ambiente
    pa = None

#: I vocabolari chiusi di `error-v1.schema.json` (plenora-contracts).
CATEGORIE = {
    "invalid_plan", "invalid_configuration", "schema", "data_mapping", "crs",
    "unsupported", "not_found", "conflict", "concurrent_modification",
    "authentication", "authorization", "timeout", "cancelled",
    "resource_limit", "io", "protocol", "transient", "execution", "internal",
}
FASI = {
    "validate", "connect", "probe", "prepare", "read", "write", "finalize",
    "commit", "rollback", "cleanup",
}
EFFETTI = {"none", "rolled_back", "partial", "committed", "unknown"}

serve_pyarrow = unittest.skipUnless(
    pa is not None,
    "serve pyarrow, l'extra `plenora-io[pyarrow]`, che l'installazione dalla sdist non porta",
)
posix = unittest.skipIf(sys.platform == "win32", "il finto e' uno script POSIX")


def binario_vero() -> str | None:
    indicato = os.environ.get(VARIABILE)
    trovato = indicato or shutil.which(NOME)
    if trovato is None and RADICE is not None:
        costruito = RADICE / "target" / "debug" / NOME
        trovato = str(costruito) if costruito.is_file() else None
    return trovato


def capacita_sane() -> dict:
    """Un documento `capabilities-v2` nella forma che il binario emette."""
    operazione = {
        "id": "io.catalog",
        "version": 1,
        "status": "available",
        "surfaces": ["cli", "rust"],
        "input": {"contract": "plenora-io-catalog-query-v1", "content_types": ["application/json"]},
        "output": {"contract": "plenora-io-catalog-v1", "content_types": ["application/json"]},
        "side_effect": "none",
        "controls": {"cancellation": False, "deadline": False, "idempotency_key": False},
    }
    lettura = dict(operazione, id="io.read", attributes={"materialization": "bounded"})
    return {
        "schema_version": 2,
        "component": "plenora-io-tools",
        "component_version": "4.1.1",
        "interfaces": [
            {"kind": "cli", "contract": "plenora-cli-v2", "version": 2, "artifact": "plenora-io"}
        ],
        "operations": [operazione, lettura],
    }


def scrittura_sana() -> dict:
    """Un risultato di `io.write` nella forma del protocollo corrente."""
    vuota = perdita_sana(lossless=True, counts=[], esempi=[])
    return {
        "format": "csv",
        "input": {
            "content_type": "application/vnd.apache.arrow.file",
            "interchange_contract": "plenora-arrow-interchange-v1",
        },
        "layers": [{"name": "ingresso", "rows": 3, "batches": 1}],
        "rows_written": 3,
        "bytes_written": 42,
        "publish_outcome": "published",
        "fidelity": fedelta_sana(),
        "input_fidelity": fedelta_sana(),
        "write_fidelity": fedelta_sana(),
        "input_loss": vuota,
        "write_loss": vuota,
    }


def busta_ok(risultato: dict) -> str:
    return json.dumps({"status": "ok", "protocol_version": 2, "result": risultato})


class LaVersioneEQuellaInstallata(unittest.TestCase):
    """§2: `version()` e i metadati sono la stessa cosa."""

    def test_version_rende_i_metadati_installati(self) -> None:
        try:
            installata = metadata.version(plenora_io.DISTRIBUTION)
        except metadata.PackageNotFoundError:
            self.skipTest("pacchetto non installato: si prova dalla wheel, nello smoke")
        self.assertEqual(plenora_io.version(), installata)
        self.assertEqual(plenora_io.version(), plenora_io.__version__)


class LeCapacitaSiDecodificano(unittest.TestCase):
    """§7: il documento `capabilities-v2`, con gli assi leggibili senza testo."""

    def test_il_documento_si_decodifica_tipizzato(self) -> None:
        capacita = Capabilities.from_json(capacita_sane())
        catalogo = capacita.operation("io.catalog")
        self.assertEqual(catalogo.version, 1)
        self.assertEqual(catalogo.surfaces, ["cli", "rust"])
        self.assertEqual(catalogo.output.contract, "plenora-io-catalog-v1")
        self.assertFalse(catalogo.controls.deadline)
        self.assertTrue(catalogo.available)
        self.assertIsNone(catalogo.attributes)
        self.assertEqual(capacita.operation("io.read").attributes, {"materialization": "bounded"})

    def test_un_altro_schema_non_si_indovina(self) -> None:
        with self.assertRaises(ProtocolError):
            Capabilities.from_json(dict(capacita_sane(), schema_version=3))

    def test_un_controllo_mancante_e_un_errore(self) -> None:
        documento = capacita_sane()
        del documento["operations"][0]["controls"]["deadline"]
        with self.assertRaises(ProtocolError):
            Capabilities.from_json(documento)

    def test_un_operazione_ignota_e_keyerror(self) -> None:
        with self.assertRaises(KeyError):
            Capabilities.from_json(capacita_sane()).operation("io.nulla")


class IMetodiSonoOperazioni(unittest.TestCase):
    """§12: ogni metodo del dominio ha la sua operazione, e nessuna e' inventata."""

    #: I metodi pubblici che non invocano il dominio: la scoperta e il profilo.
    SCOPERTA = {"version", "capabilities", "require_profile"}

    def test_ogni_metodo_pubblico_e_mappato(self) -> None:
        metodi = {
            nome
            for nome, valore in inspect.getmembers(Client, inspect.isfunction)
            if not nome.startswith("_")
        }
        self.assertEqual(metodi - self.SCOPERTA, set(OPERAZIONI))

    def test_le_operazioni_sono_quelle_del_catalogo(self) -> None:
        self.assertEqual(
            set(OPERAZIONI.values()),
            {"io.catalog", "io.inspect", "io.layers", "io.read", "io.write", "io.convert"},
        )


class GliAssiStannoSuOgniEccezione(unittest.TestCase):
    """§6: i cinque assi, con i valori di `error-v1`, su ogni eccezione pubblica."""

    def assi_validi(self, errore: PlenoraError) -> None:
        documento = errore.to_dict()
        self.assertIn(documento["category"], CATEGORIE)
        self.assertIn(documento["phase"], FASI)
        self.assertIn(documento["remote_effect"], EFFETTI)
        self.assertIsInstance(documento["retry"], dict)
        self.assertIn("kind", documento["retry"])
        self.assertTrue(documento["message"])
        if documento["remote_effect"] == "unknown":
            self.assertIn(documento["retry"]["kind"], {"never", "quarantine", "requires_recovery"})

    def test_gli_errori_dell_sdk(self) -> None:
        for errore in (
            plenora_io.BinaryNotFound(["PATH"]),
            plenora_io.ManifestError("manifesto illeggibile"),
            plenora_io.ProfileError("filegdb", "base"),
            ProtocolError("risposta fuori protocollo"),
            InvalidArgumentError("argomento"),
            OptionalDependencyError("dipendenza"),
        ):
            with self.subTest(classe=type(errore).__name__):
                self.assi_validi(errore)
                self.assertEqual(errore.remote_effect, "none")
                self.assertEqual(errore.retry, {"kind": "never"})

    def test_ogni_classe_pubblica_e_coperta(self) -> None:
        """Una classe d'errore nuova non sfugge: o ha gli assi, o e' qui."""
        for nome in plenora_io.__all__:
            oggetto = getattr(plenora_io, nome)
            if isinstance(oggetto, type) and issubclass(oggetto, BaseException):
                with self.subTest(classe=nome):
                    self.assertTrue(issubclass(oggetto, PlenoraError))
                    for asse in ("category", "phase", "remote_effect", "retry", "message"):
                        self.assertTrue(hasattr(oggetto, asse))

    def test_command_failed_porta_gli_assi_della_busta(self) -> None:
        busta = {
            "status": "error",
            "error": {
                "code": "X",
                "category": "transient",
                "phase": "write",
                "remote_effect": "none",
                "retry": {"kind": "after", "delay_ms": 10},
                "message": "riprova",
            },
        }
        errore = plenora_io.errors.failure_from_envelope(busta, 5, ["catalog"])
        self.assi_validi(errore)
        self.assertEqual(
            errore.to_dict(),
            {
                "category": "transient",
                "phase": "write",
                "remote_effect": "none",
                "retry": {"kind": "after", "delay_ms": 10},
                "message": "riprova",
                "code": "X",
            },
        )
        # L'asse e' la decisione validata, non il campo che il chiamante tiene.
        errore.envelope.retry["kind"] = "safe"
        self.assertEqual(errore.retry["kind"], "after")


class LEffettoDiUnaRispostaIlleggibile(unittest.TestCase):
    """Una risposta che non si legge, dopo una scrittura, non dice `none`."""

    def setUp(self) -> None:
        self._temporanea = tempfile.TemporaryDirectory(prefix="plenora-sdk-effetto-")
        self.addCleanup(self._temporanea.cleanup)
        self.tmp = Path(self._temporanea.name)
        self.cliente = Client(binary=finto(self.tmp, 'print("non JSON")\n'))

    @posix
    def test_dopo_write_l_effetto_e_ignoto(self) -> None:
        with self.assertRaises(ProtocolError) as preso:
            self.cliente.write(self.tmp / "in.arrow", self.tmp / "out.csv", format="csv")
        self.assertEqual(preso.exception.remote_effect, "unknown")
        self.assertEqual(preso.exception.retry, {"kind": "never"})

    @posix
    def test_dopo_una_lettura_l_effetto_e_nessuno(self) -> None:
        with self.assertRaises(ProtocolError) as preso:
            self.cliente.catalog()
        self.assertEqual(preso.exception.remote_effect, "none")

    @posix
    def test_un_risultato_che_non_si_decodifica_dopo_write(self) -> None:
        cliente = Client(binary=finto(self.tmp, f"print({busta_ok({'format': 'csv'})!r})\n"))
        with self.assertRaises(ProtocolError) as preso:
            cliente.write(self.tmp / "in.arrow", self.tmp / "out.csv", format="csv")
        self.assertEqual(preso.exception.remote_effect, "unknown")


class LeOpzioniDiWrite(unittest.TestCase):
    """`options` va a entrambi i driver anche in `write()`, come in `convert()`."""

    @posix
    def test_options_diventa_opt(self) -> None:
        with tempfile.TemporaryDirectory(prefix="plenora-sdk-opzioni-") as cartella:
            tmp = Path(cartella)
            argv = tmp / "argv.json"
            corpo = (
                f"open({str(argv)!r}, 'w').write(json.dumps(sys.argv[1:]))\n"
                f"print({busta_ok(scrittura_sana())!r})\n"
            )
            cliente = Client(binary=finto(tmp, corpo))
            cliente.write(
                tmp / "in.arrow",
                tmp / "out.csv",
                format="csv",
                options={"comune": "1"},
                read_options={"lettore": "2"},
                write_options={"sink": "3"},
            )
            ricevuti = json.loads(argv.read_text(encoding="utf-8"))
        coppie = list(zip(ricevuti, ricevuti[1:]))
        self.assertIn(("--opt", "comune=1"), coppie)
        self.assertIn(("--in-opt", "lettore=2"), coppie)
        self.assertIn(("--out-opt", "sink=3"), coppie)
        self.assertNotIn(("--in-opt", "comune=1"), coppie)


class SenzaPyarrow(unittest.TestCase):
    """L'adattatore senza la dipendenza: `unsupported`, prima di eseguire."""

    def test_read_table_si_rifiuta_prima_di_eseguire(self) -> None:
        cliente = Client.__new__(Client)  # nessun binario: non deve servire
        with mock.patch.dict(sys.modules, {"pyarrow": None, "pyarrow.ipc": None}):
            with self.assertRaises(OptionalDependencyError) as preso:
                cliente.read_table("qualunque.geojson")
        self.assertEqual(preso.exception.category, "unsupported")
        self.assertIn("plenora-io[pyarrow]", str(preso.exception))

    def test_write_da_un_oggetto_si_rifiuta_prima_di_eseguire(self) -> None:
        cliente = Client.__new__(Client)
        with mock.patch.dict(sys.modules, {"pyarrow": None, "pyarrow.ipc": None}):
            with self.assertRaises(OptionalDependencyError):
                cliente.write(object(), "uscita.csv", format="csv")

    def test_un_altra_serie_si_rifiuta(self) -> None:
        finta = mock.MagicMock(__version__="26.0.0")
        with mock.patch.dict(sys.modules, {"pyarrow": finta, "pyarrow.ipc": finta.ipc}):
            with self.assertRaises(OptionalDependencyError) as preso:
                plenora_io.arrow.pyarrow()
        self.assertIn("25.x", str(preso.exception))


def tabella():
    return pa.table({"id": pa.array([1, 2, 3], pa.int64()), "nome": ["a", "b", "c"]})


@serve_pyarrow
class LAdattatoreContrUnFinto(unittest.TestCase):
    """L'adattatore con pyarrow, contro un binario finto che copia i file.

    Il finto consegna un IPC preparato qui e conserva quello che riceve: si
    verifica il tratto fra l'oggetto Python e il file, che e' cio' che
    l'adattatore aggiunge.
    """

    def setUp(self) -> None:
        self._temporanea = tempfile.TemporaryDirectory(prefix="plenora-sdk-arrow-")
        self.addCleanup(self._temporanea.cleanup)
        self.tmp = Path(self._temporanea.name)
        self.preparato = self.tmp / "preparato.arrow"
        with pa.OSFile(str(self.preparato), "wb") as uscita:
            with pa.ipc.new_file(uscita, tabella().schema) as scrittore:
                scrittore.write_table(tabella())
        self.ricevuto = self.tmp / "ricevuto.arrow"
        lettura = validazione_sana(delivered=consegna_sana())
        corpo = (
            "import shutil\n"
            "argv = sys.argv[1:]\n"
            "if argv[0] == 'read':\n"
            f"    shutil.copyfile({str(self.preparato)!r}, argv[argv.index('--output') + 1])\n"
            f"    print({busta_ok(lettura)!r})\n"
            "else:\n"
            f"    shutil.copyfile(argv[1], {str(self.ricevuto)!r})\n"
            f"    print({busta_ok(scrittura_sana())!r})\n"
        )
        self.cliente = Client(binary=finto(self.tmp, corpo))

    @posix
    def test_read_table_rende_la_tabella_e_la_busta(self) -> None:
        esito = self.cliente.read_table(self.tmp / "sorgente.geojson", temp_dir=self.tmp)
        self.assertTrue(esito.table.equals(tabella()))
        self.assertEqual(esito.result.delivered.content_type, "application/vnd.apache.arrow.file")
        # Il file temporaneo non resta.
        self.assertEqual(sorted(p.name for p in self.tmp.iterdir() if p.is_dir()), [])

    @posix
    def test_write_da_una_tabella(self) -> None:
        esito = self.cliente.write(tabella(), self.tmp / "out.csv", format="csv", temp_dir=self.tmp)
        self.assertEqual(esito.rows_written, 3)
        with pa.OSFile(str(self.ricevuto), "rb") as sorgente:
            self.assertTrue(pa.ipc.open_file(sorgente).read_all().equals(tabella()))

    @posix
    def test_write_da_un_record_batch_e_da_un_reader(self) -> None:
        batch = tabella().to_batches()[0]
        for oggetto in (batch, pa.RecordBatchReader.from_batches(batch.schema, [batch])):
            with self.subTest(tipo=type(oggetto).__name__):
                self.cliente.write(oggetto, self.tmp / "out.csv", format="csv")
                with pa.OSFile(str(self.ricevuto), "rb") as sorgente:
                    self.assertTrue(pa.ipc.open_file(sorgente).read_all().equals(tabella()))

    def test_un_errore_di_pyarrow_non_attraversa_il_confine(self) -> None:
        guasto = self.tmp / "guasto.arrow"
        guasto.write_bytes(b"non e' Arrow")
        with self.assertRaises(ProtocolError) as preso:
            plenora_io.arrow.leggi_ipc(guasto, "application/vnd.apache.arrow.file")
        self.assertIsInstance(preso.exception.__cause__, pa.ArrowException)

    def test_un_oggetto_che_non_e_arrow_si_rifiuta(self) -> None:
        cliente = Client.__new__(Client)
        with self.assertRaises(InvalidArgumentError):
            cliente.write(object(), self.tmp / "out.csv", format="csv")


@serve_le_fixture
@serve_pyarrow
class LAdattatoreContrIlBinarioVero(unittest.TestCase):
    """Il giro completo, quando c'e' un binario: file canonico -> tabella -> file."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.binario = binario_vero()

    def setUp(self) -> None:
        if self.binario is None:
            self.skipTest("nessun binario plenora-io da esercitare")
        self._temporanea = tempfile.TemporaryDirectory(prefix="plenora-sdk-arrow-vero-")
        self.addCleanup(self._temporanea.cleanup)
        self.tmp = Path(self._temporanea.name)
        self.cliente = Client(binary=self.binario)

    def test_read_table_e_write_chiudono_il_giro(self) -> None:
        letta = self.cliente.read_table(CANONICHE / "canonico.geojson")
        self.assertGreater(letta.table.num_rows, 0)
        self.assertEqual(letta.table.num_rows, letta.result.rows_read)
        esito = self.cliente.write(letta.table, self.tmp / "giro.csv", format="csv")
        self.assertEqual(esito.rows_written, letta.table.num_rows)


@serve_le_fixture
class LeCapacitaDelBinarioVero(unittest.TestCase):
    """§7 e §12 contro il prodotto: ogni operazione mappata e' annunciata."""

    def setUp(self) -> None:
        binario = binario_vero()
        if binario is None:
            self.skipTest("nessun binario plenora-io da esercitare")
        self.cliente = Client(binary=binario)

    def test_capabilities_annuncia_ogni_operazione_mappata(self) -> None:
        capacita = self.cliente.capabilities()
        self.assertEqual(capacita.schema_version, 2)
        for metodo, operazione in OPERAZIONI.items():
            with self.subTest(metodo=metodo):
                self.assertTrue(capacita.operation(operazione).available)


if __name__ == "__main__":
    unittest.main()
