"""Gli argomenti dei metodi pubblici, normalizzati all'ingresso.

# La regola

Ogni argomento si converte **subito**, al confine, nel suo tipo base esatto --
`str`, `int`, `bool`, `float`, `dict` di `str`, `Limits` e `timedelta` nuovi --
e da li' in avanti l'SDK lavora solo sulle copie. Il codice di chi chiama
(`__fspath__`, `__str__`, `__format__`, `__contains__`, `items()`, una
sottoclasse di `Limits` o di `timedelta`) gira al piu' una volta, qui, dentro la
traduzione: qualunque cosa sollevi diventa `InvalidArgumentError` con effetto
`none`, senza messaggio del chiamante e senza catena (`from None`).

Prima ogni conversione stava dove serviva -- un `os.fspath` qui, una f-string
li' -- e ognuna era un punto in cui un'eccezione esterna attraversava il
confine, o in cui una sottoclasse eseguiva codice dopo il controllo.

# Come si ottiene il tipo esatto senza chiamare la sottoclasse

`str(x)` su una sottoclasse di `str` chiama il suo `__str__`; `int(x)` il suo
`__int__`. Qui si usano i metodi **non legati** del tipo base, che non passano
dalle ridefinizioni: `str.join("", [x])` (costruisce una `str` nuova dai
caratteri), `int.__pos__(x)`, `float.__pos__(x)`, e i descrittori di
`timedelta`. Il tipo si guarda con `type(x)` e non con `isinstance`, che
consulta `__class__` e si lascia ingannare.

# Tipi esatti, non coercizioni; grandezze prima delle conversioni

Un `layer=True` diventava `"True"`, e un `durable="no"` era vero: qui un intero
e' un `int` che non e' un `bool`, un booleano e' un `bool`. Un intero si
confronta con `2**64` **prima** di diventare testo decimale: `str(10**50000)`
solleva `ValueError` (il limite di cifre di Python), e `math.isfinite(10**1000)`
`OverflowError`. Nessun valore che la CLI accetta supera `u64`.

# La chiave di un'opzione non contiene `=`

La CLI divide `chiave=valore` al **primo** `=`: una chiave `a=b` con valore `c`
arriverebbe come chiave `a` e valore `b=c`. Si rifiuta.
"""

from __future__ import annotations

import math
from collections.abc import Mapping
from dataclasses import fields
from datetime import timedelta
from typing import Any

from .errors import InvalidArgumentError
from .limits import Limits

#: Il massimo, escluso, di un intero che arriva alla CLI: i suoi numeri sono
#: `u64`.
OLTRE_U64 = 1 << 64


def _rifiuta(nome: str, che_cosa: str) -> InvalidArgumentError:
    return InvalidArgumentError(f"`{nome}` {che_cosa}.")


def _str_esatta(valore: Any, nome: str) -> str:
    """Una `str` nuova con gli stessi caratteri, senza chiamare la sottoclasse."""
    if not issubclass(type(valore), str):
        raise _rifiuta(nome, "non e' una stringa")
    try:
        testo = str.join("", [valore])
    except Exception:  # noqa: BLE001 - il confine traduce tutto, apposta
        raise _rifiuta(nome, "non si lascia leggere come stringa") from None
    if "\x00" in testo:
        # Un processo riceve gli argomenti come stringhe C: un NUL le tronca, e
        # `Popen` lo rifiuta con `ValueError`.
        raise _rifiuta(nome, "contiene un carattere NUL, che un argomento di processo non puo' portare")
    try:
        testo.encode("utf-8")
    except UnicodeEncodeError:
        # Un surrogato isolato (per esempio da `os.fsdecode` su byte non
        # UTF-8) non si passa a un processo ne' si scrive in un percorso.
        raise _rifiuta(nome, "contiene un carattere che non si codifica in UTF-8") from None
    return testo


def _int_esatto(valore: Any, nome: str) -> int:
    tipo = type(valore)
    if tipo is bool or not issubclass(tipo, int):
        raise _rifiuta(nome, "non e' un intero")
    try:
        numero = int.__pos__(valore)
    except Exception:  # noqa: BLE001
        raise _rifiuta(nome, "non si lascia leggere come intero") from None
    if not -OLTRE_U64 < numero < OLTRE_U64:
        raise _rifiuta(nome, "e' fuori dall'intervallo che la CLI rappresenta (64 bit)")
    return numero


def percorso(valore: Any, nome: str) -> str:
    """Un percorso come `str` esatta: `str` o `os.PathLike` che renda una `str`."""
    if issubclass(type(valore), str):
        return _str_esatta(valore, nome)
    metodo = getattr(type(valore), "__fspath__", None)
    if metodo is None:
        raise _rifiuta(nome, "non e' un percorso: serve una `str` o un `os.PathLike`")
    try:
        reso = metodo(valore)
    except Exception:  # noqa: BLE001 - `__fspath__` e' codice di chi chiama
        raise _rifiuta(nome, "non si lascia leggere come percorso") from None
    if not issubclass(type(reso), str):
        # `bytes`: la riga di comando e' testo, e una decodifica scelta qui
        # sarebbe un'interpretazione che nessuno ha chiesto.
        raise _rifiuta(nome, "non e' un percorso di testo: serve una `str` o un `os.PathLike[str]`")
    return _str_esatta(reso, nome)


