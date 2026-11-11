"""Inventario conservativo syn: include anche pub in moduli privati e impl di trait.

Fallback offline a rustdoc JSON: nightly-2026-07-21 non installata nel checkout
Windows di verifica. Nessun matching testuale delle firme. syn visita ricorsivamente
i tipi e i bound (anche where/impl Trait); gli use sono risolti per scope. Il grafo
di contenimento include campi privati, enum, alias e protocolli reader/writer.
Alias e bound Arrow ai confini pubblici sono rifiutati esplicitamente; il
catalogo degli item upstream viene ricavato dai sorgenti risolti dal lock.
Le configurazioni cfg sono unite, esclusi solo i moduli cfg(test).
"""
import json
import hashlib
import os
from pathlib import Path
import subprocess
import tomllib
from functools import lru_cache

ROOT = Path(__file__).resolve().parents[1]
REGISTRY = 'assurance/registries/ingressi-arrow-rust.json'
CLASSES = {'ingresso guardato', 'helper non fallibile con precondizione dichiarata',
           'non riceve dati esterni'}


@lru_cache(maxsize=1)
def dependency_catalog(binary):
    """Rebuilt offline from Cargo's locked sources; no handwritten type list."""
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--offline', '--locked', '--format-version', '1',
         '--manifest-path', str(ROOT / 'Cargo.toml')], text=True))
    catalog = {}
    for package in metadata['packages']:
        name = package['name']
        if not (name.startswith('arrow') or name in ('parquet', 'plenora-fork-parquet')):
            continue
        source = Path(package['manifest_path']).parent / 'src'
        files = sorted(source.rglob('*.rs'))
        assert files, ('sorgenti Arrow mancanti', name)
        names = set()
        for start in range(0, len(files), 40):
            names.update(json.loads(subprocess.check_output(
                [str(binary), '--catalog', *map(str, files[start:start+40])], text=True)))
        assert names, ('catalogo Arrow vuoto', name)
        for target in package['targets']:
            if 'lib' in target['kind']:
                catalog[target['name'].replace('-', '_')] = sorted(names)
        catalog[name.replace('-', '_')] = sorted(names)
    assert catalog, 'nessuna dipendenza Arrow nel lock'
    return catalog


