#!/usr/bin/env python3
"""Validate the component-RC contract provenance without network access."""

from __future__ import annotations

import json
import re
import sys
import tomllib
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
PROVENANCE = ROOT / "release" / "contract-provenance.json"
SYSTEM_GATE = ROOT / "release" / "system-rc-gate.json"
FREEZE_READINESS = ROOT / "release" / "freeze-readiness.json"
EVIDENCE = (
    ROOT / "release" / "evidence" / "technical-freeze-v0.1.0-rc.4.json"
)
INDEPENDENT_REVIEW = ROOT / "release" / "independent-review.json"
RC3_DEVELOPMENT = ROOT / "release" / "rc3-development.json"
RC4_DEVELOPMENT = ROOT / "release" / "rc4-development.json"
CORPUS_SCHEMA = ROOT / "fuzz" / "shared-corpus-manifest.schema.json"
CORPUS_MANIFEST = ROOT / "fuzz" / "shared-corpus" / "manifest.json"
GEOMETRY_SOURCE = ROOT / "crates" / "plenora-io-model" / "src" / "geometry.rs"
WORKSPACE_MANIFEST = ROOT / "Cargo.toml"
WORKSPACE_LOCK = ROOT / "Cargo.lock"
WORKSPACE_CRATE_MANIFESTS = tuple(sorted((ROOT / "crates").glob("*/Cargo.toml")))
DETACHED_LOCKFILES = (
    ROOT / "fuzz" / "Cargo.lock",
)
FORBIDDEN_SYSTEM_HARNESS_PATHS = (
    ROOT / "conformance" / "three-component-chain",
    ROOT / "scripts" / "run_three_component_chain.py",
)
EXPECTED_ICD_TAG = "v2.0-rc8"
EXPECTED_ICD_REVISION = "62b12e3496466d2c908dac3cc098640b99b52e21"
EXPECTED_RC_BASELINE = "ea0de79677e8fc794d96ac3d95c5bc2c6e30358c"
EXPECTED_FUZZ_IO_REVISION = "1c37fb5d525647b264ce977e26fc07b346bb7914"
EXPECTED_IO_CANDIDATE = "dc85f5163860bd16c4cf0bfa1066276980d38e8c"
EXPECTED_CANDIDATE_STATE = "component_rc_verified_internally"
EXPECTED_COMPONENT_VERSION = "0.1.0-rc.4"
EXPECTED_WORKSPACE_VERSION = "0.1.0-rc.4"
EXPECTED_RELEASE_TAG = "v0.1.0-rc.4"
EXPECTED_CANDIDATE_CI_RUN = 30605882153
EXPECTED_RELEASE_DECISION_REVISION = (
    "322ff57abd872f728d3f4e10c50c800ad39fa29c"
)
EXPECTED_RELEASE_DECISION_CI_RUN = 30606393196
EXPECTED_PRE_TAG_REVISION = None
EXPECTED_PRE_TAG_CI_RUN = None
EXPECTED_COVERAGE_ARTIFACT = {
    "id": 8783562020,
    "name": "rust-coverage-lcov",
    "size_bytes": 103610,
    "sha256": "a8c262e8f3d330f70c1c820f9e355a3a349bd9bb86a0d29193e1151b365b7d24",
    "digest_source": "github_actions_artifact_api",
}
EXPECTED_WINDOWS_FILEGDB_BENCHMARK_ARTIFACT = {
    "id": 8783600878,
    "name": "windows-filegdb-narrow-benchmark",
    "size_bytes": 774,
    "sha256": "5b4f56c896813e9d72227f53094b7e72625ae23f43949d838a4982b3dfb89a6e",
    "digest_source": "github_actions_artifact_api",
}
EXPECTED_LIBRARY_COVERAGE = {"minimum_percent": 80, "gate_result": "pass"}
EXPECTED_LIBRARY_COVERAGE_SOURCE = "github_actions_coverage_gate"
EXPECTED_FREEZE_SCOPE = {
    "kind": "technical_baseline",
    "baseline_revision": EXPECTED_IO_CANDIDATE,
    "frozen_on": "2026-07-30",
    "verification_claim": "verified_internally",
    "independent_review_status": "not_performed",
    "release_tag_authorized": True,
    "assurance_promotion_authorized": False,
}
EXPECTED_RELEASE_DECISION = {
    "authorized": True,
    "scope": "component_rc",
    "verification_claim": "verified_internally",
    "independent_review_required": False,
    "independent_review_status": "not_performed",
    "independently_verified_claim_authorized": False,
    "release_tag_created": False,
    "decision_record": (
        "docs/assurance/CHANGE_IMPACT_2026-07-30_RC4_RELEASE_DECISION.md"
    ),
    "decision_revision": EXPECTED_RELEASE_DECISION_REVISION,
    "decision_ci_run": EXPECTED_RELEASE_DECISION_CI_RUN,
}
EXPECTED_RELEASE_TAG_RECORD = {
    "name": EXPECTED_RELEASE_TAG,
    "version": EXPECTED_COMPONENT_VERSION,
    "tag_form": "annotated_unsigned",
    "status": "pending_pre_tag_ci",
    "created_on": None,
    "candidate_revision": EXPECTED_IO_CANDIDATE,
    "verification_claim": "verified_internally",
    "independent_review_status": "not_performed",
    "pre_tag_revision": EXPECTED_PRE_TAG_REVISION,
    "pre_tag_ci_run": EXPECTED_PRE_TAG_CI_RUN,
    "decision_record": (
        "docs/assurance/CHANGE_IMPACT_2026-07-30_RC4_RELEASE_DECISION.md"
    ),
}
EXPECTED_FREEZE_DECISION = {
    "status": "technical_baseline_frozen",
    "baseline_revision": EXPECTED_IO_CANDIDATE,
    "frozen_on": "2026-07-30",
    "verification_claim": "verified_internally",
    "independent_review": False,
    "release_tag_created": False,
    "decision_record": (
        "docs/assurance/CHANGE_IMPACT_2026-07-30_RC4_RELEASE_DECISION.md"
    ),
}
EXPECTED_REVIEW_SCOPE = {
    "component": "plenora-IO-tools",
    "comparison_base_revision": EXPECTED_RC_BASELINE,
    "candidate_revision": EXPECTED_IO_CANDIDATE,
    "freeze_record_revision": EXPECTED_PRE_TAG_REVISION,
    "evidence_revision": EXPECTED_PRE_TAG_REVISION,
    "icd_revision": EXPECTED_ICD_REVISION,
    "packet": "docs/assurance/INDEPENDENT_REVIEW_PACKET.md",
}
EXPECTED_REVIEW_ELIGIBILITY = {
    "requires_human_person": True,
    "different_from_all_change_authors_and_coauthors": True,
    "owner_allowed_only_when_not_author_or_coauthor": True,
    "automation_or_author_self_review_accepted": False,
}
EXPECTED_PENDING_REVIEWER = {
    "name": None,
    "affiliation": None,
    "contact_or_identity_reference": None,
    "eligibility_attestation": None,
}
EXPECTED_REVIEW_COMPLETION_FIELDS = [
    "reviewer.name",
    "reviewer.affiliation",
    "reviewer.contact_or_identity_reference",
    "reviewer.eligibility_attestation",
    "reviewed_on",
    "commands_executed",
    "findings",
    "outcome",
]
RC3_COMPONENT_VERSION = "0.1.0-rc.3"
RC3_CANDIDATE_REVISION = "3f3562a4707995549ff5eb8dc03f9e37f2cde355"
RC3_CANDIDATE_CI_RUN = 30500304709
RC3_RELEASE_DECISION_REVISION = "6868990461f7ef880258f67985528dc29b0564a0"
RC3_RELEASE_DECISION_CI_RUN = 30501136176
RC3_RELEASE_TAG = "v0.1.0-rc.3"
RC3_PRE_TAG_REVISION = "ab330f8dfbcc7235c418e3e04f988317d3070525"
RC3_PRE_TAG_CI_RUN = 30501904391
EXPECTED_DATABASE_REPLAY_REVISION = "ef18e80c798126f872fd366c36ee96a029598958"
EXPECTED_SYSTEM_REVISIONS = {
    "plenora-IO-tools": EXPECTED_IO_CANDIDATE,
    "plenora-data-tools": "97e48ba469f9f55a2cc83e9598d72899c29e2be6",
    "plenora-database-tools": "2588523bf6a4ad57e62ae3d44e9f58025c55a913",
}
EXPECTED_FUZZ_STATE = "long_campaign_completed_no_findings"
REQUIRED_CANDIDATE_SECTIONS = {
    "§2",
    "§3.4/R3.4.1",
    "§4.3.1-§4.3.3",
    "§9",
    "§11",
}
SHA = re.compile(r"^[0-9a-f]{40}$")
WIRE_VERSION = re.compile(
    r'pub const PLENORA_CONTRACT_VERSION:\s*&str\s*=\s*"([0-9]+)";'
)


