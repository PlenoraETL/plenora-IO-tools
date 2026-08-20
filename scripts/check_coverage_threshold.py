"""Soglia di copertura letta **dal report esportato**, non dal profdata.

# Perche' esiste

`cargo llvm-cov report --summary-only --fail-under-lines 80` ricava i numeri dal
profdata, cioe' da una sorgente **diversa** dal report LCOV che gli altri gate
leggono. Due derivazioni della stessa misura possono divergere senza che nessuno
lo veda: e' successo il 2026-08-21 in forma estrema, quando una misura fallita
ha lasciato in piedi il profdata della corsa precedente e la soglia e' passata
su un albero che non era quello dichiarato.

Questo script legge **lo stesso file** che `check_coverage_exclusions.py` legge.
La versione di `cargo llvm-cov` resta come **controprova**: entrambe devono
passare, e se un giorno divergessero sarebbe un fatto da guardare, non un
dettaglio da mediare.

# Che cosa misura

Copertura di **riga**: righe strumentate eseguite almeno una volta, sul totale
delle righe strumentate. E' la definizione dei record `DA:` di LCOV, ed e' la
colonna su cui la soglia dell'80% e' stata ratificata.

Il perimetro e' quello del report: chi lo ha esportato ha gia' applicato le
esclusioni, e questo script non ne aggiunge ne' ne toglie. Un file che filtrasse
di nuovo direbbe di misurare la stessa cosa e misurerebbe un altro insieme.
"""

from __future__ import annotations

import argparse
import pathlib
import sys


def copertura_di_riga(percorso: pathlib.Path) -> tuple[int, int]:
    """Righe strumentate coperte e totali, da un report LCOV."""
    coperte = 0
    totali = 0
    for riga in percorso.read_text(encoding="utf-8").splitlines():
        if not riga.startswith("DA:"):
            continue
        _, _, resto = riga.partition(":")
        numero, _, conteggio = resto.partition(",")
        if not numero.strip():
            continue
        try:
            colpi = int(conteggio.split(",")[0])
        except ValueError:
            continue
        totali += 1
        if colpi > 0:
            coperte += 1
    return coperte, totali


def main() -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument("--lcov", required=True, type=pathlib.Path)
    argomenti.add_argument("--min-lines", required=True, type=float)
    opzioni = argomenti.parse_args()

    if not opzioni.lcov.exists():
        print(f"{opzioni.lcov}: report LCOV assente.", file=sys.stderr)
        return 2
    if opzioni.lcov.stat().st_size == 0:
        print(f"{opzioni.lcov}: report LCOV vuoto.", file=sys.stderr)
        return 2

    coperte, totali = copertura_di_riga(opzioni.lcov)
    if totali == 0:
        # Un report senza righe strumentate darebbe 100% per divisione vuota, e
        # sarebbe il verde piu' falso possibile.
        print(
            f"{opzioni.lcov}: nessuna riga strumentata nel report.",
            file=sys.stderr,
        )
        return 2

    percentuale = 100.0 * coperte / totali
    print(
        f"copertura di riga dal report: {percentuale:.2f}% "
        f"({coperte}/{totali} righe strumentate)"
    )
    if percentuale + 1e-9 < opzioni.min_lines:
        print(
            f"sotto la soglia di {opzioni.min_lines:.0f}%: "
            "si aggiungono test, non si abbassa la soglia.",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
