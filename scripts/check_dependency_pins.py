#!/usr/bin/env python3
"""Fail if a direct dependency is not reproducibly declared."""

from __future__ import annotations

import sys
import tomllib
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
DEPENDENCY_TABLES = {
    "dependencies",
    "dev-dependencies",
    "build-dependencies",
}


def dependency_tables(value: Any, path: tuple[str, ...] = ()):
    if not isinstance(value, dict):
        return
    for key, child in value.items():
        child_path = (*path, key)
        if key in DEPENDENCY_TABLES and isinstance(child, dict):
            yield child_path, child
        else:
            yield from dependency_tables(child, child_path)


def validate_dependency(
    manifest: Path, table: tuple[str, ...], name: str, specification: Any
) -> list[str]:
    location = f"{manifest.relative_to(ROOT)} [{'.'.join(table)}] {name}"
    if isinstance(specification, str):
        return [] if specification.startswith("=") else [f"{location}: pin non esatto"]
    if not isinstance(specification, dict):
        return [f"{location}: dichiarazione non riconosciuta"]

    version = specification.get("version")
    if version is not None and (
        not isinstance(version, str) or not version.startswith("=")
    ):
        return [f"{location}: pin di versione non esatto"]

    if "git" in specification:
        if "rev" not in specification or "branch" in specification or "tag" in specification:
            return [f"{location}: dipendenza git senza solo rev immutabile"]
        return []

    if version is None and not (
        specification.get("workspace") is True or "path" in specification
    ):
        return [f"{location}: manca versione, workspace o path"]
    return []


def main() -> int:
    manifests = [ROOT / "Cargo.toml"]
    manifests.extend(sorted((ROOT / "crates").glob("*/Cargo.toml")))
    manifests.append(ROOT / "fuzz" / "Cargo.toml")
    errors: list[str] = []

    for manifest in manifests:
        with manifest.open("rb") as stream:
            document = tomllib.load(stream)
        for table, dependencies in dependency_tables(document):
            for name, specification in dependencies.items():
                errors.extend(
                    validate_dependency(manifest, table, name, specification)
                )

    if errors:
        print("Dependency pin gate failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1
    print(f"Dependency pin gate passed ({len(manifests)} manifests).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
