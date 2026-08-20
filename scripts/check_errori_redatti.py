#!/usr/bin/env python3
"""Censimento dei costruttori d'errore **legacy**, per crate (S9, permanente).

S9 sostituisce il canale libero `message: String` con `PublicMessage`, deciso a
compile time. La migrazione e' staged: la via nuova vive accanto alla vecchia, e
i crate passano uno per volta.

Questo gate governa la transizione. Non conta quanti usi legacy restano nel
workspace — conta **dove** stanno, e impedisce che un crate gia' migrato torni
indietro.

## Perche' il conteggio globale non basta

Perche' e' gia' successo. In FZ-0.2 il registro dei fallback era rimasto fermo a
4 mentre due voci si annullavano, una in calo e una in aumento, e la stabilita'
sembrava una conferma. **Un contatore fermo non dice che niente si e' mosso.**

Un crate dichiarato migrato ha censimento **zero** e non puo' guadagnare
occorrenze: e' una proprieta' per crate, e nessun totale la esprime.

## Identita' per funzione, non per riga

La chiave e' `percorso::funzione`, come in `check_wkb_limits_defaults.py` dopo
INFRA-1. Una chiave per riga si accende sui movimenti — una `use` aggiunta
sopra, una riformattazione — e insegna a riallinearla senza guardare. Dopo la
terza volta il numero si aggiorna per far tornare il verde, e quel giorno passa
anche un'occorrenza vera.

## I tre modi di fallire

* una chiamata legacy in un crate **migrato**: la regressione che questo gate
  esiste per prendere;
* un conteggio diverso da quello censito in un crate **non ancora** migrato:
  una chiamata in piu' non e' coperta dalla ragione scritta per quelle che
  c'erano;
* una voce di censimento **senza codice**: una riga che sopravvive alla propria
  occorrenza tiene in vita una ragione che nessuno rilegge.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# I costruttori che accettano testo libero. `new` e' incluso: e' quello da cui
# passano tutti gli altri, e lasciarlo fuori renderebbe il gate aggirabile
# scrivendo `PlenoraIoError::new(..., format!(...))`.
LEGACY = (
    "PlenoraIoError::new",
    "PlenoraIoError::Contract",
    "PlenoraIoError::Unsupported",
    "PlenoraIoError::Schema",
    "PlenoraIoError::Crs",
    "PlenoraIoError::Wkb",
    "PlenoraIoError::LimitExceeded",
    "PlenoraIoError::OutputExists",
    "PlenoraIoError::format",
    "PlenoraIoError::capability",
    "PlenoraIoError::crs_unresolved",
)

CHIAMATA = re.compile("|".join(re.escape(nome) + r"\s*\(" for nome in LEGACY))
DICHIARAZIONE_FN = re.compile(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)")

# I crate **migrati**: censimento zero, e non possono guadagnare occorrenze.
MIGRATI = (
    "plenora-io-model",
    "plenora-io-core",
    "driver-common",
    "driver-geojson",
    "driver-csv",
    "driver-kml",
    "driver-gpkg",
    "driver-geoparquet",
    "driver-ipc",
    "driver-xls",
    "driver-filegdb",
    "driver-shp",
    "driver-dxf",
    "plenora-io-cli",
)

# I crate non ancora migrati, con il conteggio atteso per `percorso::funzione`.
#
# E' il **registro del debito**: 226 occorrenze in 122 funzioni alla chiusura
# della tranche 1. Ogni voce sparira' quando il suo crate sara' migrato, e il
# gate lo pretende — una voce che sopravvive al proprio codice e' una violazione
# quanto una chiamata non censita.
#
# L'elenco e' generato dal codice, non scritto a mano: un elenco copiato diverge
# alla prima chiamata aggiunta, e diverge in silenzio.
DA_MIGRARE: dict[str, int] = {}

# `plenora-bench` e `plenora-fuzz` non sono codice spedito: esclusi qui per la
# stessa ragione per cui sono esclusi dalla copertura, e l'esclusione e'
# dichiarata invece che silenziosa.
ATTREZZAGGIO = ("plenora-bench", "plenora-fuzz")

FUORI_DA_UNA_FUNZIONE = "<modulo>"


def sorgenti(radice: Path) -> list[Path]:
    """Tutto il codice Rust del repository.

    Non solo `crates/`: i target di fuzzing vivono in `fuzz/`, e fino alla
    chiusura di S9 non venivano guardati da questo gate. Un costruttore legacy
    in un harness di fuzzing e' codice che compila e che qualcuno rilegge.
    """
    trovate: list[Path] = []
    for radice_parziale in (radice / "crates", radice / "fuzz"):
        if not radice_parziale.is_dir():
            continue
        trovate.extend(
            sorgente
            for sorgente in sorted(radice_parziale.rglob("*.rs"))
            if "target" not in sorgente.relative_to(radice_parziale).parts
        )
    return trovate


INIZIO_BLOCCO = re.compile(r"^\s*(?:///|//!)\s*```(?P<attributi>.*)$")


def doctest_che_devono_compilare(sorgente: str) -> list[tuple[int, str]]:
    """`(riga d'inizio, codice)` dei blocchi doctest che **devono** compilare.

    I blocchi `compile_fail` sono esclusi, e non e' un'allowlist: un blocco
    marcato cosi' e' per definizione la prova che quel codice *non* compila.
    Contarlo come violazione significherebbe rossare proprio la prova che la
    via legacy non esiste piu'.

    Serve perche' `spoglia` cancella i commenti, e un doctest e' codice che
    vive dentro un commento: senza questa funzione il gate non lo vedrebbe.
    """
    blocchi: list[tuple[int, str]] = []
    righe = sorgente.splitlines()
    indice = 0
    while indice < len(righe):
        apertura = INIZIO_BLOCCO.match(righe[indice])
        if apertura is None:
            indice += 1
            continue
        attributi = apertura.group("attributi")
        prima_riga = indice + 2
        corpo: list[str] = []
        indice += 1
        while indice < len(righe) and INIZIO_BLOCCO.match(righe[indice]) is None:
            corpo.append(re.sub(r"^\s*(?:///|//!) ?", "", righe[indice]))
            indice += 1
        indice += 1
        if "compile_fail" not in attributi and "ignore" not in attributi:
            blocchi.append((prima_riga, "\n".join(corpo)))
    return blocchi


# Le definizioni che S9 ha rimosso. Il gate non verifica solo che nessuno le
# chiami: verifica che **non esistano**, che e' la proprieta' che rende vere le
# prove `compile_fail` — quelle passerebbero anche per un refuso, questa no.
DEFINIZIONI_RIMOSSE = (
    "new",
    "capability",
    "format",
    "crs_unresolved",
    "Contract",
    "Unsupported",
    "Schema",
    "Crs",
    "Wkb",
    "LimitExceeded",
    "OutputExists",
)
DEFINIZIONE = re.compile(
    r"pub\s+(?:const\s+)?fn\s+(" + "|".join(DEFINIZIONI_RIMOSSE) + r")\s*\("
)
SORGENTE_DEI_COSTRUTTORI = "crates/plenora-io-model/src/error.rs"


def definizioni_legacy(radice: Path) -> list[str]:
    """I costruttori legacy che fossero tornati a esistere."""
    sorgente = radice / SORGENTE_DEI_COSTRUTTORI
    if not sorgente.is_file():
        return [f"{SORGENTE_DEI_COSTRUTTORI}: sparito; il gate non ha piu' presa"]
    testo = spoglia(sorgente.read_text(encoding="utf-8"))
    return [
        f"{SORGENTE_DEI_COSTRUTTORI}: `pub fn {m.group(1)}` e' tornata a esistere. "
        "S9 l'ha rimossa: la garanzia non e' un gate ne' una convenzione, e' "
        "l'assenza della funzione."
        for m in DEFINIZIONE.finditer(testo)
    ]


def spoglia(sorgente: str) -> str:
    """Rimuove commenti e stringhe, sostituendoli con spazi.

    Questo file e la documentazione di S9 nominano i costruttori legacy in ogni
    riga: senza lo spoglio, un gate del genere conterebbe la propria
    motivazione.
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
    intervalli: list[tuple[int, int, str]] = []
    for m in DICHIARAZIONE_FN.finditer(testo):
        estremi = _corpo(testo, m.end())
        if estremi is not None:
            intervalli.append((estremi[0], estremi[1], m.group(1)))
    return intervalli


