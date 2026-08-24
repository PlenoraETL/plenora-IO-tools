"""Sonde del verbale di riproducibilita' della fixture FileGDB.

L'invariante dice «la fixture e' un FileGDB vero e riproducibile». La prima
meta' la verifica la forma dell'archivio; la seconda **non si puo'** verificare
senza GDAL e due rigenerazioni, e un gate che si limitasse a rileggere
l'archivio proverebbe la prima spacciandola per entrambe.

Cio' che il gate rilegge e' percio' il **verbale** del confronto, legato ai byte
della fixture che descrive. Queste sonde provano che il legame tiene: un verbale
di un'altra fixture, o senza i fatti che lo rendono un verbale, e' rosso.
"""

from __future__ import annotations

import hashlib
import io
import json
import pathlib
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout

from scripts import genera_fixture_filegdb as gate


class SondeDelNomeDiParte(unittest.TestCase):
    """La stessa regola del target, e per la stessa ragione.

    Il nome finisce in un `join`: qui protegge chi **costruisce** l'archivio, in
    `driver-filegdb` protegge chi lo **legge**. La prima stesura ammetteva
    `".."`, perche' era fatto di soli caratteri ammessi.
    """

    def test_i_nomi_veri_passano(self) -> None:
        for nome in ("gdb", "timestamps", "a00000001.gdbtable", "a00000009.spx"):
            self.assertTrue(gate.nome_di_parte_ammesso(nome), nome)

    def test_un_percorso_non_e_un_nome_di_parte(self) -> None:
        for nome in ("..", ".", "", "../fuori", "a/b", "a\\b", "MAIUSCOLO", "con spazio"):
            self.assertFalse(gate.nome_di_parte_ammesso(nome), repr(nome))

    def test_un_nome_che_non_comincia_per_lettera_e_rifiutato(self) -> None:
        """E' la condizione che rende `".."` rosso: senza, sarebbe fatto di soli
        caratteri ammessi."""
        for nome in (".nascosto", "1abc", "_interno"):
            self.assertFalse(gate.nome_di_parte_ammesso(nome), nome)

    def test_lo_spacchettamento_rifiuta_un_nome_ostile(self) -> None:
        """La regola vale anche in lettura: un archivio corrotto non deve poter
        far scrivere fuori dalla directory."""
        ostile = gate.INTESTAZIONE + b"\x01\x00\x00\x00" + b"\x02\x00" + b".." + b"\x01\x00\x00\x00" + b"x"
        with self.assertRaises(ValueError):
            gate.spacchetta(ostile)


