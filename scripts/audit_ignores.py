#!/usr/bin/env python3
"""Stampa i flag `--ignore` di `cargo audit` dal registro delle eccezioni.

Gli ID vivevano in due posti: un elenco nel workflow di CI e le ragioni in un
documento. Due elenchi divergono, e quello nel workflow non porta la ragione
dell'eccezione — quindi chi lo legge non sa se sia ancora giustificata.

Qui l'elenco e' uno solo, `assurance/registries/dependency-exceptions.json`, e
la CI lo legge da qui. Aggiungere un'eccezione significa scriverne motivo,
esposizione, condizione di chiusura e trigger di riesame: e' quello il punto,
non il flag.
"""

from __future__ import annotations

import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
REGISTRO = ROOT / "assurance" / "registries" / "dependency-exceptions.json"

CAMPI = {"id", "crate", "accettata_dal", "motivo", "esposizione", "chiusura", "trigger_di_riesame"}


def flag(registro: dict) -> list[str]:
    """`['--ignore', 'RUSTSEC-…', …]`, o solleva se una voce e' incompleta."""
    fuori: list[str] = []
    for voce in registro.get("accettate", []):
        mancanti = CAMPI - set(voce)
        if mancanti:
            raise ValueError(
                f"{voce.get('id', '<senza id>')}: eccezione incompleta, mancano "
                f"{sorted(mancanti)}. Un'eccezione senza condizione di chiusura "
                "non e' temporanea, e' permanente senza dirlo."
            )
        fuori.extend(["--ignore", voce["id"]])
    return fuori


def main() -> int:
    try:
        print(" ".join(flag(json.loads(REGISTRO.read_text(encoding="utf-8")))))
    except ValueError as errore:
        print(errore, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
