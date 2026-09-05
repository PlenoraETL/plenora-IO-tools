#!/usr/bin/env python3
"""`requires-python` copre esattamente le versioni che la CI prova.

# Perche' esiste

`Requires-Python` e' cio' che pip legge per decidere se un pacchetto possa
girare. Non e' una stima ne' un augurio: e' un'affermazione, e chi la legge la
usa per rifiutare un'installazione o per accettarla.

Dichiarare `>=3.11` senza limite superiore prometterebbe **ogni** Python futuro,
compresi quelli che romperanno qualcosa. Dichiarare un limite piu' stretto delle
versioni provate rifiuterebbe installazioni che funzionano. Le due sbagliano in
direzioni opposte e nessuna delle due si vede provando su una versione sola.

Il gate lega quindi la riga alla matrice della CI, nei due versi: ogni versione
provata dev'essere ammessa, e ogni versione ammessa dev'essere provata.

# Perche' non usa `packaging`

Perche' aggiungerebbe una dipendenza a un gate che gira prima di installare
qualunque cosa. Il vincolo di questo pacchetto ha la forma `>=X.Y,<Z.W`, e
confrontare due coppie di interi non richiede una libreria. Una forma diversa
ferma il gate invece di essere interpretata a caso: fallire chiuso su cio' che
non si sa leggere e' cio' che distingue un controllo da una congettura.
"""

from __future__ import annotations

import argparse
import pathlib
import re
import sys

RADICE = pathlib.Path(__file__).resolve().parent.parent
PYPROJECT = RADICE / "sdk" / "python" / "pyproject.toml"
CI = RADICE / ".github" / "workflows" / "ci.yml"


def vincolo() -> tuple[tuple[int, int], tuple[int, int]]:
    """`(minimo incluso, massimo escluso)` da `requires-python`."""
    testo = PYPROJECT.read_text(encoding="utf-8")
    trovato = re.search(r'^requires-python = "([^"]+)"$', testo, re.M)
    if trovato is None:
        raise SystemExit("`requires-python` non si trova in pyproject.toml")
    grezzo = trovato.group(1)
    forma = re.fullmatch(r">=(\d+)\.(\d+),<(\d+)\.(\d+)", grezzo)
    if forma is None:
        raise SystemExit(
            f"`requires-python` vale «{grezzo}», che questo gate non sa "
            "leggere: la forma attesa e' `>=X.Y,<Z.W`. Interpretare a caso una "
            "forma sconosciuta sarebbe una congettura, e un vincolo di "
            "installazione non e' il posto per congetturare."
        )
    minimo = (int(forma.group(1)), int(forma.group(2)))
    massimo = (int(forma.group(3)), int(forma.group(4)))
    if minimo >= massimo:
        raise SystemExit(
            f"`requires-python` vale «{grezzo}»: il minimo non e' sotto il "
            "massimo, e nessuna versione lo soddisfa."
        )
    return minimo, massimo


def matrice_della_ci() -> list[tuple[int, int]]:
    """Le versioni che il job `python-sdk` prova.

    Si leggono dal workflow e non da un elenco a parte: un elenco copiato
    sarebbe la seconda scrittura che questo gate esiste per non avere.
    """
    testo = CI.read_text(encoding="utf-8")
    blocco = re.search(r'^\s+python: \[(.+?)\]$', testo, re.M)
    if blocco is None:
        raise SystemExit(
            "la matrice `python:` non si trova in ci.yml: senza, il vincolo "
            "non e' legato a niente."
        )
    versioni = []
    for grezza in re.findall(r'"(\d+)\.(\d+)"', blocco.group(1)):
        versioni.append((int(grezza[0]), int(grezza[1])))
    if not versioni:
        raise SystemExit("la matrice `python:` e' vuota")
    return sorted(versioni)


def coperta(versione: tuple[int, int]) -> bool:
    minimo, massimo = vincolo()
    return minimo <= versione < massimo


def main(argv: list[str] | None = None) -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument(
        "--versione",
        default=None,
        help="una versione sola, per il passo che gira dentro la matrice",
    )
    opzioni = argomenti.parse_args(argv)

    minimo, massimo = vincolo()
    provate = matrice_della_ci()
    problemi: list[str] = []

    if opzioni.versione is not None:
        pezzi = opzioni.versione.split(".")
        singola = (int(pezzi[0]), int(pezzi[1]))
        if not coperta(singola):
            print(
                f"la CI prova Python {opzioni.versione}, che "
                f"`requires-python` esclude: il pacchetto si costruirebbe e non "
                "si installerebbe.",
                file=sys.stderr,
            )
            return 1
        print(f"Python {opzioni.versione} rientra nel vincolo dichiarato")
        return 0

    # --- i due versi ------------------------------------------------------
    for versione in provate:
        if not coperta(versione):
            problemi.append(
                f"la CI prova Python {versione[0]}.{versione[1]} e "
                "`requires-python` lo esclude: si prova qualcosa che il "
                "pacchetto dichiara di non supportare."
            )

    # Ogni versione **dentro** il vincolo dev'essere provata. E' il verso che
    # conta di piu': e' quello che impedisce di allargare la riga senza
    # allargare la matrice, cioe' di promettere un Python su cui nessuno ha
    # guardato.
    attese = []
    minore, maggiore = minimo, massimo
    versione = minore
    while versione < maggiore:
        attese.append(versione)
        versione = (versione[0], versione[1] + 1)
    for versione in attese:
        if versione not in provate:
            problemi.append(
                f"`requires-python` ammette Python {versione[0]}.{versione[1]} "
                "e la matrice della CI non lo prova: la riga promette una "
                "versione su cui nessuno ha guardato."
            )

    if problemi:
        for problema in problemi:
            print(problema, file=sys.stderr)
        return 1

    dichiarate = ", ".join(f"{a}.{b}" for a, b in attese)
    print(
        f"requires-python: >={minimo[0]}.{minimo[1]},<{massimo[0]}.{massimo[1]} "
        f"-- {len(attese)} versioni ammesse ({dichiarate}), tutte provate dalla CI"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
