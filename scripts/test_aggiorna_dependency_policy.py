"""La pulizia non concede deroghe e preserva gli altri repository."""
import tomllib
import unittest

from scripts import aggiorna_dependency_policy as updater


class Riconciliazione(unittest.TestCase):
    def setUp(self):
        self.data = b'''baseline = "conservata"
[[exceptions]]
repo = "plenora-IO-tools"
key = "arco-rimosso"
crate = "generic-array"
parent = "heapless@0.7.17"
[[exceptions]]
repo = "altro-repo"
key = "altro"
crate = "heapless"
[[exceptions]]
repo = "plenora-IO-tools"
key = "arco-attivo"
crate = "generic-array"
[[alignments]]
key = "da-preservare"
'''
        self.policy = tomllib.loads(self.data.decode())

    def report(self, key='arco-rimosso', rule='stale_exception', repo=updater.NAME):
        return {'errors': [dict(key=key, rule=rule, repo=repo)]}

    def test_rimuove_arco_obsoleto_preservando_crate_ancora_usato_e_altri_repo(self):
        result = updater.riconcilia(self.data, self.policy, self.report())
        parsed = tomllib.loads(result.decode())
        self.assertEqual([e['key'] for e in parsed['exceptions']], ['altro', 'arco-attivo'])
        self.assertEqual(parsed['alignments'], self.policy['alignments'])
        self.assertEqual(parsed['baseline'], self.policy['baseline'])

    def test_ultimo_blocco_non_elimina_gli_allineamenti_successivi(self):
        result = updater.riconcilia(self.data, self.policy, self.report('arco-attivo'))
        self.assertEqual(tomllib.loads(result.decode())['alignments'], self.policy['alignments'])

    def test_non_approva_devizioni_attive_o_eccezioni_di_altri_repo(self):
        for report in (self.report(rule='duplicate'), self.report('altro', repo='altro-repo')):
            with self.subTest(report=report), self.assertRaises(ValueError):
                updater.riconcilia(self.data, self.policy, report)

    def test_chiave_non_trovata_blocca_la_scrittura(self):
        with self.assertRaises(ValueError):
            updater.riconcilia(self.data, self.policy, self.report('inesistente'))

    def test_nessuna_sorpresa_preserva_i_byte(self):
        self.assertEqual(updater.riconcilia(self.data, self.policy, {'errors': []}), self.data)


if __name__ == '__main__':
    unittest.main()
