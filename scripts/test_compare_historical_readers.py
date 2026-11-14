"""Historical byte exceptions must reject unrelated changes."""
import json
from contextlib import closing
from pathlib import Path
import tempfile
import unittest
import pyarrow as pa
import pyarrow.parquet as pq
from scripts.compare_historical_readers import (explained_dimensions, matching_rejections,
    explained_preserved_srid, explained_empty_footer_types,
    explained_gpkg_field_identity, independent_gpkg, independent_wkb)

ROOT = Path(__file__).resolve().parents[1]


class HistoricalException(unittest.TestCase):
    def test_gpkg_native_identity_order_excludes_generated_covering(self):
        import sqlite3
        with tempfile.TemporaryDirectory(dir=ROOT / 'target') as folder:
            root = Path(folder)
            source = root / 'native.gpkg'
            independent_gpkg(source, 'point', 'xy', 'value', [independent_wkb('point', 'xy')])
            with closing(sqlite3.connect(source)) as db, db:
                db.execute('ALTER TABLE feature ADD COLUMN label INTEGER')
            geo = json.dumps({'columns': {'geometry': {'covering': {'bbox': {'xmin': ['bbox', 'xmin']}}}}}).encode()
            fields = [pa.field('geometry', pa.binary(), metadata={b'plenora.field_id': b'0'}),
                      pa.field('label', pa.int64()), pa.field('fid', pa.int64()),
                      pa.field('bbox', pa.struct([('xmin', pa.float64())]))]
            values = [pa.array([b'point']), pa.array([5]), pa.array([1]),
                      pa.array([{'xmin': 9.}], type=fields[-1].type)]
            old, new = root / 'old.parquet', root / 'new.parquet'
            pq.write_table(pa.Table.from_arrays(values, schema=pa.schema(fields, metadata={b'geo': geo})), old)
            for i in (1, 2): fields[i] = fields[i].with_metadata({b'plenora.field_id': str(i).encode()})
            pq.write_table(pa.Table.from_arrays(values, schema=pa.schema(fields, metadata={b'geo': geo})), new)
            self.assertTrue(explained_gpkg_field_identity(source, old, new, 'geoparquet'))
            fields[-1] = fields[-1].with_metadata({b'plenora.field_id': b'3'})
            pq.write_table(pa.Table.from_arrays(values, schema=pa.schema(fields, metadata={b'geo': geo})), new)
            self.assertFalse(explained_gpkg_field_identity(source, old, new, 'geoparquet'))

    def test_gpkg_identity_exception_is_bound_to_native_columns_and_encoded_pages(self):
        import sqlite3
        with tempfile.TemporaryDirectory(dir=ROOT / 'target') as folder:
            root = Path(folder)
            source = root / 'native.gpkg'
            value = independent_wkb('point', 'xy')
            independent_gpkg(source, 'point', 'xy', 'value', [value])
            def write(name, identity=None, number=1, extra=False, geometry_id=b'0', field_name='fid'):
                md = {} if identity is None else {b'plenora.field_id': identity}
                if extra: md[b'unrelated'] = b'change'
                schema = pa.schema([
                    pa.field('geometry', pa.binary(), metadata={b'plenora.field_id': geometry_id}),
                    pa.field(field_name, pa.int64(), metadata=md)])
                path = root / name
                pq.write_table(pa.Table.from_arrays([pa.array([value]), pa.array([number])], schema=schema),
                               path, use_dictionary=False)
                return path
            old = write('old.parquet')
            new = write('new.parquet', b'1')
            self.assertTrue(explained_gpkg_field_identity(source, old, new, 'geoparquet'))
            self.assertFalse(explained_gpkg_field_identity(source, old, new, 'ipc'))
            for changes in ({'identity': b'2'}, {'identity': b'1', 'number': 2},
                            {'identity': b'1', 'extra': True}, {'identity': b'1', 'geometry_id': b'9'},
                            {'identity': b'1', 'field_name': 'other'}):
                with self.subTest(changes=changes):
                    bad = write('bad.parquet', **changes)
                    self.assertFalse(explained_gpkg_field_identity(source, old, bad, 'geoparquet'))
            # Positional fallback is forbidden when the source declares identities.
            with closing(sqlite3.connect(source)) as db, db:
                db.execute('CREATE TABLE gpkg_metadata(md_standard_uri TEXT)')
                db.execute('INSERT INTO gpkg_metadata VALUES(?)', ('urn:plenora:gpkg:field-identity:1',))
            self.assertFalse(explained_gpkg_field_identity(source, old, new, 'geoparquet'))

    def test_crashes_are_not_matching_rejections(self):
        self.assertFalse(matching_rejections([{'exit': 1, 'response': {'diagnostic': 'crash'}}] * 2))

    def test_error_axes_must_match(self):
        import copy
        old = {'exit': 3, 'response': {'contract': 'plenora-error-v1', 'error': {'category': 'unsupported', 'phase': 'validate', 'remote_effect': 'none', 'retry': {'kind': 'never'}}}}
        new = copy.deepcopy(old)
        self.assertTrue(matching_rejections([old, new]))
        new['response']['error']['retry']['kind'] = 'immediate'
        self.assertFalse(matching_rejections([old, new]))

    def test_only_final_dimensions_are_exempted(self):
        geo = json.dumps({'version': '1.1.0', 'primary_column': 'g', 'columns': {'g': {'encoding': 'WKB', 'geometry_types': ['Point']}}}).encode()
        with tempfile.TemporaryDirectory(dir=ROOT / 'target') as folder:
            def write(name, dimension, payload=b'point-bytes', extra=False):
                md = {b'plenora.geometry.dimensions': dimension}
                if extra: md[b'unrelated'] = b'change'
                schema = pa.schema([pa.field('g', pa.binary(), metadata=md)], metadata={b'geo': geo})
                path = Path(folder) / name
                pq.write_table(pa.Table.from_arrays([pa.array([payload])], schema=schema), path, use_dictionary=False)
                return path
            old = write('old.parquet', b'unknown')
            self.assertTrue(explained_dimensions(old, write('allowed.parquet', b'xy')))
            self.assertFalse(explained_dimensions(old, write('wrong-dimension.parquet', b'xyz')))
            self.assertFalse(explained_dimensions(old, write('wrong-value.parquet', b'xy', b'other-bytes')))
            self.assertFalse(explained_dimensions(old, write('extra-metadata.parquet', b'xy', extra=True)))


