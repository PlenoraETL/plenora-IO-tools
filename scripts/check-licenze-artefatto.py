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

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import distribuzione  # noqa: E402 -- dopo sys.path, che e' il punto


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

    # L'SBOM dev'essere leggibile come SPDX 2.3, e non solo esistere.
    try:
        distribuzione.valida_spdx(sbom)
    except distribuzione.SpdxNonValido as e:
        errori.append(f"SBOM non valido: {e}")

    # --- i componenti nativi ---------------------------------------------
    #
    # Sono quelli che mettono **file** nell'albero, e portano il proprio testo
    # in `LICENSES/<nome>/`.
    nativi_nel_sbom = {
        p["name"] for p in sbom["packages"] if "nativo" in (p.get("comment") or "")
    }
    nella_provenienza = {p["nome"] for p in provenienza["pacchetti"]}
    if nativi_nel_sbom != nella_provenienza:
        errori.append(
            "SBOM e PROVENIENZA non elencano gli stessi componenti nativi. "
            f"Solo nel SBOM: {sorted(nativi_nel_sbom - nella_provenienza)}. "
            f"Solo nella provenienza: {sorted(nella_provenienza - nativi_nel_sbom)}. "
            "Sono due viste della stessa cosa, e divergono soltanto se una delle due mente."
        )

    for nome in sorted(nativi_nel_sbom | nella_provenienza):
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

    # --- i crate Rust -----------------------------------------------------
    #
    # Il difetto che questa parte chiude: l'SBOM elencava i soli componenti
    # nativi, e il gate confrontava **due elenchi entrambi incompleti**. Restava
    # verde perche' i due lati concordavano proprio in quanto sbagliavano
    # insieme -- la forma piu' comoda di un falso verde.
    #
    # I crate non mettono file nell'albero: il compilatore li linka dentro il
    # binario. Ma sono sul disco di chi installa, e un SBOM esiste per dirglielo.
    crate_nel_sbom = {
        p["name"] for p in sbom["packages"] if "crate" in (p.get("comment") or "")
    }
    elenco_crate = albero / "LICENSES" / "crate-rust" / "CRATE.json"
    if not elenco_crate.is_file():
        if crate_nel_sbom:
            errori.append(
                f"l'SBOM elenca {len(crate_nel_sbom)} crate Rust e "
                "`LICENSES/crate-rust/CRATE.json` non c'e': i loro testi di licenza non sono "
                "stati consegnati."
            )
        else:
            errori.append(
                "nessun crate Rust nell'SBOM. Un binario Rust ne linka dentro centinaia, e un "
                "SBOM che non li nomina descrive un artefatto diverso da quello che si spedisce."
            )
    else:
        dichiarati = json.loads(elenco_crate.read_text(encoding="utf-8"))
        nomi_dichiarati = {c["nome"] for c in dichiarati["pacchetti"]}
        if nomi_dichiarati != crate_nel_sbom:
            errori.append(
                "SBOM e CRATE.json non elencano gli stessi crate. "
                f"Solo nel SBOM: {sorted(crate_nel_sbom - nomi_dichiarati)[:8]}. "
                f"Solo in CRATE.json: {sorted(nomi_dichiarati - crate_nel_sbom)[:8]}."
            )
        # Ogni identificatore dichiarato ha il proprio testo, e il testo ha
        # dentro qualcosa.
        for identificatore in dichiarati["identificatori"]:
            testo = albero / "LICENSES" / "crate-rust" / f"{identificatore}.txt"
            if not testo.is_file():
                errori.append(f"crate-rust: manca il testo di «{identificatore}»")
            elif testo.stat().st_size == 0:
                errori.append(f"crate-rust: il testo di «{identificatore}» e' vuoto")
        # E ogni licenza dichiarata da un crate e' coperta da un identificatore
        # consegnato: un crate `MIT OR Apache-2.0` vuole **entrambi** i testi,
        # perche' e' chi riceve a scegliere.
        coperti = set(dichiarati["identificatori"])
        scoperti: dict[str, list[str]] = {}
        for componente in dichiarati["pacchetti"]:
            for identificatore in distribuzione.identificatori_di(componente["licenza"]):
                if identificatore not in coperti:
                    scoperti.setdefault(identificatore, []).append(componente["nome"])
        if scoperti:
            errori.append(
                "identificatori dichiarati da un crate e non consegnati: "
                + ", ".join(f"{i} ({', '.join(n[:3])})" for i, n in sorted(scoperti.items()))
            )

    licenze = manifesto.get("licenze", {})
    if licenze.get("senza_testo", 0) != 0:
        errori.append(
            f"il manifesto dichiara {licenze['senza_testo']} componenti senza testo. "
            "Dichiararlo evita il silenzio, ma non consegna la licenza."
        )

    dichiarati_nel_manifesto = licenze.get("con_testo_proprio", 0) + licenze.get(
        "con_testo_canonico", 0
    )
    trovati = len(
        [d for d in (albero / "LICENSES").iterdir() if d.is_dir() and d.name != "crate-rust"]
    )
    if dichiarati_nel_manifesto != trovati:
        errori.append(
            f"il manifesto dichiara {dichiarati_nel_manifesto} componenti nativi con testo e in "
            f"LICENSES/ ce ne sono {trovati}."
        )

    return errori


def main() -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument("--albero", required=True, type=pathlib.Path)
    argomenti.add_argument("--referto", type=pathlib.Path, default=None)
    opzioni = argomenti.parse_args()
    albero = opzioni.albero.resolve()
    if not albero.is_dir():
        sys.exit(f"{albero} non e' una directory")

    errori = verifica(albero)
    componenti = len(
        [d for d in (albero / "LICENSES").iterdir() if d.is_dir()]
    ) if (albero / "LICENSES").is_dir() else 0
    print(f"componenti con testo di licenza: {componenti}")

    if opzioni.referto:
        manifesto = json.loads((albero / "MANIFEST.json").read_text(encoding="utf-8"))
        distribuzione.scrivi_referto(
            opzioni.referto,
            verifica="licenze-artefatto",
            piattaforma=manifesto["piattaforma"],
            profilo=manifesto["profilo"],
            canale=manifesto["canale"],
            esito="verde" if not errori else "rosso",
            misure={
                "componenti_con_testo": componenti,
                "crate_rust": len(
                    [
                        p
                        for p in json.loads(
                            (albero / "SBOM.spdx.json").read_text(encoding="utf-8")
                        )["packages"]
                        if "crate" in (p.get("comment") or "")
                    ]
                ),
                **manifesto.get("licenze", {}),
            },
            errori=errori,
        )
    if errori:
        print("\n--- ROSSO ---")
        for errore in errori:
            print(f"  {errore}")
        return 1
    print("ogni componente che spedisce byte porta il testo della propria licenza")
    return 0


if __name__ == "__main__":
    sys.exit(main())
