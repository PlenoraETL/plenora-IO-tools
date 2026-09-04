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

import json
import os
import shutil
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from .errors import BinaryNotFound, ManifestError, ProfileError

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
    for radice in Path(__file__).resolve().parents:
        candidato = radice / "bin" / NOME
        if candidato.is_file() and (radice / MANIFESTO).is_file():
            return candidato
    return None


def trova_binario(esplicito: str | os.PathLike[str] | None = None) -> Path:
    """Il binario, o `BinaryNotFound` con l'elenco dei posti guardati."""
    cercati: list[str] = []

    if esplicito is not None:
        percorso = Path(esplicito)
        cercati.append(f"il percorso indicato: {percorso}")
        if percorso.is_file():
            return percorso.resolve()

    dall_ambiente = os.environ.get(VARIABILE)
    if dall_ambiente:
        percorso = Path(dall_ambiente)
        cercati.append(f"{VARIABILE}={percorso}")
        if percorso.is_file():
            return percorso.resolve()
    else:
        cercati.append(f"{VARIABILE} (non impostata)")

    accanto = _albero_accanto_al_pacchetto()
    cercati.append("bin/plenora-io accanto al pacchetto, con MANIFEST.json")
    if accanto is not None:
        return accanto.resolve()

    dal_path = shutil.which(NOME)
    cercati.append(f"PATH ({os.environ.get('PATH', '')[:120]}...)")
    if dal_path:
        return Path(dal_path).resolve()

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
    if not percorso.is_file():
        return None
    try:
        testo = percorso.read_text(encoding="utf-8")
    except OSError as errore:
        raise ManifestError(f"{percorso} non si legge: {errore}") from errore
    try:
        documento = json.loads(testo)
    except json.JSONDecodeError as errore:
        raise ManifestError(
            f"{percorso} c'e' e non e' JSON valido: {errore}. Un manifesto "
            "rotto non e' un manifesto assente: l'artefatto e' guasto."
        ) from errore
    if not isinstance(documento, dict):
        raise ManifestError(f"{percorso} non contiene un oggetto JSON.")
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
    if richiesto not in PROFILI:
        raise ProfileError(richiesto, manifesto.profile if manifesto else None)
    if manifesto is None or manifesto.profile != richiesto:
        raise ProfileError(richiesto, manifesto.profile if manifesto else None)
