"""Generated FileGDB ownership oracle. Expected outcomes come from ownership,
not from a previous executable or the mutated dataset's output.
Run on Windows (exclusive CreateFileW) and unprivileged Linux (chmod 000).
"""
import concurrent.futures
import contextlib
import ctypes
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

BASELINES = []

ROOT = Path(__file__).resolve().parents[3]
FIXTURE = ROOT / 'crates/plenora-io-tools/tests/fixtures/canoniche/canonico.gdb'
OBJECTS = tuple(range(9, 14))
EXTENSIONS = ('gdbtable', 'gdbtablx', 'gdbindexes', 'spx', 'atx', 'field.atx',
              'freelist', 'hbar', 'cdf', 'sdf', 'foreign')
DEFECTS = ('empty', 'half', 'foreign_content', 'directory', 'locked',
           'prefix_case', 'hex_case', 'extension_case', 'whole_case', 'similar_name')

@contextlib.contextmanager
def inaccessible(path):
    if os.name == 'nt':
        kernel = ctypes.WinDLL('kernel32', use_last_error=True)
        kernel.CreateFileW.argtypes = [ctypes.c_wchar_p, ctypes.c_uint32, ctypes.c_uint32,
                                      ctypes.c_void_p, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_void_p]
        kernel.CreateFileW.restype = ctypes.c_void_p
        kernel.CloseHandle.argtypes = [ctypes.c_void_p]
        handle = kernel.CreateFileW(str(path), 0x80000000, 0, None, 3, 0, None)
        if handle == ctypes.c_void_p(-1).value:
            raise OSError(ctypes.get_last_error(), 'exclusive lock failed')
        try:
            yield
        finally:
            kernel.CloseHandle(handle)
    else:
        mode = path.stat().st_mode
        path.chmod(0)
        try:
            # Root/CAP_DAC_OVERRIDE must not silently turn a negative into a pass.
            try:
                with path.open('rb'):
                    pass
            except PermissionError:
                pass
            else:
                raise RuntimeError('chmod 000 ineffective: run the matrix without root privileges')
            yield
        finally:
            path.chmod(mode)


def cases():
    # All physical members, including every system/catalog table and globals.
    members = {p.name: p.read_bytes() for p in FIXTURE.iterdir()}
    # Every extension at every object, even when optional in this fixture.
    # SPX, ATX and freelist seeds are native. Opaque members use a known
    # malformed payload, so case changes cannot make them readable.
    for owner in (1, 4, *OBJECTS):
        for ext in EXTENSIONS:
            name = f'a{owner:08x}.{ext}'
            if name not in members:
                members[name] = (members['a0000000b.spx'] if ext == 'spx' else
                                 (FIXTURE.parent.parent/'filegdb-members'/('native.' + ext.rsplit('.', 1)[-1])).read_bytes()
                                 if ext in ('atx', 'field.atx', 'freelist') else
                                 members['a00000009.gdbindexes'] if ext == 'gdbindexes' else
                                 b'FOREIGN_FORMAT_NOT_FILEGDB')
    for name in ('unrelated.txt', 'dataset.lock', 'a0000000a-.gdbtable'):
        members[name] = b'FOREIGN_FORMAT_NOT_FILEGDB'
    for name, content in sorted(members.items()):
        parts = name.split('.')
        owner = int(parts[0][1:], 16) if len(parts[0]) == 9 and parts[0][0] == 'a' else None
        native = (FIXTURE / name).is_file() or name.rsplit('.', 1)[-1] in ('spx', 'atx', 'freelist', 'gdbindexes')
        for defect in DEFECTS:
            yield name, content, owner, native, defect
    for owner in (1, 4, *OBJECTS):
        for extension in ('gdbtable', 'gdbtablx'):
            name = f'a{owner:08x}.{extension}'
            yield name, members[name], owner, True, 'missing'
    # Exact F5 trigger: unrelated uppercase TIMESTAMPS plus each locked owner
    # member, including the six types and opaque extensions in conferma5.
    for owner in OBJECTS:
        for ext in EXTENSIONS:
            name = f'a{owner:08x}.{ext}'
            yield name, members[name], owner, True, 'locked_upper_timestamps'


