#!/usr/bin/env python3
"""Le copie in `contracts/copie-dal-pin/` sono i byte del checkout fissato.

# Perche' esiste

Alcune prove Rust validano i documenti del prodotto con gli schemi del
repository dei contratti, o li confrontano con i suoi vettori e registri. Il
checkout fissato esiste solo nel job `profilo-pubblico`, quindi le prove
leggono copie. Una copia che
nessuno confronta con l'originale diverge al primo cambio di pin -- o a una
modifica fatta a mano per far tornare un verde -- e da quel giorno le prove
proverebbero la conformita' a un documento che il pin non contiene.

# Che cosa verifica

1. `provenienza.json` cita la stessa revisione di `contracts/adoption-source.json`;
2. ogni file elencato e' identico, byte per byte, al percorso di origine nel
   checkout passato con `--contracts`;
3. la cartella non contiene file che `provenienza.json` non elenca.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import sys

RADICE = pathlib.Path(__file__).resolve().parent.parent
COPIE = RADICE / "contracts" / "copie-dal-pin"
PIN = RADICE / "contracts" / "adoption-source.json"


def verifica(copie: pathlib.Path, contracts: pathlib.Path, pin: str) -> list[str]:
    errori: list[str] = []
    provenienza = json.loads((copie / "provenienza.json").read_text(encoding="utf-8"))
    if provenienza.get("revisione") != pin:
        errori.append(
            f"le copie vengono da «{provenienza.get('revisione')}» e il pin e' «{pin}»"
        )
    elencati = provenienza.get("file", {})
    for nome, origine in sorted(elencati.items()):
        copia = copie / nome
        sorgente = contracts / origine
        if not copia.is_file():
            errori.append(f"«{nome}» e' elencato e non c'e'")
        elif not sorgente.is_file():
            errori.append(f"«{origine}» non esiste nel checkout fissato")
        elif copia.read_bytes() != sorgente.read_bytes():
            errori.append(f"«{nome}» differisce da «{origine}» del checkout fissato")
    presenti = {p.name for p in copie.iterdir() if p.is_file()} - {"provenienza.json"}
    for estraneo in sorted(presenti - set(elencati)):
        errori.append(f"«{estraneo}» sta fra le copie e la provenienza non lo elenca")
    return errori


def main() -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument("--contracts", required=True)
    opzioni = argomenti.parse_args()
    pin = json.loads(PIN.read_text(encoding="utf-8"))["contracts_source"]["revision"]
    errori = verifica(COPIE, pathlib.Path(opzioni.contracts), pin)
    for errore in errori:
        print(errore, file=sys.stderr)
    if errori:
        return 1
    print(f"copie dal pin verificate contro il checkout fissato ({pin[:12]})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
