#!/usr/bin/env python3
"""Validate the component-RC contract provenance without network access."""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
PROVENANCE = ROOT / "release" / "contract-provenance.json"
SYSTEM_GATE = ROOT / "release" / "system-rc-gate.json"
FREEZE_READINESS = ROOT / "release" / "freeze-readiness.json"
EVIDENCE = ROOT / "release" / "evidence" / "technical-freeze-2026-07-29.json"
CORPUS_SCHEMA = ROOT / "fuzz" / "shared-corpus-manifest.schema.json"
CORPUS_MANIFEST = ROOT / "fuzz" / "shared-corpus" / "manifest.json"
GEOMETRY_SOURCE = ROOT / "crates" / "plenora-io-model" / "src" / "geometry.rs"
EXPECTED_ICD_TAG = "v2.0-rc8"
EXPECTED_ICD_REVISION = "62b12e3496466d2c908dac3cc098640b99b52e21"
EXPECTED_IO_BASELINE = "1c37fb5d525647b264ce977e26fc07b346bb7914"
EXPECTED_IO_CANDIDATE = "78c2d150b9c7d0ac48e4c97b03f86228e0f0a068"
EXPECTED_CANDIDATE_STATE = "technical_baseline_frozen_pending_independent_review"
EXPECTED_CANDIDATE_CI_RUN = 30415766905
EXPECTED_COVERAGE_ARTIFACT = {
    "id": 8710097703,
    "name": "rust-coverage-lcov",
    "size_bytes": 94172,
    "sha256": "f5473d8c3e55fcaecf54ff5134872157c94351686356c7fd3db3928c90b701ab",
    "digest_source": "github_actions_artifact_api",
}
EXPECTED_LIBRARY_COVERAGE = {"covered": 12769, "total": 15271, "percent": 83.62}
EXPECTED_LIBRARY_COVERAGE_SOURCE = (
    "local_reproduction_of_ci_command"
)
EXPECTED_FREEZE_SCOPE = {
    "kind": "technical_baseline",
    "baseline_revision": EXPECTED_IO_CANDIDATE,
    "frozen_on": "2026-07-29",
    "verification_claim": "verified_internally",
    "independent_review_status": "not_performed",
    "release_tag_authorized": False,
    "assurance_promotion_authorized": False,
}
EXPECTED_FREEZE_DECISION = {
    "status": "technical_baseline_frozen",
    "baseline_revision": EXPECTED_IO_CANDIDATE,
    "frozen_on": "2026-07-29",
    "verification_claim": "verified_internally",
    "independent_review": False,
    "release_tag_created": False,
    "decision_record": (
        "docs/assurance/CHANGE_IMPACT_2026-07-29_TECHNICAL_FREEZE.md"
    ),
}
EXPECTED_DATABASE_REPLAY_REVISION = "ef18e80c798126f872fd366c36ee96a029598958"
EXPECTED_SYSTEM_REVISIONS = {
    "plenora-IO-tools": EXPECTED_IO_CANDIDATE,
    "plenora-data-tools": "97e48ba469f9f55a2cc83e9598d72899c29e2be6",
    "plenora-database-tools": "2588523bf6a4ad57e62ae3d44e9f58025c55a913",
}
EXPECTED_FUZZ_STATE = "deterministic_cross_replay_passed_long_campaign_pending"
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


def validate_documents(
    provenance: dict[str, Any],
    system_gate: dict[str, Any],
    freeze_readiness: dict[str, Any],
    evidence: dict[str, Any],
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
        "component_rc": True,
        "system_rc": False,
        "avionic_certification": False,
    }:
        errors.append("contract-provenance: claims RC/sistema/avionica non fail-closed")

    if fuzz.get("state") != EXPECTED_FUZZ_STATE:
        errors.append("contract-provenance: stato fuzz coordinato inatteso")
    if fuzz.get("io_tools_revision") != EXPECTED_IO_BASELINE:
        errors.append("contract-provenance: revisione IO-tools fuzz inattesa")
    if fuzz.get("database_tools_revision") != EXPECTED_DATABASE_REPLAY_REVISION:
        errors.append("contract-provenance: revisione database-tools fuzz inattesa")
    if fuzz.get("corpus_cases") != 18 or fuzz.get("cross_replay_status") != "pass":
        errors.append("contract-provenance: replay dei 18 casi non registrato come pass")
    if fuzz.get("unclassified_differences") != 0:
        errors.append("contract-provenance: divergenze fuzz non classificate")

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
        "plenora-IO-tools": EXPECTED_IO_BASELINE,
        "plenora-database-tools": EXPECTED_DATABASE_REPLAY_REVISION,
    }:
        errors.append("shared corpus: revisioni dei producer inattese")

    if freeze_readiness.get("status") != "frozen_with_open_assurance_gates":
        errors.append("freeze readiness: stato del freeze tecnico inatteso")
    if freeze_readiness.get("freeze_scope") != "technical_baseline_only":
        errors.append("freeze readiness: perimetro tecnico non dichiarato")
    if freeze_readiness.get("release_authorized") is not False:
        errors.append("freeze readiness: release autorizzata senza review")
    readiness_gates = freeze_readiness.get("gates", {})
    if readiness_gates.get("candidate_revision_committed") is not True:
        errors.append("freeze readiness: revisione candidata non registrata")
    if freeze_readiness.get("candidate_revision") != EXPECTED_IO_CANDIDATE:
        errors.append("freeze readiness: SHA candidato inatteso")
    if readiness_gates.get("candidate_ci") is not True:
        errors.append("freeze readiness: CI candidata non registrata")
    if readiness_gates.get("technical_baseline_frozen") is not True:
        errors.append("freeze readiness: baseline tecnica non congelata")
    for open_gate in ("independent_review", "release_tag"):
        if readiness_gates.get(open_gate) is not False:
            errors.append(f"freeze readiness: gate aperto non fail-closed: {open_gate}")

    if evidence.get("status") != "technical_freeze_evidence":
        errors.append("evidence: stato del freeze tecnico inatteso")
    if evidence.get("baseline_revision") != EXPECTED_IO_BASELINE:
        errors.append("evidence: baseline IO inattesa")
    if evidence.get("candidate_revision") != EXPECTED_IO_CANDIDATE:
        errors.append("evidence: revisione candidata inattesa")
    if evidence.get("freeze_decision") != EXPECTED_FREEZE_DECISION:
        errors.append("evidence: decisione di freeze inattesa")
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

    return errors


def main() -> int:
    errors: list[str] = []
    required_files = [
        PROVENANCE,
        SYSTEM_GATE,
        FREEZE_READINESS,
        EVIDENCE,
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
                    load_json(CORPUS_SCHEMA),
                    load_json(CORPUS_MANIFEST),
                    GEOMETRY_SOURCE.read_text(encoding="utf-8"),
                )
            )
        except (OSError, ValueError, json.JSONDecodeError) as error:
            errors.append(str(error))

    if errors:
        print("Release contract gate failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1

    print(
        "Release contract gate passed "
        "(technical baseline frozen; independent review and tag open; "
        "system RC not claimed)."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
