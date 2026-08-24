#!/usr/bin/env python3
"""I semi del target `filegdb_reader`, derivati dalla fixture.

# Che cosa e' un seme, qui

L'input del target si legge cosi': il primo byte sceglie **quale** parte della
fixture sostituire, il resto e' il contenuto che ne prende il posto. Un seme
utile e' percio' una parte con il proprio contenuto **originale**: la `.gdb` che
ne risulta e' identica alla fixture, si legge, e da' al fuzzer una base valida
da mutare per quella tabella specifica.

E' l'opposto di un seme casuale. Partire da byte inventati significherebbe far
rimbalzare il driver sul riconoscimento del formato a ogni esecuzione, e la
campagna misurerebbe il rifiuto invece del formato.

# Perche' derivati e non committati

Un seme e' un file binario, e un binario committato senza il modo di riprodurlo
e' un artefatto che nessuno puo' rileggere. Qui i semi sono **funzione della
fixture**: cambiata la fixture, i semi che non la seguissero comincerebbero a
sostituire la parte sbagliata. `--verifica` lo rende visibile invece che
silenzioso.

# Uso

    python3 scripts/genera_semi_filegdb.py --scrivi
    python3 scripts/genera_semi_filegdb.py --verifica
"""

from __future__ import annotations

import argparse
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

from genera_fixture_filegdb import ARCHIVIO, spacchetta  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
SEMI = ROOT / "fuzz" / "seeds" / "filegdb_reader"


def semi() -> dict[str, bytes]:
    """Un seme per parte, piu' quello che non sostituisce niente."""
    contenuti = spacchetta(ARCHIVIO.read_bytes())
    prodotti: dict[str, bytes] = {
        # L'input vuoto materializza la fixture **intatta**. E' il seme che dice
        # se la base si legge ancora: senza, una campagna verde potrebbe voler
        # dire che nessun input arriva al driver.
        "fixture-intatta.bin": b"",
    }
    for indice, nome in enumerate(sorted(contenuti)):
        if indice > 0xFF:
            raise ValueError("piu' di 256 parti: il primo byte non basta a sceglierle")
        prodotti[f"parte-{indice:02d}-{nome}.bin"] = bytes([indice]) + contenuti[nome]
    return prodotti


def _dove() -> str:
    """Il percorso dei semi, relativo alla radice quando ci sta dentro.

    Le sonde spostano `SEMI` in una directory temporanea per provare i casi
    rossi: un `relative_to` incondizionato le farebbe fallire nel *print*, cioe'
    fuori da cio' che stanno verificando.
    """
    try:
        return SEMI.relative_to(ROOT).as_posix()
    except ValueError:
        return SEMI.as_posix()


def scrivi() -> int:
    SEMI.mkdir(parents=True, exist_ok=True)
    prodotti = semi()
    for nome, contenuto in sorted(prodotti.items()):
        (SEMI / nome).write_bytes(contenuto)
    print(f"{len(prodotti)} semi scritti in {_dove()}")
    return 0


def verifica() -> int:
    prodotti = semi()
    errori: list[str] = []

    presenti = {p.name for p in SEMI.glob("*")} if SEMI.exists() else set()
    for extra in sorted(presenti - set(prodotti)):
        errori.append(
            f"{extra}: seme non prodotto da questo generatore. Un seme che non "
            "segue la fixture sostituisce la parte sbagliata."
        )
    for nome, atteso in sorted(prodotti.items()):
        percorso = SEMI / nome
        if not percorso.exists():
            errori.append(f"{nome}: seme assente; si rigenera con `--scrivi`")
        elif percorso.read_bytes() != atteso:
            errori.append(
                f"{nome}: differisce da cio' che il generatore produce. O il seme "
                "e' stato modificato a mano, o la fixture e' cambiata senza "
                "rigenerarlo."
            )

    for messaggio in errori:
        print(messaggio, file=sys.stderr)
    if errori:
        return 1
    print(f"semi di filegdb_reader verificati: {len(prodotti)}, tutti derivati dalla fixture.")
    return 0


def main(argv: list[str] | None = None) -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    modo = argomenti.add_mutually_exclusive_group()
    modo.add_argument("--scrivi", action="store_true")
    modo.add_argument("--verifica", action="store_true")
    opzioni = argomenti.parse_args(argv)
    return scrivi() if opzioni.scrivi else verifica()


if __name__ == "__main__":
    sys.exit(main())
