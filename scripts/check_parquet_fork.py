#!/usr/bin/env python3
"""Gate di provenienza, fail-closed, del fork governato di `parquet`.

Stessa forma dei gate di `gdal`, `shapefile` e `dxf`: l'albero vendorizzato
deve coincidere col lock (impronta sull'insieme versionato, nessun artefatto
estraneo, fine riga come li registra git), il fork deve entrare nel grafo come
dipendenza diretta con nome proprio, e il registro di provenienza deve citare
lo stesso pacchetto pubblicato.

Il fork parte dal fork `-eof` di plenora-data-tools, che a sua volta parte dal
pacchetto crates.io `parquet` 60.0.0: il lock lo dice nel campo `base`, e
`functional_delta_files` elenca l'unione dei due delta.
"""

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
LOCK_PATH = ROOT / "scripts" / "parquet-fork-lock.json"
REGISTRO = ROOT / "assurance" / "registries" / "vendor-parquet-fork.json"


def fail(message: str) -> None:
    raise SystemExit(f"parquet fork gate failed: {message}")


def main() -> None:
    lock = json.loads(LOCK_PATH.read_text(encoding="utf-8"))
    expected_keys = {
        "schema_version",
        "package",
        "fork_package",
        "version",
        "source",
        "crate_sha256",
        "base",
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
        or lock["package"] != "parquet"
        or lock["version"] != "60.0.0"
        or lock["source"] != "crates.io"
    ):
        fail("identità upstream inattesa")

    vendor = ROOT / lock["vendor_path"]
    if not vendor.is_dir():
        fail(f"directory vendorizzata assente: {vendor}")
    estranei = artefatti_estranei(vendor)
    if estranei:
        fail(
            "artefatti estranei nell'albero vendorizzato: "
            + ", ".join(estranei[:5])
            + (" e altri" if len(estranei) > 5 else "")
        )
    divergenti = fini_riga_divergenti(vendor)
    if divergenti:
        fail(
            "file i cui byte sul disco non sono quelli che git registrerebbe: "
            + ", ".join(divergenti[:5])
            + (" e altri" if len(divergenti) > 5 else "")
        )

    count, digest = impronta(vendor)
    if count != lock["file_count"] or digest != lock["tree_sha256"]:
        fail(f"albero vendorizzato diverso dal lock (files={count}, sha256={digest})")

    problemi = problemi_di_risoluzione(lock)
    if problemi:
        fail("; ".join(problemi))

    registro = json.loads(REGISTRO.read_text(encoding="utf-8"))
    provenienza = json.dumps(registro, ensure_ascii=False)
    for valore in (lock["crate_sha256"], lock["base"]["revisione"]):
        if valore not in provenienza:
            fail("registro di provenienza incoerente col lock")
    # Ogni file del delta ha una ragione nel registro: un elenco di nomi senza
    # il perche' non dice che cosa si perde togliendolo.
    spiegati = set(registro.get("delta_funzionale", {}))
    non_spiegati = sorted(set(lock["functional_delta_files"]) - spiegati)
    if non_spiegati:
        fail(f"file del delta senza una ragione nel registro: {non_spiegati}")

    dichiarati = set(lock["functional_delta_files"]) | set(lock["packaging_delta_files"])
    if any(not (vendor / relativo).is_file() for relativo in dichiarati):
        fail("un file delta dichiarato è assente")

    print(
        "parquet fork verificato: "
        f"{lock['package']} {lock['version']}, {count} file, sha256={digest}"
    )


if __name__ == "__main__":
    main()
