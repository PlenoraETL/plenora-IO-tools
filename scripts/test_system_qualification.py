"""La qualifica non può essere promossa riducendo la matrice o i quattro assi."""
import copy
import hashlib
import itertools
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from scripts.system_qualification import DOMAINS, required_properties, validate

ROOT = Path(__file__).resolve().parents[1]


class SystemQualification(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(dir=ROOT / 'target')
        self.root = Path(self.directory.name)
        self.addCleanup(self.directory.cleanup)
        for name in ('contracts/adoption-source.json', 'crates/plenora-io-core/src/property_plan.rs'):
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes((ROOT / name).read_bytes())
        self.document = json.loads((ROOT / 'release/system-rc-gate.json').read_text(encoding='utf-8'))
        self.document.update(status='satisfied', open_blockers=[])
        for component in self.document['components']:
            component['revision'] = 'a' * 40
        evidence = {
            'status': 'passed', 'owner': self.document['external_owner'],
            'icd_revision': self.document['icd']['revision'],
            'components': copy.deepcopy(self.document['components']),
            'results': [], 'artifacts': [],
        }
        for kind in ('manifest', 'output', 'report', 'commands', 'environment', 'fixtures'):
            data = json.dumps({'synthetic_test_artifact': kind}).encode()
            (self.root / kind).write_bytes(data)
            evidence['artifacts'].append({'kind': kind, 'path': kind, 'sha256': hashlib.sha256(data).hexdigest()})
        for coordinates in itertools.product(*DOMAINS.values()):
            cell = dict(zip(('direction', 'platform', 'surface', 'fixture', 'variant'), coordinates))
            cell.update(status='passed', properties_passed=required_properties(self.root),
                        artifacts={kind: kind for kind in ('manifest', 'output', 'report')})
            if cell['fixture'] == 'conflicting_crs':
                cell.update(properties_passed=[], preservation={'status': 'skipped', 'reason': 'rejected_at_validating_boundary'},
                            no_effect=True, error={'category': 'crs', 'code': 'CRS_CONFLICT', 'phase': 'validate', 'remote_effect': 'none', 'retry': {'kind': 'never'}})
            evidence['results'].append(cell)
        self.document['evidence'] = evidence
        for kind in ('manifest', 'report'):
            data = {key: evidence[key] for key in ('owner', 'icd_revision', 'components', 'results')}
            self.write_artifact(kind, data)

    def write_artifact(self, kind, document):
        data = json.dumps(document).encode()
        (self.root / kind).write_bytes(data)
        artifact = next(a for a in self.document['evidence']['artifacts'] if a['kind'] == kind)
        artifact['sha256'] = hashlib.sha256(data).hexdigest()

    def test_complete_shape(self):
        self.assertEqual(validate(self.document, self.root), [])

    def test_every_domain_cannot_be_reduced(self):
        for domain in DOMAINS:
            document = copy.deepcopy(self.document)
            document[domain] = document[domain][:1]
            with self.subTest(domain=domain):
                self.assertTrue(validate(document, self.root))

    def test_partial_and_malformed_evidence(self):
        for key, value in [('results', None), ('results', []), ('artifacts', [None]), ('artifacts', []), ('components', [])]:
            document = copy.deepcopy(self.document)
            document['evidence'][key] = value
            with self.subTest(key=key, value=value):
                self.assertTrue(validate(document, self.root))

    def test_each_crs_axis_and_no_effect_are_required(self):
        for axis in ('category', 'phase', 'code', 'remote_effect', 'retry'):
            document = copy.deepcopy(self.document)
            cell = next(c for c in document['evidence']['results'] if c['fixture'] == 'conflicting_crs')
            del cell['error'][axis]
            with self.subTest(axis=axis):
                self.assertTrue(validate(document, self.root))
        document = copy.deepcopy(self.document)
        cell = next(c for c in document['evidence']['results'] if c['fixture'] == 'conflicting_crs')
        cell['no_effect'] = False
        self.assertTrue(validate(document, self.root))

    def test_digest_and_properties_cannot_be_forged(self):
        original = (self.root / 'output').read_bytes()
        (self.root / 'output').write_bytes(b'changed')
        self.assertTrue(validate(self.document, self.root))
        (self.root / 'output').write_bytes(original)
        self.assertEqual(validate(self.document, self.root), [])
        self.document['required_properties'] = []
        self.assertTrue(validate(self.document, self.root))

    def test_artifact_reference_kind_must_match(self):
        document = copy.deepcopy(self.document)
        document['evidence']['results'][0]['artifacts'] = {
            kind: 'commands' for kind in ('manifest', 'output', 'report')
        }
        self.assertTrue(validate(document, self.root))

    def test_conflict_preservation_cannot_be_green(self):
        cell = next(c for c in self.document['evidence']['results'] if c['fixture'] == 'conflicting_crs')
        cell['properties_passed'] = required_properties(self.root)
        self.assertIn('conservazione dopo rifiuto non saltata esplicitamente', validate(self.document, self.root))

    def test_hashed_report_must_attest_actual_results(self):
        report = json.loads((self.root / 'report').read_text())
        report['results'][0]['status'] = 'failed'
        self.write_artifact('report', report)
        self.assertIn('cella non attestata dal manifest o dal report', validate(self.document, self.root))

    def test_renamed_gate_and_state_incoherences(self):
        from scripts import check_release_contract as gate
        registry = json.loads((ROOT / 'assurance/registries/release-contract-current.json').read_text(encoding='utf-8'))
        voice = next(v for v in registry['invarianti'] if v['id'] == 'sistema.qualifica-cross-component')
        voice['prova']['artefatto'] = 'renamed.json'
        (self.root / 'renamed.json').write_text(json.dumps(self.document))
        with mock.patch.object(gate, 'ROOT', self.root):
            self.assertEqual(gate.stato_prova_esterna(voice, 'renamed.json'), ('passed', []))
            self.assertTrue(gate._differita_ben_dichiarata(voice))
            voice['stato'] = 'verified'
            voice['prova']['stato'] = 'passed'
            self.assertEqual(gate.condizione_qualifica_cross_component({'invarianti': [voice]}), [])
            minimal = {'status': 'satisfied', 'evidence': {'status': 'passed'}, 'components': [], 'open_blockers': []}
            (self.root / 'renamed.json').write_text(json.dumps(minimal))
            self.assertTrue(gate.condizione_qualifica_cross_component({'invarianti': [voice]}))
            self.assertTrue(gate._prove_esterne({'invarianti': [voice]}))


if __name__ == '__main__':
    unittest.main()
