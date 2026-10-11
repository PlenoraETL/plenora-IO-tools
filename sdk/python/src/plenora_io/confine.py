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

# I metodi speciali, e i valori dentro i modelli

La rete copre **ogni** callable che una classe esportata definisce, metodi
speciali compresi (`__init__`, `__repr__`, `__eq__`, `__hash__`, `__str__`,
`__bool__`, `__enter__`/`__exit__`...), con un elenco chiuso di eccezioni,
`NON_AVVOLTI`, ciascuna col suo perche'. Le classi d'eccezione comprese.

Poi, perche' quei metodi non eseguano mai codice esterno: un modello -- ogni
dataclass esportata -- normalizza i propri campi nel costruttore. Un valore
diventa il tipo base esatto (`str`, `int`, `float`, `bool`, `None`, `list`,
`tuple`, `dict` di chiavi `str`, `timedelta`), un modello resta se e' di una
classe dell'SDK, e ogni altro oggetto e' `InvalidArgumentError`. Cosi'
`repr`, `==` e `hash` di un modello non chiamano mai un `__repr__`, un `__eq__`
o un `__hash__` del chiamante. L'unica eccezione e' `TableRead.table`, che
porta una `pyarrow.Table`: codice di una dipendenza, non del chiamante.

Un modello e' **immutabile** con un errore dell'SDK (`__setattr__` e
`__delattr__` sollevano `InvalidArgumentError`, non `FrozenInstanceError`), e
il suo `hash` si calcola dalla forma canonica dei campi: prima era il `hash`
generato dalla dataclass, che su un campo `dict` sollevava `TypeError`.

# Che cosa resta fuori

`BaseException` che non sono `Exception` -- `KeyboardInterrupt`,
`SystemExit` -- passano: non sono guasti dell'SDK ma richieste del processo, e
trasformarle in un errore le annullerebbe. Nessun metodo pubblico rende un
generatore e nessuno si aggiunge a un'istanza dopo la costruzione: un
generatore uscirebbe dalla rete prima di eseguire il proprio corpo. Una prova
(`test_confini_esterni.LaRete`) verifica entrambe le cose.
"""

from __future__ import annotations

import contextvars
import functools
from typing import Any, Callable

import dataclasses
import sys
from datetime import timedelta

from .errors import InvalidArgumentError, PlenoraError, UnexpectedError

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


#: I callable che la rete non avvolge, ciascuno col suo perche'. Un elenco
#: **chiuso**: la prova per introspezione pretende che ogni altro callable
#: definito da una classe esportata sia avvolto, e che questi siano esattamente
#: i soli esclusi.
NON_AVVOLTI: dict[str, str] = {
    "__new__": "costruisce l'oggetto prima di `__init__`; i modelli non lo ridefiniscono",
    "__init_subclass__": "gira alla definizione di una sottoclasse, non su un'istanza",
    "__class_getitem__": "e' la sintassi dei generici, sulla classe",
    "__subclasshook__": "e' consultato da `isinstance`, che la rete non deve cambiare",
    "__getattribute__": "avvolgerlo farebbe passare dalla rete ogni accesso, anche i suoi",
    "__getattr__": "`hasattr` e la ricerca degli attributi vogliono `AttributeError`",
    "__reduce__": "pickle e copy: lavorano sui campi gia' normalizzati",
    "__reduce_ex__": "pickle e copy: lavorano sui campi gia' normalizzati",
    "__getstate__": "pickle e copy: lavorano sui campi gia' normalizzati",
    "__setstate__": "pickle e copy: lavorano sui campi gia' normalizzati",
    "__copy__": "copy: lavora sui campi gia' normalizzati",
    "__deepcopy__": "copy: lavora sui campi gia' normalizzati",
}


def _privato(nome: str) -> bool:
    """Un nome privato (`_x`), non un metodo speciale (`__x__`)."""
    return nome.startswith("_") and not (nome.startswith("__") and nome.endswith("__"))


def _e_un_modello(tipo: type) -> bool:
    """Una dataclass definita dall'SDK, e non una sottoclasse del chiamante."""
    return dataclasses.is_dataclass(tipo) and tipo.__module__.startswith("plenora_io")


def _tabella_pyarrow(valore: Any) -> bool:
    pa = sys.modules.get("pyarrow")
    return pa is not None and type(valore) is getattr(pa, "Table", None)