class SondeDelVerbale(unittest.TestCase):
    """Il verbale descrive **questa** fixture, o non descrive niente."""

    def setUp(self) -> None:
        temporanea = tempfile.TemporaryDirectory()
        self.addCleanup(temporanea.cleanup)
        self.finto = pathlib.Path(temporanea.name) / "fixture-filegdb.json"
        precedente = gate.PROVA
        gate.PROVA = self.finto
        self.addCleanup(setattr, gate, "PROVA", precedente)

    def verbale(self, **cambi) -> dict:
        base = {
            "schema_version": 1,
            "versione_gdal": "GDAL 3.6.2, released 2023/01/02",
            "impronta_della_fixture": hashlib.sha256(
                gate.ARCHIVIO.read_bytes()
            ).hexdigest(),
            "parti": len(gate.spacchetta(gate.ARCHIVIO.read_bytes())),
            "byte_coniati_totali": 96,
        }
        base.update(cambi)
        return base

    def scrivi(self, documento: dict) -> None:
        self.finto.write_text(
            json.dumps(documento, ensure_ascii=False), encoding="utf-8", newline="\n"
        )

    def esegui(self) -> tuple[int, str]:
        uscita, errori = io.StringIO(), io.StringIO()
        with redirect_stdout(uscita), redirect_stderr(errori):
            codice = gate.main(["--verifica"])
        return codice, uscita.getvalue() + errori.getvalue()

    def test_un_verbale_coerente_e_verde(self) -> None:
        self.scrivi(self.verbale())
        codice, testo = self.esegui()
        self.assertEqual(codice, 0, testo)
        self.assertIn("riproducibilita' provata", testo)

    def test_un_verbale_assente_e_rosso(self) -> None:
        codice, testo = self.esegui()
        self.assertEqual(codice, 1)
        self.assertIn("prova di riproducibilita' assente", testo)
        self.assertIn("genera-fixture-filegdb.sh", testo)

    def test_un_verbale_di_un_altra_fixture_e_rosso(self) -> None:
        """Il caso che il legame esiste per chiudere: una fixture rigenerata e un
        verbale rimasto indietro direbbero «riproducibile» di byte che nessuno ha
        confrontato."""
        self.scrivi(self.verbale(impronta_della_fixture="0" * 64))
        codice, testo = self.esegui()
        self.assertEqual(codice, 1)
        self.assertIn("e' di un'altra fixture", testo)

    def test_un_verbale_con_altre_parti_e_rosso(self) -> None:
        self.scrivi(self.verbale(parti=3))
        codice, testo = self.esegui()
        self.assertEqual(codice, 1)
        self.assertIn("parti", testo)

    def test_un_verbale_senza_versione_di_gdal_e_rosso(self) -> None:
        """Due versioni scrivono tabelle di metadati diverse, e la differenza non
        sarebbe un byte coniato."""
        self.scrivi(self.verbale(versione_gdal=""))
        codice, testo = self.esegui()
        self.assertEqual(codice, 1)
        self.assertIn("quale", testo)

    def test_zero_byte_coniati_non_passa_in_silenzio(self) -> None:
        """Zero byte coniati vorrebbe dire che due rigenerazioni sono identiche:
        sarebbe una buona notizia, e renderebbe vuota la tolleranza del
        confronto. Va verificato invece che dato per scontato."""
        for valore in (0, -1, True, "nessuno", None):
            with self.subTest(valore=valore):
                self.scrivi(self.verbale(byte_coniati_totali=valore))
                codice, testo = self.esegui()
                self.assertEqual(codice, 1)
                self.assertIn("byte_coniati_totali", testo)

    def test_un_verbale_illeggibile_e_rosso(self) -> None:
        self.finto.write_text("non sono json", encoding="utf-8")
        codice, testo = self.esegui()
        self.assertEqual(codice, 1)
        self.assertIn("non e' JSON leggibile", testo)


class SondaDelVerbaleVero(unittest.TestCase):
    """Il verbale committato, letto come in CI."""

    def test_il_gate_e_verde_sull_albero_corrente(self) -> None:
        uscita, errori = io.StringIO(), io.StringIO()
        with redirect_stdout(uscita), redirect_stderr(errori):
            codice = gate.main(["--verifica"])
        self.assertEqual(codice, 0, errori.getvalue())

    def test_il_verbale_dice_come_e_stato_ottenuto(self) -> None:
        prova = json.loads(gate.PROVA.read_text(encoding="utf-8"))
        self.assertIn("come_e_stata_ottenuta", prova)
        self.assertIn("due rigenerazioni", prova["come_e_stata_ottenuta"])

    def test_i_byte_coniati_sono_quelli_dei_guid(self) -> None:
        """Quarantotto byte per tabella di metadati: tre identificatori da
        sedici. Se il numero cambiasse, cambierebbe cio' che GDAL conia, e la
        prosa che lo descrive andrebbe riletta."""
        prova = json.loads(gate.PROVA.read_text(encoding="utf-8"))
        for nome, quanti in prova["byte_coniati_per_parte"].items():
            with self.subTest(nome):
                self.assertEqual(quanti % 16, 0, "i GUID sono da sedici byte")


if __name__ == "__main__":
    unittest.main()
