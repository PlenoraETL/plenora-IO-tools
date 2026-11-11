"""Lo SDK conserva lo schema IPC senza attraversare arrow-pyarrow."""

import tempfile
import os
import shutil
import unittest
import sys
from unittest import mock
from pathlib import Path

from plenora_io._arrow import scrivi_ipc, leggi_ipc
from test_contratto_sdk import pa, serve_pyarrow
from plenora_io import Client, CommandFailed, InvalidArgumentError, ProtocolError
from _nativo import client_di_prova, variante_nativa


class StructBitmapCorta(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.binario = os.environ.get("PLENORA_IO_BIN") or shutil.which("plenora-io")

    def setUp(self):
        if self.binario is None:
            self.skipTest("serve il backend plenora-io")

    def test_bitmap_padre_corta_non_consegna_batch(self):
        client = client_di_prova(self.binario)
        for mode in ("file", "stream"):
            fixture = Path(__file__).parent / "fixtures" / f"struct-short-bitmap.{mode}"
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as directory:
                source = Path(directory) / ("input.arrow" if mode == "file" else "input.arrows")
                source.write_bytes(fixture.read_bytes())
                output = Path(directory) / "consegna.arrow"
                for operation in (
                    lambda: client.inspect(source),
                    lambda: client.layers(source),
                    lambda: client.validate(source),
                    lambda: client.read(source, output),
                    lambda: client.write(source, output, format="ipc"),
                    lambda: client.convert(source, output, source_format="ipc", target_format="ipc"),
                ):
                    with self.assertRaises(CommandFailed) as caught:
                        operation()
                    error = caught.exception
                    self.assertEqual(error.category, "data_mapping")
                    self.assertEqual(error.phase, "read")
                    self.assertEqual(error.message, "bitmap di validita' Arrow piu' corta della lunghezza dichiarata")
                    self.assertNotIn("PAYLOAD_MUST_NOT_LEAK", str(error))
                    self.assertFalse(output.exists())

    def test_matrice_ipc_non_consegna_batch(self):
        client = client_di_prova(self.binario)
        cases = (
            "struct-short-bitmap", "primitive-short", "list-offset-oob",
            "map-offset-oob", "struct-children-different", "dictionary-oob",
            "struct-child-short-bitmap", "list-struct-short-bitmap", "ree-short",
        )
        for case in cases:
            for mode in ("file", "stream"):
                fixture = Path(__file__).parent / "fixtures" / f"{case}.{mode}"
                with self.subTest(case=case, mode=mode), tempfile.TemporaryDirectory() as directory:
                    source = Path(directory) / ("input.arrow" if mode == "file" else "input.arrows")
                    source.write_bytes(fixture.read_bytes())
                    output = Path(directory) / "consegna.arrow"
                    for operation in (
                        lambda: client.read(source, output),
                        lambda: client.write(source, output, format="ipc"),
                        lambda: client.convert(source, output, source_format="ipc", target_format="ipc"),
                    ):
                        with self.assertRaises(CommandFailed) as caught:
                            operation()
                        self.assertEqual(caught.exception.category, "data_mapping")
                        self.assertEqual(caught.exception.phase, "read")
                        self.assertNotIn("PAYLOAD_MUST_NOT_LEAK", str(caught.exception))
                        self.assertFalse(output.exists())


StructBitmapCortaNativa = variante_nativa(StructBitmapCorta)


@serve_pyarrow
class SchemaIpc(unittest.TestCase):
    def test_struct_bitmap_padre_corta_consegna_sdk(self):
        for mode in ("file", "stream"):
            source = Path(__file__).parent / "fixtures" / f"struct-short-bitmap.{mode}"
            with self.subTest(mode=mode), self.assertRaises(ProtocolError) as caught:
                leggi_ipc(source, f"application/vnd.apache.arrow.{mode}")
            self.assertEqual(caught.exception.category, "protocol")
            self.assertEqual(caught.exception.phase, "finalize")
            self.assertNotIn("PAYLOAD_MUST_NOT_LEAK", str(caught.exception))

    def test_struct_bitmap_padre_corta_ingresso_sdk(self):
        for mode in ("file", "stream"):
            source = Path(__file__).parent / "fixtures" / f"struct-short-bitmap.{mode}"
            reader = (pa.ipc.open_file if mode == "file" else pa.ipc.open_stream)(source)

            def batches():
                yield reader.get_batch(0) if mode == "file" else reader.read_next_batch()

            producer = pa.RecordBatchReader.from_batches(reader.schema, batches())
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as directory:
                with self.assertRaises(InvalidArgumentError) as caught:
                    scrivi_ipc(producer, Path(directory))
                self.assertEqual(caught.exception.category, "invalid_configuration")
                self.assertEqual(caught.exception.phase, "validate")
                self.assertNotIn("PAYLOAD_MUST_NOT_LEAK", str(caught.exception))
                with pa.ipc.open_file((Path(directory) / "ingresso.arrow").read_bytes()) as output:
                    self.assertEqual(output.num_record_batches, 0)
            reader = (pa.ipc.open_file if mode == "file" else pa.ipc.open_stream)(source)
            batch = reader.get_batch(0) if mode == "file" else reader.read_next_batch()
            for value in (batch, pa.Table.from_batches([batch])):
                with self.subTest(mode=mode, kind=type(value).__name__), tempfile.TemporaryDirectory() as directory:
                    with self.assertRaises(InvalidArgumentError) as caught:
                        scrivi_ipc(value, Path(directory))
                    self.assertEqual(caught.exception.category, "invalid_configuration")
                    self.assertEqual(caught.exception.phase, "validate")
                    self.assertNotIn("PAYLOAD_MUST_NOT_LEAK", str(caught.exception))
                    self.assertFalse((Path(directory) / "ingresso.arrow").exists())

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


@serve_pyarrow
class MatriceArrowEsterno(unittest.TestCase):
    def test_originali_invalidi_non_riserializzati(self):
        fixtures = Path(__file__).parent / "fixtures"
        client = Client(sys.executable)
        cases = ["struct-short-bitmap", "primitive-short", "list-offset-oob",
                 "map-offset-oob", "struct-children-different", "dictionary-oob",
                 "struct-child-short-bitmap", "list-struct-short-bitmap", "ree-short"]
        for case in cases:
            for mode in ("file", "stream"):
                source = fixtures / f"{case}.{mode}"
                for kind in ("batch", "table", "reader", "public-batch", "public-table", "public-reader"):
                    with self.subTest(case=case, mode=mode, kind=kind):
                        reader = (pa.ipc.open_file if mode == "file" else pa.ipc.open_stream)(source)
                        batch = reader.get_batch(0) if mode == "file" else reader.read_next_batch()
                        with self.assertRaises(pa.ArrowInvalid):
                            batch.validate(full=True)
                        if kind.endswith("batch"):
                            value = batch
                        elif kind.endswith("table"):
                            value = pa.Table.from_batches([batch])
                        else:
                            def batches():
                                yield batch
                            value = pa.RecordBatchReader.from_batches(batch.schema, batches())
                        with tempfile.TemporaryDirectory() as directory:
                            with mock.patch.object(Client, "_chiama") as backend:
                                with self.assertRaises(InvalidArgumentError) as caught:
                                    if kind.startswith("public-"):
                                        client.write(value, Path(directory) / "output.arrow", format="ipc", temp_dir=directory)
                                    else:
                                        scrivi_ipc(value, Path(directory))
                                backend.assert_not_called()
                            self.assertEqual(caught.exception.category, "invalid_configuration")
                            self.assertEqual(caught.exception.phase, "validate")
                            self.assertNotIn("PAYLOAD_MUST_NOT_LEAK", str(caught.exception))

    def test_consegne_invalide_non_diventano_tabelle(self):
        fixtures = Path(__file__).parent / "fixtures"
        for source in fixtures.glob("*.*"):
            if source.suffix not in (".file", ".stream"):
                continue
            with self.subTest(source=source.name), self.assertRaises(ProtocolError):
                leggi_ipc(source, "application/vnd.apache.arrow" + source.suffix)


@serve_pyarrow
class CapsuleArrow(unittest.TestCase):
    def test_capsule_valide_conservano_batch(self):
        batch = pa.record_batch([pa.array([1, 2, None])], names=["id"])

        class Stream:
            def __arrow_c_stream__(self, requested_schema=None):
                return pa.RecordBatchReader.from_batches(
                    batch.schema, [batch, batch]
                ).__arrow_c_stream__(requested_schema)

        class Array:
            def __arrow_c_array__(self, requested_schema=None):
                return batch.__arrow_c_array__(requested_schema)

        for value, count in ((Stream(), 2), (Array(), 1), (batch, 1)):
            with self.subTest(kind=type(value).__name__), tempfile.TemporaryDirectory() as directory:
                output = scrivi_ipc(value, Path(directory))
                with pa.ipc.open_file(output.read_bytes()) as reader:
                    self.assertEqual(reader.num_record_batches, count)
                    for i in range(count):
                        self.assertEqual(reader.get_batch(i), batch)

    def test_capsule_rifiutano_difetti_verificabili(self):
        for case in ("list-offset-oob", "map-offset-oob", "dictionary-oob", "ree-short", "struct-children-different"):
            for protocol in ("stream", "array"):
                source = Path(__file__).parent / "fixtures" / f"{case}.file"
                batch = pa.ipc.open_file(source).get_batch(0)

                class Stream:
                    def __arrow_c_stream__(self, requested_schema=None):
                        return pa.RecordBatchReader.from_batches(
                            batch.schema, [batch]
                        ).__arrow_c_stream__(requested_schema)

                class Array:
                    def __arrow_c_array__(self, requested_schema=None):
                        return batch.__arrow_c_array__(requested_schema)

                value = Stream() if protocol == "stream" else Array()
                with self.subTest(case=case, protocol=protocol), tempfile.TemporaryDirectory() as directory:
                    with mock.patch.object(Client, "_chiama") as backend:
                        with self.assertRaises(InvalidArgumentError) as caught:
                            Client(sys.executable).write(value, Path(directory) / "out.arrow", format="ipc", temp_dir=directory)
                        backend.assert_not_called()
                    self.assertEqual(caught.exception.category, "invalid_configuration")
                    self.assertEqual(caught.exception.phase, "validate")

    def test_reader_lazy_valida_prima_di_consegnare_al_writer(self):
        from plenora_io._arrow import _batch_validati
        source = Path(__file__).parent / "fixtures" / "struct-short-bitmap.file"
        bad = pa.ipc.open_file(source).get_batch(0)
        seen = []

        def batches():
            seen.append("read")
            yield bad

        reader = pa.RecordBatchReader.from_batches(bad.schema, batches())
        validated = _batch_validati(reader)
        self.assertEqual(seen, [])
        with self.assertRaises(pa.ArrowInvalid):
            next(validated)
        self.assertEqual(seen, ["read"])


    def test_secondo_batch_invalido_non_raggiunge_backend(self):
        bad = pa.ipc.open_file(Path(__file__).parent / "fixtures" / "struct-short-bitmap.file").get_batch(0)
        good = pa.record_batch([pa.array([None] * 9, type=bad.column(0).type)], schema=bad.schema)
        good.validate(full=True)
        reader = pa.RecordBatchReader.from_batches(bad.schema, iter([good, bad]))
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "output.arrow"
            with mock.patch.object(Client, "_chiama") as backend:
                with self.assertRaises(InvalidArgumentError):
                    Client(sys.executable).write(reader, output, format="ipc", temp_dir=directory)
                backend.assert_not_called()
            self.assertFalse(output.exists())
            self.assertEqual(list(Path(directory).iterdir()), [])
