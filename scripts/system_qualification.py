"""Offline verification of the owner bundle; integrity is not owner authentication."""
from __future__ import annotations

import hashlib
import itertools
import re
import struct
from pathlib import Path

try:
    from . import strict_json
except ImportError:
    import strict_json

OWNER = 'plenora-contracts/conformance'
COMPONENTS = {
    'plenora-IO-tools': 'file_boundary',
    'plenora-data-tools': 'transformation_core',
    'plenora-database-tools': 'database_boundary',
}
AXES = ('direction', 'platform', 'surface', 'fixture', 'variant')
DOMAINS = {
    'required_directions': ['io_to_data_to_database', 'database_to_data_to_io'],
    'required_platforms': ['linux-native', 'windows-native'],
    'required_surfaces': ['rust', 'cli', 'python-sync', 'python-async', 'runtime'],
    'required_fixtures': ['axis_lat_lon', 'conflicting_crs', 'crs_missing', 'crs_unresolved',
                         'dimensions_unknown', 'geography_semantics', 'int64_beyond_2_53',
                         'multipolygon_xyzm_srid', 'point_z', 'point_zm', 'polygon_xy_projected',
                         'types_mixed', 'uint64_ordering', 'unknown_key'],
    'fixture_variants': ['canonical_only', 'geoarrow_extension'],
}
PASS_CONDITIONS = [
    'The adopted normative contract takes precedence: lawful declarations and non-prohibited discrepancies are tested separately from prohibited CRS conflicts.',
    'Lawful inputs: identity preserves every property byte-for-byte; derived fields are reconstructed from the actual result; explicit lawful plans and unavoidable transformations have localized reports and independently verified expected output.',
    'GEO-005/REJ-001 prohibited conflicts: each component rejects at its first public validating boundary with category crs, four error axes, remote_effect none and matching before/after destination snapshots, on every surface.',
    'Preservation and propagation after a prohibited rejection are skipped with reason, never counted as passed.',
    'All three components, ICD and executed harness have immutable revisions backed by Git commit objects; executed artifacts are hashed and bound to platform, surface, command and run.',
    'All required fixtures, variants, directions, native platforms, surfaces and properties have complete evidence; not_run, partial, failed and unrelated artifacts cannot satisfy the gate.',
    'The owner bundle binds manifest, lock, actual Arrow input/output/expected observations, loss reports, commands, environment, logs and absence-of-effect evidence by SHA-256.',
]
OPERATIONS = ["identity", "derived", "explicit_plan"]

BUNDLE_VERSION = 2
KINDS = {'lock', 'manifest', 'report', 'commands', 'environment', 'fixtures',
         'output', 'fixture', 'expected', 'log', 'binary', 'git_commit', 'loss', 'snapshot'}
FIXTURE_CLASSES = {name: ('prohibited_crs' if name == 'conflicting_crs' else
                        'lawful_discrepancy' if name == 'crs_unresolved' else 'lawful')
                   for name in DOMAINS['required_fixtures']}
RULES = {
    'prohibited_crs': 'GEO-005/REJ-001: reject crs at the first validating boundary; all surfaces, four axes, no effect; preservation skipped with reason.',
    'lawful_discrepancy': 'Only discrepancies not forbidden by the adopted contract: preserve and propagate unless an explicit lawful plan changes them.',
    'lawful': 'Identity preserves properties byte-for-byte; derived and declared transformations require an independent expected result and localized reports.',
}


class Invalid(ValueError):
    """A curated validation failure; never echo artifact content."""


def require(condition, message):
    if not condition:
        raise Invalid(message)


def object_(value):
    require(isinstance(value, dict), 'oggetto del bundle non valido')
    return value


def list_(value):
    require(isinstance(value, list), 'elenco del bundle non valido')
    return value


def sha(value, size=64):
    return isinstance(value, str) and re.fullmatch(f'[0-9a-f]{{{size}}}', value) is not None


def path_in(root, name):
    require(isinstance(name, str) and name.startswith('assurance/evidence/') and
            '\\' not in name and ':' not in name and
            all(p not in ('', '.', '..') for p in name.split('/')), 'percorso evidenza non canonico')
    target = (root / name).resolve()
    require(target.is_relative_to(root.resolve()) and target.is_file(), 'artefatto assente o esterno al repository')
    return target


def load_json(data):
    return object_(strict_json.loads(data.decode('utf-8')))


