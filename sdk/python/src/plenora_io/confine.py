"""La rete di sicurezza: nessuna eccezione esterna attraversa un metodo pubblico.

# Perche' una rete, oltre ai punti tradotti

PYTHON-SDK-1.0 §6 vuole che ogni eccezione pubblica sia un `PlenoraError` con i
cinque assi. I punti in cui un'eccezione esterna puo' nascere sono tradotti
dove nascono -- e li' l'effetto e' preciso -- ma un elenco di punti si chiude
solo se qualcuno li ha visti tutti, e tre giri di revisione ne hanno trovati di
nuovi a ogni giro. La rete chiude la **classe**: ogni metodo pubblico di ogni
tipo esportato, e ogni funzione esportata, passa da `confinato`, e
un'eccezione che non e' un `PlenoraError` diventa `UnexpectedError`.

# L'effetto, conservativo

Prima dell'avvio di un processo l'effetto e' `none`: niente e' successo fuori
dal processo Python. Dopo l'avvio di un comando che scrive e' `unknown`: il
comando puo' aver pubblicato, e una risposta che non si e' letta non dice il
contrario. Per saperlo, `Runner` registra qui ogni comando che parte
(`registra_avvio`), nel contesto della chiamata pubblica piu' esterna in corso.

# Che cosa resta fuori

`BaseException` che non sono `Exception` -- `KeyboardInterrupt`,
`SystemExit` -- passano: non sono guasti dell'SDK ma richieste del processo, e
trasformarle in un errore le annullerebbe. Le classi d'eccezione esportate non
sono avvolte: i loro metodi sono le proprieta' degli assi, e un errore dentro
un errore non avrebbe un posto dove andare.
"""

from __future__ import annotations

import contextvars
import functools
from typing import Any, Callable

from .errors import PlenoraError, UnexpectedError

#: Il segno che un callable e' avvolto: la prova per introspezione lo cerca.
SEGNO = "__plenora_confine__"

#: I comandi partiti durante la chiamata pubblica piu' esterna in corso.
_avviati: contextvars.ContextVar[list[list[str]] | None] = contextvars.ContextVar(
    "plenora_io_avviati", default=None
)


def registra_avvio(argv: list[str]) -> None:
    """Chiamata da `Runner` quando un processo e' partito davvero."""
    elenco = _avviati.get()
    if elenco is not None:
        elenco.append(list(argv))


def _effetto_dopo(avviati: list[list[str]]) -> str:
    from .process import scrive  # noqa: PLC0415 - process importa questo modulo

    return "unknown" if any(scrive(argv) for argv in avviati) else "none"


def confinato(funzione: Callable[..., Any], nome: str | None = None) -> Callable[..., Any]:
    """`funzione`, con ogni eccezione non `PlenoraError` tradotta."""
    if getattr(funzione, SEGNO, False):
        return funzione
    etichetta = nome or getattr(funzione, "__qualname__", "?")

    @functools.wraps(funzione)
    def involucro(*argomenti: Any, **opzioni: Any) -> Any:
        esterna = _avviati.get() is None
        gettone = _avviati.set([]) if esterna else None
        elenco = _avviati.get()
        try:
            return funzione(*argomenti, **opzioni)
        except PlenoraError:
            raise
        except Exception:  # noqa: BLE001 - e' la rete, apposta
            effetto = _effetto_dopo(elenco or [])
            raise UnexpectedError(
                f"errore imprevisto dell'SDK in `{etichetta}`: un'eccezione "
                "esterna e' stata intercettata al confine, senza il suo testo.",
                remote_effect=effetto,
            ) from None
        finally:
            if gettone is not None:
                _avviati.reset(gettone)

    setattr(involucro, SEGNO, True)
    return involucro


def confina_classe(classe: type) -> type:
    """Avvolge i metodi, i metodi di classe, quelli statici e le proprieta' pubblici.

    Pubblici vuol dire senza `_` iniziale, piu' `__init__`, che e' il
    costruttore. Solo cio' che la classe definisce nel proprio `__dict__`: i
    metodi ereditati da un'altra classe esportata sono avvolti li'.
    """
    for nome, valore in list(vars(classe).items()):
        if nome.startswith("_") and nome != "__init__":
            continue
        etichetta = f"{classe.__name__}.{nome}"
        if isinstance(valore, classmethod):
            setattr(classe, nome, classmethod(confinato(valore.__func__, etichetta)))
        elif isinstance(valore, staticmethod):
            setattr(classe, nome, staticmethod(confinato(valore.__func__, etichetta)))
        elif isinstance(valore, property):
            setattr(
                classe,
                nome,
                property(
                    confinato(valore.fget, etichetta) if valore.fget else None,
                    confinato(valore.fset, etichetta) if valore.fset else None,
                    confinato(valore.fdel, etichetta) if valore.fdel else None,
                    valore.__doc__,
                ),
            )
        elif callable(valore) and not isinstance(valore, type):
            setattr(classe, nome, confinato(valore, etichetta))
    return classe


def confina_esportati(spazio: dict[str, Any], nomi: list[str]) -> None:
    """Avvolge ogni classe (non d'eccezione) e ogni funzione esportata da `nomi`."""
    for nome in nomi:
        valore = spazio[nome]
        if isinstance(valore, type):
            if issubclass(valore, BaseException):
                continue
            for classe in valore.__mro__:
                if classe.__module__.startswith("plenora_io"):
                    confina_classe(classe)
        elif callable(valore):
            spazio[nome] = confinato(valore, nome)