class RereadExceptions(unittest.TestCase):
    def test_srid_exception_preserves_every_other_property_and_encoded_value(self):
        import pyarrow.ipc as ipc
        with tempfile.TemporaryDirectory(dir=ROOT / 'target') as folder:
            root = Path(folder)
            def write(name, fmt, srid=None, value=b'original', extra=False, stream=False):
                md = {} if srid is None else {b'plenora.geometry.srid': srid}
                if extra: md[b'unrelated'] = b'change'
                schema = pa.schema([pa.field('g', pa.binary(), metadata=md)])
                table = pa.Table.from_arrays([pa.array([value])], schema=schema)
                path = root / name
                if fmt == 'geoparquet': pq.write_table(table, path, use_dictionary=False)
                else:
                    with (ipc.new_stream(path, schema) if stream else ipc.new_file(path, schema)) as w: w.write_table(table)
                return path
            source = write('source.parquet', 'geoparquet', b'1234')
            for fmt, stream in [('ipc', False), ('ipc', True), ('geoparquet', False)]:
                old = write('old', fmt, stream=stream)
                new = write('new', fmt, b'1234', stream=stream)
                self.assertTrue(explained_preserved_srid(source, old, new, fmt))
                for srid, value, extra in [(b'4326', b'original', False), (b'1234', b'changed', False), (b'1234', b'original', True)]:
                    bad = write('bad', fmt, srid, value, extra, stream)
                    self.assertFalse(explained_preserved_srid(source, old, bad, fmt))
                no_declaration = write('no-srid.parquet', 'geoparquet')
                self.assertFalse(explained_preserved_srid(no_declaration, old, new, fmt))

    def test_empty_footer_exception_requires_exact_error_schema_and_values(self):
        import copy
        import pyarrow.ipc as ipc
        error = {'category': 'unsupported', 'code': 'UNSUPPORTED',
                 'message': 'il formato richiede una dichiarazione preventiva dei tipi geometrici',
                 'phase': 'validate', 'remote_effect': 'none', 'retry': {'kind': 'never'}}
        executions = [{'exit': 3, 'response': {'contract': 'plenora-error-v1', 'error': error}},
                      {'exit': 0, 'response': {}}]
        column = {'encoding': 'WKB', 'geometry_types': []}
        geo = {'version': '1.1.0', 'primary_column': 'g', 'columns': {'g': column}}
        md = {b'plenora.geometry.types_declaration': b'exact', b'plenora.geometry.types': b'point', b'plenora.field_id': b'0'}
        schema = pa.schema([pa.field('g', pa.binary(), metadata=md), pa.field('id', pa.int64())],
                           metadata={b'geo': json.dumps(geo).encode(), b'plenora.contract.version': b'1'})
        with tempfile.TemporaryDirectory(dir=ROOT / 'target') as folder:
            root = Path(folder); source = root / 'source.parquet'
            table = pa.Table.from_arrays([pa.array([None], type=pa.binary()), pa.array([7])], schema=schema)
            pq.write_table(table, source)
            expected = dict(md)
            expected.update({b'plenora.geometry.native.geoparquet.primary_column': b'g',
                             b'plenora.geometry.native.geoparquet.version': b'1.1.0',
                             b'plenora.geometry.native.geoparquet.edges': b'planar',
                             b'plenora.geometry.native.geoparquet.column': b'{"encoding":"WKB","geometry_types":[]}'})
            out_schema = pa.schema([schema.field('g').with_metadata(expected), schema.field('id').with_metadata({b'plenora.field_id': b'1'})], metadata={b'plenora.contract.version': b'1'})
            for stream in [False, True]:
                out = root / 'output.arrow'
                def write(value, extra=False):
                    s = out_schema.with_metadata({b'unrelated': b'change'}) if extra else out_schema
                    t = pa.Table.from_arrays([pa.array([None], type=pa.binary()), pa.array([value])], schema=s)
                    with (ipc.new_stream(out, s) if stream else ipc.new_file(out, s)) as w: w.write_table(t)
                write(7)
                self.assertTrue(explained_empty_footer_types(source, out, 'ipc', executions))
                different_error = copy.deepcopy(executions); different_error[0]['response']['error']['category'] = 'schema'
                self.assertFalse(explained_empty_footer_types(source, out, 'ipc', different_error))
                write(8)
                self.assertFalse(explained_empty_footer_types(source, out, 'ipc', executions))
                write(7, extra=True)
                self.assertFalse(explained_empty_footer_types(source, out, 'ipc', executions))
            self.assertTrue(explained_empty_footer_types(source, source, 'geoparquet', executions))
            unknown_md = dict(md); unknown_md[b'plenora.geometry.types'] = b'unknown'
            unknown_schema = schema.set(0, schema.field(0).with_metadata(unknown_md))
            unknown = root / 'unknown.parquet'
            pq.write_table(pa.Table.from_arrays(table.columns, schema=unknown_schema), unknown)
            self.assertFalse(explained_empty_footer_types(unknown, unknown, 'geoparquet', executions))
            crash = copy.deepcopy(executions); crash[0]['exit'] = 137
            self.assertFalse(explained_empty_footer_types(source, source, 'geoparquet', crash))

            changed = root / 'changed.parquet'; pq.write_table(table.set_column(1, schema.field('id'), pa.array([8])), changed)
            self.assertFalse(explained_empty_footer_types(source, changed, 'geoparquet', executions))


if __name__ == '__main__':
    unittest.main()
