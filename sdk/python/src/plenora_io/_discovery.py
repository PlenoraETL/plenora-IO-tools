"""La scoperta del binario, e il manifesto dell'artefatto che lo accompagna.

# Fail-closed vuol dire che non si inventa niente

Quattro posti, in ordine, e nessun quinto:

1. il percorso che il chiamante passa a `Client(binary=...)`;
2. la variabile d'ambiente `PLENORA_IO_BIN`;
3. `bin/plenora-io` dentro l'albero distribuito, se il pacchetto Python e'
   stato installato accanto a uno;
4. il `PATH`.

Se non c'e', si solleva `BinaryNotFound` **dicendo dove si e' cercato**. L'SDK
non scarica: un pacchetto Python che tirasse giu' un eseguibile sarebbe una via
d'esecuzione di codice che nessun lockfile controlla, e chi lo installa non
l'ha chiesto.

L'ordine non e' casuale. L'esplicito batte l'ambiente perche' chi scrive una
riga di codice sta dicendo una cosa piu' precisa di chi ha esportato una
variabile tre shell fa; l'ambiente batte l'albero perche' e' il modo in cui si
prova un binario diverso senza reinstallare; l'albero batte il `PATH` perche' un
artefatto installato porta con se' le proprie librerie, e prendere dal `PATH` un
binario di un'altra installazione le mescolerebbe.

# Il manifesto e' opzionale, la sua rottura no

`MANIFEST.json` sta nella radice dell'albero distribuito, un livello sopra
`bin/`. Un binario costruito da `cargo` non ne ha uno, ed e' perfettamente
usabile: l'assenza non e' un errore. Un manifesto **presente e illeggibile** lo
e', perche' vuol dire che l'artefatto e' rotto, e trattarlo come assente
nasconderebbe il guasto.
"""

from __future__ import annotations

import os
import shutil
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from ._argomenti import testo
from ._errors import (
    PROFILI_NOTI,
    BinaryNotFound,
    InvalidArgumentError,
    LocalIoError,
    ManifestError,
    ProfileError,
    ProtocolError,
    _tipo,
    carica_json,
    _copia_json,
)
from ._errors import _dallo_sdk, _dallo_sdk_strutturato

#: Il nome dell'eseguibile, senza estensione: `shutil.which` aggiunge da se'
#: quelle che la piattaforma usa.
NOME = "plenora-io"

#: La variabile d'ambiente, la stessa che il gate delle buste legge per
#: esercitare un binario gia' costruito.
VARIABILE = "PLENORA_IO_BIN"

#: Il manifesto, nella radice dell'albero distribuito.
MANIFESTO = "MANIFEST.json"


def _albero_accanto_al_pacchetto() -> Path | None:
    """`bin/plenora-io` accanto al pacchetto installato, se c'e'.

    Si risale dal file di questo modulo cercando una directory che contenga sia
    `bin/plenora-io` sia il manifesto: due indizi invece di uno, perche' una
    directory `bin` qualunque nel percorso di installazione non e' un albero
    distribuito, e prenderla per tale farebbe eseguire un binario altrui.
    """
    for radice in _risolto(Path(__file__), "il pacchetto installato").parents:
        candidato = radice / "bin" / NOME
        if _e_un_file(candidato, "un candidato del binario") and _e_un_file(
            radice / MANIFESTO, "il manifesto dell'artefatto"
        ):
            return candidato
    return None


def _e_un_file(percorso: Path, che_cosa: str) -> bool:
    """`Path.is_file()`, con il guasto tradotto invece che inghiottito.

    `is_file()` rende `False` per un percorso che non c'e', ma solleva
    `OSError` per uno che non si puo' esaminare -- un permesso negato su una
    directory intermedia. Trattarlo da assente farebbe passare la ricerca al
    candidato successivo, e il binario scelto sarebbe un altro da quello che
    il primo posto indicava, in silenzio: e' `LocalIoError`.
    """
    try:
        return percorso.is_file()
    except OSError:
        raise _dallo_sdk(LocalIoError, "fs.non_esaminabile", che_cosa=che_cosa) from None


