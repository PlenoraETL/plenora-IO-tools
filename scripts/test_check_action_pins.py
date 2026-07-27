"""Regression tests for the immutable GitHub Action reference gate."""

from __future__ import annotations

import unittest

from scripts.check_action_pins import validate_reference


class ValidateReferenceTests(unittest.TestCase):
    def test_accepts_full_commit_sha(self) -> None:
        reference = "actions/checkout@" + ("a" * 40)
        self.assertIsNone(validate_reference(reference))

    def test_accepts_local_action(self) -> None:
        self.assertIsNone(validate_reference("./.github/actions/local"))

    def test_accepts_docker_digest(self) -> None:
        reference = "docker://example.invalid/tool@sha256:" + ("b" * 64)
        self.assertIsNone(validate_reference(reference))

    def test_rejects_mutable_action_tag(self) -> None:
        self.assertIsNotNone(validate_reference("actions/checkout@v4"))

    def test_rejects_mutable_action_branch(self) -> None:
        self.assertIsNotNone(validate_reference("owner/action@main"))

    def test_rejects_missing_revision(self) -> None:
        self.assertIsNotNone(validate_reference("owner/action"))

    def test_rejects_short_sha(self) -> None:
        self.assertIsNotNone(validate_reference("owner/action@deadbeef"))

    def test_rejects_mutable_docker_tag(self) -> None:
        self.assertIsNotNone(validate_reference("docker://example.invalid/tool:latest"))


if __name__ == "__main__":
    unittest.main()
