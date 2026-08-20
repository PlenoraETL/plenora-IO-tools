"""Il quartetto atteso di ogni sito d'errore, verificato sul codice presente.

# Perche' non basta il diff

La prima versione di questo gate confrontava due revisioni e cercava un
`PlenoraIoError::new(` sostituito da un costruttore di famiglia. Trovava il
difetto reale, ma aveva un vizio fatale: **dipendeva dall'esistenza dell'API
legacy**. Rimossi i costruttori storici -- che e' l'ultimo passo di S9 -- non
ci sarebbero piu' `new(` da trovare nel diff, e il gate sarebbe diventato verde
senza piu' verificare niente.

Un gate che smette di controllare senza dirlo e' il difetto che questa intera
sequenza di checkpoint ha continuato a incontrare, in cinque forme diverse.

# Che cosa verifica

Per ogni funzione che costruisce errori, il **multiinsieme dei quartetti** che
costruisce. L'identita' del sito e' `percorso::funzione`, non `path:riga`: e' la
lezione di INFRA-1, e sopravvive a spostamenti e riformattazioni.

Il quartetto si legge dal costruttore, perche' e' il costruttore a fissarlo:

* i costruttori di **famiglia** lo impongono per intero;
* `redatto` riceve il codice come primo argomento e gli assi come successivi;
* `new` -- finche' esiste -- impone `code = Generic` e riceve gli assi.

Cambiare costruttore a un sito cambia lo snapshot, e il gate lo dice. E' cosi'
che si sarebbe visto il difetto della tranche 2: `new(Schema, Validate, ...)`
diventato `schema_redatto` sposta il codice da `Generic` a `Schema` **senza
cambiare una sola riga di assi nel diff**.

# Che cosa NON verifica

Che il quartetto sia quello *giusto* per quel sito. Verifica che sia quello
**dichiarato**: cambiarlo richiede di aggiornare lo snapshot, cioe' di
scriverlo. E' la stessa proprieta' del registro dei fallback -- il numero puo'
muoversi, ma non da solo.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import re
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

from check_errori_redatti import (  # noqa: E402
    ATTREZZAGGIO,
    funzione_che_racchiude,
    intervalli_di_funzione,
    righe_di_test,
    sorgenti,
    spoglia,
)

ROOT = pathlib.Path(__file__).resolve().parent.parent
SNAPSHOT = ROOT / "docs" / "contracts" / "quartetto-siti.json"

# costruttore -> quartetto completo, quando il costruttore lo impone
FAMIGLIA = {
    "wkb_redatto": "DataMapping/Validate/Wkb/Never",
    "limite_redatto": "ResourceLimit/Validate/LimitExceeded/Never",
    "contratto_redatto": "InvalidPlan/Validate/Contract/Never",
    "non_supportato_redatto": "Unsupported/Validate/Unsupported/Never",
    "schema_redatto": "Schema/Validate/Schema/Never",
    "crs_redatto": "Crs/Validate/Crs/Never",
    "formato_redatto": "DataMapping/Read/Format/Never",
    "capability_redatta": "Unsupported/Validate/Capability/Never",
    "destinazione_esistente": "Conflict/Commit/OutputExists/Never",
    "crs_non_risolto_redatto": "Crs/Validate/CrsUnresolved/Never",
    # legacy, finche' esistono
    "Contract": "InvalidPlan/Validate/Contract/Never",
    "Unsupported": "Unsupported/Validate/Unsupported/Never",
    "Schema": "Schema/Validate/Schema/Never",
    "Crs": "Crs/Validate/Crs/Never",
    "Wkb": "DataMapping/Read/Wkb/Never",
    "LimitExceeded": "ResourceLimit/Validate/LimitExceeded/Never",
    "OutputExists": "Conflict/Commit/OutputExists/Never",
    "format": "DataMapping/Read/Format/Never",
    "capability": "Unsupported/Validate/Capability/Never",
    "crs_unresolved": "Crs/Validate/CrsUnresolved/Never",
}

# `redatto` e `new` portano gli assi: il codice si legge dal primo argomento di
# `redatto`, e per `new` e' `Generic` per costruzione.
CHIAMATA = re.compile(
    r"PlenoraIoError::(" + "|".join(sorted(FAMIGLIA, key=len, reverse=True)) + r"|redatto|new)\s*\("
)
CODICE = re.compile(r"\bIoErrorCode::(\w+)")


def quartetti_del_file(percorso: pathlib.Path) -> dict[str, list[str]]:
    grezzo = percorso.read_text(encoding="utf-8")
    testo = spoglia(grezzo)
    intervalli = intervalli_di_funzione(testo)
    solo_test = righe_di_test(testo)

    per_funzione: dict[str, list[str]] = {}
    for m in CHIAMATA.finditer(testo):
        riga = testo.count("\n", 0, m.start()) + 1
        if riga in solo_test:
            continue
        nome = m.group(1)
        if nome in FAMIGLIA:
            quartetto = FAMIGLIA[nome]
        elif nome == "new":
            quartetto = "esplicito/esplicito/Generic/esplicito"
        else:  # redatto
            coda = testo[m.end() : m.end() + 200]
            codice = CODICE.search(coda)
            quartetto = f"esplicito/esplicito/{codice.group(1) if codice else '?'}/esplicito"
        funzione = funzione_che_racchiude(intervalli, m.start())
        per_funzione.setdefault(funzione, []).append(quartetto)
    return {k: sorted(v) for k, v in per_funzione.items()}


def istantanea() -> dict[str, dict[str, list[str]]]:
    fuori: dict[str, dict[str, list[str]]] = {}
    for percorso in sorgenti(ROOT):
        if percorso.relative_to(ROOT).parts[1] in ATTREZZAGGIO:
            continue
        quartetti = quartetti_del_file(percorso)
        if quartetti:
            fuori[percorso.relative_to(ROOT).as_posix()] = quartetti
    return fuori


def confronta(atteso: dict, trovato: dict) -> list[str]:
    errori: list[str] = []
    for percorso in sorted(set(atteso) | set(trovato)):
        a = atteso.get(percorso, {})
        t = trovato.get(percorso, {})
        for funzione in sorted(set(a) | set(t)):
            qa, qt = a.get(funzione), t.get(funzione)
            if qa == qt:
                continue
            if qa is None:
                errori.append(f"{percorso}::{funzione}: sito nuovo, quartetti {qt}")
            elif qt is None:
                errori.append(f"{percorso}::{funzione}: sito sparito, attesi {qa}")
            else:
                errori.append(
                    f"{percorso}::{funzione}: quartetto cambiato\n"
                    f"    atteso:  {qa}\n"
                    f"    trovato: {qt}"
                )
    return errori


def main() -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument(
        "--aggiorna",
        action="store_true",
        help="riscrive lo snapshot; da usare solo insieme alla ragione del cambio",
    )
    opzioni = argomenti.parse_args()

    trovato = istantanea()
    if opzioni.aggiorna:
        SNAPSHOT.parent.mkdir(parents=True, exist_ok=True)
        SNAPSHOT.write_bytes(
            (json.dumps(trovato, indent=2, ensure_ascii=False, sort_keys=True) + "\n")
            .encode("utf-8")
        )
        siti = sum(len(v) for v in trovato.values())
        print(f"snapshot aggiornato: {len(trovato)} file, {siti} funzioni")
        return 0

    if not SNAPSHOT.exists():
        print(f"{SNAPSHOT}: snapshot assente. Genera con --aggiorna.", file=sys.stderr)
        return 2

    atteso = json.loads(SNAPSHOT.read_text(encoding="utf-8"))
    errori = confronta(atteso, trovato)
    for messaggio in errori:
        print(messaggio, file=sys.stderr)
    if errori:
        print(
            "\nIl quartetto (category, phase, code, retry) e' la chiave di "
            "compatibilita' ratificata da S9. Se il cambio e' voluto, aggiorna lo "
            "snapshot con --aggiorna e scrivi perche'.",
            file=sys.stderr,
        )
        return 1

    siti = sum(len(v) for v in trovato.values())
    print(f"quartetto verificato: {len(trovato)} file, {siti} funzioni")
    return 0


if __name__ == "__main__":
    sys.exit(main())