def execute(binary, output, index, case):
    name, content, owner, native, defect = case
    cells = []
    failures = []
    with tempfile.TemporaryDirectory(prefix='cell-', dir=output) as temporary:
        folder = Path(temporary)
        dataset = folder / 'case.gdb'
        shutil.copytree(FIXTURE, dataset)
        member = dataset / name
        member.write_bytes(content)
        if defect in ('empty', 'half', 'foreign_content'):
            member.write_bytes({'empty': b'', 'half': content[:len(content)//2],
                                'foreign_content': b'{"type":"FeatureCollection","features":[]}'}[defect])
        elif defect == 'missing':
            member.unlink()
        elif defect == 'directory':
            member.unlink()
            member.mkdir()
        elif defect.endswith('_case'):
            stem, dot, ext = name.partition('.')
            variant = {'prefix_case': name[0].upper() + name[1:],
                       'hex_case': stem[0] + stem[1:].upper() + dot + ext,
                       'extension_case': stem + dot + ext.upper(),
                       'whole_case': name.upper()}[defect]
            intermediate = dataset / 'rename-temporary'
            member.rename(intermediate)
            member = dataset / variant
            intermediate.rename(member)
        elif defect == 'similar_name':
            # Keep the native original; an unassignable lookalike is diagnostic.
            member = dataset / (name.replace('.', '-.', 1) if '.' in name else name + '-foreign')
            member.write_bytes(b'FOREIGN_FORMAT_NOT_FILEGDB')
        if defect == 'locked_upper_timestamps':
            (dataset/'timestamps').rename(dataset/'hold')
            (dataset/'hold').rename(dataset/'TIMESTAMPS')
        case_only = defect.endswith('_case')
        damaged = not ((case_only and native) or defect == 'similar_name')
        catalog_bad = owner in (1, 4) and damaged
        local_bad = OBJECTS.index(owner) if owner in OBJECTS and damaged else None
        # The similar-name control leaves an artificially introduced opaque
        # member malformed; the original native member is unchanged otherwise.
        if defect == 'similar_name' and not native and owner in (1, 4):
            catalog_bad = True
        if defect == 'similar_name' and not native and owner in OBJECTS:
            local_bad = OBJECTS.index(owner)
        lock = inaccessible(member) if defect.startswith('locked') else contextlib.nullcontext()
        with lock:
            operations = [('layers', None), ('inspect', None), ('implicit_read', None)]
            operations += [(op, layer) for layer in range(5) for op in ('read', 'convert')]
            for operation, layer in operations:
                args = ['read' if operation == 'implicit_read' else operation, str(dataset)]
                if operation == 'convert':
                    args += [str(folder/f'out-{layer}.arrow'), '--from', 'filegdb', '--to', 'ipc']
                if layer is not None:
                    args += ['--layer', str(layer)]
                result = subprocess.run([binary, *args], capture_output=True, timeout=60)
                doc = json.loads(result.stdout)
                success = not catalog_bad and operation != 'implicit_read' and (layer is None or layer != local_bad)
                problems = []
                if (result.returncode == 0) != success:
                    problems.append('exit contradicts ownership')
                if result.stderr:
                    problems.append('unexpected stderr')
                if success and result.returncode == 0 and operation in ('layers', 'inspect'):
                    layers = doc.get('result', {}).get('layers', [])
                    if len(layers) != 5 or [x['status']['readable'] for x in layers] != [i != local_bad for i in range(5)]:
                        problems.append('catalog positions/readability contradict ownership')
                    for item in layers:
                        if not item['status']['readable'] and not item['status'].get('error'):
                            problems.append('unreadable object lacks category')
                if success and result.returncode == 0 and operation in ('layers', 'inspect'):
                    # Ten unused system members plus gdb/timestamps in the
                    # fixture; count new unassignable names without reading them.
                    unowned = 12 + int(owner is None and not (FIXTURE/name).exists()) + int(defect == 'similar_name')
                    wanted = f'membri FileGDB non attribuiti esclusi dalla lettura; conteggio {unowned}'
                    reasons = doc['result']['fidelity']['reasons']
                    if not any(r['code'] == 'format_constraint' and r['detail'] == wanted for r in reasons):
                        problems.append('unowned members lack the expected diagnostic count')
                if operation == 'implicit_read' and not catalog_bad and doc.get('error', {}).get('code') != 'LAYER_SELECTION_REQUIRED':
                    problems.append('implicit selection accepted or misclassified')
                expected_category = ('unsupported' if defect in ('empty', 'directory', 'missing') else
                                     'io' if defect.startswith('locked') else 'data_mapping')
                if not catalog_bad and local_bad is not None and result.returncode == 0 and operation in ('layers', 'inspect'):
                    status = doc['result']['layers'][local_bad]['status']
                    if status.get('error', {}).get('error', {}).get('category') != expected_category:
                        problems.append('local category contradicts defect')
                if layer == local_bad and layer is not None and not catalog_bad and doc.get('error', {}).get('category') != expected_category:
                    problems.append('selected category contradicts defect')
                if success and result.returncode == 0 and operation == 'convert':
                    import pyarrow as pa
                    with pa.memory_map(str(folder/f'out-{layer}.arrow'), 'r') as stream:
                        delivered = pa.ipc.open_file(stream).read_all()
                    if not delivered.equals(BASELINES[layer], check_metadata=True):
                        problems.append('healthy object changed values or schema')
                if not success and not doc.get('error', {}).get('category'):
                    problems.append('failure lacks category')
                if catalog_bad and (doc.get('error', {}).get('category'), doc.get('error', {}).get('code'), doc.get('error', {}).get('phase'), result.returncode) != ('data_mapping', 'FORMAT_ERROR', 'read', 3):
                    problems.append('catalog defect is not InvalidContent')
                cell = dict(case=index, member=name, defect=defect, operation=operation,
                            layer=layer, expected_success=success, exit=result.returncode,
                            problems=problems)
                if problems:
                    with (output/'failures-live.jsonl').open('a', encoding='utf8') as error_log:
                        error_log.write(json.dumps(dict(cell, document=doc))+'\n')
                cells.append(cell)
                if problems:
                    failures.append(dict(cell, document=doc))
    return cells, failures


def main():
    binary = str(Path(sys.argv[1]).resolve())
    output = Path(os.environ.get('PLENORA_FILEGDB_ENUM_OUTPUT', ROOT/'target/filegdb-enumeration'))
    output.mkdir(parents=True, exist_ok=True)
    import pyarrow as pa
    with tempfile.TemporaryDirectory(prefix='baseline-', dir=output) as baseline_folder:
        for layer in range(5):
            destination = Path(baseline_folder)/f'{layer}.arrow'
            result = subprocess.run([binary, 'convert', str(FIXTURE), str(destination), '--from', 'filegdb', '--to', 'ipc', '--layer', str(layer)], capture_output=True, timeout=60)
            assert result.returncode == 0 and not result.stderr, result.stdout
            with pa.memory_map(str(destination), 'r') as stream:
                BASELINES.append(pa.ipc.open_file(stream).read_all())
    all_cases = list(cases())
    failures = []
    count = 0
    with (output/'cells.jsonl').open('w', encoding='utf8') as log:
        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
            futures = [pool.submit(execute, binary, output, i, case) for i, case in enumerate(all_cases)]
            for future in concurrent.futures.as_completed(futures):
                try:
                    cells, errors = future.result()
                except BaseException:
                    for queued in futures:
                        queued.cancel()
                    raise
                previous_count = count
                failures.extend(errors)
                for cell in cells:
                    log.write(json.dumps(cell)+'\n')
                count += len(cells)
                if count // 600 != previous_count // 600:
                    print(f'{count} cells; {len(failures)} failures', flush=True)
    summary = dict(cases=len(all_cases), cells=count, failures=len(failures), platform=sys.platform,
                   oracle='catalog global; member local; unowned diagnostic; unambiguous ASCII case accepted')
    (output/'summary.json').write_text(json.dumps(summary, indent=2), encoding='utf8')
    (output/'failures.json').write_text(json.dumps(failures, indent=2), encoding='utf8')
    print(json.dumps(summary), flush=True)
    return bool(failures)

if __name__ == '__main__':
    sys.exit(main())
