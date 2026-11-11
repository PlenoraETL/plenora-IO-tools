"""Closed Python API inventory: source definitions plus installed runtime exposure.

No argument-name, annotation or coroutine filter. Private modules are included;
private functions/classes and function-local definitions are not public API.
"""
import ast
import importlib
import importlib.metadata
import inspect
import json
import pkgutil
from pathlib import Path

REGISTRY = 'assurance/registries/ingressi-arrow-python.json'
CLASSES = {'ingresso guardato', 'non riceve dati Arrow esterni',
           'helper con precondizione dichiarata'}


def static_inventory(root, records):
    found = {r['id'] for r in records if r['kind'] == 'python'}

    def walk(nodes, prefix):
        for node in nodes:
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                if not node.name.startswith('_'):
                    found.add(prefix + '.' + node.name)
            elif isinstance(node, ast.ClassDef):
                if not node.name.startswith('_'):
                    walk(node.body, prefix + '.' + node.name)
            else:
                # Includes definitions under conditionals, without entering functions.
                walk(list(ast.iter_child_nodes(node)), prefix)

    base = root / 'sdk/python/src'
    for path in sorted((base / 'plenora_io').rglob('*.py')):
        parts = list(path.relative_to(base).with_suffix('').parts)
        if parts[-1] == '__init__':
            parts.pop()
        walk(ast.parse(path.read_text(encoding='utf-8')).body, '.'.join(parts))
    return found


def runtime_inventory():
    package = importlib.import_module('plenora_io')
    distribution = importlib.metadata.distribution('plenora-io')
    installed = Path(distribution.locate_file('plenora_io/__init__.py')).resolve()
    assert Path(package.__file__).resolve() == installed, 'serve il pacchetto installato'
    native = importlib.import_module('plenora_io._native')  # mandatory, never skipped
    modules = [package, native]
    modules.extend(importlib.import_module(info.name) for info in
                   pkgutil.walk_packages(package.__path__, package.__name__ + '.'))
    return runtime_members(modules)


def runtime_members(modules):
    found = set()

    def walk(obj, prefix, ancestors=()):
        if id(obj) in ancestors:
            return
        for name, member in inspect.getmembers(obj):
            if name.startswith('_'):
                continue
            key = prefix + '.' + name
            if inspect.isroutine(member):
                found.add(key)
            elif inspect.isclass(member) and member.__module__.startswith('plenora_io'):
                walk(member, key, ancestors + (id(obj),))
            elif isinstance(member, property):
                found.add(key)
    for module in modules:
        walk(module, module.__name__)
    return found


def check(root, records, runtime=False):
    registry = json.loads((root / REGISTRY).read_text(encoding='utf-8'))
    for section, found in [('static', static_inventory(root, records))] + (
            [('runtime', runtime_inventory())] if runtime else []):
        check_entries(registry[section], found, section)


def check_entries(entries, found, section):
    # Explicit version conditions only for inherited standard-library methods.
    import sys
    entries = {key: entry for key, entry in entries.items()
               if tuple(entry.get('python_min', [3, 10])) <= sys.version_info[:2]}
    assert set(entries) == found, {'sezione': section,
        'non_classificati': sorted(found - set(entries)),
        'obsoleti': sorted(set(entries) - found)}
    for key, entry in entries.items():
        assert entry['classificazione'] in CLASSES and entry['motivazione'].strip(), key


if __name__ == '__main__':
    root = Path(__file__).resolve().parents[1]
    registry = json.loads((root / REGISTRY).read_text(encoding='utf-8'))
    check_entries(registry['runtime'], runtime_inventory(), 'runtime')
    print('Inventario Python runtime installato e binding nativo: OK')
