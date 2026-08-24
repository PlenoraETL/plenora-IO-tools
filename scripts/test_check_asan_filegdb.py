"""Sonde del gate che tiene misurato il confine di AddressSanitizer.

Il gate esiste per impedire una frase: «il percorso FileGDB e' coperto da
AddressSanitizer». E' vera a meta', e la meta' falsa e' quella che conta. Se il
gate sbagliasse, la frase tornerebbe vera per omissione -- e nessuna campagna
verde la smentirebbe, perche' una campagna verde e' esattamente cio' che quella
frase userebbe come prova.

Le sonde provano le due direzioni: che la misura vera sia verde, e che ogni modo
di raccontare un confine diverso da quello descritto sia rosso.
"""

from __future__ import annotations

import io
import json
import unittest
from contextlib import redirect_stderr, redirect_stdout

from scripts import check_asan_filegdb as gate

IMPRONTA = "f" * 64


def misura_minima() -> dict:
    """La misura che descrive il confine vero, da cui le sonde tolgono un pezzo."""
    return {
        "target": "filegdb_reader",
        "impronta_perimetro": IMPRONTA,
        "libreria_collegata": {
            "soname": "libgdal.so.32",
            "percorso_risolto": "/lib/x86_64-linux-gnu/libgdal.so.32",
        },
        "libreria_gdal_dentro_l_albero_di_build": False,
        "runtime_asan_collegato": True,
        "moduli_con_contatori": 1,
        "contatori_di_copertura": 167811,
        "file_sorgente_gdal_strumentati": 0,
        "che_cosa_significa": {chiave: "prosa" for chiave in gate.AFFERMAZIONI},
    }


class SondeDelConfine(unittest.TestCase):
    def setUp(self) -> None:
        # L'impronta vera legge il working tree e chiama git: qui interessa che
        # il **confronto** avvenga, non come si calcola il valore.
        precedente = gate.impronta_del_perimetro
        gate.impronta_del_perimetro = lambda percorsi: (IMPRONTA, [])
        self.addCleanup(setattr, gate, "impronta_del_perimetro", precedente)

    def test_la_misura_vera_del_confine_e_verde(self) -> None:
        self.assertEqual(gate.verifica(misura_minima()), [])

    def test_gdal_strumentata_non_passa_in_silenzio(self) -> None:
        """Se un giorno GDAL fosse costruita con la strumentazione, sarebbe una
        **buona** notizia -- e questo gate dovrebbe comunque diventare rosso,
        perche' la prosa che descrive il confine andrebbe riscritta."""
        misura = misura_minima()
        misura["file_sorgente_gdal_strumentati"] = 412
        errori = gate.verifica(misura)
        self.assertTrue(
            any("file_sorgente_gdal_strumentati" in m for m in errori), errori
        )
        self.assertTrue(any("e' la prosa a dover cambiare" in m for m in errori))

    def test_un_secondo_modulo_con_contatori_e_rosso(self) -> None:
        """Due moduli vorrebbero dire che una libreria condivisa porta
        contatori: il fuzzer non sarebbe piu' cieco dove la prosa dice che lo
        e'."""
        misura = misura_minima()
        misura["moduli_con_contatori"] = 2
        self.assertTrue(any("moduli_con_contatori" in m for m in gate.verifica(misura)))

    def test_il_runtime_asan_assente_e_rosso(self) -> None:
        """Il caso opposto, e il piu' pericoloso: un binario senza sanitizer che
        gira una campagna e non segnala niente."""
        misura = misura_minima()
        misura["runtime_asan_collegato"] = False
        self.assertTrue(any("runtime_asan_collegato" in m for m in gate.verifica(misura)))

    def test_un_binario_senza_contatori_e_rosso(self) -> None:
        for valore in (0, -1, "molti", True, None):
            with self.subTest(valore=valore):
                misura = misura_minima()
                misura["contatori_di_copertura"] = valore
                self.assertTrue(
                    any("contatori_di_copertura" in m for m in gate.verifica(misura))
                )

    def test_un_binario_che_non_collega_gdal_e_rosso(self) -> None:
        """Senza `libgdal` il target non sta esercitando FileGDB, e una campagna
        verde direbbe qualcosa di un percorso mai percorso."""
        misura = misura_minima()
        misura["libreria_collegata"]["soname"] = "libqualcosa.so.1"
        errori = gate.verifica(misura)
        self.assertTrue(any("non collega" in m for m in errori), errori)

    def test_una_libreria_senza_percorso_risolto_e_rossa(self) -> None:
        misura = misura_minima()
        misura["libreria_collegata"]["percorso_risolto"] = ""
        self.assertTrue(any("percorso_risolto" in m for m in gate.verifica(misura)))

    def test_una_gdal_costruita_nell_albero_e_rossa(self) -> None:
        misura = misura_minima()
        misura["libreria_gdal_dentro_l_albero_di_build"] = True
        self.assertTrue(
            any("libreria_gdal_dentro_l_albero_di_build" in m for m in gate.verifica(misura))
        )

    def test_i_numeri_senza_le_frasi_sono_rossi(self) -> None:
        """Un numero senza la frase che dice che cosa significa e' un numero che
        qualcuno rileggera' come gli fa comodo."""
        for affermazione in gate.AFFERMAZIONI:
            with self.subTest(affermazione):
                misura = misura_minima()
                del misura["che_cosa_significa"][affermazione]
                errori = gate.verifica(misura)
                self.assertTrue(
                    any(affermazione in m for m in errori),
                    f"togliere «{affermazione}» deve essere rosso: {errori}",
                )

    def test_una_misura_di_un_altro_target_e_rossa(self) -> None:
        misura = misura_minima()
        misura["target"] = "shp_reader"
        self.assertTrue(any("shp_reader" in m for m in gate.verifica(misura)))


