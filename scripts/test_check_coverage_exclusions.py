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
    ancore_feature_gated,
    crate_non_libreria,
    regex_canonica,
    verifica,
    verifica_report,
    verifica_workflow,
)

ESCLUSIONE = r"(^|/)(attrezzo-bench|attrezzo-cli)/src/.*\.rs$"

WORKFLOW = f"""jobs:
  coverage:
    steps:
      - name: Measure coverage
        run: cargo llvm-cov --workspace --all-targets --all-features --locked --no-report
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

# `libro` porta **due** ancore -- riga 4 e riga 10 -- e tutti i casi che la
# prima stesura dell'estrattore non vedeva o vedeva male:
#
#   4  una funzione dietro `#[cfg(feature = ...)]`, il caso semplice;
#   10 una funzione dentro un modulo dietro `cfg(all(..., feature = ...))`:
#      l'attributo non e' la forma semplice, e la prima riga del modulo e' un
#      `use`, non una funzione;
#   14 un ramo **negativo**: nomina la feature e vale quando non c'e', quindi
#      ancorarci sopra pretenderebbe strumentata la riga che `--all-features`
#      fa sparire;
#   17 un blocco che non comincia con una funzione: non ancorabile, e va
#      dichiarato invece che finto verificato;
#   21 un modulo `cfg(test)`: fuori misura, perche' la soglia sorveglia il
#      codice di produzione.
SORGENTE_CON_FEATURE = """pub fn sempre() {}

#[cfg(feature = "extra")]
pub fn solo_con_extra() {}

#[cfg(all(unix, feature = "extra"))]
mod dentro {
    use std::fmt;

    pub fn annidata() {}
}

#[cfg(not(feature = "extra"))]
pub fn senza_extra() {}

#[cfg(feature = "extra")]
thread_local! {
    static CONTO: u8 = const { 0 };
}

#[cfg(all(test, feature = "extra"))]
mod prove {
    #[cfg(feature = "extra")]
    fn aiuto() {}
}
"""

# Gli altri due misuratori. La verifica delle feature era **globale**: con tre
# misuratori bastavano gli altri due a tenerla verde mentre uno smetteva di
# misurare, e senza questi file nella fixture le sonde non potevano accorgersene.
MISURA_NELLO_SCRIPT = """#!/usr/bin/env bash
cargo llvm-cov clean --workspace
cargo llvm-cov --workspace --all-targets --all-features --locked --no-report
"""

ALBERO = {
    ".github/workflows/ci.yml": WORKFLOW,
    "scripts/s9-checkpoint.sh": MISURA_NELLO_SCRIPT,
    "scripts/campagne_copertura.sh": MISURA_NELLO_SCRIPT,
    "Dockerfile.dev": "FROM rust:1.92-slim-bookworm\n",
    "crates/libro/Cargo.toml": '[package]\nname = "libro"\n',
    "crates/libro/src/lib.rs": SORGENTE_CON_FEATURE,
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
DA:4,1
DA:10,1
end_of_record
SF:/work/crates/quaderno/src/lib.rs
DA:1,1
end_of_record
"""

# Lo stesso report, senza le righe dietro la feature: e' cio' che
# `cargo llvm-cov` produce quando la misura non porta `--all-features`.
LCOV_SENZA_FEATURE = """SF:/work/crates/libro/src/lib.rs
DA:1,1
end_of_record
SF:/work/crates/quaderno/src/lib.rs
DA:1,1
end_of_record
"""

