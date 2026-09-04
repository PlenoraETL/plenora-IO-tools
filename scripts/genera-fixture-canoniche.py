#!/usr/bin/env python3
"""Genera le fixture canoniche delle conversioni. **Non gira in CI.**

# Che cosa fa, e che cosa non fa

Produce le sorgenti valide su cui la matrice cross-format converte. Le produce
con strumenti **indipendenti da plenora-io**: testo scritto a mano dove il
formato e' testo, `zipfile` della libreria standard per XLSX, OGR per i formati
binari spaziali. Nessuna fixture nasce da una conversione del prodotto, e
nessuna nasce da un'altra fixture.

La CI **non** esegue questo script: verifica i byte committati contro i digest
del registro (`scripts/check-fixture-canoniche.py`). Rigenerare in CI
renderebbe l'atteso una funzione dello strumento del giorno, e un atteso che si
aggiorna da solo non e' un atteso.

Aggiornare una fixture e' percio' un'operazione manuale in due passi, visibili
entrambi in review: si esegue questo script, e si aggiorna il digest nel
registro. Se il secondo passo manca, il gate diventa rosso.

# Perche' non plenora-io

Una fixture prodotta dal writer che il reader deve leggere non prova niente: un
difetto simmetrico -- si scrive male e si rilegge male allo stesso modo --
resterebbe invisibile. Le fixture Parquet e Arrow non sono generate qui ma da
`crates/plenora-bench/src/bin/genera_fixture_arrow.rs`, che usa i crate upstream
`arrow`/`parquet` **direttamente**, senza attraversare `driver-geoparquet`.

Quella e' indipendenza dal **codice del prodotto**, non una prova di
interoperabilita' con un'implementazione esterna: arrow-rs e' comunque la
libreria su cui il driver si appoggia. Provare l'interoperabilita' con
un'implementazione diversa -- pyarrow, per dire -- sarebbe un esercizio
distinto, e non e' questo.

# Una famiglia di fixture, non un file replicato dieci volte

Il dataset canonico e' uno **scenario logico** di cinque record. Non e'
l'obbligo che ogni file ne contenga cinque: alcuni formati non possono, e
pretenderlo produrrebbe fixture impoverite in silenzio invece che ristrette per
decisione.

Un formato puo' avere piu' di una fixture quando il proprio modello lo richiede,
e ogni fixture dichiara nel registro quali record rappresenta e perche'.

## Lo Shapefile, e la degradazione che non fa rumore

La specifica ESRI fissa **un solo `ShapeType` per file**, e `Point` (1) e
`PointZ` (11) sono tipi distinti: tutti i record non nulli di uno `.shp` devono
condividerlo.

Chiedendo a OGR 3.6.2 di scrivere insieme un punto 2D e un punto Z, non
rifiuta -- **degrada**:

    sorgente:  POINT Z (1652000 4852000 125.5)
    scritto:   POINT   (1652000 4852000)
    Geometry: Point    Feature Count: 2

Nessun errore, nessun avviso, la Z sparita. E' la ragione per cui i punti 2D e i
punti Z sono **due fixture distinte** con `-nlt` esplicito, e non una sola
affidata a una promozione implicita: una fixture che avesse gia' perso la Z
renderebbe verde ogni oracle scritto su di essa.

# Le tre varianti, e perche' sono tre

Il contenuto canonico e' uno; le sue rappresentazioni sono tre, perche' la
classe di CRS e' una proprieta' del **formato** e non del test.

* `proiettata` -- EPSG:3003, per i sei formati che incorporano il CRS;
* `geografica` -- EPSG:4326, per `geojson` e `kml`, che il CRS lo fissano;
* `senza_crs` -- gli stessi numeri della proiettata, per `csv` e `xls`, che il
  CRS non lo rappresentano e lo esigono da fuori.

Le coordinate della variante geografica **non** sono riproiettate qui: sono i
valori congelati che `assurance/registries/fixture-canoniche.json` registra,
calcolati una volta sola con GDAL/OSR e PROJ. Riproiettare a ogni generazione
legherebbe le fixture alla versione di PROJ installata quel giorno.
"""

from __future__ import annotations

