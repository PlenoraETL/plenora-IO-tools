#!/usr/bin/env python3
"""I byte delle fixture canoniche sono quelli che il registro dichiara.

# Il difetto che chiude

`scripts/genera-fixture-canoniche.py` produce le sorgenti della matrice
cross-format con strumenti indipendenti dal prodotto, e **non gira in CI**: una
fixture rigenerata a ogni corsa renderebbe l'atteso una funzione dello
strumento del giorno, e un atteso che si aggiorna da solo non e' un atteso.

Ma se nessuno rigenera e nessuno verifica, le fixture diventano byte di cui
nessuno risponde: un'ottimizzazione di OGR, un file toccato a mano, una
rigenerazione parziale finita a meta', e i test continuano a passare su un
ingresso diverso da quello che la review ha visto. Questo gate confronta i byte
committati con i digest del registro, uno per uno.

# Perche' anche il conteggio, e non solo i digest

Un gate che verificasse **solo** i digest dei file elencati sarebbe verde su un
albero da cui una fixture e' sparita, e verde su un albero in cui ne e'
comparsa una che nessuno ha dichiarato. I due casi sono opposti e vanno visti
entrambi: qui l'insieme dei percorsi trovati deve coincidere con l'insieme
dichiarato, e la directory vuota -- il caso limite in cui **tutti** i digest
sono vacuamente soddisfatti perche' non c'e' niente da confrontare -- e' rossa
per costruzione.

# Che cosa questo gate **non** verifica

Che le fixture siano *giuste*. Dice che sono quelle di cui la review ha
risposto, non che il loro contenuto rappresenti il dataset canonico: a dirlo
saranno le conversioni che le attraversano, che non esistono ancora. Un digest
e' una firma, non una semantica, e finche' quelle conversioni non ci sono di
queste fixture e' verificata l'identita' e non il significato.

# Aggiornare una fixture

Due passi, visibili entrambi in review:

    python3 scripts/genera-fixture-canoniche.py --lavoro /tmp/lav
    python3 scripts/check-fixture-canoniche.py --mostra-manifesto

e il secondo produce il blocco `fixture` da incollare nel registro. Se il
secondo passo manca, questo gate diventa rosso -- che e' il verso giusto
dell'errore.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import sys

RADICE = pathlib.Path(__file__).resolve().parent.parent
REGISTRO = RADICE / "assurance" / "registries" / "fixture-canoniche.json"
FIXTURE = RADICE / "crates" / "plenora-io-cli" / "tests" / "fixtures" / "canoniche"


def _mostra(percorso: pathlib.Path) -> str:
    """Il percorso come si scrive in un messaggio.

    Relativo alla radice quando ci sta dentro, assoluto altrimenti: le sonde
    puntano il gate a una directory temporanea, e un `relative_to` che sollevi
    li' trasformerebbe un rosso corretto in un errore del gate.
    """
    try:
        return percorso.relative_to(RADICE).as_posix()
    except ValueError:
        return percorso.as_posix()


def _digest(percorso: pathlib.Path) -> str:
    impronta = hashlib.sha256()
    with percorso.open("rb") as sorgente:
        for blocco in iter(lambda: sorgente.read(65536), b""):
            impronta.update(blocco)
    return impronta.hexdigest()


def _trovate() -> list[pathlib.Path]:
    """I file presenti sotto la directory delle fixture, ordinati.

    Ricorsiva perche' una fixture puo' essere una **directory**: il FileGDB e'
    trentatre' file dentro `canonico.gdb/`, e trattarlo come un'unita' sola
    perderebbe esattamente il caso in cui uno dei trentatre' cambia.
    """
    if not FIXTURE.is_dir():
        return []
    return sorted(p for p in FIXTURE.rglob("*") if p.is_file())


def _relativo(percorso: pathlib.Path) -> str:
    return percorso.relative_to(FIXTURE).as_posix()


def manifesto() -> list[dict]:
    """Il manifesto **misurato dall'albero**, nella forma del registro."""
    return [
        {
            "percorso": _relativo(p),
            "byte": p.stat().st_size,
            "sha256": _digest(p),
        }
        for p in _trovate()
    ]


