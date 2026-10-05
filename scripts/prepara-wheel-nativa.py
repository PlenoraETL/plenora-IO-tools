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
import pathlib
import shutil
import sys
import zipfile

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
    return PROGETTO


def verifica(wheel: pathlib.Path) -> list[str]:
    """La wheel porta il pacchetto Python **e** il modulo nativo."""
    with zipfile.ZipFile(wheel) as archivio:
        nomi = set(archivio.namelist())
    attesi_python = {
        f"plenora_io/{file.name}"
        for file in SORGENTE.iterdir()
        if file.is_file() and file.suffix in {".py", ".typed"}
    }
    errori = [f"manca {nome}" for nome in sorted(attesi_python - nomi)]
    if not any(n.startswith("plenora_io/_native.") for n in nomi):
        errori.append("manca il modulo nativo plenora_io/_native")
    if "-cp310-abi3-" not in wheel.name:
        errori.append(f"«{wheel.name}» non e' una wheel abi3 da Python 3.10")
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
        print(f"{opzioni.verifica.name}: pacchetto Python e modulo nativo presenti")
        return 0
    print(prepara())
    return 0


if __name__ == "__main__":
    sys.exit(main())
