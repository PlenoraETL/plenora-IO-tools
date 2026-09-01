#!/usr/bin/env python3
"""Le dipendenze Rust che finiscono **dentro il binario**.

# Perche' non basta `Cargo.lock`

`Cargo.lock` elenca tutto cio' che serve a costruire: le dipendenze di sviluppo,
quelle di build, i crate che servono soltanto ai test. Nel binario spedito non
c'e' niente di tutto questo, e un SBOM che li elencasse direbbe che spediamo
software che non spediamo -- lo stesso difetto che sul lato conda si evita
partendo dalla chiusura `DT_NEEDED` invece che dall'elenco dei pacchetti.

Si cammina quindi il grafo risolto da `cargo metadata`, partendo dal binario e
seguendo le sole dipendenze **normali**, con le feature che la costruzione
attiva davvero. Una `dev-dependency` non e' linkata; una `build-dependency`
gira a build time e non finisce nei byte spediti.

# Perche' l'SBOM ne ha bisogno

Perche' senza, l'SBOM elenca i soli componenti nativi e tace su tutto cio' che
il compilatore ha linkato staticamente. Nel profilo `base` questo significava
**un solo pacchetto** -- il runtime C -- per un binario che porta dentro
duecento crate. Il gate confrontava due elenchi entrambi incompleti, e restava
verde: e' la forma piu' comoda di un falso verde, perche' i due lati concordano
proprio in quanto sbagliano insieme.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import subprocess
import sys

RADICE = pathlib.Path(__file__).resolve().parent.parent


def grafo(profilo: str, cwd: pathlib.Path | None = None) -> dict:
    """`cargo metadata` con le feature del profilo, e `--locked`.

    `--locked` perche' un SBOM prodotto da una risoluzione diversa da quella
    che ha costruito il binario descriverebbe un altro binario.
    """
    comando = [
        "cargo", "metadata", "--format-version", "1", "--locked",
        "--filter-platform", _bersaglio(),
    ]
    if profilo == "filegdb":
        comando += ["--features", "plenora-io-cli/gdal-backend"]
    esito = subprocess.run(
        comando, capture_output=True, text=True, cwd=cwd or RADICE, check=True
    )
    return json.loads(esito.stdout)


def _bersaglio() -> str:
    """Il target host, che e' quello per cui si costruisce.

    Filtrare per piattaforma toglie dal grafo i crate che esistono solo per
    altri sistemi -- `winapi` su Linux, `nix` su Windows -- che altrimenti
    finirebbero in un SBOM di cio' che non spediamo.
    """
    esito = subprocess.run(["rustc", "-vV"], capture_output=True, text=True, check=True)
    for riga in esito.stdout.splitlines():
        if riga.startswith("host: "):
            return riga.split(": ", 1)[1].strip()
    raise SystemExit("`rustc -vV` non dichiara un host: senza, il filtro non si puo' applicare")


def linkati(metadati: dict, radice: str = "plenora-io-cli") -> list[dict]:
    """I pacchetti raggiungibili dal binario per dipendenze **normali**.

    `dep_kinds` distingue i tre tipi. Una `dev` serve ai test e una `build` gira
    al momento della costruzione: nessuna delle due finisce nei byte che si
    spediscono, e includerle gonfierebbe l'SBOM di software che chi installa non
    ha sul disco.
    """
    per_id = {p["id"]: p for p in metadati["packages"]}
    archi = {n["id"]: n for n in metadati["resolve"]["nodes"]}

    partenze = [p["id"] for p in metadati["packages"] if p["name"] == radice]
    if not partenze:
        raise SystemExit(f"il pacchetto «{radice}» non e' nel grafo")

    visti: set[str] = set()
    da_visitare = list(partenze)
    while da_visitare:
        corrente = da_visitare.pop()
        if corrente in visti:
            continue
        visti.add(corrente)
        for dipendenza in archi[corrente]["deps"]:
            tipi = {k.get("kind") for k in dipendenza.get("dep_kinds", [{}])}
            # `None` e' la dipendenza normale; `"dev"` e `"build"` no.
            if None not in tipi:
                continue
            if dipendenza["pkg"] not in visti:
                da_visitare.append(dipendenza["pkg"])

    nostri = {p["id"] for p in metadati["packages"] if p.get("source") is None}
    return sorted(
        (
            {
                "nome": per_id[i]["name"],
                "versione": per_id[i]["version"],
                "licenza": per_id[i].get("license") or "",
                "licenza_file": per_id[i].get("license_file") or "",
                "origine": per_id[i].get("source") or "questo repository",
                "nostro": i in nostri,
            }
            for i in visti
        ),
        key=lambda p: (p["nome"], p["versione"]),
    )


def main() -> int:
    a = argparse.ArgumentParser(description=__doc__)
    a.add_argument("--profilo", default="filegdb", choices=["base", "filegdb"])
    a.add_argument("--uscita", type=pathlib.Path, default=None)
    arg = a.parse_args()

    pacchetti = linkati(grafo(arg.profilo))
    di_terzi = [p for p in pacchetti if not p["nostro"]]
    senza_licenza = [p for p in di_terzi if not p["licenza"] and not p["licenza_file"]]

    print(f"profilo {arg.profilo}: {len(pacchetti)} crate linkati, {len(di_terzi)} di terzi")
    if senza_licenza:
        print(f"  senza licenza dichiarata: {[p['nome'] for p in senza_licenza]}")

    documento = {
        "profilo": arg.profilo,
        "crate_linkati": len(pacchetti),
        "di_terzi": len(di_terzi),
        "pacchetti": pacchetti,
    }
    if arg.uscita:
        arg.uscita.parent.mkdir(parents=True, exist_ok=True)
        arg.uscita.write_text(
            json.dumps(documento, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
        )
    else:
        print(json.dumps(documento, ensure_ascii=False, indent=2))
    return 1 if senza_licenza else 0


if __name__ == "__main__":
    sys.exit(main())