class SondaDellaScadenza(unittest.TestCase):
    """La misura invecchia con il binario che descrive."""

    def test_una_misura_di_un_altro_albero_e_rossa(self) -> None:
        precedente = gate.impronta_del_perimetro
        gate.impronta_del_perimetro = lambda percorsi: ("0" * 64, [])
        self.addCleanup(setattr, gate, "impronta_del_perimetro", precedente)

        errori = gate.verifica(misura_minima())
        self.assertTrue(any("impronta del perimetro diversa" in m for m in errori), errori)
        self.assertTrue(
            any("asan-filegdb.sh" in m for m in errori),
            "il messaggio deve dire come rifarla",
        )


class SondaDellaMisuraVera(unittest.TestCase):
    """L'artefatto committato, letto come in CI."""

    def test_il_gate_e_verde_sull_albero_corrente(self) -> None:
        uscita, errori = io.StringIO(), io.StringIO()
        with redirect_stdout(uscita), redirect_stderr(errori):
            codice = gate.main([])
        self.assertEqual(codice, 0, errori.getvalue())

    def test_la_misura_porta_tutte_le_affermazioni(self) -> None:
        misura = json.loads(gate.ARTEFATTO.read_text(encoding="utf-8"))
        for affermazione in gate.AFFERMAZIONI:
            self.assertIn(affermazione, misura["che_cosa_significa"])
            self.assertGreater(len(misura["che_cosa_significa"][affermazione]), 40)

    def test_la_misura_dice_come_e_stata_presa(self) -> None:
        """Un numero di cui non si sa come e' stato ottenuto non si puo'
        ricontrollare, e ricontrollarlo e' l'unico modo di fidarsene."""
        misura = json.loads(gate.ARTEFATTO.read_text(encoding="utf-8"))
        self.assertIn("come_sono_stati_contati", misura)
        self.assertIn("come_e_stata_misurata", misura["libreria_collegata"])


if __name__ == "__main__":
    unittest.main()
