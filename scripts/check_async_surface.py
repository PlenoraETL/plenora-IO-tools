"""Lo stub async deriva dalle firme sync, senza inventario duplicato dei metodi."""
import argparse
import ast
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def render(root: Path = ROOT) -> str:
    source = ast.parse((root / 'sdk/python/src/plenora_io/_client.py').read_text(encoding='utf-8'))
    operations = next(n.value for n in source.body if isinstance(n, ast.AnnAssign)
                      and isinstance(n.target, ast.Name) and n.target.id == 'OPERAZIONI')
    names = set(ast.literal_eval(operations)) | {'version', 'capabilities'}
    client = next(n for n in source.body if isinstance(n, ast.ClassDef) and n.name == 'Client')
    lines = ['# Generated from Client signatures; checked by scripts/check_async_surface.py.',
             'from ._client import *', 'from typing import Any', 'import os',
             'from ._limits import Limits', 'from ._models import *', '', 'class AsyncClient:',
             '    def __init__(self, binary: Any = None, *, timeout: float | None = None) -> None: ...']
    for node in client.body:
        if isinstance(node, ast.FunctionDef) and node.name in names:
            lines.append('    async def ' + node.name + '(' + ast.unparse(node.args) + ') -> '
                         + (ast.unparse(node.returns) if node.returns else 'Any') + ': ...')
    return '\n'.join(lines) + '\n'


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    path = ROOT / 'sdk/python/src/plenora_io/_async_client.pyi'
    expected = render()
    if args.write:
        path.write_text(expected, encoding='utf-8', newline='\n')
        return 0
    return int(path.read_text(encoding='utf-8') != expected)


if __name__ == '__main__':
    raise SystemExit(main())
