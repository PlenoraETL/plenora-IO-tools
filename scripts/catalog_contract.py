#!/usr/bin/env python3
"""Canonical v1 catalog envelope semantics shared by the release and CI gates.

This module is deliberately stdlib-only and free of repository state so that both
`check_release_contract.py` and `check_filegdb_catalog.py` can validate producer
output against the very same frozen rules, without importing each other.
"""

from __future__ import annotations

from typing import Any


EXPECTED_STATUS = "ok"
EXPECTED_DETERMINISM = "byte_for_byte"


def catalog_envelope(protocol: dict[str, Any]) -> dict[str, Any]:
    """Return the frozen catalog envelope declaration of the v1 CLI protocol."""
    envelopes = protocol.get("envelopes", {})
    envelope = envelopes.get("catalog", {}) if isinstance(envelopes, dict) else {}
    return envelope if isinstance(envelope, dict) else {}


def validate_catalog_producer(
    document: Any,
    protocol: dict[str, Any],
    *,
    current: bool,
) -> list[str]:
    """Validate a v1 catalog while preserving additive-field legacy acceptance."""
    if not isinstance(document, dict):
        return ["catalog producer: root must be a JSON object"]

    errors: list[str] = []
    catalog_contract = catalog_envelope(protocol)
    for field in catalog_contract.get("required_top_level", []):
        if field not in document:
            errors.append(f"catalog producer: required top-level field absent: {field}")

    if document.get("status", EXPECTED_STATUS) != EXPECTED_STATUS:
        errors.append("catalog producer: status is not the successful envelope status")
    if "protocol_version" in document:
        version = document["protocol_version"]
        if type(version) is not int or version != protocol.get("protocol_version"):
            errors.append(
                "catalog producer: protocol_version differs from the frozen manifest"
            )
    if "contract" in document and document["contract"] != catalog_contract.get(
        "contract"
    ):
        errors.append("catalog producer: contract differs from the frozen manifest")
    if document.get("determinism", EXPECTED_DETERMINISM) != EXPECTED_DETERMINISM:
        errors.append("catalog producer: determinism is not byte_for_byte")

    drivers = document.get("drivers")
    if not isinstance(drivers, list):
        return [*errors, "catalog producer: drivers must be an array"]
    current_fields = catalog_contract.get("current_producer", {}).get(
        "required_driver_fields", []
    )
    for index, driver in enumerate(drivers):
        if not isinstance(driver, dict):
            errors.append(f"catalog producer: driver {index} must be an object")
            continue
        if current:
            for field in current_fields:
                if field not in driver:
                    errors.append(
                        f"catalog producer: current driver {index} missing {field}"
                    )
        if "available" in driver and type(driver["available"]) is not bool:
            errors.append(f"catalog producer: driver {index} available is not boolean")
        if "required_feature" in driver:
            feature = driver["required_feature"]
            if driver.get("id") == "filegdb":
                if feature != "gdal-backend":
                    errors.append(
                        "catalog producer: filegdb required_feature is not gdal-backend"
                    )
            elif feature is not None:
                errors.append(
                    f"catalog producer: driver {index} required_feature is not null"
                )
    return errors
