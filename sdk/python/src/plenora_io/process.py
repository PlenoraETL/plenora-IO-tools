"""L'esecuzione del binario, e la lettura dei due flussi.

# Perche' un modulo suo

Ogni comando fa la stessa cosa: costruisce una riga di argomenti, esegue,
decodifica una busta. La prima stesura teneva quel codice dentro il client,
accanto ai due metodi che lo usavano; con cinque comandi sarebbe stato copiato o
generalizzato in fretta, e il posto dove generalizzarlo e' questo.

# I due flussi hanno ruoli diversi, e il protocollo lo dice

`docs/PRODUCT.md` e' esplicito: in caso di **errore** `stderr` contiene sempre e
soltanto la busta JSON; con il protocollo predefinito e **successo**, `stderr`
resta vuoto. La busta di successo va su `stdout`.

La prima stesura cercava un JSON prima su stdout e poi su stderr, prendendo il
primo che si decodificasse. Funzionava, e diceva una cosa piu' debole del vero:
che la busta sta su uno dei due. Con quella regola una busta d'errore scritta
per sbaglio su stdout sarebbe stata letta come un successo, e nessuno se ne
sarebbe accorto -- perche' `status` lo si guarda **dopo** aver scelto il flusso.

Qui il flusso si sceglie dal codice d'uscita, che e' cio' che il protocollo
lega all'esito, e il contenuto dell'altro flusso e' verificato invece che
ignorato.

# Perche' `stderr` non vuoto con successo **e'** un errore

Il protocollo v2 dice che con successo `stderr` resta vuoto, e questo esecutore
parla v2 e nient'altro. Qualunque cosa vi compaia e' percio' una violazione del
contratto, e va detta.

La prima stesura la tollerava, per il protocollo legacy: quello scrive su
`stderr` un avviso quando lo si sceglie. Era una tolleranza **implicita** --
l'SDK non espone quel flag, e nessuno l'aveva chiesta -- e il suo costo e' che
rendeva invisibile la sola forma in cui il v2 puo' sporcare quel flusso: un
avviso non previsto, una traccia di debug, la riga di una libreria che scrive
dove non deve. Un consumatore che compone la CLI in una pipeline se ne
accorgerebbe da un log corrotto, non da qui.

Se un giorno il v1 servira', avra' un percorso suo, dichiarato: un protocollo
diverso si sceglie, non si indovina dal fatto che qualcosa e' comparso su
`stderr`.

«Vuoto» vuol dire **zero byte**, non «niente di significativo»: anche uno spazio
e' contenuto. Un confine che ammettesse gli spazi lascerebbe passare una
scrittura accidentale vuota, che e' la forma in cui una violazione arriva senza
che nessuno l'abbia voluta.

# Il primo SIGINT si inoltra, e la cancellazione la fa il prodotto

Un Ctrl-C durante una conversione lunga deve fermarla **con grazia**: il
prodotto arma al primo segnale un token che la pipeline osserva ai propri punti
di verifica, e da li' torna un errore `CANCELLED` con la destinazione ripulita.
Al secondo segnale esce con 130.

Senza inoltro, il Ctrl-C arriverebbe soltanto al processo Python: la CLI
resterebbe viva a scrivere, o morirebbe di colpo lasciando uno staging sul
disco. L'esecutore installa percio' un gestore **per la durata della singola
esecuzione**, e lo rimette com'era subito dopo: una libreria che si
approprriasse del gestore del processo cambierebbe il comportamento di codice
che non l'ha chiamata.

Il gestore si puo' installare solo dal thread principale -- e' Python a
imporlo -- e fuori di li' l'esecuzione procede senza inoltro. Non e' un
fallimento chiuso e non deve esserlo: chiamare `convert()` da un thread e' un
uso legittimo, e rifiutarlo romperebbe programmi che funzionano per una
comodita' che non riguarda la correttezza di cio' che esce.
`Runner.sigint_forwarding_available` lo dice, per chi ha bisogno di saperlo
invece di scoprirlo.
"""

from __future__ import annotations

import contextlib
import json
import signal
import subprocess
import sys
import threading
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from .errors import ProtocolError, failure_from_envelope



