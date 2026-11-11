"""Prima stringa valida, offset successivo fuori buffer; nessun unsafe Rust."""
from pathlib import Path
import struct
import pyarrow as pa

array = pa.Array.from_buffers(pa.string(), 3, [
    None, pa.py_buffer(struct.pack("<4i", 0, 1, 99, 3)), pa.py_buffer(b"abc")])
batch = pa.record_batch([array], names=["v"])
sink = pa.BufferOutputStream()
with pa.ipc.new_stream(sink, batch.schema) as writer:
    writer.write_batch(batch)
Path(__file__).with_name("string-later-offset.stream").write_bytes(sink.getvalue().to_pybytes())
