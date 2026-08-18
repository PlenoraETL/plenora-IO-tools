#!/usr/bin/env python3
"""La quarantena dei fuzz target deve restare vuota (FZ-0).

`scripts/fuzz-smoke.sh` puo' saltare un target elencato in
`fuzz/quarantine.txt`. Il meccanismo serve — un gate che fallisce sempre smette
di essere letto, e copre le regressioni nuove insieme al finding vecchio — ma ha
un difetto proprio: una riga aggiunta oggi non fa rumore domani. Il debito
diventa arredamento, e lo smoke resta verde su un target che nessuno esegue.

FZ-0 ha portato la quarantena a zero impedendo i panici invece di catturarli.
Questo gate difende quel risultato: una riga attiva **blocca il rilascio**.

## Cosa non fa

Non vieta di quarantinare. Se domani un finding non fosse chiudibile, la riga si
scrive, e questo gate diventa rosso: e' il punto. Non si aggira per comodita' —
si aggira ratificando l'eccezione, e la ratifica lascia traccia dove il file da
solo non ne lascerebbe.

## Come misura

Una riga e' **attiva** se non e' vuota e non e' un commento. L'intestazione del
file — che spiega il meccanismo e va conservata — e' fatta di commenti, quindi
un file di sola documentazione passa.
"""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
QUARANTENA = "fuzz/quarantine.txt"


def righe_attive(testo: str) -> list[str]:
    """Le righe che dichiarano un target quarantinato."""
    return [
        riga.strip()
        for riga in testo.splitlines()
        if riga.strip() and not riga.lstrip().startswith("#")
    ]


def verifica(radice: Path) -> list[str]:
    """Restituisce le violazioni; vuoto se nessun target e' quarantinato."""
    percorso = radice / QUARANTENA
    if not percorso.is_file():
        # Nessun file, nessuna quarantena: e' lo stato piu' pulito possibile.
        return []

    attive = righe_attive(percorso.read_text(encoding="utf-8"))
    if not attive:
        return []

    errori = [
        f"{QUARANTENA}: {len(attive)} target in quarantena, ma la quarantena "
        "deve restare vuota. Un target quarantinato non viene eseguito dallo "
        "smoke, quindi le sue regressioni nuove non le vede nessuno."
    ]
    errori.extend(f"  - {riga.split()[0]}" for riga in attive)
    errori.append(
        "Se il finding non e' chiudibile, la quarantena va ratificata come "
        "eccezione: questo gate esiste perche' la decisione lasci traccia."
    )
    return errori


def main() -> int:
    errori = verifica(ROOT)
    if errori:
        for messaggio in errori:
            print(messaggio, file=sys.stderr)
        return 1
    print("quarantena dei fuzz target vuota: ogni target e' eseguito dallo smoke")
    return 0


if __name__ == "__main__":
    sys.exit(main())