def _voci_ben_formate(dichiarate: object) -> tuple[dict[str, dict], list[str]]:
    errori: list[str] = []
    if not isinstance(dichiarate, list) or not dichiarate:
        return {}, [
            "`fixture` assente o vuoto. Un elenco vuoto renderebbe questo gate "
            "verde su qualunque albero, compreso uno senza fixture."
        ]
    per_percorso: dict[str, dict] = {}
    for voce in dichiarate:
        if not isinstance(voce, dict):
            errori.append(f"voce non leggibile: {voce!r}")
            continue
        percorso = voce.get("percorso")
        if not isinstance(percorso, str) or not percorso:
            errori.append(f"voce senza `percorso`: {voce!r}")
            continue
        if percorso in per_percorso:
            errori.append(f"«{percorso}»: dichiarato due volte")
        if not isinstance(voce.get("byte"), int) or voce["byte"] < 0:
            errori.append(f"«{percorso}»: `byte` non e' un conteggio")
        digest = voce.get("sha256")
        if not isinstance(digest, str) or len(digest) != 64:
            errori.append(f"«{percorso}»: `sha256` non e' un digest a 64 cifre")
        per_percorso[percorso] = voce
    return per_percorso, errori


def verifica() -> list[str]:
    if not REGISTRO.exists():
        return [f"{_mostra(REGISTRO)}: registro assente"]
    registro = json.loads(REGISTRO.read_text(encoding="utf-8"))

    errori: list[str] = []
    if registro.get("schema_version") != 1:
        errori.append(f"schema_version «{registro.get('schema_version')}»: attesa 1")

    dichiarate, malformate = _voci_ben_formate(registro.get("fixture"))
    errori.extend(malformate)
    if malformate:
        # Senza un registro ben formato il confronto direbbe che mancano tutte
        # le fixture, e nasconderebbe la causa vera dietro cinquanta righe.
        return errori

    trovate = _trovate()
    if not trovate:
        errori.append(
            f"{_mostra(FIXTURE)}: nessuna fixture sull'albero, "
            f"e il registro ne dichiara {len(dichiarate)}. Una directory vuota "
            "soddisfa ogni digest per assenza di confronti: e' il modo piu' "
            "comodo di rendere verde questo gate, ed e' chiuso qui."
        )
        return errori

    presenti = {_relativo(p): p for p in trovate}
    for percorso in sorted(set(dichiarate) - set(presenti)):
        errori.append(f"«{percorso}»: dichiarato nel registro e assente dall'albero")
    for percorso in sorted(set(presenti) - set(dichiarate)):
        errori.append(
            f"«{percorso}»: presente sull'albero e non dichiarato. Una fixture di "
            "cui nessuno ha risposto in review e' un ingresso che i test "
            "attraversano senza che si sappia da dove viene."
        )

    for percorso in sorted(set(dichiarate) & set(presenti)):
        voce = dichiarate[percorso]
        file = presenti[percorso]
        byte = file.stat().st_size
        if byte != voce["byte"]:
            errori.append(
                f"«{percorso}»: {byte} byte sull'albero, {voce['byte']} nel registro"
            )
        digest = _digest(file)
        if digest != voce["sha256"]:
            errori.append(
                f"«{percorso}»: digest diverso. Albero «{digest[:16]}…», registro "
                f"«{voce['sha256'][:16]}…». Se la fixture e' stata rigenerata, il "
                "registro va aggiornato nello stesso commit."
            )
    return errori


def main() -> int:
    a = argparse.ArgumentParser(description=__doc__)
    a.add_argument(
        "--mostra-manifesto",
        action="store_true",
        help="stampa il blocco `fixture` misurato dall'albero, da incollare nel registro",
    )
    arg = a.parse_args()

    if arg.mostra_manifesto:
        voci = manifesto()
        if not voci:
            print(
                f"ERRORE: {_mostra(FIXTURE)} non contiene fixture: "
                "non c'e' nessun manifesto da mostrare",
                file=sys.stderr,
            )
            return 1
        print(json.dumps({"fixture": voci}, indent=2, ensure_ascii=False))
        return 0

    errori = verifica()
    for messaggio in errori:
        print(f"ERRORE: {messaggio}", file=sys.stderr)
    if errori:
        return 1
    voci = manifesto()
    byte = sum(v["byte"] for v in voci)
    print(f"fixture canoniche: {len(voci)} file, {byte} byte, digest verificati uno per uno")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
