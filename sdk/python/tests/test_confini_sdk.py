"""I confini dell'SDK: ogni guasto diventa un errore tipizzato, e nessun dato esce.

Revisione di #44, quattro punti:

* il processo: un flusso che non e' UTF-8, un guasto delle pipe, la seconda
  attesa dopo il kill -- per un comando che scrive diventano `ProtocolError`
  con `remote_effect: unknown` e `retry: never`; un processo che non parte
  resta `none`;
* l'adattatore Arrow: una `temp_dir` che non c'e', un produttore che solleva
  un'eccezione sua, una pulizia che fallisce dopo la pubblicazione (ERR-015);
* niente catena e niente riga di comando nei messaggi: percorsi, opzioni e
  testi di pyarrow restano fuori da `str(errore)` e dal traceback;
* `capabilities()` dichiara la superficie `python_sdk`.

Le prove si fanno con un `Popen` finto, che si comporta come quello vero
(`text=True` decodifica e solleva `UnicodeDecodeError`): non serve un binario,
e girano su ogni piattaforma.
"""

from __future__ import annotations

import subprocess
import tempfile
import traceback
import unittest
from pathlib import Path
from unittest import mock

import plenora_io
from plenora_io import (
    CleanupError,
    Client,
    CommandFailed,
    InvalidArgumentError,
    LocalIoError,
    PlenoraError,
    ProtocolError,
)
from plenora_io import _arrow as adattatore
from plenora_io._errors import ErrorEnvelope, failure_from_envelope
from plenora_io._process import Runner

from test_contratto_sdk import capacita_sane, pa, serve_pyarrow

SEGRETO = "SEGRETO-7f3a"


class FlussoFinto:
    """Una pipe finta: rende i byte una volta, o solleva in lettura o chiusura."""

    def __init__(self, dati=b"", errore=None, errore_chiusura=None, registro=None, nome=""):
        self._dati = dati
        self._errore = errore
        self._errore_chiusura = errore_chiusura
        self._registro = registro if registro is not None else []
        self._nome = nome
        self.chiuso = False

    def read(self, quanti=-1):
        if self._errore is not None:
            errore, self._errore = self._errore, None
            raise errore
        dati, self._dati = self._dati, b""
        return dati

    def close(self):
        self._registro.append(f"close {self._nome}")
        self.chiuso = True
        if self._errore_chiusura is not None:
            errore, self._errore_chiusura = self._errore_chiusura, None
            raise errore


#: Le eccezioni che un `Popen` finto solleva dall'attesa, e non dalla lettura.
DALL_ATTESA = (subprocess.TimeoutExpired, KeyboardInterrupt, SystemExit, MemoryError)