import argparse
import pathlib
import shutil
import sqlite3  # noqa: F401 -- dichiarato: OGR scrive il GPKG, e questo lo dice
import subprocess
import sys
import zipfile

RADICE = pathlib.Path(__file__).resolve().parent.parent
DESTINAZIONE = RADICE / "crates" / "plenora-io-cli" / "tests" / "fixtures" / "canoniche"

# --- il contenuto canonico ---------------------------------------------------
#
# Cinque righe, scelte perche' ciascuna porta un caso che le conversioni
# possono perdere in silenzio. `id` e' l'identificatore stabile con cui i test
# appaiano le righe: l'ordine non e' garantito da nessun driver, e appaiare per
# posizione proverebbe l'ordine invece dei valori.
#
# `intero_largo` porta 9007199254740993, cioe' 2^53+1: il primo intero che un
# float64 non rappresenta. Un formato che lo facesse passare da un double lo
# restituirebbe come 9007199254740992, e la differenza di uno e' esattamente
# cio' che un confronto per valore trova e uno per ordine di grandezza no.
RIGHE = [
    {
        "id": "r1",
        "codice": "A-1",
        "etichetta": "città",
        "intero_largo": 9007199254740993,
        "conteggio": 7,
        "misura": 1.5,
        "attivo": True,
        "istante": "2026-01-15",
        "geometria": "POINT",
    },
    {
        "id": "r2",
        "codice": "B-2",
        "etichetta": None,
        "intero_largo": -9007199254740993,
        "conteggio": None,
        "misura": -0.125,
        "attivo": False,
        "istante": "2026-02-28",
        "geometria": "LINESTRING",
    },
    {
        "id": "r3",
        "codice": "Ç-3",
        "etichetta": "naïve",
        "intero_largo": 0,
        "conteggio": 0,
        "misura": None,
        "attivo": None,
        "istante": "2026-03-01",
        "geometria": "POLYGON",
    },
    {
        "id": "r4",
        "codice": "D-4",
        "etichetta": "",
        "intero_largo": 1,
        "conteggio": -3,
        "misura": 3.141592653589793,
        "attivo": True,
        "istante": "2026-12-31",
        "geometria": "POINT_Z",
    },
    {
        "id": "r5",
        "codice": "E-5",
        "etichetta": "senza geometria",
        "intero_largo": 9007199254740992,
        "conteggio": 42,
        "misura": 0.0,
        "attivo": False,
        "istante": "2026-06-30",
        "geometria": None,
    },
]

# --- le geometrie, nelle due proiezioni --------------------------------------
#
# I valori geografici sono **congelati**: calcolati una volta con GDAL 3.6.2 e
# PROJ 9.1.1, ordine assi `lon, lat`, e registrati in
# `assurance/registries/fixture-canoniche.json` con strumento, versione e
# comando. Qui sono copiati da li' e non ricalcolati.
GEOMETRIE_PROIETTATE = {
    "POINT": "POINT (1650000 4850000)",
    "LINESTRING": "LINESTRING (1650000 4850000, 1650100 4850100)",
    "POLYGON": (
        "POLYGON ((1651000 4851000, 1651100 4851000, 1651100 4851100, "
        "1651000 4851100, 1651000 4851000))"
    ),
    "POINT_Z": "POINT Z (1652000 4852000 125.5)",
}

GEOMETRIE_GEOGRAFICHE = {
    "POINT": "POINT (10.863909301 43.787719185)",
    "LINESTRING": (
        "LINESTRING (10.863909301 43.787719185, 10.865179468 43.788598806)"
    ),
    "POLYGON": (
        "POLYGON ((10.876612647 43.796514729, 10.877855 43.79649432, "
        "10.877883186 43.797394202, 10.876640815 43.797414612, "
        "10.876612647 43.796514729))"
    ),
    "POINT_Z": "POINT Z (10.889319718 43.805308781 125.5)",
}

# Il secondo layer, per i due formati multi-layer. Schema diverso dal primo,
# perche' un secondo layer con lo stesso schema non distingue una selezione
# sbagliata da una giusta.
RIGHE_SECONDARIE = [
    {"id": "s1", "nota": "primo", "peso": 10},
    {"id": "s2", "nota": "secondo", "peso": 20},
]

