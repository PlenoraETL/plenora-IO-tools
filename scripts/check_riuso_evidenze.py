#!/usr/bin/env python3
"""Verifica il registro del riuso delle evidenze.

# Che cosa questo gate puo' dire, e che cosa no

Puo' dire che la regola e' **scritta** e che chi dovrebbe accorgersene
**esiste**: ogni tipo di modifica dichiarato, ogni campo pieno, ogni
sorvegliante nominato presente sul disco ed eseguibile come gli altri gate.

Non puo' dire che quel sorvegliante se ne accorga davvero. Che
`check_profondita_fuzz.py` invalidi una misura quando il perimetro cambia lo
provano le sue regressioni, non questa; qui si verifica che il registro non
nomini un file che nessuno ha mai scritto, o che qualcuno ha rinominato
lasciando la riga indietro. E' il difetto piu' probabile di una tabella come
questa, ed e' quello che il gate prende.

# Perche' i sei tipi sono un insieme chiuso

Perche' la via piu' breve al verde, per una tabella, e' togliere la riga che
non si vuole compilare. I sei tipi -- prodotto, test, documenti, toolchain,
feature, lock -- vengono dal criterio della voce R4, sono scritti qui e non
solo nel registro, e toglierne uno e' rosso.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
REGISTRO = ROOT / "assurance" / "registries" / "riuso-delle-evidenze.json"

#: I tipi che il criterio di R4 nomina. Insieme chiuso: ne' meno ne' altri.
TIPI = frozenset(
    {"prodotto", "test", "documenti", "toolchain", "feature", "lock"}
)

CAMPI = (
    "tipo",
    "che_cos_e",
    "che_cosa_invalida",
    "che_cosa_resta_valido",
)


def verifica(documento: Any) -> list[str]:
    """I motivi per cui il registro non regge; vuoto se regge."""
    if not isinstance(documento, dict) or not isinstance(
        documento.get("tipi_di_modifica"), list
    ):
        return ["il registro non porta un elenco `tipi_di_modifica`"]

    motivi: list[str] = []
    visti: set[str] = set()
    for indice, voce in enumerate(documento["tipi_di_modifica"]):
        dove = f"tipi_di_modifica[{indice}]"
        if not isinstance(voce, dict):
            motivi.append(f"{dove}: non e' un oggetto")
            continue
        for campo in CAMPI:
            if not isinstance(voce.get(campo), str) or not voce[campo].strip():
                motivi.append(f"{dove}: `{campo}` assente o vuoto")
        tipo = voce.get("tipo")
        if isinstance(tipo, str):
            if tipo in visti:
                motivi.append(f"{dove}: tipo «{tipo}» ripetuto")
            visti.add(tipo)

        sorveglianti = voce.get("chi_se_ne_accorge")
        if not isinstance(sorveglianti, list) or not sorveglianti:
            motivi.append(
                f"{dove}: `chi_se_ne_accorge` vuoto. Una regola che nessuno "
                "verifica e' una buona intenzione, e questo registro esiste "
                "per non averne."
            )
            continue
        for nome in sorveglianti:
            if not isinstance(nome, str) or not (ROOT / nome).is_file():
                motivi.append(
                    f"{dove}: «{nome}» non e' un file del repository. Un "
                    "sorvegliante rinominato lascia la riga indietro, e la "
                    "regola comincia a descrivere un mondo che non c'e'."
                )

    for mancante in sorted(TIPI - visti):
        motivi.append(
            f"tipo «{mancante}» assente dal registro: i sei tipi vengono dal "
            "criterio di R4, e toglierne uno sarebbe la via piu' breve al verde"
        )
    for estraneo in sorted(visti - TIPI):
        motivi.append(
            f"tipo «{estraneo}» non e' fra i sei del criterio: aggiungerne uno "
            "e' una decisione, e passa da qui"
        )
    return motivi


def main(argv: list[str] | None = None) -> int:
    del argv
    if not REGISTRO.exists():
        print(f"{REGISTRO}: registro assente", file=sys.stderr)
        return 2
    try:
        documento = json.loads(REGISTRO.read_text(encoding="utf-8"))
    except json.JSONDecodeError as guasto:
        print(f"{REGISTRO}: non e' JSON leggibile ({guasto})", file=sys.stderr)
        return 2

    motivi = verifica(documento)
    for motivo in motivi:
        print(f"riuso delle evidenze: {motivo}", file=sys.stderr)
    if motivi:
        return 1

    quanti = len(documento["tipi_di_modifica"])
    sorveglianti = {
        nome
        for voce in documento["tipi_di_modifica"]
        for nome in voce["chi_se_ne_accorge"]
    }
    print(
        f"riuso delle evidenze: {quanti} tipi di modifica, "
        f"{len(sorveglianti)} sorveglianti distinti, tutti presenti. "
        "Che se ne accorgano davvero lo provano le loro regressioni, non questa."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
