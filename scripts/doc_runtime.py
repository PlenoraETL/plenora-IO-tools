"""Casi documentati: attese dichiarate, confrontate con processi CLI reali."""
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
FORMATS = {'csv': ('csv', 'CSV'), 'shp': ('shp', 'Shapefile'),
           'gpkg': ('gpkg', 'GeoPackage'), 'filegdb': ('gdb', 'FileGDB'),
           'geojson': ('geojson', 'GeoJSON'), 'kml': ('kml', 'KML'),
           'dxf': ('dxf', 'DXF'), 'xls': ('xlsx', 'XLSX'),
           'geoparquet': ('parquet', 'Parquet'), 'ipc': ('arrow', 'Arrow IPC')}


def cases(tables):
    """Una riga per situazione/driver; nessuna applicabilita' implicita."""
    rules = {r['situation']: r for r in tables['errors']}
    result = []

    def add(situation, drivers, scenario, message=None):
        for driver in drivers:
            row = dict(rules[situation], driver=driver, scenario=scenario)
            if message is not None:
                row['message'] = message.format(label=FORMATS[driver][1], driver=driver)
            result.append(row)

    add('MissingSource', FORMATS, 'sorgente assente')
    add('InvalidContent', [d for d in FORMATS if d != 'ipc'], 'contenitore vuoto',
        'contenuto vuoto o struttura non valida; formato atteso: {label}')
    add('InvalidContent', ['ipc'], 'contenitore vuoto', 'flusso Arrow IPC troppo corto per il formato')
    add('DirectoryInsteadOfFile', [d for d in FORMATS if d != 'filegdb'], 'directory al posto del file',
        'directory al posto del file; formato atteso: {label}')
    add('InvalidConfiguration', ['csv'], 'WKT e XY insieme',
        'csv: wkt_column esclude x_column e y_column; dichiarare una sola geometria')
    add('MissingOptionColumn', ['csv'], 'colonna WKT assente', "colonna WKT assente dall'intestazione")
    add('ContradictedCrs', ['geojson', 'kml', 'shp', 'dxf', 'gpkg', 'geoparquet', 'ipc'], 'assunzione EPSG:3857 su WGS84')
    add('MissingGeometry', ['shp', 'kml', 'dxf'], 'schema senza geometria')
    add('UnrepresentableWriteType', ['csv', 'shp', 'kml', 'geojson', 'gpkg', 'xls'], 'attributo Duration')
    add('NonFiniteWriteValue', ['geojson'], 'attributo NaN', 'Float64 non finito non rappresentabile in GeoJSON')
    add('ReadMemoryExceeded', ['ipc'], 'memoria 1 byte')
    add('IneffectiveCrsAssumption', ['ipc'], 'assunzione senza geometria')
    add('MissingLayer', ['csv', 'shp', 'kml', 'dxf', 'gpkg', 'geojson', 'geoparquet', 'ipc', 'xls'], 'layer 999', "layer inesistente all'indice 999")
    add('InvalidQuota', FORMATS, 'max_rows zero', 'max_rows deve essere un intero positivo e coerente con le altre quote')
    add('InvalidSpillDirectory', [d for d in FORMATS if d != 'filegdb'], 'directory spill assente')
    add('MissingWriteBackend', ['filegdb'], 'build senza gdal-backend')
    assert set(rules) == {r['situation'] for r in result}
    return result


def run(binary, args, env=None):
    p = subprocess.run([str(binary), *map(str, args)], capture_output=True, env=env, timeout=60)
    assert not p.stderr, p.stderr.decode(errors='replace')
    return p.returncode, json.loads(p.stdout)