CRS_PROIETTATO = "EPSG:3003"
CRS_GEOGRAFICO = "EPSG:4326"


# Quali record ogni fixture rappresenta. Le righe assenti **non** sono una
# perdita della conversione: sono il perimetro dichiarato del caso, e il registro
# lo dice. Confonderle produrrebbe un `LossReport` atteso che accusa la
# conversione di ciò che la fixture non le ha mai dato.
RESTRIZIONI = {
    "canonico": ["r1", "r2", "r3", "r4", "r5"],
    "canonico_punti": ["r1", "r5"],
    "canonico_punti_z": ["r4", "r5"],
    "canonico_linee": ["r2"],
    "canonico_poligoni": ["r3"],
}


def _righe(nome: str) -> list[dict]:
    ammessi = RESTRIZIONI[nome]
    return [r for r in RIGHE if r["id"] in ammessi]


def _wkt(riga: dict, geometrie: dict) -> str:
    forma = riga["geometria"]
    return geometrie[forma] if forma else ""


# =============================================================== testo =======


def scrivi_csv(percorso: pathlib.Path) -> None:
    """Scritto a mano: il CSV e' testo, e un CSV generato da uno strumento
    porterebbe le convenzioni di quello strumento invece delle nostre."""
    righe = ["id,codice,etichetta,intero_largo,conteggio,misura,attivo,istante,geometry"]
    for r in RIGHE:
        righe.append(
            ",".join(
                [
                    r["id"],
                    r["codice"],
                    "" if r["etichetta"] is None else f'"{r["etichetta"]}"',
                    str(r["intero_largo"]),
                    "" if r["conteggio"] is None else str(r["conteggio"]),
                    "" if r["misura"] is None else repr(r["misura"]),
                    "" if r["attivo"] is None else ("true" if r["attivo"] else "false"),
                    r["istante"],
                    f'"{_wkt(r, GEOMETRIE_PROIETTATE)}"',
                ]
            )
        )
    percorso.write_text("\n".join(righe) + "\n", encoding="utf-8")


def scrivi_geojson(percorso: pathlib.Path) -> None:
    """Scritto a mano, in WGS84: GeoJSON fissa il CRS per specifica."""
    import json

    caratteristiche = []
    for r in RIGHE:
        geometria = None
        forma = r["geometria"]
        if forma == "POINT":
            geometria = {"type": "Point", "coordinates": [10.863909301, 43.787719185]}
        elif forma == "LINESTRING":
            geometria = {
                "type": "LineString",
                "coordinates": [
                    [10.863909301, 43.787719185],
                    [10.865179468, 43.788598806],
                ],
            }
        elif forma == "POLYGON":
            geometria = {
                "type": "Polygon",
                "coordinates": [
                    [
                        [10.876612647, 43.796514729],
                        [10.877855, 43.79649432],
                        [10.877883186, 43.797394202],
                        [10.876640815, 43.797414612],
                        [10.876612647, 43.796514729],
                    ]
                ],
            }
        elif forma == "POINT_Z":
            geometria = {
                "type": "Point",
                "coordinates": [10.889319718, 43.805308781, 125.5],
            }
        caratteristiche.append(
            {
                "type": "Feature",
                "properties": {
                    "id": r["id"],
                    "codice": r["codice"],
                    "etichetta": r["etichetta"],
                    "intero_largo": r["intero_largo"],
                    "conteggio": r["conteggio"],
                    "misura": r["misura"],
                    "attivo": r["attivo"],
                    "istante": r["istante"],
                },
                "geometry": geometria,
            }
        )
    documento = {"type": "FeatureCollection", "features": caratteristiche}
    percorso.write_text(
        json.dumps(documento, ensure_ascii=False, indent=1) + "\n", encoding="utf-8"
    )


