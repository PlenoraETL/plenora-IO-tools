"""I contenitori immutabili dei modelli: `FrozenList` e `FrozenDict`.

# Perche' classi proprie, e non sottoclassi di `list` e `dict`

Una sottoclasse eredita tutto cio' che non ridefinisce: anche `__init__`, che
su `list` e `dict` e' un metodo che **modifica** l'istanza. Con le sottoclassi
`catalogo.drivers.__init__([])` svuotava un elenco «immutabile», e ogni metodo
mutante che l'elenco dei bloccati avesse dimenticato restava aperto.

Qui i due contenitori stanno su `collections.abc.Sequence` e `Mapping`, con
`__slots__` e i dati in un attributo privato (`tuple` per l'elenco, `dict` per
il dizionario) che nessun metodo pubblico rende. Non esiste un metodo che
modifichi: `__setattr__` e `__delattr__` sollevano `InvalidArgumentError`, e
`__init__` non c'e' (la costruzione avviene in `__new__`). Scavalcare con
`object.__setattr__` o con il descrittore dello slot, o leggere e modificare
`_dati` direttamente, e' fuori contratto: il README dell'SDK lo dichiara fra i
limiti («La rete non e' una barriera contro codice ostile nello stesso
interprete»).

# I costruttori normalizzano

`FrozenList(x)` e `FrozenDict(x)` normalizzano ogni elemento come un campo di
un modello (`confine._esatto`): tipi base esatti, contenitori immutabili,
modelli dell'SDK, nient'altro. Costruirli direttamente non porta dentro un
oggetto del chiamante, e `pickle`, `copy` e `deepcopy` passano dallo stesso
costruttore (`__reduce__`).

# Uguaglianza e `hash`

`FrozenList` e' uguale a una `list` o a un'altra `FrozenList` con gli stessi
elementi; `FrozenDict` a un `dict` o a un altro `FrozenDict` con le stesse
coppie: il confronto con un documento JSON resta vero. L'altro operando si
normalizza prima di essere confrontato; se non si lascia normalizzare, non e'
uguale. `hash` e' coerente con `==` fra contenitori immutabili; `list` e `dict`
non sono hashabili, quindi un contenitore uguale a un `dict` ma con un `hash`
che il `dict` non ha non rompe il contratto di Python.

# `json.dumps`

Non sono piu' `list` e `dict`: `json.dumps(modello.raw)` vuole
`default=plenora_io.json_default`, oppure `plenora_io.to_plain(...)` prima.
"""

from __future__ import annotations

from collections.abc import Iterator, Mapping, Sequence
from typing import Any

from ._errors import InvalidArgumentError, ResultLookupError
from ._errors import _dallo_sdk


def _esatto(valore: Any, dove: str) -> Any:
    from ._confine import _esatto as esatto  # noqa: PLC0415 - confine importa questo modulo

    return esatto(valore, dove)


def _elenco_congelato(dati: tuple[Any, ...]) -> "FrozenList":
    """Una `FrozenList` da elementi gia' normalizzati, senza rivisitarli."""
    istanza = object.__new__(FrozenList)
    object.__setattr__(istanza, "_dati", dati)
    return istanza


def _dizionario_congelato(dati: dict[str, Any]) -> "FrozenDict":
    """Un `FrozenDict` da coppie gia' normalizzate, senza rivisitarle."""
    istanza = object.__new__(FrozenDict)
    object.__setattr__(istanza, "_dati", dati)
    return istanza


def _protocollo(valore: Any) -> int:
    """Il protocollo di `__reduce_ex__`: un intero esatto non negativo."""
    if type(valore) is not int or valore < 0:
        raise _dallo_sdk(InvalidArgumentError, "serializzazione.protocollo")
    return valore


def _memo(valore: Any) -> Any:
    """Il `memo` di `__deepcopy__`: un `dict` esatto, o `None`."""
    if valore is not None and type(valore) is not dict:
        raise _dallo_sdk(InvalidArgumentError, "serializzazione.memo")
    return valore


