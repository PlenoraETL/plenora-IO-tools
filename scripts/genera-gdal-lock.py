#!/usr/bin/env python3
"""Produce `scripts/linux-gdal-lock.json`. Si esegue **a mano**, di rado.

# Perche' e' separato dal costruttore

Questo script usa un solver e interroga il canale: e' l'unico momento in cui
farlo e' lecito. Il costruttore -- `install-linux-gdal.py` -- non ne ha uno, e
non deve averne: un artefatto la cui chiusura si ridecide a ogni costruzione
non e' fissato, e due costruzioni dello stesso commit potrebbero portare
librerie diverse senza che nessuno lo veda.

# Perche' anche il solver e' fissato

`micromamba` viene dallo stesso canale delle librerie, scaricato per URL e
verificato per sha256 prima di essere eseguito. Uno strumento che materializza
l'ambiente e che cambia da solo renderebbe non riproducibile tutto cio' che ne
esce -- e la chiusura che produce e' esattamente cio' che il lock promette di
tenere fermo.

Il pin finisce dentro il lock, sotto `risolto_con`: chi rigenera parte dagli
stessi byte, o vede subito che non lo sta facendo.

# Uso

    python3 scripts/genera-gdal-lock.py --lavoro /tmp/lock-gdal --subdir linux-64

Servono `curl`, `tar`, `bzip2` e rete. Il file prodotto va riletto da un umano
prima di essere committato: e' un contratto, non un output.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
import subprocess
import sys
from collections import defaultdict

RADICE = pathlib.Path(__file__).resolve().parent.parent
LOCK = RADICE / "scripts" / "linux-gdal-lock.json"

CANALE = "conda-forge"
# Due subdir, e non sono la stessa cosa: `SUBDIR_STRUMENTO` e' la piattaforma
# su cui gira micromamba mentre risolve -- questa macchina -- e `--subdir` e' la
# piattaforma per cui si risolve. Confonderle faceva scaricare un micromamba
# per Windows su una macchina Linux, e la confusione si vedeva soltanto perche'
# l'estrazione falliva.
SUBDIR_STRUMENTO = "linux-64"
SUBDIR_PREDEFINITO = "linux-64"

# I pacchetti virtuali con cui si risolve, dichiarati invece che ereditati.
#
# Conda deduce `__glibc`, `__osx` e `__win` dalla macchina su cui **gira il
# solver**. Risolvere per `osx-arm64` da Linux fallisce per questo -- `__osx`
# non c'e' -- ma il problema vero e' l'altro: risolvere per `linux-64` su una
# macchina con glibc 2.36 puo' dare pacchetti diversi da una con 2.35, e il
# lock non direbbe quale delle due l'ha prodotto. Un lock e' riproducibile
# soltanto se la risoluzione non dipende da chi la esegue.
#
# I valori non sono comodi: sono la **soglia dichiarata** della piattaforma,
# cioe' cio' che la macchina di destinazione deve avere. Cambiarli e' cambiare
# la promessa, e finiscono nel lock perche' chi lo rilegge sappia contro che
# cosa e' stato risolto.
VIRTUALI_PER_SUBDIR = {
    "linux-64": {
        "CONDA_OVERRIDE_GLIBC": "2.35",
    },
    "win-64": {},
    "osx-arm64": {
        "CONDA_OVERRIDE_OSX": "15.0",
    },
}
# GDAL 3.9, e non l'ultima disponibile, perche' `gdal-sys 0.10.0` spedisce
# binding pre-costruiti soltanto per 3.0-3.9. Su 3.10 la build si ferma da sola
# con «No pre-built bindings available», e le due uscite da quel vicolo sono
# entrambe peggiori di scendere di una minore.
#
# Abilitare la feature `bindgen` genererebbe i binding a build time, e non e'
# una libreria in piu': e' un **generatore di codice** che va fissato e
# qualificato insieme a cio' che gli serve -- `libclang`, gli header di GDAL, e
# la versione di clang che li interpreta. Sono tre nuovi input di costruzione,
# ciascuno capace di cambiare i binding senza che nessuna riga del repository
# cambi. I binding pre-generati tolgono quel problema alla radice: sono byte nel
# crate, gia' dentro il perimetro fissato.
#
# Dichiarare a `gdal-sys` una versione diversa da quella spedita -- che e' cio'
# che `install-windows-gdal.ps1` faceva, forzando `GDAL_VERSION=3.6.0` su una
# libreria 3.10.3 -- fa compilare binding di una ABI contro una libreria di
# un'altra. Funziona finche' funziona, e quando smette non lo dice.
#
# `BINDING_VERSION` non e' una scelta indipendente: e' la serie di
# `GDAL_VERSION`, e una sonda lo verifica.
GDAL_VERSION = "3.9.3"
PACCHETTO_RADICE = "libgdal-core"
BINDING_VERSION = ".".join(GDAL_VERSION.split(".")[:2]) + ".0"
MICROMAMBA_VERSIONE = "2.9.0"


def pin_di_micromamba(lavoro: pathlib.Path) -> dict:
    """URL, dimensione e sha256 della versione dichiarata, dal canale."""
    api = lavoro / "micromamba-api.json"
    subprocess.run(
        [
            "curl",
            "-sSL",
            "--fail",
            "-o",
            str(api),
            f"https://api.anaconda.org/package/{CANALE}/micromamba",
        ],
        check=True,
    )
    dati = json.loads(api.read_text(encoding="utf-8"))
    # `.tar.bz2` e non `.conda`: si estrae con `tar` e `bzip2`, che ogni base ha,
    # mentre `.conda` pretende `zstd` -- e a quel punto servirebbe uno strumento
    # per procurarsi lo strumento.
    candidati = [
        f
        for f in dati["files"]
        if f["attrs"].get("subdir") == SUBDIR_STRUMENTO
        and f["version"] == MICROMAMBA_VERSIONE
        and f["basename"].endswith(".tar.bz2")
    ]
    if len(candidati) != 1:
        sys.exit(
            f"micromamba {MICROMAMBA_VERSIONE}: attesa una build per {SUBDIR_STRUMENTO}, "
            f"trovate {len(candidati)}"
        )
    scelto = candidati[0]
    return {
        "strumento": "micromamba",
        "versione": scelto["version"],
        "build": scelto["attrs"].get("build"),
        "url": f"https://conda.anaconda.org/{CANALE}/{scelto['basename']}",
        "sha256": scelto["sha256"],
        "dimensione": scelto["size"],
        "nota": (
            "fissato anch'esso: uno strumento che materializza l'ambiente e che cambia da solo "
            "renderebbe non riproducibile tutto cio' che ne esce. E' stato usato **una volta**, "
            "per produrre questo file; il costruttore non lo usa."
        ),
    }


def procurati(pin: dict, lavoro: pathlib.Path) -> pathlib.Path:
    archivio = lavoro / "micromamba.tar.bz2"
    subprocess.run(["curl", "-sSL", "--fail", "-o", str(archivio), pin["url"]], check=True)
    dati = archivio.read_bytes()
    sha = hashlib.sha256(dati).hexdigest()
    if len(dati) != pin["dimensione"] or sha != pin["sha256"]:
        sys.exit(f"micromamba non corrisponde al pin: {len(dati)} byte, sha256 {sha}")
    subprocess.run(
        ["tar", "-xjf", str(archivio), "-C", str(lavoro), "bin/micromamba"], check=True
    )
    return lavoro / "bin" / "micromamba"


def soglia(vincoli: list[str]) -> str | None:
    """La versione piu' alta pretesa fra i vincoli di un requisito virtuale."""
    versioni = [
        tuple(int(x) for x in m.group(1).split("."))
        for v in vincoli
        for m in re.finditer(r">=?\s*([0-9]+(?:\.[0-9]+)*)", v)
    ]
    return ".".join(str(x) for x in max(versioni)) if versioni else None


