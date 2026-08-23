"""Sonde del confronto fra campagne di copertura.

Il confronto e' lo strumento con cui si cerca la causa di
`copertura.variazione-fra-corse`. Se sbagliasse, direbbe «le campagne
coincidono» su campagne diverse — e quella frase chiuderebbe un blocco.

Le due proprieta' opposte vanno provate entrambe: che due campagne identiche
diano zero differenze, e che ogni famiglia di differenza venga vista.
"""

from __future__ import annotations

import io
import pathlib
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout

from scripts import confronta_copertura as confronto


def lcov(*sezioni: tuple[str, list[tuple[int, int]]]) -> str:
    """Un lcov minimo: `SF:`, le righe `DA:` e `end_of_record`."""
    testo: list[str] = []
    for sorgente, righe in sezioni:
        testo.append(f"SF:{sorgente}")
        testo.extend(f"DA:{riga},{conteggio}" for riga, conteggio in righe)
        testo.append("end_of_record")
    return "\n".join(testo) + "\n"


class SondeLettura(unittest.TestCase):
    def setUp(self) -> None:
        temporanea = tempfile.TemporaryDirectory()
        self.addCleanup(temporanea.cleanup)
        self.radice = pathlib.Path(temporanea.name)
        self.quante = 0

    def scrivi(self, testo: str) -> pathlib.Path:
        self.quante += 1
        percorso = self.radice / f"campagna-{self.quante}.info"
        percorso.write_text(testo, encoding="utf-8", newline="\n")
        return percorso

    def test_legge_righe_e_conteggi(self) -> None:
        misure = confronto.leggi(
            self.scrivi(lcov(("a.rs", [(1, 3), (2, 0)]), ("b.rs", [(7, 1)])))
        )
        self.assertEqual(misure, {("a.rs", 1): 3, ("a.rs", 2): 0, ("b.rs", 7): 1})

    def test_una_riga_a_zero_e_strumentata(self) -> None:
        """«non coperta» e «non esistente» sono cose diverse, ed e' la
        distinzione su cui poggia tutto il confronto."""
        misure = confronto.leggi(self.scrivi(lcov(("a.rs", [(2, 0)]))))
        self.assertIn(("a.rs", 2), misure)
        self.assertEqual(confronto.sommario(misure), (0, 1))

    def test_un_lcov_vuoto_e_un_errore(self) -> None:
        with self.assertRaises(confronto.LcovMalformato):
            confronto.leggi(self.scrivi("TN:\n"))

    def test_una_riga_fuori_da_una_sezione_e_un_errore(self) -> None:
        with self.assertRaises(confronto.LcovMalformato):
            confronto.leggi(self.scrivi("DA:1,1\n"))

    def test_un_conteggio_non_numerico_e_un_errore(self) -> None:
        with self.assertRaises(confronto.LcovMalformato):
            confronto.leggi(self.scrivi("SF:a.rs\nDA:1,molte\nend_of_record\n"))

    def test_una_riga_ripetuta_con_conteggi_diversi_e_un_errore(self) -> None:
        """Contarne una a caso darebbe un numero che dipende dall'ordine."""
        with self.assertRaises(confronto.LcovMalformato):
            confronto.leggi(
                self.scrivi("SF:a.rs\nDA:1,1\nDA:1,4\nend_of_record\n")
            )

    def test_l_ordine_delle_sezioni_non_conta(self) -> None:
        prima = confronto.leggi(
            self.scrivi(lcov(("a.rs", [(1, 1)]), ("b.rs", [(2, 0)])))
        )
        seconda = confronto.leggi(
            self.scrivi(lcov(("b.rs", [(2, 0)]), ("a.rs", [(1, 1)])))
        )
        self.assertEqual(prima, seconda)


