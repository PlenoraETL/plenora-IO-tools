#!/usr/bin/env python3
"""Calcolo canonico dell'impronta di un fork vendorizzato.

# Perche' l'insieme non e' «i file che ci sono»

La prima stesura hashava `rglob("*")`, cioe' tutto cio' che si trovava sul
disco. Una `cargo package` di verifica lascia `vendor/<crate>/target/` dentro
l'albero, e quegli artefatti entravano nell'impronta: il gate diventava rosso,
e un lock aggiornato in quello stato avrebbe registrato un artefatto di build
come **contenuto del fork governato**.

L'impronta e' percio' calcolata **esclusivamente sull'insieme versionato**, che
git conosce. Un artefatto di build non puo' cambiarla: al piu' viene segnalato
come estraneo, che e' un'altra cosa e va detta separatamente.

# Perche' entrambe le difese, e non una sola

Hashare il solo insieme versionato rende l'impronta **stabile**. Non basta:
un file estraneo dentro un albero governato resta un problema — puo' finire in
un pacchetto, confondere chi lo legge, o essere il residuo di un'operazione che
non doveva avvenire li'. Il gate lo **rifiuta esplicitamente** invece di
ignorarlo in silenzio.

Le due proprieta' sono indipendenti:

* l'impronta non cambia, quindi il lock non puo' essere avvelenato;
* l'estraneo viene nominato, quindi non resta invisibile.
"""

from __future__ import annotations

import hashlib
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# Un `cargo package` scrive qui se nessuno gli dice altrimenti, e finirebbe
# dentro l'albero governato.
TARGET_ESTERNO = "/tmp/plenora-fork-package"


def insieme_versionato(vendor: Path) -> list[Path]:
    """I file che git traccia sotto `vendor`, in ordine stabile."""
    relativo = vendor.relative_to(ROOT).as_posix()
    uscita = subprocess.run(
        ["git", "ls-files", "-z", "--", relativo],
        cwd=ROOT,
        capture_output=True,
        check=True,
    )
    nomi = [n for n in uscita.stdout.decode("utf-8").split("\0") if n]
    return sorted((ROOT / n for n in nomi), key=lambda p: p.relative_to(vendor).as_posix())


def impronta(vendor: Path) -> tuple[int, str]:
    """`(conteggio, sha256)` sull'insieme versionato, e su nient'altro."""
    files = insieme_versionato(vendor)
    digest = hashlib.sha256()
    for percorso in files:
        relativo = percorso.relative_to(vendor).as_posix()
        digest.update(relativo.encode("utf-8"))
        digest.update(b"\0")
        digest.update(hashlib.sha256(percorso.read_bytes()).hexdigest().encode("ascii"))
        digest.update(b"\n")
    return len(files), digest.hexdigest()


def artefatti_estranei(vendor: Path) -> list[str]:
    """File presenti sul disco ma non versionati.

    Non cambiano l'impronta — e' il punto — ma non sono ammessi: un albero
    governato deve contenere cio' che dichiara e nient'altro.
    """
    versionati = {p.resolve() for p in insieme_versionato(vendor)}
    fuori: list[str] = []
    for percorso in vendor.rglob("*"):
        if not percorso.is_file():
            continue
        if percorso.resolve() in versionati:
            continue
        fuori.append(percorso.relative_to(vendor).as_posix())
    return sorted(fuori)


def comando_package(vendor: Path, extra: list[str] | None = None) -> list[str]:
    """`cargo package` con il target **fuori** dall'albero vendorizzato.

    Non e' un consiglio: senza `--target-dir` cargo scrive dentro il fork, e
    l'operazione di verifica sporca cio' che sta verificando.
    """
    nome = vendor.name
    return [
        "cargo",
        "package",
        "--manifest-path",
        str(vendor / "Cargo.toml"),
        "--target-dir",
        f"{TARGET_ESTERNO}-{nome}",
        *(extra or []),
    ]
