#!/usr/bin/env python3
"""Fail-closed provenance gate for the governed local gdal fork."""

from __future__ import annotations

import json
import sys
from pathlib import Path
import tomllib

sys.path.insert(0, str(Path(__file__).resolve().parent))

from fork_comune import artefatti_estranei, fini_riga_divergenti, impronta  # noqa: E402


ROOT = Path(__file__).resolve().parents[1]
LOCK_PATH = ROOT / "scripts" / "gdal-fork-lock.json"


def fail(message: str) -> None:
    raise SystemExit(f"GDAL fork gate failed: {message}")


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
        or lock["package"] != "gdal"
        or lock["version"] != "0.17.1"
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

    # Prima dell'impronta, perche' e' la ragione di uno dei modi in cui
    # l'impronta non torna, e detta dopo sarebbe un indizio invece di una
    # spiegazione.
    divergenti = fini_riga_divergenti(vendor)
    if divergenti:
        fail(
            "file i cui byte sul disco non sono quelli che git registrerebbe: "
            + ", ".join(divergenti[:5])
            + (" e altri" if len(divergenti) > 5 else "")
            + ". `.gitattributes` normalizza i fine riga, quindi l'impronta "
            "calcolata qui non sarebbe riproducibile su un checkout pulito. "
            "Rinormalizza i file (su Windows: un editor li ha riscritti con "
            "CRLF) e ricalcola il lock."
        )

    count, digest = impronta(vendor)
    if count != lock["file_count"] or digest != lock["tree_sha256"]:
        fail(
            "albero vendorizzato diverso dal lock "
            f"(files={count}, sha256={digest})"
        )

    cargo = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    patch = cargo.get("patch", {}).get("crates-io", {}).get("gdal")
    if patch != {"path": lock["vendor_path"]}:
        fail("Cargo.toml non usa esclusivamente il fork governato")

    package = tomllib.loads((vendor / "Cargo.toml").read_text(encoding="utf-8"))[
        "package"
    ]
    if package.get("name") != lock["package"] or package.get("version") != lock["version"]:
        fail("manifest del fork incoerente col lock")

    vcs = json.loads((vendor / ".cargo_vcs_info.json").read_text(encoding="utf-8"))
    if vcs.get("git", {}).get("sha1") != lock["upstream_revision"]:
        fail("revisione upstream nel pacchetto incoerente col lock")

    cargo_lock = tomllib.loads((ROOT / "Cargo.lock").read_text(encoding="utf-8"))
    matches = [
        item
        for item in cargo_lock["package"]
        if item.get("name") == lock["package"] and item.get("version") == lock["version"]
    ]
    if len(matches) != 1 or "source" in matches[0] or "checksum" in matches[0]:
        fail("Cargo.lock non risolve un'unica dipendenza path gdal 0.17.1")

    # Registro di provenienza **strutturato**. Era un Markdown letto come
    # database: un gate non deve dipendere dalla prosa, che nessuno puo'
    # validare e che si riscrive senza accorgersene.
    registro = json.loads(
        (ROOT / "assurance" / "registries" / "vendor-gdal-fork.json").read_text(
            encoding="utf-8"
        )
    )
    provenance = json.dumps(registro, ensure_ascii=False)
    for value in (lock["crate_sha256"], lock["upstream_revision"]):
        if value not in provenance:
            fail("registro di provenienza incoerente col lock")

    print(
        "GDAL fork verificato: "
        f"{lock['package']} {lock['version']}, {count} file, sha256={digest}"
    )


if __name__ == "__main__":
    main()