def funzione_che_racchiude(
    intervalli: list[tuple[int, int, str]], posizione: int
) -> str:
    candidato = FUORI_DA_UNA_FUNZIONE
    inizio_migliore = -1
    for inizio, fine, nome in intervalli:
        if inizio <= posizione <= fine and inizio > inizio_migliore:
            candidato = nome
            inizio_migliore = inizio
    return candidato


def e_test_per_posizione(percorso: Path, radice: Path) -> bool:
    """Un file sotto `crates/<crate>/{tests,benches,examples}/`.

    E' codice di test **per posizione**: non ha un modulo `#[cfg(test)]`,
    perche' l'intero file lo e'. `righe_di_test` cerca quel modulo e quindi non
    lo vede — una lacuna che si nota solo quando qualcuno scrive il primo test
    d'integrazione, ed e' successo con `tests/ostili.rs`.

    Non e' un'esclusione dal censimento: li' i test contano come la produzione.
    Serve ai gate che il codice di test lo escludono **gia'**, perche' lo
    escludano per una regola sola invece che per due meta'.
    """
    parti = percorso.relative_to(radice).parts
    return len(parti) > 2 and parti[0] == "crates" and parti[2] in (
        "tests",
        "benches",
        "examples",
    )


def righe_di_test(testo: str) -> set[int]:
    """Numeri di riga (1-based) dentro un modulo `#[cfg(test)]`.

    Il perimetro e' il codice **di produzione**, come per gli altri gate di
    questo repository. I test dei costruttori legacy devono continuare a
    costruirli finche' quei costruttori esistono: vietarlo li' significherebbe
    togliere la copertura alla via che si sta smantellando, proprio mentre la si
    smantella.
    """
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
        dentro.update(range(testo.count("\n", 0, j) + 1, testo.count("\n", 0, k) + 2))
        i = k
    return dentro


