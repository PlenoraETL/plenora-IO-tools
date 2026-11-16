#!/usr/bin/env python3
"""Ricevute CI riusabili soltanto per identita esatta, con log integrale."""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
ALLOWED = {
    'fmt': ['cargo', 'fmt', '--all', '--', '--check'],
    'clippy': ['cargo', 'clippy', '--workspace', '--all-targets', '--all-features', '--locked', '--', '-D', 'warnings'],
    'test_default': ['cargo', 'test', '--workspace', '--all-targets', '--locked'],
}


def sha(data):
    return hashlib.sha256(data).hexdigest()


def identity(command):
    def output(args):
        return subprocess.check_output(args, cwd=ROOT, text=True).strip()
    # Formatting is independent of native libraries; tests/clippy are not.
    native = command != ALLOWED['fmt']
    gdal = ''
    if native:
        gdal = output(['gdal-config', '--version'])
        libs = output(['gdal-config', '--libs'])
        # Exact native library bytes, not just the version string.
        import glob
        files = sorted(set(glob.glob('/usr/lib/*/libgdal.so*') + glob.glob('/lib/*/libgdal.so*')))
        if not files:
            raise ValueError('identita libreria GDAL non acquisibile: eseguire localmente')
        gdal = {'version': gdal, 'link': libs,
                'digests': sorted(set(sha(Path(p).read_bytes()) for p in files))}
    dirty = output(['git', 'status', '--porcelain', '--untracked-files=all'])
    if dirty:
        raise ValueError('ricevute CI richiedono albero pulito')
    return {'sha': output(['git', 'rev-parse', 'HEAD']), 'command': command,
            'rustc': output(['rustc', '-vV']), 'cargo': output(['cargo', '-V']),
            'system': platform.system(), 'machine': platform.machine(),
            'os_release': Path('/etc/os-release').read_text() if Path('/etc/os-release').exists() else platform.platform(),
            'gdal': gdal,
            'env': {k: os.environ.get(k, '') for k in (
                'RUSTFLAGS', 'RUSTDOCFLAGS', 'RUSTUP_TOOLCHAIN',
                'CARGO_BUILD_TARGET', 'CARGO_BUILD_JOBS', 'CARGO_INCREMENTAL',
                'CARGO_ENCODED_RUSTFLAGS', 'GDAL_HOME', 'GDAL_VERSION',
                'GDAL_INCLUDE_DIR', 'GDAL_LIB_DIR', 'LD_LIBRARY_PATH',
                'PKG_CONFIG_PATH', 'PLENORA_CROSS_FS_TEST_ROOT')}}



def validate(receipt, expected):
    if receipt.get('identity') != expected:
        return ['identita CI diversa (SHA/comando/toolchain/ambiente)']
    log = receipt.get('log')
    if receipt.get('exit') != 0 or not isinstance(log, str) or receipt.get('log_sha256') != sha(log.encode()):
        return ['ricevuta fallita o log alterato']
    if receipt.get('status') != 'PASS' or not receipt.get('origin'):
        return ['ricevuta incompleta']
    return []


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record', type=Path)
    parser.add_argument('--reuse', type=Path)
    parser.add_argument('--name', required=True, choices=ALLOWED)
    parser.add_argument('command', nargs=argparse.REMAINDER)
    args = parser.parse_args(argv)
    command = args.command
    if command and command[0] == '--':
        command = command[1:]
    if command != ALLOWED[args.name]:
        parser.error('comando diverso dal perimetro riusabile')
    if args.record:
        args.record.parent.mkdir(parents=True, exist_ok=True)
        args.record.write_text('{"status":"NON ESEGUITO"}\n', encoding='utf-8')
    expected = None
    try:
        expected = identity(command)
    except (OSError, ValueError, subprocess.SubprocessError) as exc:
        if args.record or (args.reuse and (args.reuse / (args.name + ".json")).exists()):
            print(str(exc), file=sys.stderr)
            return 1
        print(f'Riuso non applicabile: {exc}; esecuzione locale.')
    if args.reuse and expected:
        path = args.reuse / (args.name + '.json')
        if path.is_file():
            try:
                receipt = json.loads(path.read_text(encoding='utf-8'))
                errors = validate(receipt, expected)
            except (OSError, ValueError, TypeError) as exc:
                errors = [str(exc)]
            if errors:
                # Una ricevuta fornita ma fallita/alterata non si nasconde con un retry.
                print('\n'.join(errors), file=sys.stderr)
                return 1
            print('RIUSATO CI, NON RIESEGUITO: ' + json.dumps(receipt, sort_keys=True))
            print(receipt['log'], end='')
            return 0
        print('Ricevuta assente: esecuzione locale, nessun passo omesso.')
    start = time.monotonic()
    run = subprocess.run(command, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    print(run.stdout, end='')
    if args.record:
        receipt = {'identity': expected, 'exit': run.returncode,
                   'status': 'PASS' if run.returncode == 0 else 'FAIL',
                   'log': run.stdout, 'log_sha256': sha(run.stdout.encode()),
                   'seconds': time.monotonic() - start,
                   'origin': os.environ.get('GITHUB_RUN_ID', 'local')}
        args.record.parent.mkdir(parents=True, exist_ok=True)
        args.record.write_text(json.dumps(receipt, indent=2) + '\n', encoding='utf-8')
    return run.returncode


if __name__ == '__main__':
    sys.exit(main())
