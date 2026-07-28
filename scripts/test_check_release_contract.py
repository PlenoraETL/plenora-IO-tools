"""Regression tests for the component-RC provenance gate."""

from __future__ import annotations

import copy
import unittest

from scripts.check_release_contract import (
    CORPUS_SCHEMA,
    GEOMETRY_SOURCE,
    PROVENANCE,
    SYSTEM_GATE,
    load_json,
    validate_documents,
)


class ReleaseContractTests(unittest.TestCase):
    def setUp(self) -> None:
        self.provenance = load_json(PROVENANCE)
        self.system_gate = load_json(SYSTEM_GATE)
        self.corpus_schema = load_json(CORPUS_SCHEMA)
        self.geometry_source = GEOMETRY_SOURCE.read_text(encoding="utf-8")

    def validate(self, provenance=None, system_gate=None, corpus_schema=None) -> list[str]:
        return validate_documents(
            provenance if provenance is not None else self.provenance,
            system_gate if system_gate is not None else self.system_gate,
            corpus_schema if corpus_schema is not None else self.corpus_schema,
            self.geometry_source,
        )

    def test_repository_manifests_are_consistent(self) -> None:
        self.assertEqual(self.validate(), [])

    def test_rejects_system_rc_claim(self) -> None:
        provenance = copy.deepcopy(self.provenance)
        provenance["claims"]["system_rc"] = True
        self.assertTrue(self.validate(provenance=provenance))

    def test_rejects_unrecorded_candidate_status(self) -> None:
        provenance = copy.deepcopy(self.provenance)
        provenance["icd"]["conformance_claim"] = "full"
        self.assertTrue(self.validate(provenance=provenance))

    def test_rejects_wire_version_drift(self) -> None:
        provenance = copy.deepcopy(self.provenance)
        provenance["wire_contract_version"] = 2
        self.assertTrue(self.validate(provenance=provenance))

    def test_rejects_system_gate_promotion_without_new_review(self) -> None:
        gate = copy.deepcopy(self.system_gate)
        gate["status"] = "satisfied"
        self.assertTrue(self.validate(system_gate=gate))

    def test_rejects_incomplete_shared_corpus_schema(self) -> None:
        schema = copy.deepcopy(self.corpus_schema)
        del schema["properties"]["cases"]["items"]["properties"]["sha256"]
        self.assertTrue(self.validate(corpus_schema=schema))


if __name__ == "__main__":
    unittest.main()
