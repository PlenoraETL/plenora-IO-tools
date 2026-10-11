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
`object.__setattr__` o con il descrittore dello slot e' fuori contratto.

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

from .errors import InvalidArgumentError, ResultLookupError


def _esatto(valore: Any, dove: str) -> Any:
    from .confine import _esatto as esatto  # noqa: PLC0415 - confine importa questo modulo

    return esatto(valore, dove)


def _immutabile_elenco(self: Any, *argomenti: Any, **opzioni: Any) -> Any:
    raise InvalidArgumentError("un elenco di un risultato dell'SDK e' immutabile.")


def _immutabile_dizionario(self: Any, *argomenti: Any, **opzioni: Any) -> Any:
    raise InvalidArgumentError("un dizionario di un risultato dell'SDK e' immutabile.")


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

    def __new__(cls, elementi: Any = ()) -> "FrozenList":
        if cls is not FrozenList:
            raise InvalidArgumentError("`FrozenList` non si estende.")
        tipo = type(elementi)
        if tipo is FrozenList:
            sorgente = object.__getattribute__(elementi, "_dati")
        elif issubclass(tipo, list):
            sorgente = list.__iter__(elementi)
        elif issubclass(tipo, tuple):
            sorgente = tuple.__iter__(elementi)
        else:
            raise InvalidArgumentError("`FrozenList` vuole un elenco.")
        try:
            dati = tuple(_esatto(v, "FrozenList") for v in sorgente)
        except RecursionError:
            raise InvalidArgumentError("`FrozenList` e' annidato troppo a fondo.") from None
        istanza = object.__new__(cls)
        object.__setattr__(istanza, "_dati", dati)
        return istanza

    def __setattr__(self, nome: str, valore: Any) -> None:
        raise InvalidArgumentError("un elenco di un risultato dell'SDK e' immutabile.")

    def __delattr__(self, nome: str) -> None:
        raise InvalidArgumentError("un elenco di un risultato dell'SDK e' immutabile.")

    def __getitem__(self, indice: Any) -> Any:
        dati = object.__getattribute__(self, "_dati")
        if type(indice) is slice:
            return FrozenList(list(dati[indice]))
        if type(indice) is bool or not issubclass(type(indice), int):
            raise InvalidArgumentError("l'indice di un elenco e' un intero.")
        posizione = int.__index__(indice)
        if not -len(dati) <= posizione < len(dati):
            # Anche un `IndexError`: il protocollo di `Sequence` lo riconosce.
            raise ResultLookupError("indice oltre l'elenco.")
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

    def index(self, valore: Any, *argomenti: Any) -> int:
        cercato = _esatto(valore, "FrozenList")
        dati = object.__getattribute__(self, "_dati")
        if cercato not in dati:
            raise ResultLookupError("valore assente dall'elenco.")
        return dati.index(cercato, *argomenti)

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

    def __new__(cls, coppie: Any = None) -> "FrozenDict":
        if cls is not FrozenDict:
            raise InvalidArgumentError("`FrozenDict` non si estende.")
        tipo = type(coppie)
        if coppie is None:
            sorgente: Any = ()
        elif tipo is FrozenDict:
            sorgente = object.__getattribute__(coppie, "_dati").items()
        elif issubclass(tipo, dict):
            sorgente = dict.items(coppie)
        else:
            raise InvalidArgumentError("`FrozenDict` vuole un dizionario.")
        dati: dict[str, Any] = {}
        for chiave, valore in sorgente:
            if not issubclass(type(chiave), str):
                raise InvalidArgumentError("`FrozenDict` ha una chiave che non e' una stringa.")
            esatta = str.join("", [chiave])
            if esatta in dati:
                raise InvalidArgumentError(
                    "`FrozenDict` ha due chiavi che normalizzate coincidono."
                )
            try:
                dati[esatta] = _esatto(valore, "FrozenDict")
            except RecursionError:
                raise InvalidArgumentError("`FrozenDict` e' annidato troppo a fondo.") from None
        istanza = object.__new__(cls)
        object.__setattr__(istanza, "_dati", dati)
        return istanza

    def __setattr__(self, nome: str, valore: Any) -> None:
        raise InvalidArgumentError("un dizionario di un risultato dell'SDK e' immutabile.")

    def __delattr__(self, nome: str) -> None:
        raise InvalidArgumentError("un dizionario di un risultato dell'SDK e' immutabile.")

    def __getitem__(self, chiave: Any) -> Any:
        # `ResultLookupError` e' anche un `KeyError`: `get()` e `in` di
        # `Mapping` lo riconoscono. Il messaggio non nomina la chiave.
        if not issubclass(type(chiave), str):
            raise ResultLookupError("chiave non testuale.")
        dati = object.__getattribute__(self, "_dati")
        esatta = str.join("", [chiave])
        if esatta not in dati:
            raise ResultLookupError("chiave assente dal dizionario.")
        return dati[esatta]

    def __len__(self) -> int:
        return len(object.__getattribute__(self, "_dati"))

    def __iter__(self) -> Iterator[str]:
        return iter(object.__getattribute__(self, "_dati"))

    def __contains__(self, chiave: Any) -> bool:
        if not issubclass(type(chiave), str):
            return False
        return str.join("", [chiave]) in object.__getattribute__(self, "_dati")

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


def to_plain(valore: Any) -> Any:
    """Il valore con `FrozenList` e `FrozenDict` resi `list` e `dict` ordinari."""
    tipo = type(valore)
    if tipo is FrozenList:
        return [to_plain(v) for v in valore]
    if tipo is FrozenDict:
        return {k: to_plain(v) for k, v in object.__getattribute__(valore, "_dati").items()}
    return valore


def json_default(valore: Any) -> Any:
    """Per `json.dumps(..., default=json_default)`: i contenitori immutabili."""
    tipo = type(valore)
    if tipo is FrozenList or tipo is FrozenDict:
        return to_plain(valore)
    raise TypeError("non e' un contenitore dell'SDK")
