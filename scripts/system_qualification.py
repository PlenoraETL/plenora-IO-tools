"""Verifica matrice, provenienza e concordanza dei rapporti nel bundle owner."""
import hashlib
import itertools
import json
import re
from pathlib import Path
try:
    from . import strict_json
except ImportError:
    import strict_json


# Fonte normativa dei domini; il manifest viene confrontato nei due versi.
DOMAINS = {'required_directions': ['io_to_data_to_database', 'database_to_data_to_io'], 'required_platforms': ['linux-native', 'windows-native'], 'required_surfaces': ['rust', 'cli', 'python-sync', 'python-async', 'runtime'], 'required_fixtures': ['axis_lat_lon', 'conflicting_crs', 'crs_missing', 'crs_unresolved', 'dimensions_unknown', 'geography_semantics', 'int64_beyond_2_53', 'multipolygon_xyzm_srid', 'point_z', 'point_zm', 'polygon_xy_projected', 'types_mixed', 'uint64_ordering', 'unknown_key'], 'fixture_variants': ['canonical_only', 'geoarrow_extension']}

def validate(document: dict, root: Path) -> list[str]:
    errors = []
    if not isinstance(document, dict):
        return ['gate non strutturato']
    if document.get('status') != 'satisfied' or document.get('open_blockers') != []:
        errors.append('gate non soddisfatto o blocchi non chiusi')
    if not isinstance(document.get('icd'), dict):
        return ['ICD non strutturato']
    properties = required_properties(root)
    if document.get('required_properties') != properties:
        errors.append('proprieta diverse dalla tabella dichiarativa')
    adoption = json.loads((root / 'contracts/adoption-source.json').read_text(encoding='utf-8'))
    revision = adoption['contracts_source']['revision']
    if document.get('external_owner') != 'plenora-contracts/conformance':
        errors.append('owner della qualifica non valido')
    if document.get('icd', {}).get('revision') != revision:
        errors.append('ICD diverso dal contratto adottato')
    components = document.get('components', [])
    expected = {'plenora-IO-tools', 'plenora-data-tools', 'plenora-database-tools'}
    if (not isinstance(components, list) or len(components) != len(expected)
            or {c.get('name') for c in components if isinstance(c, dict) and isinstance(c.get('name'), str)} != expected
            or any(not isinstance(c, dict) or not re.fullmatch('[0-9a-f]{40}', str(c.get('revision'))) for c in components)):
        errors.append('revisioni dei tre componenti incomplete')
    evidence = document.get('evidence', {})
    if not isinstance(evidence, dict):
        return errors + ['evidenza non strutturata']
    if evidence.get('status') != 'passed':
        errors.append('evidenza non superata')
    if evidence.get('owner') != document.get('external_owner') or evidence.get('icd_revision') != revision:
        errors.append('provenienza del bundle incompleta')
    if evidence.get('components') != components:
        errors.append('revisioni bundle diverse dal gate')
    axes = ('required_directions', 'required_platforms', 'required_surfaces', 'required_fixtures', 'fixture_variants')
    domains = [DOMAINS[axis] for axis in axes]
    if any(document.get(axis) != DOMAINS[axis] for axis in axes):
        errors.append('domini del gate diversi dalla fonte normativa')
    if any(not isinstance(domain, list) or not domain or len(set(domain)) != len(domain) for domain in domains):
        return errors + ['domini della matrice mancanti o duplicati']
    wanted = set(itertools.product(*domains))
    seen = set()
    cells = evidence.get('results', [])
    if not isinstance(cells, list):
        return errors + ['risultati non strutturati']
    for cell in cells:
        if not isinstance(cell, dict):
            errors.append('cella non strutturata')
            continue
        if any(not isinstance(cell.get(axis), str) for axis in ('direction', 'platform', 'surface', 'fixture', 'variant')):
            errors.append('coordinate cella non valide')
            continue
        key = tuple(cell.get(axis) for axis in ('direction', 'platform', 'surface', 'fixture', 'variant'))
        if key in seen or key not in wanted or cell.get('status') != 'passed':
            errors.append('cella duplicata, estranea o non superata')
        seen.add(key)
        if cell.get('fixture') == 'conflicting_crs':
            preservation = cell.get('preservation')
            if (cell.get('properties_passed') != [] or not isinstance(preservation, dict)
                    or preservation.get('status') != 'skipped'
                    or preservation.get('reason') != 'rejected_at_validating_boundary'):
                errors.append('conservazione dopo rifiuto non saltata esplicitamente')
            error = cell.get('error')
            if not isinstance(error, dict) or error.get('category') != 'crs' or cell.get('no_effect') is not True or not isinstance(error.get('code'), str) or not re.fullmatch('[A-Z][A-Z0-9_]{1,63}', error['code']) or error.get('remote_effect') != 'none' or error.get('retry') != {'kind': 'never'} or error.get('phase') not in ('validate', 'prepare', 'read', 'write'):
                errors.append('rifiuto CRS incompleto')
        elif not isinstance(cell.get('properties_passed'), list) or cell['properties_passed'] != properties:
            errors.append('proprieta della cella incomplete')
    if seen != wanted:
        errors.append('matrice nativa incompleta')
    artifacts = evidence.get('artifacts', [])
    if not isinstance(artifacts, list) or not {'manifest', 'output', 'report', 'commands', 'environment', 'fixtures'} <= {
        a.get('kind') for a in artifacts if isinstance(a, dict) and isinstance(a.get('kind'), str)
    }:
        errors.append('bundle senza tutti gli artefatti richiesti')
        return errors
    paths = {}
    documents = {}
    for artifact in artifacts:
        if not isinstance(artifact, dict):
            errors.append('artefatto non strutturato')
            continue
        path = artifact.get('path', '')
        digest = artifact.get('sha256', '')
        if not isinstance(path, str) or not path or '\\' in path or ':' in path or path.startswith('/') or any(p in ('', '.', '..') for p in path.split('/')):
            errors.append('percorso artefatto non canonico')
            continue
        if path in paths:
            errors.append('artefatto duplicato')
        paths[path] = artifact.get("kind")
        target = (root / path).resolve()
        if not target.is_relative_to(root.resolve()) or not target.is_file() or not isinstance(digest, str) or not re.fullmatch('[0-9a-f]{64}', digest):
            errors.append('artefatto o digest assente')
        elif target.stat().st_size == 0 or hashlib.sha256(target.read_bytes()).hexdigest() != digest:
            errors.append('digest artefatto non corrispondente')
        elif artifact.get('kind') in ('manifest', 'report'):
            try:
                decoded = strict_json.loads(target.read_text(encoding='utf-8'))
            except (ValueError, UnicodeError):
                errors.append('manifest o report non leggibile')
                continue
            if not isinstance(decoded, dict):
                errors.append('manifest o report non strutturato')
                continue
            documents[path] = decoded
            if any(decoded.get(key) != evidence.get(key) for key in ('owner', 'icd_revision', 'components')):
                errors.append('provenienza manifest o report diversa dal bundle')
    for cell in cells:
        refs = cell.get('artifacts') if isinstance(cell, dict) else None
        if not isinstance(refs, dict) or set(refs) != {'manifest', 'output', 'report'} or any(not isinstance(v, str) or paths.get(v) != k for k, v in refs.items()):
            errors.append('cella senza riferimenti agli artefatti verificati')
            continue
        for kind in ('manifest', 'report'):
            recorded = documents.get(refs[kind], {}).get('results')
            if not isinstance(recorded, list) or recorded.count(cell) != 1:
                errors.append('cella non attestata dal manifest o dal report')
    for path, decoded in documents.items():
        kind = paths[path]
        expected_cells = [c for c in cells if isinstance(c, dict)
                          and isinstance(c.get('artifacts'), dict) and c['artifacts'].get(kind) == path]
        recorded = decoded.get('results')
        if not isinstance(recorded, list) or len(recorded) != len(expected_cells) or any(c not in expected_cells for c in recorded):
            errors.append('manifest o report con risultati estranei o mancanti')
    return errors


def required_properties(root: Path) -> list[str]:
    source = (root / 'crates/plenora-io-core/src/property_plan.rs').read_text(encoding='utf-8')
    table = source.split('pub const PROPERTIES:', 1)[1].split('];', 1)[0]
    properties = re.findall(r'\(\s*"([a-z_]+)",\s*"[^"]+"', table)
    if not properties or len(properties) != len(set(properties)):
        raise ValueError('tabella proprieta non valida')
    return properties


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    path = root / 'release/system-rc-gate.json'
    document = json.loads(path.read_text(encoding='utf-8'))
    expected = {**DOMAINS, 'required_properties': required_properties(root)}
    if args.write:
        document.update(expected)
        path.write_text(json.dumps(document, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')
    else:
        raise SystemExit(int(any(document.get(k) != v for k, v in expected.items())))
