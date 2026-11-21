"""Sorgenti senza percorso: byte in memoria e voci di un archivio ZIP.

# Chi decide che cosa

La validazione dei nomi, degli ZIP e dei limiti appartiene al core IO, e da li'
arriva con le stesse buste e gli stessi codici in Rust, CLI e SDK: voce assente
`SOURCE_NOT_FOUND`, nome non portabile e limite zero `CLI_USAGE`, archivio
annidato `UNSUPPORTED`, limite superato `LIMIT_EXCEEDED`. Qui si verificano
solo i tipi, con `InvalidArgumentError` come ogni altro argomento dell'SDK.

# Come viaggiano i byte

Mai negli argomenti: `--source-stdin` sulla riga di comando, e i byte sullo
stdin del processo (`Runner`) o in memoria nel modulo nativo (`NativeRunner`).
Un archivio dentro un archivio non ha una forma sulla riga di comando (una sola
`--archive-entry`): l'SDK lo rifiuta prima di eseguire, con la chiave
`sorgente.annidata`; un KMZ dentro uno ZIP arriva invece al core, che lo
rifiuta come annidato.
"""
from __future__ import annotations

from dataclasses import dataclass, fields
from typing import Any

from . import _argomenti as arg
from ._errors import InvalidArgumentError, _dallo_sdk

#: Il campo e la sua opzione: la tabella `archivio::REGOLE` del core.
FLAG_DEI_LIMITI: dict[str, str] = {
    "max_voci": "--archive-max-entries",
    "max_byte_decompressi": "--archive-max-bytes",
    "max_rapporto": "--archive-max-ratio",
}


@dataclass(frozen=True)
class LimitiArchivio:
    """I limiti degli archivi ZIP: `--archive-max-*`, stretti dalle quote.

    Lo zero e un valore oltre una quota della pipeline li giudica il core:
    il primo e' `CLI_USAGE`, il secondo e' dichiarato in `archive-list`.
    """

    max_voci: int = 10_000
    max_byte_decompressi: int = 512 * 1024 * 1024
    max_rapporto: int = 100

    def __post_init__(self) -> None:
        for campo in fields(LimitiArchivio):
            valore = object.__getattribute__(self, campo.name)
            object.__setattr__(
                self, campo.name, arg._int_esatto(valore, f"limiti.{campo.name}"))

    def argv(self) -> list[str]:
        argomenti: list[str] = []
        for campo in fields(LimitiArchivio):
            argomenti += [
                FLAG_DEI_LIMITI[campo.name],
                int.__repr__(object.__getattribute__(self, campo.name)),
            ]
        return argomenti


class Bytes:
    """Byte in memoria; `nome` sceglie il driver ed e' il nome nella vista."""

    __slots__ = ("_nome", "_dati")

    def __init__(self, nome: str, dati: bytes) -> None:
        self._nome = arg.testo(nome, "nome")
        if type(dati) is not bytes:
            raise _dallo_sdk(InvalidArgumentError, "sorgente.dati_non_bytes", nome="dati")
        self._dati = dati

    @property
    def nome(self) -> str:
        return self._nome

    @property
    def dati(self) -> bytes:
        return self._dati


class Archivio:
    """La voce `voce` dell'archivio ZIP `archivio`: un percorso o `Bytes`."""

    __slots__ = ("_archivio", "_voce", "_limiti")

    def __init__(
        self, archivio: object, voce: str, limiti: LimitiArchivio | None = None
    ) -> None:
        if type(archivio) is Archivio:
            raise _dallo_sdk(InvalidArgumentError, "sorgente.annidata", nome="archivio")
        self._archivio = (
            archivio if type(archivio) is Bytes else arg.percorso(archivio, "archivio"))
        self._voce = arg.testo(voce, "voce")
        if limiti is not None and type(limiti) is not LimitiArchivio:
            raise _dallo_sdk(InvalidArgumentError, "sorgente.limiti", nome="limiti")
        self._limiti = limiti

    @property
    def archivio(self) -> str | Bytes:
        return self._archivio

    @property
    def voce(self) -> str:
        return self._voce

    @property
    def limiti(self) -> LimitiArchivio | None:
        return self._limiti


def sorgente_non_percorso(source: object) -> bool:
    return type(source) is Bytes or type(source) is Archivio


def source_args(source: object, limiti: Any = None) -> list[str]:
    """Gli argomenti della sorgente, una volta sola ciascuno.

    I limiti vengono dall'`Archivio` o da `limiti`, mai da entrambi: la CLI
    rifiuta un flag ripetuto, e due valori diversi non avrebbero un vincitore.
    """
    if limiti is not None and type(limiti) is not LimitiArchivio:
        raise _dallo_sdk(InvalidArgumentError, "sorgente.limiti", nome="limiti")
    if type(source) is Archivio:
        if limiti is not None and source.limiti is not None:
            raise _dallo_sdk(InvalidArgumentError, "sorgente.limiti_doppi", nome="limiti")
        scelti = source.limiti if source.limiti is not None else limiti
        coda = ["--archive-entry", source.voce]
        coda += [] if scelti is None else scelti.argv()
        return [*source_args(source.archivio), *coda]
    coda = [] if limiti is None else limiti.argv()
    if type(source) is Bytes:
        return [source.nome, "--source-stdin", *coda]
    return [arg.percorso(source, "source"), *coda]


def ingresso(source: object) -> bytes | None:
    """I byte da consegnare su stdin, se la sorgente ne porta."""
    if type(source) is Archivio:
        return ingresso(source.archivio)
    if type(source) is Bytes:
        return source.dati
    return None
