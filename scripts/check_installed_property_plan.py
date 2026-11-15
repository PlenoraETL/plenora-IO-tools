"""Run every property-plan probe for a required installed backend, without skips."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


def leaves(suite):
    for test in suite:
        if isinstance(test, unittest.TestSuite):
            yield from leaves(test)
        else:
            yield test


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--tests', type=Path, required=True)
    parser.add_argument('--backend', choices=('native', 'process'), required=True)
    parser.add_argument('--report', type=Path, required=True)
    parser.add_argument('--worker', action='store_true', help=argparse.SUPPRESS)
    args = parser.parse_args()
    if not args.worker:
        # Copy only the tests: no SDK source or repository marker is reachable.
        with tempfile.TemporaryDirectory() as directory:
            tests = Path(directory) / 'tests'
            shutil.copytree(args.tests, tests, ignore=shutil.ignore_patterns('__pycache__'))
            command = [sys.executable, '-I', str(Path(__file__).resolve()), '--worker',
                       '--tests', str(tests), '--backend', args.backend,
                       '--report', str(args.report.resolve())]
            return subprocess.run(command, cwd=directory, check=False).returncode
    import plenora_io
    import pyarrow  # noqa: F401 -- absence must fail, not skip
    origin = Path(plenora_io.__file__).resolve()
    if not origin.is_relative_to(Path(sys.prefix).resolve()) or 'site-packages' not in origin.parts:
        raise RuntimeError('SDK non installato nell interprete di prova')
    sys.path.insert(0, str(args.tests.resolve()))
    import _repository
    if _repository.RADICE is not None:
        raise RuntimeError('le sonde vedono ancora il repository')
    if args.backend == 'native':
        os.environ['PLENORA_IO_RICHIEDI_NATIVO'] = '1'
        os.environ.pop('PLENORA_IO_BIN', None)
        import plenora_io._native  # noqa: F401 -- must be delivered by the wheel
        if plenora_io.Client().backend != 'native':
            raise RuntimeError('backend nativo non selezionato')
    elif not Path(os.environ.get('PLENORA_IO_BIN', '')).is_file():
        raise RuntimeError('artefatto CLI richiesto')
    discovered = unittest.defaultTestLoader.discover(str(args.tests), pattern='test_property_plan.py')
    tests = [t for t in leaves(discovered) if getattr(t, 'native', False) == (args.backend == 'native')]
    if not tests:
        raise RuntimeError('discovery vuota')
    ids = sorted(t.id() for t in tests)
    result = unittest.TextTestRunner(verbosity=2).run(unittest.TestSuite(tests))
    report = {'backend': args.backend, 'sdk': str(origin), 'sonde': ids,
              'eseguite': result.testsRun, 'saltate': len(result.skipped),
              'fallite': len(result.failures) + len(result.errors)}
    args.report.write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8', newline='\n')
    return int(not result.wasSuccessful() or bool(result.skipped) or result.testsRun != len(ids))


if __name__ == '__main__':
    raise SystemExit(main())
