"""Offline adapter to the adopted contract's exact Git objects, not a second oracle."""
from __future__ import annotations

import builtins
from functools import lru_cache
import json
import os
import re
from pathlib import Path
import subprocess
import sys
import types

ROOT = Path(__file__).resolve().parents[1]


def git_bytes(repository, *args):
    result = subprocess.run(['git', '-C', str(repository), *args], capture_output=True, check=False)
    if result.returncode:
        raise ValueError('oggetti del contratto adottato non disponibili offline')
    return result.stdout


class Contract:
    def __init__(self, repository, revision):
        self.repository, self.revision = repository, revision
        if git_bytes(repository, 'rev-parse', revision + '^{commit}').decode().strip() != revision:
            raise ValueError('pin normativo non immutabile')
        modules = {}
        original_import = builtins.__import__

        def imported(name, *args, **kwargs):
            return modules[name] if name in modules else original_import(name, *args, **kwargs)

        for name in ('crs_grammar', 'arrow_data', 'arrow_primary'):
            qualified = '_qualification_contract_' + revision + '_' + name
            module = types.ModuleType(qualified)
            module.__dict__['__builtins__'] = dict(vars(builtins), __import__=imported)
            sys.modules[qualified] = module
            exec(compile(self.read('tools/' + name + '.py'), qualified, 'exec'), module.__dict__)
            modules[name] = module
        self.data, self.primary = modules['arrow_data'], modules['arrow_primary']
        from jsonschema import Draft202012Validator
        self.error_schema = json.loads(self.read('schemas/error-v1.schema.json'))
        self.error_validator = Draft202012Validator(self.error_schema)
        text = self.read('specs/data/ARROW-GEOMETRY-SEMANTICS-1.0.md').decode('utf-8')
        rejection = text.split('**REJ-003**', 1)[1].split('A value class', 1)[0]
        self.schema_rejection_phases = tuple(value for value in self.error_schema['properties']['phase']['enum']
                                            if value in re.findall(r'`([^`]+)`', rejection))
        if not self.schema_rejection_phases:
            raise ValueError('regola normativa REJ-003 non leggibile')
        cli = self.read('specs/cli/CLI-2.0.md').decode('utf-8').split('## 8. Exit codes', 1)[1].split('## 9.', 1)[0]
        self.exit_codes = {category: int(code) for code, categories in re.findall(r'\| (\d+) \| ([^\n]+)', cli)
                           for category in re.findall(r'`([^`]+)`', categories)}

    def read(self, path):
        return git_bytes(self.repository, 'show', self.revision + ':' + path)

    def paths(self, directory):
        return git_bytes(self.repository, 'ls-tree', '-r', '--name-only', self.revision, directory).decode().splitlines()

    @staticmethod
    def metadata(metadata):
        # Unknown metadata remains opaque bytes. Only normative namespaces
        # enter the oracle; comparisons still retain every original byte.
        result = {}
        for key, value in (metadata or {}).items():
            if key in (b'plenora.contract.version', b'plenora.primary_geometry', b'plenora.field_id', b'ARROW:extension:name') or key.startswith(b'plenora.geometry.'):
                if key.startswith(b'plenora.geometry.native.'):
                    result[key.decode('utf-8', errors='surrogateescape')] = ''
                    continue
                result[key.decode('utf-8')] = value.decode('utf-8')
        return result

    def vector(self, table):
        return {'schema_metadata': self.metadata(table.schema.metadata), 'rows': [],
                'fields': [{'name': str(i), 'type': str(field.type), 'nullable': field.nullable,
                            'metadata': self.metadata(field.metadata)} for i, field in enumerate(table.schema)]}

    def verdict(self, table):
        """Normative schema precedence, then geometry values in row/field order.

        Arrow owns physical types (including uint64 and nested types); the
        contract's JSON vector codec is deliberately not imposed on real IPC.
        """
        table.validate(full=True)
        vector = self.vector(table)
        found = self.primary.schema_verdict(vector)
        if found:
            return found
        columns = {index: table.column(index).to_pylist() for index, field in enumerate(vector['fields'])
                   if self.data.is_geometry(field)}
        for row in range(table.num_rows):
            for index, field in enumerate(vector['fields']):
                if not table.column(index)[row].is_valid:
                    if not field['nullable']:
                        return self.data.Verdict('data_mapping', 'REJ-001')
                elif index in columns:
                    found = self.data._value(field, columns[index][row])
                    if found:
                        return found
        return None

    def contradiction(self, field):
        md = self.metadata(field.metadata)
        identity, definition = md.get('plenora.geometry.crs_id'), md.get('plenora.geometry.crs_definition')
        try:
            return bool(identity and definition and self.data.definition_contradicts(
                identity, definition, md['plenora.geometry.crs_definition_format']))
        except self.data.Malformed:
            return False  # A malformed definition is not the prescribed contradictory pair.

    def canonical_primary(self, table):
        vector = self.vector(table)
        if self.primary.schema_verdict(vector) is not None:
            return table
        key = self.primary.KEY.encode()
        metadata = dict(table.schema.metadata or {})
        value = self.primary.canonical_value(vector)
        if value is None:
            metadata.pop(key, None)
        else:
            metadata[key] = value.encode()
        return table.replace_schema_metadata(metadata)


@lru_cache(maxsize=None)
def _contract(repository, revision):
    return Contract(repository, revision)


def contract(root=ROOT):
    revision = json.loads((root / 'contracts/adoption-source.json').read_text(encoding='utf-8'))['contracts_source']['revision']
    repository = Path(os.environ.get('PLENORA_CONTRACTS', str(ROOT / '.plenora-contracts'))).resolve()
    return _contract(str(repository), revision)