def scrivi_kml(percorso: pathlib.Path) -> None:
    """Scritto a mano, in WGS84: KML fissa il CRS per specifica."""
    coordinate = {
        "POINT": "<Point><coordinates>10.863909301,43.787719185</coordinates></Point>",
        "LINESTRING": (
            "<LineString><coordinates>10.863909301,43.787719185 "
            "10.865179468,43.788598806</coordinates></LineString>"
        ),
        "POLYGON": (
            "<Polygon><outerBoundaryIs><LinearRing><coordinates>"
            "10.876612647,43.796514729 10.877855,43.79649432 "
            "10.877883186,43.797394202 10.876640815,43.797414612 "
            "10.876612647,43.796514729"
            "</coordinates></LinearRing></outerBoundaryIs></Polygon>"
        ),
        "POINT_Z": (
            "<Point><coordinates>10.889319718,43.805308781,125.5</coordinates></Point>"
        ),
    }
    parti = [
        '<?xml version="1.0" encoding="UTF-8"?>',
        '<kml xmlns="http://www.opengis.net/kml/2.2"><Document>',
        "<name>canonico</name>",
    ]
    for r in RIGHE:
        dati = "".join(
            f'<SimpleData name="{k}">{"" if r[k] is None else r[k]}</SimpleData>'
            for k in ("id", "codice", "etichetta", "intero_largo", "conteggio", "misura", "attivo", "istante")
        )
        geometria = coordinate.get(r["geometria"], "") if r["geometria"] else ""
        parti.append(
            f"<Placemark><name>{r['id']}</name>"
            f'<ExtendedData><SchemaData schemaUrl="#canonico">{dati}</SchemaData></ExtendedData>'
            f"{geometria}</Placemark>"
        )
    parti.append("</Document></kml>")
    percorso.write_text("\n".join(parti) + "\n", encoding="utf-8")


def scrivi_dxf(percorso: pathlib.Path) -> None:
    """Scritto a mano: il DXF e' testo, a coppie codice/valore.

    Porta solo la geometria: il DXF non ha un modello di attributi, ed e' una
    delle ragioni per cui la sua classe di fedelta' e' `Approximating`.
    """
    parti = ["0", "SECTION", "2", "ENTITIES"]
    parti += ["0", "POINT", "8", "0", "10", "1650000.0", "20", "4850000.0", "30", "0.0"]
    parti += [
        "0", "LWPOLYLINE", "8", "0", "90", "2", "70", "0",
        "10", "1650000.0", "20", "4850000.0",
        "10", "1650100.0", "20", "4850100.0",
    ]
    parti += [
        "0", "LWPOLYLINE", "8", "0", "90", "4", "70", "1",
        "10", "1651000.0", "20", "4851000.0",
        "10", "1651100.0", "20", "4851000.0",
        "10", "1651100.0", "20", "4851100.0",
        "10", "1651000.0", "20", "4851100.0",
    ]
    parti += ["0", "POINT", "8", "0", "10", "1652000.0", "20", "4852000.0", "30", "125.5"]
    parti += ["0", "ENDSEC", "0", "EOF"]
    percorso.write_text("\n".join(parti) + "\n", encoding="ascii")


# =============================================================== xlsx =======


