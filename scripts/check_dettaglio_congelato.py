#!/usr/bin/env python3
"""Il dettaglio congelato del v1 non raggiunge nessuna superficie pubblica.

# Perche' questo gate esiste, e perche' e' piu' stretto di quello che sostituisce

`check_confine_v1.py` presidiava due cose: che `detail_v1()` fosse chiamato da
**un solo** posto -- l'adattatore del protocollo congelato -- e che
`FidelityAssessment` non derivasse `Serialize`.

Con la 4.0.0 l'adattatore non esiste piu', e quel gate e' stato rimosso insieme
a lui: un gate che cerca il confine fra due cose di cui una manca non presidia
niente. Ma i **dati** che proteggeva sono rimasti. `FidelityReason` porta ancora
`dettaglio_v1` -- la frase congelata con dentro i nomi presi dal file -- e
`FidelityAssessment` porta ancora `prime_v1`, le sessantaquattro frasi
trattenute. Rimuoverli e' lavoro suo, e finche' ci sono il pericolo c'e'.

Quel pericolo non e' teorico: `dettaglio_v1` contiene identificatori che chi
fornisce il file decide, di lunghezza non delimitata, ed e' esattamente cio' che
il v2 toglie dalla diagnostica. Un `#[derive(Serialize)]` rimesso per distrazione
li pubblicherebbe tutti, e un `#[derive(...)]` in piu' non si nota rileggendo un
diff.

# Le due domande

* **Il derive non torna.** L'assenza di `Serialize` su `FidelityAssessment` e'
  una scelta, e il commento accanto alla struttura lo dice. Toglierlo pero' non
  impedisce a nessuno di rimetterlo: qui la sua assenza e' **pretesa**.
* **Il dettaglio resta interno.** `detail_v1()` e' un accessore pubblico, e un
  accessore pubblico e' pubblico: la visibilita' di Rust non sa dire «questo
  modulo e nessun altro». Oggi lo legge soltanto `loss.rs`, per deduplicare; la
  prima chiamata da un altro modulo di prodotto rimetterebbe sul filo del v2 i
  nomi che il v2 toglie.

# Che cosa non guarda

Il codice di prova. Le sonde della redazione **devono** poter leggere
`detail_v1()`: e' cio' su cui verificano che quei nomi non escano. Un gate che
le contasse le renderebbe impossibili da scrivere.
"""

from __future__ import annotations

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]

#: Dove vive la struttura, e dove l'accessore puo' essere letto.
#:
#: Un solo file, e non un elenco: l'esenzione e' per il modulo che **possiede**
#: il dato, non per chiunque abbia una ragione.
CASA = pathlib.Path("crates/plenora-io-core/src/loss.rs")

STRUTTURA = re.compile(
    r"((?:#\[[^\]]*\]\s*)*)\s*pub struct FidelityAssessment\b", re.M
)
MODULO_DI_TEST = re.compile(
    r"#\[\s*cfg\s*\(\s*test\s*\)\s*\]\s*(?:#\[[^\]]*\]\s*)*"
    r"(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*\{",
    re.M,
)
ACCESSORE = re.compile(r"\bdetail_v1\b")


def senza_moduli_di_test(sorgente: str) -> str:
    """Il codice di prodotto: i moduli `cfg(test)` inline se ne vanno.

    Si conta per parentesi e non per indentazione: un modulo di prova puo'
    contenerne un altro, e una regex che si fermasse alla prima graffa chiusa
    lascerebbe dentro meta' modulo -- cioe' conterebbe come prodotto righe che
    non lo sono.
    """
    fuori = sorgente
    while True:
        trovato = MODULO_DI_TEST.search(fuori)
        if trovato is None:
            return fuori
        indice = trovato.end() - 1
        profondita = 0
        while indice < len(fuori):
            if fuori[indice] == "{":
                profondita += 1
            elif fuori[indice] == "}":
                profondita -= 1
                if profondita == 0:
                    indice += 1
                    break
            indice += 1
        fuori = fuori[: trovato.start()] + fuori[indice:]


def il_derive_non_e_tornato(radice: pathlib.Path) -> list[str]:
    """`FidelityAssessment` non deriva `Serialize`."""
    percorso = radice / CASA
    if not percorso.is_file():
        return [
            f"{CASA}: assente. La struttura che questo gate presidia non si "
            "trova, e un gate che non trova il proprio soggetto non lo protegge: "
            "se e' stata spostata, va spostato anche il gate."
        ]
    sorgente = percorso.read_text(encoding="utf-8")
    trovato = STRUTTURA.search(sorgente)
    if trovato is None:
        return [
            f"{CASA}: `pub struct FidelityAssessment` non si trova. Se e' stata "
            "rinominata o resa privata, questo gate va aggiornato invece di "
            "restare verde su un nome che non esiste."
        ]
    attributi = trovato.group(1)
    if "Serialize" in attributi:
        return [
            f"{CASA}: `FidelityAssessment` deriva `Serialize`. Il derive "
            "pubblicherebbe `prime_v1` -- le sessantaquattro frasi congelate, "
            "con dentro i nomi che chi fornisce il file decide -- e la "
            "meccanica del trattenimento. La sua assenza e' una scelta, ed e' "
            "questa riga a tenerla."
        ]
    return []


def il_dettaglio_resta_interno(radice: pathlib.Path) -> list[str]:
    """`detail_v1()` si legge soltanto dal modulo che possiede il dato."""
    errori: list[str] = []
    visti = 0
    for percorso in sorted((radice / "crates").glob("*/src/**/*.rs")):
        relativo = percorso.relative_to(radice)
        if relativo == CASA or percorso.name.endswith("_tests.rs"):
            continue
        visti += 1
        codice = senza_moduli_di_test(
            percorso.read_text(encoding="utf-8", errors="replace")
        )
        # Le righe di commento non contano: un gate che le contasse renderebbe
        # impossibile spiegare perche' la regola esiste.
        vive = "\n".join(
            riga
            for riga in codice.splitlines()
            if not riga.lstrip().startswith(("//", "///", "//!"))
        )
        if ACCESSORE.search(vive):
            errori.append(
                f"{relativo}: chiama `detail_v1()`. Il dettaglio congelato "
                "porta identificatori che chi fornisce il file decide, di "
                f"lunghezza non delimitata; leggerlo fuori da {CASA} lo rimette "
                "sul filo di una superficie pubblica, che e' cio' che il v2 "
                "esiste per impedire."
            )
    if visti == 0:
        errori.append(
            "nessun sorgente di prodotto esaminato: il gate girerebbe verde per "
            "assenza di domanda."
        )
    return errori


CONTROLLI = (
    ("il derive non e' tornato", il_derive_non_e_tornato),
    ("il dettaglio resta interno", il_dettaglio_resta_interno),
)


def main() -> int:
    problemi: list[str] = []
    for nome, controllo in CONTROLLI:
        trovati = controllo(ROOT)
        if trovati:
            print(f"--- {nome}", file=sys.stderr)
            for messaggio in trovati:
                print(f"    {messaggio}", file=sys.stderr)
        problemi.extend(trovati)
    if problemi:
        return 1
    print(
        "dettaglio congelato: `FidelityAssessment` non deriva `Serialize`, e "
        f"`detail_v1()` si legge soltanto da {CASA}."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