class PopenFinto:
    """Un `Popen` con i flussi e i guasti decisi dalla prova.

    `uscite` e' l'elenco di prima: una coppia `(stdout, stderr)` sono i byte
    dei due flussi; un'eccezione prima della coppia e' un guasto. Un timeout,
    un'interruzione o un `MemoryError` li solleva `wait` (in ordine, anche
    dopo `kill`); un `OSError` che viene per primo lo solleva la lettura di
    stdout, uno che segue un guasto dell'attesa lo solleva la `wait` dopo.
    `registro` tiene l'ordine di `wait`, `kill` e `close`.
    """

    def __init__(self, uscite, codice=0, errore_kill=None, errore_chiusura=None,
                 non_muore=False):
        self._uscite = list(uscite)
        self._codice = codice
        self._errore_kill = errore_kill
        self._errore_chiusura = errore_chiusura
        self._non_muore = non_muore
        self.returncode = None
        self.pid = 4242
        self.uccisi = 0
        self.registro = []
        self.stdout = self.stderr = None
        #: I `kill` arrivati da un thread che non e' il principale, o dopo che
        #: il processo era gia' stato raccolto: su POSIX, con il PID riciclato,
        #: colpirebbero un altro processo.
        self.kill_estranei = []
        self._appeso = False
        self._ucciso = False

    def __call__(self, argv, **opzioni):
        self.registro = []
        self._attese = []
        dati = (b"", b"")
        errore_lettura = None
        for uscita in self._uscite:
            if isinstance(uscita, tuple):
                dati = uscita
                break
            if isinstance(uscita, DALL_ATTESA) or self._attese:
                self._attese.append(uscita)
            elif errore_lettura is None:
                errore_lettura = uscita
            else:
                self._attese.append(uscita)
        self.stdout = FlussoFinto(dati[0], errore_lettura, self._errore_chiusura,
                                  self.registro, "stdout")
        self.stderr = FlussoFinto(dati[1], None, None, self.registro, "stderr")
        self.returncode = None
        self._appeso = False
        self._ucciso = False
        return self

    def wait(self, timeout=None):
        """Un timeout scritto nella prova vuol dire «il processo non finisce
        finche' non lo si uccide»: ogni attesa scade (dopo un istante), come
        un processo vero che non esce."""
        import time

        self.registro.append("wait")
        if self._attese and isinstance(self._attese[0], subprocess.TimeoutExpired):
            self._attese.pop(0)
            self._appeso = True
        if self._appeso and not self._ucciso:
            time.sleep(0.005)
            raise subprocess.TimeoutExpired(["x"], timeout)
        if self._attese:
            raise self._attese.pop(0)
        if self._non_muore and self.uccisi:
            time.sleep(0.005)
            raise subprocess.TimeoutExpired(["x"], timeout)
        if self.returncode is None:
            self.returncode = -9 if self._ucciso else self._codice
        return self.returncode

    def poll(self):
        return self.returncode

    def _segnale(self, nome):
        import threading

        self.registro.append(nome)
        if threading.current_thread() is not threading.main_thread() or (
                self.returncode is not None):
            self.kill_estranei.append(nome)
        self.uccisi += 1
        if self._errore_kill is not None:
            raise self._errore_kill
        if not self._non_muore:
            self._ucciso = True

    def kill(self):
        self._segnale("kill")

    def terminate(self):
        self._segnale("terminate")

    def send_signal(self, segnale):
        pass

    # L'interfaccia di `with Popen` e di `communicate`, come la usava l'SDK
    # fino al ventesimo giro: serve a eseguire le stesse prove sul codice di
    # prima, e l'SDK di adesso non la chiama.

    def __enter__(self):
        return self

    def __exit__(self, *eccezione):
        self.registro.append("exit")
        self.stdout.close()
        self.stderr.close()
        self.wait()
        return False

    def communicate(self, timeout=None):
        self.registro.append("communicate")
        if self._attese:
            raise self._attese.pop(0)
        dati = (self.stdout.read(), self.stderr.read())
        if self.returncode is None:
            self.returncode = self._codice
        return dati


def esegui_con(popen, argv):
    with mock.patch.object(subprocess, "Popen", popen):
        return Runner(Path("plenora-io")).run(argv)


SCRIVE = ["write", f"/dati/{SEGRETO}.arrow", f"/uscita/{SEGRETO}.gpkg", "--to", "gpkg",
          "--opt", f"chiave={SEGRETO}"]


def nessun_dato(caso: unittest.TestCase, errore: BaseException) -> None:
    """Ne' il messaggio ne' il traceback portano il segreto, e la catena e' tagliata."""
    caso.assertNotIn(SEGRETO, str(errore))
    testo = "".join(traceback.format_exception(type(errore), errore, errore.__traceback__))
    caso.assertNotIn(SEGRETO, testo)
    caso.assertIsNone(errore.__cause__)


