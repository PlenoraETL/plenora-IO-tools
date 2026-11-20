"""Repository fixtures are available in a checkout, but never inherited by a sdist."""
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest
from unittest import mock

import _repository


class RepositoryFixtureDiscovery(unittest.TestCase):
    def test_exported_source_checkout_keeps_its_fixture_root(self):
        with TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "release").mkdir()
            (root / _repository.MARCATORE).write_text("{}")
            (root / "Cargo.toml").write_text("[workspace]")
            package = root / "sdk/python"
            (package / "tests").mkdir(parents=True)
            (package / "pyproject.toml").write_text("[project]")
            with mock.patch.object(_repository, "__file__", str(package / "tests/_repository.py")):
                self.assertEqual(_repository.radice(), root.resolve())

    def test_extracted_sdist_does_not_inherit_an_outer_checkout(self):
        with TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "release").mkdir()
            (root / _repository.MARCATORE).write_text("{}")
            (root / "Cargo.toml").write_text("[workspace]")
            for relative in ("target/extracted-package", "target/extracted/sdk/python"):
                package = root / relative
                (package / "tests").mkdir(parents=True)
                (package / "pyproject.toml").write_text("[project]")
                with self.subTest(relative=relative), mock.patch.object(
                    _repository, "__file__", str(package / "tests/_repository.py")
                ):
                    self.assertIsNone(_repository.radice())


if __name__ == "__main__":
    unittest.main()