def property_keys(root):
    source = (root / 'crates/plenora-io-core/src/property_plan.rs').read_text(encoding='utf-8')
    table = source.split('pub const PROPERTIES:', 1)[1].split('];', 1)[0]
    pairs = re.findall(r'\(\s*"([a-z_]+)",\s*"([^"]+)"', table)
    require(bool(pairs) and len(dict(pairs)) == len(pairs), 'tabella proprieta non valida')
    return dict(pairs)


def required_properties(root):
    return list(property_keys(root))


def policy(root):
    return {**DOMAINS, 'required_operations': OPERATIONS, 'pass_conditions': PASS_CONDITIONS, 'required_properties': required_properties(root),
            'fixture_classes': FIXTURE_CLASSES, 'input_rules': RULES,
            'bundle_contract': {'version': BUNDLE_VERSION,
                'validator': 'scripts/system_qualification.py',
                'integrity': 'SHA-256 of every referenced artifact; Git commit object identities; complete per-cell execution and property comparisons.',
                'trust_boundary': 'Offline consistency and byte comparisons do not authenticate the owner, prove execution, or establish membership of a Git object in a remote repository.'}}


def manifest_errors(document, root):
    try:
        object_(document)
        require(document.get('manifest_version') == 1 and
                document.get('gate_id') == 'plenora-system-contract-roundtrip-v1', 'identita gate non valida')
        require(document.get('ownership') == 'external_system_qualification' and
                document.get('external_owner') == OWNER, 'owner della qualifica non valido')
        require(all(document.get(k) == v for k, v in policy(root).items()), 'requisiti del gate diversi dalla fonte unica')
        adoption = load_json((root / 'contracts/adoption-source.json').read_bytes())
        require(document.get('icd') == {'revision': adoption['contracts_source']['revision'],
                                      'source': 'contracts/adoption-source.json'}, 'ICD diverso dal contratto adottato')
        components = list_(document.get('components'))
        require(len(components) == len(COMPONENTS) and
                {object_(c).get('name') for c in components} == set(COMPONENTS), 'tre componenti obbligatori')
        require(all(c.get('role') == COMPONENTS[c['name']] and
                    (c.get('revision') is None or sha(c['revision'], 40)) for c in components), 'ruoli o revisioni componenti non validi')
        require(document.get('status') in ('not_satisfied', 'satisfied'), 'stato gate non valido')
        list_(document.get('open_blockers'))
        require(object_(document.get('evidence')).get('status') in ('not_run', 'failed', 'partial', 'passed'), 'stato evidenza non valido')
        return []
    except (Invalid, KeyError, TypeError, ValueError, OSError, UnicodeError):
        return ['manifest della qualifica non conforme alla fonte unica']


class Bundle:
    def __init__(self, evidence, root):
        pointer = object_(evidence.get('bundle'))
        raw = path_in(root, pointer.get('path')).read_bytes()
        require(sha(pointer.get('sha256')) and hashlib.sha256(raw).hexdigest() == pointer['sha256'], 'digest del bundle non corrispondente')
        self.document = load_json(raw)
        require(self.document.get('schema_version') == BUNDLE_VERSION, 'versione bundle non valida')
        self.artifacts = {}
        for item in list_(self.document.get('artifacts')):
            item = object_(item)
            name = item.get('path')
            data = path_in(root, name).read_bytes()
            require(name != pointer['path'] and name not in self.artifacts and item.get('kind') in KINDS,
                    'artefatto duplicato o tipo non valido')
            require(data and sha(item.get('sha256')) and hashlib.sha256(data).hexdigest() == item['sha256'], 'digest artefatto non corrispondente')
            self.artifacts[name] = (item, data)
        self.used = set()
        self.arrow_cache = {}
        self.keys = property_keys(root)

    def ref(self, name, kind):
        require(isinstance(name, str) and name in self.artifacts, 'riferimento artefatto assente')
        item, data = self.artifacts[name]
        require(item['kind'] in ((kind,) if isinstance(kind, str) else kind), 'tipo riferimento artefatto errato')
        self.used.add(name)
        return data

    def json(self, name, kind):
        return load_json(self.ref(name, kind))

    def commit(self, name, revision):
        raw = self.ref(name, 'git_commit')
        require(sha(revision, 40) and hashlib.sha1(b'commit ' + str(len(raw)).encode() + b'\0' + raw).hexdigest() == revision,
                'revisione non corrispondente all oggetto Git')
        require(re.match(rb'tree [0-9a-f]{40}\n', raw) and b'\nauthor ' in raw and b'\ncommitter ' in raw and b'\n\n' in raw,
                'oggetto Git commit malformato')

    def arrow(self, name, kind):
        raw = self.ref(name, kind)
        if name not in self.arrow_cache:
            try:
                import pyarrow as pa
                reader = pa.ipc.open_file(pa.BufferReader(raw))
                table = reader.read_all()
                require(table.num_rows > 0, 'osservazione Arrow vuota')
                result = {key: [] for key in self.keys}
                canonical = {v.encode() for k, v in self.keys.items() if k != 'native_metadata'}
                scopes = [(None, table.schema.metadata or {})] + [
                    (i, field.metadata or {}) for i, field in enumerate(table.schema)]
                for scope, metadata in scopes:
                    for key, value in sorted(metadata.items()):
                        prop = next((p for p, pattern in self.keys.items() if
                                     key == pattern.encode() or pattern.endswith('.') and key.startswith(pattern.encode())), None)
                        if prop is None and key not in canonical and key != b'ARROW:extension:name':
                            prop = 'native_metadata'
                        if prop:
                            result[prop].append([scope, key.hex(), value.hex()])
                self.arrow_cache[name] = (result, table)
            except ImportError as exc:
                raise Invalid('PyArrow necessario per verificare le osservazioni reali') from exc
            except Exception as exc:
                raise Invalid('osservazione Arrow non leggibile') from exc
        return self.arrow_cache[name]


