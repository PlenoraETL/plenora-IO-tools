"""Il client, contro un binario finto e contro quello vero.

# Perche' un binario finto

Le sonde devono poter descrivere risposte che il prodotto vero non produce a
comando: un codice d'uscita diverso da zero con una busta di successo, un JSON
malformato, il silenzio su tutti e due i flussi. Sono le condizioni in cui il
client deve fallire chiuso, e costruirle con il binario vero vorrebbe dire
guastarlo.

# E perche' anche quello vero

Un finto risponde come lo si e' scritto: verifica il client, non il contratto.
La sonda d'integrazione esegue il binario **reale** quando c'e', e guarda che le
due buste di questo ciclo si decodifichino davvero. Senza, l'SDK sarebbe
verificato contro la propria idea del prodotto.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

from plenora_io import Client, CommandFailed, ProtocolError
from plenora_io.discovery import NOME, VARIABILE

RADICE = Path(__file__).resolve().parents[3]


def finto(directory: Path, corpo: str) -> Path:
    """Un «binario» che e' uno script Python: risponde come gli si dice."""
    percorso = directory / NOME
    percorso.write_text(
        "#!/usr/bin/env python3\nimport sys, json\n" + corpo, encoding="utf-8"
    )
    percorso.chmod(0o755)
    return percorso


class ConUnFinto(unittest.TestCase):
    def setUp(self) -> None:
        self._ambiente = dict(os.environ)
        os.environ.pop(VARIABILE, None)
        self._temporanea = TemporaryDirectory(prefix="plenora-sdk-client-")
        self.tmp = Path(self._temporanea.name)

    def tearDown(self) -> None:
        os.environ.clear()
        os.environ.update(self._ambiente)
        self._temporanea.cleanup()

    def client(self, corpo: str) -> Client:
        return Client(binary=finto(self.tmp, corpo))

    # Su Windows uno script senza interprete associato non si esegue: le sonde
    # che ne hanno bisogno lo dicono invece di fallire per una ragione che non
    # riguarda cio' che verificano.
    saltabile = unittest.skipIf(
        sys.platform == "win32", "il finto e' uno script POSIX"
    )

    @saltabile
    def test_la_busta_di_successo_arriva_da_stdout(self) -> None:
        client = self.client(
            'print(json.dumps({"status": "ok", "version": "9.9.9"}))\n'
        )
        self.assertEqual(client.version().version, "9.9.9")

    @saltabile
    def test_una_busta_d_errore_su_stderr_diventa_un_eccezione(self) -> None:
        client = self.client(
            'print(json.dumps({"status": "error", "protocol_version": 1,'
            ' "contract": "plenora-io-error-v1", "error": {"code": "FORMAT_ERROR",'
            ' "category": "io", "phase": "read", "remote_effect": "none",'
            ' "retry": {"kind": "never"}, "message": "niente da leggere"}}),'
            " file=sys.stderr)\nsys.exit(1)\n"
        )
        with self.assertRaises(CommandFailed) as preso:
            client.catalog()
        errore = preso.exception
        self.assertEqual(errore.exit_code, 1)
        self.assertEqual(errore.envelope.category, "io")
        self.assertEqual(errore.envelope.phase, "read")
        self.assertFalse(errore.retryable)
        # Gli assi arrivano interi, e non riassunti in una stringa: sono la
        # sola informazione machine-readable che la busta porta.
        self.assertEqual(errore.envelope.retry, {"kind": "never"})

    @saltabile
    def test_un_ritentativo_con_ritardo_resta_leggibile(self) -> None:
        client = self.client(
            'print(json.dumps({"status": "error", "error": {"code": "X",'
            ' "category": "transient", "phase": "connect", "remote_effect": "none",'
            ' "retry": {"kind": "after", "delay_ms": 2750}, "message": "riprova"}}),'
            " file=sys.stderr)\nsys.exit(1)\n"
        )
        with self.assertRaises(CommandFailed) as preso:
            client.catalog()
        self.assertTrue(preso.exception.retryable)
        self.assertEqual(preso.exception.envelope.retry["delay_ms"], 2750)

    @saltabile
    def test_niente_json_su_nessuno_dei_due_flussi(self) -> None:
        client = self.client('print("non sono JSON")\nsys.exit(3)\n')
        with self.assertRaises(ProtocolError) as preso:
            client.version()
        self.assertIn("senza una busta JSON", str(preso.exception))

    @saltabile
    def test_uscita_diversa_da_zero_con_una_busta_di_successo(self) -> None:
        """Il protocollo non prevede questa combinazione.

        Passarla oltre farebbe consumare come buono un documento che il
        prodotto non ha dichiarato tale.
        """
        client = self.client(
            'print(json.dumps({"status": "ok", "version": "1.0.0"}))\nsys.exit(7)\n'
        )
        with self.assertRaises(ProtocolError) as preso:
            client.version()
        self.assertIn("non prevede questa combinazione", str(preso.exception))

    @saltabile
    def test_il_manifesto_e_none_per_un_binario_nudo(self) -> None:
        client = self.client('print(json.dumps({"status": "ok", "version": "1"}))\n')
        self.assertIsNone(client.manifest)


class ContrIlBinarioVero(unittest.TestCase):
    """L'integrazione: le due buste di questo ciclo, dal prodotto."""

    @classmethod
    def setUpClass(cls) -> None:
        indicato = os.environ.get(VARIABILE)
        cls.binario = indicato or shutil.which(NOME)
        if cls.binario is None:
            costruito = RADICE / "target" / "debug" / NOME
            cls.binario = str(costruito) if costruito.is_file() else None

    def setUp(self) -> None:
        if self.binario is None:
            self.skipTest(
                "nessun binario plenora-io: la sonda d'integrazione non ha "
                "niente da esercitare"
            )

    def test_la_versione_si_decodifica(self) -> None:
        versione = Client(binary=self.binario).version()
        self.assertEqual(versione.status, "ok")
        self.assertTrue(versione.version)

    def test_il_catalogo_si_decodifica_intero(self) -> None:
        catalogo = Client(binary=self.binario).catalog()
        self.assertEqual(catalogo.contract, "plenora-io-catalog-v2")
        self.assertEqual(catalogo.protocol_version, 2)
        self.assertTrue(catalogo.drivers)
        # I dieci driver del prodotto: il numero non e' fissato qui -- lo fissa
        # il catalogo -- ma che ce ne sia piu' d'uno e che ciascuno si
        # decodifichi senza campi mancanti e' cio' che questa sonda afferma.
        for driver in catalogo.drivers:
            self.assertTrue(driver.id)
            self.assertIn(driver.direction, ("bidirectional", "read_only", "write_only"))

    def test_un_comando_che_fallisce_porta_la_busta(self) -> None:
        """La via d'errore, dal prodotto vero e non da un finto."""
        cliente = Client(binary=self.binario)
        with self.assertRaises(CommandFailed) as preso:
            cliente._esegui(["read", "/non/esiste.geojson"])
        self.assertEqual(preso.exception.envelope.phase, "read")
        self.assertTrue(preso.exception.envelope.code)


if __name__ == "__main__":
    unittest.main()
