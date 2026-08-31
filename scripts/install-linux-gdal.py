#!/usr/bin/env python3
"""Materializza il runtime GDAL di Linux dal lock, senza solver.

# Che cosa non fa

Non risolve. Non interroga il canale. Non legge metadata mobili. Prende
`scripts/linux-gdal-lock.json`, scarica gli URL che vi stanno scritti, e
**verifica dimensione e sha256 prima di aprire qualunque cosa**. Un pacchetto
che non corrisponde ferma la costruzione: un artefatto costruito su un byte
diverso da quello fissato non e' l'artefatto che il lock descrive.

La differenza fra questo script e quello che ha *prodotto* il lock e' il punto:
il produttore ha usato un solver, una volta, ed e' registrato dentro il lock
insieme al proprio sha256. Il costruttore non ne ha uno.

# Che cosa lascia dietro

Il prefisso estratto e `rilocazioni.json`, l'elenco dei file che conda
riscriverebbe all'installazione. Serve a `check-linux-gdal-runtime.py`, che
decide se qualcosa di quel prefisso finirebbe nell'artefatto in una forma che
il codice legge.
"""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import pathlib
import subprocess
import sys
import tarfile
import zipfile

RADICE = pathlib.Path(__file__).resolve().parent.parent
LOCK = RADICE / "scripts" / "linux-gdal-lock.json"


def scarica_e_verifica(pacchetto: dict, cache: pathlib.Path) -> pathlib.Path:
    """Il file, se e solo se e' quello che il lock nomina."""
    destinazione = cache / pacchetto["url"].rsplit("/", 1)[-1]
    if not destinazione.exists():
        subprocess.run(
            ["curl", "-sSL", "--fail", "-o", str(destinazione), pacchetto["url"]], check=True
        )
    dati = destinazione.read_bytes()
    sha = hashlib.sha256(dati).hexdigest()
    if len(dati) != pacchetto["dimensione"] or sha != pacchetto["sha256"]:
        # Il file rimosso, non lasciato in cache: una cache avvelenata farebbe
        # fallire ogni corsa successiva senza dire perche'.
        destinazione.unlink(missing_ok=True)
        sys.exit(
            f"{pacchetto['nome']}: il file non corrisponde al lock "
            f"({len(dati)} byte, sha256 {sha}). Atteso {pacchetto['dimensione']} byte, "
            f"sha256 {pacchetto['sha256']}."
        )
    return destinazione


def estrai(archivio: pathlib.Path, dove: pathlib.Path) -> dict:
    """Estrae un `.conda` e restituisce il suo `info/paths.json`.

    Un `.conda` e' uno zip con dentro due tarball zstd: `info-*` con i metadati
    e `pkg-*` con i file. `info/paths.json` dichiara quali file conda
    riscriverebbe e con quale placeholder.
    """
    paths: dict = {}
    with zipfile.ZipFile(archivio) as zf:
        for nome in zf.namelist():
            if not nome.endswith(".tar.zst"):
                continue
            # `zf.open` da' un flusso senza descrittore, che `subprocess` non sa
            # passare: i byte si leggono e si consegnano a `zstd` interi.
            decompresso = subprocess.run(
                ["zstd", "-d", "-c"], input=zf.read(nome), stdout=subprocess.PIPE, check=True
            ).stdout
            with tarfile.open(fileobj=io.BytesIO(decompresso)) as tf:
                if nome.startswith("info-"):
                    membro = tf.extractfile("info/paths.json")
                    if membro is not None:
                        paths = json.load(membro)
                else:
                    tf.extractall(dove, filter="tar")
    return paths


def main() -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument("--prefisso", required=True, type=pathlib.Path)
    argomenti.add_argument("--cache", required=True, type=pathlib.Path)
    opzioni = argomenti.parse_args()

    lock = json.loads(LOCK.read_text(encoding="utf-8"))
    if lock["schema_version"] != 1:
        sys.exit(f"lock con schema {lock['schema_version']}, non supportato")

    prefisso: pathlib.Path = opzioni.prefisso
    if prefisso.exists() and any(prefisso.iterdir()):
        # La stessa regola dello script Windows: un prefisso gia' abitato
        # mescolerebbe due materializzazioni, e la seconda erediterebbe i file
        # della prima senza che nessuno lo veda.
        sys.exit(f"il prefisso deve essere assente o vuoto: {prefisso}")
    prefisso.mkdir(parents=True, exist_ok=True)
    opzioni.cache.mkdir(parents=True, exist_ok=True)

    rilocazioni = []
    for pacchetto in lock["pacchetti"]:
        archivio = scarica_e_verifica(pacchetto, opzioni.cache)
        for voce in estrai(archivio, prefisso).get("paths", []):
            if voce.get("prefix_placeholder"):
                rilocazioni.append(
                    {
                        "pacchetto": pacchetto["nome"],
                        "file": voce["_path"],
                        "placeholder": voce["prefix_placeholder"],
                        "modo": voce.get("file_mode"),
                    }
                )

    (prefisso / "rilocazioni.json").write_text(
        json.dumps(rilocazioni, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    print(
        f"materializzati {len(lock['pacchetti'])} pacchetti da {lock['canale']}, "
        f"tutti verificati sul lock; {len(rilocazioni)} file con placeholder registrati"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
