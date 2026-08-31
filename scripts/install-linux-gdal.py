#!/usr/bin/env python3
"""Materializza il runtime GDAL di Linux dal lock, senza solver.

# Perche' passa da micromamba e non da `tar`

La prima stesura estraeva i pacchetti a mano. Sembrava piu' semplice e aveva
due difetti che una revisione ha trovato.

Il primo: **le rilocazioni di conda non venivano applicate**. Un pacchetto
conda dichiara in `info/paths.json` quali file portano il prefisso di
costruzione e vanno riscritti all'installazione. Senza quella riscrittura i
binari restano con un placeholder, e il placeholder non e' una stringa inerte:
in `libgdal` copre `share/gdal` e `lib/gdalplugins`, cioe' **dati e plugin**.
L'RPATH non li riguarda -- prova la risoluzione delle librerie, non quella dei
dati -- e ritenerli innocui perche' l'RPATH e' relativo era un falso verde.

Il secondo: **l'ordine di estrazione**. Cinquantotto pacchetti estratti in
ordine alfabetico possono sovrascriversi, e quell'ordine non e' quello che
conda deciderebbe. Un file vinto dal pacchetto sbagliato non si vede.

micromamba in modalita' **esplicita** risolve entrambi: applica le rilocazioni
come farebbe conda, rispetta il proprio ordine di link, e gestisce le
collisioni. E non risolve niente: la lista esplicita porta gli URL del lock,
uno per riga, ciascuno con il proprio sha256. Nessun solver, nessuna
interrogazione del canale, nessun metadata mobile.

# La verifica resta nostra

Ogni pacchetto e' scaricato e confrontato con il lock **prima** che micromamba
lo veda. Che poi anche conda controlli lo sha256 e' una seconda rete, non la
prima: un artefatto costruito su un byte diverso da quello fissato non e'
l'artefatto che il lock descrive, e a dirlo dev'essere il nostro controllo.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import subprocess
import sys
import tarfile

RADICE = pathlib.Path(__file__).resolve().parent.parent
LOCK = RADICE / "scripts" / "linux-gdal-lock.json"


def procurati_micromamba(pin: dict, lavoro: pathlib.Path) -> pathlib.Path:
    """Lo strumento, se e solo se e' quello che il lock nomina."""
    archivio = lavoro / "micromamba.tar.bz2"
    if not archivio.exists():
        subprocess.run(["curl", "-sSL", "--fail", "-o", str(archivio), pin["url"]], check=True)
    dati = archivio.read_bytes()
    sha = hashlib.sha256(dati).hexdigest()
    if len(dati) != pin["dimensione"] or sha != pin["sha256"]:
        archivio.unlink(missing_ok=True)
        sys.exit(
            f"micromamba non corrisponde al pin del lock: {len(dati)} byte, sha256 {sha}. "
            f"Attesi {pin['dimensione']} byte, sha256 {pin['sha256']}."
        )
    with tarfile.open(archivio, "r:bz2") as tf:
        tf.extract("bin/micromamba", lavoro, filter="tar")
    binario = lavoro / "bin" / "micromamba"
    binario.chmod(0o755)
    return binario


def scarica_e_verifica(pacchetto: dict, cache: pathlib.Path) -> None:
    destinazione = cache / pacchetto["url"].rsplit("/", 1)[-1]
    if not destinazione.exists():
        subprocess.run(
            ["curl", "-sSL", "--fail", "-o", str(destinazione), pacchetto["url"]], check=True
        )
    dati = destinazione.read_bytes()
    sha = hashlib.sha256(dati).hexdigest()
    if len(dati) != pacchetto["dimensione"] or sha != pacchetto["sha256"]:
        # Rimosso, non lasciato: una cache avvelenata farebbe fallire ogni
        # corsa successiva senza dire perche'.
        destinazione.unlink(missing_ok=True)
        sys.exit(
            f"{pacchetto['nome']}: il file non corrisponde al lock "
            f"({len(dati)} byte, sha256 {sha}). Attesi {pacchetto['dimensione']} byte, "
            f"sha256 {pacchetto['sha256']}."
        )


def main() -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument("--prefisso", required=True, type=pathlib.Path)
    argomenti.add_argument("--lavoro", required=True, type=pathlib.Path)
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

    lavoro: pathlib.Path = opzioni.lavoro
    cache = lavoro / "pacchetti"
    cache.mkdir(parents=True, exist_ok=True)

    micromamba = procurati_micromamba(lock["risolto_con"], lavoro)

    for pacchetto in lock["pacchetti"]:
        scarica_e_verifica(pacchetto, cache)

    esplicito = lavoro / "esplicito.txt"
    esplicito.write_text(
        "@EXPLICIT\n" + "".join(f"{p['url']}#{p['sha256']}\n" for p in lock["pacchetti"]),
        encoding="utf-8",
    )

    subprocess.run(
        [
            str(micromamba),
            "create",
            "--yes",
            "--prefix",
            str(prefisso),
            "--file",
            str(esplicito),
        ],
        check=True,
        env={
            "MAMBA_ROOT_PREFIX": str(lavoro / "root"),
            "CONDA_PKGS_DIRS": str(cache),
            "PATH": "/usr/bin:/bin",
            "HOME": str(lavoro),
        },
        stdout=subprocess.DEVNULL,
    )

    print(
        f"materializzati {len(lock['pacchetti'])} pacchetti in {prefisso}: "
        "verificati sul lock, rilocati da conda, nessun solver consultato"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