def main() -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument("--lavoro", required=True, type=pathlib.Path)
    argomenti.add_argument(
        "--subdir",
        default=SUBDIR_PREDEFINITO,
        choices=["linux-64", "win-64", "osx-arm64"],
        help="la piattaforma per cui si risolve; non quella su cui gira il risolutore",
    )
    argomenti.add_argument(
        "--piattaforma",
        default=None,
        help="l'identita' della piattaforma nella matrice di distribuzione",
    )
    argomenti.add_argument("--uscita", default=None, help="il nome del file di lock prodotto")
    opzioni = argomenti.parse_args()
    lavoro: pathlib.Path = opzioni.lavoro
    lavoro.mkdir(parents=True, exist_ok=True)
    subdir = opzioni.subdir
    piattaforma = opzioni.piattaforma or {
        "linux-64": "linux-x86_64",
        "win-64": "windows-x86_64",
        "osx-arm64": "macos-aarch64",
    }[subdir]
    uscita_lock = opzioni.uscita or f"{piattaforma.split('-')[0]}-gdal-lock.json"

    virtuali = VIRTUALI_PER_SUBDIR[subdir]
    if virtuali:
        print(f"risoluzione con pacchetti virtuali dichiarati: {virtuali}")

    pin = pin_di_micromamba(lavoro)
    micromamba = procurati(pin, lavoro)

    risoluzione = lavoro / "risoluzione.json"
    with risoluzione.open("w", encoding="utf-8") as uscita:
        subprocess.run(
            [
                str(micromamba),
                "create",
                "--dry-run",
                "--json",
                "--yes",
                "--platform",
                subdir,
                "--prefix",
                str(lavoro / "env"),
                "--override-channels",
                "--channel",
                CANALE,
                f"{PACCHETTO_RADICE}={GDAL_VERSION}",
            ],
            check=True,
            stdout=uscita,
            env={
                "MAMBA_ROOT_PREFIX": str(lavoro / "root"),
                "PATH": "/usr/bin:/bin",
                **virtuali,
            },
        )

    fetch = json.loads(risoluzione.read_text(encoding="utf-8"))["actions"]["FETCH"]

    pacchetti = []
    for p in sorted(fetch, key=lambda x: (x["name"], x["version"])):
        if not p.get("sha256"):
            sys.exit(f"{p['name']}: senza sha256, il lock non sarebbe verificabile")
        pacchetti.append(
            {
                "nome": p["name"],
                "versione": p["version"],
                "build": p.get("build_string") or p.get("build"),
                "subdir": p.get("subdir"),
                "url": p["url"],
                "dimensione": p["size"],
                "sha256": p["sha256"],
            }
        )

    requisiti: dict[str, set[str]] = defaultdict(set)
    for p in fetch:
        for dip in p.get("depends") or []:
            if dip.split()[0].startswith("__"):
                requisiti[dip.split()[0]].add(dip.strip())

    lock = {
        "schema_version": 1,
        "descrizione": (
            "Chiusura fissata di GDAL per linux-x86_64. Il costruttore la consuma cosi' com'e': "
            "nessun solver, nessuna interrogazione del canale, nessun metadata mobile. Ogni "
            "pacchetto porta URL, dimensione e sha256, e il costruttore rifiuta cio' che non "
            "corrisponde."
        ),
        "piattaforma": piattaforma,
        "subdir": subdir,
        "canale": CANALE,
        "gdal_version": GDAL_VERSION,
        "pacchetto_radice": PACCHETTO_RADICE,
        "binding_version": BINDING_VERSION,
        "risolto_con": pin,
        "virtuali_alla_risoluzione": virtuali,
        "virtuali_alla_risoluzione_nota": (
            "i pacchetti virtuali con cui la risoluzione e' stata fatta, dichiarati invece che "
            "ereditati dalla macchina del solver. Senza, lo stesso comando su due macchine "
            "diverse puo' produrre due chiusure diverse, e il lock non direbbe quale delle due "
            "e' la sua. I valori sono la soglia dichiarata della piattaforma: cambiarli e' "
            "cambiare la promessa verso chi installa."
        ),
        "requisiti_virtuali": [
            {
                "nome": nome,
                "vincoli": sorted(requisiti[nome]),
                "minimo_richiesto": soglia(sorted(requisiti[nome])),
            }
            for nome in sorted(requisiti)
        ],
        "requisiti_virtuali_nota": (
            "le dipendenze virtuali di Conda non sono pacchetti scaricabili: sono condizioni sul "
            "sistema di destinazione. Sono qui perche' il costruttore le verifichi e perche' la "
            "soglia glibc dichiarata nella matrice sia confrontabile con quella che la chiusura "
            "pretende davvero."
        ),
        "pacchetti": pacchetti,
    }

    esistente = json.loads(LOCK.read_text(encoding="utf-8")) if LOCK.exists() else {}
    if "misure_alla_creazione" in esistente:
        # Le misure sono di **quella** chiusura: rigenerandola vanno rifatte, e
        # tenerle sarebbe attribuire a byte nuovi una verifica vecchia.
        print(
            "attenzione: il lock esistente porta `misure_alla_creazione`. "
            "Se la chiusura cambia, le misure vanno rifatte con "
            "`check-linux-gdal-runtime.py` e la sonda di OpenFileGDB.",
            file=sys.stderr,
        )

    (lavoro / uscita_lock).write_text(
        json.dumps(lock, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    print(f"lock prodotto in {lavoro / uscita_lock}: {len(pacchetti)} pacchetti")
    for r in lock["requisiti_virtuali"]:
        print(f"  requisito virtuale {r['nome']}: minimo {r['minimo_richiesto']}")
    print("rileggilo prima di committarlo: e' un contratto, non un output.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