class SondeConfronto(unittest.TestCase):
    def test_due_campagne_identiche_non_divergono(self) -> None:
        """La controprova positiva: senza, «sempre diverse» sarebbe una difesa."""
        misure = {("a.rs", 1): 2, ("a.rs", 2): 0}
        differenze = confronto.confronta(misure, dict(misure))
        for famiglia in confronto.DIVERGENTI:
            self.assertEqual(differenze[famiglia], [], famiglia)

    def test_una_riga_coperta_solo_in_una_campagna(self) -> None:
        differenze = confronto.confronta({("a.rs", 1): 0}, {("a.rs", 1): 5})
        self.assertEqual(differenze["coperte_solo_nella_seconda"], [("a.rs", 1)])
        self.assertEqual(differenze["coperte_solo_nella_prima"], [])

    def test_un_denominatore_diverso_e_una_famiglia_a_parte(self) -> None:
        """Il denominatore che cambia e' piu' grave della copertura: lo
        strumento ha visto un altro insieme di righe."""
        differenze = confronto.confronta({("a.rs", 1): 1}, {("a.rs", 2): 1})
        self.assertEqual(differenze["strumentate_solo_nella_prima"], [("a.rs", 1)])
        self.assertEqual(differenze["strumentate_solo_nella_seconda"], [("a.rs", 2)])

    def test_un_conteggio_diverso_non_e_una_divergenza_di_copertura(self) -> None:
        """Una riga eseguita tre volte invece di due e' coperta in entrambi i
        casi: chiamarla divergenza di copertura sarebbe falso."""
        differenze = confronto.confronta({("a.rs", 1): 2}, {("a.rs", 1): 3})
        self.assertEqual(differenze["conteggio_diverso"], [("a.rs", 1)])
        for famiglia in confronto.DIVERGENTI:
            self.assertEqual(differenze[famiglia], [], famiglia)

    def test_le_righe_si_raggruppano_per_file(self) -> None:
        gruppi = confronto.per_file([("b.rs", 9), ("a.rs", 2), ("a.rs", 1)])
        self.assertEqual(gruppi, [("a.rs", [1, 2]), ("b.rs", [9])])


class SondeComando(unittest.TestCase):
    def setUp(self) -> None:
        temporanea = tempfile.TemporaryDirectory()
        self.addCleanup(temporanea.cleanup)
        self.radice = pathlib.Path(temporanea.name)

    def scrivi(self, nome: str, testo: str) -> pathlib.Path:
        percorso = self.radice / nome
        percorso.write_text(testo, encoding="utf-8", newline="\n")
        return percorso

    def esegui(self, *percorsi: pathlib.Path) -> tuple[int, str]:
        uscita = io.StringIO()
        with redirect_stdout(uscita):
            esito = confronto.main([str(p) for p in percorsi])
        return esito, uscita.getvalue()

    def test_campagne_identiche_escono_con_zero(self) -> None:
        testo = lcov(("a.rs", [(1, 1), (2, 0)]))
        esito, uscita = self.esegui(
            self.scrivi("prima.info", testo), self.scrivi("seconda.info", testo)
        )
        self.assertEqual(esito, 0)
        self.assertIn("coincidono riga per riga", uscita)

    def test_campagne_diverse_escono_con_uno(self) -> None:
        esito, uscita = self.esegui(
            self.scrivi("prima.info", lcov(("a.rs", [(1, 0)]))),
            self.scrivi("seconda.info", lcov(("a.rs", [(1, 7)]))),
        )
        self.assertEqual(esito, 1)
        self.assertIn("NON coincidono", uscita)
        self.assertIn("a.rs", uscita)

    def test_una_sola_campagna_non_e_un_confronto(self) -> None:
        sola = self.scrivi("sola.info", lcov(("a.rs", [(1, 1)])))
        with redirect_stderr(io.StringIO()):
            self.assertEqual(confronto.main([str(sola)]), 2)

    def test_ogni_campagna_si_confronta_con_la_prima(self) -> None:
        """Confrontare a coppie consecutive nasconderebbe una campagna che
        torna al punto di partenza."""
        esito, uscita = self.esegui(
            self.scrivi("prima.info", lcov(("a.rs", [(1, 1)]))),
            self.scrivi("seconda.info", lcov(("a.rs", [(1, 0)]))),
            self.scrivi("terza.info", lcov(("a.rs", [(1, 1)]))),
        )
        self.assertEqual(esito, 1)
        self.assertEqual(uscita.count("coperte_solo_nella_prima"), 1)


if __name__ == "__main__":
    unittest.main()
