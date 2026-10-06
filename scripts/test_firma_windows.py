"""Sonde sulla firma Authenticode con Azure Trusted Signing.

Lo strumento e il servizio sono finti: queste sonde provano le decisioni del
modulo -- quando non fa niente, quando si ferma, che cosa pretende dalla misura
-- non che Azure firmi. Quello lo prova la prima corsa candidate con la
configurazione dell'utente.
"""

from __future__ import annotations

import importlib
import json
import pathlib
import shutil
import sys
import tempfile
import unittest


RADICE = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(RADICE / "scripts"))

SOGGETTO = "CN=Plenora, O=Plenora, C=IT"


class SondeFirmaWindows(unittest.TestCase):
    def setUp(self) -> None:
        self.firma = importlib.import_module("firma_windows")
        self.distribuzione = importlib.import_module("distribuzione")
        self.tmp = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        self.exe = self.tmp / "plenora-io.exe"
        self.exe.write_bytes(b"PE non ancora firmato")
        self.signtool = self.tmp / "signtool.exe"
        self.signtool.write_bytes(b"strumento finto")
        self.dlib = self.tmp / "Azure.CodeSigning.Dlib.dll"
        self.dlib.write_bytes(b"client finto")
        self.metadati = self.tmp / "metadata.json"
        self.metadati.write_text(
            json.dumps(
                {
                    "Endpoint": "https://weu.codesigning.azure.net",
                    "CodeSigningAccountName": "plenora",
                    "CertificateProfileName": "plenora-public",
                    "ExcludeCredentials": ["ManagedIdentityCredential"],
                }
            ),
            encoding="utf-8",
        )
        self.ambiente = {
            "PLENORA_FIRMA_WINDOWS": "authenticode",
            self.firma.VAR_SIGNTOOL: str(self.signtool),
            self.firma.VAR_DLIB: str(self.dlib),
            self.firma.VAR_METADATA: str(self.metadati),
            self.firma.VAR_FIRMATARIO: SOGGETTO,
        }

    def misura_valida(self, _: pathlib.Path) -> dict:
        return {
            "firmato": True,
            "firmatario": SOGGETTO,
            "impronta_firmatario": "AB" * 20,
            "timestamp": "CN=Microsoft Public RSA Time Stamping Authority",
        }

    def esecutore_che_firma(self, chiamate: list[list[str]]):
        def esegui(comando: list[str], **_: object) -> None:
            chiamate.append(comando)
            if comando[1] == "sign":
                self.exe.write_bytes(self.exe.read_bytes() + b" firma")

        return esegui

    def applica(self, **kwargs):
        argomenti = {
            "ambiente": self.ambiente,
            "piattaforma": "win32",
        }
        argomenti.update(kwargs)
        return self.firma.applica(
            self.exe, argomenti.pop("canale", "candidate"), argomenti.pop("misura", self.misura_valida), **argomenti
        )

    # --- quando non fa niente -----------------------------------------------

    def test_senza_configurazione_non_consulta_niente(self) -> None:
        for ambiente in ({}, {"PLENORA_FIRMA_WINDOWS": ""}, {"PLENORA_FIRMA_WINDOWS": "nessuna"}):
            with self.subTest(ambiente=ambiente):
                chiamate: list = []
                stato = self.firma.applica(
                    self.exe,
                    "candidate",
                    lambda _: self.fail("non deve misurare"),
                    ambiente=ambiente,
                    esecutore=lambda *a, **k: chiamate.append(a),
                    piattaforma="linux",
                )
                self.assertEqual(stato["stato"], "non_richiesta")
                self.assertEqual(chiamate, [])

    def test_la_prova_non_firma_nemmeno_con_la_configurazione(self) -> None:
        chiamate: list = []
        stato = self.applica(
            canale="prova",
            misura=lambda _: self.fail("non deve misurare"),
            esecutore=lambda *a, **k: chiamate.append(a),
        )
        self.assertEqual(stato["stato"], "non_richiesta")
        self.assertEqual(chiamate, [])

    def test_un_valore_ignoto_della_modalita_ferma(self) -> None:
        with self.assertRaisesRegex(SystemExit, "PLENORA_FIRMA_WINDOWS"):
            self.firma.applica(
                self.exe,
                "candidate",
                self.misura_valida,
                ambiente={"PLENORA_FIRMA_WINDOWS": "Authenticode"},
                piattaforma="win32",
            )

    # --- la strada che firma -------------------------------------------------

    def test_firma_verifica_e_misura_gli_stessi_byte(self) -> None:
        chiamate: list[list[str]] = []
        stato = self.applica(esecutore=self.esecutore_che_firma(chiamate))
        self.assertEqual(stato["stato"], "apposta")
        self.assertEqual(stato["meccanismo"], "authenticode")
        self.assertEqual([c[1] for c in chiamate], ["sign", "verify"])
        firma = chiamate[0]
        self.assertEqual(firma[firma.index("/dlib") + 1], str(self.dlib))
        self.assertEqual(firma[firma.index("/dmdf") + 1], str(self.metadati))
        self.assertEqual(firma[firma.index("/tr") + 1], self.firma.URL_TIMESTAMP)
        self.assertEqual(firma[firma.index("/fd") + 1], "SHA256")
        self.assertEqual(firma[-1], str(self.exe))

    # --- quando si ferma -----------------------------------------------------

    def test_ogni_pezzo_mancante_ferma_prima_di_firmare(self) -> None:
        for variabile in (
            self.firma.VAR_SIGNTOOL,
            self.firma.VAR_DLIB,
            self.firma.VAR_METADATA,
            self.firma.VAR_FIRMATARIO,
        ):
            with self.subTest(variabile=variabile):
                ambiente = dict(self.ambiente)
                del ambiente[variabile]
                with self.assertRaisesRegex(SystemExit, variabile):
                    self.applica(
                        ambiente=ambiente,
                        esecutore=lambda *a, **k: self.fail("non deve firmare"),
                    )

    def test_fuori_da_windows_non_si_firma(self) -> None:
        with self.assertRaisesRegex(SystemExit, "Windows"):
            self.applica(piattaforma="linux")

    def test_metadati_con_chiavi_estranee_fermano(self) -> None:
        dati = json.loads(self.metadati.read_text(encoding="utf-8"))
        dati["ClientSecret"] = "non deve arrivare al client"
        self.metadati.write_text(json.dumps(dati), encoding="utf-8")
        with self.assertRaisesRegex(SystemExit, "chiavi non previste") as preso:
            self.applica(esecutore=lambda *a, **k: self.fail("non deve firmare"))
        self.assertNotIn("non deve arrivare", str(preso.exception))

    def test_un_endpoint_non_https_ferma(self) -> None:
        dati = json.loads(self.metadati.read_text(encoding="utf-8"))
        dati["Endpoint"] = "http://weu.codesigning.azure.net"
        self.metadati.write_text(json.dumps(dati), encoding="utf-8")
        with self.assertRaisesRegex(SystemExit, "https"):
            self.applica(esecutore=lambda *a, **k: self.fail("non deve firmare"))

    def test_un_successo_che_non_cambia_i_byte_e_rosso(self) -> None:
        with self.assertRaisesRegex(SystemExit, "senza cambiare"):
            self.applica(esecutore=lambda *a, **k: None)

    def test_un_soggetto_diverso_e_rosso_e_non_compare_nel_messaggio(self) -> None:
        def misura(_: pathlib.Path) -> dict:
            return {**self.misura_valida(_), "firmatario": "CN=Qualcun Altro"}

        with self.assertRaisesRegex(SystemExit, "soggetto atteso") as preso:
            self.applica(misura=misura, esecutore=self.esecutore_che_firma([]))
        self.assertNotIn("Qualcun Altro", str(preso.exception))
        self.assertNotIn(SOGGETTO, str(preso.exception))

    def test_senza_timestamp_non_e_apposta(self) -> None:
        def misura(_: pathlib.Path) -> dict:
            return {**self.misura_valida(_), "timestamp": None}

        with self.assertRaisesRegex(SystemExit, "timestamp"):
            self.applica(misura=misura, esecutore=self.esecutore_che_firma([]))

    # --- il gate vede la stessa politica -------------------------------------

    def test_il_gate_pretende_la_firma_solo_con_la_configurazione(self) -> None:
        politica = self.distribuzione.politica_di_firma
        self.assertFalse(politica("windows-x86_64", "candidate", {})["richiesta"])
        self.assertTrue(
            politica("windows-x86_64", "candidate", {"PLENORA_FIRMA_WINDOWS": "authenticode"})[
                "richiesta"
            ]
        )
        # Linux e il canale di prova non cambiano con la configurazione Windows.
        con = {"PLENORA_FIRMA_WINDOWS": "authenticode"}
        self.assertFalse(politica("linux-x86_64", "candidate", con)["richiesta"])
        self.assertFalse(politica("windows-x86_64", "prova", con)["richiesta"])


class SondeLockDelClient(unittest.TestCase):
    """Il pacchetto del client e' fissato per versione e SHA-256."""

    def test_il_lock_e_completo_e_coerente(self) -> None:
        lock = json.loads((RADICE / "scripts" / "trusted-signing-lock.json").read_text(encoding="utf-8"))
        self.assertRegex(lock["sha256"], r"^[0-9a-f]{64}$")
        self.assertIn(f"/{lock['versione']}/", lock["url"])
        self.assertTrue(lock["url"].startswith("https://api.nuget.org/"))
        self.assertEqual(lock["dlib"], "bin/x64/Azure.CodeSigning.Dlib.dll")
        firma = importlib.import_module("firma_windows")
        self.assertEqual(lock["timestamp"], firma.URL_TIMESTAMP)


if __name__ == "__main__":
    unittest.main()
