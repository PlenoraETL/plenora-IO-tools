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
DA_MIGRARE: dict[str, int] = {
    "crates/driver-dxf/src/lib.rs::build_batch_cancellable": 1,
    "crates/driver-dxf/src/lib.rs::create": 3,
    "crates/driver-dxf/src/lib.rs::declare_input_total": 1,
    "crates/driver-dxf/src/lib.rs::definition_for_write": 3,
    "crates/driver-dxf/src/lib.rs::dxf_contract": 1,
    "crates/driver-dxf/src/lib.rs::err": 1,
    "crates/driver-dxf/src/lib.rs::finish": 1,
    "crates/driver-dxf/src/lib.rs::open": 1,
    "crates/driver-dxf/src/lib.rs::push": 3,
    "crates/driver-dxf/src/lib.rs::resolve_dxf_crs": 2,
    "crates/driver-dxf/src/lib.rs::write": 2,
    "crates/driver-dxf/src/lib.rs::write_file_value": 1,
    "crates/driver-filegdb/src/lib.rs::create": 6,
    "crates/driver-filegdb/src/lib.rs::err": 1,
    "crates/driver-filegdb/src/lib.rs::finish": 1,
    "crates/driver-filegdb/src/lib.rs::from": 1,
    "crates/driver-filegdb/src/lib.rs::geometry_capability": 1,
    "crates/driver-filegdb/src/lib.rs::layer_spatial_ref": 4,
    "crates/driver-filegdb/src/lib.rs::native_i32": 2,
    "crates/driver-filegdb/src/lib.rs::ogr_to_arrow": 1,
    "crates/driver-filegdb/src/lib.rs::open": 2,
    "crates/driver-filegdb/src/lib.rs::resolve_layer_crs": 3,
    "crates/driver-shp/src/lib.rs::create": 4,
    "crates/driver-shp/src/lib.rs::declare_input_total": 1,
    "crates/driver-shp/src/lib.rs::err": 1,
    "crates/driver-shp/src/lib.rs::finish": 2,
    "crates/driver-shp/src/lib.rs::from_options": 5,
    "crates/driver-shp/src/lib.rs::publish_mode": 3,
    "crates/driver-shp/src/lib.rs::resolve_crs": 2,
    "crates/driver-shp/src/lib.rs::resolved_crs_id": 1,
    "crates/driver-shp/src/lib.rs::shapefile_source_path": 1,
    "crates/driver-shp/src/lib.rs::write": 2,
    "crates/plenora-io-cli/src/main.rs::cmd_convert": 5,
    "crates/plenora-io-cli/src/main.rs::local_err_doc": 1,
}

# `plenora-bench` e `plenora-fuzz` non sono codice spedito: esclusi qui per la
# stessa ragione per cui sono esclusi dalla copertura, e l'esclusione e'
# dichiarata invece che silenziosa.
ATTREZZAGGIO = ("plenora-bench", "plenora-fuzz")

FUORI_DA_UNA_FUNZIONE = "<modulo>"


def sorgenti(radice: Path) -> list[Path]:
    crates = radice / "crates"
    if not crates.is_dir():
        return []
    return [
        sorgente
        for sorgente in sorted(crates.rglob("*.rs"))
        if "target" not in sorgente.relative_to(crates).parts
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

    for sorgente in sorgenti(radice):
        parti = sorgente.relative_to(radice).parts
        crate = parti[1]
        if crate in ATTREZZAGGIO:
            continue
        testo = spoglia(sorgente.read_text(encoding="utf-8"))
        if not CHIAMATA.search(testo):
            continue
        percorso = sorgente.relative_to(radice).as_posix()
        intervalli = intervalli_di_funzione(testo)
        solo_test = righe_di_test(testo)
        for m in CHIAMATA.finditer(testo):
            if testo.count("\n", 0, m.start()) + 1 in solo_test:
                continue
            chiave = f"{percorso}::{funzione_che_racchiude(intervalli, m.start())}"
            trovate[chiave] = trovate.get(chiave, 0) + 1
            per_crate[crate] = per_crate.get(crate, 0) + 1

    errori: list[str] = []

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
