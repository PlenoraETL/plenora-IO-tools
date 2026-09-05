#!/usr/bin/env python3
"""L'elenco delle sonde che l'installazione non esercita e' chiuso.

# Che cosa protegge

La suite dell'SDK gira due volte: nel repository, dove il contratto e le fixture
ci sono, e dalla **sdist installata**, dove non ci sono. Nella seconda alcune
sonde saltano, ed e' giusto: sono quelle d'integrazione, e non hanno niente da
guardare.

Cio' che non e' giusto e' che quel numero resti un numero. Se domani una sonda
smettesse di girare per un difetto -- un `skipUnless` scritto male, un import
che fallisce e viene inghiottito -- il conteggio resterebbe plausibile e nessuno
guarderebbe. Un elenco chiuso nomina ciascuna.

# I due versi, e perche' il secondo conta

Una sonda che salta **senza essere registrata** e' una copertura persa in
silenzio. Una registrata che **non salta piu'** e' una riga da togliere: vuol
dire che e' diventata indipendente dal repository, e un registro che accettasse
voci obsolete descriverebbe un passato invece del presente.

# Che cosa non afferma

Che quelle sonde siano verificate **da qualche parte**. Lo sono -- il job
`python-sdk` della CI le esegue nel repository, su ogni versione della matrice
-- ma quel fatto lo stabilisce quel job, non questo gate. Qui si stabilisce che
l'elenco sia esatto: chi salta e' chi diciamo che salta, e nessun altro.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
REGISTRO = ROOT / "assurance" / "registries" / "sonde-saltate-nella-sdist.json"


def main(argv: list[str] | None = None) -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument(
        "--misurato",
        required=True,
        type=pathlib.Path,
        help="il documento che `elenca-sonde-saltate.py` ha prodotto",
    )
    opzioni = argomenti.parse_args(argv)

    registro = json.loads(REGISTRO.read_text(encoding="utf-8"))
    misurato = json.loads(opzioni.misurato.read_text(encoding="utf-8"))
    problemi: list[str] = []

    if misurato["fallite"]:
        problemi.append(
            f"{misurato['fallite']} sonde sono **fallite** nell'installazione: "
            "prima di contare le saltate, va guardato quello."
        )

    dichiarate: dict[str, str] = {}
    for condizione in registro["condizioni"]:
        for sonda in condizione["sonde"]:
            if sonda in dichiarate:
                problemi.append(
                    f"«{sonda}» e' registrata due volte, sotto "
                    f"«{dichiarate[sonda]}» e «{condizione['id']}»: una sonda "
                    "salta per una ragione sola."
                )
            dichiarate[sonda] = condizione["id"]
        for campo in ("che_cosa_manca", "perche_non_sta_nella_sdist", "che_cosa_resta_verificato"):
            if not str(condizione.get(campo, "")).strip():
                problemi.append(
                    f"la condizione «{condizione['id']}» non dice «{campo}». "
                    "Registrare una sonda saltata costa dire perche', o "
                    "l'elenco diventa il posto dove si toglie copertura senza "
                    "renderne conto."
                )

    osservate = {voce["id"]: voce["ragione"] for voce in misurato["saltate"]}

    for sonda in sorted(set(osservate) - set(dichiarate)):
        problemi.append(
            f"«{sonda}» salta nell'installazione e non e' registrata: una "
            "copertura che sparisce senza che nessuno la nomini."
        )
    for sonda in sorted(set(dichiarate) - set(osservate)):
        problemi.append(
            f"«{sonda}» e' registrata come saltata e non salta piu': se e' "
            "diventata indipendente dal repository, la riga va tolta."
        )

    # I conteggi che il registro dichiara devono venire dallo stesso fatto.
    if registro["totale"] != len(osservate):
        problemi.append(
            f"il registro dichiara {registro['totale']} saltate e ne saltano "
            f"{len(osservate)}."
        )
    eseguite = misurato["eseguite"] - len(osservate)
    if registro["eseguite_nella_sdist"] != eseguite:
        problemi.append(
            f"il registro dichiara {registro['eseguite_nella_sdist']} sonde "
            f"eseguite nell'installazione e ne girano {eseguite}."
        )

    if problemi:
        for problema in problemi:
            print(problema, file=sys.stderr)
        return 1

    dettaglio = ", ".join(
        f"{c['quante']} {c['id']}" for c in registro["condizioni"]
    )
    print(
        f"sonde saltate verificate: {len(osservate)} su {misurato['eseguite']}, "
        f"tutte registrate e nessuna registrata in piu' ({dettaglio})."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
