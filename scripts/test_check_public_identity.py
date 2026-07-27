"""Regression tests for the cross-component public identity gate."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from scripts.check_public_identity import validate_identity


def write_package(root: Path, directory: str, name: str, source: str = "") -> None:
    package = root / "crates" / directory
    (package / "src").mkdir(parents=True)
    (package / "Cargo.toml").write_text(
        f'[package]\nname = "{name}"\nversion = "0.0.0"\n',
        encoding="utf-8",
    )
    (package / "src" / "lib.rs").write_text(source, encoding="utf-8")


class PublicIdentityTests(unittest.TestCase):
    def test_accepts_unique_io_specific_identity(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_package(root, "model", "plenora-io-model", "pub enum PlenoraIoError {}")
            write_package(root, "driver", "driver-example")

            errors, manifests, sources = validate_identity(root)

            self.assertEqual(errors, [])
            self.assertEqual(manifests, 2)
            self.assertEqual(sources, 2)

    def test_rejects_reserved_package_name(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_package(root, "model", "plenora-io-model")
            write_package(root, "collision", "plenora-core")

            errors, _, _ = validate_identity(root)

            self.assertTrue(any("riservato/collidente" in error for error in errors))

    def test_rejects_duplicate_workspace_package(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_package(root, "model", "plenora-io-model")
            write_package(root, "first", "driver-example")
            write_package(root, "second", "driver-example")

            errors, _, _ = validate_identity(root)

            self.assertTrue(any("package duplicato" in error for error in errors))

    def test_rejects_old_public_error_identity(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_package(root, "model", "plenora-io-model", "pub enum PlenoraError {}")

            errors, _, _ = validate_identity(root)

            self.assertTrue(any("PlenoraError" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
