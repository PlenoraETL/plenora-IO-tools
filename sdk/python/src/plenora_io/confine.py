"""La rete di sicurezza: nessuna eccezione esterna attraversa un metodo pubblico.

# Perche' una rete, oltre ai punti tradotti

PYTHON-SDK-1.0 §6 vuole che ogni eccezione pubblica sia un `PlenoraError` con i
cinque assi. I punti in cui un'eccezione esterna puo' nascere sono tradotti
dove nascono -- e li' l'effetto e' preciso -- ma un elenco di punti si chiude
solo se qualcuno li ha visti tutti, e tre giri di revisione ne hanno trovati di
nuovi a ogni giro. La rete chiude la **classe**: ogni metodo pubblico di ogni
tipo esportato, e ogni funzione esportata, passa da `confinato`, e
un'eccezione che non e' un `PlenoraError` diventa `UnexpectedError`. Lo
diventa anche un `PlenoraError` che non e' nato nell'SDK (`_nato_nell_sdk`):
le classi sono pubbliche, e codice del chiamante che l'SDK esegue puo'
sollevarne una con il proprio testo.

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
import os
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


#: La cartella del pacchetto: un errore sollevato da un file qui dentro e'
#: dell'SDK.
_CARTELLA = os.path.normcase(os.path.dirname(os.path.abspath(__file__)))


def _nato_nell_sdk(errore: BaseException) -> bool:
    """`errore` e' un errore dell'SDK **sollevato dall'SDK**.

    Non basta che sia un `PlenoraError`: le classi sono pubbliche, e il codice
    del chiamante che l'SDK esegue -- un `__index__`, un `__fspath__`, un
    `__arrow_c_stream__` -- puo' sollevare `InvalidArgumentError("dato")`, che
    prima attraversava la rete con il suo testo. Contano due fatti, nessuno dei
    due dichiarato dall'errore stesso: la classe e' **una** di quelle di
    `errors`, per identita' (non una sottoclasse del chiamante), e il punto
    piu' interno del traceback -- dove l'istruzione `raise` e' stata eseguita
    -- sta in un file di questo pacchetto. Un `raise` senza argomenti che
    rilancia non sposta quel punto: un errore esterno resta esterno.
    """
    if not _classe_dell_sdk(type(errore)):
        return False
    passo = errore.__traceback__
    if passo is None:
        return False
    while passo.tb_next is not None:
        passo = passo.tb_next
    file = passo.tb_frame.f_code.co_filename
    return os.path.normcase(os.path.dirname(os.path.abspath(file))) == _CARTELLA


def _classe_dell_sdk(tipo: type) -> bool:
    """`tipo` e' una classe d'errore definita in `errors`, per identita'."""
    from . import errors  # noqa: PLC0415 - errors e' gia' importato qui sopra

    return any(
        tipo is classe
        for classe in vars(errors).values()
        if type(classe) is type and issubclass(classe, PlenoraError)
    )


def confinato(funzione: Callable[..., Any], nome: str | None = None) -> Callable[..., Any]:
    """`funzione`, con ogni eccezione che non e' un errore dell'SDK nato
    nell'SDK (`_nato_nell_sdk`) tradotta in `UnexpectedError`."""
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
        except Exception as errore:  # noqa: BLE001 - e' la rete, apposta
            if _nato_nell_sdk(errore):
                raise
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
#:
#: `__new__` e gli agganci di `pickle` e `copy` (`__reduce__`, `__getstate__`...)
#: non ci sono piu' (settimo giro di revisione): quando una classe dell'SDK li
#: definisce sono codice dell'SDK come gli altri, e fuori dalla rete un
#: argomento sbagliato -- `FrozenDict(**{chiave: 1})` -- usciva come `TypeError`
#: di binding, con la chiave nel messaggio. Quelli ereditati da `object` non
#: sono definiti dalle classi, e la rete non li tocca comunque.
NON_AVVOLTI: dict[str, str] = {
    "__init_subclass__": "gira alla definizione di una sottoclasse, non su un'istanza",
    "__class_getitem__": "e' la sintassi dei generici, sulla classe",
    "__subclasshook__": "e' consultato da `isinstance`, che la rete non deve cambiare",
    "__getattribute__": "avvolgerlo farebbe passare dalla rete ogni accesso, anche i suoi",
    "__getattr__": "`hasattr` e la ricerca degli attributi vogliono `AttributeError`",
}

