"""The only historical byte exception must reject unrelated changes."""
import json
from pathlib import Path
import tempfile
import unittest
import pyarrow as pa
import pyarrow.parquet as pq
from scripts.compare_historical_readers import explained_dimensions, matching_rejections

ROOT = Path(__file__).resolve().parents[1]


class HistoricalException(unittest.TestCase):
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


if __name__ == '__main__':
    unittest.main()