def testo(valore: Any, nome: str) -> str:
    """Una `str` esatta, senza NUL."""
    return _str_esatta(valore, nome)


def intero(valore: Any, nome: str) -> str:
    """Un `int` che non e' un `bool`, entro 64 bit, come testo decimale."""
    return int.__repr__(_int_esatto(valore, nome))


def booleano(valore: Any, nome: str) -> bool:
    """Un `bool`, e nient'altro: `"no"` non e' falso, `1` non e' vero."""
    if type(valore) is not bool:
        raise _rifiuta(nome, "non e' un booleano")
    return valore


def opzioni(valore: Any, nome: str) -> dict[str, str]:
    """Un dizionario nuovo di `str` esatte, letto una volta sola, qui."""
    if valore is None:
        return {}
    if not issubclass(type(valore), Mapping):
        raise _rifiuta(nome, "non e' un dizionario di stringhe")
    try:
        coppie = list(valore.items())
    except Exception:  # noqa: BLE001 - `items()` di una Mapping altrui
        raise _rifiuta(nome, "non si lascia leggere come dizionario") from None
    copia: dict[str, str] = {}
    for coppia in coppie:
        if type(coppia) is not tuple or len(coppia) != 2:
            raise _rifiuta(nome, "non rende coppie chiave-valore")
        chiave = _str_esatta(coppia[0], nome)
        interno = _str_esatta(coppia[1], nome)
        if "=" in chiave:
            raise _rifiuta(
                nome,
                "ha una chiave che contiene `=`: la CLI divide al primo `=`, e "
                "la coppia arriverebbe con un'altra chiave",
            )
        if chiave in copia:
            raise _rifiuta(nome, "ripete una chiave")
        copia[chiave] = interno
    return copia


def righe_di_opzioni(bandiera: str, mappa: dict[str, str]) -> list[str]:
    """`bandiera chiave=valore`, una per coppia, da un dizionario gia' normalizzato."""
    righe: list[str] = []
    for chiave, interno in mappa.items():
        righe += [bandiera, f"{chiave}={interno}"]
    return righe


def durata(valore: Any, nome: str) -> timedelta:
    """Una `timedelta` nuova, letta con i descrittori del tipo base."""
    if not issubclass(type(valore), timedelta):
        raise _rifiuta(nome, "non e' un `timedelta`")
    try:
        giorni = timedelta.days.__get__(valore)
        secondi = timedelta.seconds.__get__(valore)
        micro = timedelta.microseconds.__get__(valore)
        return timedelta(days=giorni, seconds=secondi, microseconds=micro)
    except Exception:  # noqa: BLE001
        raise _rifiuta(nome, "non si lascia leggere come durata") from None


def limiti(valore: Any, nome: str = "limits") -> Limits | None:
    """Un `Limits` nuovo, esatto, con ogni campo normalizzato; `None` resta `None`."""
    if valore is None:
        return None
    if not issubclass(type(valore), Limits):
        raise _rifiuta(nome, "non e' un `Limits`")
    campi: dict[str, Any] = {}
    for campo in fields(Limits):
        try:
            interno = object.__getattribute__(valore, campo.name)
        except Exception:  # noqa: BLE001 - una sottoclasse puo' ridefinirlo
            raise _rifiuta(f"{nome}.{campo.name}", "non si lascia leggere") from None
        if interno is None:
            campi[campo.name] = None
        elif campo.name == Limits.DURATA:
            campi[campo.name] = durata(interno, f"{nome}.{campo.name}")
        else:
            campi[campo.name] = _int_esatto(interno, f"{nome}.{campo.name}")
    return Limits(**campi)


def argv_dei_limiti(valore: Limits | None) -> list[str]:
    """Gli argomenti di un `Limits` gia' normalizzato."""
    return [] if valore is None else valore.to_argv()


def timeout(valore: Any) -> float | None:
    """Il timeout del client: `None`, o secondi positivi e finiti."""
    if valore is None:
        return None
    tipo = type(valore)
    if tipo is bool:
        raise _rifiuta("timeout", "non e' un numero di secondi")
    if issubclass(tipo, int):
        # La grandezza prima di qualunque conversione: `float(10**1000)` e
        # `math.isfinite(10**1000)` sollevano `OverflowError`.
        secondi: float = float(_int_esatto(valore, "timeout"))
    elif issubclass(tipo, float):
        try:
            secondi = float.__pos__(valore)
        except Exception:  # noqa: BLE001
            raise _rifiuta("timeout", "non si lascia leggere come numero") from None
    else:
        raise _rifiuta("timeout", "non e' un numero di secondi")
    if not math.isfinite(secondi) or secondi <= 0:
        raise _rifiuta("timeout", "non e' un numero di secondi positivo e finito")
    return secondi
