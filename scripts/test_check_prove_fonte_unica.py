"""Controprove: deriva degli insiemi e raccolta oltre il primo guasto."""
import copy
import json
import os
from pathlib import Path
import subprocess
import sys
import shutil
import tempfile
import unittest
from unittest import mock

from scripts import check_prove_fonte_unica as gate
from scripts import check_footer_field_conflicts as footer
from scripts import check_release_contract as release
from scripts import check_wkb_limits_defaults as wkb
from scripts import check_fixture_generators as fixtures


class InventoryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = gate.inventory(gate.ROOT)

    def test_current_sources(self):
        self.assertEqual(gate.verify(gate.ROOT, self.data), [])

    def test_new_test_is_discovered_without_registry_edit(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            folder = root / 'scripts/nested'
            folder.mkdir(parents=True)
            test = folder / 'test_new.py'
            test.write_text('import unittest\nclass New(unittest.TestCase):\n    def test_x(self): pass\n')
            self.assertEqual(gate.python_tests(root), [test])
            test.write_text('this is not Python!\n')
            self.assertEqual(gate.python_tests(root), [test])

    def test_new_registry_without_consumer_is_red(self):
        data = copy.deepcopy(self.data)
        data['registries']['assurance/registries/unwatched.json'] = []
        self.assertTrue(any('unwatched' in e for e in gate.verify(gate.ROOT, data)))

    def test_pr_gate_cannot_be_schedule_only(self):
        data = copy.deepcopy(self.data)
        del data['workflows']['.github/workflows/ci.yml']['on']['pull_request']
        self.assertTrue(any('ogni PR' in e for e in gate.verify(gate.ROOT, data)))

    def test_matrix_cannot_stop_on_first_failure(self):
        data = copy.deepcopy(self.data)
        data['workflows']['extra.yml'] = {'jobs': {'x': {'strategy': {'matrix': {'os': ['linux']}}}}}
        self.assertTrue(any('fail-fast' in e for e in gate.verify(gate.ROOT, data)))

    def test_checkpoint_substitution_with_same_count_is_red(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'scripts').mkdir()
            (root / 'assurance/registries').mkdir(parents=True)
            (root / 'scripts/s9-checkpoint.sh').write_text('passo one command\npasso other command\n')
            (root / 'assurance/registries/passi-del-checkpoint.json').write_text(
                json.dumps({'passi': [{'id': 'one'}, {'id': 'two'}]}))
            self.assertTrue(gate.check_checkpoint(root))

    def test_wkb_inventory_is_exact_not_only_a_total(self):
        observed = {}
        errors, _ = wkb.verifica(wkb.ROOT, observed)
        self.assertEqual(errors, [])
        self.assertEqual(observed, wkb.INVENTARIO)
        substituted = dict(observed)
        key = next(iter(substituted))
        substituted[key + '_renamed'] = substituted.pop(key)
        self.assertEqual(sum(observed.values()), sum(substituted.values()))
        self.assertTrue(wkb.confronta_inventario(substituted))

    def test_fixture_bytes_and_abandoned_files_are_checked(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            sdk = root / 'sdk/python/tests/fixtures'
            sdk.mkdir(parents=True)
            (root / 'scripts').mkdir()
            template = 'from pathlib import Path\nPath(__file__).with_name({name!r}).write_bytes(b"original")\n'
            (sdk / 'generate_example.py').write_text(template.format(name='example.file'))
            (sdk / 'example.file').write_bytes(b'original')
            (root / 'scripts/genera_fixture_multigeometria.py').write_text(template.format(name='other.bin'))
            (root / 'scripts/other.bin').write_bytes(b'original')
            self.assertEqual(fixtures.verify(root), [])
            (sdk / 'example.file').write_bytes(b'tampered')
            (sdk / 'orphan.stream').write_bytes(b'orphan')
            errors = fixtures.verify(root)
            self.assertTrue(any('diversa' in e for e in errors), errors)
            self.assertTrue(any('senza generatore' in e for e in errors), errors)


class CollectionTests(unittest.TestCase):
    def test_coverage_campaign_runs_all_rounds_after_clean_failure(self):
        bash = os.environ.get('PLENORA_BASH') or shutil.which('bash')
        if os.name == 'nt' and Path('C:/Program Files/Git/bin/bash.exe').exists():
            bash = 'C:/Program Files/Git/bin/bash.exe'
        if not bash:
            self.fail('Bash necessario per provare il runner delle campagne')
        (gate.ROOT / 'target').mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=gate.ROOT / 'target') as directory:
            root = Path(directory)
            (root / 'scripts').mkdir()
            script = root / 'scripts/campagne_copertura.sh'
            script.write_text((gate.ROOT / 'scripts/campagne_copertura.sh').read_text(encoding='utf-8'),
                              encoding='utf-8', newline='\n')
            env = dict(os.environ, S9_CAMPAGNE='3', S9_CAMPAGNE_DIR=(root / 'logs').as_posix(),
                       TRACE=(root / 'trace').as_posix())
            result = subprocess.run([bash, '-c',
                'cargo() { echo called >> "$TRACE"; return 23; }; export -f cargo; "$BASH" "$1"',
                'test', script.as_posix()], env=env, capture_output=True, timeout=30)
            self.assertEqual(result.returncode, 1, result.stderr)
            self.assertTrue((root / 'trace').exists(), (result.stdout, result.stderr))
            self.assertEqual((root / 'trace').read_text().splitlines(), ['called'] * 3)
            self.assertEqual(len(list((root / 'logs').glob('*.exit'))), 3)

    def test_launch_error_and_failure_do_not_hide_later_command(self):
        with tempfile.TemporaryDirectory() as directory:
            results = gate.run_all([
                [str(Path(directory) / 'missing-program')],
                [sys.executable, '-c', 'raise SystemExit(7)'],
                [sys.executable, '-c', 'print("last proof")'],
            ], Path(directory))
            self.assertEqual([r['exit'] for r in results], [125, 7, 0])
            self.assertIn('last proof', Path(results[-1]['log']).read_text())

    def test_empty_unittest_module_is_not_green(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'test_empty.py').write_text('value = 1\n')
            results = gate.run_all([[sys.executable, '-m', 'unittest', 'test_empty']], root, root)
            self.assertEqual(results[0]['exit'], 126)

    def test_release_launch_error_does_not_hide_second_gate(self):
        document = {'invarianti': [
            {'id': 'first', 'stato': 'verified', 'prova': {'tipo': 'gate', 'comando': ['missing-program']}},
            {'id': 'last', 'stato': 'verified', 'prova': {'tipo': 'gate', 'comando': ['python3', '-c', 'pass']}},
        ]}
        with mock.patch.object(release.subprocess, 'run', side_effect=[
            FileNotFoundError('absent'), subprocess.CompletedProcess([], 0, '', '')
        ]) as run:
            errors = release.esegui(document)
        self.assertEqual(run.call_count, 2)
        self.assertEqual(len(errors), 1)
        self.assertIn('first', errors[0])

    def test_release_conditions_run_after_invariant_failure(self):
        document = {'invarianti': [], 'autorizzazione_di_release': {'condizioni': [
            {'id': 'a', 'descrizione': 'a'}, {'id': 'b', 'descrizione': 'b'}]}}
        with tempfile.TemporaryDirectory() as directory:
            registry = Path(directory) / 'registry.json'
            registry.write_text(json.dumps(document))
            with mock.patch.object(release, 'REGISTRO', registry), \
                 mock.patch.object(release, 'completezza', return_value=[]), \
                 mock.patch.object(release, 'struttura', return_value=[]), \
                 mock.patch.object(release, 'esegui', return_value=['invariant failed']), \
                 mock.patch.object(release, 'verifica_condizione', side_effect=[ValueError('bad'), []]) as check:
                self.assertEqual(release.main(['--release']), 1)
                self.assertEqual(check.call_count, 2)

    def test_footer_invalid_json_and_crash_are_red_cells(self):
        for response in (b'not JSON', b'[]'):
            with self.subTest(response=response), mock.patch.object(footer.subprocess, 'run',
                    return_value=subprocess.CompletedProcess([], 3, response, b'')):
                self.assertFalse(footer.observe([], Path('absent-output'))[0])
        with mock.patch.object(footer.subprocess, 'run', side_effect=OSError('crash')):
            self.assertFalse(footer.observe([], Path('absent-output'))[0])

    def test_footer_current_contract_and_each_error_axis(self):
        response = {'contract': footer.ERROR_CONTRACT['contract'],
                    'protocol_version': footer.PROTOCOL['protocol_version'],
                    'error': {'category': 'schema', 'phase': 'validate',
                              'remote_effect': 'none', 'retry': {'kind': 'never'},
                              'code': 'schema', 'message': 'conflict'}}
        with tempfile.TemporaryDirectory() as directory:
            destination = Path(directory) / 'output.arrow'
            def observe(document, code=3):
                with mock.patch.object(footer.subprocess, 'run', return_value=
                        subprocess.CompletedProcess([], code, json.dumps(document).encode(), b'')):
                    return footer.observe([], destination)[0]
            self.assertTrue(observe(response))
            for axis, value in [('category', 'io'), ('phase', 'write'),
                                ('remote_effect', 'possible'), ('retry', {'kind': 'immediate'})]:
                changed = copy.deepcopy(response)
                changed['error'][axis] = value
                self.assertFalse(observe(changed), axis)
            self.assertFalse(observe(response, code=1))
            destination.touch()
            self.assertFalse(observe(response))


if __name__ == '__main__':
    unittest.main()