#: Le sole classi dell'SDK che definiscono un nome di `NON_AVVOLTI`, e quale. Un
#: elenco chiuso, oggi vuoto: la prova per introspezione fallisce se una classe
#: ne definisce uno, cosi' un'esclusione non si aggiunge in silenzio.
NON_AVVOLTI_DEFINITI: frozenset[tuple[str, str]] = frozenset()


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
    tipo = type(valore)
    if (
        valore is None or tipo is bool or tipo is str or tipo is int or tipo is float
        or tipo is timedelta
    ):
        return valore
    # Solo cio' che `_esatto` produce. Prima ogni altro valore tornava com'era,
    # e un `dict` rimesso in un campo da un `__post_init__` richiamato a mano
    # arrivava a `hash` come `TypeError`: ora un valore che un modello non
    # porta e' un errore dell'SDK, e non la forma di qualcos'altro.
    raise InvalidArgumentError(
        "il modello contiene un valore che un risultato dell'SDK non porta: "
        "e' stato alterato fuori contratto."
    )


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
        gettone = _in_costruzione.set(self)
        try:
            init(self, *argomenti, **opzioni)
        finally:
            _in_costruzione.reset(gettone)
        _normalizza(self)

    return costruttore


#: Il modello il cui costruttore e' in corso, nel contesto corrente: il solo su
#: cui `__post_init__` puo' girare.
_in_costruzione: contextvars.ContextVar[Any] = contextvars.ContextVar(
    "plenora_io_in_costruzione", default=None
)


def _solo_nel_costruttore(post_init: Callable[..., Any]) -> Callable[..., Any]:
    """`__post_init__`, eseguibile solo dal costruttore della stessa istanza.

    `__post_init__` e' un metodo pubblico per nome, e la dataclass lo chiama
    dal costruttore; chiamarlo a mano dopo rieseguiva la normalizzazione su un
    modello gia' costruito. Su `ErrorEnvelope` rimetteva `retry` e
    `row_diagnostics` come `dict` ordinari, e una seconda chiamata dopo
    `retry["kind"] = "safe"` riscriveva la decisione privata: un
    `CommandFailed` gia' costruito diventava ritentabile. Una chiamata fuori
    dal costruttore si rifiuta, prima di toccare qualunque campo, per ogni
    modello che lo definisce.
    """

    @functools.wraps(post_init)
    def guardia(self: Any, *argomenti: Any, **opzioni: Any) -> None:
        if _in_costruzione.get() is not self:
            raise InvalidArgumentError(
                f"`{type(self).__name__}` e' gia' costruito: `__post_init__` gira solo "
                "nel costruttore, e un risultato dell'SDK non si rinormalizza."
            )
        post_init(self, *argomenti, **opzioni)

    return guardia


def _prepara_modello(classe: type) -> None:
    """Normalizzazione nel costruttore, `==` e `hash` canonici, immutabilita',
    `__post_init__` solo dal costruttore."""
    if not e_un_modello(classe) or vars(classe).get("__plenora_modello__", False):
        return
    init = vars(classe).get("__init__")
    if init is not None and not getattr(init, SEGNO, False):
        setattr(classe, "__init__", _con_normalizzazione(init))
    post_init = vars(classe).get("__post_init__")
    if post_init is not None:
        setattr(classe, "__post_init__", _solo_nel_costruttore(post_init))
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
