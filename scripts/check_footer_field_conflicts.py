#!/usr/bin/env python3
"""End-to-end footer/field conflicts: read and convert, primary and secondary."""
import argparse
import json
from pathlib import Path
import subprocess
import pyarrow as pa
import pyarrow.parquet as pq


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--cli', type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    work = root / 'target/c3-footer-conflicts'; work.mkdir(exist_ok=True)
    source = pq.read_table(root / 'crates/driver-geoparquet/tests/fixtures/multigeometria-metadati-pyarrow.parquet')
    results = []
    for index in (0, 2):
        for prop, value in [('types', b'polygon'), ('encoding', b'ewkb'), ('dimensions', b'xyzm'),
                            ('precision', b'float32'), ('spatial_semantics', b'geography'),
                            ('native.geoparquet.edges', b'spherical'), ('crs', None)]:
            for empty in (False, True):
                table = source.slice(0, 0) if empty else source
                fields = list(table.schema)
                md = dict(fields[index].metadata)
                geo = json.loads(table.schema.metadata[b'geo'])
                if prop == 'types':
                    # Exact counterexample: canonical Polygon + Polygon WKB, footer Point.
                    md[b'plenora.geometry.types'] = b'polygon'
                    md[b'plenora.geometry.types_declaration'] = b'exact'
                    geo['columns'][fields[index].name]['geometry_types'] = ['Point']
                    if not empty:
                        polygon = source.column(0)[0].as_py()
                        table = table.set_column(index, fields[index], pa.array([polygon] * table.num_rows, type=pa.binary()))
                elif prop == 'crs':
                    # Internally consistent foreign CRS, so the footer reconciliation fails.
                    other = source.schema.field(2 if index == 0 else 0).metadata
                    for k in list(md):
                        if b'crs' in k or k.endswith(b'axis_order') or k == b'ARROW:extension:metadata': del md[k]
                    for k, v in other.items():
                        if b'crs' in k or k.endswith(b'axis_order') or k == b'ARROW:extension:metadata': md[k] = v
                else: md[('plenora.geometry.' + prop).encode()] = value
                fields[index] = fields[index].with_metadata(md)
                schema_md = dict(table.schema.metadata); schema_md[b'geo'] = json.dumps(geo).encode()
                table = pa.Table.from_arrays(table.columns, schema=pa.schema(fields, metadata=schema_md))
                src = work / f'{index}-{prop}-{empty}.parquet'; pq.write_table(table, src, use_dictionary=False)
                for operation in ('read', 'convert'):
                    dst = work / 'rejected.arrow'; dst.unlink(missing_ok=True)
                    cmd = [str(args.cli), operation, str(src)]
                    cmd += ['--output', str(dst)] if operation == 'read' else [str(dst), '--from', 'geoparquet', '--to', 'ipc']
                    p = subprocess.run(cmd + ['--format', 'json'], capture_output=True)
                    response = json.loads(p.stdout or p.stderr)
                    ok = p.returncode != 0 and response.get('error', {}).get('category') == 'schema' and not dst.exists()
                    results.append({'column': index, 'property': prop, 'empty': empty, 'operation': operation, 'pass': ok, 'response': response})
    (work / 'report.json').write_text(json.dumps(results, indent=2), encoding='utf-8')
    failures = [r for r in results if not r['pass']]
    print(json.dumps({'pass': len(results)-len(failures), 'fail': len(failures)}))
    for r in failures: print(r)
    return bool(failures)

if __name__ == '__main__':
    raise SystemExit(main())