def indexed(records, fields):
    result = {}
    for record in list_(records):
        record = object_(record)
        key = tuple(record.get(f) for f in fields)
        require(all(isinstance(v, str) and v for v in key) and key not in result, 'identita duplicata o non valida')
        result[key] = record
    return result


def validate_lock(bundle, gate):
    lock = bundle.json(bundle.document.get('lock'), 'lock')
    require(lock.get('owner') == OWNER and lock.get('components') == gate['components'] and
            lock.get('icd_revision') == gate['icd']['revision'], 'lock diverso dal gate')
    revisions = {c['name']: c['revision'] for c in gate['components']}
    revisions['plenora-contracts'] = gate['icd']['revision']
    harness = object_(lock.get('harness'))
    require(sha(harness.get('revision'), 40), 'revisione harness non fissata')
    revisions['harness'] = harness['revision']
    commits = object_(lock.get('commits'))
    require(set(commits) == set(revisions), 'oggetti Git mancanti')
    for component, revision in revisions.items():
        bundle.commit(commits[component], revision)
    executables = indexed(lock.get('executables'), ('component', 'platform', 'surface'))
    wanted = set(itertools.product(COMPONENTS, DOMAINS['required_platforms'], DOMAINS['required_surfaces']))
    require(set(executables) == wanted, 'lock eseguibili incompleto')
    for (component, _, _), executable in executables.items():
        require(executable.get('revision') == revisions[component], 'eseguibile legato a revisione diversa')
        bundle.ref(executable.get('artifact'), 'binary')
    return executables, harness


def validate_environments(bundle, harness):
    environments = indexed(bundle.json(bundle.document.get('environment'), 'environment').get('runs'), ('platform',))
    require(set(environments) == {(p,) for p in DOMAINS['required_platforms']}, 'piattaforme native incomplete')
    run_ids = set()
    for (platform,), env in environments.items():
        require(env.get('os') == {'linux-native': 'Linux', 'windows-native': 'Windows'}[platform] and
                env.get('execution') == 'native' and env.get('wsl') is False and env.get('container') is False,
                'ambiente non nativo o piattaforma discordante')
        require(isinstance(env.get('run_id'), str) and env['run_id'] and env['run_id'] not in run_ids and
                env.get('harness_revision') == harness['revision'] and isinstance(env.get('architecture'), str) and env['architecture'],
                'provenienza ambiente incompleta')
        run_ids.add(env['run_id'])
        bundle.ref(env.get('log'), 'log')
    return environments


