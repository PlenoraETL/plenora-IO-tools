"""Gli argomenti dei metodi pubblici, verificati prima di diventare una riga.

# Perche' un modulo suo

Un metodo del client trasforma i propri argomenti in testo -- `os.fspath` per
un percorso, `str` per un intero, una f-string per un'opzione -- e ognuna di
quelle conversioni chiama codice di chi chiama: `__fspath__`, `__str__`,
`__format__`, `items()`. Prima l'eccezione di quel codice attraversava il
confine com'era; un NUL arrivava a `Popen`, che solleva `ValueError`; un tipo
sbagliato diventava un `TypeError` o un `AttributeError` a meta' della riga.

Qui ogni genere di argomento ha una funzione, e ognuna rende testo o solleva
`InvalidArgumentError`: prima di eseguire, con l'effetto `none`, senza il
valore nel messaggio e senza catena (`from None`). Il messaggio nomina
l'argomento, non il suo contenuto.

# Tipi esatti, non coercizioni

Un `layer=True` diventava `"True"`, e un `durable="no"` era vero. Sono
coercizioni che nessuno ha chiesto: qui un intero e' un `int` che non e' un
`bool`, un booleano e' un `bool`, un testo e' una `str`. Il valore lo giudica
il prodotto; il tipo, che e' cio' che l'SDK promette nella firma, lo giudica
l'SDK.

# La chiave di un'opzione non contiene `=`

La CLI divide `chiave=valore` al **primo** `=`. Una chiave `a=b` con valore
`c` diventerebbe `a=b=c`, che la CLI legge come chiave `a` e valore `b=c`: un
significato diverso da quello passato, senza errore. Si rifiuta.
"""

from __future__ import annotations

import math
import os
from collections.abc import Mapping
from dataclasses import fields
from datetime import timedelta
from typing import Any

from .errors import InvalidArgumentError
from .limits import Limits


def _rifiuta(nome: str, che_cosa: str) -> InvalidArgumentError:
    return InvalidArgumentError(f"`{nome}` {che_cosa}.")


def _senza_nul(testo: str, nome: str) -> str:
    # Un processo riceve gli argomenti come stringhe C: un NUL le tronca, e
    # `Popen` lo rifiuta con `ValueError`. Si rifiuta qui, con un nome.
    if "\x00" in testo:
        raise _rifiuta(nome, "contiene un carattere NUL, che un argomento di processo non puo' portare")
    return testo


def percorso(valore: Any, nome: str) -> str:
    """Un percorso come testo: `str` o `os.PathLike` che renda una `str`."""
    try:
        testo = os.fspath(valore)
    except Exception:  # noqa: BLE001 - il confine traduce tutto, apposta
        # `TypeError` per cio' che non e' un percorso, e qualunque cosa per un
        # `__fspath__` che solleva: e' codice di chi chiama.
        raise _rifiuta(nome, "non e' un percorso: serve una `str` o un `os.PathLike`") from None
    if not isinstance(testo, str):
        # `bytes`: la riga di comando e' testo, e una decodifica scelta qui
        # sarebbe un'interpretazione che nessuno ha chiesto.
        raise _rifiuta(nome, "e' un percorso in byte: serve una `str` o un `os.PathLike[str]`")
    return _senza_nul(testo, nome)


def testo(valore: Any, nome: str) -> str:
    """Una `str`, senza NUL."""
    if not isinstance(valore, str):
        raise _rifiuta(nome, "non e' una stringa")
    return _senza_nul(valore, nome)


def intero(valore: Any, nome: str) -> str:
    """Un `int` che non e' un `bool`, come testo decimale."""
    if not isinstance(valore, int) or isinstance(valore, bool):
        raise _rifiuta(nome, "non e' un intero")
    return int.__repr__(valore)


def booleano(valore: Any, nome: str) -> bool:
    """Un `bool`, e nient'altro: `"no"` non e' falso."""
    if not isinstance(valore, bool):
        raise _rifiuta(nome, "non e' un booleano")
    return valore


def opzioni(valore: Any, nome: str) -> list[str]:
    """Le coppie `chiave=valore` di un dizionario di stringhe, in ordine."""
    if valore is None:
        return []
    if not isinstance(valore, Mapping):
        raise _rifiuta(nome, "non e' un dizionario di stringhe")
    try:
        coppie = list(valore.items())
    except Exception:  # noqa: BLE001 - `items()` di una Mapping altrui
        raise _rifiuta(nome, "non si lascia leggere come dizionario") from None
    righe = []
    for chiave, interno in coppie:
        if not isinstance(chiave, str) or not isinstance(interno, str):
            raise _rifiuta(nome, "ha una chiave o un valore che non e' una stringa")
        if "=" in chiave:
            raise _rifiuta(
                nome,
                "ha una chiave che contiene `=`: la CLI divide al primo `=`, e "
                "la coppia arriverebbe con un'altra chiave",
            )
        righe.append(_senza_nul(f"{chiave}={interno}", nome))
    return righe


def limiti(valore: Any, nome: str = "limits") -> list[str]:
    """Gli argomenti di un `Limits`, con i tipi dei campi verificati."""
    if valore is None:
        return []
    if not isinstance(valore, Limits):
        raise _rifiuta(nome, "non e' un `Limits`")
    for campo in fields(valore):
        interno = getattr(valore, campo.name)
        if interno is None:
            continue
        if campo.name == Limits.DURATA:
            if not isinstance(interno, timedelta):
                raise _rifiuta(f"{nome}.{campo.name}", "non e' un `timedelta`")
        elif not isinstance(interno, int) or isinstance(interno, bool):
            raise _rifiuta(f"{nome}.{campo.name}", "non e' un intero")
    return valore.to_argv()


def timeout(valore: Any) -> float | None:
    """Il timeout del client: `None`, o secondi positivi e finiti."""
    if valore is None:
        return None
    if isinstance(valore, bool) or not isinstance(valore, (int, float)):
        raise _rifiuta("timeout", "non e' un numero di secondi")
    if not math.isfinite(valore) or valore <= 0:
        raise _rifiuta("timeout", "non e' un numero di secondi positivo e finito")
    return valore
