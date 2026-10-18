#!/usr/bin/env python3
"""Prepara il progetto da cui maturin costruisce la wheel nativa dello SDK.

# Perche' una directory preparata

Lo SDK Python ha **un** sorgente, `sdk/python/src/plenora_io`, e due wheel:

* quella pura (`py3-none-any`), che `costruisci-pacchetto-python.py` scrive a
  mano, byte per byte riproducibile, e che esegue un binario `plenora-io`;
* quella nativa (`cp310-abi3-<piattaforma>`), che porta anche
  `plenora_io._native` -- il modulo PyO3 di `crates/plenora-io-py` -- ed
  esegue i comandi nel processo, senza un binario accanto.

maturin vuole il sorgente Python **dentro** la directory del progetto: un
`python-source` che esce dalla directory viene ignorato in silenzio, e la wheel
esce col solo modulo nativo. Lo abbiamo visto succedere. Copiare il sorgente in
una directory preparata, invece di spostarlo, tiene una sorgente sola per le
due wheel; e il controllo in coda a questo script verifica che il pacchetto
ci sia davvero.

La directory sta sotto `target/`, dentro il workspace: maturin-action costruisce
la wheel Linux in un container che monta il workspace, e un percorso fuori non
lo vedrebbe.

# Uso

    python3 scripts/prepara-wheel-nativa.py            # prepara, stampa la directory
    python3 scripts/prepara-wheel-nativa.py --verifica WHEEL
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import shutil
import sys
import zipfile

import avvisi_crate

RADICE = pathlib.Path(__file__).resolve().parent.parent
CRATE = RADICE / "crates" / "plenora-io-py"
SORGENTE = RADICE / "sdk" / "python" / "src" / "plenora_io"
PROGETTO = RADICE / "target" / "wheel-nativa"


def prepara() -> pathlib.Path:
    if PROGETTO.exists():
        shutil.rmtree(PROGETTO)
    (PROGETTO / "python").mkdir(parents=True)
    shutil.copytree(
        SORGENTE,
        PROGETTO / "python" / "plenora_io",
        ignore=shutil.ignore_patterns("__pycache__", "*.pyc"),
    )
    shutil.copy(RADICE / "sdk" / "python" / "README.md", PROGETTO / "README.md")
    shutil.copy(CRATE / "pyproject.toml", PROGETTO / "pyproject.toml")
    shutil.copy(RADICE / "LICENSE", PROGETTO / "LICENSE")
    pacchetto = PROGETTO / "python" / "plenora_io"
    shutil.copy(RADICE / "LICENSE", pacchetto / "LICENSE")
    dipendenze = avvisi_crate._dipendenze_rust()
    crate = dipendenze.linkati(dipendenze.grafo("base"), radice="plenora-io-py")
    destinazione = pacchetto / "LICENSES" / "crate-rust"
    pacchetti = avvisi_crate.pacchetti_con_avvisi(crate, destinazione)
    (destinazione / "CRATE.json").write_text(
        json.dumps({"pacchetti": pacchetti}, indent=2) + "\n", encoding="utf-8"
    )
    return PROGETTO


def verifica(wheel: pathlib.Path) -> list[str]:
    """La wheel porta il pacchetto Python **e** il modulo nativo."""
    with zipfile.ZipFile(wheel) as archivio:
        nomi = set(archivio.namelist())
        errori_licenze = verifica_licenze(archivio)
    attesi_python = {
        f"plenora_io/{file.name}"
        for file in SORGENTE.iterdir()
        if file.is_file() and file.suffix in {".py", ".typed"}
    }
    errori = errori_licenze + [f"manca {nome}" for nome in sorted(attesi_python - nomi)]
    if not any(n.startswith("plenora_io/_native.") for n in nomi):
        errori.append("manca il modulo nativo plenora_io/_native")
    if "-cp310-abi3-" not in wheel.name:
        errori.append(f"«{wheel.name}» non e' una wheel abi3 da Python 3.10")
    return errori


def verifica_licenze(archivio: zipfile.ZipFile) -> list[str]:
    """Confronta l'artefatto col grafo reale, non con il solo elenco spedito."""
    errori = []
    nomi = set(archivio.namelist())
    licenza = "plenora_io/LICENSE"
    if licenza not in nomi or archivio.read(licenza) != (RADICE / "LICENSE").read_bytes():
        errori.append("licenza del prodotto assente o diversa")
    dipendenze = avvisi_crate._dipendenze_rust()
    crate = dipendenze.linkati(dipendenze.grafo("base"), radice="plenora-io-py")
    attesi, guasti, _ = avvisi_crate.avvisi(crate)
    errori.extend(guasti)
    radice = "plenora_io/LICENSES/crate-rust/"
    elenco = radice + "CRATE.json"
    if elenco not in nomi:
        return errori + ["manca l'elenco degli avvisi dei crate"]
    try:
        dichiarati = json.loads(archivio.read(elenco))["pacchetti"]
        dichiarazione = {
            (c["nome"], c["versione"]): c["avvisi"] for c in dichiarati
        }
        if dichiarazione != {(c["nome"], c["versione"]): c["avvisi"] for c in attesi}:
            errori.append("l'elenco degli avvisi non corrisponde al grafo nativo")
    except (KeyError, ValueError, TypeError):
        errori.append("elenco degli avvisi non valido")
    for crate in attesi:
        for avviso in crate["avvisi"]:
            nome = radice + f"{crate['nome']}-{crate['versione']}/" + avviso["file"]
            if nome not in nomi:
                errori.append(f"manca {nome}")
            elif hashlib.sha256(archivio.read(nome).replace(b"\r\n", b"\n")).hexdigest() != avviso["sha256"]:
                errori.append(f"avviso alterato: {nome}")
    return errori


def main() -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument("--verifica", type=pathlib.Path)
    opzioni = argomenti.parse_args()
    if opzioni.verifica is not None:
        errori = verifica(opzioni.verifica)
        for errore in errori:
            print(errore, file=sys.stderr)
        if errori:
            return 1
        print(f"{opzioni.verifica.name}: pacchetto Python, modulo nativo e licenze verificati")
        return 0
    print(prepara())
    return 0


if __name__ == "__main__":
    sys.exit(main())
