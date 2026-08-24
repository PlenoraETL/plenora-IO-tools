#!/usr/bin/env python3
"""La fixture FileGDB del target `filegdb_reader`, e la prova che e' riproducibile.

# Perche' una fixture, e non un blob costruito dal fuzzer

Uno Shapefile si puo' scrivere dalla specifica: il formato e' pubblicato, e il
generatore dei semi di `shp_reader` lo fa in trecento righe. Un FileGDB no. E'
un formato proprietario, ricostruito per reverse engineering, fatto di una
**directory** di tabelle che si citano a vicenda per GUID. Scriverlo a mano
significherebbe riscrivere `OpenFileGDB`, e i semi sarebbero validi per
costruzione rispetto alla nostra idea del formato, non a quella di GDAL.

La fixture e' percio' un FileGDB **vero**, prodotto da GDAL stesso a partire da
un GeoJSON committato. Il fuzzer non la costruisce: la riceve intatta e ne
sostituisce **un file per volta**.

# In che senso e' riproducibile

`ogr2ogr` e' deterministico su questo input tranne che per i GUID che conia per
il dataset: quarantotto byte per tabella di metadati, tre identificatori da
sedici, scritti in `GDB_Items` e ripetuti in `GDB_ItemRelationships`. Tutto il
resto -- l'XML dei descrittori, le tabelle dei dati, l'indice spaziale -- e'
identico byte a byte fra due corse.

Dichiararlo non basterebbe. `--verifica` lo **dimostra**: rigenera la fixture
**due** volte e prende come «byte coniati» esattamente gli offset in cui le due
rigenerazioni differiscono fra loro. Poi confronta la fixture committata con la
prima rigenerazione e pretende che le differenze stiano dentro quell'insieme.

Un byte che cambia fra due rigenerazioni e' coniato; un byte che e' stabile fra
le rigenerazioni ma diverso da quello committato e' una divergenza vera -- una
versione di GDAL diversa, un input cambiato, una fixture modificata a mano -- e
diventa rosso. La tolleranza e' cosi' **derivata** da cio' che GDAL fa oggi, non
scritta da noi.

# La forma dell'archivio

La fixture e' una directory, e il target la vuole dentro il binario. Le venti
parti stanno percio' in un archivio con un indice, ordinato per nome:

```text
PLENORA-GDB-FIXTURE-1\\n
u32          numero di file
per file:    u16 lunghezza del nome, nome ASCII, u32 lunghezza, byte
```

L'ordinamento e' cio' che rende l'archivio deterministico a parita' di
contenuto, e l'indice e' cio' che permette al target di scegliere quale file
sostituire senza costruire un percorso dal payload.

# Uso

    python3 scripts/genera_fixture_filegdb.py --scrivi <directory.gdb>
    python3 scripts/genera_fixture_filegdb.py --verifica
    python3 scripts/genera_fixture_filegdb.py --confronta <gdb-1> <gdb-2>

`--scrivi` e `--confronta` vogliono un FileGDB gia' prodotto da GDAL: li produce
`scripts/genera-fixture-filegdb.sh`, che e' il solo posto in cui `ogr2ogr` viene
invocato.
"""

from __future__ import annotations

import argparse
import hashlib
import pathlib
import struct
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
ARCHIVIO = ROOT / "fuzz" / "fixtures" / "filegdb" / "citta.gdb.bundle"

INTESTAZIONE = b"PLENORA-GDB-FIXTURE-1\n"

# I nomi che una parte di FileGDB puo' avere. Non e' una convenzione nostra: e'
# cio' che `OpenFileGDB` scrive, e limitarlo qui impedisce che un archivio
# costruito altrove faccia scrivere al target un nome qualunque.
CARATTERI_AMMESSI = set("abcdefghijklmnopqrstuvwxyz0123456789._")


