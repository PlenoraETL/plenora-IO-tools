"""Le righe documentate devono essere vere su processi e dati reali."""
import copy
import os
from pathlib import Path
import subprocess
import unittest
from scripts import doc_runtime as gate

NATIVE_PROOF = True


class DocumentedExecution(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        subprocess.run(['cargo', 'build', '--locked', '-q', '-p', 'plenora-io-tools',
                        '--bin', 'plenora-io', '--example', 'doc_tables'], cwd=gate.ROOT, check=True)
        suffix = '.exe' if os.name == 'nt' else ''
        cls.binary = gate.ROOT / 'target/debug' / ('plenora-io' + suffix)
        cls.tables = gate.run(cls.binary.parent / 'examples' / ('doc_tables' + suffix), [])[1]

    def test_every_declared_error_and_deterministic_loss_sample(self):
        self.assertEqual(gate.verify(self.binary, self.tables), [])

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
