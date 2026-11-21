"""Sorgenti senza percorso; validazione ZIP e limiti appartengono al core IO."""
from __future__ import annotations
import base64
from dataclasses import dataclass
from . import _argomenti as arg

@dataclass(frozen=True)
class LimitiArchivio:
    max_voci: int = 10_000
    max_byte_decompressi: int = 512 * 1024 * 1024
    max_rapporto: int = 100

    def argv(self) -> list[str]:
        return ["--archive-max-entries", arg.intero(self.max_voci, "max_voci"),
                "--archive-max-bytes", arg.intero(self.max_byte_decompressi, "max_byte_decompressi"),
                "--archive-max-ratio", arg.intero(self.max_rapporto, "max_rapporto")]

class Bytes:
    __slots__ = ("_nome", "_dati")
    def __init__(self, nome: str, dati: bytes):
        self._nome = arg.testo(nome, "nome")
        if type(dati) is not bytes:
            raise TypeError("dati deve essere bytes")
        self._dati = dati
    @property
    def nome(self): return self._nome
    @property
    def dati(self): return self._dati

class Archivio:
    __slots__ = ("_archivio", "_voce", "_limiti")
    def __init__(self, archivio: object, voce: str, limiti: LimitiArchivio = LimitiArchivio()):
        if type(archivio) is Archivio:
            raise TypeError("archivio annidato non ammesso")
        self._archivio = archivio if type(archivio) is Bytes else arg.percorso(archivio, "archivio")
        self._voce = arg.testo(voce, "voce")
        if type(limiti) is not LimitiArchivio:
            raise TypeError("limiti archivio non validi")
        self._limiti = limiti
    @property
    def archivio(self): return self._archivio
    @property
    def voce(self): return self._voce
    @property
    def limiti(self): return self._limiti

def source_args(source: object) -> list[str]:
    if type(source) is Bytes:
        if type(source.dati) is not bytes:
            raise TypeError("dati deve essere bytes")
        return [arg.testo(source.nome, "nome"), "--source-base64", base64.b64encode(source.dati).decode("ascii")]
    if type(source) is Archivio:
        if type(source.archivio) is Archivio:
            raise TypeError("archivio annidato non ammesso")
        return [*source_args(source.archivio), "--archive-entry", arg.testo(source.voce, "voce"), *source.limiti.argv()]
    return [arg.percorso(source, "source")]
