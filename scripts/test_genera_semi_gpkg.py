"""Sonde dei semi S11 del target `gpkg_geometry`.

Un seme e' utile quando **raggiunge** cio' che dovrebbe esercitare. Queste
sonde non possono eseguire il driver -- e' Rust -- ma possono verificare le due
cose che rendono il seme quello che dichiara di essere: che l'header sia quello
della specifica, e che il payload abbia la forma che il nome promette.

La terza cosa, che il seme arrivi davvero alla classificazione ricorsiva, la
verifica il replay del fuzzer: il target attraversa `wkb_shape` prima del
decoder lossless, ed e' la ragione per cui questi semi esistono.
"""

from __future__ import annotations

import io
import pathlib
import struct
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout

from scripts import genera_semi_gpkg as gate


class SondeDellaSpecifica(unittest.TestCase):
    def test_l_header_e_quello_di_geopackage(self) -> None:
        """Otto byte: magic, versione, flag, SRID little-endian."""
        header = gate.intestazione_gpkg()
        self.assertEqual(len(header), 8)
        self.assertEqual(header[:2], b"GP")
        self.assertEqual(header[2], 0)
        self.assertEqual(header[3], 0x01)
        self.assertEqual(struct.unpack("<i", header[4:])[0], 4326)

    def test_il_bit_vuota_e_il_bit_quattro(self) -> None:
        self.assertEqual(gate.intestazione_gpkg(vuota=True)[3], 0x11)

    def test_un_punto_vuoto_e_due_nan(self) -> None:
        payload = gate.punto_vuoto()
        self.assertEqual(payload[0], 0x01)
        self.assertEqual(struct.unpack("<I", payload[1:5])[0], 1)
        x, y = struct.unpack("<dd", payload[5:])
        self.assertNotEqual(x, x)
        self.assertNotEqual(y, y)

    def test_una_collezione_dichiara_i_figli_che_porta(self) -> None:
        figli = [gate.punto_vuoto(), gate.punto(1.0, 2.0)]
        payload = gate.collezione(7, figli)
        self.assertEqual(struct.unpack("<I", payload[5:9])[0], 2)
        self.assertEqual(payload[9:], b"".join(figli))

    def test_un_poligono_dichiara_gli_anelli_e_i_loro_punti(self) -> None:
        payload = gate.poligono([0, 4])
        self.assertEqual(struct.unpack("<I", payload[5:9])[0], 2)
        self.assertEqual(struct.unpack("<I", payload[9:13])[0], 0)
        self.assertEqual(struct.unpack("<I", payload[13:17])[0], 4)
        self.assertEqual(len(payload), 17 + 4 * 16)


class SondeDeiSemi(unittest.TestCase):
    """Ogni seme deve essere il caso che il suo nome promette."""

    def test_i_semi_vuoti_dichiarano_almeno_un_figlio(self) -> None:
        """E' la condizione che rende ognuno di essi il difetto di S11: una
        collezione che dichiara figli ed e' semanticamente vuota."""
        for nome in (
            "collection-di-un-punto-vuoto",
            "multipoint-di-punti-vuoti",
            "multipolygon-di-un-poligono-senza-anelli",
            "multipolygon-anello-senza-punti",
            "collection-annidata-vuota",
        ):
            with self.subTest(nome):
                payload = gate.semi()[nome]
                quanti = struct.unpack("<I", payload[5:9])[0]
                self.assertGreater(quanti, 0, "un conteggio a zero era gia' vuoto")

    def test_il_seme_annidato_scende_di_tre_livelli(self) -> None:
        payload = gate.semi()["collection-annidata-vuota"]
        self.assertEqual(payload.count(struct.pack("<I", 7)), 3)

    def test_il_seme_oltre_la_profondita_supera_il_tetto(self) -> None:
        """Il tetto del contratto del bordo e' 64: un seme che si fermasse a 64
        proverebbe il caso ammesso, non quello rifiutato."""
        payload = gate.semi()["collection-oltre-la-profondita"]
        self.assertGreater(payload.count(struct.pack("<I", 7)), 64)

    def test_i_semi_ostili_sono_davvero_malformati(self) -> None:
        troncato = gate.semi()["collection-figlio-troncato"]
        intero = gate.collezione(7, [gate.punto(1.0, 2.0)])
        self.assertLess(len(troncato), len(intero))

        mancante = gate.semi()["collection-figlio-mancante"]
        dichiarati = struct.unpack("<I", mancante[5:9])[0]
        self.assertEqual(dichiarati, 2)
        self.assertEqual(len(mancante[9:]), len(gate.punto(1.0, 2.0)))

    def test_ogni_seme_porta_l_header(self) -> None:
        for nome, byte in gate.contenuti().items():
            with self.subTest(nome):
                self.assertTrue(nome.startswith(gate.PREFISSO))
                self.assertTrue(nome.endswith(".gpkgb"))
                self.assertEqual(byte[:2], b"GP")


class SondeDellaVerifica(unittest.TestCase):
    """`--verifica` deve accorgersi di cio' che non ha prodotto."""

    def setUp(self) -> None:
        temporanea = tempfile.TemporaryDirectory()
        self.addCleanup(temporanea.cleanup)
        self.radice = pathlib.Path(temporanea.name)
        precedente = gate.SEMI
        gate.SEMI = self.radice
        self.addCleanup(setattr, gate, "SEMI", precedente)

    def esegui(self, verifica: bool) -> tuple[int, str]:
        uscita, errori = io.StringIO(), io.StringIO()
        with redirect_stdout(uscita), redirect_stderr(errori):
            codice = gate.main(["--verifica"] if verifica else [])
        return codice, uscita.getvalue() + errori.getvalue()

    def test_i_semi_appena_scritti_si_verificano(self) -> None:
        self.assertEqual(self.esegui(False)[0], 0)
        codice, testo = self.esegui(True)
        self.assertEqual(codice, 0, testo)
        self.assertIn("byte a byte", testo)

    def test_un_seme_assente_e_rosso(self) -> None:
        codice, testo = self.esegui(True)
        self.assertEqual(codice, 1)
        self.assertIn("seme assente", testo)

    def test_un_seme_modificato_e_rosso(self) -> None:
        self.esegui(False)
        nome = sorted(gate.contenuti())[0]
        percorso = self.radice / nome
        percorso.write_bytes(percorso.read_bytes() + b"\x00")
        codice, testo = self.esegui(True)
        self.assertEqual(codice, 1)
        self.assertIn("dalla specifica", testo)

    def test_un_seme_orfano_col_prefisso_e_rosso(self) -> None:
        """Un blob che nessuno dichiara si legge come copertura e non lo e'."""
        self.esegui(False)
        (self.radice / f"{gate.PREFISSO}inventato.gpkgb").write_bytes(b"GP\x00\x01")
        codice, testo = self.esegui(True)
        self.assertEqual(codice, 1)
        self.assertIn("che questo file non dichiara", testo)

    def test_i_semi_storici_restano_fuori_dal_perimetro(self) -> None:
        """Cinque blob precedenti a questo file: dichiararli senza averli
        derivati vorrebbe dire firmare byte che non ho prodotto."""
        self.esegui(False)
        (self.radice / "point-xy-le.gpkgb").write_bytes(b"GP\x00\x01")
        self.assertEqual(self.esegui(True)[0], 0)


if __name__ == "__main__":
    unittest.main()
