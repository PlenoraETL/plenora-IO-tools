"""Deterministic external Arrow fixture for IO-a and the protocol matrix."""
from pathlib import Path
import struct
import pyarrow as pa


def make_table(opaque_count=1, precision="float64"):
    ewkb = struct.pack(">BII4d", 0, 0xe0000001, 4326, 12.000000000000002, -0.0, 5e-324, 125.5)
    opaque = {f'SECRET_KEY_{i}': 'SECRET_VALUE' for i in range(opaque_count)}
    geometry = {
        'plenora.field_id': '41', 'plenora.geometry.encoding': 'ewkb',
        'plenora.geometry.dimensions': 'xyzm', 'plenora.geometry.spatial_semantics': 'geometry',
        'plenora.geometry.precision': precision, 'plenora.geometry.types_declaration': 'exact',
        'plenora.geometry.types': 'point', 'plenora.geometry.crs_resolution': 'resolved',
        'plenora.geometry.crs_id': 'EPSG:4326', 'plenora.geometry.axis_order': 'lon_lat',
        'plenora.geometry.srid': '4326', 'ARROW:extension:name': 'geoarrow.wkb', **opaque,
    }
    schema = pa.schema([
        pa.field('a', pa.int64(), nullable=False, metadata={'plenora.field_id': '0', **opaque}),
        pa.field('b', pa.int64(), nullable=False, metadata={'plenora.field_id': '9'}),
        pa.field('geom', pa.binary(), metadata=geometry),
    ], metadata={'plenora.contract.version': '1', **opaque})
    table = pa.Table.from_arrays([
        pa.array([2**53+123, -(2**63)+1], type=pa.int64()),
        pa.array([1, 2], type=pa.int64()), pa.array([ewkb, None], type=pa.binary()),
    ], schema=schema)
    return table


if __name__ == "__main__":
    table = make_table()
    path = Path(__file__).with_name("property_plan.arrow")
    with pa.OSFile(str(path), "wb") as stream:
        with pa.ipc.new_file(stream, table.schema) as writer:
            writer.write_table(table)