def validate_fixtures(bundle):
    fixtures = indexed(bundle.json(bundle.document.get('fixtures'), 'fixtures').get('items'), ('fixture', 'variant'))
    require(set(fixtures) == set(itertools.product(DOMAINS['required_fixtures'], DOMAINS['fixture_variants'])), 'fixture o varianti mancanti')
    covered = set()
    for (fixture, variant), item in fixtures.items():
        require(item.get('input_class') == FIXTURE_CLASSES[fixture], 'classe fixture non conforme al contratto')
        props, table = bundle.arrow(item.get('artifact'), 'fixture')
        geometry = [f for f in table.schema if b'plenora.geometry.encoding' in (f.metadata or {})]
        require(bool(geometry), 'fixture priva di geometria dichiarata')
        extension = [f.metadata.get(b'ARROW:extension:name') for f in geometry]
        require(all(value == (b'geoarrow.wkb' if variant == 'geoarrow_extension' else None) for value in extension), 'variante Arrow non corrispondente ai byte')
        check_fixture(fixture, props, table)
        if fixture != 'conflicting_crs':
            covered.update(p for p, values in props.items() if values)
    require(covered == set(bundle.keys), 'proprieta mai esercitate dalle fixture')
    return fixtures


def geometry_observation(raw):
    """Bounded independent WKB/EWKB fixture oracle (types 1..7)."""
    require(isinstance(raw, bytes), 'geometria fixture non binaria')
    offset = 0

    def read(fmt, endian):
        nonlocal offset
        size = struct.calcsize(endian + fmt)
        require(offset + size <= len(raw), 'geometria fixture troncata')
        value = struct.unpack_from(endian + fmt, raw, offset)
        offset += size
        return value[0] if len(value) == 1 else value

    def geometry(depth=0):
        require(depth <= 32, 'geometria fixture troppo profonda')
        order = read('B', '<')
        require(order in (0, 1), 'endian fixture non valido')
        endian = '<' if order else '>'
        code = read('I', endian)
        base = code & 0x1fffffff
        dimension, kind = divmod(base, 1000)
        require(dimension in range(4) and kind in range(1, 8), 'tipo fixture non valido')
        z, m = bool(code & 0x80000000) or dimension in (1, 3), bool(code & 0x40000000) or dimension in (2, 3)
        dims = 'xy' + ('z' if z else '') + ('m' if m else '')
        srid = read('I', endian) if code & 0x20000000 else None

        def coordinates(count):
            require(0 < count <= (len(raw) - offset) // (8 * len(dims)), 'coordinate fixture vuote o fuori limite')
            for _ in range(count):
                read('d' * len(dims), endian)

        if kind == 1:
            coordinates(1)
        elif kind == 2:
            coordinates(read('I', endian))
        elif kind == 3:
            count = read('I', endian)
            require(0 < count <= len(raw) // 4, 'anelli fixture non validi')
            for _ in range(count):
                coordinates(read('I', endian))
        else:
            count = read('I', endian)
            require(0 < count <= len(raw) // 5, 'componenti fixture non valide')
            for _ in range(count):
                child = geometry(depth + 1)
                require(kind == 7 or (child[0] == kind - 3 and child[1] == dims), 'componenti fixture discordanti')
        return kind, dims, srid

    result = geometry()
    require(offset == len(raw), 'byte geometrici residui nella fixture')
    return result


def geometry_features(name, table):
    conflicts = []
    matching = []
    for index, field in enumerate(table.schema):
        metadata = field.metadata or {}
        if b'plenora.geometry.encoding' not in metadata:
            continue
        observations = [geometry_observation(raw) for raw in table.column(index).to_pylist() if raw is not None]
        require(bool(observations), 'fixture senza geometrie osservabili')
        crsid = metadata.get(b'plenora.geometry.crs_id', b'').decode('utf-8')
        definition = metadata.get(b'plenora.geometry.crs_definition')
        conflict = False
        if definition is not None and crsid:
            # The qualification fixture protocol uses PROJJSON so the oracle
            # can inspect root IDs without relying on the component parser.
            require(metadata.get(b'plenora.geometry.crs_definition_format') == b'projjson', 'fixture CRS richiede PROJJSON per l oracolo indipendente')
            root = load_json(definition)
            ids = root.get('ids', [root['id']] if 'id' in root else [])
            authority, code = crsid.split(':', 1)
            same = [str(object_(item).get('code')) for item in list_(ids)
                    if str(object_(item).get('authority')).translate(str.maketrans('abcdefghijklmnopqrstuvwxyz', 'ABCDEFGHIJKLMNOPQRSTUVWXYZ')) == authority.translate(str.maketrans('abcdefghijklmnopqrstuvwxyz', 'ABCDEFGHIJKLMNOPQRSTUVWXYZ'))]
            conflict = bool(same) and code not in same
        conflicts.append(conflict)
        wanted = {'point_z': (1, 'xyz'), 'point_zm': (1, 'xyzm'),
                  'multipolygon_xyzm_srid': (6, 'xyzm'), 'polygon_xy_projected': (3, 'xy')}.get(name)
        matches = wanted is None or all(o[:2] == wanted for o in observations)
        if wanted:
            type_name = {1: b'point', 3: b'polygon', 6: b'multipolygon'}[wanted[0]]
            matches = matches and metadata.get(b'plenora.geometry.dimensions') == wanted[1].encode() and metadata.get(b'plenora.geometry.types') == type_name
        if name == 'polygon_xy_projected':
            matches = matches and metadata.get(b'plenora.geometry.axis_order') == b'easting_northing'
        if name == 'multipolygon_xyzm_srid':
            matches = matches and all(str(o[2]).encode() == metadata.get(b'plenora.geometry.srid') for o in observations)
        if name == 'types_mixed':
            matches = len({o[0] for o in observations}) >= 2
        matching.append(matches)
    require(any(matching), 'payload geometrico non esercita la fixture dichiarata')
    require(any(conflicts) if name == 'conflicting_crs' else not any(conflicts), 'classe CRS non corrispondente ai byte della fixture')


def check_fixture(name, properties, table):
    """Require the named coverage features in actual input observations."""
    geometry_features(name, table)
    def values(prop):
        return {bytes.fromhex(v).decode('utf-8') for _, _, v in properties[prop]}
    requirements = {
        'axis_lat_lon': {'axis_order': 'lat_lon'},
        'crs_missing': {'crs_resolution': 'missing'},
        'crs_unresolved': {'crs_resolution': 'unresolved'},
        'dimensions_unknown': {'dimensions': 'unknown'},
        'geography_semantics': {'geometry_or_geography': 'geography'},
        'point_z': {'dimensions': 'xyz', 'geometry_types': 'point'},
        'point_zm': {'dimensions': 'xyzm', 'geometry_types': 'point'},
        'multipolygon_xyzm_srid': {'dimensions': 'xyzm', 'geometry_types': 'multipolygon'},
        'polygon_xy_projected': {'dimensions': 'xy', 'geometry_types': 'polygon', 'axis_order': 'easting_northing'},
    }
    require(all(expected in values(prop) for prop, expected in requirements.get(name, {}).items()), 'fixture nominale priva della proprieta richiesta')
    if name == 'crs_missing':
        require(not properties['crs_id'] and not properties['crs_definition'], 'fixture CRS missing contraddittoria')
    if name == 'multipolygon_xyzm_srid':
        require(bool(properties['srid']), 'fixture dimensionale senza SRID')
    if name == 'types_mixed':
        require('mixed' in values('types_declaration') or any(',' in v for v in values('geometry_types')), 'fixture senza tipi misti')
    if name == 'unknown_key':
        scopes = {scope for scope, _, _ in properties['native_metadata']}
        require(None in scopes and any(scope is not None for scope in scopes), 'metadati opachi schema/campo non esercitati')
    if name in ('int64_beyond_2_53', 'uint64_ordering'):
        import pyarrow as pa
        columns = [column.to_pylist() for column in table.columns if
                   (pa.types.is_int64(column.type) if name == 'int64_beyond_2_53' else pa.types.is_uint64(column.type))]
        require(any(any(v is not None and abs(v) > 2**53 for v in column) for column in columns), 'fixture senza interi oltre 2^53')
        if name == 'uint64_ordering':
            require(any(len(column) >= 2 and all(v is not None for v in column) and column == sorted(column) and
                        len(set(column)) > 1 and max(column) > 2**63-1 for column in columns), 'ordinamento uint64 non esercitato')


def property_locations(prop, properties, table):
    locations = []
    for scope, key, _ in properties[prop]:
        metadata = table.schema.metadata if scope is None else table.schema.field(scope).metadata
        identity = None if scope is None else metadata.get(b'plenora.field_id')
        locations.append({'field_index': scope, 'field_id': int(identity) if identity is not None else None,
                          'key_index': sorted(metadata).index(bytes.fromhex(key))})
    return locations


def canonical_arrow(table):
    """Canonical Arrow observation, not arbitrary IPC container bytes."""
    import pyarrow as pa
    schema = pa.schema([field.with_metadata(dict(sorted((field.metadata or {}).items())))
                        for field in table.schema], metadata=dict(sorted((table.schema.metadata or {}).items())))
    normalized = pa.Table.from_arrays([pa.array(column.to_pylist(), type=column.type)
                                       for column in table.columns], schema=schema)
    sink = pa.BufferOutputStream()
    with pa.ipc.new_stream(sink, schema) as writer:
        writer.write_table(normalized)
    return sink.getvalue().to_pybytes()


def compare_hop(bundle, hop, before, after):
    source, source_table = bundle.arrow(before, ('fixture', 'output'))
    actual, actual_table = bundle.arrow(after, 'output')
    expected, expected_table = bundle.arrow(hop.get('expected'), 'expected')
    require(canonical_arrow(actual_table) == canonical_arrow(expected_table), 'valori o schema di output diversi dall attesa')
    checks = object_(hop.get('properties'))
    require(set(checks) == set(bundle.keys), 'copertura proprieta incompleta')
    require(actual == expected, 'output diverso dall attesa indipendente')
    for prop, mode in checks.items():
        require(mode in ('preserved', 'derived', 'declared_transformation'), 'esito proprieta non valido')
        if mode == 'preserved':
            require(source[prop] == actual[prop], 'proprieta dichiarata conservata ma diversa')
        else:
            require(hop.get('operation') in ('derived', 'explicit_plan', 'format_conversion'), 'trasformazione senza operazione dichiarata')
            audit = bundle.json(hop.get('loss_report'), 'loss')
            require(audit.get('command') == hop.get('command'), 'rapporto perdita appartenente ad altro comando')
            entries = list_(audit.get('property_changes'))
            locations = property_locations(prop, source, source_table) or property_locations(prop, actual, actual_table)
            require(bool(locations) and any(isinstance(e, dict) and e.get('property') == prop and
                        e.get('locations') == locations and isinstance(e.get('rule'), str) and e['rule']
                        for e in entries), 'trasformazione senza rapporto localizzato')
            require(object_(hop.get('authorization')).get('model') == 'component_contract_and_options', 'modello consenso non dichiarato')
    require(hop.get('operation') in ('identity', 'derived', 'explicit_plan', 'format_conversion'), 'operazione non dichiarata')
    if hop['operation'] == 'derived':
        require('derived' in checks.values(), 'ricostruzione non provata')
    if hop['operation'] == 'explicit_plan':
        require('declared_transformation' in checks.values() and source != actual, 'piano esplicito senza trasformazione osservabile')
    if hop['operation'] == 'identity':
        require(all(m == 'preserved' for m in checks.values()) and canonical_arrow(source_table) == canonical_arrow(actual_table), 'operazione identitaria con perdita')
    if all(m == 'preserved' for m in checks.values()):
        require(canonical_arrow(source_table) == canonical_arrow(actual_table), 'valori cambiati senza trasformazione dichiarata')


def rejection(bundle, result, command):
    require(result.get('preservation') == {'status': 'skipped', 'reason': 'rejected_at_validating_boundary'} and
            result.get('properties') == {}, 'conservazione dopo rifiuto non saltata esplicitamente')
    observation = bundle.json(result.get('error'), 'report')
    require(observation.get('command') == command['id'], 'errore appartenente ad altro comando')
    observed = object_(observation.get('error'))
    require(observed.get('category') == 'crs' and observed.get('phase') in ('validate', 'prepare', 'read', 'write') and
            isinstance(observed.get('code'), str) and re.fullmatch('[A-Z][A-Z0-9_]{1,63}', observed['code']) and
            observed.get('retry') == {'kind': 'never'} and observed.get('remote_effect') == 'none', 'rifiuto CRS incompleto')
    require(result.get('boundary') == 'first_validating_boundary', 'rifiuto non al primo confine validante')
    before = bundle.json(result.get('before'), 'snapshot')
    after = bundle.json(result.get('after'), 'snapshot')
    require(before == after and before.get('capture') == 'complete_destination_state' and
            before.get('command') == command['id'] and before.get('destination') == command.get('destination') and
            isinstance(before.get('destination'), str) and before['destination'] and
            isinstance(before.get('entries'), list), 'assenza di effetto non provata')


def validate_results(bundle, executables, environments, fixtures):
    manifest = bundle.json(bundle.document.get('manifest'), 'manifest')
    report = bundle.json(bundle.document.get('report'), 'report')
    require(manifest.get('owner') == OWNER and report.get('owner') == OWNER and
            manifest.get('lock') == bundle.document['lock'] and report.get('lock') == bundle.document['lock'], 'provenienza manifest o report diversa')
    results = indexed(report.get('results'), AXES)
    wanted = set(itertools.product(*DOMAINS.values()))
    require(set(results) == wanted and manifest.get('cells') == [list(k) for k in sorted(wanted)], 'matrice incompleta o diversa dal manifest')
    commands = indexed(bundle.json(bundle.document.get('commands'), 'commands').get('executions'), ('id',))
    consumed = set()
    operations = {k: set() for k in itertools.product(DOMAINS['required_directions'], DOMAINS['required_platforms'], DOMAINS['required_surfaces'])}
    for key, cell in results.items():
        direction, platform, surface, fixture, variant = key
        require(cell.get('status') == 'passed', 'cella non superata')
        hops = list_(cell.get('hops'))
        order = list(COMPONENTS) if direction == 'io_to_data_to_database' else list(reversed(COMPONENTS))
        require([object_(h).get('component') for h in hops] == order, 'catena a tre componenti incompleta o fuori ordine')
        current = fixtures[(fixture, variant)]['artifact']
        for hop in hops:
            command_key = (hop.get('command'),)
            require(command_key in commands and command_key not in consumed, 'comando mancante o riutilizzato')
            consumed.add(command_key)
            command = commands[command_key]
            require(command.get('cell') == list(key) and command.get('component') == hop['component'] and
                    command.get('run_id') == environments[(platform,)]['run_id'], 'comando appartenente ad altra cella o ambiente')
            executable = executables[(hop['component'], platform, surface)]
            require(command.get('executable') == executable['artifact'] and command.get('revision') == executable['revision'], 'comando non legato all eseguibile qualificato')
            require(isinstance(command.get('argv'), list) and command['argv'] and all(isinstance(a, str) and a for a in command['argv']), 'comando non riproducibile')
            require(command.get('input') == current and command.get('output') == hop.get('output'), 'input/output comando discordanti')
            logged = bundle.json(command.get('log'), 'log')
            require(logged == {k: v for k, v in command.items() if k != 'log'}, 'log diverso dal comando dichiarato')
            require(type(command.get('exit_code')) is int, 'exit code non intero')
            if fixture == 'conflicting_crs':
                # Each component is independently probed at its public boundary.
                require(command['exit_code'] != 0 and hop.get('output') is None, 'conflitto CRS accettato o output pubblicato')
                rejection(bundle, hop, command)
            else:
                require(command['exit_code'] == 0, 'comando positivo fallito')
                compare_hop(bundle, hop, current, hop.get('output'))
                current = hop['output']
                operations[key[:3]].add(hop['operation'])
    require(consumed == set(commands), 'comandi estranei al bundle')
    require(all(set(OPERATIONS) <= seen for seen in operations.values()), 'operazioni derivate o piano lecito non esercitati su tutte le superfici')


def validate(document, root):
    errors = manifest_errors(document, root)
    if errors:
        return errors
    try:
        require(document['status'] == 'satisfied' and document['open_blockers'] == [] and
                document['evidence']['status'] == 'passed', 'gate non soddisfatto, evidence.status non passed o blocchi non chiusi')
        require(all(sha(c['revision'], 40) for c in document['components']), 'revisioni componenti non fissate')
        bundle = Bundle(document['evidence'], root)
        require(bundle.document.get('owner') == OWNER, 'owner bundle diverso dal gate')
        executables, harness = validate_lock(bundle, document)
        environments = validate_environments(bundle, harness)
        fixtures = validate_fixtures(bundle)
        validate_results(bundle, executables, environments, fixtures)
        require(bundle.used == set(bundle.artifacts), 'artefatti non collegati alla corsa')
        return []
    except Invalid as exc:
        return [str(exc)]
    except (KeyError, TypeError, ValueError, OSError, UnicodeError, IndexError):
        return ['bundle illeggibile o struttura non conforme']


if __name__ == '__main__':
    import argparse
    import json
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    path = root / 'release/system-rc-gate.json'
    document = load_json(path.read_bytes())
    if args.write:
        document.update(policy(root))
        path.write_text(json.dumps(document, indent=2, ensure_ascii=False) + '\n', encoding='utf-8', newline='\n')
    else:
        raise SystemExit(int(bool(manifest_errors(document, root))))
