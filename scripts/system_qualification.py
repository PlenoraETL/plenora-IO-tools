"""Offline verification of the owner bundle; integrity is not owner authentication."""
from __future__ import annotations

import hashlib
import itertools
import re
from pathlib import Path

try:
    from . import strict_json, qualification_contract
except ImportError:
    import strict_json
    import qualification_contract

OWNER = 'plenora-contracts/conformance'
COMPONENTS = {
    'plenora-IO-tools': 'file_boundary',
    'plenora-data-tools': 'transformation_core',
    'plenora-database-tools': 'database_boundary',
}
AXES = ('direction', 'platform', 'surface', 'fixture', 'variant')
# Campaign coverage, not extra normative input restrictions. Vocabulary and
# decoding are exclusively owned by the adopted contract's oracle.
FIXTURES = {
    'axis_lat_lon': {'metadata': {'axis_order': 'lat_lon'}},
    'conflicting_crs': {'rejection': ['crs', 'GEO-005']},
    'crs_missing': {'metadata': {'crs_resolution': 'missing'}},
    'crs_unresolved': {'metadata': {'crs_resolution': 'declared_unresolved'}},
    'dimensions_unknown': {'metadata': {'dimensions': 'unknown'}},
    'geography_semantics': {'metadata': {'geometry_or_geography': 'geography'}},
    'int64_beyond_2_53': {'numeric': 'int64'},
    'multipolygon_xyzm_srid': {'geometry': ['multipolygon', 'xyzm'], 'srid': True},
    'point_z': {'geometry': ['point', 'xyz']},
    'point_zm': {'geometry': ['point', 'xyzm']},
    'polygon_xy_projected': {'geometry': ['polygon', 'xy'], 'metadata': {'axis_order': 'easting_northing'}},
    'types_mixed': {'mixed': True},
    'uint64_ordering': {'numeric': 'uint64', 'unordered': True},
    'unknown_key': {'opaque_schema_and_field': True},
}
VARIANTS = {
    'canonical_only': {'extension_metadata': 'absent'},
    'geoarrow_extension': {'extension_metadata': 'json_object_on_geometry'},
}
DOMAINS = {
    'required_directions': ['io_to_data_to_database', 'database_to_data_to_io'],
    'required_platforms': ['linux-native', 'windows-native'],
    'required_surfaces': ['rust', 'cli', 'python-sync', 'python-async', 'runtime'],
    'required_fixtures': list(FIXTURES),
    'fixture_variants': list(VARIANTS),
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
OPERATION_SEMANTICS = ['pass_through', 'tabular_plan', 'metadata_normalization', 'geometry_compute']

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
    normative = qualification_contract.contract(root)
    return {**DOMAINS, 'required_operations': OPERATIONS, 'pass_conditions': PASS_CONDITIONS, 'required_properties': required_properties(root),
            'fixture_requirements': FIXTURES, 'variant_rules': VARIANTS,
            'operation_semantics': OPERATION_SEMANTICS, 'required_geometry_computation': True,
            'normative_source': {'source': 'contracts/adoption-source.json', 'loader': 'scripts/qualification_contract.py',
                                 'repository': 'PLENORA_CONTRACTS or .plenora-contracts', 'mode': 'offline Git objects at the adopted immutable revision'},
            'normative_vocabulary': {'closed_values': {k: sorted(v) for k, v in normative.data.CLOSED_VALUES.items()},
                                     'canonical_types': normative.data.CANONICAL_TYPES,
                                     'required_geometry': list(normative.data.REQUIRED_GEOMETRY),
                                     'schema_rejection_phases': list(normative.schema_rejection_phases),
                                     'crs_cli_exit': normative.exit_codes['crs']},
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
        self.contract = qualification_contract.contract(root)

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
                try:
                    reader = pa.ipc.open_file(pa.BufferReader(raw))
                except pa.ArrowInvalid:
                    reader = pa.ipc.open_stream(pa.BufferReader(raw))
                table = reader.read_all()
                result = property_observation(self.contract.canonical_primary(table), self.keys)
                self.arrow_cache[name] = (result, table)
            except ImportError as exc:
                raise Invalid('PyArrow necessario per verificare le osservazioni reali') from exc
            except Exception as exc:
                raise Invalid('osservazione Arrow non leggibile') from exc
        return self.arrow_cache[name]


def property_observation(table, keys):
    result = {key: [] for key in keys}
    canonical = {v.encode() for k, v in keys.items() if k != 'native_metadata'}
    scopes = [(None, table.schema.metadata or {})] + [
        (i, field.metadata or {}) for i, field in enumerate(table.schema)]
    for scope, metadata in scopes:
        for key, value in sorted(metadata.items()):
            prop = next((p for p, pattern in keys.items() if
                         key == pattern.encode() or pattern.endswith('.') and key.startswith(pattern.encode())), None)
            if prop is None and key not in canonical and key != b'ARROW:extension:name':
                prop = 'native_metadata'
            if prop:
                result[prop].append([scope, key.hex(), value.hex()])
    return result


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
        check_variant(variant, table, bundle.contract)
        check_fixture(fixture, props, table, bundle.contract, bundle.keys)
        if fixture != 'conflicting_crs':
            covered.update(p for p, values in props.items() if values)
    require(covered == set(bundle.keys), 'proprieta mai esercitate dalle fixture')
    return fixtures


def check_variant(variant, table, contract=None):
    contract = contract or qualification_contract.contract()
    rule = VARIANTS[variant]
    for field in table.schema:
        metadata = field.metadata or {}
        geometry = contract.data.is_geometry({'metadata': contract.metadata(metadata)})
        if not geometry:
            continue
        value = metadata.get(b'ARROW:extension:metadata')
        if rule['extension_metadata'] == 'absent':
            require(value is None, 'variante canonical_only con metadati di estensione')
        elif geometry:
            require(value is not None and isinstance(strict_json.loads(value.decode('utf-8')), dict),
                    'variante GeoArrow senza oggetto di metadati')


def check_fixture(name, properties, table, contract=None, keys=None):
    """First normative conformance; then only the declared campaign coverage."""
    contract = contract or qualification_contract.contract()
    keys = keys or property_keys(Path(__file__).resolve().parents[1])
    rule = FIXTURES[name]
    verdict = contract.verdict(table)
    if 'rejection' in rule:
        require(verdict is not None and list(verdict) == rule['rejection'] and
                any(contract.contradiction(f) for f in table.schema),
                'classe CRS non corrispondente ai byte della fixture')
    else:
        require(verdict is None, 'fixture non conforme al contratto adottato')
    if 'numeric' in rule:
        import pyarrow as pa
        predicate = pa.types.is_int64 if rule['numeric'] == 'int64' else pa.types.is_uint64
        columns = [[v for v in col.to_pylist() if v is not None] for col in table.columns if predicate(col.type)]
        require(any(any(abs(v) > 2**53 for v in col) for col in columns), 'fixture senza interi oltre 2^53')
        if rule.get('unordered'):
            require(any(len(set(col)) > 1 and col != sorted(col) and max(col) > 2**63 - 1
                        for col in columns), 'fixture senza uint64 non ordinati oltre int64')
        return
    matches = []
    for index, field in enumerate(table.schema):
        md = contract.metadata(field.metadata)
        if not contract.data.is_geometry({'metadata': md}):
            continue
        try:
            observations = [contract.data.decode_wkb(raw) for raw in table.column(index).to_pylist() if raw is not None]
        except contract.data.Malformed as exc:
            raise Invalid('payload geometrico non conforme al contratto') from exc
        match = bool(observations) and all(md.get(keys[prop]) == value for prop, value in rule.get('metadata', {}).items())
        if 'geometry' in rule:
            match = match and all(list(o[:2]) == rule['geometry'] for o in observations)
        if rule.get('srid'):
            match = match and 'plenora.geometry.srid' in md
        if rule.get('mixed'):
            match = match and len({o.type for o in observations}) >= 2
        matches.append(match)
    require(any(matches), 'payload geometrico non esercita la fixture dichiarata')
    if rule.get('opaque_schema_and_field'):
        scopes = {scope for scope, _, _ in properties['native_metadata']}
        require(None in scopes and any(scope is not None for scope in scopes), 'metadati opachi schema/campo non esercitati')


def property_locations(prop, properties, table):
    locations = []
    for scope, key, _ in properties[prop]:
        metadata = table.schema.metadata if scope is None else table.schema.field(scope).metadata
        identity = None if scope is None else metadata.get(b'plenora.field_id')
        locations.append({'field_index': scope, 'field_id': int(identity) if identity is not None else None,
                          'key_index': sorted(metadata).index(bytes.fromhex(key))})
    return locations


def property_identity(values, table):
    """ARROW-003/004 binds unchanged fields by identity, not ordinal position."""
    result = []
    for scope, key, value in values:
        identity = ('schema',) if scope is None else ('position', scope)
        if scope is not None:
            field_id = (table.schema.field(scope).metadata or {}).get(b'plenora.field_id')
            if field_id is not None:
                identity = ('field_id', field_id.decode('utf-8'))
        result.append((identity, key, value))
    return sorted(result, key=repr)


def canonical_arrow(table):
    """Canonical Arrow observation, not arbitrary IPC container bytes."""
    import pyarrow as pa
    table = qualification_contract.contract().canonical_primary(table)
    schema = pa.schema([field.with_metadata(dict(sorted((field.metadata or {}).items())))
                        for field in table.schema], metadata=dict(sorted((table.schema.metadata or {}).items())))
    normalized = table.combine_chunks()
    sink = pa.BufferOutputStream()
    with pa.ipc.new_stream(sink, schema) as writer:
        writer.write_table(normalized)
    return sink.getvalue().to_pybytes()


def compare_hop(bundle, hop, before, after, semantics):
    source, source_table = bundle.arrow(before, ('fixture', 'output'))
    actual, actual_table = bundle.arrow(after, 'output')
    expected, expected_table = bundle.arrow(hop.get('expected'), 'expected')
    require(bundle.contract.verdict(actual_table) is None and bundle.contract.verdict(expected_table) is None,
            'output o attesa non conformi al contratto adottato')
    for table in (actual_table, expected_table):
        require(table.schema.metadata == bundle.contract.canonical_primary(table).schema.metadata,
                'primaria di output non canonica PRIM-004')
    validate_operation(bundle.contract, object_(semantics), hop['operation'], source_table, actual_table)
    require(canonical_arrow(actual_table) == canonical_arrow(expected_table), 'valori o schema di output diversi dall attesa')
    checks = object_(hop.get('properties'))
    require(set(checks) == set(bundle.keys), 'copertura proprieta incompleta')
    require(actual == expected, 'output diverso dall attesa indipendente')
    for prop, mode in checks.items():
        require(mode in ('preserved', 'derived', 'declared_transformation'), 'esito proprieta non valido')
        if mode == 'preserved':
            require(property_identity(source[prop], source_table) == property_identity(actual[prop], actual_table),
                    'proprieta dichiarata conservata ma diversa')
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
    if hop['operation'] in ('derived', 'explicit_plan'):
        require(canonical_arrow(source_table) != canonical_arrow(actual_table), 'operazione senza risultato osservabile')
        require(object_(hop.get('authorization')).get('model') == 'component_contract_and_options', 'modello consenso non dichiarato')
        if hop.get('loss_report'):
            audit = bundle.json(hop['loss_report'], 'loss')
            require(audit.get('command') == hop.get('command') and isinstance(audit.get('property_changes'), list), 'rapporto di operazione non collegato')
    if hop['operation'] == 'identity':
        require(all(m == 'preserved' for m in checks.values()) and canonical_arrow(source_table) == canonical_arrow(actual_table), 'operazione identitaria con perdita')
    if hop['operation'] in ('identity', 'format_conversion') and all(m == 'preserved' for m in checks.values()):
        require(canonical_arrow(source_table) == canonical_arrow(actual_table), 'valori cambiati senza trasformazione dichiarata')


def validate_operation(contract, semantics, operation, source, actual):
    require(semantics.get('kind') in OPERATION_SEMANTICS and isinstance(semantics.get('operation_id'), str) and semantics['operation_id'],
            'semantica operativa o identificatore pubblico assenti')
    require(operation != 'identity' or semantics['kind'] == 'pass_through', 'identita classificata come calcolo')
    require(operation not in ('derived', 'explicit_plan') or semantics['kind'] != 'pass_through',
            'pass-through non prova una derivazione o un piano')
    before, after = contract.vector(source), contract.vector(actual)
    input_primary, output_primary = contract.primary.primary(before), contract.primary.primary(after)
    identity = lambda field: None if field is None else field['metadata']['plenora.field_id']
    actual_primary = identity(output_primary)
    if 'requested_primary' in semantics or 'replaces_primary_with' in semantics:
        requested = semantics.get('requested_primary', semantics.get('replaces_primary_with'))
        require(isinstance(requested, str) and (requested == contract.primary.NONE or any(
            f['metadata']['plenora.field_id'] == requested for f in contract.primary.geometry_fields(after))),
            'primaria richiesta o sostituita non presente nel risultato')
        require(actual_primary == (None if requested == contract.primary.NONE else requested), 'primaria diversa dalla richiesta attestata')
    elif contract.primary.geometry_fields(before):
        surviving = any(identity(f) == identity(input_primary) for f in contract.primary.geometry_fields(after))
        require(actual_primary == (identity(input_primary) if surviving else None), 'stato della primaria non conservato PRIM-005')
    if semantics['kind'] != 'geometry_compute':
        return
    require(operation in ('derived', 'explicit_plan'), 'calcolo geometrico senza operazione dichiarata')
    require('geometry_selection' in semantics and (semantics['geometry_selection'] is None or isinstance(semantics['geometry_selection'], str)),
            'selezione geometrica non attestata')
    operands = contract.primary.operands(contract.vector(source), semantics['geometry_selection'])
    require(not isinstance(operands, contract.data.Verdict), 'operando geometrico non conforme PRIM-007')
    try:
        decisions = [contract.data.computation_verdict(field) for field in operands]
    except contract.data.Undecidable as exc:
        raise Invalid('calcolo fuori dal dominio verificabile dall oracolo adottato; ingresso non dichiarato invalido') from exc
    require(all(decision is None for decision in decisions), 'calcolo CRS non qualificabile GEO-006/015/019/020')
    produced = list_(semantics.get('produced_geometry_ids'))
    fields = contract.primary.geometry_fields(contract.vector(actual))
    require(bool(produced) and len(produced) == len(set(produced)) and all(isinstance(i, str) for i in produced), 'provenienza dei campi calcolati assente')
    require(all(any(f['metadata']['plenora.field_id'] == identity and f['metadata']['plenora.geometry.precision'] == 'float64'
                    for f in fields) for identity in produced), 'precisione del risultato calcolato non conforme GEO-014')


def rejection(bundle, result, command):
    require(result.get('preservation') == {'status': 'skipped', 'reason': 'rejected_at_validating_boundary'} and
            result.get('properties') == {}, 'conservazione dopo rifiuto non saltata esplicitamente')
    observation = bundle.json(result.get('error'), 'report')
    require(observation.get('command') == command['id'], 'errore appartenente ad altro comando')
    observed = object_(observation.get('error'))
    require(bundle.contract.error_validator.is_valid(observed), 'busta errore non conforme allo schema adottato')
    require(observed.get('category') == 'crs' and observed.get('phase') in bundle.contract.schema_rejection_phases and
            isinstance(observed.get('code'), str) and
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
                require(surface != 'cli' or command['exit_code'] == bundle.contract.exit_codes['crs'], 'rifiuto CLI diverso da exit normativo')
                rejection(bundle, hop, command)
            else:
                require(command['exit_code'] == 0, 'comando positivo fallito')
                semantics = object_(command.get('semantics'))
                compare_hop(bundle, hop, current, hop.get('output'), semantics)
                current = hop['output']
                operations[key[:3]].add(hop['operation'])
                operations[key[:3]].add(semantics['kind'])
    require(consumed == set(commands), 'comandi estranei al bundle')
    require(all(set(OPERATIONS) <= seen for seen in operations.values()), 'operazioni derivate o piano lecito non esercitati su tutte le superfici')
    require(all('geometry_compute' in seen for seen in operations.values()), 'calcolo geometrico non esercitato su tutte le superfici')


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
