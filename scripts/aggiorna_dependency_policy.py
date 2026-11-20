#!/usr/bin/env python3
"""Riconcilia le eccezioni IO con i lock, senza cambiare la baseline di famiglia.

Uso deliberato fuori CI: python scripts/aggiorna_dependency_policy.py
Riusa il controllore dello snapshot; non approva nuove deviazioni o scadenze.
"""
import copy
import hashlib
import importlib.util
from pathlib import Path
import re
import tomllib

ROOT = Path(__file__).resolve().parents[1]
HOME = ROOT / '.ci/plenora-dependencies'
NAME = 'plenora-IO-tools'


def carica_gate(home=HOME):
    spec = importlib.util.spec_from_file_location('dependency_policy', home / 'controlla_dipendenze.py')
    gate = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(gate)
    return gate


def riconcilia(data, policy, report):
    """Rimuove solo blocchi identificati come obsoleti dal gate corrente."""
    if any(e['rule'] != 'stale_exception' or e['repo'] != NAME for e in report['errors']):
        raise ValueError('deviazioni attive: aggiornamento automatico rifiutato')
    stale = {e['key'] for e in report['errors']}
    parts = re.split(rb'(?m)(?=^\[\[exceptions\]\]\r?$)', data)
    kept = []
    removed = set()
    for part in parts:
        if part.startswith(b'[[exceptions]]'):
            entry = tomllib.loads(part.decode('utf8'))['exceptions'][0]
            if entry['key'] in stale and entry['repo'] == NAME:
                # Un eventuale blocco successivo di altra natura va preservato.
                boundary = re.search(rb'(?m)^\[', part[len(b'[[exceptions]]'):])
                if boundary:
                    kept.append(part[len(b'[[exceptions]]') + boundary.start():])
                removed.add(entry['key'])
                continue
        kept.append(part)
    result = b''.join(kept)
    expected = copy.deepcopy(policy)
    expected['exceptions'] = [e for e in policy['exceptions'] if e['key'] not in stale]
    if removed != stale or tomllib.loads(result.decode('utf8')) != expected:
        raise ValueError('riconciliazione non conservativa')
    return result


def aggiorna(root=ROOT):
    home = root / '.ci/plenora-dependencies'
    manifest = home / 'plenora-dependencies.toml'
    digest = home / 'plenora-dependencies.sha256'
    gate = carica_gate(home)
    policy = gate.load_policy(manifest, digest)
    report = gate.evaluate(policy, {NAME: root})
    data = riconcilia(manifest.read_bytes(), policy, report)
    updated = tomllib.loads(data.decode('utf8'))
    if gate.evaluate(updated, {NAME: root})['errors']:
        raise ValueError('la policy riconciliata non supera il gate')
    old_hash = hashlib.sha256(manifest.read_bytes()).hexdigest()
    new_hash = hashlib.sha256(data).hexdigest()
    workflow = root / '.github/workflows/ci.yml'
    old_pin = (old_hash + '  plenora-dependencies.toml').encode()
    workflow_data = workflow.read_bytes()
    if workflow_data.count(old_pin) != 1:
        raise ValueError('pin workflow assente o ambiguo')
    writes = {manifest: data, digest: (new_hash + '  plenora-dependencies.toml\n').encode()}
    snapshot = home / 'snapshot.sha256'
    lines = []
    for line in snapshot.read_text(encoding='ascii').splitlines():
        expected, filename = line.split()
        path = home / filename
        if path.parent != home or hashlib.sha256(path.read_bytes()).hexdigest() != expected:
            raise ValueError('snapshot iniziale non integro')
        content = writes.get(path, path.read_bytes())
        lines.append(hashlib.sha256(content).hexdigest() + '  ' + filename)
    writes[snapshot] = ('\n'.join(lines) + '\n').encode()
    writes[workflow] = workflow_data.replace(old_pin, (new_hash + '  plenora-dependencies.toml').encode())
    for path, content in writes.items():
        path.write_bytes(content)
    print(f"Rimosse {len(report['errors'])} eccezioni obsolete; digest e pin aggiornati.")


if __name__ == '__main__':
    aggiorna()
