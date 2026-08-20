"""Il quartetto `(category, phase, code, retry)` non cambia dentro S9.

# Perche' esiste

S9 sostituisce i **messaggi**, non la classificazione. La decisione 2 di S9 ha
dichiarato che il testo di `message` cambia e che la chiave di compatibilita' e'
`(category, phase, code, retry)`: cambiare il quartetto e' quindi una rottura,
non un dettaglio del refactor.

Un modo di cambiarlo in silenzio esiste, ed e' stato usato per errore nella
tranche 2:

    PlenoraIoError::new(ErrorCategory::Schema, ErrorPhase::Validate, ...)
    // code = Generic

diventato

    PlenoraIoError::schema_redatto(...)
    // code = Schema

Categoria, fase, effetto e retry restano identici -- il diff non mostra nessuna
riga di asse cambiata -- e **solo il codice si sposta**, perche' i due
costruttori lo impostano diversamente. La CIA della tranche dichiarava il
contrario, in buona fede.

# Che cosa verifica

Che nell'intervallo di S9 nessun `PlenoraIoError::new(` sia stato sostituito da
un costruttore di **famiglia** (`schema_redatto`, `limite_redatto`, ...), che
impone un codice proprio. La sostituzione lecita e' `redatto(...)`, che il
codice lo prende come argomento e permette di conservarlo.

Non e' una verifica sul presente ma su un **intervallo di revisioni**: e' l'unica
forma in cui la proprieta' «non e' cambiato» si puo' controllare.
"""

from __future__ import annotations

import argparse
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent

# I costruttori di famiglia impongono un codice; `redatto` lo riceve.
FAMIGLIA = (
    "schema_redatto",
    "limite_redatto",
    "contratto_redatto",
    "non_supportato_redatto",
    "crs_redatto",
    "wkb_redatto",
    "formato_redatto",
    "capability_redatta",
    "destinazione_esistente",
    "crs_non_risolto_redatto",
)

TOLTO_NEW = re.compile(r"^-[^-].*PlenoraIoError::new\s*\(")
MESSO_FAMIGLIA = re.compile(
    r"^\+[^+].*PlenoraIoError::(?:" + "|".join(FAMIGLIA) + r")\s*\("
)
INTESTAZIONE = re.compile(r"^\+\+\+ b/(.+)$")
HUNK = re.compile(r"^@@ ")


def verifica(base: str, head: str) -> list[str]:
    diff = subprocess.run(
        ["git", "diff", f"{base}..{head}", "-U3", "--", "crates"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        encoding="utf-8",
        check=True,
    ).stdout

    errori: list[str] = []
    percorso = ""
    tolto_new = False
    for riga in diff.splitlines():
        m = INTESTAZIONE.match(riga)
        if m:
            percorso = m.group(1)
            tolto_new = False
            continue
        if HUNK.match(riga):
            tolto_new = False
            continue
        if TOLTO_NEW.match(riga):
            tolto_new = True
            continue
        # Una riga di **contesto** chiude l'accoppiamento; altre rimozioni no.
        #
        # Con tre righe di contesto, un `new` rimosso e un costruttore di
        # famiglia aggiunto finiscono nello stesso hunk anche quando sono siti
        # diversi: senza questo, tre falsi positivi su tre. Ma chiudere anche
        # sulle rimozioni sarebbe troppo stretto e perderebbe il caso vero --
        # fra il `new(` e il costruttore nuovo stanno le quattro righe di assi,
        # anch'esse rimosse. Provato in entrambi i versi sul difetto reale.
        if riga.startswith(" "):
            tolto_new = False
            continue
        if tolto_new and MESSO_FAMIGLIA.match(riga):
            errori.append(
                f"{percorso}: `PlenoraIoError::new(...)` sostituito da un costruttore "
                f"di famiglia. Quello impone un codice proprio; `new` metteva "
                f"`Generic`. Usa `redatto(IoErrorCode::Generic, ...)` per conservare "
                f"il quartetto, oppure fai ratificare il cambio.\n    {riga.strip()}"
            )
            tolto_new = False
    return errori


def main() -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument(
        "--base",
        default="2b38c4e",
        help="prima revisione di S9 (tranche 1); il default copre tutta la migrazione",
    )
    argomenti.add_argument("--head", default="HEAD")
    opzioni = argomenti.parse_args()

    errori = verifica(opzioni.base, opzioni.head)
    for messaggio in errori:
        print(messaggio, file=sys.stderr)
    if errori:
        return 1
    print(
        f"quartetto invariato fra {opzioni.base} e {opzioni.head}: "
        "nessun `new` sostituito da un costruttore di famiglia"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
