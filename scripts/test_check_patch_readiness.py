"""Regressioni fail-closed della metadata candidate patch 1.0.1."""

from __future__ import annotations

import copy
import hashlib
import importlib.util
import shutil
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
GATE = ROOT / "scripts" / "check_patch_readiness.py"
SPEC = importlib.util.spec_from_file_location("patch_gate", GATE)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("checker patch non importabile")
gate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gate)


HISTORICAL_FILE_HASHES = {
    "release/final-1.0.0.json": "95f43ce122cb1fbf965067f83e32a45f4f8e868715d51cf25dd2cf5cbdf7a34f",
    "release/evidence/technical-freeze-v1.0.0.json": "d6d4ee645032283b432a154728d8fe94b86a5f934756a5786e35da1ef34c33b4",
}


class PatchReadinessTests(unittest.TestCase):
    def setUp(self) -> None:
        self.document = gate.load_json(gate.PATCH_MANIFEST)

    def copy_workspace_fixture(self, directory: str) -> Path:
        root = Path(directory)
        shutil.copy2(ROOT / "Cargo.toml", root / "Cargo.toml")
        shutil.copy2(ROOT / "Cargo.lock", root / "Cargo.lock")
        (root / "fuzz").mkdir()
        shutil.copy2(ROOT / "fuzz" / "Cargo.lock", root / "fuzz" / "Cargo.lock")
        for source in sorted((ROOT / "crates").glob("*/Cargo.toml")):
            destination = root / "crates" / source.parent.name / "Cargo.toml"
            destination.parent.mkdir(parents=True)
            shutil.copy2(source, destination)
        return root

    def test_repository_patch_candidate_is_consistent(self) -> None:
        self.assertEqual(gate.validate_patch_readiness(self.document), [])

    def test_rejects_premature_or_weakened_claims(self) -> None:
        mutations = (
            ("component_version", "1.0.0"),
            ("status", "component_final_tagged"),
            ("verification_claim", "verified_independently"),
            ("independent_review", True),
        )
        for field, value in mutations:
            with self.subTest(field=field):
                document = copy.deepcopy(self.document)
                document[field] = value
                self.assertTrue(gate.validate_patch_readiness(document))

        document = copy.deepcopy(self.document)
        document["claims"]["system_rc"] = True
        self.assertTrue(gate.validate_patch_readiness(document))

        document = copy.deepcopy(self.document)
        document["evidence"][0]["revision"] = "0" * 40
        self.assertTrue(gate.validate_patch_readiness(document))

        document = copy.deepcopy(self.document)
        document["release_action"]["tag_created"] = True
        self.assertTrue(gate.validate_patch_readiness(document))

        document = copy.deepcopy(self.document)
        document["release_criteria"]["revision"] = "0" * 40
        self.assertTrue(gate.validate_patch_readiness(document))

        document = copy.deepcopy(self.document)
        document["functional_delta"]["public_api"] = "changed"
        self.assertTrue(gate.validate_patch_readiness(document))

        document = copy.deepcopy(self.document)
        document["external_dependencies"] = []
        self.assertTrue(gate.validate_patch_readiness(document))

    def test_ci_uses_patch_checker_and_keeps_historical_gate(self) -> None:
        ci = (ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")
        qualification = (
            ROOT / ".github" / "workflows" / "release-qualification.yml"
        ).read_text(encoding="utf-8")
        self.assertIn("check_release_contract.py --historical", ci)
        self.assertIn("test_check_patch_readiness", ci)
        self.assertIn("check_patch_readiness.py", ci)
        self.assertIn("test_check_patch_readiness", qualification)
        self.assertIn("check_patch_readiness.py", qualification)

    def test_workspace_fixture_is_clean(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = self.copy_workspace_fixture(directory)
            self.assertEqual(gate.validate_patch_readiness(self.document, root), [])

    def test_rejects_crate_version_that_does_not_inherit_workspace(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = self.copy_workspace_fixture(directory)
            bad_manifest = next(iter(sorted((root / "crates").glob("*/Cargo.toml"))))
            bad_manifest.write_text(
                bad_manifest.read_text(encoding="utf-8").replace(
                    "version.workspace = true", 'version = "1.0.1"', 1
                ),
                encoding="utf-8",
            )
            self.assertEqual(
                gate.validate_patch_readiness(self.document, root),
                [
                    f"{bad_manifest.relative_to(root)}: "
                    "package.version deve ereditare workspace"
                ],
            )

    def test_rejects_workspace_version_drift(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = self.copy_workspace_fixture(directory)
            manifest = root / "Cargo.toml"
            manifest.write_text(
                manifest.read_text(encoding="utf-8").replace(
                    'version = "1.0.1"', 'version = "1.0.0"', 1
                ),
                encoding="utf-8",
            )
            self.assertEqual(
                gate.validate_patch_readiness(self.document, root),
                ["workspace non allineato a 1.0.1"],
            )

    def test_rejects_workspace_lock_version_drift(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = self.copy_workspace_fixture(directory)
            lockfile = root / "Cargo.lock"
            lockfile.write_text(
                lockfile.read_text(encoding="utf-8").replace(
                    'name = "driver-common"\nversion = "1.0.1"',
                    'name = "driver-common"\nversion = "1.0.0"',
                    1,
                ),
                encoding="utf-8",
            )
            self.assertEqual(
                gate.validate_patch_readiness(self.document, root),
                ["Cargo.lock non allineato a 1.0.1"],
            )

    def test_rejects_fuzz_lock_version_drift(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = self.copy_workspace_fixture(directory)
            lockfile = root / "fuzz" / "Cargo.lock"
            lockfile.write_text(
                lockfile.read_text(encoding="utf-8").replace(
                    'name = "driver-common"\nversion = "1.0.1"',
                    'name = "driver-common"\nversion = "1.0.0"',
                    1,
                ),
                encoding="utf-8",
            )
            self.assertEqual(
                gate.validate_patch_readiness(self.document, root),
                ["fuzz/Cargo.lock non allineato a 1.0.1"],
            )

    def test_historical_release_records_are_byte_immutable(self) -> None:
        for relative, expected in HISTORICAL_FILE_HASHES.items():
            with self.subTest(path=relative):
                normalized = (ROOT / relative).read_bytes().replace(b"\r\n", b"\n")
                self.assertEqual(hashlib.sha256(normalized).hexdigest(), expected)


if __name__ == "__main__":
    unittest.main()
