"""The generated registry cannot hide lost coverage or invent counts."""
import copy
import unittest
from scripts.check_sonde_saltate import genera


class Registry(unittest.TestCase):
    def setUp(self):
        self.policy = {'condizioni': [{'id': 'optional', 'ragione_del_salto': 'dependency absent',
            'che_cosa_manca': 'optional dependency', 'perche_non_sta_nella_sdist': 'pure package',
            'che_cosa_resta_verificato': 'separate installed backend job'}]}
        self.measured = {'sonde': ['a', 'b'], 'eseguite': 2, 'fallite': 0,
                         'saltate': [{'id': 'a', 'ragione': 'dependency absent'}]}

    def test_generated_identities_and_counts(self):
        result = genera(self.policy, self.measured)
        self.assertEqual(result['totale'], 1)
        self.assertEqual(result['eseguite_nella_sdist'], 1)
        self.assertEqual(result['sonde_eseguite'], ['b'])
        self.assertEqual(result['condizioni'][0]['sonde'], ['a'])

    def test_unknown_reason_cannot_be_registered(self):
        self.measured['saltate'][0]['ragione'] = 'new accidental skip'
        with self.assertRaises(ValueError):
            genera(self.policy, self.measured)

    def test_missing_explanation_fails(self):
        self.policy['condizioni'][0]['che_cosa_manca'] = ''
        with self.assertRaises(ValueError):
            genera(self.policy, self.measured)

    def test_incomplete_failed_or_duplicate_measurement_fails(self):
        for key, value in [('sonde', []), ('sonde', ['a', 'a']), ('eseguite', 3),
                           ('fallite', 1), ('saltate', self.measured['saltate'] * 2)]:
            changed = copy.deepcopy(self.measured)
            changed[key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                genera(self.policy, changed)

    def test_equal_counts_cannot_hide_changed_identities(self):
        baseline = genera(self.policy, self.measured)
        self.measured['saltate'][0]['id'] = 'b'
        self.assertNotEqual(baseline, genera(self.policy, self.measured))

    def test_platform_exclusion_does_not_hide_required_linux_execution(self):
        condition = copy.deepcopy(self.policy['condizioni'][0])
        condition.update(id='posix', ragione_del_salto='POSIX only', piattaforme=['win32'])
        self.policy['condizioni'].append(condition)
        windows = copy.deepcopy(self.measured)
        windows['piattaforma'] = 'win32'
        windows['saltate'].append({'id': 'b', 'ragione': 'POSIX only'})
        registry = genera(self.policy, windows)
        self.measured['piattaforma'] = 'linux'
        self.assertEqual(registry, genera(self.policy, self.measured, registry))
        self.measured['saltate'].append({'id': 'b', 'ragione': 'POSIX only'})
        with self.assertRaises(ValueError):
            genera(self.policy, self.measured, registry)