def _risolto(percorso: Path, che_cosa: str) -> Path:
    """`Path.resolve()`, con il guasto tradotto: un ciclo di collegamenti
    (`RuntimeError` fino a Python 3.12, `OSError` dopo) o un permesso."""
    try:
        return percorso.resolve()
    except (OSError, RuntimeError):
        raise _dallo_sdk(LocalIoError, "fs.non_risolvibile", che_cosa=che_cosa) from None


def _percorso_indicato(esplicito: Any) -> Path:
    """Il percorso passato a `Client(binary=...)`, o `InvalidArgumentError`.

    Normalizzato in una `str` esatta da `argomenti.percorso` -- una sola
    lettura di `__fspath__`, senza NUL, senza surrogati -- prima di diventare
    un `Path`: niente del chiamante si formatta o si confronta prima.
    """
    from ._argomenti import percorso as percorso_esatto  # noqa: PLC0415

    return Path(percorso_esatto(esplicito, "binary"))


def trova_binario(esplicito: str | os.PathLike[str] | None = None) -> Path:
    """Il binario, o `BinaryNotFound` con l'elenco dei posti guardati."""
    cercati: list[str] = []

    if esplicito is not None:
        percorso = _percorso_indicato(esplicito)
        cercati.append("il percorso indicato con Client(binary=...)")
        if _e_un_file(percorso, "il percorso indicato"):
            return _risolto(percorso, "il percorso indicato")

    dall_ambiente = os.environ.get(VARIABILE)
    if dall_ambiente:
        percorso = Path(dall_ambiente)
        cercati.append("la variabile d'ambiente PLENORA_IO_BIN")
        if _e_un_file(percorso, VARIABILE):
            return _risolto(percorso, VARIABILE)
    else:
        cercati.append("PLENORA_IO_BIN (non impostata)")

    accanto = _albero_accanto_al_pacchetto()
    cercati.append("bin/plenora-io accanto al pacchetto, con MANIFEST.json")
    if accanto is not None:
        return _risolto(accanto, "il binario accanto al pacchetto")

    # `shutil.which` legge `PATH` e il filesystem: un `PATH` con un NUL o una
    # voce che non si esamina solleva, e non e' un binario introvabile.
    try:
        dal_path = shutil.which(NOME)
    except Exception:  # noqa: BLE001 - il confine traduce tutto, apposta
        raise _dallo_sdk(LocalIoError, "binario.ricerca_path") from None
    cercati.append("il `PATH`")
    if dal_path:
        return _risolto(Path(dal_path), "il binario dal PATH")

    raise _dallo_sdk_strutturato(BinaryNotFound, cercati)


@dataclass(frozen=True)
class Manifest:
    """Il `MANIFEST.json` dell'artefatto distribuito.

    Solo i campi che l'SDK usa per **decidere** qualcosa, piu' il documento
    intero in `raw`. Ricopiare qui i quattordici campi comuni li farebbe
    divergere: il manifesto della distribuzione ha il proprio gate, e questo non
    e' un secondo posto in cui dichiararne la forma.
    """

    name: str
    version: str
    platform: str
    profile: str
    channel: str
    release: bool
    revision: str | None
    raw: dict[str, Any]

    @classmethod
    def from_json(cls, documento: dict[str, Any]) -> "Manifest":
        if _tipo(documento) != "object":
            raise _dallo_sdk(ManifestError, "manifesto.non_oggetto", tipo=_tipo(documento))
        mancanti = [
            campo
            for campo in ("nome", "versione", "piattaforma", "profilo", "canale", "non_release")
            if campo not in documento
        ]
        if mancanti:
            raise _dallo_sdk(ManifestError, "manifesto.mancanti", mancanti=mancanti)
        # I tipi, non solo le chiavi. `release = not documento["non_release"]`
        # dava `True` -- «e' una release» -- per un `null`, e `False` per la
        # stringa `"false"`: un manifesto rotto si dichiarava release. I tipi
        # sono esatti per la stessa ragione di `errors._tipo`.
        attesi = {
            "nome": ("string",),
            "versione": ("string",),
            "piattaforma": ("string",),
            "profilo": ("string",),
            "canale": ("string",),
            "non_release": ("boolean",),
            "revisione": ("string", "null"),
        }
        for campo, tipi in attesi.items():
            if campo in documento and _tipo(documento[campo]) not in tipi:
                raise _dallo_sdk(
                    ManifestError, "manifesto.tipo_campo",
                    campo=campo,
                    tipo=_tipo(documento[campo]),
                    tipi=tuple(tipi),
                )
        try:
            documento = _copia_json(documento, "MANIFEST.json")
        except ProtocolError as errore:
            raise _dallo_sdk(ManifestError, "manifesto.da_errore", errore=errore) from None
        return cls(
            name=documento["nome"],
            version=documento["versione"],
            platform=documento["piattaforma"],
            profile=documento["profilo"],
            channel=documento["canale"],
            # `non_release` e' il campo del wire, e il suo verso e' negativo:
            # qui si espone `release`, perche' un booleano negato costringe chi
            # legge a fare la doppia negazione a ogni uso.
            release=not documento["non_release"],
            revision=documento.get("revisione"),
            raw=dict(documento),
        )


