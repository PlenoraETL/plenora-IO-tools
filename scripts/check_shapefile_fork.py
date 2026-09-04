#!/usr/bin/env python3
"""Fail-closed provenance gate for the governed local shapefile fork."""

from __future__ import annotations

import json
import sys
from pathlib import Path
import tomllib

sys.path.insert(0, str(Path(__file__).resolve().parent))

from fork_comune import artefatti_estranei, impronta  # noqa: E402


ROOT = Path(__file__).resolve().parents[1]
LOCK_PATH = ROOT / "scripts" / "shapefile-fork-lock.json"


def fail(message: str) -> None:
    raise SystemExit(f"shapefile fork gate failed: {message}")


def main() -> None:
    lock = json.loads(LOCK_PATH.read_text(encoding="utf-8"))
    expected_keys = {
        "schema_version",
        "package",
        "version",
        "source",
        "crate_sha256",
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
        or lock["package"] != "shapefile"
        or lock["version"] != "0.6.0"
        or lock["source"] != "crates.io"
    ):
        fail("identità upstream inattesa")

    vendor = ROOT / lock["vendor_path"]
    if not vendor.is_dir():
        fail(f"directory vendorizzata assente: {vendor}")
    # L'impronta e' calcolata sul solo insieme versionato, quindi un artefatto
    # di build non puo' cambiarla. Resta pero' un file che non dovrebbe stare
    # in un albero governato, e va nominato: ignorarlo in silenzio sarebbe la
    # meta' sbagliata della difesa.
    estranei = artefatti_estranei(vendor)
    if estranei:
        fail(
            "artefatti estranei nell'albero vendorizzato: "
            + ", ".join(estranei[:5])
            + (" e altri" if len(estranei) > 5 else "")
            + ". Non alterano l'impronta, calcolata sul solo insieme "
            "versionato, ma un albero governato contiene cio' che dichiara e "
            "nient'altro: `cargo package` va eseguito con --target-dir fuori "
            "dal fork."
        )

    count, digest = impronta(vendor)
    if count != lock["file_count"] or digest != lock["tree_sha256"]:
        fail(
            "albero vendorizzato diverso dal lock "
            f"(files={count}, sha256={digest})"
        )

    cargo = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    patch = cargo.get("patch", {}).get("crates-io", {}).get("shapefile")
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
        fail("Cargo.lock non risolve un'unica dipendenza path shapefile 0.6.0")

    # Registro di provenienza **strutturato**. Era un Markdown letto come
    # database: un gate non deve dipendere dalla prosa, che nessuno puo'
    # validare e che si riscrive senza accorgersene.
    registro = json.loads(
        (ROOT / "assurance" / "registries" / "vendor-shapefile-fork.json").read_text(
            encoding="utf-8"
        )
    )
    provenance = json.dumps(registro, ensure_ascii=False)
    for value in (
        lock["crate_sha256"],
        lock["upstream_revision"],
    ):
        if value not in provenance:
            fail("registro di provenienza incoerente col lock")

    declared_delta = set(lock["functional_delta_files"]) | set(
        lock["packaging_delta_files"]
    )
    if any(not (vendor / relative).is_file() for relative in declared_delta):
        fail("un file delta dichiarato è assente")

    print(
        "shapefile fork verificato: "
        f"{lock['package']} {lock['version']}, {count} file, sha256={digest}"
    )


if __name__ == "__main__":
    main()
