#!/usr/bin/env python3
"""Generate and verify the deterministic cross-repository WKB/EWKB corpus."""

from __future__ import annotations

import argparse
import hashlib
import json
import struct
import sys
from dataclasses import dataclass
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_OUTPUT = ROOT / "fuzz" / "shared-corpus"
ICD_TAG = "v2.0-rc8"
ICD_REVISION = "62b12e3496466d2c908dac3cc098640b99b52e21"
COMMON_INVARIANTS = [
    "no_panic",
    "full_consumption",
    "bounded_components",
    "bounded_depth",
    "checked_arithmetic",
    "stable_with_looser_limits",
]


@dataclass(frozen=True)
class Case:
    name: str
    payload: bytes
    dialect: str
    expectation: str
    geometry_type: str | None = None
    dimensions: str | None = None
    srid: int | None = None
    expected_error_category: str | None = None
    roundtrip: bool = False
    difference_dimension: str | None = None
    difference_classification: str | None = None
    difference_reason: str | None = None


def u32(value: int, little: bool = True) -> bytes:
    return struct.pack("<I" if little else ">I", value)


def f64(value: float, little: bool = True) -> bytes:
    return struct.pack("<d" if little else ">d", value)


def geometry(
    type_word: int,
    body: bytes,
    *,
    little: bool = True,
    srid: int | None = None,
) -> bytes:
    payload = bytes([1 if little else 0]) + u32(type_word, little)
    if srid is not None:
        payload += u32(srid, little)
    return payload + body


def point(
    x: float,
    y: float,
    *,
    type_word: int = 1,
    extra: tuple[float, ...] = (),
    little: bool = True,
    srid: int | None = None,
) -> bytes:
    ordinates = (x, y, *extra)
    return geometry(
        type_word,
        b"".join(f64(value, little) for value in ordinates),
        little=little,
        srid=srid,
    )


def line(points: list[tuple[float, float]], *, type_word: int = 2) -> bytes:
    body = u32(len(points))
    body += b"".join(f64(x) + f64(y) for x, y in points)
    return geometry(type_word, body)


def polygon(rings: list[list[tuple[float, float]]]) -> bytes:
    body = u32(len(rings))
    for ring in rings:
        body += u32(len(ring))
        body += b"".join(f64(x) + f64(y) for x, y in ring)
    return geometry(3, body)


def children(base_type: int, values: list[bytes]) -> bytes:
    return geometry(base_type, u32(len(values)) + b"".join(values))


def corpus_cases() -> list[Case]:
    point_xy = point(1.0, 2.0)
    line_xy = line([(0.0, 0.0), (1.0, 1.0)])
    polygon_xy = polygon([[(0.0, 0.0), (2.0, 0.0), (0.0, 2.0), (0.0, 0.0)]])
    return [
        Case("point-xy-le", point_xy, "wkb", "accepted", "point", "xy", roundtrip=True),
        Case(
            "point-xy-be",
            point(1.0, 2.0, little=False),
            "wkb",
            "accepted",
            "point",
            "xy",
            roundtrip=True,
        ),
        Case("linestring-xy", line_xy, "wkb", "accepted", "linestring", "xy", roundtrip=True),
        Case("polygon-xy", polygon_xy, "wkb", "accepted", "polygon", "xy", roundtrip=True),
        Case(
            "multipoint-xy",
            children(4, [point_xy, point(3.0, 4.0)]),
            "wkb",
            "accepted",
            "multipoint",
            "xy",
            roundtrip=True,
        ),
        Case(
            "multilinestring-xy",
            children(5, [line_xy]),
            "wkb",
            "accepted",
            "multilinestring",
            "xy",
            roundtrip=True,
        ),
        Case(
            "multipolygon-xy",
            children(6, [polygon_xy]),
            "wkb",
            "accepted",
            "multipolygon",
            "xy",
            roundtrip=True,
        ),
        Case(
            "geometrycollection-xy",
            children(7, [point_xy, line_xy]),
            "wkb",
            "accepted",
            "geometrycollection",
            "xy",
            roundtrip=True,
        ),
        Case(
            "point-xyz-iso",
            point(1.0, 2.0, type_word=1001, extra=(3.0,)),
            "wkb",
            "accepted",
            "point",
            "xyz",
            roundtrip=True,
        ),
        Case(
            "point-xym-iso",
            point(1.0, 2.0, type_word=2001, extra=(4.0,)),
            "wkb",
            "accepted",
            "point",
            "xym",
            roundtrip=True,
        ),
        Case(
            "point-xyzm-iso",
            point(1.0, 2.0, type_word=3001, extra=(3.0, 4.0)),
            "wkb",
            "accepted",
            "point",
            "xyzm",
            roundtrip=True,
        ),
        Case(
            "point-xyzm-ewkb-srid-4326",
            point(
                1.0,
                2.0,
                type_word=0xE000_0001,
                extra=(3.0, 4.0),
                srid=4326,
            ),
            "ewkb",
            "accepted",
            "point",
            "xyzm",
            4326,
            roundtrip=True,
        ),
        Case(
            "empty-linestring",
            geometry(2, u32(0)),
            "either",
            "accepted",
            "linestring",
            "xy",
            roundtrip=True,
        ),
        Case(
            "circularstring-capability-difference",
            line([(0.0, 0.0), (1.0, 1.0)], type_word=8),
            "either",
            "implementation_defined",
            "circularstring",
            "xy",
            difference_dimension="acceptance",
            difference_classification="intentional_capability_difference",
            difference_reason=(
                "database-tools supports extended EWKB curves; "
                "IO-tools supports the seven simple WKB types and rejects the rest"
            ),
        ),
        Case(
            "invalid-trailing-byte",
            point_xy + b"\x00",
            "invalid",
            "rejected",
            expected_error_category="data_mapping",
        ),
        Case(
            "invalid-truncated-point",
            point_xy[:-1],
            "invalid",
            "rejected",
            expected_error_category="data_mapping",
        ),
        Case(
            "invalid-byte-order",
            b"\x02" + point_xy[1:],
            "invalid",
            "rejected",
            expected_error_category="data_mapping",
        ),
        Case(
            "invalid-linestring-count-bomb",
            geometry(2, u32(0xFFFF_FFFF)),
            "invalid",
            "rejected",
            expected_error_category="resource_limit",
            difference_dimension="error_category",
            difference_classification="ambiguous_or_noncanonical_input",
            difference_reason=(
                "the payload simultaneously exceeds the component budget and "
                "is truncated; category precedence is not defined by the ICD"
            ),
        ),
    ]