@contextlib.contextmanager
def _inoltra_sigint(processo: "subprocess.Popen[str]"):
    """Inoltra SIGINT al figlio, per la durata di questo blocco.

    Il gestore precedente viene ripristinato all'uscita, sempre: una libreria
    non e' padrona del gestore dei segnali del processo che la ospita.

    Su Windows `SIGINT` a un processo figlio non si manda: `os.kill` con quel
    segnale uccide il processo invece di consegnarglielo, e
    `CTRL_C_EVENT` va all'intero gruppo -- compreso chi ci ospita. Li' l'inoltro
    non si arma, e la cancellazione con grazia non e' disponibile.
    """
    if not sigint_inoltrabile():
        yield
        return

    def inoltra(numero, frame):  # noqa: ARG001 - la firma la impone `signal`
        # Non si solleva `KeyboardInterrupt` qui: il figlio deve ricevere il
        # segnale e chiudere da se', e un'eccezione sollevata adesso
        # abbandonerebbe la `communicate` lasciando il processo vivo.
        with contextlib.suppress(ProcessLookupError, OSError):
            processo.send_signal(signal.SIGINT)

    precedente = signal.signal(signal.SIGINT, inoltra)
    try:
        yield
    finally:
        signal.signal(signal.SIGINT, precedente)


def sigint_inoltrabile() -> bool:
    """L'inoltro del segnale e' possibile in questo contesto.

    Due condizioni, ed e' Python a imporle entrambe: il gestore si installa solo
    dal thread principale, e su Windows non c'e' modo di consegnare un `SIGINT`
    a un figlio senza colpire chi lo ospita.
    """
    return (
        sys.platform != "win32"
        and threading.current_thread() is threading.main_thread()
    )


@dataclass(frozen=True)
class Completed:
    """Che cosa e' uscito da un'esecuzione, prima di darle un significato."""

    argv: list[str]
    exit_code: int
    stdout: str
    stderr: str

    def stream(self, name: str) -> str:
        return self.stdout if name == "stdout" else self.stderr


