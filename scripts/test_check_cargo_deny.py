"""Le controprove del gate cargo-deny."""

from __future__ import annotations

import json
import pathlib
import sys
import tomllib
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import check_cargo_deny as gate  # noqa: E402


def politica(*ignorati: str, yanked: str = "deny") -> dict:
    return {
        "advisories": {
            "yanked": yanked,
            "ignore": [{"id": i, "reason": "r"} for i in ignorati],
        },
        "sources": {"unknown-registry": "deny", "unknown-git": "deny"},
    }


def registro(*accettati: str) -> dict:
    return {"accettate": [{"id": i} for i in accettati]}


class LaCoerenza(unittest.TestCase):
    def test_gli_stessi_id_passano(self) -> None:
        self.assertEqual(gate.coerenza(politica("A"), registro("A")), [])

    def test_un_ignorato_senza_registro_e_rosso(self) -> None:
        errori = gate.coerenza(politica("A", "B"), registro("A"))
        self.assertTrue(any("«B»" in e and "deny.toml" in e for e in errori), errori)

    def test_un_registrato_non_ignorato_e_rosso(self) -> None:
        errori = gate.coerenza(politica(), registro("A"))
        self.assertTrue(any("«A»" in e for e in errori), errori)

    def test_yanked_va_rifiutato(self) -> None:
        errori = gate.coerenza(politica(yanked="warn"), registro())
        self.assertTrue(any("yanked" in e for e in errori), errori)

    def test_le_sorgenti_sconosciute_vanno_rifiutate(self) -> None:
        d = politica()
        d["sources"]["unknown-git"] = "allow"
        self.assertTrue(gate.coerenza(d, registro()))

    def test_solo_zip_esatto_puo_essere_yanked(self) -> None:
        for spec in ("zip", "zip@8", "zip@^8.6.0", "zip@=8.6.1", "altro@=8.6.0"):
            with self.subTest(spec=spec):
                d = politica()
                d["advisories"]["ignore"].append({"crate": spec, "reason": "r"})
                r = {"yanked_accettati": [{"crate": spec}]}
                self.assertTrue(gate.coerenza(d, r))

    def test_zip_esatto_non_ignora_advisory(self) -> None:
        d = politica("RUSTSEC-2023-0089")
        d["advisories"]["ignore"].append({"crate": "zip@=8.6.0", "reason": "r"})
        r = registro("RUSTSEC-2023-0089")
        r["yanked_accettati"] = [{"crate": "zip@=8.6.0"}]
        self.assertEqual(gate.coerenza(d, r), [])
        self.assertTrue(gate.coerenza(d, registro("RUSTSEC-2023-0089")))
        d["advisories"]["disable-yank-checking"] = True
        self.assertTrue(gate.coerenza(d, r))

    def test_nuovi_consumer_zip8_vietati(self) -> None:
        for nome in ("calamine", "rust_xlsxwriter", "driver-xls", "altro"):
            with self.subTest(nome=nome):
                lock = {"package": [{
                    "name": nome, "dependencies": ["zip 8.6.0"],
                    "source": "registry+https://github.com/rust-lang/crates.io-index",
                }]}
                self.assertEqual(bool(gate.ambito_zip(lock)), nome not in {"calamine", "rust_xlsxwriter"})

    def test_entrambi_i_lockfile_rispettano_ambito(self) -> None:
        for percorso in (gate.ROOT / "Cargo.lock", gate.ROOT / "fuzz/Cargo.lock"):
            self.assertEqual(gate.ambito_zip(tomllib.loads(percorso.read_text())), [])

    def test_la_forma_breve_degli_ignorati(self) -> None:
        d = {"advisories": {"yanked": "deny", "ignore": ["A"]}, "sources": politica()["sources"]}
        self.assertEqual(gate.coerenza(d, registro("A")), [])


class LaVersione(unittest.TestCase):
    def test_il_pin_si_legge(self) -> None:
        self.assertEqual(
            gate.versione_fissata("# x\nPLENORA_CARGO_DENY_VERSION=0.20.2\n"), "0.20.2"
        )
        self.assertIsNone(gate.versione_fissata("ALTRO=1\n"))

    def test_l_uscita_dello_strumento_si_legge_esatta(self) -> None:
        self.assertEqual(gate.versione_installata("cargo-deny 0.20.2\n"), "0.20.2")
        self.assertIsNone(gate.versione_installata("cargo-deny 0.20.2-rc1"))
        self.assertIsNone(gate.versione_installata("cargo-audit 0.20.2"))


class IlRepositoryReale(unittest.TestCase):
    def test_deny_toml_e_il_registro_coincidono(self) -> None:
        politica_reale = tomllib.loads(gate.POLITICA.read_text(encoding="utf-8"))
        registro_reale = json.loads(gate.REGISTRO.read_text(encoding="utf-8"))
        self.assertEqual(gate.coerenza(politica_reale, registro_reale), [])

    def test_il_pin_esiste(self) -> None:
        self.assertIsNotNone(gate.versione_fissata(gate.PIN.read_text(encoding="utf-8")))


if __name__ == "__main__":
    unittest.main()
