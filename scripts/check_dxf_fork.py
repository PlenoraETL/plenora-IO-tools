#!/usr/bin/env python3
"""Fail-closed provenance gate for the governed local dxf fork."""

from __future__ import annotations

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from fork_comune import (  # noqa: E402
    artefatti_estranei,
    fini_riga_divergenti,
    impronta,
    problemi_di_risoluzione,
)


ROOT = Path(__file__).resolve().parents[1]
LOCK_PATH = ROOT / "scripts" / "dxf-fork-lock.json"


def fail(message: str) -> None:
    raise SystemExit(f"DXF fork gate failed: {message}")


def main() -> None:
    lock = json.loads(LOCK_PATH.read_text(encoding="utf-8"))
    expected_keys = {
        "schema_version",
        "package",
        "fork_package",
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

    # Dipendenza diretta per percorso, con nome proprio, e nessuna
    # `[patch]` nei manifesti di workspace: vedi `fork_comune`.
    problemi = problemi_di_risoluzione(lock)
    if problemi:
        fail("; ".join(problemi))

    # Registro di provenienza **strutturato**. Era un Markdown letto come
    # database: un gate non deve dipendere dalla prosa, che nessuno puo'
    # validare e che si riscrive senza accorgersene.
    registro = json.loads(
        (ROOT / "assurance" / "registries" / "vendor-dxf-fork.json").read_text(
            encoding="utf-8"
        )
    )
    provenance = json.dumps(registro, ensure_ascii=False)
    for value in (
        lock["crate_sha256"],
        lock["upstream_tag"],
        lock["upstream_tag_object"],
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
        "DXF fork verificato: "
        f"{lock['package']} {lock['version']}, {count} file, sha256={digest}"
    )


if __name__ == "__main__":
    main()
