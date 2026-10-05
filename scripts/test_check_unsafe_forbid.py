"""Le sonde di `check_unsafe_forbid.py`: ogni modo di perdere il `forbid`."""

from __future__ import annotations

import json
import pathlib
import tempfile
import unittest

from scripts import check_unsafe_forbid as gate

WORKSPACE = '[workspace]\nmembers = ["crates/*"]\n\n[workspace.lints.rust]\nunsafe_code = "{livello}"\n'
CRATE = '[package]\nname = "{nome}"\n\n{lints}[dependencies]\n{dipendenze}'
EREDITA = "[lints]\nworkspace = true\n\n"


class IlGate(unittest.TestCase):
    def setUp(self) -> None:
        self._temporanea = tempfile.TemporaryDirectory(prefix="plenora-unsafe-")
        self.radice = pathlib.Path(self._temporanea.name)
        self.scrivi_registro([{"crate_del_workspace": "plenora-io-py", "dipendenza": "pyo3"}])
        self.scrivi_workspace("forbid")
        self.scrivi_crate("plenora-io-model", EREDITA, "", "pub fn f() {}\n")
        self.scrivi_crate(
            "plenora-io-py", EREDITA, 'pyo3 = "=0.29.2"\n', "#[pyfunction]\nfn g() {}\n"
        )

    def tearDown(self) -> None:
        self._temporanea.cleanup()

    def scrivi_workspace(self, livello: str) -> None:
        (self.radice / "Cargo.toml").write_text(WORKSPACE.format(livello=livello), encoding="utf-8")

    def scrivi_registro(self, voci: list[dict]) -> None:
        cartella = self.radice / "assurance" / "registries"
        cartella.mkdir(parents=True, exist_ok=True)
        (cartella / "dependency-exceptions.json").write_text(
            json.dumps({"accettate": [], "unsafe_nelle_dipendenze": voci}), encoding="utf-8"
        )

    def scrivi_crate(self, nome: str, lints: str, dipendenze: str, sorgente: str) -> None:
        crate = self.radice / "crates" / nome
        (crate / "src").mkdir(parents=True, exist_ok=True)
        (crate / "Cargo.toml").write_text(
            CRATE.format(nome=nome, lints=lints, dipendenze=dipendenze), encoding="utf-8"
        )
        (crate / "src" / "lib.rs").write_text(sorgente, encoding="utf-8")

    def test_il_caso_buono_passa(self) -> None:
        self.assertEqual(gate.problemi(self.radice), [])

    def test_deny_non_basta(self) -> None:
        self.scrivi_workspace("deny")
        self.assertEqual(len(gate.problemi(self.radice)), 1)

    def test_un_crate_che_non_eredita_i_lint(self) -> None:
        self.scrivi_crate("plenora-io-py", "", 'pyo3 = "=0.29.2"\n', "fn g() {}\n")
        self.assertIn("non eredita", " ".join(gate.problemi(self.radice)))

    def test_un_blocco_unsafe_scritto(self) -> None:
        self.scrivi_crate("plenora-io-model", EREDITA, "", "fn f() { unsafe { x() } }\n")
        self.assertIn("lib.rs:1", " ".join(gate.problemi(self.radice)))

    def test_un_allow_locale(self) -> None:
        self.scrivi_crate("plenora-io-model", EREDITA, "", "#[allow(unsafe_code)]\nfn f() {}\n")
        self.assertIn("lib.rs:1", " ".join(gate.problemi(self.radice)))

    def test_la_parola_in_un_commento_non_conta(self) -> None:
        self.scrivi_crate(
            "plenora-io-model",
            EREDITA,
            "",
            '// niente unsafe { qui }\nconst M: &str = "unsafe fn";\n',
        )
        self.assertEqual(gate.problemi(self.radice), [])

    def test_una_deroga_dichiarata_e_non_pubblicabile(self) -> None:
        registro = self.radice / "assurance" / "registries" / "dependency-exceptions.json"
        dati = json.loads(registro.read_text(encoding="utf-8"))
        dati["deroghe_al_forbid"] = [{"crate_del_workspace": "plenora-bench"}]
        registro.write_text(json.dumps(dati), encoding="utf-8")
        crate = self.radice / "crates" / "plenora-bench"
        (crate / "src").mkdir(parents=True)
        (crate / "src" / "main.rs").write_text("unsafe fn f() {}\n", encoding="utf-8")
        (crate / "Cargo.toml").write_text(
            '[package]\nname = "plenora-bench"\npublish = false\n', encoding="utf-8"
        )
        self.assertEqual(gate.problemi(self.radice), [])
        (crate / "Cargo.toml").write_text('[package]\nname = "plenora-bench"\n', encoding="utf-8")
        self.assertIn("pubblicabile", " ".join(gate.problemi(self.radice)))

    def test_pyo3_senza_voce_nel_registro(self) -> None:
        self.scrivi_registro([])
        self.assertIn("pyo3", " ".join(gate.problemi(self.radice)))


if __name__ == "__main__":
    unittest.main()
