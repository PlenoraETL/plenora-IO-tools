#!/usr/bin/env python3
"""Compare IO/database observations for the shared WKB/EWKB corpus."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any


def load_object(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{path}: radice non object")
    return value


def normalize_category(value: object) -> str | None:
    if not isinstance(value, str):
        return None
    return re.sub(r"(?<!^)(?=[A-Z])", "_", value).lower()


def case_map(report: dict[str, Any], source: str) -> dict[str, dict[str, Any]]:
    cases = report.get("cases")
    if not isinstance(cases, list):
        raise ValueError(f"{source}: cases non array")
    result: dict[str, dict[str, Any]] = {}
    for case in cases:
        if not isinstance(case, dict) or not isinstance(case.get("path"), str):
            raise ValueError(f"{source}: caso privo di path")
        path = case["path"]
        if path in result:
            raise ValueError(f"{source}: caso duplicato {path}")
        result[path] = case
    return result


def compare(
    manifest: dict[str, Any],
    io_report: dict[str, Any],
    database_report: dict[str, Any],
) -> dict[str, Any]:
    manifest_cases = case_map({"cases": manifest.get("cases")}, "manifest")
    io_cases = case_map(io_report, "IO-tools")
    database_cases = case_map(database_report, "database-tools")
    errors: list[str] = []
    classifications: list[dict[str, str]] = []

    expected_paths = set(manifest_cases)
    for source, cases in (("IO-tools", io_cases), ("database-tools", database_cases)):
        missing = sorted(expected_paths - set(cases))
        extra = sorted(set(cases) - expected_paths)
        errors.extend(f"{source}: caso mancante {path}" for path in missing)
        errors.extend(f"{source}: caso inatteso {path}" for path in extra)

    compared = 0
    for path in sorted(expected_paths & set(io_cases) & set(database_cases)):
        compared += 1
        declared = manifest_cases[path]
        io_case = io_cases[path]
        database_case = database_cases[path]
        io_accepted = io_case.get("accepted")
        database_accepted = database_case.get("accepted")
        if not isinstance(io_accepted, bool) or not isinstance(database_accepted, bool):
            errors.append(f"{path}: accepted non booleano")
            continue
        if io_accepted != database_accepted:
            known = declared.get("known_difference")
            if (
                isinstance(known, dict)
                and known.get("dimension") == "acceptance"
                and isinstance(known.get("classification"), str)
                and isinstance(known.get("reason"), str)
            ):
                classifications.append(
                    {
                        "path": path,
                        "classification": known["classification"],
                        "reason": known["reason"],
                    }
                )
            else:
                errors.append(f"{path}: accettazione divergente non classificata")
            continue
        if io_accepted:
            for key in ("geometry_type", "dimensions", "srid"):
                if io_case.get(key) != database_case.get(key):
                    errors.append(f"{path}: {key} divergente")
        else:
            io_category = normalize_category(io_case.get("error_category"))
            database_category = normalize_category(database_case.get("error_category"))
            if io_category != database_category:
                known = declared.get("known_difference")
                if (
                    isinstance(known, dict)
                    and known.get("dimension") == "error_category"
                    and isinstance(known.get("classification"), str)
                    and isinstance(known.get("reason"), str)
                ):
                    classifications.append(
                        {
                            "path": path,
                            "classification": known["classification"],
                            "reason": known["reason"],
                        }
                    )
                else:
                    errors.append(
                        f"{path}: categoria errore divergente "
                        f"({io_category!r} != {database_category!r})"
                    )

    return {
        "schema_version": 1,
        "corpus_id": manifest.get("corpus_id"),
        "components": {
            "io": io_report.get("component"),
            "database": database_report.get("component"),
        },
        "compared_cases": compared,
        "classified_differences": classifications,
        "errors": errors,
        "status": "pass" if not errors else "fail",
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("manifest", type=Path)
    parser.add_argument("io_report", type=Path)
    parser.add_argument("database_report", type=Path)
    parser.add_argument("--json-out", type=Path)
    args = parser.parse_args()
    try:
        result = compare(
            load_object(args.manifest),
            load_object(args.io_report),
            load_object(args.database_report),
        )
    except (OSError, UnicodeError, json.JSONDecodeError, ValueError) as exc:
        print(exc, file=sys.stderr)
        return 1
    encoded = json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True)
    print(encoded)
    if args.json_out is not None:
        args.json_out.parent.mkdir(parents=True, exist_ok=True)
        args.json_out.write_text(encoded + "\n", encoding="utf-8")
    return 0 if result["status"] == "pass" else 1


if __name__ == "__main__":
    sys.exit(main())
