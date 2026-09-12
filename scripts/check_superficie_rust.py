#!/usr/bin/env python3
"""La superficie Rust pubblica, provata da fuori e dall'archivio distribuito.

# Che cosa questo gate afferma

Tre cose, e la terza e' quella che gli altri due non possono dare:

1. la mappatura `contracts/superficie-rust.json` e il consumatore esterno
   nominano **lo stesso** insieme di export, nei due versi -- un export
   documentato e mai importato e' una promessa che nessuno prova, e uno
   importato e mai documentato e' una dipendenza che nessuno ha dichiarato;
2. ogni operazione del catalogo comune ha una riga nella mappatura;
3. il consumatore **compila e gira** contro l'archivio sorgente distribuito,
   non contro l'albero di lavoro.

# Perche' il terzo punto non e' pedanteria

Un test dentro il workspace vede i `pub(crate)`, eredita i lint e compila
contro i file che ha sotto mano. Puo' passare su una superficie che nessun
estraneo raggiunge, e su un archivio che non contiene i file che servono --
`ESCLUSI` in `costruisci-archivio-sorgente.py` decide che cosa l'archivio porta,
e se escludesse per sbaglio un crate necessario nessuna prova interna lo
direbbe.

Qui l'archivio si costruisce, si estrae, e il consumatore ci si compila accanto.
Se un export documentato diventasse privato, cambiasse firma o non fosse nel
tar, la compilazione fallirebbe.

# Il costo, dichiarato

E' il gate piu' lento del repository: compila il workspace da zero in una
directory temporanea, senza cache condivisa. Gira in CI e **non** nel
checkpoint, per la stessa ragione per cui non girano li' le campagne di fuzzing:
un checkpoint che compila due volte lo stesso workspace misura la macchina.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
MAPPATURA = ROOT / "contracts" / "superficie-rust.json"
CONSUMATORE = ROOT / "conformance" / "consumatore-rust"
CATALOGO_OPERAZIONI = (
    "io.catalog",
    "io.inspect",
    "io.layers",
    "io.read",
    "io.write",
    "io.convert",
)


def export_dichiarati(mappatura: dict) -> set[str]:
    """Gli export che la mappatura promette, operazioni e tipi insieme."""
    nomi = {voce["export"] for voce in mappatura["operazioni"]}
    nomi |= {voce["export"] for voce in mappatura["tipi_pubblici"]}
    return nomi


def export_importati() -> set[str]:
    """Gli export che il consumatore nomina, letti dal suo sorgente.

    Si leggono dal `use` e dagli usi qualificati, non da un elenco a parte: un
    elenco si aggiorna a mano e divergerebbe dal codice che compila.
    """
    testo = (CONSUMATORE / "src" / "main.rs").read_text(encoding="utf-8")
    trovati: set[str] = set()
    # `use plenora_io_cli::operazioni::{self, Esito, Richiesta};`
    for blocco in re.finditer(
        r"use\s+(plenora_io_cli(?:::[a-z_]+)*)::\{([^}]*)\}", testo
    ):
        radice, elenco = blocco.group(1), blocco.group(2)
        for nome in (n.strip() for n in elenco.split(",")):
            if not nome or nome == "self":
                continue
            trovati.add(f"{radice}::{nome}")
    # `operazioni::catalog(` e simili, che il `self` rende possibili.
    for uso in re.finditer(r"\boperazioni::([a-z_]+)\s*\(", testo):
        trovati.add(f"plenora_io_cli::operazioni::{uso.group(1)}")
    return trovati


def coerenza(mappatura: dict) -> list[str]:
    problemi: list[str] = []

    dichiarati = export_dichiarati(mappatura)
    importati = export_importati()

    for mancante in sorted(dichiarati - importati):
        problemi.append(
            f"«{mancante}» e' documentato e il consumatore non lo importa: una "
            "promessa che nessuno prova"
        )
    for estraneo in sorted(importati - dichiarati):
        problemi.append(
            f"«{estraneo}» e' importato e la mappatura non lo documenta: una "
            "dipendenza che nessuno ha dichiarato"
        )

    mappate = {voce["operazione"] for voce in mappatura["operazioni"]}
    for operazione in CATALOGO_OPERAZIONI:
        if operazione not in mappate:
            problemi.append(f"«{operazione}» non ha una riga nella mappatura")
    for operazione in sorted(mappate - set(CATALOGO_OPERAZIONI)):
        problemi.append(
            f"«{operazione}» e' mappata e non e' un'operazione del catalogo"
        )

    # Ogni riga nomina i due contratti, e sono quelli pubblicati.
    schemi = ROOT / "contracts" / "schemas"
    for voce in mappatura["operazioni"]:
        for chiave in ("contratto_ingresso", "contratto_uscita"):
            nome = voce[chiave]
            if not (schemi / f"{nome}.schema.json").is_file():
                problemi.append(
                    f"{voce['operazione']}: «{nome}» non e' uno schema pubblicato"
                )
    return problemi


def inclusioni_fuori_dal_crate() -> list[tuple[str, str]]:
    """I file che un crate compila dentro di se' prendendoli da fuori.

    `include_str!` e `include_bytes!` con un percorso che esce dal crate legano
    la compilazione a un file che sta altrove nel repository. Se l'archivio non
    lo porta, non compila -- e nessuna prova interna puo' dirlo, perche' dentro
    il repository quel file c'e' sempre.

    E' la classe di difetti che ha reso rosso questo gate la prima volta:
    `assurance/` era fra gli esclusi e `driver-geoparquet` ne compila dentro
    quattro schemi.
    """
    trovate: list[tuple[str, str]] = []
    for sorgente in (ROOT / "crates").rglob("*.rs"):
        testo = sorgente.read_text(encoding="utf-8", errors="replace")
        for uso in re.finditer(r'include_(?:str|bytes)!\(\s*"([^"]+)"', testo):
            percorso = uso.group(1)
            if not percorso.startswith("../"):
                continue
            risolto = (sorgente.parent / percorso).resolve()
            try:
                relativo = risolto.relative_to(ROOT)
            except ValueError:
                continue
            # Fuori dal crate che lo include: e' il caso che ci interessa.
            if not str(relativo).replace("\\", "/").startswith("crates/"):
                trovate.append((str(sorgente.relative_to(ROOT)), str(relativo).replace("\\", "/")))
    return trovate


def compila_dall_archivio(lavoro: pathlib.Path) -> list[str]:
    """Costruisce l'archivio, lo estrae, e ci compila il consumatore accanto."""
    problemi: list[str] = []

    uscita = lavoro / "archivio"
    subprocess.run(
        [sys.executable, str(ROOT / "scripts" / "costruisci-archivio-sorgente.py"),
         "--uscita", str(uscita)],
        check=True,
        capture_output=True,
    )
    manifesto = json.loads((uscita / "MANIFEST-sorgente.json").read_text(encoding="utf-8"))
    tar = uscita / manifesto["artefatto"]

    estratto = lavoro / "estratto"
    estratto.mkdir()
    with tarfile.open(tar) as archivio:
        archivio.extractall(estratto)
    radice = estratto / f"plenora-io-{manifesto['versione']}"
    if not (radice / "crates" / "plenora-io-cli" / "src" / "lib.rs").is_file():
        return [
            "l'archivio non contiene `crates/plenora-io-cli/src/lib.rs`: la "
            "superficie non e' distribuita"
        ]
    # Ogni file che un crate compila dentro di se' prendendolo da fuori deve
    # essere nell'archivio, o quel crate non compila.
    for sorgente, incluso in inclusioni_fuori_dal_crate():
        if (radice / incluso).exists():
            continue
        # Un'inclusione dentro `#[cfg(test)]` puo' mancare: l'archivio non
        # promette le suite di prova. Lo si distingue guardando se il file sta
        # sotto un percorso che l'archivio dichiara di escludere per le prove.
        if incluso.startswith(("fuzz/seeds", "fuzz/fixtures", "fuzz/corpus")):
            continue
        problemi.append(
            f"`{sorgente}` compila dentro di se' `{incluso}`, che l'archivio non "
            "contiene: il crate non compilerebbe fuori dal repository"
        )
    if problemi:
        return problemi

    for fork in ("gdal", "shapefile", "dxf"):
        if not (radice / "vendor" / fork / "Cargo.toml").is_file():
            return [
                f"l'archivio non contiene `vendor/{fork}`: il workspace lo "
                "sostituisce a una dipendenza di crates.io, e senza il fork un "
                "consumatore non compila affatto"
            ]

    # Il consumatore accanto all'albero estratto, con la dipendenza riscritta
    # sul percorso dell'archivio.
    fuori = lavoro / "consumatore"
    shutil.copytree(CONSUMATORE, fuori)
    manifesto_consumatore = (fuori / "Cargo.toml").read_text(encoding="utf-8")
    for relativo, assoluto in (
        ("../../crates/plenora-io-cli", radice / "crates" / "plenora-io-cli"),
        ("../../vendor/gdal", radice / "vendor" / "gdal"),
        ("../../vendor/shapefile", radice / "vendor" / "shapefile"),
        ("../../vendor/dxf", radice / "vendor" / "dxf"),
    ):
        manifesto_consumatore = manifesto_consumatore.replace(
            f'path = "{relativo}"', f'path = "{assoluto.as_posix()}"'
        )
    (fuori / "Cargo.toml").write_text(manifesto_consumatore, encoding="utf-8")

    # La toolchain e' quella che l'archivio dichiara, non quella di sistema.
    # Fuori dal workspace il pin non si eredita, e senza di esso cargo sceglie
    # il canale di default: il consumatore fallirebbe con «requires rustc
    # 1.98.1» su una macchina che quel compilatore ce l'ha. E' anche cio' che un
    # consumatore vero deve fare -- la versione minima e' una condizione d'uso
    # della superficie, non un dettaglio di questa prova.
    pin = radice / "rust-toolchain.toml"
    if not pin.is_file():
        return ["l'archivio non contiene `rust-toolchain.toml`: la versione minima non e' distribuita"]
    shutil.copy(pin, fuori / "rust-toolchain.toml")

    # Senza `--offline`, e la ragione e' che la proprieta' sotto esame non e' la
    # ermeticita': e' che un consumatore esterno possa costruire questa
    # superficie. Un consumatore esterno scarica le proprie dipendenze, e i tre
    # fork governati arrivano comunque dall'archivio perche' sono patch per
    # percorso.
    #
    # `--offline` c'era, e passava in locale perche' la cache del container
    # aveva gia' tutto. In CI, su un runner pulito, falliva: la prova stava
    # misurando la cache invece della distribuzione.
    corsa = subprocess.run(
        ["cargo", "run", "--quiet"],
        cwd=fuori,
        capture_output=True,
        text=True,
    )
    if corsa.returncode != 0:
        problemi.append(
            "il consumatore esterno non compila o non gira contro l'archivio:\n"
            + (corsa.stderr or corsa.stdout)[-1800:]
        )
    elif "sette export documentati" not in corsa.stdout:
        problemi.append(f"il consumatore non ha confermato: {corsa.stdout.strip()[:200]}")
    return problemi


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--solo-coerenza",
        action="store_true",
        help="salta la compilazione dall'archivio; per un ciclo di sviluppo rapido",
    )
    opzioni = parser.parse_args()

    mappatura = json.loads(MAPPATURA.read_text(encoding="utf-8"))
    problemi = coerenza(mappatura)

    if not opzioni.solo_coerenza:
        with tempfile.TemporaryDirectory(prefix="superficie-rust-") as temporanea:
            problemi.extend(compila_dall_archivio(pathlib.Path(temporanea)))

    if problemi:
        for problema in problemi:
            print(problema, file=sys.stderr)
        print(
            f"\n{len(problemi)} divergenze fra la mappatura, il consumatore e "
            "l'archivio.",
            file=sys.stderr,
        )
        return 1

    quante = len(mappatura["operazioni"]) + len(mappatura["tipi_pubblici"])
    coda = (
        " (compilazione dall'archivio saltata)"
        if opzioni.solo_coerenza
        else "; il consumatore esterno compila e gira contro l'archivio distribuito"
    )
    print(
        f"superficie Rust verificata: {quante} export documentati e importati, "
        f"{len(CATALOGO_OPERAZIONI)} operazioni mappate{coda}."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
