#!/usr/bin/env python3
"""Fail-closed provenance gate for the governed local dxf fork."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import tomllib


ROOT = Path(__file__).resolve().parents[1]
LOCK_PATH = ROOT / "scripts" / "dxf-fork-lock.json"


def fail(message: str) -> None:
    raise SystemExit(f"DXF fork gate failed: {message}")


def tree_digest(root: Path) -> tuple[int, str]:
    files = sorted(
        (path for path in root.rglob("*") if path.is_file()),
        key=lambda path: path.relative_to(root).as_posix(),
    )
    digest = hashlib.sha256()
    for path in files:
        relative = path.relative_to(root).as_posix()
        digest.update(relative.encode("utf-8"))
        digest.update(b"\0")
        digest.update(hashlib.sha256(path.read_bytes()).hexdigest().encode("ascii"))
        digest.update(b"\n")
    return len(files), digest.hexdigest()


def main() -> None:
    lock = json.loads(LOCK_PATH.read_text(encoding="utf-8"))
    expected_keys = {
        "schema_version",
        "package",
        "version",
        "source",
        "crate_sha256",
        "upstream_tag",
        "upstream_tag_object",
        "upstream_revision",
        "vendor_path",
        "file_count",
        "tree_sha256",
        "functional_delta_files",
        "packaging_delta_files",
    }
    if set(lock) != expected_keys:
        fail("schema del lock inatteso")
    if (
        lock["schema_version"] != 1
        or lock["package"] != "dxf"
        or lock["version"] != "0.6.1"
        or lock["source"] != "upstream_git_tag_and_crates_io_release"
    ):
        fail("identità upstream inattesa")

    vendor = ROOT / lock["vendor_path"]
    if not vendor.is_dir():
        fail(f"directory vendorizzata assente: {vendor}")
    count, digest = tree_digest(vendor)
    if count != lock["file_count"] or digest != lock["tree_sha256"]:
        fail(
            "albero vendorizzato diverso dal lock "
            f"(files={count}, sha256={digest})"
        )

    cargo = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    patch = cargo.get("patch", {}).get("crates-io", {}).get("dxf")
    if patch != {"path": lock["vendor_path"]}:
        fail("Cargo.toml non usa esclusivamente il fork governato")

    package = tomllib.loads((vendor / "Cargo.toml").read_text(encoding="utf-8"))[
        "package"
    ]
    if package.get("name") != lock["package"] or package.get("version") != lock["version"]:
        fail("manifest del fork incoerente col lock")

    cargo_lock = tomllib.loads((ROOT / "Cargo.lock").read_text(encoding="utf-8"))
    matches = [
        item
        for item in cargo_lock["package"]
        if item.get("name") == lock["package"] and item.get("version") == lock["version"]
    ]
    if len(matches) != 1 or "source" in matches[0] or "checksum" in matches[0]:
        fail("Cargo.lock non risolve un'unica dipendenza path dxf 0.6.1")

    provenance = (vendor / "PLENORA_FORK.md").read_text(encoding="utf-8")
    for value in (
        lock["crate_sha256"],
        lock["upstream_tag"],
        lock["upstream_tag_object"],
        lock["upstream_revision"],
    ):
        if value not in provenance:
            fail("registro umano di provenienza incoerente col lock")

    declared_delta = set(lock["functional_delta_files"]) | set(
        lock["packaging_delta_files"]
    )
    if any(not (vendor / relative).is_file() for relative in declared_delta):
        fail("un file delta dichiarato è assente")

    print(
        "DXF fork verificato: "
        f"{lock['package']} {lock['version']}, {count} file, sha256={digest}"
    )


if __name__ == "__main__":
    main()
