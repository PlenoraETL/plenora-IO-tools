"""Public regressions: selection, faithful reconstruction and physical Arrow families."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

NATIVE_PROOF = True

ROOT = Path(__file__).resolve().parents[1]
BINARY = Path(os.environ.get('PLENORA_IO_BIN', ROOT / 'target/debug' / ('plenora-io.exe' if os.name == 'nt' else 'plenora-io')))

@unittest.skipUnless(BINARY.is_file(), 'compiled CLI required')
class PublicRegressionTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.work = Path(self.directory.name)

    def cli(self, *args, success=True):
        result = subprocess.run([str(BINARY), *map(str, args)], capture_output=True, timeout=60)
        self.assertEqual(result.stderr, b'')
        document = json.loads(result.stdout)
        self.assertEqual(result.returncode == 0, success, document)
        return document.get('result', document.get('error'))

    def test_sheet_selects_one_object_and_conflicts_are_rejected(self):
        import openpyxl
        book = openpyxl.Workbook()
        book.active.title = 'Primo'
        book.active.append(['name', 'x', 'y'])
        book.active.append(['first', 1, 2])
        second = book.create_sheet('Secondo')
        second.append(['name', 'x', 'y'])
        for i in range(3):
            second.append(['second', i, 2])
        source = self.work / 'two.xlsx'
        book.save(source)
        self.assertEqual(self.cli('read', source, success=False)['code'], 'LAYER_SELECTION_REQUIRED')
        self.assertEqual(self.cli('read', source, '--opt', 'sheet=Secondo')['rows_read'], 3)
        self.assertEqual(self.cli('read', source, '--opt', 'sheet=Secondo', '--layer', '0', success=False)['category'], 'invalid_configuration')
        result = self.cli('convert', source, self.work / 'selected.gpkg', '--from', 'xls', '--to', 'gpkg', '--in-opt', 'sheet=Secondo', '--in-opt', 'x_column=x', '--in-opt', 'y_column=y', '--assume-crs', 'EPSG:4326')
        self.assertEqual(result['total_rows'], 3)
        self.assertEqual(len(result['layers']), 1)

    def test_kml_automatic_copy_does_not_duplicate_nested_views(self):
        point = '<Placemark><Point><coordinates>1,2</coordinates></Point></Placemark>'
        for index, body in enumerate([point, '<Document>'+point+'</Document>', '<Document><Folder>'+point+'</Folder></Document>', '<Document><Folder>'+point+'</Folder><Folder>'+point+'</Folder></Document>']):
            source = self.work / f'folder{index}.kml'
            source.write_text('<kml xmlns="http://www.opengis.net/kml/2.2">'+body+'</kml>', encoding='utf8')
            layers = self.cli('layers', source)['layers']
            names = [layer['name'].lower() for layer in layers]
            self.assertEqual(len(names), len(set(names)))
            expected = 2 if index == 3 else 1
            self.assertEqual(self.cli('read', source)['rows_read'], expected)
            result = self.cli('convert', source, self.work / f'copy{index}.gpkg', '--from', 'kml', '--to', 'gpkg')
            self.assertEqual(result['total_rows'], expected)
            self.assertEqual(len(result['layers']), 1)

    def test_faithful_public_schema_has_no_invented_metadata_or_nullability_loss(self):
        import pyarrow as pa
        source = self.work / 'native.csv'
        source.write_text('name\nx\n', encoding='utf8')
        first, second = self.work / 'first.arrow', self.work / 'second.arrow'
        self.cli('read', source, '--output', first)
        result = self.cli('write', first, self.work / 'copy.csv', '--to', 'csv')
        self.cli('read', self.work / 'copy.csv', '--output', second)
        with pa.memory_map(str(first), 'r') as a, pa.memory_map(str(second), 'r') as b:
            self.assertTrue(pa.ipc.open_file(a).read_all().equals(pa.ipc.open_file(b).read_all(), check_metadata=True))
        self.assertEqual(result['write_loss']['counts'], [])
        self.assertNotIn('nullability_changed', [r['code'] for r in result['write_fidelity']['reasons']])

    def test_selection_enumerates_native_multi_object_containers(self):
        import openpyxl
        book = openpyxl.Workbook()
        for index, sheet in enumerate([book.active, book.create_sheet('Second')]):
            sheet.title = ['First', 'Second'][index]
            sheet.append(['name', 'x', 'y'])
            for row in range(index + 1):
                sheet.append(['value', row + 1, 2])
        source = self.work / 'objects.xlsx'
        book.save(source)
        catalog = self.cli('catalog')['drivers']
        containers = {d['id'] for d in catalog if d['multi_layer']}
        self.assertEqual(containers, {'gpkg', 'filegdb'})
        for driver in catalog:
            if driver['id'] not in containers or not driver['available']:
                continue
            suffix = {'gpkg': 'gpkg', 'filegdb': 'gdb'}[driver['id']]
            destination = self.work / ('objects.' + suffix)
            result = self.cli('convert', source, destination, '--from', 'xls', '--to', driver['id'], '--in-opt', 'x_column=x', '--in-opt', 'y_column=y', '--assume-crs', 'EPSG:4326')
            self.assertEqual(result['total_rows'], 3)
            self.assertEqual(len(result['layers']), 2)
            self.assertEqual(self.cli('read', destination, success=False)['code'], 'LAYER_SELECTION_REQUIRED')
            self.assertEqual(self.cli('read', destination, '--layer', '1')['rows_read'], 2)

    def test_geojson_standard_crs_converts_without_assumption(self):
        source = self.work / 'source.geojson'
        source.write_text(json.dumps({'type':'FeatureCollection','features':[{'type':'Feature','properties':{},'geometry':{'type':'Point','coordinates':[1,2]}}]}), encoding='utf8')
        result = self.cli('convert', source, self.work / 'copy.gpkg', '--from', 'geojson', '--to', 'gpkg')
        self.assertEqual(result['total_rows'], 1)
        self.assertEqual(self.cli('read', self.work / 'copy.gpkg')['rows_read'], 1)

    def test_geojson_non_dataset_error_names_class_and_phase_without_data(self):
        point = {'type': 'Point', 'coordinates': [123.456, 44.556]}
        feature = {'type': 'Feature', 'properties': {'private': 'secret-value'}, 'geometry': point}
        for index, value in enumerate([point, feature]):
            source = self.work / f'object{index}.geojson'
            source.write_text(json.dumps(value), encoding='utf8')
            error = self.cli('read', source, success=False)
            self.assertEqual(error['phase'], 'read')
            self.assertIn('FeatureCollection', error['message'])
            self.assertIn('classe GeoJSON', error['message'])
            self.assertNotIn('secret-value', json.dumps(error))
            self.assertNotIn('123.456', json.dumps(error))

    def test_filegdb_warning_is_redacted_and_native_values_survive(self):
        import pyarrow as pa
        import struct
        catalog = self.cli('catalog')
        driver = next(d for d in catalog['drivers'] if d['id'] == 'filegdb')
        # The base profile deliberately cannot write FileGDB.
        if not driver['available']:
            self.skipTest('FileGDB backend unavailable')
        md = {
            'ARROW:extension:name': 'geoarrow.wkb', 'crs': 'EPSG:4326',
            'plenora.geometry.axis_order': 'lon_lat',
            'plenora.geometry.crs_id': 'EPSG:4326',
            'plenora.geometry.crs_resolution': 'resolved',
            'plenora.geometry.dimensions': 'xy',
            'plenora.geometry.encoding': 'wkb',
            'plenora.geometry.precision': 'float64',
            'plenora.geometry.spatial_semantics': 'geometry',
            'plenora.geometry.types': 'point',
            'plenora.geometry.types_declaration': 'exact',
            'plenora.field_id': '0',
        }
        schema = pa.schema([pa.field('geometry', pa.binary(), metadata=md), pa.field('value', pa.float64(), metadata={'plenora.field_id': '1'})], metadata={'plenora.contract.version': '1'})
        values = [0.1, 1.0/3.0, 1e-20, 1e20, -0.0]
        table = pa.Table.from_arrays([pa.array([struct.pack('<BIdd',1,1,1.,2.)]*len(values)), pa.array(values)], schema=schema)
        source = self.work / 'secret.layer.arrow'
        with pa.OSFile(str(source), 'wb') as file:
            with pa.ipc.new_file(file, schema) as writer:
                writer.write_table(table)
        for index in range(10):
            destination = self.work / f'output{index}.gdb'
            result = self.cli('write', source, destination, '--to', 'filegdb')
            serialized = json.dumps(result['write_fidelity']) + json.dumps(result['write_loss'])
            self.assertNotIn('secret.layer', serialized)
            self.assertIn('native_layer_metadata_not_preserved', serialized)
            self.assertIn('diagnostica libreria GDAL', serialized)
            back = self.work / f'back{index}.arrow'
            self.cli('read', destination, '--output', back)
            with pa.memory_map(str(back), 'r') as file:
                actual = pa.ipc.open_file(file).read_all()['value'].to_pylist()
            self.assertEqual([struct.pack('<d',x) for x in actual], [struct.pack('<d',x) for x in values])

if __name__ == '__main__':
    unittest.main()
