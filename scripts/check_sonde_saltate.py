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

try:
    from . import strict_json
except ImportError:
    import strict_json

ROOT = pathlib.Path(__file__).resolve().parents[1]
REGISTRO = ROOT / "assurance" / "registries" / "sonde-saltate-nella-sdist.json"
POLITICA = REGISTRO.with_name("sonde-saltate-policy.json")


def genera(politica: dict, misurato: dict, precedente: dict | None = None) -> dict:
    """Only approved reasons; identities and totals come from actual discovery."""
    import copy
    result = copy.deepcopy(politica)
    ids = misurato.get("sonde", [])
    skipped = misurato["saltate"]
    platform = misurato.get('piattaforma', sys.platform)
    if (misurato["fallite"] or not ids or len(ids) != len(set(ids)) or
            misurato["eseguite"] != len(ids) or
            len({s['id'] for s in skipped}) != len(skipped) or
            not {s['id'] for s in skipped} <= set(ids)):
        raise ValueError("misurazione incompleta, duplicata o con fallimenti")
    reasons = [c['ragione_del_salto'] for c in result['condizioni']]
    if len(set(reasons)) != len(reasons) or any(s['ragione'] not in reasons for s in skipped):
        raise ValueError("ragione di salto non approvata o ambigua")
    for condition in result['condizioni']:
        for key in ('che_cosa_manca', 'perche_non_sta_nella_sdist', 'che_cosa_resta_verificato'):
            if not condition.get(key, '').strip():
                raise ValueError("esclusione senza motivazione")
        condition['sonde'] = sorted(s['id'] for s in skipped if s['ragione'] == condition['ragione_del_salto'])
        if 'piattaforme' in condition and platform not in condition['piattaforme']:
            if condition['sonde']:
                raise ValueError('salto non applicabile alla piattaforma')
            previous = next((c for c in (precedente or {}).get('condizioni', []) if c['id'] == condition['id']), {})
            condition['sonde'] = previous.get('sonde', [])
            if not set(condition['sonde']) <= set(ids):
                raise ValueError('esclusione di piattaforma obsoleta')
        condition['quante'] = len(condition['sonde'])
    common = {s for c in result['condizioni'] if 'piattaforme' not in c for s in c['sonde']}
    result['totale'] = len(common)
    result['eseguite_nella_sdist'] = len(ids) - len(common)
    result['sonde_eseguite'] = sorted(set(ids) - common)
    return result


def main(argv: list[str] | None = None) -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument('--write', action='store_true', help='rigenera dalla misurazione, solo con ragioni approvate')
    argomenti.add_argument(
        "--misurato",
        required=True,
        type=pathlib.Path,
        help="il documento che `elenca-sonde-saltate.py` ha prodotto",
    )
    opzioni = argomenti.parse_args(argv)

    registro = strict_json.loads(REGISTRO.read_text(encoding="utf-8"))
    misurato = strict_json.loads(opzioni.misurato.read_text(encoding="utf-8"))
    try:
        expected = genera(strict_json.loads(POLITICA.read_text(encoding='utf-8')), misurato, registro)
    except (ValueError, KeyError, TypeError):
        print('misurazione non generabile: verificare completezza e politica delle esclusioni', file=sys.stderr)
        return 1
    if opzioni.write:
        REGISTRO.write_text(json.dumps(expected, ensure_ascii=False, indent=2) + '\n', encoding='utf-8', newline='\n')
        return 0
    problemi: list[str] = []
    if registro != expected:
        problemi.append('registro diverso dalla fonte unica: rigenerare da una misurazione verificata con --write')

    if misurato["fallite"]:
        problemi.append(
            f"{misurato['fallite']} sonde sono **fallite** nell'installazione: "
            "prima di contare le saltate, va guardato quello."
        )

    dichiarate: dict[str, str] = {}
    for condizione in registro["condizioni"]:
        if misurato.get('piattaforma', sys.platform) not in condizione.get('piattaforme', [misurato.get('piattaforma', sys.platform)]):
            continue
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
    expected_total = len(dichiarate)
    if expected_total != len(osservate):
        problemi.append(
            f"il registro dichiara {expected_total} saltate sulla piattaforma e ne saltano "
            f"{len(osservate)}."
        )
    eseguite = misurato["eseguite"] - len(osservate)
    platform_extra = sum(c['quante'] for c in registro['condizioni'] if misurato.get('piattaforma', sys.platform) in c.get('piattaforme', []))
    if registro["eseguite_nella_sdist"] - platform_extra != eseguite:
        problemi.append(
            f"il registro dichiara {registro['eseguite_nella_sdist'] - platform_extra} sonde "
            f"eseguite nell'installazione e ne girano {eseguite}."
        )

    if problemi:
        for problema in problemi:
            print(problema, file=sys.stderr)
        return 1

    dettaglio = ", ".join(
        f"{c['quante']} {c['id']}" for c in registro["condizioni"]
        if misurato.get('piattaforma', sys.platform) in c.get('piattaforme', [misurato.get('piattaforma', sys.platform)])
    )
    print(
        f"sonde saltate verificate: {len(osservate)} su {misurato['eseguite']}, "
        f"tutte registrate e nessuna registrata in piu' ({dettaglio})."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
