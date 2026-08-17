"""Sonde negative del gate sullo scope della coverage (INFRA-0.1).

Un gate che non fallisce mai non e' un gate. Queste sonde costruiscono un
workspace minimo con lo scope rispettato, verificano che passi, poi
introducono un indebolimento per volta e verificano che venga intercettato.

L'albero e' finto apposta: provare le sonde mutando i file veri lascerebbe il
repository sporco se un test si interrompe a meta', ed e' proprio la
condizione in cui il gate `readiness` diventa rosso senza una causa leggibile.
"""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from scripts.check_coverage_exclusions import (
    crate_non_libreria,
    regex_canonica,
    verifica,
    verifica_report,
)

ESCLUSIONE = r"(^|/)(attrezzo-bench|attrezzo-cli)/src/.*\.rs$"

WORKFLOW = f"""jobs:
  coverage:
    steps:
      - name: Measure coverage
        run: cargo llvm-cov --workspace --all-targets --locked --no-report
      - name: Export LCOV report
        run: >-
          cargo llvm-cov report --lcov --output-path lcov.info
          --ignore-filename-regex '{ESCLUSIONE}'
      - name: Verify coverage exclusions
        run: python3 scripts/check_coverage_exclusions.py --lcov lcov.info
      - name: Enforce library coverage threshold
        run: >-
          cargo llvm-cov report --summary-only
          --ignore-filename-regex '{ESCLUSIONE}'
          --fail-under-lines 80
"""

ALBERO = {
    ".github/workflows/ci.yml": WORKFLOW,
    "Dockerfile.dev": "FROM rust:1.92-slim-bookworm\n",
    "crates/libro/Cargo.toml": '[package]\nname = "libro"\n',
    "crates/libro/src/lib.rs": "pub fn niente() {}\n",
    "crates/quaderno/Cargo.toml": '[package]\nname = "quaderno"\n',
    "crates/quaderno/src/lib.rs": "pub fn niente() {}\n",
    "crates/attrezzo-bench/Cargo.toml": '[package]\nname = "attrezzo-bench"\n',
    "crates/attrezzo-bench/src/main.rs": "fn main() {}\n",
    "crates/attrezzo-bench/src/bin/spola.rs": "fn main() {}\n",
    "crates/attrezzo-cli/Cargo.toml": '[package]\nname = "attrezzo-cli"\n',
    "crates/attrezzo-cli/src/main.rs": "fn main() {}\n",
}

LCOV = """SF:/work/crates/libro/src/lib.rs
DA:1,1
end_of_record
SF:/work/crates/quaderno/src/lib.rs
DA:1,1
end_of_record
"""