def load_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{path}: radice JSON non object")
    return value


def load_toml(path: Path) -> dict[str, Any]:
    value = tomllib.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{path}: radice TOML non table")
    return value


def validate_workspace_versions(
    workspace_manifest: dict[str, Any],
    crate_manifests: list[dict[str, Any]],
    lockfile: dict[str, Any],
    detached_lockfiles: list[dict[str, Any]] | None = None,
) -> list[str]:
    errors: list[str] = []
    workspace_version = (
        workspace_manifest.get("workspace", {}).get("package", {}).get("version")
    )
    if workspace_version != EXPECTED_WORKSPACE_VERSION:
        errors.append("workspace: versione RC centrale inattesa")

    crate_names: set[str] = set()
    for manifest in crate_manifests:
        package = manifest.get("package", {})
        name = package.get("name")
        if not isinstance(name, str):
            errors.append("workspace: crate senza nome")
            continue
        crate_names.add(name)
        if package.get("version") != {"workspace": True}:
            errors.append(f"workspace: {name} non eredita la versione centrale")

    locked_versions = {
        package.get("name"): package.get("version")
        for package in lockfile.get("package", [])
        if isinstance(package, dict) and package.get("name") in crate_names
    }
    for crate_name in sorted(crate_names):
        if locked_versions.get(crate_name) != EXPECTED_WORKSPACE_VERSION:
            errors.append(f"Cargo.lock: versione RC inattesa per {crate_name}")

    for index, detached_lockfile in enumerate(detached_lockfiles or []):
        detached_versions = {
            package.get("name"): package.get("version")
            for package in detached_lockfile.get("package", [])
            if isinstance(package, dict) and package.get("name") in crate_names
        }
        for crate_name, version in sorted(detached_versions.items()):
            if version != EXPECTED_WORKSPACE_VERSION:
                errors.append(
                    "lockfile detached "
                    f"{index}: versione RC inattesa per {crate_name}"
                )

    return errors


