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
"""

from __future__ import annotations

import json
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from .errors import ProtocolError, failure_from_envelope


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

    def _execute(self, argv: list[str]) -> Completed:
        try:
            esito = subprocess.run(
                [str(self._binary), *argv],
                capture_output=True,
                text=True,
                encoding="utf-8",
                timeout=self._timeout,
                check=False,
            )
        except subprocess.TimeoutExpired as errore:
            raise ProtocolError(
                f"`plenora-io {' '.join(argv)}` non ha risposto entro "
                f"{self._timeout}s. Il timeout lo sceglie chi chiama, e questo "
                "errore non dice che il comando sia fallito: dice che non si sa."
            ) from errore
        except OSError as errore:
            raise ProtocolError(
                f"`{self._binary}` non si e' potuto eseguire: {errore}"
            ) from errore
        return Completed(
            argv=list(argv),
            exit_code=esito.returncode,
            stdout=esito.stdout,
            stderr=esito.stderr,
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
