"""Reviewed inventory: a new validator predicate requires an explicit audit update."""
import ast
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PATH = ROOT / 'release/system-qualification-audit.json'
AUTHORITIES = {
    'check_variant': 'Campaign variant_rules; ARROW-005 is delegated to the normative oracle, on both variants.',
    'check_fixture': 'Campaign fixture_requirements after ARROW/VOCABULARY/GEO/PRIM input conformance; no operation applicability inferred from a pass-through.',
    'compare_hop': 'ARROW-003/004/008/010, PRIM-004; campaign expected observations and localized declared transformations. Legal plans may change rows without metadata loss.',
    'rejection': 'REJ-001/003 and error-v1.schema.json; campaign first-boundary probes, explicit skipped preservation, public code and complete no-effect snapshots.',
    'validate_results': 'Campaign complete matrix and bound command chain; CLI-2.0 section 8 for the CRS exit code.',
    'validate_operation': 'GEO-000/006/014/015/019/020 and PRIM-005/007; operation semantics bound to command/log, no computation verdict inferred from input validity.',
    'validate_fixtures': 'Campaign complete fixture x variant coverage and all declared properties; normative validity delegated to adopted oracle.',
    'manifest_errors': 'Campaign owner, identities, complete domains, pending/passed states; adoption-source.json fixes normative authority.',
    'validate': 'Campaign no green without the complete owner bundle; no unused artifact may substitute execution evidence.',
    'validate_lock': 'Campaign immutable component/ICD/harness revisions and executed binary coverage; integrity does not authenticate remote membership.',
    'validate_environments': 'Campaign native Linux/Windows run provenance; a binary digest alone is not an execution.',
    'property_keys': 'IO declarative property table; campaign coverage, not a duplicate normative vocabulary.',
}


def predicates(root=ROOT):
    rows = []
    for path in ('scripts/system_qualification.py', 'scripts/qualification_contract.py'):
        tree = ast.parse((root / path).read_text(encoding='utf-8'))
        class Visitor(ast.NodeVisitor):
            def __init__(self):
                self.function = ''

            def visit_FunctionDef(self, node):
                previous, self.function = self.function, node.name
                self.generic_visit(node)
                self.function = previous

            def visit_Call(self, node):
                if isinstance(node.func, ast.Name) and node.func.id == 'require':
                    rows.append({'file': path, 'function': self.function,
                                 'predicate': ast.unparse(node.args[0]), 'diagnostic': ast.literal_eval(node.args[1]),
                                 'authority': AUTHORITIES.get(self.function, 'Bundle representation/integrity policy; no additional Arrow input restriction.')})
                self.generic_visit(node)

            def visit_If(self, node):
                if path.endswith('qualification_contract.py') and any(isinstance(n, (ast.Raise, ast.Return)) for n in node.body):
                    rows.append({'file': path, 'function': self.function, 'predicate': ast.unparse(node.test),
                                 'diagnostic': 'normative adapter branch',
                                 'authority': 'Adopted immutable Git oracle; Arrow physical storage and REJ ordering; preserve opaque metadata bytes.'})
                self.generic_visit(node)
        Visitor().visit(tree)
    return rows


def inventory(root=ROOT):
    gate = json.loads((root / 'release/system-rc-gate.json').read_text(encoding='utf-8'))
    return {'revision': gate['icd']['revision'], 'checks': predicates(root),
            'gate_fields': {key: {'sha256': hashlib.sha256(json.dumps(value, sort_keys=True).encode()).hexdigest(),
                                 'authority': 'adopted contract' if key in ('icd', 'normative_source', 'normative_vocabulary') else 'campaign requirement or explicit pending state'}
                            for key, value in gate.items()},
            'delegation': {'input': 'arrow_primary.schema_verdict then arrow_data._value, from adopted Git objects; JSON vector type limits are not Arrow type limits',
                           'geometry': 'arrow_data.decode_wkb: all contract types, empty geometries, ISO/extended exclusivity, ring closure, member SRID/dimensions, depth, trailing bytes',
                           'crs': 'arrow_data._crs/definition_contradicts: WKT/WKT2/PROJJSON top-level identities, grammar and GEO-020; computation_verdict applies only to coordinate computations',
                           'errors': 'error-v1.schema.json, REJ-003 schema phases from normative text, CLI-2.0 exit projection',
                           'producer_and_operations': 'GEO-006/014/015/019/020 and PRIM-005/007 checked against command semantics, operands and output; other producer/source promises use independent expected outputs and provenance. Out-of-domain computation remains unqualified, never an input rejection. Hashes alone do not authenticate execution.',
                           'streaming': 'IPC file and stream observations admitted; this offline bundle does not infer runtime bounded-memory behavior from serialized bytes'},
            'baseline_findings': {
                'variant_name': 'Removed forbidden absence of geoarrow.wkb; only extension metadata distinguishes the variants.',
                'geometry_everywhere': 'Removed for integer fixtures; each geometry actually present still obeys the normative oracle.',
                'projjson_only': 'Removed; every declared CRS format is interpreted by the pinned grammar.',
                'unresolved': 'Replaced by declared_unresolved from the normative vocabulary; unresolved remains valid only for types_declaration.',
                'uint64_sorted_input': 'Replaced by unordered campaign input; legal sort plans compare independently expected values without fictitious metadata loss.',
                'local_wkb_parser': 'Removed types 1..7 and nonempty restrictions; added all normative value rules through the single decoder.',
                'absent_schema_checks': 'Added version, required keys, storage, IDs, canonical types, CRS dependencies, primary geometry and output validation.',
                'error_axes': 'Full error schema; schema rejection phases validate/connect/probe, CLI exit from contract.',
                'empty_observations': 'Removed blanket nonempty-output restriction; fixture coverage still requires actual sample values.',
                'ordinal_identity': 'Preservation compares fields by field_id; physical reordering is lawful under a declared plan.',
                'primary_spelling': 'PRIM-004 normalization preserves primary state; output must use its normative spelling.'}}


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    document = inventory()
    if args.write:
        PATH.write_text(json.dumps(document, ensure_ascii=False, indent=2) + '\n', encoding='utf-8', newline='\n')
    else:
        raise SystemExit(int(json.loads(PATH.read_text(encoding='utf-8')) != document))
