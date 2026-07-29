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
EVIDENCE = ROOT / "release" / "evidence" / "pre-freeze-2026-07-28.json"
CORPUS_SCHEMA = ROOT / "fuzz" / "shared-corpus-manifest.schema.json"
CORPUS_MANIFEST = ROOT / "fuzz" / "shared-corpus" / "manifest.json"
GEOMETRY_SOURCE = ROOT / "crates" / "plenora-io-model" / "src" / "geometry.rs"
EXPECTED_ICD_TAG = "v2.0-rc8"
EXPECTED_ICD_REVISION = "62b12e3496466d2c908dac3cc098640b99b52e21"
EXPECTED_IO_BASELINE = "1c37fb5d525647b264ce977e26fc07b346bb7914"
EXPECTED_DATABASE_REPLAY_REVISION = "ef18e80c798126f872fd366c36ee96a029598958"
EXPECTED_SYSTEM_REVISIONS = {
    "plenora-IO-tools": EXPECTED_IO_BASELINE,
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
    if provenance.get("freeze_status") not in {"pre_freeze", "frozen"}:
        errors.append("contract-provenance: freeze_status non valido")
    if not SHA.fullmatch(str(provenance.get("implementation_revision", ""))):
        errors.append("contract-provenance: implementation_revision non è uno SHA completo")
    elif provenance.get("implementation_revision") != EXPECTED_IO_BASELINE:
        errors.append("contract-provenance: baseline IO inattesa")
    if provenance.get("candidate_worktree") != "uncommitted_eight_point_completion":
        errors.append("contract-provenance: worktree candidato non dichiarato")

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

    if freeze_readiness.get("status") != "not_ready_to_freeze":
        errors.append("freeze readiness: il worktree non può risultare congelabile")
    readiness_gates = freeze_readiness.get("gates", {})
    for open_gate in (
        "candidate_revision_committed",
        "candidate_ci",
        "independent_review",
        "release_tag",
    ):
        if readiness_gates.get(open_gate) is not False:
            errors.append(f"freeze readiness: gate aperto non fail-closed: {open_gate}")

    if evidence.get("status") != "candidate_worktree_local_evidence":
        errors.append("evidence: stato candidato inatteso")
    if evidence.get("baseline_revision") != EXPECTED_IO_BASELINE:
        errors.append("evidence: baseline IO inattesa")
    if evidence.get("candidate_revision") is not None:
        errors.append("evidence: il worktree non committato non può avere una revisione")
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

    print("Release contract gate passed (component RC only; system RC not claimed).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