def parti(directory: pathlib.Path) -> dict[str, bytes]:
    """Le parti del FileGDB, per nome. Solo file, nessuna ricorsione."""
    trovate: dict[str, bytes] = {}
    for percorso in sorted(directory.iterdir()):
        if not percorso.is_file() or percorso.is_symlink():
            continue
        nome = percorso.name
        if not nome or set(nome) - CARATTERI_AMMESSI:
            raise ValueError(f"nome di parte non ammesso: {nome!r}")
        trovate[nome] = percorso.read_bytes()
    if not trovate:
        raise ValueError(f"{directory}: nessuna parte da archiviare")
    return trovate


def impacchetta(contenuti: dict[str, bytes]) -> bytes:
    corpo = [INTESTAZIONE, struct.pack("<I", len(contenuti))]
    for nome, byte in sorted(contenuti.items()):
        grezzo = nome.encode("ascii")
        corpo.append(struct.pack("<H", len(grezzo)))
        corpo.append(grezzo)
        corpo.append(struct.pack("<I", len(byte)))
        corpo.append(byte)
    return b"".join(corpo)


def spacchetta(archivio: bytes) -> dict[str, bytes]:
    if not archivio.startswith(INTESTAZIONE):
        raise ValueError("archivio senza intestazione")
    posizione = len(INTESTAZIONE)
    (quante,) = struct.unpack_from("<I", archivio, posizione)
    posizione += 4
    contenuti: dict[str, bytes] = {}
    for _ in range(quante):
        (lunghezza_nome,) = struct.unpack_from("<H", archivio, posizione)
        posizione += 2
        nome = archivio[posizione : posizione + lunghezza_nome].decode("ascii")
        posizione += lunghezza_nome
        (lunghezza,) = struct.unpack_from("<I", archivio, posizione)
        posizione += 4
        contenuti[nome] = archivio[posizione : posizione + lunghezza]
        posizione += lunghezza
    if posizione != len(archivio):
        raise ValueError("archivio con byte di coda non dichiarati")
    return contenuti


def offset_coniati(uno: bytes, due: bytes) -> set[int]:
    """Gli offset in cui due rigenerazioni differiscono fra loro.

    Sono i byte che GDAL conia a ogni corsa. Derivarli dal confronto e' l'unico
    modo di non doverli scrivere a mano: una tolleranza scritta a mano
    resterebbe ferma il giorno in cui GDAL ne conia uno in piu'.
    """
    if len(uno) != len(due):
        # Una differenza di lunghezza non e' un byte coniato: e' un'altra
        # fixture. Nessun offset e' tollerato.
        return set()
    return {i for i in range(len(uno)) if uno[i] != due[i]}


def confronta(
    committata: dict[str, bytes],
    prima: dict[str, bytes],
    seconda: dict[str, bytes],
) -> list[str]:
    """La fixture committata e' quella che GDAL produce, salvo i byte coniati."""
    errori: list[str] = []

    if set(prima) != set(seconda):
        return ["due rigenerazioni con parti diverse: GDAL non e' stabile nemmeno nei nomi"]
    mancanti = sorted(set(prima) - set(committata))
    estranee = sorted(set(committata) - set(prima))
    if mancanti:
        errori.append(f"parti prodotte da GDAL e assenti dalla fixture: {mancanti}")
    if estranee:
        errori.append(f"parti nella fixture che GDAL non produce: {estranee}")
    if errori:
        return errori

    for nome in sorted(prima):
        coniati = offset_coniati(prima[nome], seconda[nome])
        atteso, ottenuto = prima[nome], committata[nome]
        if len(atteso) != len(ottenuto):
            errori.append(
                f"{nome}: la fixture ha {len(ottenuto)} byte, GDAL ne produce "
                f"{len(atteso)}. Una differenza di lunghezza non e' un byte coniato."
            )
            continue
        diversi = {i for i in range(len(atteso)) if atteso[i] != ottenuto[i]}
        stabili = sorted(diversi - coniati)
        if stabili:
            errori.append(
                f"{nome}: {len(stabili)} byte differiscono dalla fixture e sono "
                f"**stabili** fra due rigenerazioni (primo offset {stabili[0]}). "
                "Un byte stabile e diverso non e' coniato da GDAL: o la fixture "
                "e' stata modificata, o l'input e la versione di GDAL non sono "
                "piu' quelli con cui e' stata prodotta."
            )
    return errori