def _unico_argomento(nome: str, argomenti: tuple[Any, ...], opzioni: dict[str, Any],
                     predefinito: Any) -> Any:
    """L'argomento del costruttore, uno solo e per posizione.

    La firma e' aperta apposta: con `coppie=None` un argomento per nome
    sconosciuto era un `TypeError` di binding dell'interprete, e il suo
    messaggio riportava il nome -- `FrozenDict(**{chiave: 1})` metteva la
    chiave nell'errore. Qui ogni forma sbagliata e' `InvalidArgumentError`, e
    il messaggio non nomina niente di cio' che si e' passato.
    """
    if opzioni or len(argomenti) > 1:
        raise _dallo_sdk(InvalidArgumentError, "contenitore.argomenti", nome=nome)
    return argomenti[0] if argomenti else predefinito


def _intero_o_niente(valore: Any, quale: str) -> int | None:
    """`None`, o l'intero esatto: `__index__` del chiamante non gira mai.

    `quale` dice di che componente si tratta, e sceglie il messaggio.
    """
    if valore is None:
        return None
    tipo = type(valore)
    if tipo is bool or not issubclass(tipo, int):
        if quale == "passo":
            raise _dallo_sdk(InvalidArgumentError, "elenco.passo_non_intero")
        if quale == "inizio":
            raise _dallo_sdk(InvalidArgumentError, "elenco.inizio_non_intero")
        if quale == "fine":
            raise _dallo_sdk(InvalidArgumentError, "elenco.fine_non_intero")
        raise _dallo_sdk(InvalidArgumentError, "elenco.limite_non_intero")
    return int.__index__(valore)


def _immutabile_elenco(self: Any, *argomenti: Any, **opzioni: Any) -> Any:
    raise _dallo_sdk(InvalidArgumentError, "elenco.immutabile")


def _immutabile_dizionario(self: Any, *argomenti: Any, **opzioni: Any) -> Any:
    raise _dallo_sdk(InvalidArgumentError, "dizionario.immutabile")


