#!/usr/bin/env python3
"""Confine workspace-internal dell'`InputPermit` (INV-13, Lotto 0 / S4.b.3).

INV-13 dichiara che il permit non e' separabile dal proprio bundle. Rust non
sa esprimere quella garanzia fra crate distinti: `pub(crate)` non basta —
`plenora-io-core` e' un crate diverso da `plenora-io-model` — e un
`pub(workspace)` non esiste. Un'API che il core deve poter chiamare e'
necessariamente `pub`, quindi visibile a chiunque aggiunga il modello fra le
proprie dipendenze.

La formulazione onesta e' percio' piu' stretta di quella originale: il permit
e' **non costruibile, non clonabile e legato al context**, e queste tre sono
garanzie del linguaggio; e' invece **separabile per move**, e quella
separazione e' confinata al workspace da tre fatti verificabili, che questo
gate controlla:

1. entrambi i crate sono `publish = false`, quindi l'API non raggiunge un
   consumer esterno per la via del registry;
2. esiste **una sola** via di decomposizione, marcata `#[doc(hidden)]`;
3. nessun altro crate del workspace la usa.

Non e' una prova di impossibilita' — nessun grep lo e'. E' cio' che rende il
confine verificabile invece che dichiarato, ed e' esattamente la differenza
che INV-13 nella vecchia formulazione nascondeva.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# I due crate ai quali il confine e' riservato.
CRATE_INTERNI = ("plenora-io-model", "plenora-io-core")

# L'unica via di decomposizione rimasta, piu' il tipo che protegge.
DECOMPOSIZIONE = re.compile(r"\.into_components\(\)|\.into_budget\(\)")
PERMIT = re.compile(r"\bInputPermit\b")

# Firme che non devono ricomparire.
ESTRATTORE_RIMOSSO = re.compile(r"pub (?:const )?fn take_input_permit")
DOC_HIDDEN = re.compile(
    r"#\[doc\(hidden\)\]\s*(?:#\[must_use\]\s*)?pub fn (into_components|into_budget)"
)


def errore(messaggi: list[str], testo: str) -> None:
    messaggi.append(testo)


def main() -> int:
    errori: list[str] = []

    # 1. Premessa del confine: nessuno dei due crate e' pubblicabile.
    for crate in CRATE_INTERNI:
        manifest = ROOT / "crates" / crate / "Cargo.toml"
        if not manifest.is_file():
            errore(errori, f"{crate}: Cargo.toml assente")
            continue
        if "publish = false" not in manifest.read_text(encoding="utf-8"):
            errore(
                errori,
                f"{crate}: manca `publish = false`. Il confine workspace-internal "
                "del permit poggia sul fatto che questi crate non raggiungano un "
                "consumer esterno; senza, la garanzia di INV-13 va riformulata di "
                "nuovo, non solo il gate.",
            )

    budget = ROOT / "crates" / "plenora-io-model" / "src" / "budget.rs"
    testo_budget = budget.read_text(encoding="utf-8")

    # 2. Una sola via di decomposizione, e marcata.
    if ESTRATTORE_RIMOSSO.search(testo_budget):
        errore(
            errori,
            "plenora-io-model: e' ricomparso un `take_input_permit` pubblico. La "
            "decomposizione deve restare un solo punto: due vie per la stessa "
            "separazione sono cio' che S4.b.3 ha rimosso.",
        )
    marcati = set(DOC_HIDDEN.findall(testo_budget))
    for atteso in ("into_components", "into_budget"):
        if atteso not in marcati:
            errore(
                errori,
                f"plenora-io-model: `{atteso}` non e' marcato `#[doc(hidden)]`. "
                "La marcatura e' meta' del confine: senza, l'API compare nella "
                "documentazione come se fosse d'uso generale.",
            )

    # 3. Nessun altro crate attraversa il confine.
    for sorgente in sorted((ROOT / "crates").glob("*/src/**/*.rs")):
        crate = sorgente.relative_to(ROOT / "crates").parts[0]
        if crate in CRATE_INTERNI:
            continue
        contenuto = sorgente.read_text(encoding="utf-8")
        percorso = sorgente.relative_to(ROOT).as_posix()
        if DECOMPOSIZIONE.search(contenuto):
            errore(
                errori,
                f"{percorso}: usa l'API di decomposizione delle parti, riservata a "
                f"{' e '.join(CRATE_INTERNI)}. Un driver riceve le opzioni gia' "
                "costruite: non deve mai scomporre le parti da se'.",
            )
        if PERMIT.search(contenuto):
            errore(
                errori,
                f"{percorso}: nomina `InputPermit`. Il permit non deve uscire dal "
                "confine model/core nemmeno come tipo.",
            )

    # 4. Lato core, l'estrattore non e' pubblico.
    driver = ROOT / "crates" / "plenora-io-core" / "src" / "driver.rs"
    testo_driver = driver.read_text(encoding="utf-8")
    if not re.search(r"pub\(crate\) const fn take_input_permit", testo_driver):
        errore(
            errori,
            "plenora-io-core: `ReadOptions::take_input_permit` deve essere "
            "`pub(crate)`. L'unico chiamante legittimo e' il preflight, che vive "
            "in questo crate.",
        )

    if errori:
        for messaggio in errori:
            print(messaggio, file=sys.stderr)
        return 1

    print(
        "confine del permit verificato: una sola via di decomposizione, marcata, "
        f"confinata a {' e '.join(CRATE_INTERNI)}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
