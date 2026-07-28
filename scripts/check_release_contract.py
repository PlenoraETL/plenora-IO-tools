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
CORPUS_SCHEMA = ROOT / "fuzz" / "shared-corpus-manifest.schema.json"
GEOMETRY_SOURCE = ROOT / "crates" / "plenora-io-model" / "src" / "geometry.rs"
EXPECTED_ICD_TAG = "v2.0-rc3"
EXPECTED_ICD_REVISION = "ef2640348426425585ad228312468e7cf1d0e50f"
EXPECTED_DATABASE_REVISION = "834fff4fbe0c62cc2f02278073e58b0cf2159f8d"
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
    corpus_schema: dict[str, Any],
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
        and deviation.get("rule") == "§15.4 step 1"
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

    if fuzz.get("state") != "protocol_defined_campaign_not_started":
        errors.append("contract-provenance: campagna condivisa non deve risultare avviata")
    if fuzz.get("database_tools_revision") != EXPECTED_DATABASE_REVISION:
        errors.append("contract-provenance: revisione database-tools fuzz inattesa")

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

    component_names = {
        component.get("name")
        for component in system_gate.get("components", [])
        if isinstance(component, dict) and SHA.fullmatch(str(component.get("revision", "")))
    }
    expected_components = {
        "plenora-IO-tools",
        "plenora-data-tools",
        "plenora-database-tools",
    }
    if component_names != expected_components:
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
        "invariants",
    ):
        if required_property not in case_properties:
            errors.append(
                f"shared corpus: proprietà caso assente: {required_property}"
            )

    return errors


def main() -> int:
    errors: list[str] = []
    required_files = [
        PROVENANCE,
        SYSTEM_GATE,
        ROOT / "docs" / "assurance" / "RELEASE_CANDIDATE_SCOPE.md",
        ROOT / "docs" / "assurance" / "SYSTEM_RC_GATE.md",
        ROOT / "docs" / "assurance" / "WKB_EWKB_FUZZ_COORDINATION.md",
        ROOT
        / "docs"
        / "assurance"
        / "CHANGE_IMPACT_2026-07-28_RC_PROVENANCE_FUZZ_COORDINATION.md",
        CORPUS_SCHEMA,
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
                    load_json(CORPUS_SCHEMA),
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
