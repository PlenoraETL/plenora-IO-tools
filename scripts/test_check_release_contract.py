"""Regression tests for the component-RC provenance gate."""

from __future__ import annotations

import copy
import unittest

from scripts.check_release_contract import (
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
    validate_workspace_versions,
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

    def test_rejects_release_version_or_tag_form_drift(self) -> None:
        for path, replacement in (
            (("component_version",), "0.1.0-rc.3"),
            (("release_tag", "name"), "v0.1.0-rc.3"),
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

    def test_rejects_rollback_after_technical_freeze(self) -> None:
        provenance = copy.deepcopy(self.provenance)
        provenance["freeze_status"] = "pre_freeze"
        self.assertTrue(self.validate(provenance=provenance))

    def test_allows_component_rc_tag_while_review_is_open(self) -> None:
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
            (("library_line_coverage", "covered"), 0),
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