class FrozenList(Sequence):
    """Un elenco immutabile, con elementi normalizzati.

    I metodi di modifica di `list` esistono solo per rispondere con un errore
    dell'SDK invece che con l'`AttributeError` o il `TypeError` dell'interprete:
    nessuno modifica niente.
    """

    __slots__ = ("_dati",)

    __setitem__ = _immutabile_elenco
    __delitem__ = _immutabile_elenco
    __iadd__ = _immutabile_elenco
    __imul__ = _immutabile_elenco
    append = _immutabile_elenco
    extend = _immutabile_elenco
    insert = _immutabile_elenco
    remove = _immutabile_elenco
    pop = _immutabile_elenco
    clear = _immutabile_elenco
    sort = _immutabile_elenco
    reverse = _immutabile_elenco

    def __new__(cls, *argomenti: Any, **opzioni: Any) -> "FrozenList":
        if cls is not FrozenList:
            raise _dallo_sdk(InvalidArgumentError, "contenitore.non_si_estende", nome='FrozenList')
        elementi = _unico_argomento("FrozenList", argomenti, opzioni, ())
        tipo = type(elementi)
        if tipo is FrozenList:
            sorgente = object.__getattribute__(elementi, "_dati")
        elif issubclass(tipo, list):
            sorgente = list.__iter__(elementi)
        elif issubclass(tipo, tuple):
            sorgente = tuple.__iter__(elementi)
        else:
            raise _dallo_sdk(InvalidArgumentError, "elenco.vuole_elenco")
        try:
            dati = tuple(_esatto(v, "FrozenList") for v in sorgente)
        except RecursionError:
            raise _dallo_sdk(InvalidArgumentError, "modello.annidato", dove='FrozenList') from None
        istanza = object.__new__(cls)
        object.__setattr__(istanza, "_dati", dati)
        return istanza

    def __setattr__(self, nome: str, valore: Any) -> None:
        raise _dallo_sdk(InvalidArgumentError, "elenco.immutabile")

    def __delattr__(self, nome: str) -> None:
        raise _dallo_sdk(InvalidArgumentError, "elenco.immutabile")

    def __getitem__(self, indice: Any) -> Any:
        dati = object.__getattribute__(self, "_dati")
        if type(indice) is slice:
            # I tre componenti si normalizzano prima dell'uso: affettare una
            # `tuple` con lo slice del chiamante eseguiva il loro `__index__`,
            # e cio' che sollevava -- anche un errore dell'SDK con un dato --
            # usciva intatto.
            passo = _intero_o_niente(indice.step, "passo")
            if passo == 0:
                raise _dallo_sdk(InvalidArgumentError, "elenco.passo_zero")
            esatto = slice(
                _intero_o_niente(indice.start, "inizio"),
                _intero_o_niente(indice.stop, "fine"),
                passo,
            )
            return FrozenList(list(dati[esatto]))
        if type(indice) is bool or not issubclass(type(indice), int):
            raise _dallo_sdk(InvalidArgumentError, "elenco.indice_non_intero")
        posizione = int.__index__(indice)
        if not -len(dati) <= posizione < len(dati):
            # Anche un `IndexError`: il protocollo di `Sequence` lo riconosce.
            raise _dallo_sdk(ResultLookupError, "elenco.indice_oltre")
        return dati[posizione]

    def __len__(self) -> int:
        return len(object.__getattribute__(self, "_dati"))

    def __iter__(self) -> Iterator[Any]:
        return iter(object.__getattribute__(self, "_dati"))

    def __contains__(self, valore: Any) -> bool:
        # Il valore si normalizza prima del confronto: un oggetto del chiamante
        # non viene confrontato con i nostri elementi.
        try:
            cercato = _esatto(valore, "FrozenList")
        except InvalidArgumentError:
            return False
        return any(elemento == cercato for elemento in object.__getattribute__(self, "_dati"))

    def index(self, valore: Any, *argomenti: Any, **opzioni: Any) -> int:
        # `inizio` e `fine` sono interi esatti o `None` prima dell'uso, come i
        # componenti di uno slice: `tuple.index` ne chiamava `__index__`.
        if opzioni or len(argomenti) > 2:
            raise _dallo_sdk(InvalidArgumentError, "elenco.index_argomenti")
        limiti = [_intero_o_niente(a, "limite") for a in argomenti]
        cercato = _esatto(valore, "FrozenList")
        dati = object.__getattribute__(self, "_dati")
        inizio, fine, _ = slice(*(limiti + [None] * (2 - len(limiti)))).indices(len(dati))
        for posizione in range(inizio, fine):
            if dati[posizione] == cercato:
                return posizione
        raise _dallo_sdk(ResultLookupError, "elenco.valore_assente")

    def count(self, valore: Any) -> int:
        cercato = _esatto(valore, "FrozenList")
        return object.__getattribute__(self, "_dati").count(cercato)

    def __eq__(self, altro: Any) -> bool:
        tipo = type(altro)
        if tipo is not FrozenList and tipo is not list:
            return NotImplemented
        try:
            normalizzato = altro if tipo is FrozenList else FrozenList(altro)
        except InvalidArgumentError:
            return False
        return object.__getattribute__(self, "_dati") == object.__getattribute__(
            normalizzato, "_dati"
        )

    def __ne__(self, altro: Any) -> bool:
        uguale = self.__eq__(altro)
        return uguale if uguale is NotImplemented else not uguale

    def __hash__(self) -> int:
        return hash(("FrozenList", object.__getattribute__(self, "_dati")))

    def __repr__(self) -> str:
        return repr(list(object.__getattribute__(self, "_dati")))

    def __reduce__(self) -> Any:
        return (FrozenList, (list(object.__getattribute__(self, "_dati")),))

    # Il protocollo di serializzazione e' dell'SDK, nella rete: ereditato da
    # `object` lasciava uscire `TypeError` per un argomento sbagliato, e un
    # `__index__` ostile la propria eccezione (diciannovesimo giro).

    def __reduce_ex__(self, protocollo: Any) -> Any:
        _protocollo(protocollo)
        return self.__reduce__()

    def __getstate__(self) -> None:
        return None

    def __copy__(self) -> "FrozenList":
        return _elenco_congelato(object.__getattribute__(self, "_dati"))

    def __deepcopy__(self, memo: Any = None) -> "FrozenList":
        _memo(memo)
        return FrozenList(to_plain(self))

    def __reversed__(self) -> Iterator[Any]:
        return reversed(object.__getattribute__(self, "_dati"))