def verify(binary, tables):
    import pyarrow as pa
    binary = Path(binary).resolve()
    errors = []
    with tempfile.TemporaryDirectory(prefix='doc-runtime-', dir=ROOT / 'target') as directory:
        work = Path(directory)
        source = work / 'source.geojson'
        source.write_text(json.dumps({'type': 'FeatureCollection', 'features': [
            {'type': 'Feature', 'properties': {'value': 1.5},
             'geometry': {'type': 'Point', 'coordinates': [1, 2]}}]}), encoding='utf-8')
        ipc = work / 'source.arrow'
        code, response = run(binary, ['read', source, '--output', ipc])
        assert code == 0, response
        with pa.memory_map(str(ipc), 'r') as f:
            table = pa.ipc.open_file(f).read_all()
        original_ipc = ipc
        original_table = table
        # EPSG con codice intero e' rappresentabile anche in GeoPackage.
        fields = []
        for field in table.schema:
            md = dict(field.metadata or {})
            if field.type == pa.binary():
                md = {b'plenora.field_id': b'0', b'plenora.geometry.encoding': b'wkb', b'plenora.geometry.dimensions': b'xy',
                      b'plenora.geometry.spatial_semantics': b'geometry', b'plenora.geometry.precision': b'float64',
                      b'plenora.geometry.types_declaration': b'exact', b'plenora.geometry.types': b'point',
                      b'plenora.geometry.crs_resolution': b'resolved', b'plenora.geometry.crs_id': b'EPSG:4326',
                      b'plenora.geometry.axis_order': b'lon_lat', b'plenora.geometry.srid': b'4326',
                      b'ARROW:extension:name': b'geoarrow.wkb'}
            fields.append(field.with_metadata(md))
        table = pa.Table.from_arrays(table.columns, schema=pa.schema(fields, metadata=table.schema.metadata))

        def save(name, value):
            path = work / (name + '.arrow')
            with pa.OSFile(str(path), 'wb') as f, pa.ipc.new_file(f, value.schema) as w:
                w.write_table(value)
            return path

        ipc = save('source-epsg', table)
        no_geometry = save('no-geometry', pa.table({'value': [1]}))
        geometry = next(field for field in table.schema if field.type == pa.binary())
        def attribute(name, array, original=False):
            geom = next(f for f in original_table.schema if f.type == pa.binary()) if original else geometry
            return save(name, pa.Table.from_arrays([table[geometry.name], array], schema=pa.schema([
                geom, pa.field('value', array.type, metadata={b'plenora.field_id': b'1'})], metadata={b'plenora.contract.version': b'1'})))
        duration = attribute('duration', pa.array([1], type=pa.duration('ms')))
        duration_ogc = attribute('duration-ogc', pa.array([1], type=pa.duration('ms')), True)
        nan = attribute('nan', pa.array([float('nan')]), True)
        paths = {'geojson': source, 'ipc': ipc}
        for driver in FORMATS:
            if driver in paths or driver == 'filegdb':
                continue
            path = work / ('source.' + FORMATS[driver][0])
            code, response = run(binary, ['write', original_ipc if driver == 'kml' else ipc, path, '--to', driver] + (['--out-opt', 'publish_mode=loose_shapefile_set'] if driver == 'shp' else []))
            assert code == 0, (driver, response)
            paths[driver] = path
        catalog = run(binary, ['catalog'])[1]['result']['drivers']
        native = next(d for d in catalog if d['id'] == 'filegdb')['available']
        for i, row in enumerate(cases(tables)):
            driver, situation = row['driver'], row['situation']
            ext = FORMATS[driver][0]
            path = paths.get(driver, work / 'absent.gdb')
            env = dict(os.environ)
            args = ['read', path]
            if situation == 'MissingSource':
                args = ['read', work / ('absent.' + ext)]
            elif situation in ('InvalidContent', 'DirectoryInsteadOfFile'):
                path = work / (f'bad-{i}.' + ext)
                if situation == 'DirectoryInsteadOfFile' or driver == 'filegdb':
                    path.mkdir()
                else:
                    path.write_bytes(b'')
                args = ['read', path]
            elif situation == 'InvalidConfiguration':
                args += ['--opt', 'wkt_column=a', '--opt', 'x_column=b', '--opt', 'y_column=c']
            elif situation == 'MissingOptionColumn':
                args += ['--opt', 'wkt_column=absent']
            elif situation == 'ContradictedCrs':
                args += ['--layer', '0', '--assume-crs', 'EPSG:3857']
            elif situation in ('MissingGeometry', 'UnrepresentableWriteType', 'NonFiniteWriteValue', 'MissingWriteBackend'):
                input_path = {'MissingGeometry': no_geometry, 'UnrepresentableWriteType': duration,
                              'NonFiniteWriteValue': nan, 'MissingWriteBackend': ipc}[situation]
                if situation == 'UnrepresentableWriteType' and driver in ('kml', 'geojson'):
                    input_path = duration_ogc
                args = ['write', input_path, work / (f'out-{i}.' + ext), '--to', driver]
                if situation == 'MissingWriteBackend' and native:
                    # La riga dichiara esplicitamente la build senza backend.
                    continue
            elif situation == 'ReadMemoryExceeded':
                args += ['--memory-bytes', '1', '--max-wkb-cell-bytes', '1']
            elif situation == 'IneffectiveCrsAssumption':
                args = ['read', no_geometry, '--assume-crs', 'EPSG:4326']
            elif situation == 'MissingLayer':
                args += ['--layer', '999']
                if driver in ('dxf', 'xls'):
                    args += ['--assume-crs', 'EPSG:4326']
            elif situation == 'InvalidQuota':
                args += ['--max-rows', '0']
            elif situation == 'InvalidSpillDirectory':
                env['PLENORA_SPILL_DIR'] = str(work / 'absent-spill')
            code, observed = run(binary, args, env)
            actual = observed.get('error', {})
            for key in ('category', 'phase', 'code', 'message'):
                if actual.get(key) != row[key]:
                    errors.append(f'{situation}/{driver}: {key}: {actual.get(key)!r} != {row[key]!r}')
            if code != row['exit']:
                errors.append(f'{situation}/{driver}: exit {code} != {row["exit"]}')
        # Campione fisso, distinto dall'enumerazione bidirezionale generale.
        # Le celle contestate e due trasporti Arrow conservativi hanno oracoli
        # indipendenti su tipo/valore riletto o attributo realmente omesso.
        from decimal import Decimal
        samples = {'Binary': pa.array([b'\x00\xff']),
                   'Decimal': pa.array([Decimal('12.25')], type=pa.decimal128(8, 2)),
                   'NestedDictionary': pa.array([[1, 2]], type=pa.list_(pa.int64()))}
        matrix = {(d['driver'], e['element']): e for d in tables['losses'] for e in d['elements']}
        for driver in ('shp', 'dxf', 'ipc', 'geoparquet'):
            for element, values in samples.items():
                rule = matrix[driver, element]['write']
                input_path = attribute('sample-' + driver + element, values)
                output = work / (driver + element + '.' + FORMATS[driver][0])
                args = ['write', input_path, output, '--to', driver]
                if driver == 'shp':
                    args += ['--out-opt', 'publish_mode=loose_shapefile_set']
                code, response = run(binary, args)
                if rule == 'Rejected':
                    expected = next(r for r in tables['errors'] if r['situation'] == 'UnrepresentableWriteType')
                    error = response.get('error', {})
                    if code != expected['exit'] or any(error.get(k) != expected[k] for k in ('category', 'phase', 'code', 'message')) or output.exists():
                        errors.append(f'loss {driver}/{element}: rifiuto diverso dal documentato: {response}')
                    continue
                if code:
                    errors.append(f'loss {driver}/{element}: scrittura rifiutata: {response}')
                    continue
                delivered = work / (driver + element + '-read.arrow')
                code, read = run(binary, ['read', output, '--output', delivered])
                if code:
                    errors.append(f'loss {driver}/{element}: rilettura rifiutata: {read}')
                    continue
                with pa.memory_map(str(delivered), 'r') as f:
                    actual = pa.ipc.open_file(f).read_all()
                if rule == 'Preserved':
                    if 'value' not in actual.column_names or actual['value'].type != values.type or actual['value'].to_pylist() != values.to_pylist():
                        errors.append(f'loss {driver}/{element}: tipo o valore non conservato')
                elif rule == 'Transformed(DxfAttribute)':
                    if 'value' in actual.column_names or 'dxf_attribute_not_represented' not in json.dumps(response):
                        errors.append(f'loss {driver}/{element}: omissione non osservata o non dichiarata')
                else:
                    errors.append(f'loss {driver}/{element}: regola nuova senza oracolo: {rule}')
    return errors
