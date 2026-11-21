"""Sorgenti senza percorso (L-IO1): `Bytes`, `Archivio`, `elenca_archivio`.

I byte viaggiano sullo stdin del processo o nella memoria del modulo nativo,
mai negli argomenti; gli errori di nomi, ZIP e limiti sono quelli del core,
con gli stessi codici della CLI.
"""

from __future__ import annotations

import asyncio
import inspect
import io
import os
import shutil
import unittest
import zipfile

from plenora_io import (
    Archivio,
    AsyncClient,
    Bytes,
    ElencoArchivio,
    InvalidArgumentError,
    InvalidConfigurationError,
    LimitiArchivio,
    NotFoundError,
    ResourceLimitError,
    UnsupportedError,
    VoceArchivio,
)
from plenora_io._client import OPERAZIONI, SUPERFICI_LOCALI
from plenora_io._discovery import NOME, VARIABILE
from plenora_io._sources import ingresso, source_args

from _nativo import client_di_prova, variante_nativa
from _repository import RADICE, serve_le_fixture


def _zip(voci: dict[str, bytes]) -> bytes:
    uscita = io.BytesIO()
    with zipfile.ZipFile(uscita, "w", zipfile.ZIP_DEFLATED) as archivio:
        for nome, dati in voci.items():
            archivio.writestr(nome, dati)
    return uscita.getvalue()


class GliArgomenti(unittest.TestCase):
    """La riga di comando: nessun byte, nessun flag ripetuto, tipi esatti."""

    def test_i_byte_non_entrano_negli_argomenti(self) -> None:
        sorgente = Bytes("x.geojson", b"{}" * 100_000)
        argv = source_args(sorgente)
        self.assertEqual(argv, ["x.geojson", "--source-stdin"])
        self.assertIs(ingresso(sorgente), sorgente.dati)
        self.assertIs(ingresso(Archivio(sorgente, "a.csv")), sorgente.dati)
        self.assertIsNone(ingresso("x.csv"))

    def test_i_limiti_compaiono_una_volta(self) -> None:
        limiti = LimitiArchivio(max_voci=3)
        argv = source_args(Archivio("a.zip", "b.csv", limiti))
        self.assertEqual(argv.count("--archive-max-entries"), 1)
        argv = source_args("a.zip", limiti)
        self.assertEqual(argv.count("--archive-max-entries"), 1)
        with self.assertRaises(InvalidArgumentError):
            source_args(Archivio("a.zip", "b.csv", limiti), limiti)

    def test_i_tipi_sbagliati_sono_errori_dell_sdk(self) -> None:
        for costruzione in (
            lambda: Bytes("x", "testo"),  # type: ignore[arg-type]
            lambda: Bytes(1, b"x"),  # type: ignore[arg-type]
            lambda: Archivio(Archivio("a.zip", "b"), "c"),
            lambda: Archivio("a.zip", "b", limiti=3),  # type: ignore[arg-type]
            lambda: LimitiArchivio(max_voci="3"),  # type: ignore[arg-type]
            lambda: LimitiArchivio(max_voci=1 << 64),
        ):
            with self.assertRaises(InvalidArgumentError) as preso:
                costruzione()
            self.assertEqual(preso.exception.category, "invalid_configuration")

    def test_i_campi_dei_limiti_sono_la_tabella_del_core(self) -> None:
        self.assertEqual(
            LimitiArchivio().argv(),
            ["--archive-max-entries", "10000", "--archive-max-bytes", str(512 << 20),
             "--archive-max-ratio", "100"],
        )

    def test_elenca_archivio_e_un_comando_locale(self) -> None:
        self.assertNotIn("elenca_archivio", OPERAZIONI)
        self.assertEqual(SUPERFICI_LOCALI, {"elenca_archivio": "archive-list"})
        self.assertTrue(inspect.iscoroutinefunction(AsyncClient.elenca_archivio))


