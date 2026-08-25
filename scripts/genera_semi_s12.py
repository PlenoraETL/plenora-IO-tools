#!/usr/bin/env python3
"""I semi del lotto S12: i due confini limitati durante il parse.

# Perche' derivati e non committati come blob

Un seme committato a mano e' un blob di cui nessuno sa piu' che cosa dovesse
dire. Qui i byte vengono dalla **specifica** -- la grammatica WKT dell'OGC e
`RFC 7946` per il GeoJSON -- e non dal writer dei driver, che li renderebbe
validi per costruzione anche il giorno in cui sbagliasse.

`--verifica` li ricontrolla byte a byte: se un seme sul disco non e' quello che
questa specifica produce, il gate e' rosso.

# Che cosa devono raggiungere

Non sono un campionario di geometrie: sono gli **ingressi che portano il target
dove la misura di profondita' dice che arriva**. Ogni famiglia ha il suo scopo:

* le forme complete di ciascun tipo, per l'analisi e la conversione;
* le forme ostili -- troncate, con dimensionalita' mista, con tipi inventati --
  per i rami di rifiuto;
* l'annidamento oltre il tetto, per il budget della profondita', che e' l'unico
  dei tre raggiungibile da un input piu' corto del cap del harness.

Il tetto sui componenti non e' qui, e non e' una dimenticanza: centomila
coordinate non stanno in un input da quattro kilobyte. A provarlo sono le sonde
unitarie dei due moduli, che dal default derivano quote strette.

# Uso

    python3 scripts/genera_semi_s12.py
    python3 scripts/genera_semi_s12.py --verifica
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SEMI_WKT = ROOT / "fuzz" / "seeds" / "wkt_parse"
SEMI_GEOJSON = ROOT / "fuzz" / "seeds" / "geojson_reader"

# Il prefisso dei semi di questo file: gli altri nella stessa directory non
# sono suoi. Per `wkt_parse` ce ne sono due, precedenti, che restano fuori.
PREFISSO = "s12-"

# Il tetto sull'annidamento che i due target dichiarano. Il seme deve
# superarlo, non sfiorarlo: fermarsi al tetto proverebbe il caso ammesso.
#
# I due numeri sono diversi, e la differenza e' misurata. Il WKT usa quello del
# bordo, 64: sessantacinque `GEOMETRYCOLLECTION` stanno in milleduecento byte.
# Il GeoJSON no: `serde_json` ha un limite di ricorsione suo -- 128 livelli
# JSON, e ogni livello GeoJSON ne costa due -- quindi oltre i sessantadue e'
# **lui** a rifiutare per primo. Con 64 il nostro tetto non morderebbe mai, e la
# campagna non lo eserciterebbe: il punto d'ingresso del fuzzing ne dichiara 32,
# e il seme ne porta 34.
PROFONDITA_WKT = 64
PROFONDITA_GEOJSON = 32


def semi_wkt() -> dict[str, str]:
    """I testi WKT, uno per famiglia di ramo."""
    annidato = "POINT (1 2)"
    for _ in range(PROFONDITA_WKT + 2):
        annidato = f"GEOMETRYCOLLECTION ({annidato})"

    return {
        "punto-xy": "POINT (1 2)",
        "punto-xyz": "POINT Z (1 2 3)",
        "punto-xym": "POINT M (1 2 3)",
        "punto-negativo": "POINT (-1 -2)",
        "linea-negativa": "LINESTRING (-1.5 2.5,-3 -4)",
        "punto-xyzm": "POINT ZM (1 2 3 4)",
        # La forma senza spazio fra il tipo e il suffisso: e' quella che la
        # sonda comparativa ha trovato mancante, e vive in file veri.
        "punto-suffisso-attaccato": "POINTZ (1 2 3)",
        "linea": "LINESTRING (0 0,1 1,2 2)",
        "linea-vuota": "LINESTRING EMPTY",
        "poligono": "POLYGON ((0 0,1 0,1 1,0 0))",
        "poligono-due-anelli": "POLYGON ((0 0,1 0,1 1,0 0),(0 0,1 0,1 1,0 0))",
        "multipunto-nudo": "MULTIPOINT (1 2,3 4)",
        "multipunto-fra-parentesi": "MULTIPOINT ((1 2),(3 4))",
        "multilinea": "MULTILINESTRING ((0 0,1 1),(2 2,3 3))",
        "multipoligono": "MULTIPOLYGON (((0 0,1 0,1 1,0 0)))",
        "collezione": "GEOMETRYCOLLECTION (POINT (1 2),LINESTRING (0 0,1 1))",
        "collezione-annidata": "GEOMETRYCOLLECTION (GEOMETRYCOLLECTION (POINT (1 2)))",
        # I rifiuti.
        "rifiuto-punto-vuoto": "POINT EMPTY",
        "rifiuto-dimensioni-miste": "LINESTRING (0 0,1 1 1)",
        "rifiuto-tipo-ignoto": "CIRCULARSTRING (0 0,1 1,2 2)",
        "rifiuto-troncato": "POLYGON ((0 0,1 0,1 1",
        "rifiuto-testo-residuo": "POINT (1 2))",
        "rifiuto-coordinata-non-numerica": "POINT (1 due)",
        # Il budget della profondita', l'unico raggiungibile da qui.
        "tetto-annidamento": annidato,
    }


def _feature(geometria: object) -> str:
    """Una `FeatureCollection` di una feature, che e' cio' che il target legge."""
    documento = {
        "type": "FeatureCollection",
        "features": [
            {
                "type": "Feature",
                "geometry": geometria,
                "properties": {"nome": "seme"},
            }
        ],
    }
    return json.dumps(documento, ensure_ascii=False, separators=(",", ":"))


def semi_geojson() -> dict[str, str]:
    """I documenti GeoJSON, uno per famiglia di ramo."""
    annidata: object = {"type": "Point", "coordinates": [1, 2]}
    for _ in range(PROFONDITA_GEOJSON + 2):
        annidata = {"type": "GeometryCollection", "geometries": [annidata]}

    anello = [[0, 0], [1, 0], [1, 1], [0, 0]]
    return {
        "punto-xy": _feature({"type": "Point", "coordinates": [1, 2]}),
        "punto-xyz": _feature({"type": "Point", "coordinates": [1, 2, 3]}),
        # `serde_json` consegna un intero non negativo come `u64` e uno
        # negativo come `i64`: senza un seme negativo meta' del visitor non
        # viene mai eseguita, e le longitudini negative sono la meta' del mondo.
        "punto-negativo": _feature({"type": "Point", "coordinates": [-1, -2]}),
        "linea-negativa": _feature(
            {"type": "LineString", "coordinates": [[-1.5, 2.5], [-3, -4]]}
        ),
        "linea": _feature({"type": "LineString", "coordinates": [[0, 0], [1, 1]]}),
        "multipunto": _feature({"type": "MultiPoint", "coordinates": [[0, 0], [1, 1]]}),
        "poligono": _feature({"type": "Polygon", "coordinates": [anello]}),
        "poligono-due-anelli": _feature(
            {"type": "Polygon", "coordinates": [anello, anello]}
        ),
        "multilinea": _feature(
            {"type": "MultiLineString", "coordinates": [[[0, 0], [1, 1]], [[2, 2], [3, 3]]]}
        ),
        "multipoligono": _feature({"type": "MultiPolygon", "coordinates": [[anello]]}),
        "collezione": _feature(
            {
                "type": "GeometryCollection",
                "geometries": [
                    {"type": "Point", "coordinates": [1, 2]},
                    {"type": "LineString", "coordinates": [[0, 0], [1, 1]]},
                ],
            }
        ),
        # Le chiavi in ordine inverso: in JSON non hanno ordine, ed e' la
        # ragione per cui l'albero delle coordinate esiste.
        "chiavi-invertite": _feature(
            {"coordinates": [[0, 0], [1, 1]], "type": "LineString"}
        ),
        "geometria-nulla": _feature(None),
        # I rifiuti.
        "rifiuto-tipo-ignoto": _feature({"type": "Punto", "coordinates": [1, 2]}),
        "rifiuto-senza-tipo": _feature({"coordinates": [1, 2]}),
        "rifiuto-dimensioni-miste": _feature(
            {"type": "LineString", "coordinates": [[1, 2], [1, 2, 3]]}
        ),
        "rifiuto-quattro-ordinate": _feature(
            {"type": "Point", "coordinates": [1, 2, 3, 4]}
        ),
        "rifiuto-annidamento-del-tipo": _feature(
            {"type": "Point", "coordinates": [[1, 2]]}
        ),
        "rifiuto-troncato": _feature({"type": "Point", "coordinates": [1, 2]})[:-3],
        # Il budget della profondita'.
        "tetto-annidamento": _feature(annidata),
    }


def contenuti() -> dict[Path, dict[str, bytes]]:
    """`{directory: {nome file: byte}}`."""
    return {
        SEMI_WKT: {
            f"{PREFISSO}{nome}.wkt": testo.encode("utf-8")
            for nome, testo in semi_wkt().items()
        },
        SEMI_GEOJSON: {
            f"{PREFISSO}{nome}.geojson": testo.encode("utf-8")
            for nome, testo in semi_geojson().items()
        },
    }


def _dove(directory: Path) -> str:
    try:
        return directory.relative_to(ROOT).as_posix()
    except ValueError:
        return directory.as_posix()


def scrivi() -> int:
    quanti = 0
    for directory, file in contenuti().items():
        directory.mkdir(parents=True, exist_ok=True)
        for nome, byte in file.items():
            (directory / nome).write_bytes(byte)
        quanti += len(file)
        print(f"semi S12 scritti in {_dove(directory)}: {len(file)}")
    return 0 if quanti else 1


def verifica() -> int:
    errori: list[str] = []
    quanti = 0
    for directory, attesi in contenuti().items():
        quanti += len(attesi)
        for nome, byte in attesi.items():
            percorso = directory / nome
            if not percorso.is_file():
                errori.append(f"{nome}: seme assente")
                continue
            trovato = percorso.read_bytes()
            if trovato != byte:
                errori.append(
                    f"{nome}: {len(trovato)} byte sul disco, {len(byte)} dalla "
                    "specifica. Un seme che non e' quello che la specifica "
                    "produce descrive un caso che nessuno ha piu' scritto."
                )
        orfani = sorted(
            p.name
            for p in directory.glob(f"{PREFISSO}*")
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
        f"semi S12 verificati: {quanti} file, byte a byte dalla grammatica WKT "
        "dell'OGC e da RFC 7946"
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
