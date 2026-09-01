#!/usr/bin/env python3
"""Il gate che rifiuta un artefatto la cui `LICENSES/` non e' completa.

# Che cosa pretende

Che ogni componente che mette **byte** nell'artefatto abbia accanto il testo
della propria licenza. Non il nome, non l'identificatore: il testo. Un elenco di
licenze non e' cio' che una licenza obbliga a distribuire, e un artefatto con un
elenco al posto dei testi e' un artefatto che non si puo' consegnare.

# Perche' e' un gate e non una verifica dentro il costruttore

Il costruttore gia' si ferma se non riesce a procurarsi un testo. Ma il
costruttore verifica cio' che sta **facendo**, e questo verifica cio' che
**c'e'**: fra i due momenti l'albero puo' essere stato assemblato da una
versione precedente, estratto e rimpacchettato, o modificato. Le due domande si
somigliano e non sono la stessa, e la seconda e' quella che riguarda chi
riceve.

# I metapacchetti

Un pacchetto che non spedisce byte non compare fra i componenti, e non gli si
chiede niente: non c'e' nulla da licenziare. La distinzione non e' un'esenzione
ma il criterio stesso -- «ha messo un file in questo albero» -- e una sonda
verifica che resti tale, perche' altrimenti basterebbe declassare un componente
a metapacchetto per farlo tacere.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import sys


def verifica(albero: pathlib.Path) -> list[str]:
    errori: list[str] = []

    manifesto_percorso = albero / "MANIFEST.json"
    sbom_percorso = albero / "SBOM.spdx.json"
    provenienza_percorso = albero / "LICENSES" / "PROVENIENZA.json"
    for percorso in (manifesto_percorso, sbom_percorso, provenienza_percorso):
        if not percorso.is_file():
            return [f"{percorso.relative_to(albero)} assente: l'artefatto non e' completo"]

    manifesto = json.loads(manifesto_percorso.read_text(encoding="utf-8"))
    sbom = json.loads(sbom_percorso.read_text(encoding="utf-8"))
    provenienza = json.loads(provenienza_percorso.read_text(encoding="utf-8"))

    nel_sbom = {p["name"] for p in sbom["packages"]}
    nella_provenienza = {p["nome"] for p in provenienza["pacchetti"]}
    if nel_sbom != nella_provenienza:
        errori.append(
            "SBOM e PROVENIENZA non elencano gli stessi componenti. "
            f"Solo nel SBOM: {sorted(nel_sbom - nella_provenienza)}. "
            f"Solo nella provenienza: {sorted(nella_provenienza - nel_sbom)}. "
            "Sono due viste della stessa cosa, e divergono soltanto se una delle due mente."
        )

    # Il cuore: ogni componente ha un testo, e il testo ha dentro qualcosa.
    for nome in sorted(nel_sbom | nella_provenienza):
        directory = albero / "LICENSES" / nome
        if not directory.is_dir():
            errori.append(
                f"{nome}: nessuna directory in LICENSES/. Mette byte nell'artefatto e non "
                "porta il testo della propria licenza."
            )
            continue
        testi = [f for f in sorted(directory.rglob("*")) if f.is_file()]
        if not testi:
            errori.append(f"{nome}: LICENSES/{nome}/ e' vuota.")
            continue
        vuoti = [f.name for f in testi if f.stat().st_size == 0]
        if vuoti:
            errori.append(
                f"{nome}: testi vuoti: {vuoti}. Un file di zero byte supera «esiste» e non "
                "consegna niente."
            )

    licenze = manifesto.get("licenze", {})
    if licenze.get("senza_testo", 0) != 0:
        errori.append(
            f"il manifesto dichiara {licenze['senza_testo']} componenti senza testo. "
            "Dichiararlo evita il silenzio, ma non consegna la licenza."
        )

    # Il conto dichiarato e quello trovato devono coincidere: se il manifesto
    # dicesse quaranta e in LICENSES/ ce ne fossero trenta, ognuna delle due
    # verifiche sopra potrebbe restare verde su cio' che guarda.
    dichiarati = licenze.get("con_testo_proprio", 0) + licenze.get("con_testo_canonico", 0)
    trovati = len([d for d in (albero / "LICENSES").iterdir() if d.is_dir()])
    if dichiarati != trovati:
        errori.append(
            f"il manifesto dichiara {dichiarati} componenti con testo e in LICENSES/ ce ne "
            f"sono {trovati}."
        )

    return errori


def main() -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument("--albero", required=True, type=pathlib.Path)
    opzioni = argomenti.parse_args()
    albero = opzioni.albero.resolve()
    if not albero.is_dir():
        sys.exit(f"{albero} non e' una directory")

    errori = verifica(albero)
    componenti = len(
        [d for d in (albero / "LICENSES").iterdir() if d.is_dir()]
    ) if (albero / "LICENSES").is_dir() else 0
    print(f"componenti con testo di licenza: {componenti}")
    if errori:
        print("\n--- ROSSO ---")
        for errore in errori:
            print(f"  {errore}")
        return 1
    print("ogni componente che spedisce byte porta il testo della propria licenza")
    return 0


if __name__ == "__main__":
    sys.exit(main())
