"""Real GPKG round-trips: the same property plan through sync and async SDK."""
import asyncio
import os
from pathlib import Path
import struct
import tempfile
import unittest

from plenora_io import AsyncClient, Client, ResourceLimitError
from _repository import RADICE

try:
    import pyarrow as pa
except ImportError:
    pa = None


class PropertyFixture:
    native = False

    def client(self, asynchronous=False):
        kind = AsyncClient if asynchronous else Client
        client = kind() if self.native else kind(binary=self.binary)
        if self.native and not asynchronous:
            self.assertEqual(client.backend, "native")
        return client

    def setUp(self):
        if pa is None:
            self.skipTest("serve pyarrow, l'extra `plenora-io[pyarrow]`, che l'installazione dalla sdist non porta")
        default = RADICE / 'target/debug' / ('plenora-io.exe' if os.name == 'nt' else 'plenora-io') if RADICE else None
        configured = os.environ.get('PLENORA_IO_BIN') or default
        self.binary = Path(configured) if configured else None
        if not self.native and (self.binary is None or not self.binary.is_file()):
            self.skipTest("real CLI required")
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.input = self.root / 'input.arrow'
        self.output = self.root / 'output.gpkg'
        self.delivered = self.root / 'read.arrow'
        self.ordinates = (12.000000000000002, -0.0, 5e-324, 125.5)
        self.ewkb = struct.pack('>BII4d', 0, 0xe0000001, 4326, *self.ordinates)
        self.expected = struct.pack('<BI4d', 1, 3001, *self.ordinates)
        self.make_input()

    def make_input(self, opaque_count=1, precision="float64"):
        from fixtures.generate_property_plan import make_table
        self.table = make_table(opaque_count, precision)
        schema = self.table.schema
        with pa.OSFile(str(self.input), 'wb') as stream:
            with pa.ipc.new_file(stream, schema) as writer:
                writer.write_table(self.table)

    def check_reports(self, written, read):
        changes = written.write_loss.property_changes
        self.assertTrue(changes)
        removed = [c.effect for c in changes if c.effect.rule == 'removed']
        self.assertEqual({(e.field_index, e.field_id) for e in removed}, {(None, None), (0, 0), (2, 41)})
        for rule in ('ewkb_to_wkb', 'srid_to_catalog'):
            effect = next(c.effect for c in changes if c.effect.rule == rule)
            self.assertEqual((effect.field_index, effect.field_id), (2, 41))
        self.assertEqual([c.effect.raw for c in changes], [c.effect.raw for c in read.loss.property_changes])
        self.assertTrue(all(c.phase == 'source_write' for c in read.loss.property_changes))
        self.assertEqual(read.loss.count('gpkg_metadata_not_preserved'), 0)
        self.assertEqual(read.loss.count('geometry_representation_normalized'), 0)
        self.assertNotIn('SECRET', str(written.write_loss.raw))
        self.assertNotIn('SECRET', str(read.loss.raw))
        with pa.memory_map(str(self.delivered), 'r') as stream:
            output = pa.ipc.open_file(stream).read_all()
        self.assertEqual(output['geom'].to_pylist(), [self.expected, None])
        self.assertEqual(output['a'].to_pylist(), self.table['a'].to_pylist())
        self.assertEqual(output.schema.field('geom').metadata[b'plenora.field_id'], b'41')
        for metadata in [output.schema.metadata, *(f.metadata for f in output.schema)]:
            self.assertFalse(any(b'SECRET' in k or b'SECRET' in v for k, v in (metadata or {}).items()))


class PropertyPlanSync(PropertyFixture, unittest.TestCase):
    def test_roundtrip(self):
        client = self.client()
        written = client.write(self.input, self.output, format='gpkg')
        read = client.read(self.output, self.delivered)
        self.check_reports(written, read)

    def test_complete_report_budget_precedes_effect(self):
        self.make_input(200)
        with self.assertRaises(ResourceLimitError) as caught:
            self.client().write(self.input, self.output, format='gpkg')
        self.assertNotIn('SECRET', str(caught.exception))
        self.assertFalse(self.output.exists())
        self.assertEqual(list(self.root.iterdir()), [self.input])

    def test_native_precision_reconstruction_is_declared(self):
        self.make_input(precision="native")
        client = self.client()
        written = client.write(self.input, self.output, format="gpkg")
        read = client.read(self.output, self.delivered)
        self.check_reports(written, read)
        effect = next(c.effect for c in written.write_loss.property_changes
                      if c.effect.property == "precision")
        self.assertEqual((effect.field_index, effect.field_id, effect.rule), (2, 41, "reconstructed"))
        with pa.memory_map(str(self.delivered), "r") as stream:
            schema = pa.ipc.open_file(stream).schema
        self.assertEqual(schema.field("geom").metadata[b"plenora.geometry.precision"], b"float64")

    def test_persisted_plan_rejects_mutation_without_echoing_data(self):
        import json
        import sqlite3
        from plenora_io import PlenoraError
        client = self.client()
        client.write(self.input, self.output, format="gpkg")
        baseline = self.output.read_bytes()
        for key, value in (("field_id", 999), ("field_id", None), ("field_index", 999),
                           ("key_index", 999), ("rule", "srid_removed"),
                           ("property", "SECRET_INVALID")):
            self.output.write_bytes(baseline)
            with sqlite3.connect(self.output) as connection:
                row = connection.execute("SELECT id, metadata FROM gpkg_metadata").fetchone()
                document = json.loads(row[1])
                effect = next(e for e in document["properties"]["effects"]
                              if e["field_index"] == 2)
                effect[key] = value
                connection.execute("UPDATE gpkg_metadata SET metadata=? WHERE id=?", (json.dumps(document), row[0]))
            connection.close()
            with self.assertRaises(PlenoraError) as caught:
                client.read(self.output, self.delivered)
            self.assertNotIn("SECRET", str(caught.exception))
            self.assertFalse(self.delivered.exists())


class PropertyPlanAsync(PropertyFixture, unittest.IsolatedAsyncioTestCase):
    async def test_roundtrip(self):
        client = self.client(True)
        written = await client.write(self.input, self.output, format='gpkg')
        read = await client.read(self.output, self.delivered)
        await asyncio.to_thread(self.check_reports, written, read)

    async def test_complete_report_budget_precedes_effect(self):
        await asyncio.to_thread(self.make_input, 200)
        with self.assertRaises(ResourceLimitError) as caught:
            await self.client(True).write(self.input, self.output, format='gpkg')
        self.assertNotIn('SECRET', str(caught.exception))
        self.assertFalse(await asyncio.to_thread(self.output.exists))
        self.assertEqual(await asyncio.to_thread(lambda: list(self.root.iterdir())), [self.input])


class PropertyPlanNativeSync(PropertyPlanSync):
    native = True

    def setUp(self):
        if pa is None:
            super().setUp()
        from _nativo import binario_nativo
        if binario_nativo() is None:
            self.skipTest("native module required")
        super().setUp()
        self.assertEqual(Client().backend, "native")


class PropertyPlanNativeAsync(PropertyPlanAsync):
    native = True

    def setUp(self):
        if pa is None:
            super().setUp()
        from _nativo import binario_nativo
        if binario_nativo() is None:
            self.skipTest("native module required")
        super().setUp()
        self.assertEqual(Client().backend, "native")
