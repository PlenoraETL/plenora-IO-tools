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

from .contenitori import FrozenDict, FrozenList
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

#: Le sole classi dell'SDK che definiscono un nome di `NON_AVVOLTI`, e quale. Un
#: elenco chiuso: la prova per introspezione fallisce se una classe ne definisce
#: un altro, cosi' un'esclusione non si aggiunge in silenzio.
NON_AVVOLTI_DEFINITI: frozenset[tuple[str, str]] = frozenset(
    {
        ("FrozenDict", "__reduce__"),
        ("FrozenList", "__reduce__"),
        ("FrozenDict", "__new__"),
        ("FrozenList", "__new__"),
    }
)


def _privato(nome: str) -> bool:
    """Un nome privato (`_x`), non un metodo speciale (`__x__`)."""
    return nome.startswith("_") and not (nome.startswith("__") and nome.endswith("__"))


#: I modelli dell'SDK, per **nome**: un elenco chiuso ed esplicito. Prima un
#: modello si riconosceva dal prefisso del modulo (`plenora_io...`), e passava
#: anche una dataclass interna come `process.Completed`, o una del chiamante in
#: un modulo `plenora_io_plugin`. Ora vale l'identita' della classe, fissata da
#: `registra_modelli` all'import, e una prova confronta l'elenco con le
#: dataclass esportate.
NOMI_DEI_MODELLI: tuple[str, ...] = (
    "Capabilities", "CapabilityInterface", "CapabilityOperation", "Catalog",
    "ConvertResult", "ConvertedLayer", "CrsResolution", "Delivered", "Driver",
    "ErrorEnvelope", "Fidelity", "FidelityReason", "Field", "FormatDescriptor",
    "Geometry", "Inspect", "Layer", "LayerSummary", "Layers", "Limits", "LossCount",
    "LossExample", "LossReport", "Manifest", "Omissions", "OperationContent",
    "OperationControls", "TableRead", "Validation", "Version", "WriteInput",
    "WriteResult",
)

#: Le classi di `NOMI_DEI_MODELLI`, riempito da `registra_modelli`. Un elenco e
#: non un insieme, e lo si scorre con `is`: l'appartenenza a un `set` o a una
#: `tuple` passa per `__eq__` e `__hash__`, che la metaclasse di una classe del
#: chiamante puo' ridefinire.
MODELLI: list[type] = []


def e_un_modello(tipo: type) -> bool:
    """`tipo` e' una delle classi del modello, per identita'."""
    return any(tipo is classe for classe in MODELLI)

#: L'unico campo che porta un oggetto di una dipendenza: la tabella di
#: `TableRead`. Ogni altro campo che ne contenesse una e' un errore.
CAMPI_CON_TABELLA: frozenset[tuple[str, str]] = frozenset({("TableRead", "table")})


def registra_modelli(spazio: dict[str, Any]) -> None:
    """Le classi dei modelli, prese per nome dallo spazio di `__init__`."""
    for nome in NOMI_DEI_MODELLI:
        classe = spazio[nome]
        if not dataclasses.is_dataclass(classe):
            raise TypeError(f"{nome} non e' una dataclass")
        MODELLI.append(classe)


def _tabella_pyarrow(valore: Any) -> bool:
    pa = sys.modules.get("pyarrow")
    return pa is not None and type(valore) is getattr(pa, "Table", None)


def _chiave(valore: Any, dove: str) -> str:
    if not issubclass(type(valore), str):
        raise InvalidArgumentError(f"`{dove}` ha una chiave che non e' una stringa.")
    return str.join("", [valore])


def _esatto(valore: Any, dove: str, tabella_ammessa: bool = False) -> Any:
    """Il valore nel tipo base esatto e **immutabile**, senza chiamare codice
    del chiamante: `FrozenList` al posto di `list` e `tuple`, `FrozenDict` al
    posto di `dict` (vedi `contenitori`)."""
    tipo = type(valore)
    if valore is None or tipo is bool or tipo is str or tipo is int or tipo is float:
        return valore
    if tipo is FrozenList:
        # Si ricostruisce: il costruttore rinormalizza gli elementi, e un
        # modello annidato alterato fuori contratto non passa com'e'.
        return FrozenList(valore)
    if tipo is FrozenDict:
        return FrozenDict(valore)
    if issubclass(tipo, (list, tuple)):
        base = list if issubclass(tipo, list) else tuple
        return FrozenList([_esatto(v, dove) for v in base.__iter__(valore)])
    if issubclass(tipo, dict):
        copia: dict[str, Any] = {}
        for chiave, interno in dict.items(valore):
            esatta = _chiave(chiave, dove)
            if esatta in copia:
                # Due chiavi distinte -- per esempio due sottoclassi di `str`
                # con lo stesso contenuto -- che diventano la stessa: una
                # avrebbe coperto l'altra in silenzio.
                raise InvalidArgumentError(
                    f"`{dove}` ha due chiavi che normalizzate coincidono."
                )
            copia[esatta] = _esatto(interno, dove)
        return FrozenDict(copia)
    if issubclass(tipo, str):
        return str.join("", [valore])
    if issubclass(tipo, int):
        return int.__pos__(valore)
    if issubclass(tipo, float):
        return float.__pos__(valore)
    if issubclass(tipo, timedelta):
        from .argomenti import durata  # noqa: PLC0415 - argomenti importa i modelli

        return durata(valore, dove)
    if e_un_modello(tipo):
        # Un modello dell'SDK, per identita' della classe: si rinormalizza
        # comunque, perche' un modello alterato fuori contratto non porti
        # dentro un altro il suo stato.
        _normalizza(valore)
        return valore
    if tabella_ammessa and _tabella_pyarrow(valore):
        return valore
    raise InvalidArgumentError(
        f"`{dove}` contiene un oggetto di un tipo che un risultato dell'SDK non porta."
    )