class FrozenDict(Mapping):
    """Un dizionario immutabile, con chiavi `str` e valori normalizzati.

    Come per `FrozenList`, i metodi di modifica di `dict` rispondono con un
    errore dell'SDK e non modificano niente.
    """

    __slots__ = ("_dati",)

    __setitem__ = _immutabile_dizionario
    __delitem__ = _immutabile_dizionario
    __ior__ = _immutabile_dizionario
    clear = _immutabile_dizionario
    pop = _immutabile_dizionario
    popitem = _immutabile_dizionario
    setdefault = _immutabile_dizionario
    update = _immutabile_dizionario

    def __new__(cls, *argomenti: Any, **opzioni: Any) -> "FrozenDict":
        if cls is not FrozenDict:
            raise _dallo_sdk(InvalidArgumentError, "contenitore.non_si_estende", nome='FrozenDict')
        coppie = _unico_argomento("FrozenDict", argomenti, opzioni, None)
        tipo = type(coppie)
        if coppie is None:
            sorgente: Any = ()
        elif tipo is FrozenDict:
            sorgente = object.__getattribute__(coppie, "_dati").items()
        elif issubclass(tipo, dict):
            sorgente = dict.items(coppie)
        else:
            raise _dallo_sdk(InvalidArgumentError, "dizionario.vuole_dizionario")
        dati: dict[str, Any] = {}
        for chiave, valore in sorgente:
            if not issubclass(type(chiave), str):
                raise _dallo_sdk(
                    InvalidArgumentError, "modello.chiave_non_stringa",
                    dove='FrozenDict',
                )
            esatta = str.join("", [chiave])
            if esatta in dati:
                raise _dallo_sdk(
                    InvalidArgumentError, "modello.chiavi_coincidenti",
                    dove='FrozenDict',
                )
            try:
                dati[esatta] = _esatto(valore, "FrozenDict")
            except RecursionError:
                raise _dallo_sdk(
                    InvalidArgumentError, "modello.annidato",
                    dove='FrozenDict',
                ) from None
        istanza = object.__new__(cls)
        object.__setattr__(istanza, "_dati", dati)
        return istanza

    def __setattr__(self, nome: str, valore: Any) -> None:
        raise _dallo_sdk(InvalidArgumentError, "dizionario.immutabile")

    def __delattr__(self, nome: str) -> None:
        raise _dallo_sdk(InvalidArgumentError, "dizionario.immutabile")

    def __getitem__(self, chiave: Any) -> Any:
        # `ResultLookupError` e' anche un `KeyError`: `get()` e `in` di
        # `Mapping` lo riconoscono. Il messaggio non nomina la chiave.
        if not issubclass(type(chiave), str):
            raise _dallo_sdk(ResultLookupError, "dizionario.chiave_non_testuale")
        dati = object.__getattribute__(self, "_dati")
        esatta = str.join("", [chiave])
        if esatta not in dati:
            raise _dallo_sdk(ResultLookupError, "dizionario.chiave_assente")
        return dati[esatta]

    def __len__(self) -> int:
        return len(object.__getattribute__(self, "_dati"))

    def __iter__(self) -> Iterator[str]:
        return iter(object.__getattribute__(self, "_dati"))

    def __contains__(self, chiave: Any) -> bool:
        if not issubclass(type(chiave), str):
            return False
        return str.join("", [chiave]) in object.__getattribute__(self, "_dati")

    # `get`, `keys`, `items` e `values` erano i metodi misti di `Mapping`:
    # codice della libreria standard, fuori dalla rete, che rendeva viste di
    # Python (diciassettesimo giro). Ora sono dell'SDK, e rendono contenitori
    # dell'SDK: le chiavi e i valori come `FrozenList`, le coppie come
    # `FrozenList` di due elementi.

    def get(self, chiave: Any, predefinito: Any = None) -> Any:
        if not issubclass(type(chiave), str):
            return predefinito
        dati = object.__getattribute__(self, "_dati")
        esatta = str.join("", [chiave])
        return dati[esatta] if esatta in dati else predefinito

    def keys(self) -> "FrozenList":
        return FrozenList(list(object.__getattribute__(self, "_dati")))

    def values(self) -> "FrozenList":
        return FrozenList(list(object.__getattribute__(self, "_dati").values()))

    def items(self) -> "FrozenList":
        return FrozenList(
            [[chiave, valore] for chiave, valore in object.__getattribute__(self, "_dati").items()]
        )

    def __eq__(self, altro: Any) -> bool:
        tipo = type(altro)
        if tipo is not FrozenDict and tipo is not dict:
            return NotImplemented
        try:
            normalizzato = altro if tipo is FrozenDict else FrozenDict(altro)
        except InvalidArgumentError:
            return False
        return object.__getattribute__(self, "_dati") == object.__getattribute__(
            normalizzato, "_dati"
        )

    def __ne__(self, altro: Any) -> bool:
        uguale = self.__eq__(altro)
        return uguale if uguale is NotImplemented else not uguale

    def __hash__(self) -> int:
        return hash(("FrozenDict", tuple(sorted(object.__getattribute__(self, "_dati").items()))))

    def __repr__(self) -> str:
        return repr(dict(object.__getattribute__(self, "_dati")))

    def __reduce__(self) -> Any:
        return (FrozenDict, (dict(object.__getattribute__(self, "_dati")),))

    def __reduce_ex__(self, protocollo: Any) -> Any:
        _protocollo(protocollo)
        return self.__reduce__()

    def __getstate__(self) -> None:
        return None

    def __copy__(self) -> "FrozenDict":
        return _dizionario_congelato(dict(object.__getattribute__(self, "_dati")))

    def __deepcopy__(self, memo: Any = None) -> "FrozenDict":
        _memo(memo)
        return FrozenDict(to_plain(self))


