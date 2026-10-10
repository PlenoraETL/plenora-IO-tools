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

from .argomenti import testo
from .errors import (
    BinaryNotFound,
    InvalidArgumentError,
    LocalIoError,
    ManifestError,
    ProfileError,
    ProtocolError,
    _tipo,
    carica_json,
    copia_json,
)

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
        raise LocalIoError(
            f"{che_cosa} non si e' potuto esaminare: un permesso o un guasto del "
            "filesystem impedisce di dire se c'e'."
        ) from None


def _risolto(percorso: Path, che_cosa: str) -> Path:
    """`Path.resolve()`, con il guasto tradotto: un ciclo di collegamenti
    (`RuntimeError` fino a Python 3.12, `OSError` dopo) o un permesso."""
    try:
        return percorso.resolve()
    except (OSError, RuntimeError):
        raise LocalIoError(
            f"{che_cosa} non si e' potuto risolvere in un percorso assoluto."
        ) from None


def _percorso_indicato(esplicito: Any) -> Path:
    """Il percorso passato a `Client(binary=...)`, o `InvalidArgumentError`.

    `Path()` chiama `__fspath__`, che e' codice di chi chiama e puo' sollevare
    qualunque cosa, e rifiuta con `TypeError` cio' che non e' un percorso.
    """
    try:
        percorso = Path(esplicito)
    except Exception:  # noqa: BLE001 - il confine traduce tutto, apposta
        raise InvalidArgumentError(
            "`binary` non e' un percorso: serve una `str` o un `os.PathLike`."
        ) from None
    if "\x00" in str(percorso):
        raise InvalidArgumentError("`binary` contiene un carattere NUL.")
    return percorso


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
        cercati.append(f"la variabile d'ambiente {VARIABILE}")
        if _e_un_file(percorso, VARIABILE):
            return _risolto(percorso, VARIABILE)
    else:
        cercati.append(f"{VARIABILE} (non impostata)")

    accanto = _albero_accanto_al_pacchetto()
    cercati.append("bin/plenora-io accanto al pacchetto, con MANIFEST.json")
    if accanto is not None:
        return _risolto(accanto, "il binario accanto al pacchetto")

    # `shutil.which` legge `PATH` e il filesystem: un `PATH` con un NUL o una
    # voce che non si esamina solleva, e non e' un binario introvabile.
    try:
        dal_path = shutil.which(NOME)
    except Exception:  # noqa: BLE001 - il confine traduce tutto, apposta
        raise LocalIoError(
            "la ricerca del binario nel `PATH` non si e' potuta fare: una voce "
            "del `PATH` non si esamina."
        ) from None
    cercati.append("il `PATH`")
    if dal_path:
        return _risolto(Path(dal_path), "il binario dal PATH")

    raise BinaryNotFound(cercati)


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
            raise ManifestError(f"MANIFEST.json e' {_tipo(documento)} e non un oggetto.")
        mancanti = [
            campo
            for campo in ("nome", "versione", "piattaforma", "profilo", "canale", "non_release")
            if campo not in documento
        ]
        if mancanti:
            raise ManifestError(
                f"MANIFEST.json senza i campi {mancanti}. Sono fra quelli che "
                "entrambi i costruttori devono scrivere, e uno che manca vuol "
                "dire che l'artefatto non e' stato prodotto dalla pipeline."
            )
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
                raise ManifestError(
                    f"MANIFEST.json: `{campo}` e' {_tipo(documento[campo])} e non "
                    f"{' o '.join(tipi)}."
                )
        try:
            documento = copia_json(documento, "MANIFEST.json")
        except ProtocolError as errore:
            raise ManifestError(str(errore)) from None
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
        testo = percorso.read_text(encoding="utf-8")
    except OSError:
        raise ManifestError(f"{MANIFESTO} c'e' e non si legge.") from None
    except UnicodeDecodeError:
        raise ManifestError(
            f"{MANIFESTO} c'e' e non e' UTF-8: l'artefatto e' guasto."
        ) from None
    try:
        documento = carica_json(testo)
    except ProtocolError as errore:
        raise ManifestError(
            f"{MANIFESTO} c'e' e non si legge come JSON: {errore} Un manifesto "
            "rotto non e' un manifesto assente: l'artefatto e' guasto."
        ) from None
    if _tipo(documento) != "object":
        raise ManifestError(f"{MANIFESTO} non contiene un oggetto JSON.")
    return Manifest.from_json(documento)


#: I profili che la distribuzione produce, e che cosa ciascuno porta.
#:
#: `base` e' Rust puro; `filegdb` aggiunge il runtime GDAL da cui dipende il
#: driver FileGDB. Un terzo profilo non esiste, e un manifesto che ne
#: dichiarasse uno sconosciuto e' un artefatto che questo SDK non sa descrivere.
PROFILI = ("base", "filegdb")


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
        raise ProfileError(richiesto, manifesto.profile if manifesto else None)
    if manifesto is None or manifesto.profile != richiesto:
        raise ProfileError(richiesto, manifesto.profile if manifesto else None)
