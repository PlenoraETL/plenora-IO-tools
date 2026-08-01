"""Regression tests for the component-RC provenance gate."""

from __future__ import annotations

import copy
import subprocess
import unittest
from pathlib import Path

from scripts.check_release_contract import (
    CLI_PROTOCOL_V1,
    CORPUS_SCHEMA,
    CORPUS_MANIFEST,
    DETACHED_LOCKFILES,
    EVIDENCE,
    FREEZE_READINESS,
    GEOMETRY_SOURCE,
    INDEPENDENT_REVIEW,
    PROVENANCE,
    SYSTEM_GATE,
    WORKSPACE_CRATE_MANIFESTS,
    WORKSPACE_LOCK,
    WORKSPACE_MANIFEST,
    load_json,
    load_toml,
    validate_documents,
    validate_catalog_producer,
    validate_cli_protocol_v1,
    validate_current_checkout,
    validate_release_tag_binding,
    validate_workspace_versions,
)


ROOT = Path(__file__).resolve().parents[1]
CI_WORKFLOW = ROOT / ".github" / "workflows" / "ci.yml"
RELEASE_QUALIFICATION_WORKFLOW = (
    ROOT / ".github" / "workflows" / "release-qualification.yml"
)


class ReleaseContractTests(unittest.TestCase):
    def setUp(self) -> None:
        self.provenance = load_json(PROVENANCE)
        self.system_gate = load_json(SYSTEM_GATE)
        self.freeze_readiness = load_json(FREEZE_READINESS)
        self.evidence = load_json(EVIDENCE)
        self.independent_review = load_json(INDEPENDENT_REVIEW)
        self.corpus_schema = load_json(CORPUS_SCHEMA)
        self.corpus_manifest = load_json(CORPUS_MANIFEST)
        self.geometry_source = GEOMETRY_SOURCE.read_text(encoding="utf-8")
        self.workspace_manifest = load_toml(WORKSPACE_MANIFEST)
        self.crate_manifests = [
            load_toml(path) for path in WORKSPACE_CRATE_MANIFESTS
        ]
        self.lockfile = load_toml(WORKSPACE_LOCK)
        self.detached_lockfiles = [load_toml(path) for path in DETACHED_LOCKFILES]
        self.cli_protocol = load_json(CLI_PROTOCOL_V1)

    def validate(
        self,
        provenance=None,
        system_gate=None,
        freeze_readiness=None,
        evidence=None,
        independent_review=None,
        corpus_schema=None,
    ) -> list[str]:
        return validate_documents(
            provenance if provenance is not None else self.provenance,
            system_gate if system_gate is not None else self.system_gate,
            (
                freeze_readiness
                if freeze_readiness is not None
                else self.freeze_readiness
            ),
            evidence if evidence is not None else self.evidence,
            (
                independent_review
                if independent_review is not None
                else self.independent_review
            ),
            corpus_schema if corpus_schema is not None else self.corpus_schema,
            self.corpus_manifest,
            self.geometry_source,
        )

    def test_repository_manifests_are_consistent(self) -> None:
        self.assertEqual(self.validate(), [])
        self.assertEqual(
            validate_workspace_versions(
                self.workspace_manifest,
                self.crate_manifests,
                self.lockfile,
                self.detached_lockfiles,
            ),
            [],
        )

    def test_release_qualification_requires_functional_gates_for_same_sha(self) -> None:
        ci = CI_WORKFLOW.read_text(encoding="utf-8")
        release = RELEASE_QUALIFICATION_WORKFLOW.read_text(encoding="utf-8")

        self.assertIn("  workflow_call:", ci)
        self.assertIn(
            "  functional-gates:\n    uses: ./.github/workflows/ci.yml",
            release,
        )
        self.assertIn(
            "  qualify-current-checkout:\n    needs: functional-gates",
            release,
        )

    def test_every_functional_gate_checks_out_the_caller_sha(self) -> None:
        ci = CI_WORKFLOW.read_text(encoding="utf-8")
        checkout_steps = ci.count("- uses: actions/checkout@")
        pinned_checkouts = ci.count("ref: ${{ env.CANDIDATE_SHA }}")

        self.assertGreater(checkout_steps, 0)
        self.assertEqual(pinned_checkouts, checkout_steps)
        self.assertIn(
            "CANDIDATE_SHA: ${{ github.event_name == 'pull_request' && "
            "github.event.pull_request.head.sha || github.sha }}",
            ci,
        )
        self.assertIn(
            '--expected-revision "$CANDIDATE_SHA"',
            ci,
        )

    def test_direct_ci_push_trigger_excludes_tags(self) -> None:
        ci = CI_WORKFLOW.read_text(encoding="utf-8")

        self.assertIn('  push:\n    branches:\n      - "**"', ci)

    def test_gdal_matrix_only_runs_filegdb_writes_when_runtime_supports_create(self) -> None:
        ci = CI_WORKFLOW.read_text(encoding="utf-8")

        self.assertIn("filegdb_write_available: false", ci)
        self.assertIn("filegdb_write_available: true", ci)
        self.assertIn(
            "if: matrix.filegdb_write_available",
            ci,
        )
        self.assertIn(
            "--expect-available ${{ matrix.filegdb_write_available }}",
            ci,
        )

    def test_release_workflow_binds_manual_sha_and_tag_version(self) -> None:
        release = RELEASE_QUALIFICATION_WORKFLOW.read_text(encoding="utf-8")

        self.assertIn("      expected_revision:", release)
        self.assertIn("required: true", release)
        self.assertIn("inputs.expected_revision", release)
        self.assertIn("--release-tag", release)
        self.assertIn("github.ref_name", release)

    def test_current_checkout_qualification_accepts_clean_expected_revision(self) -> None:
        revision = "a" * 40

        def run_git(arguments: list[str]) -> subprocess.CompletedProcess[str]:
            outputs = {
                ("rev-parse", "--verify", "HEAD^{commit}"): revision + "\n",
                ("rev-parse", "--verify", f"{revision}^{{commit}}"): revision + "\n",
                ("status", "--porcelain=v1", "--untracked-files=all"): "",
            }
            return subprocess.CompletedProcess(arguments, 0, outputs[tuple(arguments)], "")

        self.assertEqual(validate_current_checkout(revision, run_git), [])

    def test_current_checkout_qualification_rejects_revision_mismatch(self) -> None:
        head = "a" * 40
        expected = "b" * 40

        def run_git(arguments: list[str]) -> subprocess.CompletedProcess[str]:
            outputs = {
                ("rev-parse", "--verify", "HEAD^{commit}"): head + "\n",
                ("rev-parse", "--verify", f"{expected}^{{commit}}"): expected + "\n",
                ("status", "--porcelain=v1", "--untracked-files=all"): "",
            }
            return subprocess.CompletedProcess(arguments, 0, outputs[tuple(arguments)], "")

        errors = validate_current_checkout(expected, run_git)
        self.assertTrue(any("does not match" in error for error in errors))

    def test_current_checkout_qualification_rejects_tracked_and_untracked_dirt(self) -> None:
        revision = "a" * 40
        for status in (" M tracked.txt\n", "?? untracked.txt\n"):
            with self.subTest(status=status):
                def run_git(arguments: list[str]) -> subprocess.CompletedProcess[str]:
                    outputs = {
                        ("rev-parse", "--verify", "HEAD^{commit}"): revision + "\n",
                        ("rev-parse", "--verify", f"{revision}^{{commit}}"): revision + "\n",
                        ("status", "--porcelain=v1", "--untracked-files=all"): status,
                    }
                    return subprocess.CompletedProcess(
                        arguments, 0, outputs[tuple(arguments)], ""
                    )

                errors = validate_current_checkout(revision, run_git)
                self.assertTrue(any("not clean" in error for error in errors))

    def test_current_checkout_qualification_rejects_anything_but_one_full_sha(self) -> None:
        def unexpected_git(arguments: list[str]) -> subprocess.CompletedProcess[str]:
            self.fail(f"git must not run for a non-exact revision: {arguments}")

        for revision in (
            "",
            "a" * 7,
            "a" * 39,
            "a" * 41,
            "HEAD",
            "HEAD~1",
            "main",
            "refs/heads/main",
            "v1.0.0-rc.2",
            "-c",
            "--upload-pack=echo",
            "A" * 40,
            "a" * 39 + "G",
            "a" * 40 + "\n",
            " " + "a" * 40,
        ):
            with self.subTest(revision=revision):
                self.assertTrue(validate_current_checkout(revision, unexpected_git))

    def test_release_tag_binding_accepts_only_the_declared_workspace_version(self) -> None:
        version = self.workspace_manifest["workspace"]["package"]["version"]
        self.assertEqual(
            validate_release_tag_binding(f"v{version}", self.workspace_manifest),
            [],
        )

        for tag in (
            "",
            version,
            f"V{version}",
            f"v{version}.1",
            "v9.9.9",
            f"refs/tags/v{version}",
        ):
            with self.subTest(tag=tag):
                self.assertTrue(
                    validate_release_tag_binding(tag, self.workspace_manifest)
                )

    def test_release_tag_binding_fails_closed_without_a_workspace_version(self) -> None:
        version = self.workspace_manifest["workspace"]["package"]["version"]
        for manifest in (
            {},
            {"workspace": {}},
            {"workspace": {"package": {}}},
            {"workspace": {"package": {"version": ""}}},
            {"workspace": {"package": {"version": 1}}},
        ):
            with self.subTest(manifest=manifest):
                self.assertTrue(validate_release_tag_binding(f"v{version}", manifest))

    def test_rejects_workspace_version_drift(self) -> None:
        manifest = copy.deepcopy(self.workspace_manifest)
        manifest["workspace"]["package"]["version"] = "0.0.0"
        self.assertTrue(
            validate_workspace_versions(
                manifest,
                self.crate_manifests,
                self.lockfile,
                self.detached_lockfiles,
            )
        )

    def test_cli_protocol_freezes_six_json_envelopes_not_the_rust_api(self) -> None:
        self.assertEqual(validate_cli_protocol_v1(self.cli_protocol), [])
        self.assertEqual(self.cli_protocol["compatibility_scope"], "cli_json_only")
        self.assertEqual(self.cli_protocol["rust_api"]["status"], "internal_unstable")
        self.assertEqual(len(self.cli_protocol["envelopes"]), 6)

    def test_cli_protocol_rejects_legacy_convert_loss_field(self) -> None:
        protocol = copy.deepcopy(self.cli_protocol)
        protocol["envelopes"]["convert"]["forbidden_legacy_fields"] = []
        self.assertTrue(validate_cli_protocol_v1(protocol))

    def test_cli_protocol_rejects_catalog_driver_field_declaration_mutations(self) -> None:
        catalog = self.cli_protocol["envelopes"]["catalog"]
        mutations = (
            ("optional_driver_fields", []),
            ("current_producer", {"required_driver_fields": ["available"]}),
            ("driver_field_semantics", {}),
        )
        for field, replacement in mutations:
            with self.subTest(field=field):
                protocol = copy.deepcopy(self.cli_protocol)
                protocol["envelopes"]["catalog"][field] = replacement
                self.assertTrue(validate_cli_protocol_v1(protocol))

        self.assertNotIn("required_driver_fields", catalog)

    def test_cli_protocol_rejects_catalog_driver_semantic_mutations(self) -> None:
        mutations = (
            (("available", "type"), "string"),
            (("available", "true_when"), "cargo_feature_enabled"),
            (("required_feature", "type"), ["string"]),
            (("required_feature", "filegdb"), None),
            (("required_feature", "other_drivers"), "gdal-backend"),
        )
        for path, replacement in mutations:
            with self.subTest(path=path):
                protocol = copy.deepcopy(self.cli_protocol)
                target = protocol["envelopes"]["catalog"]["driver_field_semantics"]
                target[path[0]][path[1]] = replacement
                self.assertTrue(validate_cli_protocol_v1(protocol))

    def test_legacy_catalog_producer_may_omit_additive_driver_fields(self) -> None:
        legacy = {
            "status": "ok",
            "protocol_version": 1,
            "contract": "plenora-io-catalog-v1",
            "determinism": "byte_for_byte",
            "drivers": [{"id": "filegdb"}],
        }
        self.assertEqual(
            validate_catalog_producer(legacy, self.cli_protocol, current=False),
            [],
        )

    def test_legacy_catalog_producer_still_rejects_invalid_optional_field(self) -> None:
        legacy = {
            "status": "ok",
            "protocol_version": 1,
            "contract": "plenora-io-catalog-v1",
            "determinism": "byte_for_byte",
            "drivers": [{"id": "filegdb", "available": "false"}],
        }
        self.assertTrue(
            validate_catalog_producer(legacy, self.cli_protocol, current=False)
        )

    def test_current_catalog_producer_must_emit_both_additive_fields(self) -> None:
        current = {
            "status": "ok",
            "protocol_version": 1,
            "contract": "plenora-io-catalog-v1",
            "determinism": "byte_for_byte",
            "drivers": [{"id": "filegdb", "available": False}],
        }
        self.assertTrue(
            validate_catalog_producer(current, self.cli_protocol, current=True)
        )

    def test_current_catalog_producer_rejects_envelope_identity_drift(self) -> None:
        current = {
            "status": "ok",
            "protocol_version": 1,
            "contract": "plenora-io-catalog-v1",
            "determinism": "byte_for_byte",
            "drivers": [
                {"id": "geojson", "available": True, "required_feature": None},
                {"id": "filegdb", "available": True, "required_feature": "gdal-backend"},
            ],
        }
        self.assertEqual(
            validate_catalog_producer(current, self.cli_protocol, current=True),
            [],
        )

        for field, replacement in (
            ("status", "error"),
            ("protocol_version", 2),
            ("protocol_version", True),
            ("protocol_version", "1"),
            ("contract", "plenora-io-catalog-v2"),
            ("determinism", "best_effort"),
        ):
            with self.subTest(field=field, replacement=replacement):
                document = copy.deepcopy(current)
                document[field] = replacement
                self.assertTrue(
                    validate_catalog_producer(
                        document, self.cli_protocol, current=True
                    )
                )

        self.assertTrue(
            validate_catalog_producer(
                ["not", "an", "object"], self.cli_protocol, current=True
            )
        )

    def test_rejects_crate_not_inheriting_release_version(self) -> None:
        manifests = copy.deepcopy(self.crate_manifests)
        manifests[0]["package"]["version"] = "0.1.0-rc.3"
        self.assertTrue(
            validate_workspace_versions(
                self.workspace_manifest,
                manifests,
                self.lockfile,
                self.detached_lockfiles,
            )
        )

    def test_rejects_stale_path_dependency_in_detached_lockfile(self) -> None:
        detached = copy.deepcopy(self.detached_lockfiles)
        changed = False
        for package in detached[0]["package"]:
            if package.get("name") == "plenora-io-model":
                package["version"] = "0.0.0"
                changed = True
                break
        self.assertTrue(changed)
        self.assertTrue(
            validate_workspace_versions(
                self.workspace_manifest,
                self.crate_manifests,
                self.lockfile,
                detached,
            )
        )

    def test_rejects_system_rc_claim(self) -> None:
        provenance = copy.deepcopy(self.provenance)
        provenance["claims"]["system_rc"] = True
        self.assertTrue(self.validate(provenance=provenance))

    def test_rejects_unrecorded_candidate_status(self) -> None:
        provenance = copy.deepcopy(self.provenance)
        provenance["icd"]["conformance_claim"] = "full"
        self.assertTrue(self.validate(provenance=provenance))

    def test_rejects_undeclared_proposed_gap(self) -> None:
        provenance = copy.deepcopy(self.provenance)
        provenance["declared_gaps"] = [
            gap for gap in provenance["declared_gaps"] if gap["rule"] != "R2.8"
        ]
        self.assertTrue(self.validate(provenance=provenance))

    def test_rejects_release_version_or_tag_form_drift(self) -> None:
        for path, replacement in (
            (("component_version",), "0.1.0-rc.2"),
            (("release_tag", "name"), "v0.1.0-rc.2"),
            (("release_tag", "tag_form"), "lightweight"),
        ):
            provenance = copy.deepcopy(self.provenance)
            target = provenance
            for key in path[:-1]:
                target = target[key]
            target[path[-1]] = replacement
            self.assertTrue(self.validate(provenance=provenance), path)

    def test_rejects_wire_version_drift(self) -> None:
        provenance = copy.deepcopy(self.provenance)
        provenance["wire_contract_version"] = 2
        self.assertTrue(self.validate(provenance=provenance))

    def test_rejects_system_gate_promotion_without_new_review(self) -> None:
        gate = copy.deepcopy(self.system_gate)
        gate["status"] = "satisfied"
        self.assertTrue(self.validate(system_gate=gate))

    def test_rejects_external_qualification_result_drift(self) -> None:
        gate = copy.deepcopy(self.system_gate)
        gate["evidence"]["current_system_qualification"]["roundtrip"] = "84/84"
        self.assertTrue(self.validate(system_gate=gate))

    def test_rejects_rollback_after_technical_freeze(self) -> None:
        provenance = copy.deepcopy(self.provenance)
        provenance["freeze_status"] = "pre_freeze"
        self.assertTrue(self.validate(provenance=provenance))

    def test_allows_component_rc_tag_after_pre_tag_ci_while_review_is_open(self) -> None:
        self.assertTrue(self.freeze_readiness["release_authorized"])
        self.assertTrue(self.freeze_readiness["gates"]["pre_tag_ci"])
        self.assertFalse(
            self.freeze_readiness["assurance_attributes"]["independent_review"]
        )
        self.assertNotIn("independent_review", self.freeze_readiness["gates"])
        self.assertEqual(self.validate(), [])

    def test_rejects_independent_claim_while_review_is_open(self) -> None:
        provenance = copy.deepcopy(self.provenance)
        provenance["release_decision"]["verification_claim"] = (
            "verified_independently"
        )
        errors = self.validate(provenance=provenance)
        self.assertTrue(
            any("verified_independently senza review" in error for error in errors)
        )

    def test_rejects_independent_review_reintroduced_as_rc_gate(self) -> None:
        readiness = copy.deepcopy(self.freeze_readiness)
        readiness["gates"]["independent_review"] = False
        self.assertTrue(self.validate(freeze_readiness=readiness))

    def test_rejects_incomplete_shared_corpus_schema(self) -> None:
        schema = copy.deepcopy(self.corpus_schema)
        del schema["properties"]["cases"]["items"]["properties"]["sha256"]
        self.assertTrue(self.validate(corpus_schema=schema))

    def test_rejects_candidate_ci_evidence_drift(self) -> None:
        for path, replacement in (
            (("run_id",), 0),
            (("coverage_artifact", "sha256"), "0" * 64),
            (("windows_filegdb_benchmark_artifact", "sha256"), "0" * 64),
            (("library_line_coverage", "minimum_percent"), 0),
        ):
            evidence = copy.deepcopy(self.evidence)
            target = evidence["candidate_ci"]
            for key in path[:-1]:
                target = target[key]
            target[path[-1]] = replacement
            self.assertTrue(self.validate(evidence=evidence), path)

    def test_rejects_fabricated_or_misdirected_independent_review(self) -> None:
        for field, replacement in (
            ("status", "pass"),
            ("reviewed_on", "2026-07-29"),
            ("findings", []),
            ("outcome", "pass"),
        ):
            review = copy.deepcopy(self.independent_review)
            review[field] = replacement
            self.assertTrue(
                self.validate(independent_review=review),
                field,
            )

        review = copy.deepcopy(self.independent_review)
        review["review_scope"]["candidate_revision"] = "0" * 40
        self.assertTrue(
            self.validate(independent_review=review),
            "candidate_revision",
        )


if __name__ == "__main__":
    unittest.main()
