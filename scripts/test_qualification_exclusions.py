"""Enumerate captured, redacted observations; these fixtures do not attest a run."""
import copy
import itertools
import json
from pathlib import Path
from types import SimpleNamespace
import unittest

import pyarrow as pa

from scripts import qualification_exclusions as policy
from scripts import system_qualification as q

ROOT = Path(__file__).resolve().parents[1]
CASES = json.loads((ROOT / 'scripts/fixtures/qualification_i_cases.json').read_text(encoding='utf-8'))


class Exclusions(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.keys = q.property_keys(ROOT)
        cls.phases = q.qualification_contract.contract().schema_rejection_phases

    def missing(self, cell):
        return policy.missing(cell, self.keys, self.phases)

    def test_enumerate_classes_and_every_reason(self):
        classes = set()
        count = 0
        for base, variant, surface in itertools.product(CASES, q.DOMAINS['fixture_variants'],
                ('CLI', 'Rust', 'Runtime', 'Python 3.14', 'Python async 3.14')):
            cell = copy.deepcopy(base)
            cell['assi'].update(variante=variant, strada=surface)
            original = copy.deepcopy(cell)
            with self.subTest(axes=cell['assi']):
                self.assertEqual(self.missing(cell), [])
                for row in cell['righe'] + cell['extra']:
                    if row['stato'] == 'VERDE':
                        continue
                    classes.add(policy.rule(cell, row, self.keys, self.phases))
                    for change in ({'motivo': 'motivo errato'}, {'stato': 'ROSSO'}, {'stato': 'not_run'}):
                        self.assertIsNone(policy.rule(cell, {**row, **change}, self.keys, self.phases))
                self.assertEqual(cell, original)
            count += 1
        self.assertEqual(count, 140)  # 100 exception cases plus 40 positive obligations.
        self.assertEqual(classes, {'GEO-005/REJ-001', 'SQL-source-limit',
                                   'GPKG-CRS-required', 'declared-PG-rejection'})

    def test_every_property_is_required_exactly_once(self):
        for base in CASES:
            for index in range(len(base['righe'])):
                for duplicate in (False, True):
                    cell = copy.deepcopy(base)
                    row = cell['righe'].pop(index)
                    if duplicate:
                        cell['righe'].extend([row, row])
                    with self.subTest(axes=base['assi'], index=index, duplicate=duplicate):
                        self.assertTrue(self.missing(cell))

    def test_every_required_independent_proof_missing_or_duplicated(self):
        tested = set()
        for base in CASES:
            for index, row in enumerate(base['extra']):
                cell = copy.deepcopy(base)
                cell['extra'].pop(index)
                if not policy.supplementary_missing(cell):
                    continue  # Optional observations are not promoted into obligations.
                tested.add(base['assi']['fixture'])
                self.assertTrue(self.missing(cell))
                cell = copy.deepcopy(base)
                cell['extra'].append(copy.deepcopy(row))
                self.assertTrue(self.missing(cell))
        self.assertEqual(tested, {c['assi']['fixture'] for c in CASES})

    def test_absent_or_invalid_rejection_witness(self):
        for base in CASES:
            if not policy.witness(base, self.phases):
                continue
            for index, row in enumerate(base['extra']):
                if row.get('prova') not in ('GEO-005 indipendente', 'rifiuto senza effetto permanente',
                                            'rifiuto contrattuale GEO-009'):
                    continue
                for change in ({'stato': 'SALTATO'}, {'assi': {}}, {'assi': {**row['assi'], 'retry': 'safe'}},
                               {'assi': {**row['assi'], 'remote_effect': 'partial'}}):
                    cell = copy.deepcopy(base)
                    cell['extra'][index].update(change)
                    self.assertFalse(policy.witness(cell, self.phases))
                    self.assertTrue(self.missing(cell))

    def test_gpkg_declared_negative_needs_positive_ipc_witness(self):
        for base in CASES:
            if base['assi']['fixture'] != 'crs_missing':
                continue
            cell = copy.deepcopy(base)
            cell['extra'] = [e for e in cell['extra'] if e.get('prova') != 'senza CRS: IPC positivo']
            self.assertTrue(self.missing(cell))
            row = next(e for e in base['extra'] if e['stato'] == 'ATTESO-ROSSO')
            for change in ({'lacune': []}, {'lacune': ['altro']}, {'sorpresa': True}):
                self.assertIsNone(policy.rule(base, {**row, **change}, self.keys, self.phases))

    def test_post_rejection_skips_cannot_be_promoted_to_passed(self):
        for cell in CASES:
            if cell['assi']['fixture'] != 'conflicting_crs':
                continue
            for row in cell['righe'] + cell['extra']:
                if row['stato'] == 'SALTATO' and policy.rule(cell, row, self.keys, self.phases) == 'GEO-005/REJ-001':
                    self.assertIsNone(policy.rule(cell, {**row, 'stato': 'VERDE'}, self.keys, self.phases))

    def test_sql_scope_and_declared_witness(self):
        base = next(c for c in CASES if c['assi']['fixture'] == 'axis_lat_lon' and
                    c['assi']['direzione'] == policy.DIREZIONI[1])
        limits = policy.limiti_sql('axis_lat_lon', self.keys)
        hop = {'component': 'plenora-database-tools', 'status': 'skipped', 'source_limits': limits,
               'reason': next(iter(limits.values())), 'command': 'c', 'output': 'o', 'expected': 'e'}
        cell = {'status': 'partial', 'direction': policy.DIREZIONI[1], 'fixture': 'axis_lat_lon',
                'observed': base, 'hops': [hop]}
        self.assertTrue(policy.qualified(cell, self.keys, self.phases))
        for change in ({'source_limits': {}}, {'source_limits': {**limits, 'srid': 'extra'}},
                       {'execution': 'not_run'}, {'execution': 'attempted'}, {'reason': 'errato'},
                       {'component': 'plenora-data-tools'}, {'expected': None}, {'status': 'failed'}):
            self.assertFalse(policy.qualified({**cell, 'hops': [{**hop, **change}]}, self.keys, self.phases))

    def test_sql_retains_residual_byte_comparisons_and_never_counts_computation(self):
        md = {self.keys[p].encode(): b'original' for p in self.keys}
        table = pa.table({'value': [1, 2]}).replace_schema_metadata({b'plenora.contract.version': b'1'})
        table = table.cast(table.schema.set(0, table.schema.field(0).with_metadata(md)))
        actual = table
        bundle = SimpleNamespace(keys=self.keys, contract=SimpleNamespace(verdict=lambda _: None),
                                 arrow=lambda name, kind: ({}, actual if name == 'o' else table))
        cell = {'direction': policy.DIREZIONI[1], 'fixture': 'axis_lat_lon'}
        hop = {'component': 'plenora-database-tools', 'output': 'o', 'expected': 'e', 'operation': 'identity',
               'source_limits': policy.limiti_sql('axis_lat_lon', self.keys),
               'properties': dict.fromkeys(self.keys, 'preserved')}
        command = {'semantics': {'kind': 'pass_through', 'operation_id': 'database.read'}}
        q.compare_sql_source(bundle, cell, hop, command)
        for prop in self.keys.keys() - hop['source_limits'].keys():
            if prop == 'contract_version':
                actual = table.replace_schema_metadata({b'plenora.contract.version': b'changed'})
            else:
                changed = {**md, self.keys[prop].encode(): b'changed'}
                actual = table.cast(table.schema.set(0, table.schema.field(0).with_metadata(changed)))
            with self.subTest(property=prop), self.assertRaises(q.Invalid):
                q.compare_sql_source(bundle, cell, hop, command)
        actual = table.set_column(0, table.schema.field(0), pa.array([99, 2]))
        with self.assertRaisesRegex(q.Invalid, 'valori SQL diversi'):
            q.compare_sql_source(bundle, cell, hop, command)
        actual = table
        for operation, kind in itertools.product(('derived', 'explicit_plan'), q.OPERATION_SEMANTICS):
            with self.assertRaises(q.Invalid):
                q.compare_sql_source(bundle, cell, {**hop, 'operation': operation},
                                     {'semantics': {'kind': kind, 'operation_id': 'calcolo'}})
        for value in (None, '', 1):
            with self.assertRaises(q.Invalid):
                q.compare_sql_source(bundle, cell, hop,
                                     {'semantics': {'kind': 'pass_through', 'operation_id': value}})

    def test_attempted_rejection_bound_to_public_command(self):
        for direction in policy.DIREZIONI:
            proof = 'rifiuto senza effetto permanente' if direction == policy.DIREZIONI[0] else 'rifiuto contrattuale GEO-009'
            axes = {'category': 'crs', 'phase': 'validate', 'remote_effect': 'none', 'retry': 'never'}
            cell = {'direction': direction, 'observed': {'extra': [{'prova': proof, 'stato': 'VERDE', 'assi': axes}]}}
            observation = {'operation': 'database.write' if direction == policy.DIREZIONI[0] else 'database.read',
                           'exit': 3, 'error': axes}
            command = {'exit_code': 3, 'exit_observation': observation, 'public_traces': [observation]}
            q.attempted_rejection(cell, {'output': None}, command)
            for change in ({'exit_code': 0}, {'public_traces': []},
                           {'exit_observation': {**observation, 'operation': 'database.execute'}},
                           {'exit_observation': {**observation, 'error': {}}}):
                with self.assertRaises(q.Invalid):
                    q.attempted_rejection(cell, {'output': None}, {**command, **change})


if __name__ == '__main__':
    unittest.main()
