"""Synthetic manifests derived from the current policy, never campaign snapshots."""
import copy
import json

from scripts import system_qualification as q
from scripts import qualification_exclusions as exclusions


def observed(cell, root):
    """Synthetic mandatory inventory, not evidence from an external execution."""
    fixture, direction = cell['fixture'], cell['direction']
    rows = [{'hop': h, 'proprieta': p, 'stato': 'VERDE', 'motivo': 'synthetic comparison'}
            for h in exclusions.HOP[direction] for p in q.required_properties(root)]
    extra = [{'hop': 'supplementare indipendente', 'prova': p, 'stato': 'VERDE'}
             for p in exclusions.SUPPLEMENTARY.get(fixture, [])]
    if fixture == 'crs_missing':
        extra.append({'hop': 'io.write gpkg negativo', 'prova': 'gpkg-crs-obbligatorio', 'stato': 'VERDE'})
        for row in extra:
            if row['prova'] == 'gpkg-crs-obbligatorio':
                row.update(stato='ATTESO-ROSSO', lacune=['gpkg-crs-obbligatorio'],
                           motivo='rifiuto contrattuale distinto da IPC')
    if fixture == 'unknown_key':
        extra.extend({'hop': 'data.run', 'prova': p, 'stato': 'VERDE'} for p in
                     ('piano esplicito lecito', 'piano esplicito: rapporto localizzato'))
    if fixture in exclusions.PG_REJECTIONS and direction == exclusions.DIREZIONI[0]:
        extra.append({'hop': 'database.write', 'prova': 'rifiuto senza effetto permanente', 'stato': 'VERDE'})
    if fixture == 'dimensions_unknown' and direction == exclusions.DIREZIONI[1]:
        extra.append({'hop': 'database.read', 'prova': 'rifiuto contrattuale GEO-009', 'stato': 'VERDE'})
    if fixture == 'conflicting_crs':
        for row in rows:
            row.update(stato='SALTATO', motivo=exclusions.DEPENDENCY)
        reasons = {
            'conflitto propagato': 'GEO-005 e gate IO: propagazione non applicabile dopo rifiuto verificato',
            'conflitto propagato: report data.run': 'report positivo non applicabile alla prova di rifiuto; nessuna conservazione attestata',
            'conflitto rifiutato: report io.write': 'report positivo non applicabile alla prova di rifiuto; nessuna conservazione attestata',
            'piano risolutivo': 'ingresso conflittuale rifiutato da GEO-005 prima della decisione; ResolvedByDecision su contratto gia costruito non prova questa catena',
        }
        for row in extra:
            if row['prova'] in reasons:
                row.update(stato='SALTATO', motivo=reasons[row['prova']])
        for hop in cell['hops']:
            label = {'plenora-IO-tools': 'io', 'plenora-data-tools': 'data', 'plenora-database-tools': 'database'}[hop['component']]
            extra.append({'hop': label, 'prova': 'GEO-005 indipendente', 'stato': 'VERDE',
                          'before': hop['before'], 'after': hop['after'], 'error': hop['error'],
                          'boundary': hop['boundary'], 'assi': {'category': 'crs', 'phase': 'validate',
                                                               'remote_effect': 'none', 'retry': 'never'}})
    return {'assi': {'fixture': fixture, 'direzione': direction, 'variante': cell['variant'],
                     'piattaforma': {'windows-native': 'Windows', 'linux-native': 'Linux'}[cell['platform']],
                     'strada': {'cli': 'CLI', 'rust': 'Rust', 'runtime': 'Runtime',
                                'python-sync': 'Python 3.14', 'python-async': 'Python async 3.14'}[cell['surface']]},
            'righe': rows, 'extra': extra}


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