@serve_le_fixture
class ContrIlBinarioVero(unittest.TestCase):
    """Le stesse sorgenti, dal prodotto: stessi esiti di un percorso."""

    @classmethod
    def setUpClass(cls) -> None:
        indicato = os.environ.get(VARIABILE)
        cls.binario = indicato or shutil.which(NOME)
        if cls.binario is None:
            costruito = RADICE / "target" / "debug" / NOME
            if not costruito.is_file():
                costruito = costruito.with_suffix(".exe")
            cls.binario = str(costruito) if costruito.is_file() else None

    def setUp(self) -> None:
        if self.binario is None:
            self.skipTest("nessun binario plenora-io")
        fixture = RADICE / "crates/plenora-io-tools/tests/fixtures/canoniche"
        self.geojson = (fixture / "canonico.geojson").read_bytes()
        self.kml = (fixture / "canonico.kml").read_bytes()

    def client(self):
        return client_di_prova(self.binario)

    def test_byte_e_voci_si_leggono_come_il_file(self) -> None:
        client = self.client()
        dal_file = client.validate(
            RADICE / "crates/plenora-io-tools/tests/fixtures/canoniche/canonico.geojson")
        da_byte = client.validate(Bytes("inesistente.geojson", self.geojson))
        zip_ = Bytes("d.zip", _zip({"Dir/A.geojson": self.geojson}))
        da_voce = client.validate(Archivio(zip_, "dir/a.geojson"))
        self.assertEqual(dal_file.rows_read, da_byte.rows_read)
        self.assertEqual(dal_file.rows_read, da_voce.rows_read)
        self.assertEqual(client.inspect(Bytes("x.geojson", self.geojson)).format.id, "geojson")
        self.assertEqual(client.layers(Bytes("m.kmz", _zip({"doc.kml": self.kml}))).format, "kml")

    def test_elenca_archivio_rende_il_modello(self) -> None:
        client = self.client()
        elenco = client.elenca_archivio(
            Bytes("d.zip", _zip({"a.csv": b"x\n1\n", "b/c.csv": b"y\n"})),
            limiti=LimitiArchivio(max_voci=50),
        )
        self.assertIsInstance(elenco, ElencoArchivio)
        self.assertIsInstance(elenco.entries[0], VoceArchivio)
        self.assertEqual(elenco.entries[0].name, "a.csv")
        self.assertEqual(elenco.entries[0].declared_uncompressed_bytes, 4)
        self.assertEqual(elenco.limits[0].quota, "archive_max_entries")
        self.assertEqual(elenco.limits[0].limit, 50)
        with self.assertRaises(InvalidArgumentError):
            client.elenca_archivio(Archivio("a.zip", "b.zip"))

    def test_gli_errori_hanno_i_codici_della_cli(self) -> None:
        client = self.client()
        zip_ = Bytes("d.zip", _zip({"a.geojson": self.geojson,
                                    "m.kmz": _zip({"doc.kml": self.kml})}))
        for sorgente, classe, codice in (
            (Archivio(zip_, "manca.geojson"), NotFoundError, "SOURCE_NOT_FOUND"),
            (Archivio(zip_, "../a.geojson"), InvalidConfigurationError, "CLI_USAGE"),
            (Bytes("CON.geojson", self.geojson), InvalidConfigurationError, "CLI_USAGE"),
            (Archivio(zip_, "m.kmz"), UnsupportedError, "UNSUPPORTED"),
            (Archivio(zip_, "a.geojson", LimitiArchivio(max_voci=0)),
             InvalidConfigurationError, "CLI_USAGE"),
            (Archivio(zip_, "a.geojson", LimitiArchivio(max_voci=1)),
             ResourceLimitError, "LIMIT_EXCEEDED"),
        ):
            with self.assertRaises(classe) as preso:
                client.validate(sorgente)
            self.assertEqual(preso.exception.code, codice)

    def test_il_client_async_usa_la_stessa_via(self) -> None:
        async def principale():
            client = AsyncClient(None if self.binario.startswith("<") else self.binario)
            return await client.elenca_archivio(Bytes("d.zip", _zip({"a.csv": b"x\n"})))

        if self.binario.startswith("<"):
            self.skipTest("la variante nativa la esercita il client sincrono")
        self.assertEqual(asyncio.run(principale()).entries[0].name, "a.csv")


class ConIlModuloNativo(variante_nativa(ContrIlBinarioVero)):  # type: ignore[misc]
    """I byte restano in memoria nel modulo nativo: stessi esiti."""


if __name__ == "__main__":
    unittest.main()
