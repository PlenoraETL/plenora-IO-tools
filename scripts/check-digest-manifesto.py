#!/usr/bin/env python3
"""Ricalcola i digest del manifesto sull'albero **estratto**.

# Il difetto che chiude

Il manifesto porta un digest per ogni file spedito. Erano scritti e nessuno li
rileggeva: un digest che nessuno verifica e' un numero, non una garanzia -- e
per giunta una garanzia apparente, perche' chi legge il manifesto suppone che
qualcuno l'abbia controllata.

La verifica va fatta sull'albero **estratto dall'archivio**, non su quello che
il costruttore ha appena scritto. Fra i due c'e' l'archiviazione, il trasporto
e l'estrazione, e sono esattamente i passaggi in cui un file puo' cambiare o
sparire senza che nessuno lo noti: verificare l'albero di partenza direbbe
soltanto che il costruttore sa calcolare uno sha256.

# Le tre domande

1. Ogni file dichiarato nel manifesto **c'e'**, e il suo digest corrisponde.
2. Nessun file in piu': un albero che contiene qualcosa che il manifesto non
   dichiara e' un albero di cui non si sa tutto.
3. Il manifesto non dichiara zero file, che sarebbe un modo di superare le
   prime due senza guardare niente.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import distribuzione  # noqa: E402 -- dopo sys.path, che e' il punto


def sha256(percorso: pathlib.Path) -> str:
    digesto = hashlib.sha256()
    with percorso.open("rb") as f:
        for blocco in iter(lambda: f.read(1 << 20), b""):
            digesto.update(blocco)
    return digesto.hexdigest()


def verifica(albero: pathlib.Path) -> tuple[list[str], dict]:
    manifesto_percorso = albero / "MANIFEST.json"
    if not manifesto_percorso.is_file():
        return ([f"{manifesto_percorso} assente"], {})
    manifesto = json.loads(manifesto_percorso.read_text(encoding="utf-8"))

    dichiarati = manifesto.get("file") or []
    errori: list[str] = []
    if not dichiarati:
        return (
            [
                "il manifesto non dichiara nessun file. Un elenco vuoto supera ogni confronto "
                "senza guardare niente, ed e' il modo piu' comodo di rendere verde questo "
                "controllo."
            ],
            {"file_dichiarati": 0},
        )

    # Il manifesto puo' portare i file come stringhe (forma vecchia) o come
    # voci con il proprio digest. Solo la seconda si puo' verificare, e il
    # controllo lo dice invece di accontentarsi.
    if isinstance(dichiarati[0], str):
        return (
            [
                "il manifesto elenca i file come nomi, senza digest. Un elenco di nomi dice che "
                "cosa c'era, non che cosa c'e': non si puo' verificare, e questo controllo non "
                "puo' fingere di averlo fatto."
            ],
            {"file_dichiarati": len(dichiarati)},
        )

    attesi = {v["percorso"].replace("\\", "/"): v for v in dichiarati}
    presenti = {
        str(p.relative_to(albero)).replace("\\", "/"): p
        for p in albero.rglob("*")
        if p.is_file() and p.name != "MANIFEST.json"
    }

    mancanti = sorted(set(attesi) - set(presenti))
    se_ne_stanno_in_piu = sorted(set(presenti) - set(attesi))
    if mancanti:
        errori.append(
            f"{len(mancanti)} file dichiarati e assenti dall'albero estratto: {mancanti[:6]}"
        )
    if se_ne_stanno_in_piu:
        errori.append(
            f"{len(se_ne_stanno_in_piu)} file presenti e non dichiarati: "
            f"{se_ne_stanno_in_piu[:6]}. Un albero che contiene qualcosa che il manifesto non "
            "nomina e' un albero di cui non si sa tutto."
        )

    divergenti = []
    for relativo, voce in sorted(attesi.items()):
        percorso = presenti.get(relativo)
        if percorso is None:
            continue
        calcolato = sha256(percorso)
        if calcolato != voce["sha256"]:
            divergenti.append(f"{relativo}: {calcolato[:16]}… invece di {voce['sha256'][:16]}…")
        elif voce.get("byte") is not None and percorso.stat().st_size != voce["byte"]:
            divergenti.append(f"{relativo}: {percorso.stat().st_size} byte invece di {voce['byte']}")
    if divergenti:
        errori.append(f"{len(divergenti)} file con digest o dimensione diversi: {divergenti[:6]}")

    misure = {
        "file_dichiarati": len(attesi),
        "file_verificati": len(attesi) - len(mancanti),
        "file_non_dichiarati": len(se_ne_stanno_in_piu),
        "digest_divergenti": len(divergenti),
    }
    return errori, misure


def main() -> int:
    a = argparse.ArgumentParser(description=__doc__)
    a.add_argument("--albero", required=True, type=pathlib.Path)
    a.add_argument("--referto", type=pathlib.Path, default=None)
    arg = a.parse_args()

    albero = arg.albero.resolve()
    if not albero.is_dir():
        sys.exit(f"{albero} non e' una directory")

    errori, misure = verifica(albero)
    for chiave, valore in sorted(misure.items()):
        print(f"  {chiave}: {valore}")

    if arg.referto and (albero / "MANIFEST.json").is_file():
        manifesto = json.loads((albero / "MANIFEST.json").read_text(encoding="utf-8"))
        distribuzione.scrivi_referto(
            arg.referto,
            verifica="digest-manifesto",
            piattaforma=manifesto["piattaforma"],
            profilo=manifesto["profilo"],
            canale=manifesto["canale"],
            esito="verde" if not errori else "rosso",
            misure=misure,
            errori=errori,
            note=(
                "i digest sono ricalcolati sull'albero **estratto dall'archivio**: fra il "
                "manifesto e chi riceve ci sono l'archiviazione, il trasporto e l'estrazione, e "
                "sono i passaggi in cui un file puo' cambiare senza che nessuno lo noti."
            ),
        )

    if errori:
        print("\n--- ROSSO ---")
        for errore in errori:
            print(f"  {errore}")
        return 1
    print("ogni file dichiarato c'e', e il suo digest corrisponde")
    return 0


if __name__ == "__main__":
    sys.exit(main())