def _esatto(valore: Any, dove: str) -> Any:
    """Il valore nel tipo base esatto, senza chiamare codice del chiamante."""
    tipo = type(valore)
    if valore is None or tipo in (bool, str, int, float):
        return valore
    if tipo in (list, tuple) or issubclass(tipo, (list, tuple)):
        base = list if issubclass(tipo, list) else tuple
        elementi = [_esatto(v, dove) for v in base.__iter__(valore)]
        return elementi if base is list else tuple(elementi)
    if tipo is dict or issubclass(tipo, dict):
        copia = {}
        for chiave, interno in dict.items(valore):
            if not issubclass(type(chiave), str):
                raise InvalidArgumentError(f"`{dove}` ha una chiave che non e' una stringa.")
            copia[str.join("", [chiave])] = _esatto(interno, dove)
        return copia
    if issubclass(tipo, str):
        return str.join("", [valore])
    if issubclass(tipo, int):
        return int.__pos__(valore)
    if issubclass(tipo, float):
        return float.__pos__(valore)
    if issubclass(tipo, timedelta):
        from .argomenti import durata  # noqa: PLC0415 - argomenti importa i modelli

        return durata(valore, dove)
    if _e_un_modello(tipo) or _tabella_pyarrow(valore):
        return valore
    raise InvalidArgumentError(
        f"`{dove}` contiene un oggetto di un tipo che un risultato dell'SDK non porta."
    )


def _normalizza(istanza: Any) -> None:
    """Ogni campo dell'istanza nel tipo base esatto (vedi `_esatto`)."""
    for campo in dataclasses.fields(istanza):
        valori = object.__getattribute__(istanza, "__dict__")
        if campo.name not in valori:
            continue
        dove = f"{type(istanza).__name__}.{campo.name}"
        try:
            esatto = _esatto(valori[campo.name], dove)
        except RecursionError:
            raise InvalidArgumentError(f"`{dove}` e' annidato troppo a fondo.") from None
        object.__setattr__(istanza, campo.name, esatto)


def _forma(valore: Any) -> Any:
    """Una forma canonica e hashable di un valore normalizzato."""
    if dataclasses.is_dataclass(valore) and not isinstance(valore, type):
        valori = object.__getattribute__(valore, "__dict__")
        return (
            type(valore).__qualname__,
            tuple((c.name, _forma(valori.get(c.name))) for c in dataclasses.fields(valore)),
        )
    if type(valore) in (list, tuple):
        return tuple(_forma(v) for v in valore)
    if type(valore) is dict:
        return tuple(sorted((k, _forma(v)) for k, v in valore.items()))
    if _tabella_pyarrow(valore):
        return ("pyarrow.Table", id(valore))
    return valore


def _hash_del_modello(self: Any) -> int:
    return hash(_forma(self))


def _immutabile(self: Any, *argomenti: Any) -> None:
    raise InvalidArgumentError(
        f"`{type(self).__name__}` e' immutabile: un risultato dell'SDK non si modifica."
    )


def _con_normalizzazione(init: Callable[..., Any]) -> Callable[..., Any]:
    @functools.wraps(init)
    def costruttore(self: Any, *argomenti: Any, **opzioni: Any) -> None:
        init(self, *argomenti, **opzioni)
        _normalizza(self)

    return costruttore


def _prepara_modello(classe: type) -> None:
    """Normalizzazione nel costruttore, `hash` canonico, immutabilita'."""
    if not _e_un_modello(classe) or getattr(classe, "__plenora_modello__", False):
        return
    init = vars(classe).get("__init__")
    if init is not None and not getattr(init, SEGNO, False):
        setattr(classe, "__init__", _con_normalizzazione(init))
    parametri = getattr(classe, "__dataclass_params__", None)
    if parametri is not None and parametri.frozen:
        if parametri.eq:
            setattr(classe, "__hash__", _hash_del_modello)
        setattr(classe, "__setattr__", _immutabile)
        setattr(classe, "__delattr__", _immutabile)
    setattr(classe, "__plenora_modello__", True)


def confina_classe(classe: type) -> type:
    """Avvolge ogni callable che la classe definisce, salvo `NON_AVVOLTI`.

    Metodi, metodi di classe, statici, proprieta' (getter, setter, deleter) e
    metodi speciali. Solo cio' che la classe definisce nel proprio `__dict__`:
    i metodi ereditati da un'altra classe esportata sono avvolti li', e quelli
    di `object` o di `Exception` non sono codice dell'SDK.
    """
    _prepara_modello(classe)
    for nome, valore in list(vars(classe).items()):
        if _privato(nome) or nome in NON_AVVOLTI:
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
    """Avvolge ogni classe, eccezioni comprese, e ogni funzione esportata da `nomi`."""
    for nome in nomi:
        valore = spazio[nome]
        if isinstance(valore, type):
            for classe in valore.__mro__:
                if classe.__module__.startswith("plenora_io"):
                    confina_classe(classe)
        elif callable(valore):
            spazio[nome] = confinato(valore, nome)