def scrivi_xlsx(percorso: pathlib.Path) -> None:
    """Un XLSX a mano: e' uno zip di XML, e la libreria standard basta.

    Scriverlo a mano invece che con OGR o openpyxl da' il controllo esatto su
    tipi e celle vuote, che e' precisamente cio' che questa fixture deve
    portare: una cella vuota e una cella con stringa vuota **non** sono la
    stessa cosa, e uno scrittore di comodo tende a confonderle.
    """
    intestazioni = [
        "id", "codice", "etichetta", "intero_largo", "conteggio",
        "misura", "attivo", "istante", "geometry",
    ]

    def cella(riferimento: str, valore, tipo: str | None) -> str:
        if valore is None:
            return ""  # cella assente: non e' una stringa vuota
        if tipo == "inlineStr":
            testo = (
                str(valore)
                .replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
            )
            return (
                f'<c r="{riferimento}" t="inlineStr"><is><t xml:space="preserve">'
                f"{testo}</t></is></c>"
            )
        return f'<c r="{riferimento}">{valore}</c>'

    colonne = "ABCDEFGHI"
    righe_xml = [
        "<row r=\"1\">"
        + "".join(cella(f"{colonne[i]}1", h, "inlineStr") for i, h in enumerate(intestazioni))
        + "</row>"
    ]
    for indice, r in enumerate(RIGHE, start=2):
        valori = [
            (r["id"], "inlineStr"),
            (r["codice"], "inlineStr"),
            (r["etichetta"], "inlineStr"),
            # L'intero largo come **testo**: una cella numerica di XLSX e' un
            # double, e 2^53+1 non ci sta. E' il formato a imporlo, e l'oracle
            # lo registra come tale invece di pretendere un numero.
            (str(r["intero_largo"]), "inlineStr"),
            (r["conteggio"], None),
            (r["misura"], None),
            (None if r["attivo"] is None else str(r["attivo"]).lower(), "inlineStr"),
            (r["istante"], "inlineStr"),
            (_wkt(r, GEOMETRIE_PROIETTATE) or None, "inlineStr"),
        ]
        celle = "".join(
            cella(f"{colonne[i]}{indice}", v, t) for i, (v, t) in enumerate(valori)
        )
        righe_xml.append(f'<row r="{indice}">{celle}</row>')

    foglio = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">'
        f"<sheetData>{''.join(righe_xml)}</sheetData></worksheet>"
    )
    tipi = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
        '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
        '<Default Extension="xml" ContentType="application/xml"/>'
        '<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>'
        '<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>'
        "</Types>"
    )
    rels = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>'
        "</Relationships>"
    )
    workbook = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" '
        'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">'
        '<sheets><sheet name="canonico" sheetId="1" r:id="rId1"/></sheets></workbook>'
    )
    workbook_rels = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>'
        "</Relationships>"
    )

    # `ZIP_STORED` e data fissa: due generazioni dello stesso contenuto devono
    # dare gli stessi byte, altrimenti il digest del registro cambierebbe senza
    # che sia cambiato niente.
    percorso.unlink(missing_ok=True)
    with zipfile.ZipFile(percorso, "w", zipfile.ZIP_STORED) as z:
        for nome, testo in (
            ("[Content_Types].xml", tipi),
            ("_rels/.rels", rels),
            ("xl/workbook.xml", workbook),
            ("xl/_rels/workbook.xml.rels", workbook_rels),
            ("xl/worksheets/sheet1.xml", foglio),
        ):
            info = zipfile.ZipInfo(nome, date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_STORED
            z.writestr(info, testo)


# =============================================================== OGR ========


def _csv_per_ogr(
    lavoro: pathlib.Path,
    geometrie: dict,
    crs: str,
    restrizione: str = "canonico",
    nome_file: str = "sorgente.csv",
) -> pathlib.Path:
    """Il CSV che OGR legge per produrre i formati binari.

    Non e' una fixture e non finisce nel repository: e' l'ingresso dello
    strumento indipendente, e vive solo dentro la directory di lavoro.
    """
    percorso = lavoro / nome_file
    righe = ["id,codice,etichetta,intero_largo,conteggio,misura,attivo,istante,WKT"]
    for r in _righe(restrizione):
        righe.append(
            ",".join(
                [
                    r["id"],
                    r["codice"],
                    "" if r["etichetta"] is None else f'"{r["etichetta"]}"',
                    str(r["intero_largo"]),
                    "" if r["conteggio"] is None else str(r["conteggio"]),
                    "" if r["misura"] is None else repr(r["misura"]),
                    "" if r["attivo"] is None else ("true" if r["attivo"] else "false"),
                    r["istante"],
                    f'"{_wkt(r, geometrie)}"',
                ]
            )
        )
    percorso.write_text("\n".join(righe) + "\n", encoding="utf-8")
    return percorso


def _ogr(argomenti: list[str]) -> None:
    esito = subprocess.run(["ogr2ogr", *argomenti], capture_output=True, text=True)
    if esito.returncode != 0:
        raise SystemExit(f"ogr2ogr {' '.join(argomenti)}\n{esito.stderr}")


def scrivi_shp(base: pathlib.Path, lavoro: pathlib.Path) -> list[str]:
    """Due shapefile, omogenei per `ShapeType`, con `-nlt` esplicito.

    `canonico_punti` porta `Point` (r1 2D, r5 senza geometria);
    `canonico_punti_z` porta `PointZ` (r4, r5). Non si affidano a una
    promozione implicita: OGR, messi insieme, degraderebbe la Z in silenzio.

    Il record a geometria nulla sta in entrambe perche' il `ShapeType` vincola i
    record **non nulli**: una geometria assente e' ammessa in un file di
    qualunque tipo, ed e' cio' che rende provabile il null in tutte e due.
    """
    prodotte: list[str] = []
    for nome, restrizione, tipo in (
        ("canonico_punti", "canonico_punti", "POINT"),
        ("canonico_punti_z", "canonico_punti_z", "POINTZ"),
    ):
        percorso = base.parent / f"{nome}.shp"
        for compagno in (".shp", ".shx", ".dbf", ".prj", ".cpg"):
            percorso.with_suffix(compagno).unlink(missing_ok=True)
        sorgente = _csv_per_ogr(
            lavoro, GEOMETRIE_PROIETTATE, CRS_PROIETTATO, restrizione, f"{nome}.csv"
        )
        _ogr([
            "-f", "ESRI Shapefile", str(percorso), str(sorgente),
            "-nlt", tipo, "-a_srs", CRS_PROIETTATO,
            "-oo", "GEOM_POSSIBLE_NAMES=WKT", "-oo", "KEEP_GEOM_COLUMNS=NO",
            "-lco", "ENCODING=UTF-8",
        ])
        prodotte.append(f"{nome}.shp")
    return prodotte


def scrivi_geojson_omogeneo(percorso: pathlib.Path) -> None:
    """Un GeoJSON di soli punti 2D, adatto a uno Shapefile.

    Serve al caso `geojson -> shp`, che deve provare il troncamento dei nomi di
    campo a dieci caratteri: partendo dal GeoJSON misto la conversione si
    fermerebbe sul tipo di geometria e non arriverebbe mai al `write_loss`.
    """
    import json

    caratteristiche = []
    for r in _righe("canonico_punti"):
        geometria = (
            {"type": "Point", "coordinates": [10.863909301, 43.787719185]}
            if r["geometria"]
            else None
        )
        caratteristiche.append({
            "type": "Feature",
            "properties": {
                "id": r["id"], "codice": r["codice"], "etichetta": r["etichetta"],
                "intero_largo": r["intero_largo"], "conteggio": r["conteggio"],
                "misura": r["misura"], "attivo": r["attivo"], "istante": r["istante"],
            },
            "geometry": geometria,
        })
    percorso.write_text(
        json.dumps(
            {"type": "FeatureCollection", "features": caratteristiche},
            ensure_ascii=False, indent=1,
        ) + "\n",
        encoding="utf-8",
    )


def scrivi_gpkg(percorso: pathlib.Path, lavoro: pathlib.Path) -> None:
    """Multi-layer: `principale` con le cinque righe, `secondario` con altro schema.

    GeoPackage ha un tipo `GEOMETRY` generico, quindi il layer principale porta
    lo scenario intero -- punti, linea, poligono, Z e geometria nulla -- ed e'
    l'unica fixture spaziale che lo fa.
    """
    percorso.unlink(missing_ok=True)
    sorgente = _csv_per_ogr(lavoro, GEOMETRIE_PROIETTATE, CRS_PROIETTATO)
    _ogr([
        "-f", "GPKG", str(percorso), str(sorgente), "-nln", "principale",
        "-a_srs", CRS_PROIETTATO, "-oo", "GEOM_POSSIBLE_NAMES=WKT",
        "-oo", "KEEP_GEOM_COLUMNS=NO",
    ])
    secondario = lavoro / "secondario.csv"
    secondario.write_text(
        "id,nota,peso\n"
        + "\n".join(f"{r['id']},{r['nota']},{r['peso']}" for r in RIGHE_SECONDARIE)
        + "\n",
        encoding="utf-8",
    )
    _ogr([
        "-f", "GPKG", "-update", "-append", str(percorso), str(secondario),
        "-nln", "secondario",
    ])


def scrivi_gdb(percorso: pathlib.Path, lavoro: pathlib.Path) -> None:
    """FileGDB: quattro feature class, come si usano davvero.

    Una feature class ha un tipo geometrico solo, quindi punti, linee e poligoni
    stanno in tre classi distinte; `secondario` non ha geometria. Ogni
    conversione deve **nominare** la classe che seleziona: con quattro classi di
    schemi e geometrie diverse, prendere quella sbagliata non passa inosservato.

    E' una directory: il digest e' dell'albero, non di un file.
    """
    if percorso.exists():
        shutil.rmtree(percorso)
    primo = True
    for classe, restrizione, tipo in (
        ("punti", "canonico_punti", "POINT"),
        ("punti_z", "canonico_punti_z", "POINTZ"),
        ("linee", "canonico_linee", "LINESTRING"),
        ("poligoni", "canonico_poligoni", "POLYGON"),
    ):
        sorgente = _csv_per_ogr(
            lavoro, GEOMETRIE_PROIETTATE, CRS_PROIETTATO, restrizione, f"gdb_{classe}.csv"
        )
        argomenti = ["-f", "OpenFileGDB", str(percorso), str(sorgente), "-nln", classe,
                     "-nlt", tipo, "-a_srs", CRS_PROIETTATO,
                     "-oo", "GEOM_POSSIBLE_NAMES=WKT", "-oo", "KEEP_GEOM_COLUMNS=NO"]
        if not primo:
            argomenti[2:2] = ["-update", "-append"]
        _ogr(argomenti)
        primo = False
    secondario = lavoro / "gdb_secondario.csv"
    secondario.write_text(
        "id,nota,peso\n"
        + "\n".join(f"{r['id']},{r['nota']},{r['peso']}" for r in RIGHE_SECONDARIE)
        + "\n",
        encoding="utf-8",
    )
    _ogr([
        "-f", "OpenFileGDB", "-update", "-append", str(percorso), str(secondario),
        "-nln", "secondario",
    ])


# =============================================================== main =======

GENERATE_QUI = ("csv", "geojson", "kml", "dxf", "xlsx", "shp", "gpkg", "gdb")


def main() -> int:
    a = argparse.ArgumentParser(description=__doc__)
    a.add_argument("--lavoro", type=pathlib.Path, required=True,
                   help="directory temporanea per gli ingressi di OGR")
    a.add_argument("--solo", nargs="*", default=list(GENERATE_QUI), choices=GENERATE_QUI)
    arg = a.parse_args()

    DESTINAZIONE.mkdir(parents=True, exist_ok=True)
    arg.lavoro.mkdir(parents=True, exist_ok=True)

    fatte: list[str] = []
    if "csv" in arg.solo:
        scrivi_csv(DESTINAZIONE / "canonico.csv"); fatte.append("canonico.csv")
    if "geojson" in arg.solo:
        scrivi_geojson(DESTINAZIONE / "canonico.geojson"); fatte.append("canonico.geojson")
    if "kml" in arg.solo:
        scrivi_kml(DESTINAZIONE / "canonico.kml"); fatte.append("canonico.kml")
    if "dxf" in arg.solo:
        scrivi_dxf(DESTINAZIONE / "canonico.dxf"); fatte.append("canonico.dxf")
    if "xlsx" in arg.solo:
        scrivi_xlsx(DESTINAZIONE / "canonico.xlsx"); fatte.append("canonico.xlsx")
    if "geojson" in arg.solo:
        scrivi_geojson_omogeneo(DESTINAZIONE / "canonico_punti.geojson")
        fatte.append("canonico_punti.geojson")
    if "shp" in arg.solo:
        fatte.extend(scrivi_shp(DESTINAZIONE / "canonico.shp", arg.lavoro))
    if "gpkg" in arg.solo:
        scrivi_gpkg(DESTINAZIONE / "canonico.gpkg", arg.lavoro); fatte.append("canonico.gpkg")
    if "gdb" in arg.solo:
        scrivi_gdb(DESTINAZIONE / "canonico.gdb", arg.lavoro); fatte.append("canonico.gdb")

    for nome in fatte:
        print(f"  {nome}")
    print(f"{len(fatte)} fixture generate in {DESTINAZIONE.relative_to(RADICE).as_posix()}")
    print("Parquet e Arrow: crates/plenora-bench/src/bin/genera_fixture_arrow.rs")
    print("Ora aggiorna i digest: scripts/check-fixture-canoniche.py --mostra-digest")
    return 0


if __name__ == "__main__":
    sys.exit(main())
