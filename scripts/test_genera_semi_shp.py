"""Sonde del generatore dei semi di `shp_reader`.

Il generatore e' la sola cosa che tiene i semi leggibili: sono binari, e un
binario che nessuno sa riprodurre non si puo' rileggere. `--verifica` gira in
CI, e se sbagliasse direbbe «tutti riproducibili» su semi modificati a mano o
lasciati indietro da un cambio di formato — cioe' proprio nei due casi per cui
esiste.

Le sonde provano le due direzioni: che i semi prodotti abbiano la forma che il
target si aspetta, e che ogni modo di divergere diventi rosso.
"""

from __future__ import annotations

import io
import pathlib
import struct
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout

from scripts import genera_semi_shp as generatore


def diviso(bundle: bytes) -> tuple[bytes, bytes, bytes, bytes]:
    """La divisione che `driver_shp::__fuzz_dividi_bundle` fa in Rust.

    Riscritta qui apposta: leggere il bundle con la stessa funzione che lo ha
    scritto proverebbe soltanto che il generatore e' coerente con se stesso.
    """
    shp, shx, dbf = generatore.INTESTAZIONE.unpack(bundle[:6])
    resto = bundle[6:]
    return (
        resto[:shp],
        resto[shp : shp + shx],
        resto[shp + shx : shp + shx + dbf],
        resto[shp + shx + dbf :],
    )


class SondeDellaForma(unittest.TestCase):
    """I semi hanno la forma che il target attende, non una qualunque."""

    def setUp(self) -> None:
        self.prodotti = generatore.semi()

    def test_ogni_seme_si_divide_senza_resto_inatteso(self) -> None:
        for nome, contenuto in self.prodotti.items():
            with self.subTest(nome):
                shp, shx, dbf, prj = diviso(contenuto)
                self.assertEqual(
                    len(shp) + len(shx) + len(dbf) + len(prj) + 6, len(contenuto)
                )
                self.assertTrue(shp, "il `.shp` non puo' mancare")
                self.assertTrue(dbf, "il `.dbf` non puo' mancare")

    def test_l_intestazione_shp_dichiara_la_lunghezza_vera(self) -> None:
        """La lunghezza e' in parole da 16 bit: sbagliarla e' il difetto piu'
        comune di uno Shapefile scritto a mano, e renderebbe i semi rifiutati
        all'apertura senza che il replay se ne accorga."""
        for nome, contenuto in self.prodotti.items():
            with self.subTest(nome):
                shp, shx, _, _ = diviso(contenuto)
                for parte, etichetta in ((shp, "shp"), (shx, "shx")):
                    if not parte:
                        continue
                    (codice,) = struct.unpack(">i", parte[:4])
                    (parole,) = struct.unpack(">i", parte[24:28])
                    self.assertEqual(codice, generatore.CODICE_FILE, etichetta)
                    self.assertEqual(parole * 2, len(parte), etichetta)

    def test_l_indice_punta_ai_record_del_file_principale(self) -> None:
        """Un `.shx` che mente e' un difetto del seme, non del lettore: il
        driver conterebbe forme che non esistono e il rifiuto che ne segue
        verrebbe letto come una difesa del reader."""
        shp, shx, _, _ = diviso(self.prodotti["punti-con-attributi.bundle"])
        voci = [
            struct.unpack(">ii", shx[posizione : posizione + 8])
            for posizione in range(100, len(shx), 8)
        ]
        self.assertEqual(len(voci), 2, "due punti, due voci d'indice")
        for numero, (offset, lunghezza) in enumerate(voci, 1):
            inizio = offset * 2
            dichiarato, parole = struct.unpack(">ii", shp[inizio : inizio + 8])
            self.assertEqual(dichiarato, numero, "numero di record")
            self.assertEqual(parole, lunghezza, "lunghezza del record")

    def test_i_due_semi_disallineati_differiscono_solo_per_l_indice(self) -> None:
        """Sono due rami diversi dello stesso rifiuto — all'apertura e durante
        il drenaggio — e se differissero anche nel `.shp` o nel `.dbf` non
        proverebbero piu' che a cambiare e' solo la presenza dell'indice."""
        con = diviso(self.prodotti["disallineati-con-indice.bundle"])
        senza = diviso(self.prodotti["disallineati-senza-indice.bundle"])
        self.assertEqual((con[0], con[2], con[3]), (senza[0], senza[2], senza[3]))
        self.assertTrue(con[1])
        self.assertEqual(senza[1], b"")

    def test_il_dbf_disallineato_ha_meno_record_delle_forme(self) -> None:
        _, _, dbf, _ = diviso(self.prodotti["disallineati-con-indice.bundle"])
        (record,) = struct.unpack("<I", dbf[4:8])
        self.assertEqual(record, 1, "un solo record contro due forme")

    def test_solo_un_seme_porta_il_prj(self) -> None:
        """Presenza e assenza del `.prj` sono due percorsi del driver, e senza
        il seme senza `.prj` la sonda di isolamento del driver non avrebbe modo
        di distinguere una directory nuova da una riusata."""
        con_prj = {
            nome for nome, contenuto in self.prodotti.items() if diviso(contenuto)[3]
        }
        self.assertEqual(con_prj, {"punti-con-prj.bundle"})

    def test_un_bundle_che_non_entra_in_un_u16_e_rifiutato(self) -> None:
        """Le lunghezze dichiarate stanno in due byte: un seme piu' grande
        verrebbe troncato in silenzio, e il seme sul disco non sarebbe piu'
        quello che il generatore crede di aver scritto."""
        with self.assertRaises(ValueError):
            generatore.bundle(b"x" * 0x10000, b"", b"y")