def verifica(radice: Path) -> tuple[list[str], dict[str, int]]:
    """`(violazioni, occorrenze per `percorso::funzione`)`."""
    trovate: dict[str, int] = {}
    per_crate: dict[str, int] = {}
    violazioni_doctest: list[str] = []

    for sorgente in sorgenti(radice):
        parti = sorgente.relative_to(radice).parts
        crate = parti[1] if parti[0] == "crates" else "fuzz"
        grezzo = sorgente.read_text(encoding="utf-8")

        # I doctest: codice dentro un commento, che `spoglia` cancella.
        percorso_doc = sorgente.relative_to(radice).as_posix()
        for riga, codice in doctest_che_devono_compilare(grezzo):
            for m in CHIAMATA.finditer(spoglia(codice)):
                nome = m.group(0).rstrip("( \t")
                violazioni_doctest.append(
                    f"{percorso_doc}: doctest alla riga {riga} usa `{nome}`. "
                    "Un doctest che deve compilare e' codice come un altro; se "
                    "serve mostrare la via vecchia, il blocco va marcato "
                    "`compile_fail`, che e' la prova opposta."
                )

        testo = spoglia(grezzo)
        if not CHIAMATA.search(testo):
            continue
        percorso = sorgente.relative_to(radice).as_posix()
        intervalli = intervalli_di_funzione(testo)
        # I test contano quanto la produzione. Fino alla chiusura di S9 erano
        # esclusi, perche' la migrazione procedeva un crate per volta e i test
        # erano l'ultima cosa a muoversi; ora che la via legacy non esiste,
        # un'occorrenza in un test e' codice che non compilerebbe.
        for m in CHIAMATA.finditer(testo):
            chiave = f"{percorso}::{funzione_che_racchiude(intervalli, m.start())}"
            trovate[chiave] = trovate.get(chiave, 0) + 1
            per_crate[crate] = per_crate.get(crate, 0) + 1

    errori: list[str] = definizioni_legacy(radice) + violazioni_doctest

    # 1. Un crate migrato non puo' avere nemmeno un'occorrenza.
    for crate in MIGRATI:
        quante = per_crate.get(crate, 0)
        if quante:
            dove = sorted(k for k in trovate if k.startswith(f"crates/{crate}/"))
            errori.append(
                f"{crate}: dichiarato migrato ma ha {quante} chiamate legacy. "
                "Un crate migrato non torna indietro: usa i costruttori redatti "
                f"({', '.join(dove)})"
            )

    # 2. I crate non ancora migrati: conteggio esatto per funzione.
    for chiave in sorted(trovate):
        crate = chiave.split("/")[1]
        if crate in MIGRATI:
            continue
        atteso = DA_MIGRARE.get(chiave)
        if atteso is None:
            errori.append(
                f"{chiave}: chiamata legacy non censita. Va migrata ai "
                "costruttori redatti, oppure censita in DA_MIGRARE nello stesso "
                "commit che la introduce."
            )
        elif atteso != trovate[chiave]:
            errori.append(
                f"{chiave}: {trovate[chiave]} chiamate, {atteso} censite. Una "
                "chiamata in piu' non e' coperta dalla ragione scritta per "
                "quelle che c'erano."
            )

    # 3. Nessuna voce fantasma.
    for chiave in sorted(DA_MIGRARE):
        if chiave not in trovate:
            errori.append(
                f"{chiave}: censita ma non piu' presente nel codice. Una voce "
                "che sopravvive alla propria occorrenza tiene in vita una "
                "ragione che nessuno rilegge: va tolta nello stesso commit che "
                "toglie il codice."
            )

    return errori, per_crate


def main() -> int:
    errori, per_crate = verifica(ROOT)
    if errori:
        for messaggio in errori:
            print(messaggio, file=sys.stderr)
        return 1
    residui = sum(per_crate.values())
    migrati = ", ".join(MIGRATI)
    print(
        f"costruttori d'errore legacy: {residui} residui in "
        f"{len(per_crate)} crate; migrati e a zero: {migrati}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
