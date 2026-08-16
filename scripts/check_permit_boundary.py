#!/usr/bin/env python3
"""Confine workspace-internal dell'`InputPermit` (INV-13, Lotto 0 / S4.b.3).

La formulazione originaria di INV-13 dichiarava il permit "mai separabile"
dal proprio bundle. Rust non sa esprimere quella garanzia fra crate distinti:
`pub(crate)` non basta — `plenora-io-core` e' un crate diverso da
`plenora-io-model` — e un `pub(workspace)` non esiste. Un'API che il core deve
poter chiamare e' necessariamente `pub`, quindi visibile a chiunque aggiunga
il modello fra le proprie dipendenze.

INV-13 e' stato percio' corretto in S4.b.3, e dice ora qualcosa di piu'
stretto ma vero: il permit e' **non costruibile, non clonabile e legato al
context**, e queste tre sono garanzie del linguaggio; e' invece **separabile
per move**, e quella separazione e' confinata al workspace da tre fatti
verificabili, che questo gate controlla:

1. entrambi i crate sono `publish = false`, quindi l'API non raggiunge un
   consumer esterno per la via del registry;
2. esiste **una sola** via di decomposizione, marcata `#[doc(hidden)]`;
3. nessun altro crate del workspace la usa.

Non e' una prova di impossibilita' — nessun grep lo e'. E' cio' che rende il
confine verificabile invece che dichiarato, ed e' esattamente la differenza
che la vecchia formulazione di INV-13 nascondeva.

## Perimetro e forme riconosciute (S4.d, parte 0)

La prima versione guardava solo `crates/*/src/**` e la sola forma a metodo.
Due buchi reali:

* un test d'integrazione in `tests/`, un benchmark in `benches/`, un
  `examples/` o un `build.rs` potevano attraversare il confine senza che
  nulla lo vedesse — e sono proprio i posti dove si scrive codice "di
  servizio" con meno attenzione;
* `ReadBudgetParts::into_components(parts)` in forma UFCS fa esattamente cio'
  che `.into_components()` fa, e non veniva contato.

Allo stesso modo `publish = false` era cercato come testo: una riga
commentata lo avrebbe soddisfatto. Ora il manifesto viene letto come TOML.
"""

from __future__ import annotations

import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# I due crate ai quali il confine e' riservato.
CRATE_INTERNI = ("plenora-io-model", "plenora-io-core")

# L'unica via di decomposizione rimasta, nelle forme in cui Rust la consente:
# metodo, UFCS, e riferimento alla funzione senza chiamata. La parentesi non
# e' richiesta: `let f = ReadBudgetParts::into_components;` attraversa il
# confine esattamente come una chiamata, solo piu' tardi.
DECOMPOSIZIONE = re.compile(
    r"\.into_components\b|\.into_budget\b|"
    r"\b(?:ReadBudgetParts|WriteBudgetParts)\s*::\s*into_(?:components|budget)\b"
)
PERMIT = re.compile(r"\bInputPermit\b")

# Firme che non devono ricomparire.
ESTRATTORE_RIMOSSO = re.compile(r"pub (?:const )?fn take_input_permit")
DOC_HIDDEN = re.compile(
    r"#\[doc\(hidden\)\]\s*(?:#\[must_use\]\s*)?pub fn (into_components|into_budget)"
)


def sorgenti_rust(root: Path) -> list[tuple[str, Path]]:
    """Ogni `.rs` di ogni crate, non solo quelli sotto `src/`.

    Restituisce coppie (crate, percorso). `target/` e' escluso perche'
    contiene artefatti generati, non sorgenti del workspace.
    """
    trovati: list[tuple[str, Path]] = []
    for crate_dir in sorted((root / "crates").iterdir()):
        if not crate_dir.is_dir():
            continue
        for sorgente in sorted(crate_dir.rglob("*.rs")):
            if "target" in sorgente.relative_to(crate_dir).parts:
                continue
            trovati.append((crate_dir.name, sorgente))
    # Il crate di fuzzing vive fuori da `crates/` ma e' workspace a tutti gli
    # effetti: i suoi target sono codice che compila contro il modello.
    fuzz = root / "fuzz"
    if fuzz.is_dir():
        for sorgente in sorted(fuzz.rglob("*.rs")):
            if "target" in sorgente.relative_to(fuzz).parts:
                continue
            trovati.append(("fuzz", sorgente))
    return trovati


