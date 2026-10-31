"""Fixture GeoParquet prodotta con PyArrow, senza il writer IO.

Nessuno dei 38 Parquet tracciati al 28/10/2026 aveva più voci geo.columns.
PyArrow è già usato nell'attrezzaggio del repository; non aggiunge dipendenze
al prodotto. Rigenerazione: python scripts/genera_fixture_multigeometria.py.
"""

import json
import struct
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq


def genera():
    geografico = {
        "type": "GeographicCRS", "name": "WGS 84",
        "datum": {"type": "GeodeticReferenceFrame", "name": "World Geodetic System 1984",
                  "ellipsoid": {"name": "WGS 84", "semi_major_axis": 6378137,
                                "inverse_flattening": 298.257223563}},
        "coordinate_system": {"subtype": "ellipsoidal", "axis": [
            {"name": "Longitude", "abbreviation": "Lon", "direction": "east", "unit": "degree"},
            {"name": "Latitude", "abbreviation": "Lat", "direction": "north", "unit": "degree"}]},
    }
    proiettato = {
        "type": "ProjectedCRS", "name": "WGS 84 / UTM zone 32N", "base_crs": geografico,
        "conversion": {"name": "UTM zone 32N", "method": {"name": "Transverse Mercator"},
                       "parameters": [
                           {"name": "Latitude of natural origin", "value": 0, "unit": "degree"},
                           {"name": "Longitude of natural origin", "value": 9, "unit": "degree"},
                           {"name": "Scale factor at natural origin", "value": 0.9996, "unit": "unity"},
                           {"name": "False easting", "value": 500000, "unit": "metre"},
                           {"name": "False northing", "value": 0, "unit": "metre"}]},
        "coordinate_system": {"subtype": "Cartesian", "axis": [
            {"name": "Easting", "abbreviation": "E", "direction": "east", "unit": "metre"},
            {"name": "Northing", "abbreviation": "N", "direction": "north", "unit": "metre"}]},
        "id": {"authority": "EPSG", "code": 32632},
    }
    geo = {"version": "1.1.0", "primary_column": "posizione", "columns": {
        "traccia": {"encoding": "WKB", "geometry_types": ["LineString"], "crs": proiettato},
        "posizione": {"encoding": "WKB", "geometry_types": ["Point"]}}}
    punto = struct.pack("<BIdd", 1, 1, 9.0, 45.0)
    linea = struct.pack("<BII4d", 1, 2, 2, 500000.0, 4982950.0, 500010.0, 4982960.0)
    schema = pa.schema([pa.field("traccia", pa.binary()), pa.field("id", pa.int64()),
                        pa.field("posizione", pa.binary())],
                       metadata={b"geo": json.dumps(geo, separators=(",", ":")).encode()})
    tabella = pa.Table.from_arrays([pa.array([linea, None, linea]), pa.array([1, 2, 3]),
                                   pa.array([punto, punto, None])], schema=schema)
    percorso = Path(__file__).resolve().parents[1] / "crates/driver-geoparquet/tests/fixtures/multigeometria-pyarrow.parquet"
    with pq.ParquetWriter(percorso, schema, compression="NONE", store_schema=False) as writer:
        writer.add_key_value_metadata({"geo": json.dumps(geo, separators=(",", ":"))})
        writer.write_table(tabella)
    # Seconda fixture: schema Arrow incorporato e identita' diversa dall'indice.
    # L'anello esterno percorre un quadrato in senso antiorario.
    geo["columns"]["traccia"].update({
        "geometry_types": ["Polygon"],
        "bbox": [500000, 4982950, 500010, 4982960],
        "orientation": "counterclockwise",
    })
    geo["columns"]["posizione"]["bbox"] = [9, 45, 9, 45]
    geo["columns"]["posizione"]["orientation"] = "counterclockwise"
    poligono = struct.pack("<BIII10d", 1, 3, 1, 5,
                          500000, 4982950, 500010, 4982950, 500010, 4982960,
                          500000, 4982960, 500000, 4982950)
    schema = pa.schema([
        pa.field("traccia", pa.binary(), metadata={b"plenora.field_id": b"42"}),
        pa.field("id", pa.int64()),
        pa.field("posizione", pa.binary(), metadata={b"plenora.field_id": b"73"}),
    ], metadata={b"geo": json.dumps(geo, separators=(",", ":")).encode()})
    tabella = pa.Table.from_arrays([pa.array([poligono, None, poligono]), pa.array([1, 2, 3]),
                                   pa.array([punto, punto, None])], schema=schema)
    pq.write_table(tabella, percorso.with_name("multigeometria-metadati-pyarrow.parquet"),
                   compression="NONE", store_schema=True)
    print(f"{percorso.name}: PyArrow {pa.__version__}")


if __name__ == "__main__":
    genera()
