"""La rete di sicurezza: nessuna eccezione esterna attraversa un metodo pubblico.

# Perche' una rete, oltre ai punti tradotti

PYTHON-SDK-1.0 §6 vuole che ogni eccezione pubblica sia un `PlenoraError` con i
cinque assi. I punti in cui un'eccezione esterna puo' nascere sono tradotti
dove nascono -- e li' l'effetto e' preciso -- ma un elenco di punti si chiude
solo se qualcuno li ha visti tutti, e tre giri di revisione ne hanno trovati di
nuovi a ogni giro. La rete chiude la **classe**: ogni metodo pubblico di ogni
tipo esportato, e ogni funzione esportata, passa da `_confinato`, e
un'eccezione che non e' un errore dell'SDK diventa `UnexpectedError`. Un
errore dell'SDK esce ricostruito dal suo stato fissato, non com'e' arrivato
(`_al_confine`); uno che il chiamante ha costruito, anche di una classe
dell'SDK, e' un'eccezione esterna come le altre.

La rete non e' una barriera contro codice ostile nello stesso interprete:
chi scrive con `object.__setattr__`, nel `__dict__` o negli attributi privati,
chiama la fabbrica privata `errors._dallo_sdk` o modifica i moduli del
pacchetto puo' aggirarla. Sono fuori contratto, e il README li elenca fra i
limiti dichiarati.

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

from ._contenitori import FrozenDict, FrozenList
from ._errors import (
    PROVENIENZA,
    SIGILLO,
    InvalidArgumentError,
    PlenoraError,
    UnexpectedError,
    classe_dell_sdk,
    costruito_dall_sdk,
    ricostruito,
)
from ._errors import _dallo_sdk

#: Il segno che un callable e' avvolto: la prova per introspezione lo cerca.
SEGNO = "__plenora_confine__"

#: Le etichette che la rete ha dato ai callable delle classi e alle funzioni
#: esportate, all'import: il vocabolario del parametro `etichetta` di
#: `rete.imprevisto`. Lo riempiono soltanto `_confina_classe` e
#: `_confina_esportati`.
ETICHETTE: set[str] = set()

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
    from ._process import scrive  # noqa: PLC0415 - process importa questo modulo

    return "unknown" if any(scrive(argv) for argv in avviati) else "none"


def _al_confine(errore: Exception, avviati: list[list[str]], etichetta: str) -> PlenoraError:
    """Che cosa esce dalla rete al posto di `errore`.

    Un errore dell'SDK -- una classe di `errors` per identita', sigillato e
    costruito da codice dell'SDK (`errors.costruito_dall_sdk`) -- esce
    **ricostruito** dal suo stato fissato (`errors._ricostruito`): un'istanza
    nuova della stessa classe, che non legge mai `args`, `__notes__` o `str()`
    dell'oggetto ricevuto. Una callback che prende un errore dell'SDK, ne
    riscrive `args` e lo rilancia non fa uscire il proprio testo, e
    `ResultLookupError` resta un `KeyError` per `Mapping.get`.

    Ogni altra eccezione -- di Python, di una dipendenza, del chiamante, anche
    un `PlenoraError` che il chiamante ha costruito o derivato -- diventa
    `UnexpectedError`, senza testo e con l'effetto conservativo.

    Fino all'ottavo giro la rete guardava invece il file in cui era stato
    eseguito il `raise`: certificava dove l'errore era nato e non che cosa
    portasse, e il confronto dei percorsi sbagliava con PyInstaller, con un
    nome di file relativo dopo un `chdir` o con il bytecode altrove.
    """
    if costruito_dall_sdk(errore):
        return ricostruito(errore)
    return _dallo_sdk(
        UnexpectedError, "rete.imprevisto",
        etichetta=etichetta,
        remote_effect=_effetto_dopo(avviati),
    )


def _confinato(funzione: Callable[..., Any], nome: str | None = None) -> Callable[..., Any]:
    """`funzione`, con ogni eccezione passata da `_al_confine`."""
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
            sostituto = _al_confine(errore, elenco or [], etichetta)
        finally:
            if gettone is not None:
                _avviati.reset(gettone)
        # Fuori dall'`except` della rete, e poi senza contesto: se il
        # chiamante usa l'SDK dentro un proprio `except`, Python mette la sua
        # eccezione come `__context__` di cio' che si solleva, e `from None`
        # la nasconde soltanto. La si toglie e si rilancia con un `raise`
        # nudo, che non concatena di nuovo.
        try:
            raise sostituto from None
        except PlenoraError:
            sostituto.__context__ = None
            raise

    setattr(involucro, SEGNO, True)
    return involucro


#: L'errore dell'SDK il cui costruttore piu' esterno e' in corso.
_errore_in_costruzione: contextvars.ContextVar[Any] = contextvars.ContextVar(
    "plenora_io_errore_in_costruzione", default=None
)


def nome_sdk(tipo: Any) -> str:
    """Il nome della prima classe dell'SDK nell'MRO di `tipo`, o `?`.

    Per i messaggi: `type(self).__name__` di un'istanza puo' essere il nome di
    una sottoclasse che il chiamante ha chiamato come vuole --
    `type("SEGRETO", (Version,), {})` -- e il nome entrava nel testo. Qui il
    nome e' sempre quello di una classe dell'SDK, riconosciuta per identita'.
    """
    try:
        ordine = type.__dict__["__mro__"].__get__(tipo, type)
    except TypeError:
        return "?"
    note = [*MODELLI, FrozenList, FrozenDict]
    for classe in ordine:
        if classe_dell_sdk(classe) or any(classe is nota for nota in note):
            return type.__dict__["__name__"].__get__(classe, type)
    return "?"


def _init_sigillato(init: Callable[..., Any]) -> Callable[..., Any]:
    """`__init__` di un errore dell'SDK: una volta sola, poi sigillato.

    Il costruttore piu' esterno -- quello che la classe concreta chiama, non i
    `super().__init__` che ne seguono -- finito, mette il sigillo: da li'
    `PlenoraError.__setattr__` rifiuta ogni attributo che non sia
    dell'interprete, e un secondo `__init__` e' `InvalidArgumentError`.

    La provenienza e' sempre **esterna**: il costruttore pubblico, chiamato in
    qualunque modo -- da una callback, da `map` o `functools.partial` in C, da
    `eval` -- non sa chi lo chiama, e non prova a indovinarlo dai frame (fino
    al decimo giro lo faceva, e un callable in C non aggiunge un frame: si
    vedeva quello dell'SDK che eseguiva la callback). Un errore e' dell'SDK
    solo se nasce da `errors._dallo_sdk`, la fabbrica privata che lo marca
    dopo averlo costruito.
    """

    @functools.wraps(init)
    def costruttore(self: Any, *argomenti: Any, **opzioni: Any) -> None:
        stato = object.__getattribute__(self, "__dict__")
        if dict.get(stato, SIGILLO) is True:
            raise _dallo_sdk(
                InvalidArgumentError, "errore.gia_costruito",
                nome=nome_sdk(type(self)),
            )
        if _errore_in_costruzione.get() is self:
            init(self, *argomenti, **opzioni)
            return
        gettone = _errore_in_costruzione.set(self)
        try:
            init(self, *argomenti, **opzioni)
        finally:
            _errore_in_costruzione.reset(gettone)
        stato[PROVENIENZA] = False
        stato[SIGILLO] = True

    return costruttore


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
#: `_registra_modelli` all'import, e una prova confronta l'elenco con le
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

#: Le classi di `NOMI_DEI_MODELLI`, riempito da `_registra_modelli`. Un elenco e
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


def _registra_modelli(spazio: dict[str, Any]) -> None:
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
        raise _dallo_sdk(InvalidArgumentError, "modello.chiave_non_stringa", dove=dove)
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
                raise _dallo_sdk(InvalidArgumentError, "modello.chiavi_coincidenti", dove=dove)
            copia[esatta] = _esatto(interno, dove)
        return FrozenDict(copia)
    if issubclass(tipo, str):
        return str.join("", [valore])
    if issubclass(tipo, int):
        return int.__pos__(valore)
    if issubclass(tipo, float):
        return float.__pos__(valore)
    if issubclass(tipo, timedelta):
        from ._argomenti import durata  # noqa: PLC0415 - _argomenti importa i modelli

        return durata(valore, dove)
    if e_un_modello(tipo):
        # Un modello dell'SDK, per identita' della classe: si rinormalizza
        # comunque, perche' un modello alterato fuori contratto non porti
        # dentro un altro il suo stato.
        _normalizza(valore)
        return valore
    if tabella_ammessa and _tabella_pyarrow(valore):
        return valore
    raise _dallo_sdk(InvalidArgumentError, "modello.oggetto_non_ammesso", dove=dove)


def _classe_del_modello(tipo: Any) -> Any:
    """La prima classe di `MODELLI` nell'MRO di `tipo`, per identita', o `None`."""
    try:
        ordine = type.__dict__["__mro__"].__get__(tipo, type)
    except TypeError:
        return None
    for base in ordine:
        for modello in MODELLI:
            if base is modello:
                return modello
    return None


def _campi(tipo: Any) -> tuple[Any, ...]:
    """I campi della classe del modello dell'SDK, mai quelli di `tipo`.

    Una sottoclasse puo' ridefinire `__dataclass_fields__`, o un campo come
    proprieta': i nomi e l'elenco dei campi vengono dalla classe dell'SDK
    riconosciuta per identita' (tredicesimo giro di revisione).
    """
    modello = _classe_del_modello(tipo)
    if modello is None:
        raise _dallo_sdk(InvalidArgumentError, "rete.self_estraneo")
    return dataclasses.fields(modello)


def _normalizza(istanza: Any) -> None:
    """Ogni campo dell'istanza nel tipo base esatto e immutabile (vedi `_esatto`)."""
    nome_classe = nome_sdk(type(istanza))
    for campo in _campi(type(istanza)):
        valori = object.__getattribute__(istanza, "__dict__")
        if campo.name not in valori:
            continue
        dove = f"{nome_classe}.{campo.name}"
        try:
            esatto = _esatto(
                valori[campo.name], dove, (nome_classe, campo.name) in CAMPI_CON_TABELLA
            )
        except RecursionError:
            raise _dallo_sdk(InvalidArgumentError, "modello.annidato", dove=dove) from None
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
            tuple((c.name, _forma(valori.get(c.name))) for c in _campi(type(valore))),
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
    raise _dallo_sdk(InvalidArgumentError, "modello.alterato")


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
        mie.get(c.name) == sue.get(c.name) for c in _campi(type(self))
    )