def validate_rc3_development(document: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    if document.get("manifest_version") != 1:
        errors.append("rc3-development: manifest_version inattesa")
    if document.get("component") != "plenora-IO-tools":
        errors.append("rc3-development: componente inatteso")
    if document.get("component_version") != RC3_COMPONENT_VERSION:
        errors.append("rc3-development: versione release inattesa")
    if document.get("status") != "component_rc_tagged":
        errors.append("rc3-development: stato finale inatteso")
    if document.get("baseline_release") != {
        "tag": "v0.1.0-rc.2",
        "target_revision": "f47bf4605b248d127205e49a7e6ebd2a0984a83f",
        "immutable": True,
    }:
        errors.append("rc3-development: baseline RC2 inattesa")
    if document.get("scope") != "component_only":
        errors.append("rc3-development: perimetro non limitato al componente")
    if document.get("system_qualification_ownership") != "external":
        errors.append("rc3-development: qualifica di sistema non dichiarata esterna")
    if document.get("scope_decision") != (
        "docs/assurance/CHANGE_IMPACT_2026-07-30_RC3_CRS_SCOPE.md"
    ):
        errors.append("rc3-development: decisione di perimetro inattesa")
    if document.get("candidate") != {
        "implementation_revision": RC3_CANDIDATE_REVISION,
        "candidate_ci_run": RC3_CANDIDATE_CI_RUN,
        "release_decision_revision": RC3_RELEASE_DECISION_REVISION,
        "release_decision_ci_run": RC3_RELEASE_DECISION_CI_RUN,
        "release_tag": RC3_RELEASE_TAG,
        "release_tag_status": "created",
        "pre_tag_revision": RC3_PRE_TAG_REVISION,
        "pre_tag_ci_run": RC3_PRE_TAG_CI_RUN,
    }:
        errors.append("rc3-development: candidato pre-tag inatteso")
    workstreams = document.get("workstreams", {})
    if set(workstreams) != {
        "long_fuzz_campaign",
        "independent_review",
        "canonical_wkb_types",
        "crs_inconsistency_read_declaration",
        "icd_ratification_alignment",
    }:
        errors.append("rc3-development: workstream RC3 inattesi")
    if workstreams.get("long_fuzz_campaign") != "completed_no_findings":
        errors.append("rc3-development: stato campagna fuzz lunga inatteso")
    if workstreams.get("canonical_wkb_types") != "implemented_local_gates_passed":
        errors.append("rc3-development: stato codec canonico inatteso")
    if workstreams.get("crs_inconsistency_read_declaration") != (
        "implemented_targeted_gates_passed"
    ):
        errors.append("rc3-development: stato dichiarazione CRS inatteso")
    if workstreams.get("independent_review") != (
        "assurance_attribute_open_non_blocking"
    ):
        errors.append("rc3-development: review indipendente riclassificata come gate")
    if workstreams.get("icd_ratification_alignment") != (
        "owner_decision_open_not_component_code_gate"
    ):
        errors.append("rc3-development: ratifica ICD riclassificata come gate esterno")
    if document.get("deferred_to_rc4") != {
        "streaming_and_cancellation": "blocked_prerequisites_revalidated",
        "openfilegdb_native_pushdown": "design_constraint_open",
        "filegdb_windows_and_filesystem_matrix": (
            "bundled_candidate_rejected_performance_veto_environment_open"
        ),
    }:
        errors.append("rc3-development: backlog RC4 inatteso")
    if document.get("diagnostic_fuzz_run") != {
        "library_baseline_revision": "f8a89170785c938a9105deae6cc479576abb969a",
        "baseline_ci_run": 30447756574,
        "run_id": "20260729T113223Z",
        "duration_seconds": 3600,
        "libfuzzer_executions": 271369231,
        "structured_iterations": 5720360000,
        "findings": 0,
        "crash_artifacts": 0,
        "container_exit_code": 0,
        "provenance_status": "not_release_evidence_uncommitted_harness",
        "uncommitted_harness_sha256": {
            "fuzz/fuzz_targets/from_wkb.rs": (
                "759421a3d6249cf6898f3f4ca58ef99c"
                "975cd493b7f42c320716276cfc741ee9"
            ),
            "fuzz/fuzz_targets/wkt_parse.rs": (
                "6af6c2adeaa8efe5db0c21f2d987fdd2"
                "6ca5d6f6b8ae0d71fd9f39edff16c9af"
            ),
        },
    }:
        errors.append("rc3-development: record diagnostico fuzz inatteso")
    if document.get("fuzz_campaign_evidence") != {
        "baseline_revision": "2353e32da15cf25537a79c3a7dd507054c013764",
        "baseline_ci_run": 30457744328,
        "run_id": "20260729T181308Z",
        "duration_seconds": 3600,
        "container_image": (
            "sha256:bf24b399447ea3b8cd68c4d248cf42bc"
            "6bc0f9a7e895c50f2b55ab58cbfbbe65"
        ),
        "libfuzzer_executions": 210118046,
        "new_corpus_units": 16794,
        "structured_iterations": 5705840000,
        "peak_rss_mb": 530,
        "findings": 0,
        "crash_artifacts": 0,
        "container_exit_code": 0,
        "working_tree_clean_before_and_after": True,
    }:
        errors.append("rc3-development: evidenza fuzz lunga inattesa")
    if document.get("claims") != {
        "component_rc": True,
        "system_rc": False,
        "avionic_certification": False,
    }:
        errors.append("rc3-development: claim prematuri")
    return errors


def validate_rc4_development(document: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    if document != {
        "manifest_version": 1,
        "component": "plenora-IO-tools",
        "component_version": EXPECTED_WORKSPACE_VERSION,
        "status": "candidate_frozen_pending_pre_tag_ci",
        "baseline_release": {
            "tag": RC3_RELEASE_TAG,
            "target_revision": "ea0de79677e8fc794d96ac3d95c5bc2c6e30358c",
            "implementation_revision": RC3_CANDIDATE_REVISION,
            "immutable": True,
        },
        "program_record": (
            "docs/assurance/CHANGE_IMPACT_2026-07-30_RC4_PROGRAM.md"
        ),
        "scope": "component_only",
        "system_qualification_ownership": "external",
        "candidate": {
            "implementation_revision": EXPECTED_IO_CANDIDATE,
            "candidate_ci_run": EXPECTED_CANDIDATE_CI_RUN,
            "release_decision_revision": EXPECTED_RELEASE_DECISION_REVISION,
            "release_decision_ci_run": EXPECTED_RELEASE_DECISION_CI_RUN,
            "release_tag": EXPECTED_RELEASE_TAG,
            "release_tag_status": "pending_pre_tag_ci",
        },
        "workstreams": {
            "xlsx_bounded_spool_streaming": "implemented_benchmark_passed",
            "kml_event_streaming": "implemented_benchmark_passed",
            "dxf_progressive_reader": "implemented_governed_fork_benchmark_passed",
            "openfilegdb_native_pushdown": (
                "implemented_governed_fork_benchmark_passed"
            ),
            "filegdb_windows_and_filesystem_matrix": (
                "implemented_native_matrix_and_benchmark_passed"
            ),
        },
        "non_code_dependencies": {
            "combined_read_write_boundary_crs_policy": "owner_decision_open",
            "reader_loss_cli_observability": "external_contracts_follow_up",
            "icd_ratification_alignment": (
                "owner_decision_open_not_component_code_gate"
            ),
        },
        "claims": {
            "component_rc": False,
            "system_rc": False,
            "avionic_certification": False,
        },
    }:
        errors.append("rc4-development: programma o claim inattesi")
    return errors


def validate_documents(
    provenance: dict[str, Any],
    system_gate: dict[str, Any],
    freeze_readiness: dict[str, Any],
    evidence: dict[str, Any],
    independent_review: dict[str, Any],
    corpus_schema: dict[str, Any],
    corpus_manifest: dict[str, Any],
    geometry_source: str,
) -> list[str]:
    errors: list[str] = []
    icd = provenance.get("icd", {})
    claims = provenance.get("claims", {})
    fuzz = provenance.get("fuzz_coordination", {})

    if provenance.get("manifest_version") != 1:
        errors.append("contract-provenance: manifest_version deve essere 1")
    if provenance.get("component") != "plenora-IO-tools":
        errors.append("contract-provenance: componente inatteso")
    if provenance.get("release_kind") != "component_rc":
        errors.append("contract-provenance: release_kind deve essere component_rc")
    if provenance.get("component_version") != EXPECTED_COMPONENT_VERSION:
        errors.append("contract-provenance: versione componente inattesa")
    if provenance.get("freeze_status") != "frozen":
        errors.append("contract-provenance: baseline tecnica non congelata")
    if not SHA.fullmatch(str(provenance.get("implementation_revision", ""))):
        errors.append("contract-provenance: implementation_revision non è uno SHA completo")
    elif provenance.get("implementation_revision") != EXPECTED_IO_CANDIDATE:
        errors.append("contract-provenance: revisione candidata IO inattesa")
    if provenance.get("candidate_state") != EXPECTED_CANDIDATE_STATE:
        errors.append("contract-provenance: stato candidato inatteso")
    if provenance.get("freeze_scope") != EXPECTED_FREEZE_SCOPE:
        errors.append("contract-provenance: perimetro del freeze inatteso")
    if provenance.get("release_decision") != EXPECTED_RELEASE_DECISION:
        errors.append("contract-provenance: decisione release interna inattesa")
    if provenance.get("release_tag") != EXPECTED_RELEASE_TAG_RECORD:
        errors.append("contract-provenance: record del tag RC inatteso")

    if icd.get("tag") != EXPECTED_ICD_TAG:
        errors.append("contract-provenance: tag ICD inatteso")
    if icd.get("revision") != EXPECTED_ICD_REVISION:
        errors.append("contract-provenance: revisione ICD inattesa")
    if icd.get("tag_form") != "annotated_unsigned":
        errors.append("contract-provenance: la firma assente del tag deve restare esplicita")
    if icd.get("normative_status") != "partially_ratified":
        errors.append("contract-provenance: stato normativo deve essere partially_ratified")
    if icd.get("conformance_claim") != "candidate_implementation_only":
        errors.append("contract-provenance: vietata una dichiarazione di conformità piena")

    sections = provenance.get("candidate_sections_adopted", [])
    observed_sections = {
        section.get("section")
        for section in sections
        if isinstance(section, dict) and section.get("status") == "proposal"
    }
    missing_sections = REQUIRED_CANDIDATE_SECTIONS - observed_sections
    if missing_sections:
        errors.append(
            "contract-provenance: sezioni candidate mancanti: "
            + ", ".join(sorted(missing_sections))
        )

    deviations = provenance.get("declared_deviations", [])
    if not any(
        isinstance(deviation, dict)
        and deviation.get("rule") == "§15.4 step 1 / DER-ICD-002"
        and deviation.get("status") == "active"
        and deviation.get("exit_condition")
        for deviation in deviations
    ):
        errors.append("contract-provenance: deroga emissione §15.4 non dichiarata")

    if claims != {
        "component_rc": False,
        "system_rc": False,
        "avionic_certification": False,
    }:
        errors.append("contract-provenance: claims RC/sistema/avionica non fail-closed")

    if fuzz.get("state") != EXPECTED_FUZZ_STATE:
        errors.append("contract-provenance: stato fuzz coordinato inatteso")
    if fuzz.get("io_tools_revision") != EXPECTED_FUZZ_IO_REVISION:
        errors.append("contract-provenance: revisione IO-tools fuzz inattesa")
    if fuzz.get("database_tools_revision") != EXPECTED_DATABASE_REPLAY_REVISION:
        errors.append("contract-provenance: revisione database-tools fuzz inattesa")
    if fuzz.get("corpus_cases") != 18 or fuzz.get("cross_replay_status") != "pass":
        errors.append("contract-provenance: replay dei 18 casi non registrato come pass")
    if fuzz.get("unclassified_differences") != 0:
        errors.append("contract-provenance: divergenze fuzz non classificate")
    if fuzz.get("long_campaign") != {
        "baseline_revision": "2353e32da15cf25537a79c3a7dd507054c013764",
        "baseline_ci_run": 30457744328,
        "duration_seconds": 3600,
        "libfuzzer_executions": 210118046,
        "structured_iterations": 5705840000,
        "findings": 0,
        "working_tree_clean_before_and_after": True,
    }:
        errors.append("contract-provenance: campagna fuzz lunga inattesa")

    version_match = WIRE_VERSION.search(geometry_source)
    if version_match is None:
        errors.append("geometry.rs: PLENORA_CONTRACT_VERSION non rilevabile")
    elif int(version_match.group(1)) != provenance.get("wire_contract_version"):
        errors.append("contract-provenance: wire version diversa dal codice")

    gate_icd = system_gate.get("icd", {})
    if system_gate.get("manifest_version") != 1:
        errors.append("system-rc-gate: manifest_version deve essere 1")
    if system_gate.get("status") != "not_satisfied":
        errors.append("system-rc-gate: una RC di sistema non può essere dichiarata")
    if gate_icd.get("tag") != EXPECTED_ICD_TAG:
        errors.append("system-rc-gate: tag ICD diverso dalla provenienza")
    if gate_icd.get("revision") != EXPECTED_ICD_REVISION:
        errors.append("system-rc-gate: revisione ICD diversa dalla provenienza")

    component_revisions = {
        component.get("name"): component.get("revision")
        for component in system_gate.get("components", [])
        if isinstance(component, dict) and SHA.fullmatch(str(component.get("revision", "")))
    }
    if component_revisions != EXPECTED_SYSTEM_REVISIONS:
        errors.append("system-rc-gate: revisioni complete dei tre componenti richieste")
    if system_gate.get("ownership") != "external_system_qualification":
        errors.append("system-rc-gate: ownership della qualifica di sistema inattesa")
    if system_gate.get("external_owner") != "plenora-contracts/conformance":
        errors.append("system-rc-gate: owner esterno della qualifica inatteso")
    if system_gate.get("component_repository_harness") != "not_present":
        errors.append("system-rc-gate: harness cross-component non ammesso nel repository IO")
    if not system_gate.get("open_blockers"):
        errors.append("system-rc-gate: stato aperto senza blocker dichiarati")

    if corpus_schema.get("$schema") != "https://json-schema.org/draft/2020-12/schema":
        errors.append("shared corpus: JSON Schema draft inatteso")
    cases = corpus_schema.get("properties", {}).get("cases", {})
    if cases.get("type") != "array":
        errors.append("shared corpus: cases deve essere un array")
    case_properties = cases.get("items", {}).get("properties", {})
    for required_property in (
        "sha256",
        "path",
        "dialect",
        "expectation",
        "expected_error_category",
        "known_difference",
        "invariants",
    ):
        if required_property not in case_properties:
            errors.append(
                f"shared corpus: proprietà caso assente: {required_property}"
            )

    if corpus_manifest.get("corpus_id") != fuzz.get("corpus_id"):
        errors.append("shared corpus: corpus_id diverso dalla provenienza")
    manifest_cases = corpus_manifest.get("cases", [])
    if not isinstance(manifest_cases, list) or len(manifest_cases) != 18:
        errors.append("shared corpus: sono richiesti esattamente 18 casi")
    if corpus_manifest.get("producer_revisions") != {
        "plenora-IO-tools": EXPECTED_FUZZ_IO_REVISION,
        "plenora-database-tools": EXPECTED_DATABASE_REPLAY_REVISION,
    }:
        errors.append("shared corpus: revisioni dei producer inattese")

    if freeze_readiness.get("status") != "pre_tag_pending_ci":
        errors.append("freeze readiness: stato RC interno inatteso")
    if freeze_readiness.get("freeze_scope") != "technical_baseline_only":
        errors.append("freeze readiness: perimetro tecnico non dichiarato")
    if freeze_readiness.get("release_authorized") is not False:
        errors.append("freeze readiness: autorizzazione dichiarata prima della CI pre-tag")
    readiness_gates = freeze_readiness.get("gates", {})
    expected_gates = {
        "candidate_code_complete",
        "local_workspace_tests",
        "local_safety_clippy",
        "candidate_revision_committed",
        "candidate_ci",
        "technical_baseline_frozen",
        "verification_claim_declared",
        "component_scope_declared",
        "release_decision_recorded",
        "release_decision_ci",
        "pre_tag_ci",
    }
    if set(readiness_gates) != expected_gates:
        errors.append("freeze readiness: insieme dei gate obbligatori inatteso")
    for gate in expected_gates - {"pre_tag_ci"}:
        if readiness_gates.get(gate) is not True:
            errors.append(f"freeze readiness: gate obbligatorio non soddisfatto: {gate}")
    if readiness_gates.get("pre_tag_ci") is not False:
        errors.append("freeze readiness: CI pre-tag dichiarata prima dell'esecuzione")
    if freeze_readiness.get("candidate_revision") != EXPECTED_IO_CANDIDATE:
        errors.append("freeze readiness: SHA candidato inatteso")
    assurance_attributes = freeze_readiness.get("assurance_attributes", {})
    if assurance_attributes != {
        "verification_claim": "verified_internally",
        "independent_review": False,
        "independent_review_status": "pending_eligible_reviewer",
        "independently_verified_claim_authorized": False,
        "release_tag_created": False,
        "release_tag_name": EXPECTED_RELEASE_TAG,
        "release_tag_form": "annotated_unsigned",
        "release_tag_status": "pending_pre_tag_ci",
    }:
        errors.append("freeze readiness: attributi assurance inattesi")
    if "independent_review" in readiness_gates:
        errors.append("freeze readiness: independent_review non deve essere un gate RC")

    if evidence.get("status") != "technical_freeze_pre_tag":
        errors.append("evidence: stato del freeze tecnico inatteso")
    if evidence.get("baseline_revision") != EXPECTED_RC_BASELINE:
        errors.append("evidence: baseline IO inattesa")
    if evidence.get("candidate_revision") != EXPECTED_IO_CANDIDATE:
        errors.append("evidence: revisione candidata inattesa")
    if evidence.get("freeze_decision") != EXPECTED_FREEZE_DECISION:
        errors.append("evidence: decisione di freeze inattesa")
    if evidence.get("independent_review_record") != {
        "path": "release/independent-review.json",
        "status": "pending_eligible_reviewer",
        "blocks_component_rc_release": False,
    }:
        errors.append("evidence: riferimento alla review indipendente inatteso")
    if evidence.get("release_decision") != {
        "status": "authorized_as_verified_internally_component_rc",
        "verification_claim": "verified_internally",
        "independent_review": False,
        "independently_verified_claim_authorized": False,
        "release_tag_created": False,
        "decision_record": (
            "docs/assurance/CHANGE_IMPACT_2026-07-30_RC4_RELEASE_DECISION.md"
        ),
        "decision_revision": EXPECTED_RELEASE_DECISION_REVISION,
        "ci": {
            "run_id": EXPECTED_RELEASE_DECISION_CI_RUN,
            "url": (
                "https://github.com/PlenoraETL/plenora-IO-tools/actions/runs/"
                f"{EXPECTED_RELEASE_DECISION_CI_RUN}"
            ),
            "head_revision": EXPECTED_RELEASE_DECISION_REVISION,
            "result": "pass",
            "jobs": ["rust", "coverage", "windows", "macos-publish"],
        },
        "release_tag": {
            key: value
            for key, value in EXPECTED_RELEASE_TAG_RECORD.items()
            if key != "decision_record"
        },
    }:
        errors.append("evidence: decisione RC interna inattesa")
    candidate_ci = evidence.get("candidate_ci", {})
    if candidate_ci.get("head_revision") != EXPECTED_IO_CANDIDATE:
        errors.append("evidence: revisione CI candidata inattesa")
    if candidate_ci.get("run_id") != EXPECTED_CANDIDATE_CI_RUN:
        errors.append("evidence: run CI candidato inatteso")
    if candidate_ci.get("url") != (
        "https://github.com/PlenoraETL/plenora-IO-tools/actions/runs/"
        f"{EXPECTED_CANDIDATE_CI_RUN}"
    ):
        errors.append("evidence: URL CI candidata inatteso")
    if candidate_ci.get("jobs") != [
        "rust",
        "coverage",
        "windows",
        "macos-publish",
    ]:
        errors.append("evidence: matrice job CI candidata inattesa")
    if candidate_ci.get("result") != "pass":
        errors.append("evidence: CI candidata non verde")
    if candidate_ci.get("coverage_artifact") != EXPECTED_COVERAGE_ARTIFACT:
        errors.append("evidence: digest artifact coverage inatteso")
    if (
        candidate_ci.get("windows_filegdb_benchmark_artifact")
        != EXPECTED_WINDOWS_FILEGDB_BENCHMARK_ARTIFACT
    ):
        errors.append("evidence: digest artifact benchmark Windows inatteso")
    coverage = candidate_ci.get("library_line_coverage", {})
    if coverage != EXPECTED_LIBRARY_COVERAGE:
        errors.append("evidence: coverage candidata inattesa")
    if (
        candidate_ci.get("library_line_coverage_source")
        != EXPECTED_LIBRARY_COVERAGE_SOURCE
    ):
        errors.append("evidence: fonte coverage candidata inattesa")
    replay = (
        evidence.get("candidate_local_verification", {})
        .get("shared_wkb_ewkb_replay", {})
    )
    if replay.get("result") != "pass" or replay.get("unclassified_differences") != 0:
        errors.append("evidence: replay differenziale non verde")

    if independent_review.get("review_record_version") != 1:
        errors.append("independent review: versione record inattesa")
    if independent_review.get("status") != "pending_eligible_reviewer":
        errors.append("independent review: stato pendente non preservato")
    if independent_review.get("review_scope") != EXPECTED_REVIEW_SCOPE:
        errors.append("independent review: perimetro o revisioni inattesi")
    if independent_review.get("eligibility") != EXPECTED_REVIEW_ELIGIBILITY:
        errors.append("independent review: criteri di eleggibilità inattesi")
    if independent_review.get("reviewer") != EXPECTED_PENDING_REVIEWER:
        errors.append("independent review: revisore precompilato senza review")
    for pending_field in ("reviewed_on", "commands_executed", "findings", "outcome"):
        if independent_review.get(pending_field) is not None:
            errors.append(
                f"independent review: campo compilato prematuramente: {pending_field}"
            )
    if (
        independent_review.get("required_completion_fields")
        != EXPECTED_REVIEW_COMPLETION_FIELDS
    ):
        errors.append("independent review: campi obbligatori inattesi")
    if independent_review.get("release_effect") != {
        "blocks_component_rc_release": False,
        "component_rc_release_authorized": False,
        "independently_verified_claim_authorized": False,
        "release_tag_created": False,
    }:
        errors.append("independent review: separazione da release interna inattesa")

    verification_claim = (
        provenance.get("release_decision", {}).get("verification_claim")
    )
    independent_review_complete = (
        independent_review.get("status") == "completed"
        and independent_review.get("outcome")
        in {"pass", "pass_with_non_blocking_findings"}
    )
    if verification_claim == "verified_independently" and not independent_review_complete:
        errors.append(
            "assurance: claim verified_independently senza review completata"
        )

    return errors


def main() -> int:
    errors: list[str] = []
    for path in FORBIDDEN_SYSTEM_HARNESS_PATHS:
        if path.exists():
            errors.append(
                f"system qualification harness non ammesso nel repository IO: {path}"
            )
    required_files = [
        PROVENANCE,
        SYSTEM_GATE,
        FREEZE_READINESS,
        EVIDENCE,
        INDEPENDENT_REVIEW,
        RC3_DEVELOPMENT,
        RC4_DEVELOPMENT,
        WORKSPACE_MANIFEST,
        WORKSPACE_LOCK,
        *WORKSPACE_CRATE_MANIFESTS,
        *DETACHED_LOCKFILES,
        ROOT / "docs" / "assurance" / "RELEASE_CANDIDATE_SCOPE.md",
        ROOT / "docs" / "assurance" / "SYSTEM_RC_GATE.md",
        ROOT / "docs" / "assurance" / "WKB_EWKB_FUZZ_COORDINATION.md",
        ROOT
        / "docs"
        / "assurance"
        / "CHANGE_IMPACT_2026-07-28_RC_PROVENANCE_FUZZ_COORDINATION.md",
        ROOT
        / "docs"
        / "assurance"
        / "CHANGE_IMPACT_2026-07-28_EIGHT_POINT_COMPLETION.md",
        ROOT
        / "docs"
        / "assurance"
        / "CHANGE_IMPACT_2026-07-29_CANDIDATE_REBASELINE.md",
        ROOT
        / "docs"
        / "assurance"
        / "CHANGE_IMPACT_2026-07-29_TECHNICAL_FREEZE.md",
        ROOT
        / "docs"
        / "assurance"
        / "INDEPENDENT_REVIEW_PACKET.md",
        ROOT
        / "docs"
        / "assurance"
        / "CHANGE_IMPACT_2026-07-29_INDEPENDENT_REVIEW_PACKET.md",
        ROOT
        / "docs"
        / "assurance"
        / "CHANGE_IMPACT_2026-07-29_INTERNAL_RC_RELEASE.md",
        ROOT
        / "docs"
        / "assurance"
        / "CHANGE_IMPACT_2026-07-29_RC_VERSION_AND_TAG.md",
        ROOT
        / "docs"
        / "assurance"
        / "CHANGE_IMPACT_2026-07-29_RC2_RELEASE_DECISION.md",
        ROOT
        / "docs"
        / "assurance"
        / "CHANGE_IMPACT_2026-07-29_RC3_PROGRAM.md",
        ROOT
        / "docs"
        / "assurance"
        / "CHANGE_IMPACT_2026-07-29_RC3_EXTENDED_WKB.md",
        ROOT
        / "docs"
        / "assurance"
        / "CHANGE_IMPACT_2026-07-29_RC3_FILEGDB_PUSHDOWN.md",
        ROOT
        / "docs"
        / "assurance"
        / "CHANGE_IMPACT_2026-07-29_RC3_EXTERNAL_GATES.md",
        ROOT
        / "docs"
        / "assurance"
        / "CHANGE_IMPACT_2026-07-29_RC3_FUZZ_CAMPAIGN.md",
        ROOT
        / "docs"
        / "assurance"
        / "CHANGE_IMPACT_2026-07-30_RC3_WINDOWS_GDAL.md",
        ROOT
        / "docs"
        / "assurance"
        / "CHANGE_IMPACT_2026-07-30_RC3_CRS_SCOPE.md",
        ROOT
        / "docs"
        / "assurance"
        / "CHANGE_IMPACT_2026-07-30_RC3_RELEASE_DECISION.md",
        ROOT
        / "docs"
        / "assurance"
        / "CHANGE_IMPACT_2026-07-30_RC4_PROGRAM.md",
        ROOT
        / "docs"
        / "assurance"
        / "CHANGE_IMPACT_2026-07-30_RC4_RELEASE_DECISION.md",
        CORPUS_SCHEMA,
        CORPUS_MANIFEST,
    ]
    for path in required_files:
        if not path.is_file():
            errors.append(f"{path.relative_to(ROOT)}: file obbligatorio assente")

    if not errors:
        try:
            errors.extend(
                validate_documents(
                    load_json(PROVENANCE),
                    load_json(SYSTEM_GATE),
                    load_json(FREEZE_READINESS),
                    load_json(EVIDENCE),
                    load_json(INDEPENDENT_REVIEW),
                    load_json(CORPUS_SCHEMA),
                    load_json(CORPUS_MANIFEST),
                    GEOMETRY_SOURCE.read_text(encoding="utf-8"),
                )
            )
            errors.extend(
                validate_workspace_versions(
                    load_toml(WORKSPACE_MANIFEST),
                    [load_toml(path) for path in WORKSPACE_CRATE_MANIFESTS],
                    load_toml(WORKSPACE_LOCK),
                    [load_toml(path) for path in DETACHED_LOCKFILES],
                )
            )
            errors.extend(validate_rc3_development(load_json(RC3_DEVELOPMENT)))
            errors.extend(validate_rc4_development(load_json(RC4_DEVELOPMENT)))
        except (OSError, ValueError, json.JSONDecodeError) as error:
            errors.append(str(error))

    if errors:
        print("Release contract gate failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1

    print(
        "Release contract gate passed "
        "(v0.1.0-rc.3 remains immutable; v0.1.0-rc.4 is frozen pending "
        "pre-tag CI; component RC, system RC and avionic certification are "
        "not yet claimed for RC4)."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
