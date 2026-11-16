"""Controprove: lavoro incompleto e ricevute alterate non diventano PASS."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock
from scripts import fuzz_release as gate
from scripts import ci_receipt


class Replay(unittest.TestCase):
    def setUp(self):
        self.expected = {'dxf_reader': {'fuzz/seeds/dxf_reader/a.dxf': 'a' * 64}}
        log = 'Executed fuzz/seeds/dxf_reader/a.dxf in 1 ms\n'
        self.receipt = {'policy': gate.POLICY, 'inventario': self.expected,
                        'revisione': 'a' * 40, 'esito': 'PASS', 'budget_esecuzioni': 1,
                        'toolchain': 'nightly', 'ambiente': {'gdal': '3'},
                        'ricevute': {'dxf_reader': {'exit': 0, 'log': log,
                            'log_sha256': gate.digest(log.encode()), 'esecuzioni': 1}}}

        self.receipt.update(bersagli_dichiarati=['dxf_reader'], hanno_finito=['dxf_reader'],
                            fermati_a_finding_noto=[], falliti_su_finding_nuovo=[], illeggibili=[])
        self.receipt['ricevute']['dxf_reader']['comando'] = gate.command_for('nightly', 'dxf_reader', self.expected['dxf_reader'])

    def test_complete(self):
        self.assertEqual(gate.verify(self.receipt, self.expected, 'a' * 40), [])

    def test_missing_seed_target_budget_identity_and_old_policy(self):
        for key, value in [('inventario', {}), ('ricevute', {}), ('budget_esecuzioni', 0),
                           ('policy', 'timed-smoke'), ('esito', 'NON ESEGUITO'),
                           ('toolchain', ''), ('ambiente', {})]:
            with self.subTest(key=key):
                bad = copy.deepcopy(self.receipt)
                bad[key] = value
                self.assertTrue(gate.verify(bad, self.expected))
        self.assertTrue(gate.verify(self.receipt, self.expected, 'b' * 40))

    def test_no_output_duplicate_crash_or_wrong_digest(self):
        for change in ({'log': ''}, {'exit': 1}, {'log_sha256': '0' * 64}, {'esecuzioni': 0}):
            bad = copy.deepcopy(self.receipt)
            bad['ricevute']['dxf_reader'].update(change)
            self.assertTrue(gate.verify(bad, self.expected))
        bad = copy.deepcopy(self.receipt)
        r = bad['ricevute']['dxf_reader']
        r['log'] *= 2
        r['log_sha256'] = gate.digest(r['log'].encode())
        self.assertTrue(gate.verify(bad, self.expected))

    def test_real_inventory_includes_history_and_all_targets(self):
        data = gate.inventory()
        self.assertEqual(len(data), 15)
        self.assertIn('fuzz/seeds/dxf_reader/datatable-conteggio-negativo.dxf', data['dxf_reader'])
        self.assertIn('fuzz/seeds/kml_reader/fuzz_utf16_sniff_regression.kml', data['kml_reader'])
        self.assertEqual(len((gate.ROOT / 'fuzz/seeds/kml_reader/fuzz_utf16_sniff_regression.kml').read_bytes()), 314)
        self.assertTrue(data['from_wkb'])
        self.assertTrue(data['shp_wkb'])

    def test_kml_seeds_reproduce_the_historical_test_inputs(self):
        import re
        import codecs
        source = (gate.ROOT / 'crates/driver-kml/src/tests.rs').read_text(encoding='utf-8')
        for name in ('FUZZ_TIMEOUT_REGRESSION', 'FUZZ_EMPTY_POINT_REGRESSION'):
            raw = re.search(r'const ' + name + r':.*?br#"(.*?)"#;', source, re.S)[1].encode()
            self.assertEqual((gate.ROOT / 'fuzz/seeds/kml_reader' / (name.lower() + '.kml')).read_bytes(), raw)
        text = re.search(r'const FUZZ_UTF16_SNIFF_REGRESSION:.*?b"(.*?)";', source, re.S)[1]
        text = re.sub(r'\\\n\s*', '', text)
        raw = codecs.decode(text, 'unicode_escape').encode('latin1')
        self.assertEqual((gate.ROOT / 'fuzz/seeds/kml_reader/fuzz_utf16_sniff_regression.kml').read_bytes(), raw)

    def test_failed_preparation_overwrites_old_pass(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'receipt.json'
            path.write_text(json.dumps(self.receipt))
            with mock.patch.object(gate, 'execute', side_effect=ValueError('build unavailable')):
                self.assertEqual(gate.main(['--execute', '--receipt', str(path)]), 1)
            self.assertEqual(json.loads(path.read_text())['esito'], 'NON ESEGUITO')

    def test_runner_does_not_turn_empty_exit_zero_into_completed_work(self):
        import subprocess
        for log, expected_code in [('Executed fuzz/seeds/dxf_reader/a.dxf in 1 ms\n', 0), ('', 1)]:
            with self.subTest(log=log), tempfile.TemporaryDirectory() as tmp:
                path = Path(tmp) / 'receipt.json'
                def output(command, **kwargs):
                    return 'a' * 40 if 'rev-parse' in command else b''
                def run(command, **kwargs):
                    return subprocess.CompletedProcess(command, 0, stdout=log)
                with mock.patch.object(gate, 'inventory', return_value=self.expected), \
                     mock.patch.object(gate, 'environment', return_value={'gdal': '3'}), \
                     mock.patch.object(gate.subprocess, 'check_output', side_effect=output), \
                     mock.patch.object(gate.subprocess, 'run', side_effect=run), \
                     mock.patch.dict(gate.os.environ, {'S9_CI_FUZZ_RECEIPT': ''}):
                    self.assertEqual(gate.execute(path), expected_code)
                receipt = json.loads(path.read_text())
                self.assertEqual(receipt['esito'], 'PASS' if expected_code == 0 else 'FAIL')

    def test_reuse_requires_same_environment_and_sha(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'ci.json'
            path.write_text(json.dumps(self.receipt), encoding='utf-8')
            reused = gate.reuse(path, self.expected, 'a' * 40, 'nightly', {'gdal': '3'})
            self.assertEqual(reused['riuso']['revisione'], 'a' * 40)
            with self.assertRaises(ValueError):
                gate.reuse(path, self.expected, 'a' * 40, 'nightly', {'gdal': '4'})
            with self.assertRaises(ValueError):
                gate.reuse(path, self.expected, 'b' * 40, 'nightly', {'gdal': '3'})

    def test_missing_or_duplicate_seed_and_wrong_command(self):
        bad = copy.deepcopy(self.receipt)
        bad['ricevute']['dxf_reader']['comando'][-3] = '-runs=0'
        self.assertTrue(gate.verify(bad, self.expected))
        bad = copy.deepcopy(self.receipt)
        bad['hanno_finito'] = []
        self.assertTrue(gate.verify(bad, self.expected))



class CIReceipt(unittest.TestCase):
    def test_same_sha_is_not_enough(self):
        identity = {'sha': 'a' * 40, 'command': ['cargo', 'test'], 'gdal': 'one'}
        receipt = {'identity': identity, 'status': 'PASS', 'exit': 0,
                   'log': 'ok', 'log_sha256': ci_receipt.sha(b'ok'), 'origin': 'run-1'}
        self.assertEqual(ci_receipt.validate(receipt, identity), [])
        for field, value in [('gdal', 'two'), ('command', ['cargo', 'check']), ('sha', 'b' * 40)]:
            expected = dict(identity, **{field: value})
            self.assertTrue(ci_receipt.validate(receipt, expected))
        for field, value in [('exit', 1), ('status', 'NON ESEGUITO'), ('log', 'altered'), ('origin', '')]:
            self.assertTrue(ci_receipt.validate(dict(receipt, **{field: value}), identity))