def _immutabile(self: Any, *argomenti: Any) -> None:
    raise _dallo_sdk(InvalidArgumentError, "modello.immutabile", nome=nome_sdk(type(self)))


def _con_normalizzazione(init: Callable[..., Any]) -> Callable[..., Any]:
    @functools.wraps(init)
    def costruttore(self: Any, *argomenti: Any, **opzioni: Any) -> None:
        # Una seconda chiamata a `__init__` su un modello gia' costruito lo
        # reinizializzerebbe: e' una modifica, e si rifiuta.
        if object.__getattribute__(self, "__dict__"):
            raise _dallo_sdk(
                InvalidArgumentError, "modello.gia_costruito",
                nome=nome_sdk(type(self)),
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
            raise _dallo_sdk(InvalidArgumentError, "modello.post_init", nome=nome_sdk(type(self)))
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


def _con_self(funzione: Callable[..., Any], classe: type) -> Callable[..., Any]:
    """`funzione` di `classe`, che rifiuta un primo argomento di un'altra classe.

    Un metodo dell'SDK si puo' chiamare non legato -- `Catalog.__init__(x, ...)`
    -- su un oggetto qualunque, e prima girava: `_normalizza` leggeva i campi
    di `x` e ne metteva i nomi nei messaggi. Ora un `self` che non ha `classe`
    nel proprio MRO, per identita', e' `InvalidArgumentError` con un messaggio
    fisso, prima di eseguire qualunque cosa.
    """

    @functools.wraps(funzione)
    def controllato(*argomenti: Any, **opzioni: Any) -> Any:
        if not argomenti:
            raise _dallo_sdk(InvalidArgumentError, "rete.self_estraneo")
        try:
            ordine = type.__dict__["__mro__"].__get__(type(argomenti[0]), type)
        except TypeError:
            ordine = ()
        if not any(base is classe for base in ordine):
            raise _dallo_sdk(InvalidArgumentError, "rete.self_estraneo")
        return funzione(*argomenti, **opzioni)

    return controllato


def _confina_classe(classe: type) -> type:
    """Avvolge ogni callable che la classe definisce, salvo `NON_AVVOLTI`.

    Metodi, metodi di classe, statici, proprieta' (getter, setter, deleter) e
    metodi speciali. Solo cio' che la classe definisce nel proprio `__dict__`:
    i metodi ereditati da un'altra classe esportata sono avvolti li', e quelli
    di `object` o di `Exception` non sono codice dell'SDK.
    """
    _prepara_modello(classe)
    if classe_dell_sdk(classe):
        init = vars(classe).get("__init__")
        if init is not None and not getattr(init, SEGNO, False):
            setattr(classe, "__init__", _init_sigillato(init))
    for nome, valore in list(vars(classe).items()):
        if _privato(nome) or nome in NON_AVVOLTI:
            continue
        etichetta = f"{classe.__name__}.{nome}"
        ETICHETTE.add(etichetta)
        # Una classe si incontra piu' volte (una per ogni nome esportato che
        # la ha nell'MRO). Cio' che e' gia' avvolto non si riavvolge: prima
        # `_con_self` avvolgeva la rete una seconda volta, ne copiava il segno
        # con `functools.wraps`, e `_confinato` lo lasciava fuori dalla rete
        # -- il controllo del `self` sollevava senza traduzione ne' catena
        # azzerata (sedicesimo giro).
        interne = (
            [valore.__func__] if isinstance(valore, (classmethod, staticmethod))
            else [valore.fget, valore.fset, valore.fdel] if isinstance(valore, property)
            else [valore]
        )
        if any(getattr(interna, SEGNO, False) for interna in interne if interna is not None):
            continue
        if isinstance(valore, classmethod):
            setattr(classe, nome, classmethod(_confinato(valore.__func__, etichetta)))
        elif isinstance(valore, staticmethod):
            setattr(classe, nome, staticmethod(_confinato(valore.__func__, etichetta)))
        elif isinstance(valore, property):
            setattr(
                classe,
                nome,
                property(
                    _confinato(_con_self(valore.fget, classe), etichetta) if valore.fget else None,
                    _confinato(_con_self(valore.fset, classe), etichetta) if valore.fset else None,
                    _confinato(_con_self(valore.fdel, classe), etichetta) if valore.fdel else None,
                    valore.__doc__,
                ),
            )
        elif callable(valore) and not isinstance(valore, type):
            setattr(classe, nome, _confinato(_con_self(valore, classe), etichetta))
    return classe


def _confina_esportati(spazio: dict[str, Any], nomi: list[str]) -> None:
    """Avvolge ogni classe, eccezioni comprese, e ogni funzione esportata da `nomi`."""
    for nome in nomi:
        valore = spazio[nome]
        if isinstance(valore, type):
            for classe in valore.__mro__:
                if classe.__module__.startswith("plenora_io"):
                    _confina_classe(classe)
        elif callable(valore):
            ETICHETTE.add(nome)
            spazio[nome] = _confinato(valore, nome)
