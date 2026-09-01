"""Regression tests for the immutable GitHub Action reference gate."""

from __future__ import annotations

import unittest

from scripts.check_action_pins import required_tool_input, validate_reference


class ValidateReferenceTests(unittest.TestCase):
    def test_accepts_full_commit_sha(self) -> None:
        reference = "actions/checkout@" + ("a" * 40)
        self.assertIsNone(validate_reference(reference))

    def test_accepts_local_action(self) -> None:
        self.assertIsNone(validate_reference("./.github/actions/local"))

    def test_accepts_docker_digest(self) -> None:
        reference = "docker://example.invalid/tool@sha256:" + ("b" * 64)
        self.assertIsNone(validate_reference(reference))

    def test_rejects_mutable_action_tag(self) -> None:
        self.assertIsNotNone(validate_reference("actions/checkout@v4"))

    def test_rejects_mutable_action_branch(self) -> None:
        self.assertIsNotNone(validate_reference("owner/action@main"))

    def test_rejects_missing_revision(self) -> None:
        self.assertIsNotNone(validate_reference("owner/action"))

    def test_rejects_short_sha(self) -> None:
        self.assertIsNotNone(validate_reference("owner/action@deadbeef"))

    def test_rejects_mutable_docker_tag(self) -> None:
        self.assertIsNotNone(validate_reference("docker://example.invalid/tool:latest"))

    def test_install_action_sha_requires_explicit_tool(self) -> None:
        reference = "taiki-e/install-action@" + ("a" * 40)
        lines = [
            f"      - uses: {reference}",
            "      - name: Next step",
        ]
        self.assertIsNotNone(required_tool_input(reference, lines, 0))

    def test_install_action_accepts_explicit_tool(self) -> None:
        reference = "taiki-e/install-action@" + ("a" * 40)
        lines = [
            f"      - uses: {reference}",
            "        with:",
            "          tool: cargo-llvm-cov",
            "      - name: Next step",
        ]
        self.assertIsNone(required_tool_input(reference, lines, 0))

    def test_other_actions_do_not_require_tool_input(self) -> None:
        reference = "actions/checkout@" + ("a" * 40)
        self.assertIsNone(required_tool_input(reference, [f"- uses: {reference}"], 0))


if __name__ == "__main__":
    unittest.main()


class SondeDellaVerificaOnline(unittest.TestCase):
    """La verifica facoltativa che interroga GitHub.

    Il difetto che la motiva l'ho introdotto io: scrivendo il workflow di
    distribuzione ho fissato uno SHA di `actions/download-artifact` a memoria, e
    quello SHA non e' mai esistito. Il gate lo ha accettato, perche' verifica la
    forma e non l'esistenza -- quaranta cifre esadecimali qualunque passano.

    Queste sonde provano la **mappatura delle risposte**, non la rete: un test
    che chiamasse GitHub sarebbe rosso quando la rete e' lenta, e un test che
    diventa rosso per ragioni sue smette di essere creduto.
    """

    def setUp(self) -> None:
        import importlib.util
        import pathlib

        percorso = pathlib.Path(__file__).resolve().parent / "check_action_pins.py"
        spec = importlib.util.spec_from_file_location("pins", percorso)
        self.gate = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.gate)

    def test_uno_sha_inesistente_e_un_difetto(self) -> None:
        for codice in (404, 422):
            with self.subTest(codice=codice):
                diagnosi = self.gate.diagnosi_http(codice, "actions/download-artifact")
                self.assertIsNotNone(diagnosi)
                self.assertIn("non esiste", diagnosi)

    def test_il_rate_limit_non_si_confonde_con_un_pin_sbagliato(self) -> None:
        """Sono due cose diverse, e il messaggio deve dirlo: un 403 e' «non ho
        potuto chiedere», non «il pin e' sbagliato»."""
        diagnosi = self.gate.diagnosi_http(403, "actions/checkout")
        self.assertIsNotNone(diagnosi)
        self.assertNotIn("non esiste", diagnosi)
        self.assertIn("non verificabile", diagnosi)

    def test_non_rispondere_non_e_un_si(self) -> None:
        """Fail-closed: ogni risposta che non sia un 200 lascia il gate rosso."""
        for codice in (403, 404, 422, 500, 503):
            with self.subTest(codice=codice):
                self.assertIsNotNone(self.gate.diagnosi_http(codice, "qualcuno/qualcosa"))
