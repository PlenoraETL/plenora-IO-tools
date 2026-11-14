"""Synthetic manifests derived from the current policy, never campaign snapshots."""
import copy
import json

from scripts import system_qualification as q


def manifest(root):
    adoption = json.loads((root / 'contracts/adoption-source.json').read_text(encoding='utf-8'))
    return copy.deepcopy({
        **q.policy(root),
        'manifest_version': 1,
        'gate_id': 'plenora-system-contract-roundtrip-v1',
        'ownership': 'external_system_qualification',
        'external_owner': q.OWNER,
        'icd': {'revision': adoption['contracts_source']['revision'],
                'source': 'contracts/adoption-source.json'},
        'components': [{'name': name, 'role': role, 'revision': None}
                       for name, role in q.COMPONENTS.items()],
        'status': 'satisfied',
        'open_blockers': [],
        'evidence': {'status': 'not_run'},
    })
