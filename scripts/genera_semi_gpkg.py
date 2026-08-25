#!/usr/bin/env python3
"""I semi del target `gpkg_geometry` che riguardano le collezioni (lotto S11).

# Perche' derivati e non committati come blob

Un seme committato a mano e' un blob di cui nessuno sa piu' che cosa dovesse
dire: il giorno in cui il driver cambia, nessuno puo' distinguere un seme che
descrive un caso reale da uno scritto male. Qui i byte vengono dalla
**specifica** -- `GeoPackageBinaryHeader` (GeoPackage 1.3, clausola 2.1.3.1.1)
e ISO WKB per il payload -- e non dal writer del driver, che li renderebbe
validi per costruzione anche il giorno in cui sbagliasse.

`--verifica` li ricontrolla byte a byte: se un seme sul disco non e' quello che
questa specifica produce, il gate e' rosso.

# Il perimetro

Solo i semi del lotto S11: le forme in cui una collezione e' vuota pur
dichiarando figli, e le forme ostili che la discesa nei figli deve rifiutare
chiudendo. I cinque semi storici -- `point-xy-le`, `point-xyzm-iso`,
`polygon-xy`, `polygon-xy-envelope`, `geometrycollection-xy` -- sono
precedenti a questo file e restano fuori: dichiararli qui senza averli
derivati vorrebbe dire firmare byte che non ho prodotto.

# Uso

    python3 scripts/genera_semi_gpkg.py
    python3 scripts/genera_semi_gpkg.py --verifica
"""

from __future__ import annotations

import argparse
import struct
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SEMI = ROOT / "fuzz" / "seeds" / "gpkg_geometry"

# Il prefisso dei semi di questo file. Serve a `--verifica` per sapere di quali
# semi risponde: gli altri nella stessa directory non sono suoi.
PREFISSO = "s11-"

# --- la specifica, in due funzioni -----------------------------------------


def intestazione_gpkg(srs_id: int = 4326, vuota: bool = False) -> bytes:
    """`StandardGeoPackageBinary` senza envelope: otto byte.

    Bit 0 dei flag = little-endian; bit 4 = geometria vuota. Il bit "vuota" e'
    proprio cio' che la classificazione ricorsiva decide, e un seme che lo
    porta sbagliato e' un ingresso legittimo: il target non lo confronta, lo
    attraversa.
    """
    flag = 0x11 if vuota else 0x01
    return b"GP" + bytes([0, flag]) + struct.pack("<i", srs_id)


def punto(x: float, y: float) -> bytes:
    return b"\x01" + struct.pack("<I", 1) + struct.pack("<dd", x, y)


def punto_vuoto() -> bytes:
    """`POINT EMPTY`, che sul filo e' `POINT (NaN NaN)`."""
    return punto(float("nan"), float("nan"))


def collezione(tipo: int, figli: list[bytes]) -> bytes:
    corpo = b"".join(figli)
    return b"\x01" + struct.pack("<I", tipo) + struct.pack("<I", len(figli)) + corpo


def poligono(anelli: list[int]) -> bytes:
    """Un poligono XY con gli anelli dichiarati, ciascuno con i punti indicati."""
    corpo = b"\x01" + struct.pack("<I", 3) + struct.pack("<I", len(anelli))
    for punti in anelli:
        corpo += struct.pack("<I", punti) + b"\x00" * (punti * 16)
    return corpo


# --- i semi -----------------------------------------------------------------
#
# Ogni voce dice, nel nome, che cosa il target deve attraversare. I primi
# quattro sono le forme che fino a `f7b6d79` venivano classificate «non vuote»
# guardando il solo conteggio di primo livello; gli ultimi tre sono le forme
# ostili che la discesa deve rifiutare senza panicare.


def semi() -> dict[str, bytes]:
    annidata = collezione(7, [collezione(7, [collezione(7, [punto_vuoto()])])])
    troncata = collezione(7, [punto(1.0, 2.0)])[:-4]
    profonda = punto_vuoto()
    for _ in range(70):
        profonda = collezione(7, [profonda])

    return {
        "collection-di-un-punto-vuoto": collezione(7, [punto_vuoto()]),
        "multipoint-di-punti-vuoti": collezione(4, [punto_vuoto(), punto_vuoto()]),
        "multipolygon-di-un-poligono-senza-anelli": collezione(6, [poligono([])]),
        "multipolygon-anello-senza-punti": collezione(6, [poligono([0])]),
        "collection-annidata-vuota": annidata,
        "collection-mista": collezione(7, [punto_vuoto(), punto(1.0, 2.0)]),
        "collection-figlio-troncato": troncata,
        "collection-figlio-mancante": b"\x01" + struct.pack("<I", 7) + struct.pack("<I", 2) + punto(1.0, 2.0),
        "collection-oltre-la-profondita": profonda,
    }


def contenuti() -> dict[str, bytes]:
    """`{nome file: byte}`. L'header porta il bit "vuota" **non** calcolato.

    Il seme e' un ingresso, non un'asserzione: se portasse gia' la risposta,
    descriverebbe un file che qualcuno ha scritto bene invece dell'input che il
    target deve saper attraversare comunque.
    """
    return {
        f"{PREFISSO}{nome}.gpkgb": intestazione_gpkg() + payload
        for nome, payload in semi().items()
    }


def _dove() -> str:
    """Il percorso dei semi, relativo alla radice quando ci sta dentro.

    Le sonde spostano `SEMI` in una directory temporanea: `relative_to`
    solleverebbe, e un helper che fallisce sul proprio messaggio nasconde
    l'esito che stava per stampare."""
    try:
        return SEMI.relative_to(ROOT).as_posix()
    except ValueError:
        return SEMI.as_posix()


def scrivi() -> int:
    SEMI.mkdir(parents=True, exist_ok=True)
    for nome, byte in contenuti().items():
        (SEMI / nome).write_bytes(byte)
    print(f"semi S11 scritti in {_dove()}: {len(contenuti())}")
    return 0


def verifica() -> int:
    attesi = contenuti()
    errori: list[str] = []
    for nome, byte in attesi.items():
        percorso = SEMI / nome
        if not percorso.is_file():
            errori.append(f"{nome}: seme assente")
            continue
        trovato = percorso.read_bytes()
        if trovato != byte:
            errori.append(
                f"{nome}: {len(trovato)} byte sul disco, {len(byte)} dalla "
                "specifica. Un seme che non e' quello che la specifica produce "
                "descrive un caso che nessuno ha piu' scritto."
            )
    # Un seme con il prefisso di questo file ma che il file non dichiara e' un
    # blob orfano: nessuno ne risponde, e la sua presenza si legge come
    # copertura.
    orfani = sorted(
        p.name
        for p in SEMI.glob(f"{PREFISSO}*")
        if p.is_file() and p.name not in attesi
    )
    errori.extend(
        f"{nome}: seme con il prefisso «{PREFISSO}» che questo file non dichiara"
        for nome in orfani
    )
    for messaggio in errori:
        print(messaggio, file=sys.stderr)
    if errori:
        return 1
    print(
        f"semi S11 verificati: {len(attesi)} file, byte a byte dalla specifica "
        "GeoPackage 1.3 e ISO WKB"
    )
    return 0


def main(argv: list[str] | None = None) -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument(
        "--verifica",
        action="store_true",
        help="ricontrolla i semi sul disco invece di riscriverli",
    )
    opzioni = argomenti.parse_args(argv)
    return verifica() if opzioni.verifica else scrivi()


if __name__ == "__main__":
    sys.exit(main())
