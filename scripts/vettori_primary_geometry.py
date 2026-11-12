#!/usr/bin/env python3
"""PRIM-001..009: vettori normativi contro IO, output e assi di rifiuto.

Le catene data-only non espongono operazioni IO. Per gli handoff si verificano
separatamente il bordo io.read sull'input e io.write sull'expected_output di
data: questo non dichiara eseguito il kernel data ne' qualifica interop-fin.
La composizione verifica il trasporto dell'output, non implementa join/union.
"""
import argparse
import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile

import pyarrow as pa
import pyarrow.ipc as ipc
import pyarrow.parquet as pq

from vettori_arrow_data import tabella, esegui, verdetto

KEY = b"plenora.primary_geometry"


def save(table, path, stream=False):
    with pa.OSFile(str(path), "wb") as sink:
        with (ipc.new_stream if stream else ipc.new_file)(sink, table.schema) as writer:
            writer.write_table(table, max_chunksize=1)


def load(path):
    with pa.memory_map(str(path), "r") as source:
        try:
            return ipc.open_file(source).read_all()
        except pa.ArrowInvalid:
            source.seek(0)
            return ipc.open_stream(source).read_all()


def same(actual, expected):
    assert actual.equals(expected), "valori o tipi diversi"
    assert actual.schema.equals(expected.schema, check_metadata=True), (actual.schema, expected.schema)


def parquet_expected(table, geo_text, canonical):
    """Oracolo dal vettore e dal footer sorgente: nessun metadato dall'output Arrow."""
    geo = json.loads(geo_text)
    decoder = json.JSONDecoder()
    start = geo_text.index('"columns"') + len('"columns"')
    start = geo_text.index('{', start) + 1
    raw = {}
    while True:
        while geo_text[start].isspace() or geo_text[start] == ',': start += 1
        if geo_text[start] == '}': break
        name, end = decoder.raw_decode(geo_text, start)
        start = geo_text.index(':', end) + 1
        while geo_text[start].isspace(): start += 1
        _, end = decoder.raw_decode(geo_text, start)
        raw[name] = geo_text[start:end]
        start = end
    fields = []
    for f in table.schema:
        md = dict(f.metadata or {})
        if f.name in geo['columns']:
            col = geo['columns'][f.name]
            assert col['encoding'] == 'WKB'
            types = {'point': 'Point', 'linestring': 'LineString'}
            assert col['geometry_types'] == [types[md[b'plenora.geometry.types'].decode()]]
            identity = md.get(b'plenora.geometry.crs_id')
            if identity == b'EPSG:4326' and md.get(b'plenora.geometry.axis_order') == b'lon_lat':
                identity = b'OGC:CRS84'
                md[b'plenora.geometry.crs_id'] = identity
                md.pop(b'plenora.geometry.crs_definition', None)
                md.pop(b'plenora.geometry.crs_definition_format', None)
            md[b'crs'] = identity
            if identity.startswith(b'EPSG:'): md[b'plenora.geometry.srid'] = identity.split(b':')[1]
            else: md.pop(b'plenora.geometry.srid', None)
            native = {'column': raw[f.name], 'edges': 'planar', 'primary_column': geo['primary_column'], 'version': geo['version']}
            if f.name == geo['primary_column'] and len(geo['columns']) > 1:
                native['altre_colonne'] = ','.join(sorted(n for n in geo['columns'] if n != f.name))
            for k, value in native.items(): md[('plenora.geometry.native.geoparquet.' + k).encode()] = value.encode()
        fields.append(f.with_metadata(md))
    return pa.Table.from_arrays(table.columns, schema=pa.schema(fields, metadata=canonical.schema.metadata))


def rejected(result, category, path):
    assert result["exit"] == 3, result
    assert result["category"] == category, result
    assert result["phase"] == "validate", result
    assert result["remote_effect"] == "none" and result["retry"] == "never", result
    assert not path.exists(), "output pubblicato sul rifiuto"