class Runner:
    """Esegue il binario e restituisce la busta, o solleva.

    Non sa niente dei comandi: prende argomenti e rende un documento. I metodi
    che sanno quale busta aspettarsi stanno nel client, ed e' li' che il tipo
    si costruisce.
    """

    def __init__(self, binary: Path, *, timeout: float | None = None) -> None:
        self._binary = binary
        self._timeout = timeout

    @property
    def binary(self) -> Path:
        return self._binary

    def run(self, argv: list[str]) -> dict[str, Any]:
        """La busta di successo, o `CommandFailed` con quella d'errore."""
        completed = self._execute(argv)
        if completed.exit_code == 0:
            return self._success(completed)
        raise failure_from_envelope(
            self._failure(completed), completed.exit_code, argv
        )

    # --- l'esecuzione -----------------------------------------------------

    @property
    def sigint_forwarding_available(self) -> bool:
        """Un Ctrl-C durante un comando arriva al prodotto.

        Falso da un thread che non sia il principale e su Windows, e in
        entrambi i casi il comando funziona lo stesso: cambia solo che non lo si
        puo' fermare con grazia.
        """
        return sigint_inoltrabile()

    def _execute(self, argv: list[str]) -> Completed:
        # `Popen` e non `run`: senza il pid non c'e' niente a cui inoltrare il
        # segnale, e l'inoltro e' il modo in cui un Ctrl-C diventa una
        # cancellazione cooperativa invece di un file mezzo scritto.
        try:
            processo = subprocess.Popen(  # noqa: S603 - argv e' costruito qui
                [str(self._binary), *argv],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                encoding="utf-8",
            )
        except OSError as errore:
            raise ProtocolError(
                f"`{self._binary}` non si e' potuto eseguire: {errore}"
            ) from errore

        with processo:
            try:
                with _inoltra_sigint(processo):
                    stdout, stderr = processo.communicate(timeout=self._timeout)
            except subprocess.TimeoutExpired as errore:
                # Il processo va chiuso prima di sollevare: lasciarlo vivo
                # significherebbe restituire il controllo a chi chiama con un
                # binario che continua a scrivere sulla destinazione.
                processo.kill()
                processo.communicate()
                raise ProtocolError(
                    f"`plenora-io {' '.join(argv)}` non ha risposto entro "
                    f"{self._timeout}s ed e' stato terminato. Il timeout lo "
                    "sceglie chi chiama, e questo errore non dice che il "
                    "comando sia fallito: dice che non si sa, e che una "
                    "destinazione parziale puo' essere rimasta."
                ) from errore

        return Completed(
            argv=list(argv),
            exit_code=processo.returncode,
            stdout=stdout,
            stderr=stderr,
        )

    # --- i due flussi, ciascuno al proprio posto ---------------------------

    def _success(self, completed: Completed) -> dict[str, Any]:
        # `!= ""`, non `.strip()`: anche uno spazio e' contenuto. Uno `strip()`
        # lascerebbe passare una scrittura accidentale vuota o di soli spazi --
        # un `eprintln!("")` di troppo, un a capo rimasto -- che e' proprio la
        # forma in cui una violazione arriva senza che nessuno l'abbia voluta.
        # Il confine dev'essere quello che si puo' verificare guardando i byte.
        if completed.stderr != "":
            # Il v2 tace su stderr quando riesce. Non e' pignoleria: e' la sola
            # affermazione che rende quel flusso utilizzabile da chi compone la
            # CLI in una pipeline, e tollerarne la violazione la toglierebbe.
            raise ProtocolError(
                f"`plenora-io {' '.join(completed.argv)}` e' riuscito e ha "
                "scritto su stderr, dove il protocollo v2 non mette niente in "
                "caso di successo. L'SDK parla v2: un altro protocollo si "
                f"sceglie, non si deduce.\nstderr: {completed.stderr[:200]!r}"
            )
        documento = self._decode(completed, "stdout")
        stato = documento.get("status")
        if stato != "ok":
            # Un'uscita a zero con una busta che non si dichiara riuscita: il
            # protocollo non lo prevede, e leggerla come successo darebbe per
            # buono un documento che il prodotto non ha dichiarato tale.
            raise ProtocolError(
                f"`plenora-io {' '.join(completed.argv)}` e' uscito con zero e "
                f"ha scritto su stdout una busta di stato «{stato}»: il "
                "protocollo non prevede questa combinazione."
            )
        return documento

    def _failure(self, completed: Completed) -> dict[str, Any]:
        documento = self._decode(completed, "stderr")
        if documento.get("status") != "error":
            raise ProtocolError(
                f"`plenora-io {' '.join(completed.argv)}` e' uscito con "
                f"{completed.exit_code} e ha scritto su stderr una busta di "
                f"stato «{documento.get('status')}»: un'uscita diversa da zero "
                "porta una busta d'errore."
            )
        if completed.stdout.strip():
            # Il protocollo scrive la busta d'errore su stderr **e nient'altro
            # su stdout**: un output parziale consegnato prima di un errore
            # terminale e' cio' che i target di fuzzing cercano, e un SDK che
            # lo ignorasse lo lascerebbe consumare.
            raise ProtocolError(
                f"`plenora-io {' '.join(completed.argv)}` e' fallito e ha "
                "comunque scritto su stdout: un output parziale prima di un "
                f"errore terminale non e' consumabile.\n"
                f"stdout: {completed.stdout[:200]!r}"
            )
        return documento

    @staticmethod
    def _decode(completed: Completed, stream: str) -> dict[str, Any]:
        testo = completed.stream(stream)
        if not testo.strip():
            altro = "stderr" if stream == "stdout" else "stdout"
            raise ProtocolError(
                f"`plenora-io {' '.join(completed.argv)}` e' uscito con "
                f"{completed.exit_code} e non ha scritto niente su {stream}, "
                f"dove il protocollo mette la busta.\n"
                f"{altro}: {completed.stream(altro)[:200]!r}"
            )
        try:
            documento = json.loads(testo)
        except json.JSONDecodeError as errore:
            raise ProtocolError(
                f"cio' che `plenora-io {' '.join(completed.argv)}` ha scritto "
                f"su {stream} non e' JSON: {errore}\n{testo[:200]!r}"
            ) from errore
        if not isinstance(documento, dict):
            raise ProtocolError(
                f"la busta su {stream} e' {type(documento).__name__} e non un "
                "oggetto JSON."
            )
        return documento