class SondeScopeCoverage(unittest.TestCase):
    def albero(self, sostituzioni: dict[str, str] | None = None) -> Path:
        """Scrive l'albero di riferimento, con i file indicati rimpiazzati."""
        radice = Path(tempfile.mkdtemp())
        self.addCleanup(self._rimuovi, radice)
        contenuti = dict(ALBERO)
        for relativo, valore in (sostituzioni or {}).items():
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

    def con_workflow(self, vecchio: str, nuovo: str) -> Path:
        self.assertIn(vecchio, WORKFLOW, "la sonda non trova cio' che vuole mutare")
        return self.albero({".github/workflows/ci.yml": WORKFLOW.replace(vecchio, nuovo)})

    def con_lcov(self, radice: Path, testo: str) -> Path:
        percorso = radice / "lcov.info"
        percorso.write_text(testo, encoding="utf-8")
        return percorso

    # --- l'albero conforme passa ----------------------------------------

    def test_albero_conforme_non_produce_errori(self) -> None:
        radice = self.albero()
        self.assertEqual(verifica(radice, self.con_lcov(radice, LCOV)), [])

    def test_regex_derivata_dal_workspace(self) -> None:
        radice = self.albero()
        self.assertEqual(crate_non_libreria(radice), ("attrezzo-bench", "attrezzo-cli"))
        self.assertEqual(regex_canonica(radice), ESCLUSIONE)

    def test_una_sezione_lib_basta_a_fare_una_libreria(self) -> None:
        """`[lib]` con path esplicito, senza `src/lib.rs`: resta nel denominatore."""
        radice = self.albero()
        cartella = radice / "crates" / "agenda"
        (cartella / "src").mkdir(parents=True)
        (cartella / "Cargo.toml").write_text(
            '[package]\nname = "agenda"\n\n[lib]\npath = "src/api.rs"\n', encoding="utf-8"
        )
        (cartella / "src" / "api.rs").write_text("pub fn niente() {}\n", encoding="utf-8")
        self.assertNotIn("agenda", crate_non_libreria(radice))

    # --- indebolimenti della regex --------------------------------------

    def test_regex_che_nomina_solo_main_rs(self) -> None:
        """L'errore corretto da INFRA-0.1: esclude un file per crate, non la crate."""
        radice = self.con_workflow(
            ESCLUSIONE, r"(^|/)(attrezzo-bench|attrezzo-cli)/src/main\.rs$"
        )
        self.assertTrue(verifica(radice))

    def test_regex_che_dimentica_una_crate_binaria(self) -> None:
        radice = self.con_workflow(ESCLUSIONE, r"(^|/)(attrezzo-bench)/src/.*\.rs$")
        self.assertTrue(verifica(radice))

    def test_regex_che_esclude_anche_una_libreria(self) -> None:
        radice = self.con_workflow(
            ESCLUSIONE, r"(^|/)(attrezzo-bench|attrezzo-cli|quaderno)/src/.*\.rs$"
        )
        self.assertTrue(verifica(radice))

    def test_crate_binaria_nuova_non_ancora_esclusa(self) -> None:
        """La regex non cambia, il workspace si': la divergenza e' la stessa."""
        radice = self.albero()
        (radice / "crates" / "attrezzo-nuovo" / "src").mkdir(parents=True)
        (radice / "crates" / "attrezzo-nuovo" / "Cargo.toml").write_text(
            '[package]\nname = "attrezzo-nuovo"\n', encoding="utf-8"
        )
        (radice / "crates" / "attrezzo-nuovo" / "src" / "main.rs").write_text(
            "fn main() {}\n", encoding="utf-8"
        )
        self.assertTrue(verifica(radice))

    def test_esclusione_divergente_nell_immagine(self) -> None:
        radice = self.albero(
            {
                "Dockerfile.dev": "FROM rust:1.92-slim-bookworm\n"
                "# cargo llvm-cov report --summary-only "
                "--ignore-filename-regex '(^|/)attrezzo-bench/src/main\\.rs$'\n"
            }
        )
        self.assertTrue(verifica(radice))

    # --- cancellazioni: il flag sparisce invece di divergere -------------

    def test_esclusione_cancellata_dall_export(self) -> None:
        radice = self.con_workflow(
            "          cargo llvm-cov report --lcov --output-path lcov.info\n"
            f"          --ignore-filename-regex '{ESCLUSIONE}'\n",
            "          cargo llvm-cov report --lcov --output-path lcov.info\n",
        )
        self.assertTrue(verifica(radice))

    def test_esclusione_cancellata_dalla_soglia(self) -> None:
        radice = self.con_workflow(
            f"          --ignore-filename-regex '{ESCLUSIONE}'\n          --fail-under-lines 80\n",
            "          --fail-under-lines 80\n",
        )
        self.assertTrue(verifica(radice))

    def test_soglia_cancellata(self) -> None:
        radice = self.con_workflow("\n          --fail-under-lines 80", "")
        self.assertTrue(verifica(radice))

    def test_export_lcov_cancellato(self) -> None:
        radice = self.con_workflow("--lcov --output-path lcov.info", "--summary-only")
        self.assertTrue(verifica(radice))

    # --- la soglia si muove insieme alla misura -------------------------

    def test_soglia_abbassata(self) -> None:
        radice = self.con_workflow("--fail-under-lines 80", "--fail-under-lines 70")
        self.assertTrue(verifica(radice))

    def test_soglia_alzata(self) -> None:
        radice = self.con_workflow("--fail-under-lines 80", "--fail-under-lines 90")
        self.assertTrue(verifica(radice))

    # --- osservazione del report ----------------------------------------

    def test_report_con_un_binario_sotto_src_bin(self) -> None:
        """Il caso che ha motivato il gate, osservato sul report invece che sulla regex."""
        radice = self.albero()
        percorso = self.con_lcov(
            radice,
            LCOV + "SF:/work/crates/attrezzo-bench/src/bin/spola.rs\nDA:1,0\nend_of_record\n",
        )
        errori = verifica_report(radice, percorso)
        self.assertTrue(any("attrezzo-bench" in messaggio for messaggio in errori))

    def test_report_con_un_main_rs(self) -> None:
        radice = self.albero()
        percorso = self.con_lcov(
            radice,
            LCOV + "SF:/work/crates/attrezzo-cli/src/main.rs\nDA:1,0\nend_of_record\n",
        )
        self.assertTrue(verifica_report(radice, percorso))

    def test_report_senza_una_libreria(self) -> None:
        radice = self.albero()
        percorso = self.con_lcov(
            radice, "SF:/work/crates/libro/src/lib.rs\nDA:1,1\nend_of_record\n"
        )
        errori = verifica_report(radice, percorso)
        self.assertTrue(any("quaderno" in messaggio for messaggio in errori))

    def test_report_vuoto(self) -> None:
        radice = self.albero()
        self.assertTrue(verifica_report(radice, self.con_lcov(radice, "")))

    def test_report_assente(self) -> None:
        radice = self.albero()
        self.assertTrue(verifica_report(radice, radice / "lcov.info"))

    def test_percorsi_relativi_e_separatori_windows(self) -> None:
        """Il report puo' arrivare con percorsi relativi o con backslash."""
        radice = self.albero()
        percorso = self.con_lcov(
            radice,
            "SF:crates/libro/src/lib.rs\nDA:1,1\nend_of_record\n"
            "SF:crates\\quaderno\\src\\lib.rs\nDA:1,1\nend_of_record\n"
            "SF:crates\\attrezzo-cli\\src\\main.rs\nDA:1,0\nend_of_record\n",
        )
        errori = verifica_report(radice, percorso)
        self.assertEqual(len(errori), 1)
        self.assertIn("attrezzo-cli", errori[0])


if __name__ == "__main__":
    unittest.main()
