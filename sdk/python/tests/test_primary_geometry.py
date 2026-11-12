"""PRIM sulle superfici Table, reader lazy e produttore C stream terzo."""
import json
import os
from pathlib import Path
import tempfile
import unittest

import pyarrow as pa
from plenora_io import Client, PlenoraError

ROOT = Path(__file__).resolve().parents[3]
KEY = b"plenora.primary_geometry"

class CStream:
    def __init__(self, table): self.table = table
    def __arrow_c_stream__(self, requested_schema=None):
        return self.table.__arrow_c_stream__(requested_schema)

class PrimaryGeometry(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        binary = os.environ.get("PLENORA_IO_BIN") or str(ROOT / "target/debug/plenora-io.exe")
        if not Path(binary).exists(): raise unittest.SkipTest("binario IO non costruito")
        cls.client = Client(binary=binary)

    def table(self, count, key, empty):
        fields = []
        for i in range(count):
            md = {b"ARROW:extension:name": b"geoarrow.wkb", b"plenora.field_id": str(17 + i*74).encode(),
                  b"plenora.geometry.encoding": b"wkb", b"plenora.geometry.dimensions": b"xy",
                  b"plenora.geometry.spatial_semantics": b"geometry", b"plenora.geometry.precision": b"float64",
                  b"plenora.geometry.types_declaration": b"exact", b"plenora.geometry.types": b"point",
                  b"plenora.geometry.crs_resolution": b"resolved", b"plenora.geometry.crs_id": b"EPSG:4326",
                  b"plenora.geometry.axis_order": b"lon_lat"}
            fields.append(pa.field("g"+str(i), pa.binary(), metadata=md))
        md = {b"plenora.contract.version": b"1"}
        if key is not None: md[KEY] = key
        point = bytes.fromhex("0101000000000000000000f03f0000000000000040")
        return pa.Table.from_arrays([pa.array([] if empty else [point, None], type=pa.binary()) for _ in fields], schema=pa.schema(fields, metadata=md))

    def test_roundtrip_primary_all_arrow_producers(self):
        for count, key, canonical in [(1,None,None),(1,b"none",b"none"),(1,b"17",None),(2,None,None),(2,b"none",None),(2,b"91",b"91")]:
            for empty in (False,True):
                for kind in ("table","reader","capsule"):
                    with self.subTest(count=count,key=key,empty=empty,kind=kind), tempfile.TemporaryDirectory() as tmp:
                        table = self.table(count,key,empty)
                        producer = table if kind == "table" else CStream(table) if kind == "capsule" else pa.RecordBatchReader.from_batches(table.schema, table.to_batches(max_chunksize=1))
                        dest = Path(tmp)/"result.arrow"
                        self.client.write(producer,dest,format="ipc")
                        with pa.memory_map(str(dest)) as src: observed = pa.ipc.open_file(src).read_all()
                        metadata = dict(table.schema.metadata); metadata.pop(KEY,None)
                        if canonical is not None: metadata[KEY] = canonical
                        self.assertTrue(observed.equals(table.replace_schema_metadata(metadata),check_metadata=True))

    def test_invalid_capsule_does_not_publish(self):
        with tempfile.TemporaryDirectory() as tmp:
            dest = Path(tmp)/"result.arrow"
            with self.assertRaises(PlenoraError):
                self.client.write(CStream(self.table(2,b"999",False)),dest,format="ipc")
            self.assertFalse(dest.exists())


class NativePrimaryGeometry(PrimaryGeometry):
    @classmethod
    def setUpClass(cls):
        from unittest import mock
        try:
            from plenora_io import _native
        except ImportError:
            raise unittest.SkipTest("binding nativo non installato in questo ambiente")
        with mock.patch.dict(os.environ):
            os.environ.pop("PLENORA_IO_BIN", None)
            cls.client = Client()
        if cls.client.backend != "native":
            raise AssertionError("la prova nativa non puo usare subprocess")
