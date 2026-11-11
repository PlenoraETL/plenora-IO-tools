"""Fixture deterministiche: python sdk/python/tests/fixtures/generate_struct_bitmap.py.

PyArrow 25.0.1; modello: database-tools a8b330d9. Solo il descriptor della
bitmap del padre passa da due byte a uno; figlio, nodi e corpo restano validi.
Le prove Rust e Python consumano gli stessi byte versionati.
"""
from pathlib import Path
import struct

import pyarrow as pa

MARKER = "PAYLOAD_MUST_NOT_LEAK"


def slot(data, table, index):
    vtable = table - struct.unpack_from("<i", data, table)[0]
    return table + struct.unpack_from("<H", data, vtable + 4 + index * 2)[0]


def generate():
    array = pa.StructArray.from_arrays(
        [pa.array([MARKER] * 9)], names=["child"],
        mask=pa.array([True] + [False] * 8),
    )
    batch = pa.record_batch([array], names=["parent"])
    for mode, start in (("file", 8), ("stream", 0)):
        sink = pa.BufferOutputStream()
        writer = pa.ipc.new_file if mode == "file" else pa.ipc.new_stream
        with writer(sink, batch.schema) as output:
            output.write_batch(batch)
        raw = sink.getvalue().to_pybytes()
        reader = (pa.ipc.open_file if mode == "file" else pa.ipc.open_stream)(raw)
        actual = reader.get_batch(0) if mode == "file" else reader.read_next_batch()
        assert actual.equals(batch)
        data = bytearray(raw)
        start += 8 + struct.unpack_from("<I", data, start + 4)[0]
        message = start + 8
        table = message + struct.unpack_from("<I", data, message)[0]
        assert data[slot(data, table, 1)] == 3  # RecordBatch
        header = slot(data, table, 2)
        record = header + struct.unpack_from("<I", data, header)[0]
        buffers = slot(data, record, 2)
        vector = buffers + struct.unpack_from("<I", data, buffers)[0]
        length = vector + 4 + 8  # primo buffer: validita' del padre
        assert struct.unpack_from("<q", data, length)[0] == 2
        struct.pack_into("<q", data, length, 1)
        assert sum(a != b for a, b in zip(raw, data)) == 1
        (Path(__file__).parent / f"struct-short-bitmap.{mode}").write_bytes(data)


if __name__ == "__main__":
    generate()