def e_pubblicabile(manifest: Path) -> bool | None:
    """`True`/`False` secondo il TOML, `None` se il manifesto non si legge.

    Letto come TOML e non cercato come testo: `# publish = false` in un
    commento soddisfaceva la ricerca letterale pur non avendo alcun effetto.
    Il default di Cargo, in assenza della chiave, e' pubblicabile.
    """
    try:
        dati = tomllib.loads(manifest.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError):
        return None
    package = dati.get("package")
    if not isinstance(package, dict):
        return None
    publish = package.get("publish", True)
    # Cargo accetta anche una lista di registry: una lista vuota equivale a
    # `false`, una non vuota consente la pubblicazione su quei registry.
    if isinstance(publish, list):
        return len(publish) > 0
    return bool(publish)


def main() -> int:
    errori: list[str] = []

    # 1. Premessa del confine: nessuno dei due crate e' pubblicabile.
    for crate in CRATE_INTERNI:
        manifest = ROOT / "crates" / crate / "Cargo.toml"
        if not manifest.is_file():
            errori.append(f"{crate}: Cargo.toml assente")
            continue
        pubblicabile = e_pubblicabile(manifest)
        if pubblicabile is None:
            errori.append(
                f"{crate}: Cargo.toml illeggibile o senza sezione [package]; "
                "il confine del permit non e' verificabile."
            )
        elif pubblicabile:
            errori.append(
                f"{crate}: risulta pubblicabile. Il confine workspace-internal "
                "del permit poggia sul fatto che questi crate non raggiungano "
                "un consumer esterno; senza, la garanzia di INV-13 va "
                "riformulata di nuovo, non solo il gate."
            )

    budget = ROOT / "crates" / "plenora-io-model" / "src" / "budget.rs"
    testo_budget = budget.read_text(encoding="utf-8")

    # 2. Una sola via di decomposizione, e marcata.
    if ESTRATTORE_RIMOSSO.search(testo_budget):
        errori.append(
            "plenora-io-model: e' ricomparso un `take_input_permit` pubblico. La "
            "decomposizione deve restare un solo punto: due vie per la stessa "
            "separazione sono cio' che S4.b.3 ha rimosso."
        )
    marcati = set(DOC_HIDDEN.findall(testo_budget))
    for atteso in ("into_components", "into_budget"):
        if atteso not in marcati:
            errori.append(
                f"plenora-io-model: `{atteso}` non e' marcato `#[doc(hidden)]`. "
                "La marcatura e' meta' del confine: senza, l'API compare nella "
                "documentazione come se fosse d'uso generale."
            )

    # 3. Nessun altro crate attraversa il confine, in nessuna delle sue forme.
    for crate, sorgente in sorgenti_rust(ROOT):
        if crate in CRATE_INTERNI:
            continue
        contenuto = sorgente.read_text(encoding="utf-8")
        percorso = sorgente.relative_to(ROOT).as_posix()
        if DECOMPOSIZIONE.search(contenuto):
            errori.append(
                f"{percorso}: usa l'API di decomposizione delle parti, riservata a "
                f"{' e '.join(CRATE_INTERNI)}. Un driver riceve le opzioni gia' "
                "costruite: non deve mai scomporre le parti da se'."
            )
        if PERMIT.search(contenuto):
            errori.append(
                f"{percorso}: nomina `InputPermit`. Il permit non deve uscire dal "
                "confine model/core nemmeno come tipo."
            )

    # 4. Lato core, l'estrattore non e' pubblico.
    driver = ROOT / "crates" / "plenora-io-core" / "src" / "driver.rs"
    testo_driver = driver.read_text(encoding="utf-8")
    if not re.search(r"pub\(crate\) (?:const )?fn take_input_permit", testo_driver):
        errori.append(
            "plenora-io-core: `ReadOptions::take_input_permit` deve essere "
            "`pub(crate)`. L'unico chiamante legittimo e' il preflight, che vive "
            "in questo crate."
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