def _normalizza(istanza: Any) -> None:
    """Ogni campo dell'istanza nel tipo base esatto e immutabile (vedi `_esatto`)."""
    nome_classe = type(istanza).__name__
    for campo in dataclasses.fields(istanza):
        valori = object.__getattribute__(istanza, "__dict__")
        if campo.name not in valori:
            continue
        dove = f"{nome_classe}.{campo.name}"
        try:
            esatto = _esatto(
                valori[campo.name], dove, (nome_classe, campo.name) in CAMPI_CON_TABELLA
            )
        except RecursionError:
            raise InvalidArgumentError(f"`{dove}` e' annidato troppo a fondo.") from None
        object.__setattr__(istanza, campo.name, esatto)


def _forma(valore: Any) -> Any:
    """Una forma canonica e hashable di un valore normalizzato.

    La tabella di `TableRead` vale una costante: due `TableRead` uguali
    (stessa busta, tabelle uguali per pyarrow) hanno cosi' lo stesso `hash`, e
    l'uguaglianza resta quella dei campi.
    """
    if e_un_modello(type(valore)):
        valori = object.__getattribute__(valore, "__dict__")
        return (
            type(valore).__qualname__,
            tuple((c.name, _forma(valori.get(c.name))) for c in dataclasses.fields(valore)),
        )
    if type(valore) is FrozenList:
        return tuple(_forma(v) for v in valore)
    if type(valore) is FrozenDict:
        return tuple(sorted((k, _forma(v)) for k, v in valore.items()))
    if _tabella_pyarrow(valore):
        return ("pyarrow.Table",)
    return valore


def _hash_del_modello(self: Any) -> int:
    return hash(_forma(self))


def _uguale(self: Any, altro: Any) -> Any:
    """`==` fra modelli: la stessa classe, per identita', e gli stessi campi.

    Il `__eq__` generato dalla dataclass leggeva `altro.__class__`, che un
    oggetto del chiamante puo' ridefinire: qui si guarda `type(altro)`. I campi
    sono normalizzati, quindi il confronto non chiama codice del chiamante.
    """
    if type(altro) is not type(self):
        return NotImplemented
    mie = object.__getattribute__(self, "__dict__")
    sue = object.__getattribute__(altro, "__dict__")
    return all(
        mie.get(c.name) == sue.get(c.name) for c in dataclasses.fields(self)
    )


def _immutabile(self: Any, *argomenti: Any) -> None:
    raise InvalidArgumentError(
        f"`{type(self).__name__}` e' immutabile: un risultato dell'SDK non si modifica."
    )


def _con_normalizzazione(init: Callable[..., Any]) -> Callable[..., Any]:
    @functools.wraps(init)
    def costruttore(self: Any, *argomenti: Any, **opzioni: Any) -> None:
        # Una seconda chiamata a `__init__` su un modello gia' costruito lo
        # reinizializzerebbe: e' una modifica, e si rifiuta.
        if object.__getattribute__(self, "__dict__"):
            raise InvalidArgumentError(
                f"`{type(self).__name__}` e' gia' costruito: un risultato dell'SDK non si "
                "reinizializza."
            )
        init(self, *argomenti, **opzioni)
        _normalizza(self)

    return costruttore


def _prepara_modello(classe: type) -> None:
    """Normalizzazione nel costruttore, `==` e `hash` canonici, immutabilita'."""
    if not e_un_modello(classe) or vars(classe).get("__plenora_modello__", False):
        return
    init = vars(classe).get("__init__")
    if init is not None and not getattr(init, SEGNO, False):
        setattr(classe, "__init__", _con_normalizzazione(init))
    setattr(classe, "__eq__", _uguale)
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