# Una **sola** delle due ancore. E' la forma che il gate lasciava passare: gli
# bastava che l'elenco delle raggiunte non fosse vuoto, quindi una crate con
# venti blocchi dietro due feature restava verde con una feature compilata.
LCOV_MEZZA_FEATURE = """SF:/work/crates/libro/src/lib.rs
DA:1,1
DA:4,1
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
            "SF:crates/libro/src/lib.rs\nDA:1,1\nDA:4,1\nDA:10,1\nend_of_record\n"
            "SF:crates\\quaderno\\src\\lib.rs\nDA:1,1\nend_of_record\n"
            "SF:crates\\attrezzo-cli\\src\\main.rs\nDA:1,0\nend_of_record\n",
        )
        errori = verifica_report(radice, percorso)
        self.assertEqual(len(errori), 1)
        self.assertIn("attrezzo-cli", errori[0])


class SondeDelCodiceDietroUnaFeature(unittest.TestCase):
    """Il denominatore comprende il codice feature-gated, o non e' la libreria.

    Il gate guardava che ogni crate libreria comparisse nel report, e una crate
    compare anche quando se ne misura la meta': `driver-filegdb` tiene l'intero
    percorso GDAL dietro `gdal-backend`, e senza quella feature cinquecento
    righe di produzione restavano fuori dalla soglia -- non «scoperte», ma
    invisibili.
    """

    def setUp(self) -> None:
        temporanea = tempfile.TemporaryDirectory()
        self.addCleanup(temporanea.cleanup)
        self.radice = Path(temporanea.name)
        for relativo, testo in ALBERO.items():
            percorso = self.radice / relativo
            percorso.parent.mkdir(parents=True, exist_ok=True)
            percorso.write_text(testo, encoding="utf-8")

    def con_lcov(self, testo: str) -> Path:
        percorso = self.radice / "lcov.info"
        percorso.write_text(testo, encoding="utf-8")
        return percorso

    def test_un_report_con_il_codice_feature_gated_e_verde(self) -> None:
        self.assertEqual(verifica_report(self.radice, self.con_lcov(LCOV)), [])

    def test_un_report_senza_il_codice_feature_gated_e_rosso(self) -> None:
        """E' cio' che `cargo llvm-cov` produce senza `--all-features`: la crate
        c'e', il suo codice dietro la feature no."""
        errori = verifica_report(self.radice, self.con_lcov(LCOV_SENZA_FEATURE))
        self.assertTrue(any("senza `--all-features`" in m for m in errori), errori)
        self.assertTrue(any("libro" in m for m in errori))

    def test_una_sola_ancora_su_due_non_basta(self) -> None:
        """Il modo in cui la verifica era piu' debole della sua promessa: le
        bastava **una** ancora raggiunta per crate."""
        errori = verifica_report(self.radice, self.con_lcov(LCOV_MEZZA_FEATURE))
        self.assertTrue(any("1 blocchi su 2" in m for m in errori), errori)
        self.assertTrue(any("lib.rs:10" in m for m in errori), errori)

    def test_le_ancore_si_derivano_dal_sorgente(self) -> None:
        """Non sono scritte nel gate: vengono dagli attributi che il codice
        porta, cosi' un blocco nuovo entra nella verifica da solo."""
        ancore, _ = ancore_feature_gated(self.radice)
        self.assertIn("libro", ancore)
        self.assertEqual(
            ancore["libro"],
            [
                ("crates/libro/src/lib.rs", 4, "extra"),
                ("crates/libro/src/lib.rs", 10, "extra"),
            ],
        )
        self.assertNotIn("quaderno", ancore)

    def test_un_ramo_negativo_non_e_un_ancora(self) -> None:
        """`#[cfg(not(feature = ...))]` nomina la feature e vale quando **non**
        c'e': ancorarci sopra pretenderebbe strumentata la riga che
        `--all-features` fa sparire."""
        ancore, non_ancorati = ancore_feature_gated(self.radice)
        righe = [r for _, r, _ in ancore["libro"]] + [r for _, r, _ in non_ancorati]
        self.assertNotIn(14, righe)

    def test_un_blocco_che_non_e_una_funzione_e_dichiarato(self) -> None:
        """Non ha una riga di cui llvm-cov garantisca il record. Il gate lo
        conta invece di far finta di guardarlo."""
        _, non_ancorati = ancore_feature_gated(self.radice)
        self.assertEqual(non_ancorati, [("crates/libro/src/lib.rs", 16, "extra")])

    def test_i_moduli_di_prova_restano_fuori(self) -> None:
        """La soglia sorveglia il codice di produzione, e un helper di prova che
        nessuno chiama puo' non avere alcun record a feature compilata: e'
        successo su `opzioni_scrittura` in `driver-filegdb`."""
        ancore, non_ancorati = ancore_feature_gated(self.radice)
        righe = [r for _, r, _ in ancore["libro"]] + [r for _, r, _ in non_ancorati]
        self.assertNotIn(21, righe)
        self.assertNotIn(24, righe)

    def test_la_misura_deve_portare_tutte_le_feature(self) -> None:
        """Il comando che **misura**, non quello che riporta: e' il primo a
        decidere che cosa entra nel denominatore."""
        percorso = self.radice / ".github/workflows/ci.yml"
        percorso.write_text(
            WORKFLOW.replace(
                "--all-targets --all-features --locked", "--all-targets --locked"
            ),
            encoding="utf-8",
        )
        errori = verifica_workflow(self.radice)
        self.assertTrue(any("--all-features" in m for m in errori), errori)

    def test_un_solo_misuratore_senza_la_misura_e_rosso(self) -> None:
        """La verifica era **globale**: con tre misuratori, cancellare la misura
        da uno solo lasciava gli altri due a tenerla verde."""
        for relativo in ("scripts/s9-checkpoint.sh", "scripts/campagne_copertura.sh"):
            with self.subTest(relativo):
                percorso = self.radice / relativo
                originale = percorso.read_text(encoding="utf-8")
                percorso.write_text(
                    originale.replace("cargo llvm-cov --workspace", "echo salto"),
                    encoding="utf-8",
                )
                errori = verifica_workflow(self.radice)
                percorso.write_text(originale, encoding="utf-8")
                self.assertTrue(
                    any(relativo in m and "nessuna invocazione" in m for m in errori),
                    errori,
                )

    def test_un_solo_misuratore_senza_le_feature_e_rosso(self) -> None:
        percorso = self.radice / "scripts/campagne_copertura.sh"
        percorso.write_text(
            MISURA_NELLO_SCRIPT.replace(" --all-features", ""), encoding="utf-8"
        )
        errori = verifica_workflow(self.radice)
        self.assertTrue(
            any("campagne_copertura.sh" in m and "--all-features" in m for m in errori),
            errori,
        )

    def test_un_misuratore_assente_e_rosso(self) -> None:
        """Finche' e' fra i misuratori, la sua misura fa parte della promessa."""
        (self.radice / "scripts/s9-checkpoint.sh").unlink()
        errori = verifica_workflow(self.radice)
        self.assertTrue(
            any("s9-checkpoint.sh" in m and "assente" in m for m in errori), errori
        )

    def test_senza_nessuna_invocazione_di_misura_e_rosso(self) -> None:
        percorso = self.radice / ".github/workflows/ci.yml"
        percorso.write_text(
            WORKFLOW.replace("cargo llvm-cov --workspace", "echo salto"),
            encoding="utf-8",
        )
        errori = verifica_workflow(self.radice)
        self.assertTrue(
            any("nessuna invocazione" in m for m in errori), errori
        )


if __name__ == "__main__":
    unittest.main()