def leggi_manifesto(binario: Path) -> Manifest | None:
    """Il manifesto accanto al binario, o `None` se l'artefatto non ne ha.

    Cercato in `<radice>/MANIFEST.json` dove `<radice>` e' la directory che
    contiene `bin/`: e' dove i due costruttori lo scrivono.
    """
    # Un `Path` esatto da una `str` esatta: `leggi_manifesto` e' pubblica, e
    # una sottoclasse di `Path` sceglieva il testo di `read_text`, e con esso
    # il motivo e la posizione dell'errore JSON.
    from ._argomenti import percorso as percorso_esatto  # noqa: PLC0415

    binario = Path(percorso_esatto(binario, "binary"))
    radice = binario.parent.parent
    percorso = radice / MANIFESTO
    if not _e_un_file(percorso, "il manifesto dell'artefatto"):
        return None
    # Tre guasti, una traduzione: il file che non si legge, i byte che non
    # sono UTF-8, il testo che non e' JSON. Un manifesto rotto non e' un
    # manifesto assente -- l'artefatto e' guasto -- e nessuno dei tre lascia
    # la propria eccezione nella catena (`from None`): il testo del sistema o
    # del parser puo' citare il contenuto, e il percorso nel messaggio basta.
    try:
        with open(str(percorso), "rb") as file:
            dati = file.read()
    except OSError:
        raise _dallo_sdk(ManifestError, "manifesto.illeggibile") from None
    try:
        testo = bytes.decode(dati, "utf-8")
    except UnicodeDecodeError:
        raise _dallo_sdk(ManifestError, "manifesto.non_utf8") from None
    try:
        documento = carica_json(testo)
    except ProtocolError as errore:
        raise _dallo_sdk(ManifestError, "manifesto.non_json", errore=errore) from None
    if _tipo(documento) != "object":
        raise _dallo_sdk(ManifestError, "manifesto.non_oggetto_json")
    return Manifest.from_json(documento)


#: I profili che la distribuzione produce, e che cosa ciascuno porta.
#:
#: `base` e' Rust puro; `filegdb` aggiunge il runtime GDAL da cui dipende il
#: driver FileGDB. Un terzo profilo non esiste, e un manifesto che ne
#: dichiarasse uno sconosciuto e' un artefatto che questo SDK non sa descrivere.
PROFILI = PROFILI_NOTI


def verifica_profilo(manifesto: Manifest | None, richiesto: str) -> None:
    """Solleva se l'artefatto non ha il profilo richiesto.

    Senza manifesto la risposta e' **no**, non «forse». Un binario di cui non si
    sa il profilo non si puo' dichiarare adatto: dirlo adatto per non bloccare
    chi sta provando trasformerebbe questa verifica in un augurio, e il
    fallimento tornerebbe piu' avanti con un altro nome.
    """
    # `ProfileError` cita il profilo richiesto: un oggetto qualunque passerebbe
    # il proprio `__format__`, che e' codice di chi chiama, e un NUL non e' il
    # nome di un profilo.
    richiesto = testo(richiesto, "profile")
    if richiesto not in PROFILI:
        raise _dallo_sdk_strutturato(
            ProfileError, richiesto, manifesto.profile if manifesto else None
        )
    if manifesto is None or manifesto.profile != richiesto:
        raise _dallo_sdk_strutturato(
            ProfileError, richiesto, manifesto.profile if manifesto else None
        )
