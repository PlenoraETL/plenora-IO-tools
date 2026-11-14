"""Enumerate the pinned normative vectors, vocabularies and campaign domains."""
import copy
import itertools
import json
from pathlib import Path
import struct
import unittest
from types import SimpleNamespace

import pyarrow as pa
from scripts import qualification_contract as qc, system_qualification as q, qualification_audit

ROOT = Path(__file__).resolve().parents[1]


def arrow(vector):
    aliases = {'utf8': pa.string(), 'large_utf8': pa.large_string()}
    fields, columns = [], []
    for field in vector['fields']:
        kind = aliases.get(field['type']) or pa.type_for_alias(field['type'])
        fields.append(pa.field(field['name'], kind, field['nullable'], field['metadata']))
        values = [row[field['name']] for row in vector.get('rows', [])]
        if pa.types.is_binary(kind) or pa.types.is_large_binary(kind):
            values = [None if v is None else bytes.fromhex(v) for v in values]
        columns.append(pa.array(values, type=kind))
    return pa.Table.from_arrays(columns, schema=pa.schema(fields, metadata=vector['schema_metadata']))


class NormativeEnumeration(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.contract = qc.contract()

    def test_every_pinned_input_vector(self):
        paths = [path for directory in ('vectors/arrow-v1', 'vectors/arrow-data-v1', 'vectors/arrow-primary-v1')
                 for path in self.contract.paths(directory) if path.endswith('.json')]
        self.assertTrue(paths)
        for path in paths:
            vector = json.loads(self.contract.read(path))
            with self.subTest(path=path):
                found = self.contract.verdict(arrow(vector))
                self.assertEqual(found is None, vector['expect'] == 'valid')
                if 'category' in vector:
                    self.assertEqual(found.category, vector['category'])

    def test_every_validator_predicate_and_gate_field_has_reviewed_authority(self):
        self.assertEqual(json.loads(qualification_audit.PATH.read_text(encoding='utf-8')), qualification_audit.inventory())

    def test_every_enum_value_and_missing_key_against_normative_oracle(self):
        base = json.loads(self.contract.read('vectors/arrow-data-v1/wkb-with-declared-srid.json'))
        base['rows'] = []
        index = next(i for i, f in enumerate(base['fields']) if self.contract.data.is_geometry(f))
        for key, values in self.contract.data.CLOSED_VALUES.items():
            for value in [*sorted(values), 'UNRECOGNIZED', None]:
                vector = copy.deepcopy(base)
                if value is None:
                    vector['fields'][index]['metadata'].pop(key, None)
                else:
                    vector['fields'][index]['metadata'][key] = value
                with self.subTest(key=key, value=value):
                    self.assertEqual(self.contract.verdict(arrow(vector)), self.contract.primary.verdict(vector))
        gate = json.loads((ROOT / 'release/system-rc-gate.json').read_text(encoding='utf-8'))
        self.assertEqual(gate['normative_vocabulary']['closed_values'], {k: sorted(v) for k, v in self.contract.data.CLOSED_VALUES.items()})

    def test_wkt_wkt2_projjson_all_top_level_identifier_rules(self):
        for format_, consistent, contradictory, nested in (
            ('wkt', 'GEOGCS["x",AUTHORITY["EPSG","4326"]]', 'GEOGCS["x",AUTHORITY["EPSG","3003"]]', 'PROJCS["x",GEOGCS["y",AUTHORITY["EPSG","3003"]]]'),
            ('wkt2', 'GEOGCRS["x",ID["EPSG",4326]]', 'GEOGCRS["x",ID["EPSG",3003]]', 'PROJCRS["x",BASEGEOGCRS["y",ID["EPSG",3003]]]'),
            ('projjson', '{"id":{"authority":"EPSG","code":4326}}', '{"id":{"authority":"EPSG","code":3003}}', '{"base_crs":{"id":{"authority":"EPSG","code":3003}}}'),
        ):
            for definition, expected in ((consistent, False), (contradictory, True), (nested, False)):
                with self.subTest(format=format_, expected=expected):
                    field = pa.field('g', pa.binary(), metadata={'plenora.geometry.crs_id': 'EPSG:4326',
                        'plenora.geometry.crs_definition': definition, 'plenora.geometry.crs_definition_format': format_})
                    self.assertEqual(self.contract.contradiction(field), expected)

    def test_non_geometry_uint64_and_all_variants(self):
        table = pa.table({'value': pa.array([2**64 - 1, 0, 2**63], pa.uint64())}).replace_schema_metadata({'plenora.contract.version': '1'})
        self.assertIsNone(self.contract.verdict(table))
        for variant in q.VARIANTS:
            q.check_variant(variant, table)
            q.check_fixture('uint64_ordering', {}, table)
        with self.assertRaises(q.Invalid):
            q.check_fixture('uint64_ordering', {}, table.sort_by('value'))

    def test_all_geometry_types_and_empty_values_are_normatively_decoded(self):
        # Header-only containers and empty lines are lawful; the old parser
        # rejected empties and all types beyond seven before consulting ICD.
        for code, name in self.contract.data.GEOMETRY_TYPES.items():
            if name == 'point':
                blob = struct.pack('<BI2d', 1, code, 1., 2.)
            else:
                blob = struct.pack('<BII', 1, code, 0)
            with self.subTest(type=name):
                self.assertEqual(self.contract.data.decode_wkb(blob).type, name)

    def test_campaign_has_exactly_one_requirement_per_fixture_and_variant(self):
        gate = json.loads((ROOT / 'release/system-rc-gate.json').read_text(encoding='utf-8'))
        self.assertEqual(set(gate['required_fixtures']), set(q.FIXTURES))
        self.assertEqual(set(gate['fixture_variants']), set(q.VARIANTS))
        self.assertEqual(gate['fixture_requirements'], q.FIXTURES)
        self.assertEqual(gate['variant_rules'], q.VARIANTS)
        self.assertEqual(len(list(itertools.product(q.FIXTURES, q.VARIANTS))),
                         len(gate['required_fixtures']) * len(gate['fixture_variants']))

    def test_schema_rejection_phases_come_from_rej003_and_error_schema(self):
        self.assertEqual(set(self.contract.schema_rejection_phases), {'validate', 'connect', 'probe'})
        self.assertLessEqual(set(self.contract.schema_rejection_phases), set(self.contract.error_schema['properties']['phase']['enum']))

    def test_lawful_sort_and_reorder_do_not_require_invented_metadata_loss(self):
        schema = pa.schema([pa.field('value', pa.uint64(), metadata={'plenora.field_id': '7'}),
                            pa.field('other', pa.int64(), metadata={'plenora.field_id': '8'})],
                           metadata={'plenora.contract.version': '1'})
        source = pa.Table.from_arrays([pa.array([2**64 - 1, 0, 2**63], pa.uint64()), pa.array([1, 2, 3])], schema=schema)
        actual = source.sort_by('value').select(['other', 'value'])
        keys = q.property_keys(ROOT)
        tables = {'source': source, 'actual': actual, 'expected': actual}
        bundle = SimpleNamespace(keys=keys, contract=self.contract,
            arrow=lambda name, kind: (q.property_observation(tables[name], keys), tables[name]))
        hop = {'operation': 'explicit_plan', 'expected': 'expected',
               'authorization': {'model': 'component_contract_and_options'},
               'properties': dict.fromkeys(keys, 'preserved')}
        q.compare_hop(bundle, hop, 'source', 'actual', {'kind': 'tabular_plan', 'operation_id': 'data.run@3'})
        hop['operation'] = 'identity'
        with self.assertRaises(q.Invalid):
            q.compare_hop(bundle, hop, 'source', 'actual', {'kind': 'tabular_plan', 'operation_id': 'data.run@3'})

    def test_primary_normalization_preserves_state_not_redundant_spelling(self):
        vector = json.loads(self.contract.read('vectors/arrow-primary-v1/primary-explicit-one-geometry.json'))
        source = arrow(vector)
        actual = self.contract.canonical_primary(source)
        self.assertNotEqual(source.schema.metadata, actual.schema.metadata)
        self.assertEqual(q.canonical_arrow(source), q.canonical_arrow(actual))

    def test_arrow_types_and_float_payloads_are_not_rebuilt_through_python(self):
        arrays = [pa.UnionArray.from_sparse(pa.array([0, 1], pa.int8()), [pa.array([1, None]), pa.array([None, 'a'])]),
                  pa.array(['a', 'b', 'a']).dictionary_encode(),
                  pa.RunEndEncodedArray.from_arrays(pa.array([2, 4], pa.int16()), pa.array([1, 2]))]
        bits = struct.pack('<IIII', 0x80000000, 0x7fc00001, 0x7fc00002, 0x00000001)
        arrays.append(pa.Array.from_buffers(pa.float32(), 4, [None, pa.py_buffer(bits)]))
        for array in arrays:
            table = pa.table({'value': array}).replace_schema_metadata({'plenora.contract.version': '1'})
            with self.subTest(type=str(array.type)):
                self.assertIsNone(self.contract.verdict(table))
                output = pa.ipc.open_stream(q.canonical_arrow(table)).read_all()
                self.assertEqual(output.schema, table.schema)
                if pa.types.is_float32(array.type):
                    self.assertEqual(output.column(0).chunk(0).buffers()[1].to_pybytes(), bits)
                else:
                    self.assertTrue(output.equals(table))

    def test_computation_is_not_inferred_from_input_conformance(self):
        vector = json.loads(self.contract.read('vectors/arrow-data-v1/crs-local-authority-mysql-epsg-like-code.json'))
        table = arrow(vector)
        self.assertIsNone(self.contract.verdict(table))
        semantics = {'kind': 'geometry_compute', 'operation_id': 'data.run@3',
                     'geometry_selection': None, 'produced_geometry_ids': ['1']}
        with self.assertRaisesRegex(q.Invalid, 'GEO-006/015/019/020'):
            q.validate_operation(self.contract, semantics, 'derived', table, table)
        semantics['geometry_selection'] = '999'
        with self.assertRaisesRegex(q.Invalid, 'PRIM-007'):
            q.validate_operation(self.contract, semantics, 'derived', table, table)
        q.validate_operation(self.contract, {'kind': 'pass_through', 'operation_id': 'io.read@2'}, 'identity', table, table)
        with self.assertRaisesRegex(q.Invalid, 'pass-through'):
            q.validate_operation(self.contract, {'kind': 'pass_through', 'operation_id': 'io.read@2'}, 'derived', table, table)
