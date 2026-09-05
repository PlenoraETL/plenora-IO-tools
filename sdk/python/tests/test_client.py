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
    def test_niente_json_dove_il_protocollo_mette_la_busta(self) -> None:
        """Il messaggio nomina il **flusso** atteso, e mostra l'altro.

        Non «non ho trovato JSON»: chi legge deve poter capire se il binario ha
        scritto sul flusso sbagliato o non ha scritto affatto, e sono due
        guasti diversi.
        """
        client = self.client('print("non sono JSON")\nsys.exit(3)\n')
        with self.assertRaises(ProtocolError) as preso:
            client.version()
        self.assertIn("su stderr", str(preso.exception))
        self.assertIn("non sono JSON", str(preso.exception))

    @saltabile
    def test_uscita_a_zero_con_una_busta_che_non_si_dichiara_riuscita(self) -> None:
        """Il protocollo non prevede questa combinazione.

        Passarla oltre farebbe consumare come buono un documento che il
        prodotto non ha dichiarato tale. L'altra meta' -- un'uscita diversa da
        zero con una busta di successo -- la copre `test_process.py`, dove il
        flusso atteso e' l'altro.
        """
        client = self.client(
            'print(json.dumps({"status": "boh", "version": "1.0.0"}))\n'
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


    def test_inspect_si_decodifica_intero(self) -> None:
        cliente = Client(binary=self.binario)
        fixture = RADICE / "crates/plenora-io-cli/tests/fixtures/canoniche/canonico.geojson"
        esito = cliente.inspect(fixture)

        self.assertEqual(esito.contract, "plenora-io-inspect-v2")
        self.assertEqual(esito.protocol_version, 2)
        self.assertEqual(esito.format.id, "geojson")
        self.assertTrue(esito.format.readable and esito.format.writable)

        strato = esito.layers[0]
        self.assertTrue(strato.fields)
        # La geometria sta fra i campi **e** ha una sezione sua: le due viste
        # devono concordare, o una delle due mente.
        geometriche = [c for c in strato.fields if c.geometry]
        self.assertEqual(len(geometriche), 1)
        self.assertEqual(geometriche[0].name, strato.geometry.name)
        self.assertEqual(
            len(strato.attributes) + 1, len(strato.fields), "gli attributi sono il resto"
        )
        self.assertEqual(strato.field(geometriche[0].name).type, geometriche[0].type)

        # Il CRS e da dove viene: `status` distingue un identificatore risolto
        # da uno assunto, e i due hanno lo stesso aspetto in `id`.
        self.assertTrue(strato.geometry.crs)
        self.assertTrue(strato.geometry.crs_resolution.status)
        self.assertEqual(strato.geometry.crs_resolution.id, strato.geometry.crs)

    def test_layers_riassume_senza_lo_schema(self) -> None:
        cliente = Client(binary=self.binario)
        fixture = RADICE / "crates/plenora-io-cli/tests/fixtures/canoniche/canonico.gpkg"
        esito = cliente.layers(fixture)

        self.assertEqual(esito.contract, "plenora-io-layers-v2")
        self.assertEqual(esito.format, "gpkg")
        self.assertTrue(esito.layers)
        for riassunto in esito.layers:
            self.assertGreater(riassunto.field_count, 0)
            self.assertTrue(riassunto.geometry_crs)

    def test_i_due_comandi_concordano_sullo_stesso_file(self) -> None:
        """`layers` e' `inspect` senza lo schema, e i due devono dire lo stesso.

        E' la sonda che rende utile avere entrambi i metodi: se divergessero,
        chi sceglie il piu' economico otterrebbe una risposta diversa da chi
        paga l'inferenza.
        """
        cliente = Client(binary=self.binario)
        fixture = RADICE / "crates/plenora-io-cli/tests/fixtures/canoniche/canonico.gpkg"
        completo = cliente.inspect(fixture)
        riassunto = cliente.layers(fixture)

        self.assertEqual(completo.format.id, riassunto.format)
        self.assertEqual(
            [(s.id, s.name) for s in completo.layers],
            [(s.id, s.name) for s in riassunto.layers],
        )
        for strato, sommario in zip(completo.layers, riassunto.layers):
            self.assertEqual(len(strato.fields), sommario.field_count)
            self.assertEqual(strato.geometry.crs, sommario.geometry_crs)

    def test_la_fedelta_arriva_strutturata(self) -> None:
        cliente = Client(binary=self.binario)
        fixture = RADICE / "crates/plenora-io-cli/tests/fixtures/canoniche/canonico.gpkg"
        fedelta = cliente.layers(fixture).fidelity

        self.assertTrue(fedelta.level)
        self.assertFalse(fedelta.troncato)
        self.assertTrue(fedelta.omesse_esatte)
        self.assertFalse(fedelta.omesse.any, "niente e' stato lasciato fuori")
        # GPKG dichiara una fedelta' condizionata, quindi le ragioni ci sono:
        # una sezione senza ragioni e un livello non esatto sarebbero
        # incoerenti, ed e' cio' che questa sonda distingue.
        self.assertTrue(fedelta.reasons)
        for ragione in fedelta.reasons:
            self.assertTrue(ragione.code and ragione.detail)

    def test_un_crs_che_non_si_risolve_e_un_rifiuto_tipizzato(self) -> None:
        """Il driver rifiuta chiuso invece di indovinare un codice EPSG.

        E' una decisione di prodotto, e l'SDK la porta a chi lo usa come
        categoria: `assume_crs` e' la via per dire «lo so io».
        """
        cliente = Client(binary=self.binario)
        fixture = RADICE / "crates/plenora-io-cli/tests/fixtures/canoniche/canonico_punti.shp"
        with self.assertRaises(CommandFailed) as preso:
            cliente.inspect(fixture)
        self.assertEqual(preso.exception.envelope.category, "crs")

        # E con l'assunzione esplicita il file si legge.
        esito = cliente.inspect(fixture, assume_crs="EPSG:3003")
        self.assertEqual(esito.format.id, "shp")
        self.assertEqual(esito.layers[0].geometry.crs, "EPSG:3003")

    def test_un_comando_che_fallisce_porta_la_busta(self) -> None:
        """La via d'errore, dal prodotto vero e non da un finto.

        Passa da `inspect()`, che e' pubblico: la prima stesura chiamava un
        metodo privato dell'esecutore, e verificava una strada che nessun utente
        dell'SDK percorre.
        """
        cliente = Client(binary=self.binario)
        with self.assertRaises(CommandFailed) as preso:
            cliente.inspect("/non/esiste.geojson")
        self.assertEqual(preso.exception.envelope.phase, "read")
        self.assertTrue(preso.exception.envelope.code)
        # La classe viene dalla categoria, non dal testo: quale sia la
        # categoria lo decide il prodotto, e la sonda non la fissa -- verifica
        # che la classe sollevata sia **quella** che le compete.
        from plenora_io import errors

        attesa = errors.CATEGORIE[preso.exception.envelope.category]
        self.assertIs(type(preso.exception), attesa)


if __name__ == "__main__":
    unittest.main()