def scrivi(directory: pathlib.Path) -> int:
    contenuti = parti(directory)
    archivio = impacchetta(contenuti)
    ARCHIVIO.parent.mkdir(parents=True, exist_ok=True)
    ARCHIVIO.write_bytes(archivio)
    print(
        f"fixture archiviata in {ARCHIVIO.relative_to(ROOT).as_posix()}: "
        f"{len(contenuti)} parti, {len(archivio)} byte, "
        f"sha256 {hashlib.sha256(archivio).hexdigest()[:16]}"
    )
    return 0


def verifica_forma() -> int:
    """Il controllo che costa millisecondi: l'archivio si rilegge intero.

    Non dice che la fixture sia quella che GDAL produce -- per quello serve
    GDAL, e serve `--confronta`. Dice che l'archivio non e' troncato, che i nomi
    sono nomi di parte e che il target trovera' cio' che si aspetta.
    """
    if not ARCHIVIO.exists():
        print(f"{ARCHIVIO}: fixture assente", file=sys.stderr)
        return 1
    try:
        contenuti = spacchetta(ARCHIVIO.read_bytes())
    except (ValueError, struct.error, UnicodeDecodeError) as errore:
        print(f"{ARCHIVIO}: archivio illeggibile ({errore})", file=sys.stderr)
        return 1

    errori: list[str] = []
    for nome in sorted(contenuti):
        if not nome or set(nome) - CARATTERI_AMMESSI:
            errori.append(f"nome di parte non ammesso: {nome!r}")
    # Un FileGDB senza il file `gdb` non e' un FileGDB, e senza almeno una
    # tabella non ha niente da leggere: sono le due condizioni che rendono la
    # fixture una base di partenza invece di una directory qualunque.
    if "gdb" not in contenuti:
        errori.append("la fixture non contiene il file `gdb`: non e' un FileGDB")
    if not any(nome.endswith(".gdbtable") for nome in contenuti):
        errori.append("la fixture non contiene nessuna tabella `.gdbtable`")
    if not contenuti:
        errori.append("archivio senza parti: non c'e' niente da materializzare")

    for messaggio in errori:
        print(messaggio, file=sys.stderr)
    if errori:
        return 1
    print(
        f"fixture FileGDB verificata: {len(contenuti)} parti, "
        f"{sum(len(v) for v in contenuti.values())} byte di contenuto."
    )
    return 0


def main(argv: list[str] | None = None) -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    modo = argomenti.add_mutually_exclusive_group()
    modo.add_argument("--scrivi", type=pathlib.Path, metavar="GDB")
    modo.add_argument("--verifica", action="store_true")
    modo.add_argument("--confronta", nargs=2, type=pathlib.Path, metavar=("GDB1", "GDB2"))
    opzioni = argomenti.parse_args(argv)

    if opzioni.scrivi:
        return scrivi(opzioni.scrivi)
    if opzioni.confronta:
        if not ARCHIVIO.exists():
            print(f"{ARCHIVIO}: fixture assente", file=sys.stderr)
            return 1
        errori = confronta(
            spacchetta(ARCHIVIO.read_bytes()),
            parti(opzioni.confronta[0]),
            parti(opzioni.confronta[1]),
        )
        for messaggio in errori:
            print(messaggio, file=sys.stderr)
        if errori:
            return 1
        print(
            "fixture riproducibile: le differenze dalla rigenerazione stanno "
            "tutte fra i byte che GDAL conia a ogni corsa."
        )
        return 0
    return verifica_forma()


if __name__ == "__main__":
    sys.exit(main())
