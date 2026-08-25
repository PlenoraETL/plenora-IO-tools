#!/usr/bin/env python3
"""Censimento dei `WkbLimits::default()` residui (Lotto 0 / S5, permanente).

S5 ha portato le quote configurate fino all'inferenza di CSV, GeoJSON e XLSX:
fino ad allora quelle passate usavano il default del contratto, e
`--max-wkb-cell-bytes` non le raggiungeva. Chi stringeva il flag otteneva un
rifiuto piu' tardi, o non lo otteneva affatto.

Restano occorrenze legittime, e vanno tenute distinte da quelle che sarebbero
un ritorno del difetto. Questo gate le classifica e fissa il conteggio: **non**
vieta il simbolo — sarebbe sbagliato, alcune di quelle occorrenze sono corrette
— ma impedisce che ne compaia una nuova senza che qualcuno la classifichi.

## Il censimento e' strutturale, non per riga

Fino a S6 la chiave era `percorso:riga`. Era fragile per costruzione: qualunque
modifica sopra un'occorrenza — una dichiarazione aggiunta, una riformattazione,
un commento — la spostava, e il gate diventava rosso a codice **invariato**. E'
successo con le dichiarazioni di schema di S6, e sarebbe successo ancora. Un
gate che si accende sui movimenti insegna a riallinearlo senza guardare, che e'
il modo in cui un gate smette di essere letto.

La chiave e' ora `percorso::funzione` con il **numero di occorrenze attese
dentro quella funzione**. Sposta pure il codice, riformattalo, aggiungi righe
sopra: la chiave non cambia. Aggiungi una `WkbLimits::default()` in una
funzione nuova, o una seconda in una gia' censita, e il gate diventa rosso —
che e' l'unico caso in cui deve.

## Le categorie

* **test** — un modulo `#[cfg(test)]` che decodifica un WKB prodotto dal test
  stesso. Il tetto non governa nulla: il dato e' noto e piccolo. Sarebbe
  rumore imporre quote configurate a un `decode_wkb` di verifica.
* **attrezzaggio** — `plenora-bench` e `plenora-fuzz`. Non sono codice
  spedito; il fuzz harness ha le proprie quote strette, e il benchmark misura
  il percorso, non le quote.
* **produzione** — tutto il resto. Ogni occorrenza qui deve avere una ragione
  scritta accanto, e il gate la elenca perche' sia visibile in review.

Come il registro dei fallback la misura e' sintattica, ma il sorgente viene
prima **spogliato di commenti e stringhe**: la doc che spiega perche' un
default e' stato rimosso ne nomina il simbolo, e senza lo spoglio il gate
conterebbe la propria motivazione come residuo. E' successo alla prima corsa
dopo S5.1.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

OCCORRENZA = re.compile(r"WkbLimits::default\(\)")
DICHIARAZIONE_FN = re.compile(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)")

# Conteggio atteso per categoria. `produzione` puo' solo scendere: e' il
# residuo del difetto che S5 ha corretto.
# `test` passa da 47 a 50 con il lotto S11: `driver-gpkg` ne aggiunge tre.
# Uno e' l'helper `forma`, che tiene la quota predefinita in un posto solo
# invece che su ogni riga; due sono le sonde dei budget, che dai campi di
# `WkbLimits` derivano il payload oltre il tetto -- provano i limiti, quindi
# e' li' che il default e' l'oggetto della prova e non una scorciatoia.
ATTESI = {"test": 50, "attrezzaggio": 4, "produzione": 2}

# Occorrenze di produzione **legittime**: il default e' la scelta giusta, non
# un residuo. Chiave: `percorso::funzione`; valore: `(quante, perche')`.
#
# La funzione e' il contesto strutturale, non il numero di riga: spostare il
# codice non tocca la chiave, aggiungere un'occorrenza si'.
LEGITTIME: dict[str, tuple[int, str]] = {
    "crates/driver-gpkg/src/lib.rs::__fuzz_gpkg_geometry": (
        1,
        "entry point `#[doc(hidden)]` per libFuzzer. L'input del fuzzer e' "
        "gia' bounded a 1 MiB dall'harness, quindi il tetto di 64 MiB non "
        "governa nulla; e non ci sono opzioni da cui prendere una quota, "
        "perche' il target non apre un dataset. **Da mettere dietro la "
        "feature `fuzzing` in S12**: `doc(hidden)` lo toglie dalla "
        "documentazione, non dalla superficie pubblica",
    ),
    "crates/driver-shp/src/lib.rs::__fuzz_wkb_roundtrip": (
        1,
        "stessa natura del precedente, stessa azione in S12",
    ),
}

# Non esiste piu' una categoria "residuo dichiarato". S5 ne aveva lasciata una,
# `collect_read_violations`, sul percorso comune di lettura che ogni driver
# attraversa: la firma non riceveva le opzioni, quindi un
# `--max-wkb-cell-bytes` piu' stretto del default era applicato in inferenza e
# nella materializzazione ma non nella validazione del batch. S5.1 l'ha chiusa
# passando `&WkbLimits` dal `PipelineContext`.
#
# Da allora una occorrenza di produzione ha due sole uscite: sta in LEGITTIME
# con la ragione scritta, oppure il codice cambia. Dichiararla e rinviarla non
# e' piu' un esito accettato — era il meccanismo che teneva aperto il difetto.

ATTREZZAGGIO = ("plenora-bench", "plenora-fuzz", "fuzz")

FUORI_DA_UNA_FUNZIONE = "<modulo>"


def sorgenti(radice: Path) -> list[Path]:
    trovati: list[Path] = []
    for sotto in (radice / "crates", radice / "fuzz"):
        if not sotto.is_dir():
            continue
        for sorgente in sorted(sotto.rglob("*.rs")):
            if "target" in sorgente.relative_to(sotto).parts:
                continue
            trovati.append(sorgente)
    return trovati


def spoglia(sorgente: str) -> str:
    """Rimuove commenti e stringhe, sostituendoli con spazi.

    Un commento che spiega **perche'** un tipo e' stato rimosso ne nomina il
    nome, ed e' esattamente cio' che questo file fa in ogni sua riga. Senza lo
    spoglio, un gate del genere vieterebbe di documentare la propria ragione.
    """
    fuori: list[str] = []
    i = 0
    n = len(sorgente)
    while i < n:
        c = sorgente[i]
        due = sorgente[i : i + 2]
        if due == "//":
            j = sorgente.find("\n", i)
            j = n if j == -1 else j
            fuori.append(" " * (j - i))
            i = j
        elif due == "/*":
            profondita = 1
            j = i + 2
            while j < n and profondita:
                if sorgente[j : j + 2] == "/*":
                    profondita += 1
                    j += 2
                elif sorgente[j : j + 2] == "*/":
                    profondita -= 1
                    j += 2
                else:
                    j += 1
            fuori.append("".join(" " if ch != "\n" else "\n" for ch in sorgente[i:j]))
            i = j
        elif c == "r" and sorgente[i + 1 : i + 2] in ('"', "#"):
            m = re.match(r'r(#*)"', sorgente[i:])
            if not m:
                fuori.append(c)
                i += 1
                continue
            chiusura = '"' + m.group(1)
            j = sorgente.find(chiusura, i + m.end())
            j = n if j == -1 else j + len(chiusura)
            fuori.append("".join(" " if ch != "\n" else "\n" for ch in sorgente[i:j]))
            i = j
        elif c == '"':
            j = i + 1
            while j < n:
                if sorgente[j] == "\\":
                    j += 2
                    continue
                if sorgente[j] == '"':
                    j += 1
                    break
                j += 1
            fuori.append("".join(" " if ch != "\n" else "\n" for ch in sorgente[i:j]))
            i = j
        else:
            fuori.append(c)
            i += 1
    return "".join(fuori)


def _corpo(testo: str, dopo_il_nome: int) -> tuple[int, int] | None:
    """Estremi del corpo della funzione che comincia a `dopo_il_nome`.

    Salta generics e argomenti bilanciando le tonde, poi prende la prima
    graffa. Un `;` a tonde chiuse significa dichiarazione senza corpo — un
    metodo di trait — e non apre nessun intervallo.
    """
    n = len(testo)
    i = dopo_il_nome
    tonde = 0
    while i < n:
        c = testo[i]
        if c == "(":
            tonde += 1
        elif c == ")":
            tonde -= 1
        elif tonde == 0 and c == ";":
            return None
        elif tonde == 0 and c == "{":
            break
        i += 1
    if i >= n:
        return None
    profondita = 0
    j = i
    while j < n:
        if testo[j] == "{":
            profondita += 1
        elif testo[j] == "}":
            profondita -= 1
            if profondita == 0:
                return (i, j)
        j += 1
    return None


def intervalli_di_funzione(testo: str) -> list[tuple[int, int, str]]:
    """`(inizio, fine, nome)` per ogni funzione con un corpo."""
    intervalli: list[tuple[int, int, str]] = []
    for m in DICHIARAZIONE_FN.finditer(testo):
        estremi = _corpo(testo, m.end())
        if estremi is not None:
            intervalli.append((estremi[0], estremi[1], m.group(1)))
    return intervalli


def funzione_che_racchiude(
    intervalli: list[tuple[int, int, str]], posizione: int
) -> str:
    """Il nome della funzione piu' interna che contiene `posizione`.

    La piu' interna e' quella che comincia piu' tardi fra quelle che la
    contengono: le funzioni annidate sono rare in Rust, ma esistono, e
    attribuire l'occorrenza a quella esterna renderebbe la chiave meno stabile
    proprio dove serve.
    """
    candidato = FUORI_DA_UNA_FUNZIONE
    inizio_migliore = -1
    for inizio, fine, nome in intervalli:
        if inizio <= posizione <= fine and inizio > inizio_migliore:
            candidato = nome
            inizio_migliore = inizio
    return candidato


def righe_di_test(testo: str) -> set[int]:
    """Numeri di riga (1-based) dentro un modulo `#[cfg(test)]`."""
    dentro: set[int] = set()
    i = 0
    while True:
        j = testo.find("#[cfg(test)]", i)
        if j == -1:
            break
        apertura = testo.find("{", j)
        if apertura == -1:
            break
        if "mod " not in testo[j:apertura]:
            i = j + 1
            continue
        profondita = 0
        k = apertura
        while k < len(testo):
            if testo[k] == "{":
                profondita += 1
            elif testo[k] == "}":
                profondita -= 1
                if profondita == 0:
                    break
            k += 1
        prima = testo.count("\n", 0, j) + 1
        ultima = testo.count("\n", 0, k) + 1
        dentro.update(range(prima, ultima + 1))
        i = k
    return dentro


def verifica(radice: Path) -> tuple[list[str], dict[str, int]]:
    """Ritorna `(violazioni, conteggi per categoria)`."""
    conteggi = {nome: 0 for nome in ATTESI}
    produzione: dict[str, int] = {}

    for sorgente in sorgenti(radice):
        testo = spoglia(sorgente.read_text(encoding="utf-8"))
        if not OCCORRENZA.search(testo):
            continue
        percorso = sorgente.relative_to(radice).as_posix()
        parti = sorgente.relative_to(radice).parts
        crate = parti[1] if percorso.startswith("crates/") else parti[0]
        in_test = righe_di_test(testo)
        intervalli = intervalli_di_funzione(testo)
        for trovata in OCCORRENZA.finditer(testo):
            riga = testo.count("\n", 0, trovata.start()) + 1
            if crate in ATTREZZAGGIO:
                conteggi["attrezzaggio"] += 1
            elif riga in in_test:
                conteggi["test"] += 1
            else:
                conteggi["produzione"] += 1
                chiave = (
                    f"{percorso}::"
                    f"{funzione_che_racchiude(intervalli, trovata.start())}"
                )
                produzione[chiave] = produzione.get(chiave, 0) + 1

    errori: list[str] = []
    for nome, atteso in sorted(ATTESI.items()):
        if conteggi[nome] != atteso:
            errori.append(
                f"{nome}: {conteggi[nome]} occorrenze, attese {atteso}. "
                "Il conteggio va aggiornato nello stesso commit che lo cambia, "
                "cosi' una nuova occorrenza non passa senza essere classificata."
            )

    for chiave in sorted(produzione):
        trovate = produzione[chiave]
        censite = LEGITTIME.get(chiave)
        if censite is None:
            errori.append(
                f"{chiave}: `WkbLimits::default()` su un percorso di produzione "
                "non censito. S5 ha portato le quote configurate fino "
                "all'inferenza e S5.1 fino alla validazione del batch: un "
                "default qui le riporterebbe indietro. O il codice prende la "
                "quota dal `PipelineContext`, o l'occorrenza entra in LEGITTIME "
                "con la ragione per cui il default e' la scelta giusta."
            )
        elif trovate != censite[0]:
            errori.append(
                f"{chiave}: {trovate} occorrenze, {censite[0]} censite. Un "
                "default in piu' dentro una funzione gia' censita non e' "
                "coperto dalla ragione scritta per quello che c'era: o e' la "
                "stessa ragione, e il conteggio sale nello stesso commit, "
                "oppure e' un residuo."
            )

    for chiave in sorted(LEGITTIME):
        if chiave not in produzione:
            errori.append(
                f"{chiave}: censita in LEGITTIME ma non piu' presente nel "
                "codice. Una voce che sopravvive alla propria occorrenza "
                "tiene in vita una ragione che nessuno rilegge: va tolta nello "
                "stesso commit che toglie il codice."
            )

    return errori, conteggi


def main() -> int:
    errori, conteggi = verifica(ROOT)
    if errori:
        for messaggio in errori:
            print(messaggio, file=sys.stderr)
        return 1

    print(
        "WkbLimits::default() censiti: "
        f"{len(LEGITTIME)} legittimi in produzione (per funzione, non per riga), "
        "zero residui, "
        f"{conteggi['test']} nei test, "
        f"{conteggi['attrezzaggio']} nell'attrezzaggio"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