class IlProcesso(unittest.TestCase):
    def assi_ignoti(self, errore: PlenoraError) -> None:
        self.assertEqual(errore.remote_effect, "unknown")
        self.assertEqual(errore.retry, {"kind": "never"})

    def test_un_flusso_che_non_e_utf8_e_un_protocol_error(self) -> None:
        """Prima: `UnicodeDecodeError`, fuori dalla gerarchia e senza assi."""
        with self.assertRaises(ProtocolError) as preso:
            esegui_con(PopenFinto([(b"\xff\xfe\x00 " + SEGRETO.encode(), b"")]), SCRIVE)
        self.assi_ignoti(preso.exception)
        nessun_dato(self, preso.exception)

    def test_un_guasto_delle_pipe_e_un_protocol_error(self) -> None:
        with self.assertRaises(ProtocolError) as preso:
            esegui_con(PopenFinto([BrokenPipeError(SEGRETO), (b"", b"")]), SCRIVE)
        self.assi_ignoti(preso.exception)
        nessun_dato(self, preso.exception)

    def test_la_seconda_attesa_dopo_il_kill_non_sostituisce_l_errore(self) -> None:
        """Il timeout uccide il processo; se la seconda `communicate` fallisce,
        l'errore resta quello del timeout, tipizzato."""
        finto = PopenFinto([
            subprocess.TimeoutExpired([SEGRETO], 1.0),
            OSError(SEGRETO),
        ])
        with mock.patch.object(subprocess, "Popen", finto):
            with self.assertRaises(ProtocolError) as preso:
                Runner(Path("plenora-io"), timeout=0.2).run(SCRIVE)
        self.assi_ignoti(preso.exception)
        nessun_dato(self, preso.exception)

    def test_il_timeout_del_chiamante_non_entra_nel_messaggio(self) -> None:
        """Quindicesimo giro: il messaggio del timeout scriveva per intero il
        decimale scelto dal chiamante («0.12345678901234566s»), un dato in un
        errore fidato; e un valore grande diventava «?s». Ora il messaggio non
        porta il valore, e nessun modello del catalogo prende un decimale."""
        from plenora_io import _errors as errors

        # Valori piccoli: il timeout scade davvero (il `Popen` finto non esce).
        for scelto in (0.12345678901234567, 0.25):
            with self.subTest(timeout=scelto):
                finto = PopenFinto([subprocess.TimeoutExpired(["x"], scelto), (b"", b"")])
                with mock.patch.object(subprocess, "Popen", finto):
                    with self.assertRaises(ProtocolError) as preso:
                        Runner(Path("plenora-io"), timeout=scelto).run(SCRIVE)
                testo = str(preso.exception)
                self.assertTrue(errors.costruito_dall_sdk(preso.exception))
                self.assertNotIn(repr(scelto), testo)
                self.assertNotIn(str(scelto).split(".")[0] + "s", testo)
                self.assertNotIn("?s", testo)
                self.assertIn("tempo massimo", testo)
        for chiave, tipi in errors.PARAMETRI.items():
            for nome, tipo in tipi.items():
                with self.subTest(chiave=chiave, parametro=nome):
                    self.assertNotEqual(type(tipo).__name__, "_Decimale")

    def test_un_processo_che_non_parte_non_ha_effetto(self) -> None:
        def non_parte(argv, **opzioni):
            raise FileNotFoundError(SEGRETO)

        with self.assertRaises(ProtocolError) as preso:
            esegui_con(non_parte, SCRIVE)
        self.assertEqual(preso.exception.remote_effect, "none")
        nessun_dato(self, preso.exception)

    def test_una_busta_illeggibile_non_porta_la_riga_di_comando(self) -> None:
        with self.assertRaises(ProtocolError) as preso:
            esegui_con(PopenFinto([(b"non e' JSON " + SEGRETO.encode(), b"")]), SCRIVE)
        self.assertIn("`plenora-io write`", str(preso.exception))
        nessun_dato(self, preso.exception)

    def test_command_failed_non_porta_la_riga_di_comando(self) -> None:
        busta = {
            "status": "error",
            "error": {
                "code": "X",
                "category": "io",
                "phase": "write",
                "remote_effect": "none",
                "retry": {"kind": "never"},
                "message": "m",
            },
        }
        errore = failure_from_envelope(busta, 4, SCRIVE)
        self.assertIsInstance(errore, CommandFailed)
        self.assertIn("`plenora-io write`", str(errore))
        self.assertNotIn(SEGRETO, str(errore))
        # Solo il sottocomando, da un elenco chiuso: la riga intera porta dati
        # di chi chiama, che li ha gia'.
        self.assertEqual(errore.command, "write")
        self.assertFalse(hasattr(errore, "argv"))