def extract(root):
    target = ROOT / 'target/arrow-api'
    target.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, CARGO_BUILD_JOBS='3')
    with (ROOT / 'target/arrow-api-build.log').open('w') as log:
        subprocess.run(['cargo', 'build', '--offline', '--locked', '--manifest-path',
                        str(ROOT / 'scripts/arrow-api/Cargo.toml'), '--target-dir', str(target)],
                       env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
    binary = target / 'debug' / ('arrow-api-inventory.exe' if os.name == 'nt' else 'arrow-api-inventory')
    records = json.loads(subprocess.check_output([str(binary), str(root / 'crates')], text=True))
    records.append({'kind': 'catalog', 'items': dependency_catalog(binary)})
    # Cargo dependency renames are crate aliases in every module of that crate.
    manifests = {}
    for path in (ROOT / 'crates').glob('*/Cargo.toml'):
        manifest = tomllib.loads(path.read_text(encoding='utf-8'))
        aliases = []
        for section in ('dependencies', 'dev-dependencies', 'build-dependencies'):
            for alias, spec in manifest.get(section, {}).items():
                if isinstance(spec, dict) and 'package' in spec:
                    aliases.append([alias.replace('-', '_'), spec['package'].replace('-', '_')])
        manifests[manifest['package']['name'].replace('-', '_')] = aliases
    for record in records:
        if record['kind'] == 'scope':
            record['uses'].extend(manifests.get(record['scope'].split('::')[0], []))
    return records


def inventory(records):
    catalog = next(r['items'] for r in records if r['kind'] == 'catalog')
    def external(path):
        parts = path.split('::')
        return parts[0] in catalog and (len(parts) == 1 or any(p in catalog[parts[0]] for p in parts[1:]))
    scopes = {}
    types = {}
    functions = []
    for r in records:
        if r['kind'] == 'scope':
            scopes.setdefault(r['scope'], []).extend(r['uses'])
        elif r['kind'] == 'type':
            types.setdefault(r['id'], []).append(r)
        elif r['kind'] == 'fn':
            functions.append(r)
    for r in records:
        if r['kind'] == 'alias' and r['id'] not in types:
            types[r['id']] = [r]

    def absolute(path, scope):
        parts = scope.split('::')
        if path == 'crate' or path.startswith('crate::'):
            return parts[0] + path[5:]
        if path == 'self' or path.startswith('self::'):
            return scope + path[4:]
        while path.startswith('super::'):
            parts.pop()
            path = path[7:]
        return '::'.join(parts + [path])

    def resolve(path, scope, seen=frozenset()):
        key = (path, scope)
        if key in seen:
            return set()
        seen = seen | {key}
        first, _, rest = path.partition('::')
        if first in catalog:
            return {path}
        result = {path} if path in types else set()
        local = absolute(path, scope)
        if local in types:
            result.add(local)
        # Resolve module-qualified re-exports, including an alias in the
        # middle of a path (crate::module::arrow_alias::Int64Array).
        for candidate in (path, local):
            parts = candidate.split('::')
            for split in range(len(parts) - 1, 0, -1):
                parent = '::'.join(parts[:split])
                if parent in scopes:
                    result |= resolve('::'.join(parts[split:]), parent, seen)
                    break
        for alias, source in scopes.get(scope, []):
            if alias == first:
                dest = source + ('::' + rest if rest else '')
                result |= resolve(dest, scope, seen)
            elif alias == '*':
                dest = absolute(source, scope) if source.split('::')[0] in ('crate','self','super') else source
                result |= resolve(path, dest, seen)
                relative = absolute(source, scope)
                if relative in scopes:
                    result |= resolve(path, relative, seen)
                if source.split('::')[0] in catalog:
                    result.add(source + '::' + path)
        if not result:
            # Conservative union for inherited super::* and split impl modules:
            # ambiguity increases the inventory; it never silently removes input.
            result |= {t for t in types if t.rsplit('::', 1)[-1] == path.rsplit('::', 1)[-1]}
            for imports in scopes.values():
                for alias, source in imports:
                    if alias == path and source.split('::')[0] in catalog:
                        result.add(source)
        return result

    deps = {name: set().union(*(resolve(p, r['scope']) for r in entries for p in r['deps']))
            for name, entries in types.items()}
    tainted = set()
    while True:
        added = {name for name, ds in deps.items() if any(external(d) or d in tainted for d in ds)} - tainted
        if not added:
            break
        tainted |= added
    def arrow_paths(record):
        return {d for p in record['deps'] for d in resolve(p, record['scope'])
                if external(d) or d in tainted}
    forbidden = [r['id'] for r in records if r['kind'] == 'forbidden'
                 and any(external(d) for p in r['deps'] for d in resolve(p, r['scope']))]
    # Private aliases are rejected when reached by a public signature. Public
    # aliases and associated definitions are checked even without a consumer.
    aliases = {r['id'] for r in records if r['kind'] == 'alias' and arrow_paths(r)}
    while True:
        added = {name for name, ds in deps.items() if ds & aliases} - aliases
        if not added:
            break
        aliases |= added
    forbidden.extend(r['id'] for r in records if r['kind'] == 'forbidden'
                     and any(resolve(p, r['scope']) & aliases for p in r['deps']))
    for r in functions:
        if any(resolve(p, r['scope']) & aliases for p in r['deps']):
            forbidden.append(r['id'])
    assert not forbidden, ('non_classificati: alias o bound Arrow vietati', sorted(set(forbidden)))
    found = {}
    for r in functions:
        ds = set().union(*(resolve(p, r['scope']) for p in r['deps']))
        evidence = sorted(d for d in ds if external(d) or d in tainted)
        if evidence:
            key = r['file'] + '::' + r['id']
            found[key] = sorted(set(found.get(key, [])) | set(evidence))
    return found


def check(root, records=None):
    if records is None:
        records = extract(root)
    found = inventory(records)
    registry = json.loads((root / REGISTRY).read_text(encoding='utf-8'))
    opaque = {r['file'] + '::' + r['scope'] + '::' + hashlib.sha256(r['tokens'].encode()).hexdigest()
              for r in records if r['kind'] == 'opaque'}
    assert opaque == set(registry['macro_items']), 'macro non classificata o modificata: espansione da riesaminare'
    for entry in registry['macro_items'].values():
        assert entry['classificazione'] in CLASSES and entry['motivazione'].strip()
    reexports = {r['file'] + '::' + r['scope'] + '::' + r['tokens']
                 for r in records if r['kind'] == 'reexport'}
    assert reexports == set(registry['reexports']), 're-export non classificato o modificato'
    for entry in registry['reexports'].values():
        assert entry['motivazione'].strip()
    entries = registry['items']
    assert set(entries) == set(found), {'non_classificati': sorted(set(found)-set(entries)),
                                       'obsoleti': sorted(set(entries)-set(found))}
    for key, entry in entries.items():
        assert entry['classificazione'] in CLASSES, key
        assert entry['motivazione'].strip(), key
        assert entry['tipi'] == found[key], (key, 'tipi cambiati', found[key])
    return found
