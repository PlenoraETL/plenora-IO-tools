"""Mutazioni della policy: fonte, metadata, ABI e workflow devono concordare."""
import shutil
import tempfile
import unittest
from pathlib import Path
from scripts.check_python_version import ROOT, check


class PythonPolicy(unittest.TestCase):
    def test_repository(self):
        self.assertEqual(check(), [])

    def test_mutations(self):
        with tempfile.TemporaryDirectory(dir=ROOT / 'target') as directory:
            root = Path(directory)
            files = ['.python-version', 'sdk/python/pyproject.toml', 'crates/plenora-io-py/pyproject.toml', 'crates/plenora-io-py/Cargo.toml', 'Dockerfile.dev', 'Dockerfile.build-linux']
            files += [p.relative_to(ROOT).as_posix() for p in (ROOT / '.github/workflows').glob('*.yml')]
            for name in files:
                (root / name).parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / name, root / name)
            self.assertEqual(check(root), [])
            for name, before, after in [
                ('.python-version', '3.14', '3.13'),
                ('sdk/python/pyproject.toml', '>=3.14', '>=3.13'),
                ('crates/plenora-io-py/Cargo.toml', 'abi3-py314', 'abi3-py313'),
                ('.github/workflows/ci.yml', 'python-version-file: .python-version', 'python-version: "3.14"'),
                ('Dockerfile.dev', 'RUN bash /tmp/plenora-python/install-python.sh', 'RUN true'),
                ('Dockerfile.build-linux', '/opt/plenora-python/bin:${PATH}', '${PATH}'),
            ]:
                path = root / name
                original = path.read_text(encoding='utf-8')
                with self.subTest(name=name):
                    self.assertIn(before, original)
                    path.write_text(original.replace(before, after), encoding='utf-8')
                    self.assertTrue(check(root))
                path.write_text(original, encoding='utf-8')


if __name__ == '__main__':
    unittest.main()
