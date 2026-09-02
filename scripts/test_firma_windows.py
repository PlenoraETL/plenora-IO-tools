"""Sonde sulla firma Authenticode applicata dal costruttore Windows."""

from __future__ import annotations

import importlib
import pathlib
import shutil
import sys
import tempfile
import unittest


RADICE = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(RADICE / "scripts"))


class SondeFirmaWindows(unittest.TestCase):
    def setUp(self) -> None:
        self.firma = importlib.import_module("firma_windows")
        self.tmp = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        self.exe = self.tmp / "plenora-io.exe"
        self.exe.write_bytes(b"PE non ancora firmato")
        self.signtool = self.tmp / "signtool.exe"
        self.signtool.write_bytes(b"strumento fixture")
        self.impronta = "AB" * 20
        self.ambiente = {
            self.firma.VAR_SIGNTOOL: str(self.signtool),
            self.firma.VAR_CERTIFICATO: self.impronta,
        }

    def misura_valida(self, _: pathlib.Path) -> dict:
        return {
            "firmato": True,
            "firmatario": "CN=Plenora",
            "impronta_firmatario": self.impronta,
            "timestamp": "CN=DigiCert Timestamp",
        }

    def test_prova_non_consulta_ne_strumento_ne_segreti(self) -> None:
        chiamate = []
        stato = self.firma.applica(
            self.exe,
            "prova",
            lambda _: self.fail("non deve misurare"),
            ambiente={},
            esecutore=lambda *a, **k: chiamate.append((a, k)),
            piattaforma="linux",
        )
        self.assertEqual(stato["stato"], "non_richiesta")
        self.assertEqual(chiamate, [])

    def test_candidate_senza_materiale_e_rossa_prima_di_firmare(self) -> None:
        with self.assertRaisesRegex(SystemExit, self.firma.VAR_SIGNTOOL):
            self.firma.applica(
                self.exe,
                "candidate",
                self.misura_valida,
                ambiente={},
                piattaforma="win32",
            )

    def test_candidate_firma_verifica_e_misura_gli_stessi_byte(self) -> None:
        chiamate: list[list[str]] = []

        def esegui(comando: list[str], **_: object) -> None:
            chiamate.append(comando)
            if comando[1] == "sign":
                self.exe.write_bytes(self.exe.read_bytes() + b" firma")

        stato = self.firma.applica(
            self.exe,
            "candidate",
            self.misura_valida,
            ambiente=self.ambiente,
            esecutore=esegui,
            piattaforma="win32",
        )
        self.assertEqual(stato["stato"], "apposta")
        self.assertEqual([c[1] for c in chiamate], ["sign", "verify"])
        firma = chiamate[0]
        self.assertIn("/fd", firma)
        self.assertIn("/td", firma)
        self.assertIn("/tr", firma)
        self.assertIn(self.firma.URL_TIMESTAMP, firma)
        self.assertEqual(firma[-1], str(self.exe))

    def test_la_password_non_puo_entrare_nella_command_line(self) -> None:
        chiamate = []
        ambiente = {**self.ambiente, "PASSWORD_CHE_NON_DEVE_PASSARE": "segreto-unico"}

        def esegui(comando: list[str], **_: object) -> None:
            chiamate.append(comando)
            if comando[1] == "sign":
                self.exe.write_bytes(self.exe.read_bytes() + b" firma")

        self.firma.applica(
            self.exe,
            "candidate",
            self.misura_valida,
            ambiente=ambiente,
            esecutore=esegui,
            piattaforma="win32",
        )
        self.assertNotIn("segreto-unico", " ".join(p for c in chiamate for p in c))

    def test_successo_senza_byte_cambiati_non_e_una_firma(self) -> None:
        with self.assertRaisesRegex(SystemExit, "senza cambiare"):
            self.firma.applica(
                self.exe,
                "candidate",
                self.misura_valida,
                ambiente=self.ambiente,
                esecutore=lambda *a, **k: None,
                piattaforma="win32",
            )

    def test_una_firma_valida_di_un_altro_certificato_e_rossa(self) -> None:
        def esegui(comando: list[str], **_: object) -> None:
            if comando[1] == "sign":
                self.exe.write_bytes(self.exe.read_bytes() + b" firma")

        def misura(_: pathlib.Path) -> dict:
            return {
                **self.misura_valida(self.exe),
                "impronta_firmatario": "CD" * 20,
            }

        with self.assertRaisesRegex(SystemExit, "certificato selezionato"):
            self.firma.applica(
                self.exe,
                "candidate",
                misura,
                ambiente=self.ambiente,
                esecutore=esegui,
                piattaforma="win32",
            )

    def test_timestamp_assente_non_produce_una_candidate(self) -> None:
        def esegui(comando: list[str], **_: object) -> None:
            if comando[1] == "sign":
                self.exe.write_bytes(self.exe.read_bytes() + b" firma")

        def misura(_: pathlib.Path) -> dict:
            return {**self.misura_valida(self.exe), "timestamp": None}

        with self.assertRaisesRegex(SystemExit, "timestamp"):
            self.firma.applica(
                self.exe,
                "candidate",
                misura,
                ambiente=self.ambiente,
                esecutore=esegui,
                piattaforma="win32",
            )


if __name__ == "__main__":
    unittest.main()
