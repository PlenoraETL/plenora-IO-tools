#!/usr/bin/env python3
"""Validate feature-on catalog evidence emitted by the real CLI binary."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any, Sequence

try:  # importable as `scripts.check_filegdb_catalog` and as a plain script
    from scripts.catalog_contract import validate_catalog_producer
except ImportError:  # pragma: no cover - taken when run as scripts/<file>.py
    from catalog_contract import validate_catalog_producer


ROOT = Path(__file__).resolve().parents[1]
CLI_PROTOCOL_V1 = ROOT / "release" / "cli-protocol-v1.json"


def load_protocol(path: Path = CLI_PROTOCOL_V1) -> dict[str, Any]:
    """Load the frozen v1 CLI protocol manifest the evidence is judged against."""
    document = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(document, dict):
        raise ValueError(f"{path}: root JSON is not an object")
    return document


def validate_catalog(
    document: Any,
    protocol: dict[str, Any],
    *,
    expected_available: bool = True,
) -> list[str]:
    """Require the canonical contract and the expected runtime FileGDB state."""
    errors = validate_catalog_producer(document, protocol, current=True)
    if not isinstance(document, dict):
        return errors

    drivers = document.get("drivers")
    if not isinstance(drivers, list):
        return errors
    filegdb = [
        driver
        for driver in drivers
        if isinstance(driver, dict) and driver.get("id") == "filegdb"
    ]
    if len(filegdb) != 1:
        errors.append("catalog: exactly one filegdb driver is required")
        return errors

    entry = filegdb[0]
    if entry.get("available") is not expected_available:
        expected = str(expected_available).lower()
        errors.append(f"catalog: filegdb.available must be boolean {expected}")
    if entry.get("required_feature") != "gdal-backend":
        errors.append(
            'catalog: filegdb.required_feature must be exactly "gdal-backend"'
        )
    return errors


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "catalog",
        nargs="?",
        type=Path,
        help="catalog JSON file; stdin is used when omitted",
    )
    parser.add_argument(
        "--expect-available",
        choices=("true", "false"),
        default="true",
        help="expected boolean value of filegdb.available (default: true)",
    )
    arguments = parser.parse_args(argv)
    try:
        protocol = load_protocol()
        source = (
            arguments.catalog.read_text(encoding="utf-8")
            if arguments.catalog is not None
            else sys.stdin.read()
        )
        document = json.loads(source)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"FileGDB catalog evidence failed: {error}", file=sys.stderr)
        return 1

    expected_available = arguments.expect_available == "true"
    errors = validate_catalog(
        document,
        protocol,
        expected_available=expected_available,
    )
    if errors:
        print("FileGDB catalog evidence failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1
    print(
        "FileGDB catalog evidence passed: the real CLI emitted a conforming "
        "plenora-io-catalog-v1 current-producer envelope reporting boolean "
        f"available={str(expected_available).lower()} and "
        'required_feature="gdal-backend" for FileGDB.'
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
