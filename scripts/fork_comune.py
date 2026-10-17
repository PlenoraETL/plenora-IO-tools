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
import tomllib
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


# --- risoluzione dei fork -----------------------------------------------------
#
# Fino alla 4.1.1 i tre fork entravano come `[patch.crates-io]`. Cargo applica
# una patch **solo dal workspace radice**: chi dipendeva da questi crate per git
# o per percorso riceveva `gdal`, `shapefile` e `dxf` da crates.io, cioe' senza
# i delta. Che la compilazione allora fallisse era una coincidenza -- i driver
# chiamano API che solo i fork espongono --, non una garanzia: un delta di solo
# comportamento sarebbe passato in silenzio.
#
# Ora ogni fork e' una dipendenza diretta per percorso con un nome di pacchetto
# proprio. Le funzioni qui sotto tengono ferma quella forma per tutti e tre, e
# rifiutano che una sezione `[patch]` ricompaia in uno dei due manifesti di
# workspace.

#: I manifesti che risolvono il grafo: il workspace e quello staccato del fuzz.
MANIFESTI_DI_WORKSPACE = ("Cargo.toml", "fuzz/Cargo.toml")


def patch_presenti(radice: Path = ROOT) -> list[str]:
    """Le sezioni `[patch]` nei manifesti di workspace: devono essere zero.

    Una patch, qualunque sorgente sostituisca, vale solo qui e non per chi ci
    usa come dipendenza: e' esattamente la forma che ha reso i fork non
    transitivi.
    """
    trovate: list[str] = []
    for manifesto in MANIFESTI_DI_WORKSPACE:
        dati = tomllib.loads((radice / manifesto).read_text(encoding="utf-8"))
        for sorgente in sorted(dati.get("patch", {})):
            trovate.append(f"{manifesto}: [patch.{sorgente}]")
    return trovate


def problemi_di_risoluzione(lock: dict, radice: Path = ROOT) -> list[str]:
    """Che il fork del `lock` sia risolto come dipendenza diretta e con nome proprio.

    `lock["package"]` e' il nome **upstream** (l'identita' da cui il fork
    deriva), `lock["fork_package"]` quello con cui il fork entra nel grafo.
    """
    problemi = list(patch_presenti(radice))
    upstream = lock["package"]
    fork = lock["fork_package"]
    if fork == upstream or not fork.startswith("plenora-fork-"):
        problemi.append(
            f"il nome del fork «{fork}» non e' un nome proprio: un nome uguale a "
            "quello upstream si puo' risolvere da crates.io"
        )

    cargo = tomllib.loads((radice / "Cargo.toml").read_text(encoding="utf-8"))
    dichiarata = cargo.get("workspace", {}).get("dependencies", {}).get(upstream)
    attesa = {
        "package": fork,
        "version": f"={lock['version']}",
        "path": lock["vendor_path"],
    }
    if not isinstance(dichiarata, dict) or {
        chiave: dichiarata.get(chiave) for chiave in attesa
    } != attesa:
        problemi.append(
            f"Cargo.toml non dichiara «{upstream}» come dipendenza diretta del "
            f"fork: atteso {attesa}"
        )

    vendor = radice / lock["vendor_path"]
    manifesto = tomllib.loads((vendor / "Cargo.toml").read_text(encoding="utf-8"))
    pacchetto = manifesto.get("package", {})
    if pacchetto.get("name") != fork or pacchetto.get("version") != lock["version"]:
        problemi.append("manifest del fork incoerente col lock")
    # Il nome della libreria resta quello upstream: il codice dei driver, i
    # simboli e i requisiti di profondita' del fuzz lo nominano.
    if manifesto.get("lib", {}).get("name") != upstream:
        problemi.append(
            f"il fork non dichiara `[lib] name = \"{upstream}\"`: il crate "
            "compilato cambierebbe nome"
        )

    for nome_lock in ("Cargo.lock", "fuzz/Cargo.lock"):
        pacchetti = tomllib.loads(
            (radice / nome_lock).read_text(encoding="utf-8")
        )["package"]
        del_fork = [p for p in pacchetti if p.get("name") == fork]
        if (
            len(del_fork) != 1
            or del_fork[0].get("version") != lock["version"]
            or "source" in del_fork[0]
            or "checksum" in del_fork[0]
        ):
            problemi.append(
                f"{nome_lock} non risolve un'unica dipendenza per percorso "
                f"{fork} {lock['version']}"
            )
        # Il controllo che conta: l'upstream non deve comparire affatto. Se
        # comparisse, qualcosa nel grafo lo chiederebbe ancora per nome.
        if any(p.get("name") == upstream for p in pacchetti):
            problemi.append(
                f"{nome_lock} contiene ancora il pacchetto upstream «{upstream}»"
            )
    return problemi
