"""Censimento dei fallback `unwrap_or*`, contato sul codice e non sul testo.

# Perche' esiste

Il registro dei fallback nasce da H-01: ogni `unwrap_or*` e' una decisione presa
in mancanza di meglio, e il numero di quelle decisioni non deve muoversi senza
che qualcuno lo abbia scritto. Il meccanismo era un `rg -o` sul sorgente
grezzo.

Il 2026-08-21, migrando `driver-filegdb`, un **commento** ha fatto salire il
contatore. Il commento spiegava perche' in quel punto *non* si stesse usando
`unwrap_or(...)`, e conteneva quella stringa: il gate l'ha contata come se ci
fosse una chiamata.

E' la stessa classe di fragilita' che INFRA-1 aveva chiuso per il censimento dei
tetti WKB, sostituendo `path:riga` con `percorso::funzione`. Un gate che guarda
il testo si accende su cio' che il testo dice, non su cio' che il codice fa — e
il modo in cui si impara a conviverci e' peggiore del difetto: si riformula il
commento, e la prossima volta si riformula senza pensarci.

# Che cosa cambia

Prima di contare, commenti e stringhe letterali sono sostituiti con spazi. Lo
strip e' quello di `check_errori_redatti.py`, che esiste per la stessa ragione:
quel gate nomina i costruttori legacy in ogni riga della propria motivazione, e
senza spoglio conterebbe se stesso.

Il registro **non e' stato riallineato** all'introduzione di questo script: i
conteggi qui sono quelli che il gate testuale dava, meno le occorrenze che erano
solo commenti o stringhe. Ogni differenza e' annotata.
"""

from __future__ import annotations

import pathlib
import json
import re
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

from check_errori_redatti import spoglia  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent

CHIAMATA = re.compile(r"\bunwrap_or(?:_else|_default)?\s*\(")

# Il registro. Ogni numero e' una decisione scritta in
# `assurance/registries/fallback-register.json`; muoverlo senza aggiornare quella
# motivazione e' esattamente cio' che il gate esiste per impedire.
# Il registro vive in `assurance/registries/fallback-register.json`: numero e
# ragione nello stesso posto, in una forma che si puo' validare. Prima il
# numero stava qui e la ragione in un Markdown, ed erano due fonti che
# potevano divergere senza che nulla lo dicesse.
REGISTRO = ROOT / "assurance" / "registries" / "fallback-register.json"
_registro = json.loads(REGISTRO.read_text(encoding="utf-8"))
ATTESI: dict[str, int] = {
    crate: voce["conteggio"] for crate, voce in _registro["per_crate"].items()
}
TOTALE_ATTESO = _registro["totale"]


def conta(crate: str) -> int:
    """Occorrenze di `unwrap_or*` nel **codice** di un crate."""
    totale = 0
    for sorgente in sorted((ROOT / "crates" / crate).rglob("*.rs")):
        testo = spoglia(sorgente.read_text(encoding="utf-8"))
        totale += len(CHIAMATA.findall(testo))
    return totale


def verifica(attesi: dict[str, int], totale_atteso: int) -> list[str]:
    errori: list[str] = []

    # Un conteggio senza ragione e' una casella riempita: e' la stessa regola
    # che ASSURANCE-N1 applica alle disposizioni.
    for crate, voce in _registro["per_crate"].items():
        if not voce.get("ragione"):
            errori.append(f"{crate}: conteggio senza ragione nel registro")

    totale = 0
    for crate, atteso in attesi.items():
        if not (ROOT / "crates" / crate).is_dir():
            errori.append(f"{crate}: registrato ma il crate non esiste")
            continue
        trovati = conta(crate)
        totale += trovati
        if trovati != atteso:
            errori.append(f"{crate}: fallback registrati={atteso}, trovati={trovati}")

    presenti = {
        percorso.name
        for percorso in (ROOT / "crates").iterdir()
        if percorso.is_dir() and (percorso / "Cargo.toml").exists()
    }
    # Un crate nuovo non deve entrare senza una riga nel registro: senza questo
    # controllo il gate resterebbe verde su un crate che nessuno ha guardato.
    for crate in sorted(presenti - set(attesi)):
        errori.append(f"{crate}: crate presente ma non registrato")

    if totale != totale_atteso:
        errori.append(f"totale fallback del workspace inatteso: {totale}")
    return errori


def main() -> int:
    errori = verifica(ATTESI, TOTALE_ATTESO)
    for messaggio in errori:
        print(messaggio, file=sys.stderr)
    if errori:
        return 1
    print(f"fallback assurance verificati: {TOTALE_ATTESO}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
