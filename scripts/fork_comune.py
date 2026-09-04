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


def _oid(files: list[Path], filtri: bool) -> list[str]:
    """Gli oid che git assegnerebbe ai file, con o senza i filtri di `.gitattributes`."""
    argomenti = ["git", "hash-object"]
    if not filtri:
        argomenti.append("--no-filters")
    uscita = subprocess.run(
        [*argomenti, "--", *(str(p) for p in files)],
        cwd=ROOT,
        capture_output=True,
        check=True,
    )
    return uscita.stdout.decode("ascii").split()


def fini_riga_divergenti_fra(files: list[Path], radice: Path) -> list[str]:
    """I file di `files` i cui byte sul disco non sono quelli che git registrerebbe.

    Vedi `fini_riga_divergenti`, che e' questa applicata a un albero
    vendorizzato. Sta a parte perche' lo stesso difetto colpisce ogni impronta
    calcolata leggendo il disco -- anche quella del perimetro delle misure di
    profondita' -- e la difesa non ha ragione di vivere nel gate dei fork.
    """
    if not files:
        return []
    filtrati = _oid(files, filtri=True)
    grezzi = _oid(files, filtri=False)
    return sorted(
        percorso.relative_to(radice).as_posix()
        for percorso, con, senza in zip(files, filtrati, grezzi)
        if con != senza
    )


def fini_riga_divergenti(vendor: Path) -> list[str]:
    """File i cui byte sul disco non sono quelli che git registrerebbe.

    # Perche' serve, e perche' e' separato dall'impronta

    `impronta` legge il disco, ed e' giusto: deve accorgersi di una modifica
    prima che qualcuno la committi. Ma `.gitattributes` impone `eol=lf` a
    sorgenti e script, quindi cio' che git registra puo' differire da cio' che
    sta sul disco -- ed e' esattamente cio' che succede su Windows, dove un
    editor riscrive un file intero con CRLF.

    L'impronta diventa allora **dipendente dalla piattaforma**: quella
    calcolata su un albero con CRLF non e' riproducibile in CI, che lavora su
    un checkout con LF. Il lock registra un digest che nessun altro puo'
    ottenere, e il gate del fork e' rosso ovunque tranne che dove e' stato
    scritto.

    E' successo il 2026-09-04, e per capire perche' ci sono volute una corsa di
    CI e mezz'ora: il rosso diceva «albero vendorizzato diverso dal lock», che
    e' vero e non e' la ragione. Questo controllo nomina la ragione.

    Il confronto e' fra i due oid che git stesso calcola, con e senza i filtri:
    se coincidono, il disco e' gia' cio' che verra' registrato. Nessun oggetto
    viene scritto -- `hash-object` senza `-w` calcola e basta -- e il controllo
    non dipende da quale attributo sia in gioco.
    """
    return fini_riga_divergenti_fra(insieme_versionato(vendor), vendor)


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
