"""Lo SDK conserva lo schema IPC senza attraversare arrow-pyarrow."""

import tempfile
import unittest
from pathlib import Path

from plenora_io._arrow import scrivi_ipc
from test_contratto_sdk import pa, serve_pyarrow


@serve_pyarrow
class SchemaIpc(unittest.TestCase):
    def test_map_keys_sorted_conservato_su_piu_batch(self):
        tipo = pa.map_(pa.int32(), pa.string(), keys_sorted=True)
        array = pa.array([[(1, "a"), (2, "b")], None], type=tipo)
        batch = pa.record_batch([array], names=["m"])
        lettore = pa.RecordBatchReader.from_batches(batch.schema, [batch, batch])
        with tempfile.TemporaryDirectory() as directory:
            file = scrivi_ipc(lettore, Path(directory))
            with pa.OSFile(str(file), "rb") as source, pa.ipc.open_file(source) as reader:
                self.assertTrue(reader.schema.field(0).type.keys_sorted)
                self.assertEqual(reader.num_record_batches, 2)
                self.assertEqual(reader.get_batch(0), batch)
                self.assertEqual(reader.get_batch(1), batch)
