#!/usr/bin/env python3
"""Enforce the ratified cross-component identity constraints R8.1/R8.4."""

from __future__ import annotations

import re
import sys
import tomllib
from collections import Counter
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
EXPECTED_MODEL_PACKAGE = "plenora-io-model"
FORBIDDEN_PACKAGE_NAMES = {"plenora-core"}
FORBIDDEN_PUBLIC_IDENTITY = re.compile(r"\bPlenoraError\b")


def validate_identity(root: Path) -> tuple[list[str], int, int]:
    manifests = sorted((root / "crates").glob("*/Cargo.toml"))
    package_names: list[tuple[Path, str]] = []
    errors: list[str] = []

    for manifest in manifests:
        with manifest.open("rb") as stream:
            document = tomllib.load(stream)
        name = document.get("package", {}).get("name")
        if not isinstance(name, str) or not name:
            errors.append(f"{manifest.relative_to(root)}: package.name assente")
            continue
        package_names.append((manifest, name))
        if name in FORBIDDEN_PACKAGE_NAMES:
            errors.append(
                f"{manifest.relative_to(root)}: nome package trasversale "
                f"riservato/collidente: {name}"
            )

    counts = Counter(name for _, name in package_names)
    for name, count in sorted(counts.items()):
        if count > 1:
            errors.append(f"package duplicato nel workspace: {name} ({count})")
    if counts[EXPECTED_MODEL_PACKAGE] != 1:
        errors.append(
            f"atteso esattamente un package {EXPECTED_MODEL_PACKAGE}, "
            f"trovati {counts[EXPECTED_MODEL_PACKAGE]}"
        )

    source_files = sorted((root / "crates").glob("*/src/**/*.rs"))
    for source in source_files:
        for line_number, line in enumerate(
            source.read_text(encoding="utf-8").splitlines(), start=1
        ):
            if FORBIDDEN_PUBLIC_IDENTITY.search(line):
                errors.append(
                    f"{source.relative_to(root)}:{line_number}: "
                    "identità pubblica collidente PlenoraError"
                )

    return errors, len(manifests), len(source_files)


def main() -> int:
    errors, manifest_count, source_count = validate_identity(ROOT)
    if errors:
        print("Public identity gate failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1

    print(
        "Public identity gate passed "
        f"({manifest_count} manifests, {source_count} Rust sources)."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
