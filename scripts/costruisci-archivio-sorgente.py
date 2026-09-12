#!/usr/bin/env python3
"""L'archivio sorgente del workspace Rust, con versione e digest.

# Perche' un archivio e non crates.io

Il perimetro della 4.0.0 lo dice: il workspace Rust si distribuisce come
archivio sorgente, e crates.io non e' richiesto. Le due strade non sono
equivalenti e la scelta non e' di comodo: pubblicare undici crate su un indice
pubblico vorrebbe dire promettere semver su ciascuno, mentre di pubblico c'e'
una superficie sola -- le sei operazioni -- e gli altri dieci crate sono il
confine plug-in dei driver, che resta interno.

Un archivio con un digest dice esattamente quel che serve: **questi byte** sono
la revisione da cui quella superficie e' stata verificata.

# Perche' `git archive` e non un tar a mano

Perche' il contenuto dev'essere derivabile dalla revisione e non dallo stato
della directory di lavoro. `git archive` prende l'albero di un commit, rispetta
`.gitattributes` e non vede i file ignorati: un archivio costruito con `tar`
avrebbe incluso `target/`, `.plenora-contracts/` e qualunque cosa fosse rimasta
in giro, e il digest avrebbe misurato la macchina invece della revisione.

# Perche' l'archivio e' riproducibile

Due ragioni, e la seconda l'ho scoperta sbagliando. `git archive` di un
**commit** -- non di un albero -- prende i tempi di modifica dalla data del
commit: due archivi della stessa revisione hanno gli stessi byte del tar. Non
serve un `--mtime`, che in git 2.39 non esiste nemmeno.

Ma `--format=tar.gz` non basta: la busta gzip porta un timestamp suo, e due
compressioni dello stesso tar darebbero digest diversi. Quindi si produce il
tar e lo si comprime con `gzip -n`, che quel campo non lo scrive. Un digest che
cambia a ogni corsa non identifica una revisione: misura l'orologio.

# Che cosa l'archivio contiene, e che cosa no

Contiene i crate, i vendor governati, `Cargo.lock` e `Cargo.toml`: tutto cio'
che serve a compilare la superficie. Non contiene le fixture canoniche del CLI
ne' i corpora di fuzzing, che pesano e servono alle nostre prove, non a chi
consuma -- e che restano verificabili nel repository.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]

#: Cio' che l'archivio **non** porta.
#:
#: La regola e' precisa e l'ho imparata sbagliando: l'archivio deve contenere
#: tutto cio' che serve a compilare i **target libreria**, e puo' lasciare fuori
#: cio' che serve solo alle prove. Non e' la stessa cosa di «le nostre prove
#: pesano», che era la regola che avevo scritto: `assurance/` era escluso e
#: `driver-geoparquet` ne compila dentro quattro schemi con `include_str!`, per
#: cui l'archivio non compilava affatto. Nessuna prova interna poteva dirlo --
#: dentro il repository quei file ci sono sempre.
#:
#: Cio' che resta fuori e' quindi solo cio' che nessun target libreria include:
#: le fixture dei test d'integrazione e i corpora di fuzzing. Un consumatore
#: puo' compilare e usare la superficie; **non** puo' eseguire le suite di
#: prova, e questa e' la conseguenza che va detta invece di scoprirla.
ESCLUSI = (
    "crates/plenora-io-cli/tests/fixtures",
    "crates/driver-filegdb/tests",
    "fuzz/seeds",
    "fuzz/fixtures",
    "fuzz/corpus",
    ".s9-checkpoint",
)


def versione() -> str:
    """La versione del workspace, dal manifesto e non da un argomento."""
    manifesto = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    trovata = re.search(r'^version\s*=\s*"([^"]+)"', manifesto, re.M)
    if trovata is None:
        raise SystemExit("Cargo.toml: nessuna versione del workspace")
    return trovata.group(1)


def revisione() -> str:
    return subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()


def costruisci(uscita: pathlib.Path) -> dict[str, object]:
    sha = revisione()
    ver = versione()
    uscita.mkdir(parents=True, exist_ok=True)
    archivio = uscita / f"plenora-io-{ver}-src.tar.gz"

    comando = [
        "git",
        "archive",
        "--format=tar",
        f"--prefix=plenora-io-{ver}/",
        sha,
    ]
    # I percorsi esclusi si passano come pathspec negativi: `git archive` non
    # ha un `--exclude`, e filtrare dopo vorrebbe dire riscrivere il tar.
    comando.extend(f":(exclude){percorso}" for percorso in ESCLUSI)
    tar = subprocess.run(comando, cwd=ROOT, capture_output=True, check=True).stdout
    # `-n`: nessun nome e nessun timestamp nella busta gzip. Vedi la nota sulla
    # riproducibilita' in testa al modulo.
    compresso = subprocess.run(
        ["gzip", "-n", "-9"], input=tar, capture_output=True, check=True
    ).stdout
    archivio.write_bytes(compresso)

    byte = archivio.read_bytes()
    digest = hashlib.sha256(byte).hexdigest()
    (uscita / f"{archivio.name}.sha256").write_text(
        f"{digest}  {archivio.name}\n", encoding="utf-8"
    )

    manifesto = {
        "component": "plenora-io-tools",
        "artefatto": archivio.name,
        "versione": ver,
        "revisione": sha,
        "byte": len(byte),
        "sha256": digest,
        "esclusi": list(ESCLUSI),
        "che_cosa_identifica": (
            "la revisione da cui la superficie Rust pubblica e' stata "
            "verificata. Il digest e' sui byte dell'archivio, non sull'albero: "
            "due archivi della stessa revisione hanno lo stesso digest perche' "
            "`git archive` di un commit prende i tempi da lui e la compressione "
            "non scrive il proprio."
        ),
    }
    (uscita / "MANIFEST-sorgente.json").write_text(
        json.dumps(manifesto, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    return manifesto


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--uscita", type=pathlib.Path, required=True)
    opzioni = parser.parse_args()

    manifesto = costruisci(opzioni.uscita)
    print(
        f"{manifesto['artefatto']}: {manifesto['byte']} byte, "
        f"sha256={manifesto['sha256'][:16]}…, revisione {manifesto['revisione'][:7]}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