class SondeDellaVerifica(unittest.TestCase):
    """`--verifica` diventa rossa in ogni modo in cui i semi possono divergere."""

    def setUp(self) -> None:
        temporanea = tempfile.TemporaryDirectory()
        self.addCleanup(temporanea.cleanup)
        self.radice = pathlib.Path(temporanea.name)

        precedente = generatore.SEMI
        generatore.SEMI = self.radice
        self.addCleanup(setattr, generatore, "SEMI", precedente)

    def esegui(self, argomenti: list[str]) -> tuple[int, str]:
        uscita, errori = io.StringIO(), io.StringIO()
        with redirect_stdout(uscita), redirect_stderr(errori):
            codice = generatore.main(argomenti)
        return codice, uscita.getvalue() + errori.getvalue()

    def test_scrivi_poi_verifica_e_verde(self) -> None:
        self.assertEqual(self.esegui(["--scrivi"])[0], 0)
        codice, testo = self.esegui(["--verifica"])
        self.assertEqual(codice, 0, testo)
        self.assertIn("tutti riproducibili", testo)

    def test_verifica_e_il_modo_predefinito(self) -> None:
        """La CI la invoca esplicitamente, ma un'invocazione senza argomenti non
        deve **riscrivere** i semi: sarebbe un gate che si rende verde da se'."""
        self.esegui(["--scrivi"])
        self.assertEqual(self.esegui([])[0], 0)
        (self.radice / "punti-con-prj.bundle").unlink()
        self.assertEqual(self.esegui([])[0], 1)
        self.assertFalse((self.radice / "punti-con-prj.bundle").exists())

    def test_un_seme_modificato_a_mano_e_rosso(self) -> None:
        self.esegui(["--scrivi"])
        percorso = self.radice / "punti-con-attributi.bundle"
        grezzo = bytearray(percorso.read_bytes())
        grezzo[-1] ^= 0xFF  # un byte solo, dentro il `.dbf`
        percorso.write_bytes(bytes(grezzo))

        codice, testo = self.esegui(["--verifica"])
        self.assertEqual(codice, 1)
        self.assertIn("punti-con-attributi.bundle", testo)
        self.assertIn("modificato a mano", testo)

    def test_un_seme_assente_e_rosso(self) -> None:
        self.esegui(["--scrivi"])
        (self.radice / "polilinea.bundle").unlink()
        codice, testo = self.esegui(["--verifica"])
        self.assertEqual(codice, 1)
        self.assertIn("polilinea.bundle: seme assente", testo)

    def test_un_seme_estraneo_e_rosso(self) -> None:
        """Un binario che il generatore non produce non e' riproducibile, e
        tollerarlo lascerebbe rientrare proprio i blob committati a mano."""
        self.esegui(["--scrivi"])
        (self.radice / "arrivato-da-chissa-dove.bundle").write_bytes(b"\x00")
        codice, testo = self.esegui(["--verifica"])
        self.assertEqual(codice, 1)
        self.assertIn("arrivato-da-chissa-dove.bundle", testo)

    def test_una_cartella_vuota_e_rossa_su_ogni_seme(self) -> None:
        codice, testo = self.esegui(["--verifica"])
        self.assertEqual(codice, 1)
        for nome in generatore.semi():
            self.assertIn(f"{nome}: seme assente", testo)


class SondaDeiSemiVersionati(unittest.TestCase):
    """I semi committati sono quelli che il generatore produce **oggi**.

    E' la stessa asserzione che gira in CI, e sta qui perche' `unittest` da'
    la diagnostica per seme mentre lo script da' solo un codice d'uscita.
    """

    def test_i_semi_sul_disco_coincidono(self) -> None:
        for nome, atteso in generatore.semi().items():
            percorso = generatore.SEMI / nome
            with self.subTest(nome):
                self.assertTrue(percorso.exists(), f"{percorso} assente")
                self.assertEqual(percorso.read_bytes(), atteso)


if __name__ == "__main__":
    unittest.main()
