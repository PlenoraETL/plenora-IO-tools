#!/usr/bin/env python3
"""Inventario temporaneo degli usi del modello budget legacy (Lotto 0, S4).

La migrazione al modello unificato attraversa quattro sottopassi e non puo'
essere fatta in un commit solo senza rendere l'albero irrevisionabile. Nel
mezzo i due modelli coesistono, ed e' esattamente la fase in cui un uso
legacy nuovo passerebbe inosservato: il codice compila, i test sono verdi, e
il debito cresce senza che nessun gate lo veda.

Questo gate conta gli usi residui e li confronta con un tetto dichiarato. Il
tetto puo' solo **scendere**: ogni sottopasso lo abbassa, e S4.e lo porta a
zero rimuovendo il tipo transitorio insieme al gate stesso.

Non e' una misura semantica ma sintattica, come il registro dei fallback: un
uso legacy scritto in una forma che la regex non riconosce non viene contato.
Serve a impedire la crescita distratta, non a dimostrare l'assenza.

## Ridefinizione delle categorie in S4.b

Le regex della prima versione misuravano il testo sbagliato e vanno lette con
questa avvertenza quando si confrontano i numeri fra S4.b parte 1 e parte 2:

* `campo_*` intercettava anche `opts.cancellation()`, cioe' l'**accessore
  nuovo**, non il campo legacy. Il numero saliva mentre la migrazione
  procedeva. Ora una lookahead negativa esclude la chiamata.
* `*_literal` intercettava `pub struct ReadOptions {`, `impl ReadOptions {` e
  `-> ReadOptions {`: dichiarazioni, non costruzioni. Ora sono escluse.

Le tre categorie `campo_*` e le due `*_literal` scendono percio' a zero in
S4.b parte 2 — il payload privato rende quelle forme **inesprimibili**, non
solo assenti — e restano a zero per costruzione. Non sono piu' la misura
utile: da qui in avanti il ponte si misura con le categorie aggiunte sotto,
che contano le uniche vie rimaste verso il modello legacy.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# Tetto corrente per categoria. Ogni sottopasso di S4 lo abbassa; S4.e lo
# porta a zero e rimuove questo script.
TETTI: dict[str, int] = {
    # Costruiscono il payload `Legacy` perche' `Default` non sa fare altro.
    # Scendono a zero in S4.e, quando `Default` sparisce e ogni chiamante
    # deve dichiarare il proprio `PipelineContext`.
    "read_options_default": 74,
    "write_options_default": 74,
    # Inesprimibili da S4.b parte 2: il payload e' privato. Restano nel
    # censimento come rete contro una riapertura dei campi.
    "read_options_literal": 0,
    "write_options_literal": 0,
    "campo_limits": 0,
    "campo_resource_budget": 0,
    "campo_cancellation": 0,
    # Le vie residue verso il modello legacy, tutte esplicite e marcate
    # "punto di rimozione: S4.e".
    "costruttore_legacy": 11,
    "accessore_legacy": 18,
    "ponte_richiede_legacy": 11,
}

# Le lookbehind escludono `struct X {`, `impl X {` e `-> X {`: sono
# dichiarazioni del tipo, non costruzioni del payload legacy. `(?!\s*\()`
# esclude gli accessori nuovi `opts.cancellation()` dai conteggi dei campi.
MODELLI: dict[str, re.Pattern[str]] = {
    "read_options_default": re.compile(r"ReadOptions::default\(\)"),
    "write_options_default": re.compile(r"WriteOptions::default\(\)"),
    "read_options_literal": re.compile(
        r"(?<!struct )(?<!impl )(?<!-> )ReadOptions \{"
    ),
    "write_options_literal": re.compile(
        r"(?<!struct )(?<!impl )(?<!-> )WriteOptions \{"
    ),
    "campo_limits": re.compile(r"\b(?:opts|options)\.limits\b(?!\s*\()"),
    "campo_resource_budget": re.compile(r"\b(?:opts|options)\.resource_budget\b(?!\s*\()"),
    "campo_cancellation": re.compile(r"\b(?:opts|options)\.cancellation\b(?!\s*\()"),
    "costruttore_legacy": re.compile(r"(?:ReadOptions|WriteOptions)::from_legacy\("),
    "accessore_legacy": re.compile(r"\.legacy_(?:budget|limits)\(\)"),
    "ponte_richiede_legacy": re.compile(r"bridge_richiede_legacy"),
}


def conta(root: Path) -> dict[str, int]:
    conteggi = {nome: 0 for nome in MODELLI}
    for sorgente in sorted((root / "crates").glob("*/src/**/*.rs")):
        testo = sorgente.read_text(encoding="utf-8")
        for nome, modello in MODELLI.items():
            conteggi[nome] += len(modello.findall(testo))
    return conteggi


# Da S4.c il ponte verso il modello legacy e' nominato **solo** dentro
# `plenora-io-core`. Non e' una questione di conteggio ma di struttura: se un
# driver torna a scrivere `opts.legacy_budget().ok_or_else(...)`, S4.d dovra'
# di nuovo cambiare tredici punti invece di due, ed e' esattamente cio' che
# rende non atomico il passaggio al modello unificato.
PONTE = re.compile(r"\.legacy_(?:budget|limits)\(\)|bridge_richiede_legacy")
CRATE_DEL_PONTE = "plenora-io-core"


def fuori_dal_core(root: Path) -> list[str]:
    trovati: list[str] = []
    for sorgente in sorted((root / "crates").glob("*/src/**/*.rs")):
        crate = sorgente.relative_to(root / "crates").parts[0]
        if crate == CRATE_DEL_PONTE:
            continue
        if PONTE.search(sorgente.read_text(encoding="utf-8")):
            trovati.append(sorgente.relative_to(root).as_posix())
    return trovati


def main() -> int:
    conteggi = conta(ROOT)
    errori: list[str] = []

    for percorso in fuori_dal_core(ROOT):
        errori.append(
            f"{percorso}: nomina il ponte verso il modello legacy. Da S4.c la "
            f"scelta di quale modello governi i contatori appartiene a "
            f"{CRATE_DEL_PONTE}: un driver riceve le opzioni e le passa, non "
            "le interroga sul modello."
        )
    for nome, tetto in sorted(TETTI.items()):
        trovati = conteggi[nome]
        if trovati > tetto:
            errori.append(
                f"{nome}: {trovati} usi, tetto {tetto}. "
                "Il tetto puo' solo scendere: la migrazione non deve creare "
                "usi legacy nuovi."
            )
        elif trovati < tetto:
            errori.append(
                f"{nome}: {trovati} usi, tetto {tetto}. "
                "Il tetto va abbassato a questo valore nello stesso commit "
                "che riduce gli usi, altrimenti il margine lascia rientrare "
                "in silenzio cio' che e' appena uscito."
            )

    if errori:
        for errore in errori:
            print(errore, file=sys.stderr)
        return 1

    totale = sum(conteggi.values())
    print(f"inventario budget legacy verificato: {totale} usi residui")
    if totale == 0:
        print(
            "nessun uso residuo: rimuovere il tipo transitorio e questo gate "
            "(S4.e)"
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