def to_plain(valore: Any) -> Any:
    """Una copia profonda nuova, con `FrozenList` e `FrozenDict` resi `list` e
    `dict` ordinari.

    Copia anche i contenitori gia' nativi -- `list`, `tuple` (resa `list`) e
    `dict`, letti con i metodi dei tipi base -- cosi' che il valore reso non
    condivida niente con quello ricevuto: prima un ingresso nativo tornava
    identico. Le foglie restano quelle che sono.
    """
    tipo = type(valore)
    if tipo is FrozenList:
        return [to_plain(v) for v in object.__getattribute__(valore, "_dati")]
    if tipo is FrozenDict:
        return {k: to_plain(v) for k, v in object.__getattribute__(valore, "_dati").items()}
    if issubclass(tipo, list):
        return [to_plain(v) for v in list.__iter__(valore)]
    if issubclass(tipo, tuple):
        return [to_plain(v) for v in tuple.__iter__(valore)]
    if issubclass(tipo, dict):
        return {k: to_plain(v) for k, v in dict.items(valore)}
    return valore


def json_default(valore: Any) -> Any:
    """Per `json.dumps(..., default=json_default)`: i contenitori immutabili."""
    tipo = type(valore)
    if tipo is FrozenList or tipo is FrozenDict:
        return to_plain(valore)
    raise TypeError("non e' un contenitore dell'SDK")