def case_entry(case: Case) -> dict[str, object]:
    invariants = list(COMMON_INVARIANTS)
    if case.roundtrip:
        invariants.append("roundtrip")
    entry: dict[str, object] = {
        "sha256": hashlib.sha256(case.payload).hexdigest(),
        "path": f"cases/{case.name}.wkb",
        "dialect": case.dialect,
        "expectation": case.expectation,
        "invariants": invariants,
    }
    if case.geometry_type is not None:
        entry["geometry_type"] = case.geometry_type
    if case.dimensions is not None:
        entry["dimensions"] = case.dimensions
    if case.srid is not None:
        entry["srid"] = case.srid
        entry["metadata_srid"] = case.srid
        entry["invariants"] = [*invariants, "srid_coherence"]
    if case.expected_error_category is not None:
        entry["expected_error_category"] = case.expected_error_category
    differences = (
        case.difference_dimension,
        case.difference_classification,
        case.difference_reason,
    )
    if all(value is not None for value in differences):
        entry["known_difference"] = {
            "dimension": case.difference_dimension,
            "classification": case.difference_classification,
            "reason": case.difference_reason,
        }
    return entry


def manifest(io_revision: str, database_revision: str) -> dict[str, object]:
    return {
        "schema_version": 1,
        "corpus_id": "plenora-wkb-ewkb-cross-repository-v1",
        "icd": {"tag": ICD_TAG, "revision": ICD_REVISION},
        "producer_revisions": {
            "plenora-IO-tools": io_revision,
            "plenora-database-tools": database_revision,
        },
        "cases": [case_entry(case) for case in corpus_cases()],
    }


def write_corpus(output: Path, io_revision: str, database_revision: str) -> None:
    cases_dir = output / "cases"
    cases_dir.mkdir(parents=True, exist_ok=True)
    expected_names = set()
    for case in corpus_cases():
        path = cases_dir / f"{case.name}.wkb"
        path.write_bytes(case.payload)
        expected_names.add(path.name)
    for path in cases_dir.iterdir():
        if path.is_file() and path.name not in expected_names:
            path.unlink()
    encoded = json.dumps(
        manifest(io_revision, database_revision),
        ensure_ascii=False,
        indent=2,
        sort_keys=True,
    )
    (output / "manifest.json").write_text(encoded + "\n", encoding="utf-8")


def verify_corpus(output: Path) -> list[str]:
    errors: list[str] = []
    manifest_path = output / "manifest.json"
    try:
        actual_manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        return [f"manifest non leggibile: {exc}"]
    revisions = actual_manifest.get("producer_revisions", {})
    if not isinstance(revisions, dict):
        return ["producer_revisions non è un object"]
    expected = manifest(
        str(revisions.get("plenora-IO-tools", "")),
        str(revisions.get("plenora-database-tools", "")),
    )
    if actual_manifest != expected:
        errors.append("manifest non corrisponde ai casi deterministici")
    expected_paths = set()
    for case in corpus_cases():
        path = output / "cases" / f"{case.name}.wkb"
        expected_paths.add(path)
        try:
            payload = path.read_bytes()
        except OSError as exc:
            errors.append(f"{path.relative_to(output)}: {exc}")
            continue
        if payload != case.payload:
            errors.append(f"{path.relative_to(output)}: payload divergente")
    cases_dir = output / "cases"
    if cases_dir.is_dir():
        extras = sorted(path for path in cases_dir.iterdir() if path not in expected_paths)
        errors.extend(f"caso inatteso: {path.relative_to(output)}" for path in extras)
    return errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--io-revision")
    parser.add_argument("--database-revision")
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()

    if args.check:
        errors = verify_corpus(args.output)
        if errors:
            print("\n".join(errors), file=sys.stderr)
            return 1
        print(f"shared WKB/EWKB corpus verified: {len(corpus_cases())} cases")
        return 0

    if args.io_revision is None or args.database_revision is None:
        parser.error("--io-revision e --database-revision sono obbligatori in generazione")
    write_corpus(args.output, args.io_revision, args.database_revision)
    errors = verify_corpus(args.output)
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print(f"shared WKB/EWKB corpus generated: {len(corpus_cases())} cases")
    return 0


if __name__ == "__main__":
    sys.exit(main())
