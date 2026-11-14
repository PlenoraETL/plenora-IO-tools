"""Synthetic bundles exercise the validator, never attest an external campaign."""
import copy
import hashlib
import itertools
import json
from pathlib import Path
import struct
import tempfile
import unittest
from unittest import mock

import pyarrow as pa

from scripts import system_qualification as q

ROOT = Path(__file__).resolve().parents[1]


def geometry_fixture(name):
    point = struct.pack('<BIdd', 1, 1, 1., -0.)
    if name in ('point_z', 'point_zm'):
        point = struct.pack('<BI', 1, 1001 if name == 'point_z' else 3001) + struct.pack('<ddd', 1., 2., 3.)
        if name == 'point_zm':
            point += struct.pack('<d', 4.)
    if name in ('polygon_xy_projected', 'multipolygon_xyzm_srid'):
        zm = name == 'multipolygon_xyzm_srid'
        ring = [(0., 0.), (2., 0.), (2., 2.), (0., 2.), (0., 0.)]
        point = struct.pack('<BIII', 1, 3003 if zm else 3, 1, len(ring))
        point += b''.join(struct.pack('<dddd', x, y, 3., 4.) if zm else struct.pack('<dd', x, y) for x, y in ring)
        if zm:
            point = struct.pack('<BIII', 1, 0xe0000006, 4326, 1) + point
    if name == 'types_mixed':
        return [point, struct.pack('<BII', 1, 2, 2) + struct.pack('<dddd', 0., 0., 1., 1.)]
    return [point, point]