def command(cli, args):
    result = subprocess.run([str(cli), *map(str, args), "--format", "json"], capture_output=True, text=True, timeout=120)
    envelope = json.loads(result.stdout)
    error = envelope.get("error") or {}
    return {"exit": result.returncode, "category": error.get("category"), "phase": error.get("phase"),
            "remote_effect": error.get("remote_effect"), "retry": (error.get("retry") or {}).get("kind"), "envelope": envelope}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--contracts", type=Path, required=True)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--referto", type=Path, required=True)
    args = parser.parse_args()
    sys.path.insert(0, str(args.contracts / "tools"))
    import arrow_primary as oracle
    results = []
    def record(name, action):
        try:
            action()
            results.append({"vector": name, "status": "pass"})
        except Exception as exc:
            results.append({"vector": name, "status": "fail", "detail": str(exc)})
    def expected(v):
        v = copy.deepcopy(v)
        oracle.canonical(v)
        return tabella(v)
    def transport(v, work, operation="write", stream=False, output_stream=False):
        src, dst = work / "input.arrow", work / "output.arrow"
        dst.unlink(missing_ok=True)
        save(tabella(v), src, stream)
        result = esegui(args.cli, operation, src, dst, output_stream)
        assert verdetto(v, result, oracle) == "conforme", result
        if v["expect"] == "valid":
            same(load(dst), expected(v))
        else:
            assert not dst.exists()
    with tempfile.TemporaryDirectory(prefix="io-primary-") as tmp:
        work = Path(tmp)
        for p in sorted((args.contracts / "vectors/arrow-primary-v1").glob("*.json")):
            v = json.loads(p.read_text())
            for op in ("read", "write"):
                for stream in (False, True):
                    for output_stream in (False, True):
                        record(f"{p.stem}/{op}/in-{stream}/out-{output_stream}", lambda v=v,op=op,stream=stream,output_stream=output_stream: transport(v, work, op, stream, output_stream))
        renamed = json.loads((args.contracts / "vectors/arrow-primary-v1/primary-second-of-two.json").read_text())
        renamed["fields"][2]["name"] = "renamed_position"
        for row in renamed["rows"]: row["renamed_position"] = row.pop("position")
        renamed["fields"] = list(reversed(renamed["fields"]))
        record("local/rename-reorder-nonpositional-identity", lambda: transport(renamed, work))
        for name in ("primary-second-of-two", "primary-none-one-geometry", "primary-none-two-geometries"):
            source_path = args.contracts / f"vectors/arrow-primary-v1/{name}.json"
            plain = json.loads(source_path.read_text())
            def plain_parquet(plain=plain):
                src, dst = work / "plain.parquet", work / "plain.arrow"
                dst.unlink(missing_ok=True)
                pq.write_table(tabella(plain), src)
                result = command(args.cli, ["read", src, "--output", dst])
                assert result["exit"] == 0, result
                same(load(dst), expected(plain))
            record(f"local/plain-parquet/{name}", plain_parquet)
        def dangling_parquet():
            src, dst = work / "dangling.parquet", work / "dangling.arrow"
            malformed = tabella(renamed)
            malformed = malformed.replace_schema_metadata(dict(malformed.schema.metadata, **{"plenora.primary_geometry": "999"}))
            pq.write_table(malformed, src)
            rejected(command(args.cli, ["read", src, "--output", dst]), "schema", dst)
        record("local/plain-parquet/dangling-before-retag", dangling_parquet)
        for sink in ("gpkg", "shp", "csv", "xls", "geojson", "dxf", "kml"):
            def single_none(sink=sink):
                src, dst = work / "none.arrow", work / ("none." + sink)
                source = json.loads((args.contracts / "vectors/arrow-primary-v1/primary-none-one-geometry.json").read_text())
                save(tabella(source), src)
                rejected(command(args.cli, ["write", src, dst, "--to", sink]), "unsupported", dst)
            record(f"local/none-single-sink/{sink}", single_none)
        for p in sorted((args.contracts / "vectors/interop-primary-v1").glob("*.json")):
            v = json.loads(p.read_text())
            io = [s for s in v["chain"] if s["component"] == "plenora-io-tools"]
            if not io:
                results.append({"vector": p.stem, "status": "not_applicable", "reason": "catena esclusivamente data; IO non espone quei kernel"})
                continue
            source = json.loads((p.parent / v["input"]).read_text())
            if v["kind"] == "handoff":
                record(p.stem + "/io.read", lambda: transport(source, work, "read"))
                output = dict(v["expected_output"], expect="valid")
                record(p.stem + "/io.write-data-output", lambda: transport(output, work))
            elif io[0]["operation"] == "io.read":
                record(p.stem, lambda: transport(source, work, "read"))
            else:
                def refusal():
                    src, dst = work / "input.arrow", work / "refused.gpkg"
                    save(tabella(source), src)
                    rejected(command(args.cli, ["write", src, dst, "--to", "gpkg"]), "unsupported", dst)
                record(p.stem, refusal)
        seed = json.loads((args.contracts / "vectors/arrow-primary-v1/primary-none-one-geometry.json").read_text())
        for p in sorted((args.contracts / "vectors/primary-composition-v1").glob("*.json")):
            v = json.loads(p.read_text())
            assert not oracle.composition_errors(v), (p.name, oracle.composition_errors(v))
            table = {"expect": "valid", "schema_metadata": {"plenora.contract.version": "1"}, "fields": [], "rows": []}
            if v["expected_key"] is not None:
                table["schema_metadata"][KEY.decode()] = v["expected_key"]
            row = {}
            for identity in dict.fromkeys(m["output"] for m in v["mapping"]):
                f = copy.deepcopy(seed["fields"][1]); f["name"] = "g" + identity
                f["metadata"]["plenora.field_id"] = identity
                table["fields"].append(f); row[f["name"]] = seed["rows"][0]["position"]
            table["rows"] = [row]
            record(p.stem + "/output-transport", lambda: transport(table, work))
        for p in sorted((args.contracts / "vectors/geoparquet-primary-v1").glob("*.json")):
            v = json.loads(p.read_text())
            table = json.loads((p.parent / v["table"]).read_text())
            # I vettori sono corrispondenze: completamento della fixture file
            # con il PROJJSON 32632 gia' governato nel repository IO.
            fixture = Path(__file__).resolve().parents[1] / "crates/driver-geoparquet/tests/fixtures/multigeometria-pyarrow.parquet"
            pj = json.loads(pq.read_metadata(fixture).metadata[b"geo"])["columns"]["traccia"]["crs"]
            for field in table["fields"]:
                md = field.get("metadata", {})
                if md.get("plenora.geometry.crs_id") == "EPSG:32632":
                    md["plenora.geometry.crs_definition_format"] = "projjson"
                    md["plenora.geometry.crs_definition"] = json.dumps(pj, separators=(",", ":"))
            def geoparquet():
                src, dst, back = work / "input.arrow", work / "output.parquet", work / "back.arrow"
                dst.unlink(missing_ok=True); back.unlink(missing_ok=True)
                save(tabella(table), src)
                if v["direction"] == "read":
                    independent = tabella(table)
                    fields = []
                    for f in independent.schema:
                        fm = dict(f.metadata or {})
                        if fm.get(b"plenora.geometry.crs_id") == b"EPSG:4326" and fm.get(b"plenora.geometry.axis_order") == b"lon_lat":
                            fm[b"plenora.geometry.crs_id"] = b"OGC:CRS84"
                        fields.append(f.with_metadata(fm))
                    independent = pa.Table.from_arrays(independent.columns, schema=pa.schema(fields, metadata=independent.schema.metadata))
                    md = dict(independent.schema.metadata or {})
                    md.pop(KEY, None)
                    columns = {}
                    for field in independent.schema:
                        fm = field.metadata or {}
                        if fm.get(b"ARROW:extension:name") != b"geoarrow.wkb": continue
                        crs = json.loads(fm[b"plenora.geometry.crs_definition"]) if b"plenora.geometry.crs_definition" in fm else None
                        columns[field.name] = {"encoding": "WKB", "geometry_types": [{"point":"Point", "linestring":"LineString"}[fm[b"plenora.geometry.types"].decode()]]}
                        if crs is not None: columns[field.name]["crs"] = crs
                    md[b"geo"] = json.dumps({"version": "1.1.0", "primary_column": v["geo"]["primary_column"], "columns": columns}, separators=(",", ":")).encode()
                    independent = independent.replace_schema_metadata(md)
                    pq.write_table(independent, dst)
                    assert KEY not in (pq.read_schema(dst).metadata or {})
                    result = {"exit": 0}
                else:
                    result = command(args.cli, ["write", src, dst, "--to", "geoparquet"])
                if "expected_error" in v:
                    rejected(result, v["expected_error"]["category"], dst); return
                assert result["exit"] == 0, result
                meta = pq.read_metadata(dst).metadata
                geo = json.loads(meta[b"geo"])
                assert geo["primary_column"] == v["geo"]["primary_column"]
                assert set(geo["columns"]) == set(v["geo"]["columns"])
                result = command(args.cli, ["read", dst, "--output", back])
                assert result["exit"] == 0, result
                observed = load(back)
                wanted = parquet_expected(tabella(table), meta[b"geo"].decode(), expected(table))
                assert observed.equals(wanted, check_metadata=True), (observed.schema, wanted.schema)
            record(p.stem + "/real-file", geoparquet)
        def renamed_geo_roundtrip():
            fixture = Path(__file__).resolve().parents[1] / "crates/driver-geoparquet/tests/fixtures/multigeometria-pyarrow.parquet"
            raw, src, dst, back = (work / n for n in ("fixture.arrow", "renamed.arrow", "renamed.parquet", "renamed-back.arrow"))
            assert command(args.cli, ["read", fixture, "--output", raw])["exit"] == 0
            original = load(raw)
            identity = original.schema.metadata[KEY]
            index = next(i for i, f in enumerate(original.schema) if f.metadata.get(b"plenora.field_id") == identity)
            names = original.column_names.copy(); names[index] = "current_primary_name"
            renamed_table = original.rename_columns(names).replace_schema_metadata(original.schema.metadata)
            save(renamed_table, src)
            assert command(args.cli, ["write", src, dst, "--to", "geoparquet"])["exit"] == 0
            native = pq.read_table(dst)
            geo = json.loads(pq.read_metadata(dst).metadata[b"geo"])
            assert geo["primary_column"] == names[index]
            assert command(args.cli, ["read", dst, "--output", back])["exit"] == 0
            observed = load(back)
            assert observed.equals(renamed_table)
            assert observed.schema.metadata[KEY] == identity
            for value in (b"999", b"none"):
                bad, out = work / "conflict.parquet", work / "conflict.arrow"
                md = dict(native.schema.metadata); md[KEY] = value
                pq.write_table(native.replace_schema_metadata(md), bad)
                rejected(command(args.cli, ["read", bad, "--output", out]), "schema", out)
        record("local/geoparquet-rename-stale-provenance-and-embedded-conflict", renamed_geo_roundtrip)
    args.referto.write_text(json.dumps({"results": results, "handoff_scope": "IO boundaries; data kernels not executed"}, indent=2) + "\n")
    failures = [r for r in results if r["status"] == "fail"]
    print(json.dumps({s: sum(r["status"] == s for r in results) for s in ("pass", "fail", "not_applicable")}))
    for r in failures: print(r)
    return bool(failures)

if __name__ == "__main__":
    sys.exit(main())
