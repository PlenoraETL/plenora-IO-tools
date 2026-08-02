#!/usr/bin/env python3
"""Gate fail-closed della metadata candidate patch IO Tools 1.0.1."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import tomllib
from pathlib import Path
from typing import Any, Callable, Sequence

try:
    from scripts.check_release_contract import (
        load_toml,
        validate_current_checkout,
        validate_release_tag_binding,
    )
except ModuleNotFoundError:
    from check_release_contract import (
        load_toml,
        validate_current_checkout,
        validate_release_tag_binding,
    )

ROOT = Path(__file__).resolve().parents[1]
PATCH_MANIFEST = ROOT / "release" / "1.0.1.json"
EXPECTED_VERSION = "1.0.1"
EXPECTED_BASE = "966005d67b6f2d4fcfe5d62e58fced17881eff06"
EXPECTED_PREVIOUS_RELEASE = "ca7f34c6b732700fa79f579f7b003df27ce54b09"
EXPECTED_CONTRACTS_REVISION = "ec55bf1379b8b26ca9b9e29e10b954e39c2258ed"
EXPECTED_CI_RUN = 30742404497
EXPECTED_RELEASE_CRITERIA = {
    "repository": "plenora-contracts",
    "revision": EXPECTED_CONTRACTS_REVISION,
    "tag": None,
    "document": "conformance/releases/v1.0.0.json",
    "normative_status": "frozen_campaign_baseline",
}
EXPECTED_FUNCTIONAL_DELTA = {
    "public_api": "unchanged",
    "contracts": "unchanged",
    "wire_formats": "unchanged",
    "error_semantics": "unchanged",
    "cleanup": "behavior_preserving",
    "filegdb_release_catalog": "corrected_and_qualified",
}
EXPECTED_EXTERNAL_DEPENDENCIES = [
    {
        "id": "PLN-IO-CROSS-LIBRARY",
        "owner": "plenora-contracts/conformance",
        "status": "pending",
        "description": (
            "Le catene PostgreSQL/MySQL Database -> Data -> IO devono passare "
            "sui nuovi artefatti prima del tag."
        ),
    },
    {
        "id": "PLN-IO-SYSTEM",
        "owner": "plenora-contracts/conformance",
        "status": "pending",
        "description": (
            "La comparativa Plenora resta separata e non autorizza claim system_rc."
        ),
    },
]
EXPECTED_RELEASE_ACTION = {
    "allowed": False,
    "reason": "CI same-SHA del commit metadata e catene cross-library ancora obbligatorie.",
    "intended_tag": "v1.0.1",
    "tag_created": False,
    "tag_target": None,
    "tag_object": None,
}
WORKSPACE_PACKAGES = {
    "driver-common",
    "driver-csv",
    "driver-dxf",
    "driver-filegdb",
    "driver-geojson",
    "driver-geoparquet",
    "driver-gpkg",
    "driver-ipc",
    "driver-kml",
    "driver-shp",
    "driver-xls",
    "plenora-bench",
    "plenora-fuzz",
    "plenora-io-cli",
    "plenora-io-core",
    "plenora-io-model",
}
FUZZ_WORKSPACE_PACKAGES = {
    "driver-common",
    "driver-dxf",
    "driver-geojson",
    "driver-geoparquet",
    "driver-kml",
    "driver-shp",
    "plenora-io-core",
    "plenora-io-model",
}
PRODUCTION_DELTA = {"Cargo.toml", "Cargo.lock", "fuzz/Cargo.lock"}
ASSURANCE_DELTA = {
    ".github/workflows/ci.yml",
    ".github/workflows/release-qualification.yml",
    "docs/PATCH-1.0.1-READINESS.md",
    "release/1.0.1.json",
    "scripts/check_patch_readiness.py",
    "scripts/check_release_contract.py",
    "scripts/test_check_patch_readiness.py",
    "scripts/test_check_release_contract.py",
}


def load_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{path}: oggetto JSON atteso")
    return value


def workspace_versions(root: Path) -> tuple[str | None, dict[str, str], dict[str, str]]:
    cargo = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    lock = tomllib.loads((root / "Cargo.lock").read_text(encoding="utf-8"))
    fuzz_lock = tomllib.loads((root / "fuzz" / "Cargo.lock").read_text(encoding="utf-8"))
    locked = {
        package["name"]: package["version"]
        for package in lock["package"]
        if package["name"] in WORKSPACE_PACKAGES
    }
    fuzz_locked = {
        package["name"]: package["version"]
        for package in fuzz_lock["package"]
        if package["name"] in FUZZ_WORKSPACE_PACKAGES
    }
    return cargo.get("workspace", {}).get("package", {}).get("version"), locked, fuzz_locked


def validate_workspace_inheritance(root: Path) -> list[str]:
    errors: list[str] = []
    manifests = tuple(sorted((root / "crates").glob("*/Cargo.toml")))
    if not manifests:
        return ["nessun manifest crate workspace trovato"]
    for manifest in manifests:
        document = tomllib.loads(manifest.read_text(encoding="utf-8"))
        if document.get("package", {}).get("version") != {"workspace": True}:
            errors.append(
                f"{manifest.relative_to(root)}: package.version deve ereditare workspace"
            )
    return errors


def validate_patch_readiness(document: dict[str, Any], root: Path = ROOT) -> list[str]:
    errors: list[str] = []
    if document.get("manifest_version") != 1:
        errors.append("manifest_version deve essere 1")
    if document.get("component") != "plenora-IO-tools":
        errors.append("component inatteso")
    if document.get("component_version") != EXPECTED_VERSION:
        errors.append("component_version deve essere 1.0.1")
    if document.get("status") != "metadata_candidate_pending_same_sha_ci":
        errors.append("stato candidate inatteso")
    if document.get("revision") != EXPECTED_BASE:
        errors.append("evidence base inattesa")
    if document.get("verification_claim") != "verified_internally":
        errors.append("claim di verifica inatteso")
    if document.get("independent_review") is not False:
        errors.append("review indipendente sovra-affermata")
    if document.get("claims") != {
        "component_rc": True,
        "system_rc": False,
        "avionic_certification": False,
    }:
        errors.append("claims patch inattesi")

    if document.get("supersedes") != {
        "tag": "v1.0.0",
        "revision": EXPECTED_PREVIOUS_RELEASE,
        "immutable": True,
    }:
        errors.append("identita release precedente inattesa")

    candidate = document.get("candidate", {})
    if candidate.get("evidence_base_revision") != EXPECTED_BASE:
        errors.append("candidate evidence base inattesa")
    if candidate.get("evidence_base_result") != "passed":
        errors.append("candidate evidence base non passed")
    if candidate.get("metadata_delta_requires_new_same_sha_ci") is not True:
        errors.append("nuova CI same-SHA non richiesta")
    if set(candidate.get("production_delta", [])) != PRODUCTION_DELTA:
        errors.append("delta produzione inatteso")
    if set(candidate.get("assurance_delta", [])) != ASSURANCE_DELTA:
        errors.append("delta assurance inatteso")

    evidence = document.get("evidence", [])
    if not isinstance(evidence, list) or len(evidence) != 1 or not isinstance(evidence[0], dict):
        errors.append("evidenza CI patch inattesa")
    else:
        item = evidence[0]
        if item != {
            "id": "ci_same_sha",
            "status": "passed",
            "revision": EXPECTED_BASE,
            "run_id": EXPECTED_CI_RUN,
            "url": (
                "https://github.com/PlenoraETL/plenora-IO-tools/actions/runs/"
                f"{EXPECTED_CI_RUN}"
            ),
        }:
            errors.append("evidenza CI patch divergente")

    if document.get("release_criteria") != EXPECTED_RELEASE_CRITERIA:
        errors.append("release criteria Contracts inattesi")
    if document.get("functional_delta") != EXPECTED_FUNCTIONAL_DELTA:
        errors.append("delta funzionale inatteso")
    if document.get("external_dependencies") != EXPECTED_EXTERNAL_DEPENDENCIES:
        errors.append("dipendenze esterne inattese")

    if document.get("release_action") != EXPECTED_RELEASE_ACTION:
        errors.append("release action inattesa")

    workspace, locked, fuzz_locked = workspace_versions(root)
    if workspace != EXPECTED_VERSION:
        errors.append("workspace non allineato a 1.0.1")
    if locked != {name: EXPECTED_VERSION for name in WORKSPACE_PACKAGES}:
        errors.append("Cargo.lock non allineato a 1.0.1")
    if fuzz_locked != {name: EXPECTED_VERSION for name in FUZZ_WORKSPACE_PACKAGES}:
        errors.append("fuzz/Cargo.lock non allineato a 1.0.1")
    errors.extend(validate_workspace_inheritance(root))
    return errors


def git_runner(root: Path) -> Callable[[Sequence[str]], subprocess.CompletedProcess[str]]:
    def run(arguments: Sequence[str]) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["git", *arguments],
            cwd=root,
            check=False,
            text=True,
            capture_output=True,
        )

    return run


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("manifest", nargs="?", type=Path, default=PATCH_MANIFEST)
    parser.add_argument("--repo", type=Path, default=ROOT)
    parser.add_argument("--qualify-current", action="store_true")
    parser.add_argument("--expected-revision")
    parser.add_argument("--release-tag")
    parser.add_argument("--json-out", type=Path)
    arguments = parser.parse_args()
    root = arguments.repo.resolve()
    try:
        document = load_json(arguments.manifest)
        errors = validate_patch_readiness(document, root)
        if arguments.qualify_current:
            errors.extend(
                validate_current_checkout(
                    arguments.expected_revision or "", git_runner(root)
                )
            )
            if arguments.release_tag is not None:
                errors.extend(
                    validate_release_tag_binding(
                        arguments.release_tag, load_toml(root / "Cargo.toml")
                    )
                )
        elif arguments.expected_revision is not None or arguments.release_tag is not None:
            errors.append("--expected-revision/--release-tag richiedono --qualify-current")
    except (OSError, ValueError, json.JSONDecodeError, tomllib.TOMLDecodeError) as error:
        errors = [str(error)]

    result = {
        "schema_version": 1,
        "status": "passed" if not errors else "failed",
        "component": "plenora-IO-tools",
        "component_version": EXPECTED_VERSION,
        "manifest": str(arguments.manifest),
        "errors": errors,
    }
    rendered = json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    if arguments.json_out is not None:
        arguments.json_out.write_text(rendered, encoding="utf-8")
    (sys.stdout if not errors else sys.stderr).write(rendered)
    return 0 if not errors else 1


if __name__ == "__main__":
    raise SystemExit(main())
