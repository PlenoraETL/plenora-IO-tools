"""Invoked by the Rust integration test with the freshly built binary."""
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))
from plenora_io import Client
import pyarrow.ipc as ipc

client = Client(binary=sys.argv[1])
source = Path(sys.argv[2])
inspected = client.inspect(source)
layer = inspected.layers[0]
assert layer.field("stato").alias == "Stato città"
assert layer.field("misura").domain == "intervallo_量"
assert inspected.domini() == layer.domini()
assert layer.domini()["stato_è"]["code_type"] == "int32"
assert layer.sottotipi() == {
    "field": "stato", "default": 1, "values": [
        {"code": 1, "name": "Strada città", "domains": {"misura": "intervallo_量"}}
    ],
}
output = source.parent / "metadata.arrow"
result = client.read(source, output, layer=0)
with ipc.open_file(output) as reader:
    table = reader.read_all()
assert table.num_rows == 1
assert table["stato"].to_pylist() == [1]
assert table["misura"].to_pylist() == [2.5]
assert table.schema.field("stato").metadata[b"plenora.field.alias"].decode() == layer.field("stato").alias
assert json.loads(table.schema.metadata[b"plenora.dataset.domains"]) == inspected.domini()
assert json.loads(table.schema.metadata[b"plenora.layer.subtypes"]) == layer.sottotipi()
for category in ("native_domain_not_transported", "native_subtype_not_transported", "native_alias_not_transported"):
    assert category not in result.raw.get("read_loss", {}).get("counts", {})

# Native writer: domains and aliases survive; GDAL offers no subtype creation API.
written = source.parent / "written.gdb"
converted = client.convert(output, written, source_format="ipc", target_format="filegdb", layer=0)
after = client.inspect(written).layers[0]
assert after.domini() == layer.domini()
assert after.field("stato").alias == layer.field("stato").alias
assert after.field("misura").domain == layer.field("misura").domain
assert after.sottotipi() is None
