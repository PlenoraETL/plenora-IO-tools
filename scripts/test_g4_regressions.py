"""G4 public regressions, generated from fresh native/Arrow inputs."""
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
BINARY = Path(os.environ.get('PLENORA_IO_BIN', ROOT / 'target/debug/plenora-io.exe'))
NATIVE_PROOF = True


@unittest.skipUnless(BINARY.is_file(), 'compiled CLI required')
class G4RegressionTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.work = Path(self.directory.name)
        self.sequence = 0

    def cli(self, *args, success=True):
        result = subprocess.run([str(BINARY), *map(str, args)], capture_output=True, timeout=90)
        self.assertEqual(result.stderr, b'')
        doc = json.loads(result.stdout)
        self.assertEqual(result.returncode == 0, success, doc)
        return doc.get('result', doc.get('error'))

    def table(self, name='F_G4_729', values=('VALUE_G4_729',), crs='EPSG:4326'):
        import pyarrow as pa
        md = {'ARROW:extension:name': 'geoarrow.wkb', 'crs': crs,
              'plenora.geometry.axis_order': 'lon_lat', 'plenora.geometry.crs_id': crs,
              'plenora.geometry.crs_resolution': 'resolved', 'plenora.geometry.dimensions': 'xy',
              'plenora.geometry.encoding': 'wkb', 'plenora.geometry.precision': 'float64',
              'plenora.geometry.spatial_semantics': 'geometry', 'plenora.geometry.types': 'point',
              'plenora.geometry.types_declaration': 'exact', 'plenora.field_id': '0'}
        schema = pa.schema([pa.field('G_G4_729', pa.binary(), metadata=md),
                            pa.field(name, pa.array(values).type, metadata={'plenora.field_id': '1'})],
                           metadata={'plenora.contract.version': '1'})
        return pa.Table.from_arrays([pa.array([struct.pack('<BIdd', 1, 1, 1., 2.)] * len(values)),
                                     pa.array(values)], schema=schema)

    def save(self, table, name):
        import pyarrow as pa
        path = self.work / name
        with pa.OSFile(str(path), 'wb') as file:
            with pa.ipc.new_file(file, table.schema) as writer:
                writer.write_table(table)
        return path

    def load(self, path):
        import pyarrow as pa
        with pa.memory_map(str(path), 'r') as file:
            return pa.ipc.open_file(file).read_all()

    def assert_texts_private(self, document):
        text_keys = {'message', 'detail', 'context', 'cause', 'knowledge_limits'}
        def walk(value):
            if isinstance(value, dict):
                for key, child in value.items():
                    if key in text_keys:
                        text = json.dumps(child)
                        for sentinel in ['F_G4_729', 'G_G4_729', 'L_G4_729', 'VALUE_G4_729']:
                            self.assertNotIn(sentinel, text)
                    walk(child)
            elif isinstance(value, list):
                for child in value:
                    walk(child)
        walk(document)

    def test_all_drivers_read_write_and_sdk_keep_names_out_of_text(self):
        import pyarrow as pa
        from plenora_io import Client, PlenoraError
        suffixes = {'csv': 'csv', 'xls': 'xlsx', 'shp': 'shp.d', 'kml': 'kml', 'dxf': 'dxf',
                    'filegdb': 'gdb', 'gpkg': 'gpkg', 'geojson': 'geojson',
                    'geoparquet': 'parquet', 'ipc': 'arrow'}
        source = self.save(self.table(), 'L_G4_729.arrow')
        invalid = self.table()
        invalid = invalid.set_column(0, invalid.schema.field(0), pa.array([b'VALUE_G4_729']))
        bad = self.save(invalid, 'bad.arrow')
        client = Client(binary=BINARY)
        tested = set()
        for driver in self.cli('catalog')['drivers']:
            name = driver['id']
            if not driver['available']:
                continue
            tested.add(name)
            with self.subTest(driver=name):
                source = self.save(self.table(crs='OGC:CRS84' if name in ('kml', 'geojson') else 'EPSG:4326'), name + '-input-L_G4_729.arrow')
                invalid = self.table(crs='OGC:CRS84' if name in ('kml', 'geojson') else 'EPSG:4326')
                invalid = invalid.set_column(0, invalid.schema.field(0), pa.array([b'VALUE_G4_729']))
                bad = self.save(invalid, name + '-bad.arrow')
                destination = self.work / (name + '-L_G4_729.' + suffixes[name])
                self.assert_texts_private(self.cli('write', source, destination, '--to', name))
                options = ['--assume-crs', 'EPSG:4326', '--opt', 'wkt_column=geometry'] if name in ('csv', 'xls') else []
                self.assert_texts_private(self.cli('read', destination, *options))
                error = self.cli('read', destination, '--layer', '999999', *options, success=False)
                self.assert_texts_private(error)
                self.assertNotIn('VALUE_G4_729', json.dumps(error))
                with self.assertRaises(PlenoraError) as read_error:
                    client.read(destination, self.work / ('read-error-' + name + '.arrow'),
                                layer=999999, assume_crs='EPSG:4326',
                                options={'wkt_column': 'geometry'} if name in ('csv', 'xls') else None)
                self.assert_texts_private(read_error.exception.to_dict())
                for sentinel in ('F_G4_729', 'G_G4_729', 'L_G4_729', 'VALUE_G4_729'):
                    self.assertNotIn(sentinel, str(read_error.exception))
                if name in ('gpkg', 'filegdb', 'ipc'):
                    objects = self.cli('layers', destination)['layers']
                    self.assertTrue(any('L_G4_729' in obj['name'] for obj in objects))
                error = self.cli('write', bad, self.work / ('bad-' + name + '.' + suffixes[name]), '--to', name, success=False)
                self.assert_texts_private(error)
                self.assertIn('row_diagnostics', error.get('details', {}), error)
                self.assertEqual(error['details']['row_diagnostics']['examples'][0]['column'], 'G_G4_729')
                self.assertNotIn('VALUE_G4_729', json.dumps(error))
                with self.assertRaises(PlenoraError) as caught:
                    client.write(bad, self.work / ('sdk-' + name + '.' + suffixes[name]), format=name)
                self.assert_texts_private(caught.exception.to_dict())
                self.assert_texts_private(dict(caught.exception.envelope.row_diagnostics))
                self.assertEqual(caught.exception.envelope.row_diagnostics['examples'][0]['column'], 'G_G4_729')
                self.assertNotIn('G_G4_729', str(caught.exception))
                self.assertNotIn('VALUE_G4_729', str(caught.exception))
        self.assertTrue(set(suffixes) - {'filegdb'} <= tested)
        for driver in ('kml', 'shp'):
            source = self.save(self.table(values=(float('nan'),), crs='OGC:CRS84' if driver == 'kml' else 'EPSG:4326'), driver + '-nan.arrow')
            error = self.cli('write', source, self.work / ('nan-' + driver + '.' + suffixes[driver]), '--to', driver, success=False)
            self.assert_texts_private(error)
            self.assertIn('row_diagnostics', error.get('details', {}), error)
            self.assertEqual(error['details']['row_diagnostics']['examples'][0]['column'], 'F_G4_729')

    def test_dxf_identical_attributes_and_real_counterexamples(self):
        import pyarrow as pa
        source = self.save(self.table(), 'source.arrow')
        native = self.work / 'source.dxf'
        self.cli('write', source, native, '--to', 'dxf')
        first = self.work / 'first.arrow'
        self.cli('read', native, '--output', first)
        baseline = self.load(first)
        for index, (column, value, count) in enumerate([
                (None, None, 0), ('layer', 'PRIVATE_LAYER', 0),
                ('dxf_type', 'OTHER', 1), ('text', 'PRIVATE_TEXT', 1), ('text', '', 1)]):
            table = baseline
            if column:
                i = table.schema.get_field_index(column)
                table = table.set_column(i, table.schema.field(i), pa.array([value]))
            arrow = self.save(table, f'case{index}.arrow')
            output = self.work / f'case{index}.dxf'
            result = self.cli('write', arrow, output, '--to', 'dxf')
            actual = sum(c['conteggio'] for c in result['write_loss']['counts']
                         if c['categoria'] == 'dxf_attribute_not_represented')
            self.assertEqual(actual, count)
            back = self.work / f'back{index}.arrow'
            self.cli('read', output, '--output', back)
            self.assertEqual(table.equals(self.load(back), check_metadata=True), count == 0)
            reasons = [r['code'] for r in result['write_fidelity']['reasons']]
            self.assertEqual('attribute_loss' in reasons, count != 0)

    def test_filegdb_member_errors_are_local(self):
        import openpyxl
        if not next(d for d in self.cli('catalog')['drivers'] if d['id'] == 'filegdb')['available']:
            self.skipTest('GDAL backend unavailable')
        workbook = openpyxl.Workbook()
        for i, sheet in enumerate([workbook.active, workbook.create_sheet('Second')]):
            sheet.title = ['First', 'Second'][i]
            sheet.append(['x', 'y', 'value'])
            sheet.append([1, 2, i])
        source = self.work / 'multi.xlsx'
        workbook.save(source)
        native = self.work / 'multi.gdb'
        self.cli('convert', source, native, '--from', 'xls', '--to', 'filegdb',
                 '--in-opt', 'x_column=x', '--in-opt', 'y_column=y', '--assume-crs', 'EPSG:4326')
        for identifier in (9, 10):
            for extension in ('gdbtable', 'gdbtablx'):
                for mode in ('missing', 'empty', 'truncated'):
                    with self.subTest(identifier=identifier, extension=extension, mode=mode):
                        copy = self.work / f'{identifier}-{extension}-{mode}.gdb'
                        shutil.copytree(native, copy)
                        member = copy / f'a{identifier:08x}.{extension}'
                        if mode == 'missing':
                            member.unlink()
                        else:
                            member.write_bytes(b'' if mode == 'empty' else member.read_bytes()[:3])
                        objects = self.cli('layers', copy)['layers']
                        self.assertEqual(len(objects), 2)
                        self.assertFalse(objects[identifier - 9]['status']['readable'])
                        self.assertTrue(objects[10 - identifier]['status']['readable'])
                        implicit = self.cli('read', copy, success=False)
                        self.assertEqual(implicit['code'], 'LAYER_SELECTION_REQUIRED')
                        self.assertEqual(len(implicit['details']['objects']), 2)
                        self.assertEqual(self.cli('read', copy, '--layer', 10 - identifier)['rows_read'], 1)
                        self.cli('read', copy, '--layer', identifier - 9, success=False)

    def test_duplicate_geometry_headers_are_configuration_errors(self):
        for index, header in enumerate(['wkt,wkt\nPOINT(1 2),POINT(1 2)\n', 'x,x,y\n1,1,2\n']):
            source = self.work / f'duplicate{index}.csv'
            source.write_text(header)
            options = ['wkt_column=wkt'] if index == 0 else ['x_column=x', 'y_column=y']
            args = [item for option in options for item in ['--opt', option]]
            error = self.cli('read', source, '--assume-crs', 'EPSG:4326', *args, success=False)
            self.assertEqual((error['category'], error['phase']), ('invalid_configuration', 'validate'))

    def test_filegdb_f64_repeated_publication(self):
        from concurrent.futures import ThreadPoolExecutor
        if not next(d for d in self.cli('catalog')['drivers'] if d['id'] == 'filegdb')['available']:
            self.skipTest('GDAL backend unavailable')
        cases = [(0.1, -0.0, None), (1e300, 5e-324, 123456789.123456789),
                 (1., 2., None), (1 / 3, 2., None), (1e-9, 2., None),
                 (0.12345678, 123456789012.5, None), (1e15, 2., None)]
        inputs = [self.save(self.table(values=values), f'f64-{i}.arrow')
                  for i, values in enumerate(cases)]
        def run(index):
            case = index % len(cases)
            output = self.work / f'f64-{index}.gdb'
            self.cli('write', inputs[case], output, '--to', 'filegdb')
            back = self.work / f'f64-back-{index}.arrow'
            self.cli('read', output, '--output', back)
            bits = lambda values: [None if v is None else struct.pack('<d', v) for v in values]
            self.assertEqual(bits(self.load(back)['F_G4_729'].to_pylist()), bits(cases[case]))
        with ThreadPoolExecutor(max_workers=4) as pool:
            list(pool.map(run, range(112)))


if __name__ == '__main__':
    unittest.main()