@serve_pyarrow
class LAdattatore(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp())

    def test_una_temp_dir_che_non_c_e_e_un_errore_tipizzato(self) -> None:
        """Prima: `FileNotFoundError` con il percorso di chi chiama."""
        inesistente = self.tmp / SEGRETO / "nonc'e'"
        with self.assertRaises(LocalIoError) as preso:
            with adattatore.cartella_temporanea(inesistente):
                pass
        errore = preso.exception
        self.assertEqual((errore.category, errore.phase), ("io", "prepare"))
        self.assertEqual(errore.remote_effect, "none")
        nessun_dato(self, errore)

    def test_un_produttore_che_solleva_e_un_errore_tipizzato(self) -> None:
        """Prima: l'eccezione del produttore attraversava il confine com'era."""

        class Produttore:
            def __arrow_c_stream__(self, requested_schema=None):
                raise RuntimeError(SEGRETO)

        with self.assertRaises(InvalidArgumentError) as preso:
            adattatore.scrivi_ipc(Produttore(), self.tmp)
        nessun_dato(self, preso.exception)

    def test_un_produttore_che_si_rompe_a_meta_e_un_errore_tipizzato(self) -> None:
        schema = pa.schema([("x", pa.int64())])

        def batch():
            yield pa.record_batch([pa.array([1])], schema=schema)
            raise ValueError(SEGRETO)

        lettore = pa.RecordBatchReader.from_batches(schema, batch())
        with self.assertRaises(InvalidArgumentError) as preso:
            adattatore.scrivi_ipc(lettore, self.tmp)
        nessun_dato(self, preso.exception)

    def test_una_pulizia_fallita_dopo_la_pubblicazione_e_err_015(self) -> None:
        def non_cancella(percorso, *argomenti, **opzioni):
            raise PermissionError(SEGRETO)

        with mock.patch.object(adattatore.shutil, "rmtree", non_cancella):
            with self.assertRaises(CleanupError) as preso:
                with adattatore.cartella_temporanea(self.tmp, effetto="committed"):
                    pass
        errore = preso.exception
        self.assertEqual(errore.phase, "cleanup")
        self.assertEqual(errore.remote_effect, "committed")
        self.assertEqual(errore.retry, {"kind": "never"})
        self.assertIn(errore.category, ("io",))
        nessun_dato(self, errore)

    def test_una_pulizia_fallita_durante_un_errore_non_lo_sostituisce(self) -> None:
        def non_cancella(percorso, *argomenti, **opzioni):
            raise PermissionError(SEGRETO)

        with mock.patch.object(adattatore.shutil, "rmtree", non_cancella):
            with self.assertRaises(InvalidArgumentError) as preso:
                with adattatore.cartella_temporanea(self.tmp, effetto="committed"):
                    raise InvalidArgumentError("l'errore del blocco")
        self.assertTrue(preso.exception.cleanup_failed)

    def test_write_traduce_la_pulizia_fallita_dopo_la_pubblicazione(self) -> None:
        """Dal metodo pubblico: `write` riesce, la pulizia no -> ERR-015."""
        cliente = Client.__new__(Client)
        tabella = pa.table({"x": [1, 2, 3]})
        risultato = object()

        def scrittura_riuscita(argv, modello):
            # Il comando riesce: la pulizia che segue e' cio' che si prova.
            return risultato

        def non_cancella(percorso, *argomenti, **opzioni):
            raise PermissionError(SEGRETO)

        with mock.patch.object(cliente, "_chiama", scrittura_riuscita, create=True), \
                mock.patch.object(adattatore.shutil, "rmtree", non_cancella):
            with self.assertRaises(CleanupError) as preso:
                Client.write(cliente, tabella, self.tmp / "o.gpkg", format="gpkg",
                             temp_dir=self.tmp)
        self.assertEqual(preso.exception.remote_effect, "committed")


class LeCapacita(unittest.TestCase):
    def test_capabilities_dichiara_la_superficie_python_sdk(self) -> None:
        """PYTHON-SDK-1.0 §7, CAP-003, CAP-006: un MUST, non una deroga."""

        class RunnerFinto:
            def run(self, argv):
                assert argv == ["capabilities"]
                return capacita_sane()

        cliente = Client.__new__(Client)
        cliente._runner = RunnerFinto()
        capacita = cliente.capabilities()
        interfacce = {voce.kind: voce for voce in capacita.interfaces}
        self.assertIn("python_sdk", interfacce)
        self.assertEqual(interfacce["python_sdk"].contract, "plenora-python-sdk-v1")
        self.assertEqual(interfacce["python_sdk"].artifact, plenora_io.DISTRIBUTION)
        for operazione in capacita.operations:
            with self.subTest(operazione=operazione.id):
                self.assertIn("python_sdk", operazione.surfaces)

    def test_il_documento_del_binario_non_si_tocca(self) -> None:
        documento = capacita_sane()
        prima = repr(documento)
        plenora_io._client.con_superficie_python(documento)
        self.assertEqual(repr(documento), prima)


if __name__ == "__main__":
    unittest.main()
