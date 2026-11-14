"""Rigenera in isolamento le fixture Python e confronta tutti i byte prodotti."""
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def verify(root: Path = ROOT) -> list[str]:
    generators = sorted((root / "sdk/python/tests/fixtures").glob("generate_*.py"))
    generators += [root / "scripts/genera_fixture_multigeometria.py"]
    errors = []
    produced = set()
    for generator in generators:
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            relative = generator.relative_to(root)
            copy = work / relative
            copy.parent.mkdir(parents=True)
            shutil.copyfile(generator, copy)
            try:
                run = subprocess.run([sys.executable, str(copy)], cwd=work,
                                     capture_output=True, timeout=60)
                if run.returncode:
                    errors.append(f"{relative}: exit {run.returncode}: {run.stderr.decode(errors='replace')}")
                    continue
            except (OSError, subprocess.TimeoutExpired) as exc:
                errors.append(f"{relative}: {exc}")
                continue
            outputs = [p for p in work.rglob("*") if p.is_file() and p != copy]
            if not outputs:
                errors.append(f"{relative}: nessuna fixture prodotta")
            for output in outputs:
                relative_output = output.relative_to(work)
                if relative_output in produced:
                    errors.append(f"fixture prodotta da piu' generatori: {relative_output}")
                produced.add(relative_output)
                original = root / relative_output
                if not original.is_file() or output.read_bytes() != original.read_bytes():
                    errors.append(f"{relative}: fixture diversa o assente: {original.relative_to(root)}")
    # L'insieme e' chiuso nei due versi: una fixture SDK abbandonata dal
    # generatore non puo' restare verde per assenza di un confronto.
    sdk = root / 'sdk/python/tests/fixtures'
    expected = {p.relative_to(root) for p in sdk.iterdir() if p.suffix in ('.file', '.stream')}
    actual = {p for p in produced if p.parts[:4] == ('sdk', 'python', 'tests', 'fixtures')}
    for missing in sorted(expected - actual):
        errors.append(f'fixture SDK senza generatore: {missing}')
    return errors


if __name__ == "__main__":
    failures = verify()
    for failure in failures:
        print(failure, file=sys.stderr)
    raise SystemExit(bool(failures))
