"""Le righe documentate devono essere vere su processi e dati reali.

Senza PLENORA_DOC_BINARY compila il profilo base; con la variabile verifica
il binario indicato (e il suo examples/doc_tables) senza cambiarne il profilo,
come check_docset.py --binary. Il catalogo seleziona i campioni applicabili.
"""
import copy
import os
from pathlib import Path
import subprocess
import unittest
from unittest import mock
from scripts import doc_runtime as gate

NATIVE_PROOF = True


class DocumentedExecution(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        suffix = '.exe' if os.name == 'nt' else ''
        if os.environ.get('PLENORA_DOC_BINARY'):
            cls.binary = Path(os.environ['PLENORA_DOC_BINARY']).resolve()
        else:
            subprocess.run(['cargo', 'build', '--locked', '-q', '-p', 'plenora-io-tools',
                            '--bin', 'plenora-io', '--example', 'doc_tables'], cwd=gate.ROOT, check=True)
            cls.binary = Path(os.environ.get('CARGO_TARGET_DIR', gate.ROOT / 'target')) / 'debug' / ('plenora-io' + suffix)
        cls.tables = gate.run(cls.binary.parent / 'examples' / ('doc_tables' + suffix), [])[1]

    def test_every_declared_error_and_deterministic_loss_sample(self):
        self.assertEqual(gate.verify(self.binary, self.tables), [])

    def test_filegdb_expectations_are_declared_for_both_profiles(self):
        rows = gate.cases(self.tables)
        for situation, category, code, exit_code in (
                ('MissingSource', 'not_found', 'SOURCE_NOT_FOUND', 5),
                ('InvalidContent', 'data_mapping', 'FORMAT_ERROR', 3)):
            declared = {r['profile']: r for r in rows
                        if r['driver'] == 'filegdb' and r['situation'] == situation}
            self.assertEqual(set(declared), {'base', 'gdal-backend'})
            for profile, expected in (
                    ('base', ('unsupported', 'validate', 'UNSUPPORTED', 3)),
                    ('gdal-backend', (category, 'read', code, exit_code))):
                self.assertEqual(tuple(declared[profile][k] for k in
                                       ('category', 'phase', 'code', 'exit')), expected)
            self.assertEqual(declared['base']['message'],
                             'FileGDB richiede il tier GDB: compilare con --features gdal-backend')
        for available, profile in ((False, 'base'), (True, 'gdal-backend')):
            selected = gate.active_cases(self.tables, [{'id': 'filegdb', 'available': available}])
            for situation in ('MissingSource', 'InvalidContent'):
                matches = [r for r in selected if r['driver'] == 'filegdb'
                           and r['situation'] == situation]
                self.assertEqual(len(matches), 1)
                self.assertEqual(matches[0]['profile'], profile)
            self.assertTrue(all(r['profile'] in ('tutti', profile) for r in selected))
            self.assertEqual(sum(r['situation'] == 'MissingWriteBackend' for r in selected),
                             int(not available))

    def test_wrong_active_filegdb_expectation_is_detected(self):
        rows = gate.cases(self.tables)
        for row in rows:
            if row['driver'] == 'filegdb' and row['situation'] in ('MissingSource', 'InvalidContent'):
                row['message'] = 'falso'
        with mock.patch.object(gate, 'cases', return_value=rows):
            errors = gate.verify(self.binary, self.tables)
        for situation in ('MissingSource', 'InvalidContent'):
            self.assertTrue(any(f'{situation}/filegdb: message' in e for e in errors), errors)

    def test_false_documented_message_and_loss_are_detected(self):
        tables = copy.deepcopy(self.tables)
        next(e for e in tables['errors'] if e['situation'] == 'ContradictedCrs')['message'] = 'falso'
        driver = next(d for d in tables['losses'] if d['driver'] == 'ipc')
        next(e for e in driver['elements'] if e['element'] == 'Binary')['write'] = 'Rejected'
        errors = gate.verify(self.binary, tables)
        self.assertTrue(any('ContradictedCrs/shp: message' in e for e in errors), errors)
        self.assertTrue(any('loss ipc/Binary' in e for e in errors), errors)


if __name__ == '__main__':
    unittest.main()
