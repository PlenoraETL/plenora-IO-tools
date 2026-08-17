"""Sonde negative del gate anti-divergenza dei pin di toolchain (INFRA-0).

Un gate che non fallisce mai non e' un gate. Queste sonde costruiscono un
albero minimo con i pin allineati, verificano che passi, poi introducono una
divergenza per volta e verificano che venga intercettata.

L'albero e' finto apposta: provare le sonde mutando i file veri lascerebbe il
repository sporco se un test si interrompe a meta', ed e' proprio la condizione
in cui il gate `readiness` diventa rosso senza una causa leggibile.
"""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from scripts.check_toolchain_pins import verifica

PINS = """
PLENORA_RUST_STABLE=1.92.0
PLENORA_RUST_NIGHTLY=nightly-2026-07-21
PLENORA_CARGO_FUZZ_VERSION=0.13.2
PLENORA_CARGO_LLVM_COV_VERSION=0.9.0
"""

ALBERO = {
    "scripts/toolchain-pins.env": PINS,
    "rust-toolchain.toml": '[toolchain]\nchannel = "1.92.0"\n',
    "Dockerfile.dev": (
        "FROM rust:1.92-slim-bookworm\n"
        "ARG PLENORA_RUST_NIGHTLY=nightly-2026-07-21\n"
        "ARG PLENORA_CARGO_FUZZ_VERSION=0.13.2\n"
        "ARG PLENORA_CARGO_LLVM_COV_VERSION=0.9.0\n"
    ),
    "fuzz/Dockerfile": (
        "FROM rust:1.92-slim-bookworm\n"
        "ARG PLENORA_RUST_NIGHTLY=nightly-2026-07-21\n"
        "ARG PLENORA_CARGO_FUZZ_VERSION=0.13.2\n"
        "ENV PLENORA_FUZZ_TOOLCHAIN=nightly-2026-07-21\n"
    ),
    ".github/workflows/ci.yml": (
        "jobs:\n"
        "  fuzz:\n"
        "    env:\n"
        "      RUSTUP_TOOLCHAIN: nightly-2026-07-21\n"
        "    steps:\n"
        '      - uses: setup-rust\n        with:\n          toolchain: "nightly-2026-07-21"\n'
        "      - run: cargo install cargo-fuzz --locked --version 0.13.2\n"
        "  coverage:\n"
        "    steps:\n"
        '      - uses: setup-rust\n        with:\n          toolchain: "1.92.0"\n'
        "      - uses: install-action\n        with:\n          tool: cargo-llvm-cov@0.9.0\n"
    ),
    "scripts/fuzz-smoke.sh": 'toolchain="${PLENORA_FUZZ_TOOLCHAIN:-nightly-2026-07-21}"\n',
    "scripts/fuzz-campaign.sh": 'toolchain="${PLENORA_FUZZ_TOOLCHAIN:-nightly-2026-07-21}"\n',
}


