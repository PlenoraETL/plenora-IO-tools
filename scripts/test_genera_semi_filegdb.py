"""Sonde della fixture FileGDB e dei semi che ne derivano.

La fixture e' il punto di partenza di **ogni** input del target: se perdesse una
parte, o se i semi smettessero di seguirla, il fuzzer sostituirebbe la tabella
sbagliata e la campagna misurerebbe qualcos'altro senza dirlo. Sono due modi di
diventare inutili in silenzio, ed e' quello che queste sonde tolgono.
"""

from __future__ import annotations

import io
import pathlib
import struct
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout

from scripts import genera_fixture_filegdb as fixture
from scripts import genera_semi_filegdb as semi


class SondeDellArchivio(unittest.TestCase):
    """L'archivio si rilegge per intero, o non si rilegge affatto."""

    def setUp(self) -> None:
        self.contenuti = fixture.spacchetta(fixture.ARCHIVIO.read_bytes())

    def test_la_fixture_e_un_filegdb(self) -> None:
        """Il file `gdb` e almeno una tabella: senza, e' una directory qualunque
        e il driver non avrebbe ragione di aprirla."""
        self.assertIn("gdb", self.contenuti)
        self.assertTrue(any(n.endswith(".gdbtable") for n in self.contenuti))
        self.assertGreaterEqual(len(self.contenuti), 2)

    def test_impacchetta_e_spacchetta_si_annullano(self) -> None:
        rifatto = fixture.impacchetta(self.contenuti)
        self.assertEqual(fixture.spacchetta(rifatto), self.contenuti)
        self.assertEqual(rifatto, fixture.ARCHIVIO.read_bytes())

    def test_un_archivio_troncato_e_rifiutato(self) -> None:
        grezzo = fixture.ARCHIVIO.read_bytes()
        for taglio in (0, 8, len(fixture.INTESTAZIONE), len(grezzo) // 2, len(grezzo) - 1):
            with self.subTest(taglio=taglio):
                with self.assertRaises((ValueError, struct.error, UnicodeDecodeError)):
                    fixture.spacchetta(grezzo[:taglio])

    def test_byte_di_coda_non_dichiarati_sono_rifiutati(self) -> None:
        with self.assertRaises(ValueError):
            fixture.spacchetta(fixture.ARCHIVIO.read_bytes() + b"\x00")

    def test_i_nomi_delle_parti_non_sono_percorsi(self) -> None:
        """Ogni nome finisce in un `join` dentro il target: un separatore o un
        `..` sarebbe un percorso costruito da byte."""
        for nome in self.contenuti:
            with self.subTest(nome):
                self.assertFalse(set(nome) - fixture.CARATTERI_AMMESSI)
                self.assertNotIn("/", nome)
                self.assertNotIn("\\", nome)
                self.assertNotEqual(nome.strip("."), "")


class SondeDeiByteConiati(unittest.TestCase):
    """La tolleranza sulla riproducibilita' e' **derivata**, non scritta.

    E' il cuore della prova: gli offset tollerati sono quelli in cui due
    rigenerazioni differiscono fra loro. Un byte stabile e diverso non e'
    coniato, ed e' rosso.
    """

    def test_gli_offset_coniati_vengono_dal_confronto(self) -> None:
        self.assertEqual(fixture.offset_coniati(b"abcd", b"abcd"), set())
        self.assertEqual(fixture.offset_coniati(b"abcd", b"aXcd"), {1})
        self.assertEqual(fixture.offset_coniati(b"abcd", b"aXYd"), {1, 2})

    def test_una_lunghezza_diversa_non_e_un_byte_coniato(self) -> None:
        """Due file di lunghezza diversa non sono lo stesso file con un GUID
        nuovo: nessun offset e' tollerato."""
        self.assertEqual(fixture.offset_coniati(b"abcd", b"abcde"), set())

    def test_una_differenza_stabile_e_rossa(self) -> None:
        committata = {"a00000001.gdbtable": b"AAAA", "gdb": b"G"}
        prima = {"a00000001.gdbtable": b"BAAA", "gdb": b"G"}
        seconda = {"a00000001.gdbtable": b"BAAA", "gdb": b"G"}
        errori = fixture.confronta(committata, prima, seconda)
        self.assertTrue(any("stabili" in m for m in errori), errori)

    def test_una_differenza_coniata_e_tollerata(self) -> None:
        committata = {"a00000001.gdbtable": b"AAAA", "gdb": b"G"}
        prima = {"a00000001.gdbtable": b"BAAA", "gdb": b"G"}
        seconda = {"a00000001.gdbtable": b"CAAA", "gdb": b"G"}
        self.assertEqual(fixture.confronta(committata, prima, seconda), [])

    def test_una_parte_in_piu_o_in_meno_e_rossa(self) -> None:
        prima = {"gdb": b"G", "a00000001.gdbtable": b"T"}
        self.assertTrue(fixture.confronta({"gdb": b"G"}, prima, prima))
        self.assertTrue(fixture.confronta({**prima, "extra": b"X"}, prima, prima))

    def test_una_lunghezza_diversa_dalla_fixture_e_rossa(self) -> None:
        prima = {"gdb": b"GG"}
        errori = fixture.confronta({"gdb": b"G"}, prima, prima)
        self.assertTrue(any("differenza di lunghezza" in m for m in errori), errori)


class SondeDeiSemi(unittest.TestCase):
    """I semi seguono la fixture, o sostituiscono la parte sbagliata."""

    def test_c_e_un_seme_per_parte_piu_quello_intatto(self) -> None:
        contenuti = fixture.spacchetta(fixture.ARCHIVIO.read_bytes())
        prodotti = semi.semi()
        self.assertEqual(len(prodotti), len(contenuti) + 1)
        self.assertEqual(prodotti["fixture-intatta.bin"], b"")

    def test_ogni_seme_porta_il_proprio_indice_e_il_contenuto_originale(self) -> None:
        """E' cio' che rende il seme una base valida: la `.gdb` che ne risulta e'
        identica alla fixture, e il fuzzer muta da li'."""
        contenuti = fixture.spacchetta(fixture.ARCHIVIO.read_bytes())
        ordinati = sorted(contenuti)
        for indice, nome in enumerate(ordinati):
            with self.subTest(nome):
                seme = semi.semi()[f"parte-{indice:02d}-{nome}.bin"]
                self.assertEqual(seme[0], indice)
                self.assertEqual(seme[1:], contenuti[nome])

    def test_i_semi_sul_disco_coincidono(self) -> None:
        for nome, atteso in semi.semi().items():
            percorso = semi.SEMI / nome
            with self.subTest(nome):
                self.assertTrue(percorso.exists(), f"{percorso} assente")
                self.assertEqual(percorso.read_bytes(), atteso)


class SondeDellaVerifica(unittest.TestCase):
    """`--verifica` diventa rossa in ogni modo in cui i semi possono divergere."""

    def setUp(self) -> None:
        temporanea = tempfile.TemporaryDirectory()
        self.addCleanup(temporanea.cleanup)
        self.radice = pathlib.Path(temporanea.name)
        precedente = semi.SEMI
        semi.SEMI = self.radice
        self.addCleanup(setattr, semi, "SEMI", precedente)

    def esegui(self, argomenti: list[str]) -> tuple[int, str]:
        uscita, errori = io.StringIO(), io.StringIO()
        with redirect_stdout(uscita), redirect_stderr(errori):
            codice = semi.main(argomenti)
        return codice, uscita.getvalue() + errori.getvalue()

    def test_scrivi_poi_verifica_e_verde(self) -> None:
        self.assertEqual(self.esegui(["--scrivi"])[0], 0)
        codice, testo = self.esegui(["--verifica"])
        self.assertEqual(codice, 0, testo)

    def test_verifica_e_il_modo_predefinito(self) -> None:
        """Un'invocazione senza argomenti non deve **riscrivere** i semi:
        sarebbe un gate che si rende verde da se'."""
        self.esegui(["--scrivi"])
        (self.radice / "fixture-intatta.bin").unlink()
        self.assertEqual(self.esegui([])[0], 1)
        self.assertFalse((self.radice / "fixture-intatta.bin").exists())

    def test_un_seme_modificato_a_mano_e_rosso(self) -> None:
        self.esegui(["--scrivi"])
        nome = sorted(n for n in semi.semi() if n.startswith("parte-"))[0]
        percorso = self.radice / nome
        grezzo = bytearray(percorso.read_bytes())
        grezzo[-1] ^= 0xFF
        percorso.write_bytes(bytes(grezzo))
        codice, testo = self.esegui(["--verifica"])
        self.assertEqual(codice, 1)
        self.assertIn("modificato a mano", testo)

    def test_un_seme_estraneo_e_rosso(self) -> None:
        self.esegui(["--scrivi"])
        (self.radice / "arrivato-da-chissa-dove.bin").write_bytes(b"\x00")
        codice, testo = self.esegui(["--verifica"])
        self.assertEqual(codice, 1)
        self.assertIn("sostituisce la parte sbagliata", testo)

    def test_una_cartella_vuota_e_rossa_su_ogni_seme(self) -> None:
        codice, testo = self.esegui(["--verifica"])
        self.assertEqual(codice, 1)
        for nome in semi.semi():
            self.assertIn(f"{nome}: seme assente", testo)


if __name__ == "__main__":
    unittest.main()
