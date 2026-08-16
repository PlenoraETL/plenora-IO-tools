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
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# Tetto corrente per categoria. Ogni sottopasso di S4 lo abbassa; S4.e lo
# porta a zero e rimuove questo script.
TETTI: dict[str, int] = {
    "read_options_default": 76,
    "write_options_default": 76,
    "read_options_literal": 20,
    "write_options_literal": 6,
    "campo_limits": 45,
    "campo_resource_budget": 36,
    "campo_cancellation": 28,
}

MODELLI: dict[str, re.Pattern[str]] = {
    "read_options_default": re.compile(r"ReadOptions::default\(\)"),
    "write_options_default": re.compile(r"WriteOptions::default\(\)"),
    "read_options_literal": re.compile(r"ReadOptions \{"),
    "write_options_literal": re.compile(r"WriteOptions \{"),
    "campo_limits": re.compile(r"\b(?:opts|options)\.limits\b"),
    "campo_resource_budget": re.compile(r"\b(?:opts|options)\.resource_budget\b"),
    "campo_cancellation": re.compile(r"\b(?:opts|options)\.cancellation\b"),
}


def conta(root: Path) -> dict[str, int]:
    conteggi = {nome: 0 for nome in MODELLI}
    for sorgente in sorted((root / "crates").glob("*/src/**/*.rs")):
        testo = sorgente.read_text(encoding="utf-8")
        for nome, modello in MODELLI.items():
            conteggi[nome] += len(modello.findall(testo))
    return conteggi


def main() -> int:
    conteggi = conta(ROOT)
    errori: list[str] = []
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