class SondeDivergenza(unittest.TestCase):
    def albero(self, **sostituzioni: str) -> Path:
        """Scrive l'albero di riferimento, con i file indicati rimpiazzati."""
        radice = Path(tempfile.mkdtemp())
        self.addCleanup(self._rimuovi, radice)
        contenuti = dict(ALBERO)
        for chiave, valore in sostituzioni.items():
            relativo = chiave.replace("__", "/").replace("_dot_", ".")
            self.assertIn(relativo, contenuti, "sostituzione di un file non nell'albero")
            contenuti[relativo] = valore
        for relativo, testo in contenuti.items():
            percorso = radice / relativo
            percorso.parent.mkdir(parents=True, exist_ok=True)
            percorso.write_text(testo, encoding="utf-8")
        return radice

    @staticmethod
    def _rimuovi(radice: Path) -> None:
        for percorso in sorted(radice.rglob("*"), reverse=True):
            if percorso.is_file():
                percorso.unlink()
            else:
                percorso.rmdir()
        radice.rmdir()

    def sostituisci(self, relativo: str, vecchio: str, nuovo: str) -> Path:
        testo = ALBERO[relativo]
        self.assertIn(vecchio, testo, "la sonda non trova cio' che vuole mutare")
        chiave = relativo.replace("/", "__").replace(".", "_dot_")
        return self.albero(**{chiave: testo.replace(vecchio, nuovo)})

    # --- l'albero allineato passa ---------------------------------------

    def test_albero_allineato_non_produce_errori(self) -> None:
        self.assertEqual(verifica(self.albero()), [])

    # --- divergenze: ogni consumatore, ogni famiglia --------------------

    def test_nightly_divergente_in_ci(self) -> None:
        radice = self.sostituisci(
            ".github/workflows/ci.yml", 'toolchain: "nightly-2026-07-21"', 'toolchain: "nightly-2026-09-01"'
        )
        self.assertTrue(verifica(radice))

    def test_nightly_divergente_nello_smoke(self) -> None:
        radice = self.sostituisci(
            "scripts/fuzz-smoke.sh", "nightly-2026-07-21", "nightly-2025-01-01"
        )
        self.assertTrue(verifica(radice))

    def test_nightly_divergente_nella_campagna(self) -> None:
        radice = self.sostituisci(
            "scripts/fuzz-campaign.sh", "nightly-2026-07-21", "nightly-2026-07-22"
        )
        self.assertTrue(verifica(radice))

    def test_nightly_divergente_nel_dockerfile_fuzz(self) -> None:
        radice = self.sostituisci(
            "fuzz/Dockerfile",
            "ENV PLENORA_FUZZ_TOOLCHAIN=nightly-2026-07-21",
            "ENV PLENORA_FUZZ_TOOLCHAIN=nightly-2026-08-01",
        )
        self.assertTrue(verifica(radice))

    def test_cargo_fuzz_divergente_in_ci(self) -> None:
        radice = self.sostituisci(
            ".github/workflows/ci.yml",
            "cargo install cargo-fuzz --locked --version 0.13.2",
            "cargo install cargo-fuzz --locked --version 0.14.0",
        )
        self.assertTrue(verifica(radice))

    def test_cargo_fuzz_divergente_nell_immagine(self) -> None:
        radice = self.sostituisci(
            "Dockerfile.dev",
            "ARG PLENORA_CARGO_FUZZ_VERSION=0.13.2",
            "ARG PLENORA_CARGO_FUZZ_VERSION=0.12.0",
        )
        self.assertTrue(verifica(radice))

    def test_cargo_llvm_cov_divergente_in_ci(self) -> None:
        radice = self.sostituisci(
            ".github/workflows/ci.yml", "cargo-llvm-cov@0.9.0", "cargo-llvm-cov@0.8.7"
        )
        self.assertTrue(verifica(radice))

    def test_cargo_llvm_cov_divergente_nell_immagine(self) -> None:
        radice = self.sostituisci(
            "Dockerfile.dev",
            "ARG PLENORA_CARGO_LLVM_COV_VERSION=0.9.0",
            "ARG PLENORA_CARGO_LLVM_COV_VERSION=0.8.7",
        )
        self.assertTrue(verifica(radice))

    def test_stable_divergente_nel_toolchain_toml(self) -> None:
        radice = self.sostituisci(
            "rust-toolchain.toml", 'channel = "1.92.0"', 'channel = "1.93.0"'
        )
        self.assertTrue(verifica(radice))

    def test_immagine_base_divergente(self) -> None:
        radice = self.sostituisci(
            "Dockerfile.dev", "FROM rust:1.92-slim-bookworm", "FROM rust:1.91-slim-bookworm"
        )
        self.assertTrue(verifica(radice))

    def test_stable_divergente_in_un_job_ci(self) -> None:
        radice = self.sostituisci(
            ".github/workflows/ci.yml", 'toolchain: "1.92.0"', 'toolchain: "1.91.0"'
        )
        self.assertTrue(verifica(radice))

    # --- cancellazioni: il pin sparisce invece di divergere -------------

    def test_pin_cancellato_dallo_smoke(self) -> None:
        radice = self.sostituisci(
            "scripts/fuzz-smoke.sh",
            'toolchain="${PLENORA_FUZZ_TOOLCHAIN:-nightly-2026-07-21}"',
            'toolchain="${PLENORA_FUZZ_TOOLCHAIN:?}"',
        )
        self.assertTrue(verifica(radice))

    def test_pin_cancellato_dall_immagine(self) -> None:
        radice = self.sostituisci(
            "Dockerfile.dev", "ARG PLENORA_CARGO_LLVM_COV_VERSION=0.9.0", ""
        )
        self.assertTrue(verifica(radice))

    def test_fonte_unica_mancante(self) -> None:
        radice = self.albero()
        (radice / "scripts" / "toolchain-pins.env").unlink()
        self.assertTrue(verifica(radice))


if __name__ == "__main__":
    unittest.main()