class SystemQualification(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.directory = tempfile.TemporaryDirectory(dir=ROOT / 'target')
        cls.addClassCleanup(cls.directory.cleanup)
        cls.root = Path(cls.directory.name)
        for name in ('contracts/adoption-source.json', 'crates/plenora-io-core/src/property_plan.rs'):
            path = cls.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes((ROOT / name).read_bytes())
        cls.original = json.loads((ROOT / 'release/system-rc-gate.json').read_text(encoding='utf-8'))
        cls.original.update(status='satisfied', open_blockers=[])
        cls.artifacts = {}
        cls.files = {}

        def artifact(kind, label, data):
            name = f'assurance/evidence/synthetic/{label}'
            if not isinstance(data, bytes):
                data = (json.dumps(data, sort_keys=True) + '\n').encode()
            cls.files[name] = data
            cls.artifacts[name] = {'kind': kind, 'path': name, 'sha256': hashlib.sha256(data).hexdigest()}
            return name

        commits = {}
        revisions = {}
        for name in [*q.COMPONENTS, 'plenora-contracts', 'harness']:
            raw = (f'tree {"a" * 40}\nauthor Synthetic <test@example.invalid> 0 +0000\n'
                   f'committer Synthetic <test@example.invalid> 0 +0000\n\n{name}\n').encode()
            if name == 'plenora-contracts':
                oracle = q.qualification_contract.contract()
                raw = q.qualification_contract.git_bytes(oracle.repository, 'cat-file', 'commit', oracle.revision)
            revisions[name] = hashlib.sha1(b'commit ' + str(len(raw)).encode() + b'\0' + raw).hexdigest()
            commits[name] = artifact('git_commit', name + '.commit', raw)
        for component in cls.original['components']:
            component['revision'] = revisions[component['name']]
        cls.original['icd']['revision'] = revisions['plenora-contracts']
        adoption = json.loads((cls.root / 'contracts/adoption-source.json').read_text(encoding='utf-8'))
        adoption['contracts_source']['revision'] = revisions['plenora-contracts']
        (cls.root / 'contracts/adoption-source.json').write_text(json.dumps(adoption), encoding='utf-8')
        executables = []
        for component, platform, surface in itertools.product(q.COMPONENTS, q.DOMAINS['required_platforms'], q.DOMAINS['required_surfaces']):
            binary = artifact('binary', f'{component}-{platform}-{surface}.bin', b'synthetic executable identity')
            executables.append({'component': component, 'platform': platform, 'surface': surface,
                                'revision': revisions[component], 'artifact': binary})
        lock = artifact('lock', 'lock.json', {'owner': q.OWNER, 'components': cls.original['components'],
            'icd_revision': revisions['plenora-contracts'], 'commits': commits, 'executables': executables,
            'harness': {'revision': revisions['harness']}})
        environments = []
        for platform, os_name in [('linux-native', 'Linux'), ('windows-native', 'Windows')]:
            environments.append({'platform': platform, 'os': os_name, 'architecture': 'x86_64',
                'execution': 'native', 'container': False, 'wsl': False, 'run_id': platform,
                'harness_revision': revisions['harness'], 'log': artifact('log', platform + '.log', b'synthetic environment')})
        environment = artifact('environment', 'environment.json', {'runs': environments})
        fixture_items = []
        outputs = {}
        expected = {}
        source_tables = {}
        for fixture, variant in itertools.product(q.DOMAINS['required_fixtures'], q.DOMAINS['fixture_variants']):
            metadata = {value: 'synthetic' for key, value in q.property_keys(cls.root).items()
                        if key not in ('contract_version', 'native_metadata')}
            metadata.update({'plenora.field_id': '41', 'plenora.geometry.encoding': 'wkb',
                'plenora.geometry.precision': 'float64', 'plenora.geometry.types_declaration': 'exact',
                'plenora.geometry.types': 'point', 'plenora.geometry.dimensions': 'xy',
                'plenora.geometry.spatial_semantics': 'geometry', 'plenora.geometry.srid': '4326',
                'plenora.geometry.crs_id': 'EPSG:4326', 'plenora.geometry.crs_resolution': 'resolved',
                'plenora.geometry.crs_definition': '{"id":{"authority":"EPSG","code":4326}}',
                'plenora.geometry.crs_definition_format': 'projjson', 'plenora.geometry.axis_order': 'lon_lat',
                'plenora.geometry.native.sentinel': 'synthetic'})
            if fixture == 'crs_unresolved':
                metadata['plenora.geometry.crs_resolution'] = 'declared_unresolved'
            if fixture == 'axis_lat_lon':
                metadata['plenora.geometry.axis_order'] = 'lat_lon'
            if fixture == 'dimensions_unknown':
                metadata['plenora.geometry.dimensions'] = 'unknown'
            if fixture in ('point_z', 'point_zm', 'multipolygon_xyzm_srid'):
                metadata['plenora.geometry.dimensions'] = 'xyz' if fixture == 'point_z' else 'xyzm'
            if fixture == 'geography_semantics':
                metadata['plenora.geometry.spatial_semantics'] = 'geography'
            if fixture == 'multipolygon_xyzm_srid':
                metadata['plenora.geometry.types'] = 'multipolygon'
                metadata['plenora.geometry.encoding'] = 'ewkb'
            if fixture == 'polygon_xy_projected':
                metadata['plenora.geometry.types'] = 'polygon'
                metadata['plenora.geometry.axis_order'] = 'easting_northing'
            if fixture == 'types_mixed':
                metadata['plenora.geometry.types'] = 'point,linestring'
                metadata['plenora.geometry.precision'] = 'native'
                reference = json.loads(q.qualification_contract.contract().read('vectors/arrow-data-v1/crs-proj-epsg-4326-projjson.json'))
                metadata['plenora.geometry.crs_definition'] = next(f['metadata']['plenora.geometry.crs_definition'] for f in reference['fields'] if 'plenora.geometry.crs_definition' in f['metadata'])
            if fixture == 'crs_missing':
                metadata['plenora.geometry.crs_resolution'] = 'missing'
                for key in ('crs_id', 'crs_definition', 'crs_definition_format', 'srid', 'axis_order'):
                    metadata.pop('plenora.geometry.' + key)
            if fixture == 'conflicting_crs':
                metadata['plenora.geometry.crs_definition'] = '{"id":{"authority":"EPSG","code":3003}}'
            metadata['ARROW:extension:name'] = 'geoarrow.wkb'
            if variant == 'geoarrow_extension':
                metadata['ARROW:extension:metadata'] = '{}'
            schema = pa.schema([pa.field('geometry', pa.binary(), metadata=metadata), pa.field('value', pa.uint64() if fixture == 'uint64_ordering' else pa.int64())],
                               metadata={'plenora.contract.version': '1', 'opaque': 'synthetic'})
            table = pa.Table.from_arrays([pa.array(geometry_fixture(fixture)), pa.array([2**64-1, 2**63+1], type=pa.uint64()) if fixture == 'uint64_ordering' else pa.array([2**53+1, 2**53+2])], schema=schema)
            if fixture in ('int64_beyond_2_53', 'uint64_ordering'):
                table = table.select(['value'])
            raw = cls.arrow_bytes(table)
            source_tables[(fixture, variant)] = table
            label = fixture + '-' + variant
            source = artifact('fixture', label + '.arrow', raw)
            fixture_items.append({'fixture': fixture, 'variant': variant, 'input_class': q.FIXTURE_CLASSES[fixture], 'artifact': source})
            if fixture != 'conflicting_crs':
                if fixture == 'crs_unresolved':
                    metadata['plenora.geometry.crs_resolution'] = 'resolved'
                    schema = pa.schema([schema.field(0).with_metadata(metadata), schema.field(1)], metadata=schema.metadata)
                    table = pa.Table.from_arrays(table.columns, schema=schema)
                    raw = cls.arrow_bytes(table)
                if fixture == 'types_mixed':
                    metadata['plenora.geometry.precision'] = 'float64'
                    schema = table.schema.set(0, table.schema.field(0).with_metadata(metadata))
                    table = pa.Table.from_arrays(table.columns, schema=schema)
                    raw = cls.arrow_bytes(table)
                outputs[(fixture, variant)] = artifact('output', label + '-out.arrow', raw)
                expected[(fixture, variant)] = artifact('expected', label + '-expected.arrow', raw)
        fixtures = artifact('fixtures', 'fixtures.json', {'items': fixture_items})
        cells = []
        commands = []
        for number, key in enumerate(itertools.product(*q.DOMAINS.values())):
            direction, platform, surface, fixture, variant = key
            cell = dict(zip(q.AXES, key), status='passed', hops=[])
            order = list(q.COMPONENTS) if direction == 'io_to_data_to_database' else list(reversed(q.COMPONENTS))
            current = next(f['artifact'] for f in fixture_items if (f['fixture'], f['variant']) == (fixture, variant))
            for step, component in enumerate(order):
                command_id = f'cell-{number}-hop-{step}'
                executable = next(e for e in executables if (e['component'], e['platform'], e['surface']) == (component, platform, surface))
                output = None if fixture == 'conflicting_crs' else outputs[(fixture, variant)]
                command = {'id': command_id, 'cell': list(key), 'component': component,
                    'run_id': platform, 'executable': executable['artifact'], 'revision': revisions[component],
                    'argv': ['synthetic-runner', component, surface], 'input': current, 'output': output,
                    'destination': command_id + '-destination', 'exit_code': 3 if output is None else 0}
                if output is not None:
                    kind = 'geometry_compute' if step == 0 and fixture == 'types_mixed' else 'metadata_normalization' if step == 0 and fixture == 'crs_unresolved' else 'pass_through'
                    command['semantics'] = {'kind': kind, 'operation_id': component + '.synthetic@1'}
                    if kind == 'geometry_compute':
                        command['semantics'].update(geometry_selection='41', produced_geometry_ids=['41'])
                command['log'] = artifact('log', command_id + '.log', copy.deepcopy(command))
                commands.append(command)
                hop = {'component': component, 'command': command_id, 'output': output}
                if output is None:
                    hop.update(preservation={'status': 'skipped', 'reason': 'rejected_at_validating_boundary'},
                               properties={}, boundary='first_validating_boundary')
                    hop['error'] = artifact('report', command_id + '-error.json', {'command': command_id,
                        'error': {'category': 'crs', 'message': 'CRS conflict', 'code': 'CRS_CONFLICT', 'phase': 'validate', 'remote_effect': 'none', 'retry': {'kind': 'never'}}})
                    state = {'capture': 'complete_destination_state', 'entries': [],
                             'command': command_id, 'destination': command['destination']}
                    hop['before'] = artifact('snapshot', command_id + '-before.json', state)
                    hop['after'] = artifact('snapshot', command_id + '-after.json', state)
                else:
                    hop.update(operation='identity', expected=expected[(fixture, variant)],
                               properties=dict.fromkeys(q.required_properties(cls.root), 'preserved'))
                    prop = None
                    if step == 0 and fixture == 'types_mixed':
                        hop['operation'], prop = 'derived', 'precision'
                        hop['properties'][prop] = 'derived'
                    if step == 0 and fixture == 'crs_unresolved':
                        hop['operation'], prop = 'explicit_plan', 'crs_resolution'
                        hop['properties'][prop] = 'declared_transformation'
                    if prop:
                        hop['authorization'] = {'model': 'component_contract_and_options'}
                        hop['loss_report'] = artifact('loss', command_id + '-loss.json', {'command': command_id,
                            'property_changes': [{'property': prop, 'locations': [{'field_index': 0, 'field_id': 41, 'key_index': sorted(source_tables[(fixture, variant)].schema.field(0).metadata).index(q.property_keys(cls.root)[prop].encode())}], 'rule': hop['operation']}]})
                    current = output
                cell['hops'].append(hop)
            cells.append(cell)
        report = artifact('report', 'report.json', {'owner': q.OWNER, 'lock': lock, 'results': cells})
        manifest = artifact('manifest', 'manifest.json', {'owner': q.OWNER, 'lock': lock,
            'cells': [list(k) for k in sorted(itertools.product(*q.DOMAINS.values()))]})
        commands_path = artifact('commands', 'commands.json', {'executions': commands})
        cls.baseline_bundle = {'schema_version': q.BUNDLE_VERSION, 'owner': q.OWNER,
            'artifacts': list(cls.artifacts.values()), 'manifest': manifest, 'report': report,
            'commands': commands_path, 'environment': environment, 'fixtures': fixtures, 'lock': lock}
        cls.index_path = 'assurance/evidence/synthetic/bundle.json'
        for name, data in cls.files.items():
            path = cls.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)

    @staticmethod
    def arrow_bytes(table):
        stream = pa.BufferOutputStream()
        with pa.ipc.new_file(stream, table.schema) as writer:
            writer.write_table(table)
        return stream.getvalue().to_pybytes()

    def setUp(self):
        self.document = copy.deepcopy(self.original)
        self.bundle = copy.deepcopy(self.baseline_bundle)
        self.changed = set()
        self.seal()

    def tearDown(self):
        for name in self.changed:
            (self.root / name).write_bytes(self.files[name])

    def seal(self):
        raw = json.dumps(self.bundle).encode()
        (self.root / self.index_path).write_bytes(raw)
        self.document['evidence'] = {'status': 'passed', 'bundle': {'path': self.index_path, 'sha256': hashlib.sha256(raw).hexdigest()}}

    def read(self, name):
        return json.loads((self.root / name).read_bytes())

    def rewrite(self, name, data):
        self.changed.add(name)
        raw = data if isinstance(data, bytes) else json.dumps(data).encode()
        (self.root / name).write_bytes(raw)
        next(a for a in self.bundle['artifacts'] if a['path'] == name)['sha256'] = hashlib.sha256(raw).hexdigest()
        self.seal()

    def check(self):
        return q.validate(self.document, self.root)

    def test_complete_evidence(self):
        self.assertEqual(self.check(), [])

    def test_conflict_must_exist_in_fixture_bytes(self):
        item = next(i for i in self.read(self.bundle['fixtures'])['items'] if i['fixture'] == 'conflicting_crs')
        table = pa.ipc.open_file(self.root / item['artifact']).read_all()
        metadata = dict(table.schema.field(0).metadata)
        metadata[b'plenora.geometry.crs_definition'] = b'{"id":{"authority":"EPSG","code":4326}}'
        schema = table.schema.set(0, table.schema.field(0).with_metadata(metadata))
        self.rewrite(item['artifact'], self.arrow_bytes(pa.Table.from_arrays(table.columns, schema=schema)))
        self.assertIn('classe CRS non corrispondente ai byte della fixture', self.check())

    def test_geometry_payload_must_exercise_named_feature(self):
        for fixture in ('point_z', 'point_zm', 'multipolygon_xyzm_srid', 'polygon_xy_projected', 'types_mixed'):
            with self.subTest(fixture=fixture):
                item = next(i for i in self.read(self.bundle['fixtures'])['items'] if i['fixture'] == fixture)
                table = pa.ipc.open_file(self.root / item['artifact']).read_all()
                wrong = pa.Table.from_arrays([pa.array(geometry_fixture('point')), table.column(1)], schema=table.schema)
                self.rewrite(item['artifact'], self.arrow_bytes(wrong))
                self.assertTrue(self.check())
                self.rewrite(item['artifact'], self.files[item['artifact']])

    def test_conversion_cannot_hide_changed_values(self):
        report = self.read(self.bundle['report'])
        cell = next(c for c in report['results'] if c['fixture'] == 'axis_lat_lon')
        hop = cell['hops'][0]
        hop['operation'] = 'format_conversion'
        for path in (hop['output'], hop['expected']):
            table = pa.ipc.open_file(self.root / path).read_all()
            changed = pa.Table.from_arrays([table.column(0), pa.array([0, 1], type=pa.int64())], schema=table.schema)
            self.rewrite(path, self.arrow_bytes(changed))
        self.rewrite(self.bundle['report'], report)
        self.assertIn('valori cambiati senza trasformazione dichiarata', self.check())

    def test_fixture_requirements_cannot_be_split_across_fields(self):
        item = next(i for i in self.read(self.bundle['fixtures'])['items'] if i['fixture'] == 'multipolygon_xyzm_srid')
        table = pa.ipc.open_file(self.root / item['artifact']).read_all()
        wrong = dict(table.schema.field(0).metadata)
        wrong[b'plenora.geometry.types'] = b'point'
        schema = pa.schema([table.schema.field(0).with_metadata(wrong),
                            pa.field('other', pa.binary(), metadata=table.schema.field(0).metadata)], metadata=table.schema.metadata)
        crossed = pa.Table.from_arrays([table.column(0), pa.array(geometry_fixture('point'))], schema=schema)
        self.rewrite(item['artifact'], self.arrow_bytes(crossed))
        self.assertTrue(self.check())

    def test_each_domain_and_property_cannot_be_removed(self):
        baseline = copy.deepcopy(self.document)
        for field in (*q.DOMAINS, 'required_properties', 'required_operations'):
            self.document = copy.deepcopy(baseline)
            self.document[field] = self.document[field][:-1]
            with self.subTest(field=field):
                self.assertTrue(self.check())

    def test_missing_bundle_does_not_pass(self):
        self.document['evidence'] = {'status': 'passed'}
        self.assertTrue(self.check())

    def test_commit_identity_is_checked(self):
        lock = self.read(self.bundle['lock'])
        self.rewrite(lock['commits']['plenora-IO-tools'], b'not a commit')
        self.assertIn('revisione non corrispondente all oggetto Git', self.check())

    def test_component_removed_from_lock(self):
        lock = self.read(self.bundle['lock']); lock['components'].pop()
        self.rewrite(self.bundle['lock'], lock)
        self.assertTrue(self.check())

    def test_binary_digest_and_revision_binding(self):
        lock = self.read(self.bundle['lock']); lock['executables'][0]['revision'] = 'a' * 40
        self.rewrite(self.bundle['lock'], lock)
        self.assertTrue(self.check())

    def test_environment_is_native_and_bound_to_run(self):
        name = self.bundle['environment']; baseline = self.read(name)
        for field, value in [('os', 'Windows'), ('wsl', True), ('container', True), ('run_id', 'wrong-run')]:
            changed = copy.deepcopy(baseline); changed['runs'][0][field] = value
            self.rewrite(name, changed)
            with self.subTest(field=field):
                self.assertTrue(self.check())

    def test_command_cannot_be_reused_for_another_cell(self):
        name = self.bundle['report']; report = self.read(name)
        report['results'][1]['hops'][0]['command'] = report['results'][0]['hops'][0]['command']
        self.rewrite(name, report)
        self.assertTrue(self.check())

    def test_fixture_variants_are_verified_in_arrow(self):
        name = self.bundle['fixtures']; fixtures = self.read(name)
        fixtures['items'][0]['artifact'] = fixtures['items'][1]['artifact']
        self.rewrite(name, fixtures)
        self.assertTrue(self.check())

    def test_observed_property_tampering_even_with_updated_hash(self):
        name = self.read(self.bundle['report'])['results'][0]['hops'][0]['output']
        table = pa.ipc.open_file(pa.BufferReader((self.root / name).read_bytes())).read_all()
        metadata = dict(table.schema.field(0).metadata); metadata[b'plenora.field_id'] = b'42'
        schema = pa.schema([table.schema.field(0).with_metadata(metadata), table.schema.field(1)], metadata=table.schema.metadata)
        self.rewrite(name, self.arrow_bytes(pa.Table.from_arrays(table.columns, schema=schema)))
        self.assertTrue(self.check())

    def test_values_are_compared_not_only_metadata(self):
        name = self.read(self.bundle['report'])['results'][0]['hops'][0]['output']
        table = pa.ipc.open_file(pa.BufferReader((self.root / name).read_bytes())).read_all()
        self.rewrite(name, self.arrow_bytes(table.set_column(1, table.schema.field(1), pa.array([2**53, 2**53]))))
        self.assertIn('valori o schema di output diversi dall attesa', self.check())

    def test_every_property_requires_a_check(self):
        name = self.bundle['report']; report = self.read(name)
        report['results'][0]['hops'][0]['properties'].pop('srid')
        self.rewrite(name, report)
        self.assertTrue(self.check())

    def test_rejection_axes_and_effects(self):
        name = self.bundle['report']; baseline = self.read(name)
        hop = next(c for c in baseline['results'] if c['fixture'] == 'conflicting_crs')['hops'][0]
        error_name = hop['error']; original = self.read(error_name)
        for field in ('category', 'phase', 'code', 'retry', 'remote_effect'):
            error = copy.deepcopy(original); del error['error'][field]
            self.rewrite(error_name, error)
            with self.subTest(field=field):
                self.assertTrue(self.check())
        self.rewrite(error_name, original)
        self.assertEqual(self.check(), [])
        state = self.read(hop['after']); state['entries'] = ['permanent effect']
        self.rewrite(hop['after'], state)
        self.assertIn('assenza di effetto non provata', self.check())

    def test_preservation_after_rejection_is_not_green(self):
        name = self.bundle['report']; report = self.read(name)
        hop = next(c for c in report['results'] if c['fixture'] == 'conflicting_crs')['hops'][0]
        hop['preservation']['status'] = 'passed'
        self.rewrite(name, report)
        self.assertTrue(self.check())

    def test_malformed_json_duplicate_keys_and_path_escape(self):
        for pointer in ({'path': '../outside', 'sha256': 'a' * 64}, None, {'path': self.index_path, 'sha256': 'a' * 64}):
            self.document['evidence']['bundle'] = pointer
            with self.subTest(pointer=pointer):
                self.assertTrue(self.check())
        self.seal()
        self.rewrite(self.bundle['environment'], b'{"runs":[],"runs":[]}')
        self.assertTrue(self.check())

    def test_state_derivation_uses_invariant_identity(self):
        from scripts import check_release_contract as gate
        registry = json.loads((ROOT / 'assurance/registries/release-contract-current.json').read_text(encoding='utf-8'))
        voice = copy.deepcopy(next(v for v in registry['invarianti'] if v['id'] == 'sistema.qualifica-cross-component'))
        voice['prova']['artefatto'] = 'renamed.json'
        (self.root / 'renamed.json').write_text(json.dumps(self.document), encoding='utf-8')
        with mock.patch.object(gate, 'ROOT', self.root):
            self.assertEqual(gate.stato_prova_esterna(voice, 'renamed.json'), ('passed', []))
            self.assertTrue(gate._differita_ben_dichiarata(voice))
            voice['stato'] = 'verified'; voice['prova']['stato'] = 'passed'
            self.assertEqual(gate.condizione_qualifica_cross_component({'invarianti': [voice]}), [])
            (self.root / 'renamed.json').write_text('{"status":"satisfied","evidence":{"status":"passed"}}', encoding='utf-8')
            self.assertTrue(gate._prove_esterne({'invarianti': [voice]}))


if __name__ == '__main__':
    unittest.main()
