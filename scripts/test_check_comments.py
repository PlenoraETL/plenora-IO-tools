"""Le controprove del gate dei commenti.

Un gate che passa su un albero pulito non ha ancora dimostrato niente: passa
anche un gate che non guarda. Qui si costruiscono i file che deve respingere e
quelli che deve lasciare stare, perche' la meta' difficile di questa regola non
e' vietare, e' **non** vietare la prosa che spiega il perche'.
"""

from __future__ import annotations

import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import check_comments as gate  # noqa: E402


class UnFileFinto(unittest.TestCase):
    """Scrive un file nel perimetro del gate e ne raccoglie le violazioni."""

    def violazioni(self, contenuto: str, nome: str = "finto.rs") -> list[str]:
        with tempfile.TemporaryDirectory(dir=gate.RADICE) as temporanea:
            percorso = pathlib.Path(temporanea) / nome
            percorso.write_bytes(contenuto.encode("utf-8"))
            return gate.violazioni([percorso])


class IlDebitoAnonimoNonEntra(UnFileFinto):
    def test_i_quattro_marcatori_sono_respinti(self) -> None:
        for marcatore in ("TODO", "FIXME", "HACK", "XXX"):
            with self.subTest(marcatore=marcatore):
                trovate = self.violazioni(f"// {marcatore}: da sistemare\n")
                self.assertEqual(len(trovate), 1, trovate)
                self.assertIn("debito anonimo", trovate[0])

    def test_una_parola_che_li_contiene_non_e_un_marcatore(self) -> None:
        """`METODO` contiene `TODO`, e non e' un promemoria.

        E' il difetto che un gate scritto con una sottostringa avrebbe: accusa
        una costante di essere un lavoro non fatto, e si impara a rinominare la
        costante invece di togliere il promemoria.
        """
        for parola in ("METODO_CHIUSO", "metodo", "PREFIXME_NO", "XXXL"):
            with self.subTest(parola=parola):
                self.assertEqual(self.violazioni(f"// {parola}\n"), [])


class LaCronacaIrrisolvibile(UnFileFinto):
    def test_il_lotto_numerato_e_respinto(self) -> None:
        for frase in (
            "// e' la regressione della tranche 2.",
            "// il difetto che questa tranche ha gia' trovato",
            "// La stessa tranche NON ha aggiunto niente",
            "// scritto dal writer pre-fix",
        ):
            with self.subTest(frase=frase):
                trovate = self.violazioni(frase + "\n")
                self.assertEqual(len(trovate), 1, trovate)
                self.assertIn("cronaca di processo", trovate[0])

    def test_la_prosa_che_spiega_il_perche_resta(self) -> None:
        """La meta' che conta: queste frasi **non** sono violazioni.

        Sono le forme piu' frequenti nell'albero, e ciascuna introduce una
        motivazione ancora vera. Un gate che le vietasse otterrebbe commenti
        piu' corti e meno utili, ed e' precisamente cio' che la decisione D6 ha
        scelto di non fare.
        """
        for frase in (
            "// la prima stesura di questo passo la trattava come un difetto",
            "// La versione precedente usciva subito su una pagina non a dizionario",
            "// Da allora la forma sciolta e' un opt-in esplicito",
            "// Qui c'era un `continue`, e sopra il perche'",
            "// prima era rifiutato con un messaggio che diceva il contrario",
            "// una tranche per commit: non si comincia la successiva prima",
            "// il campo `roadmap` del documento capability",
            "// nello stesso commit non si comincia la successiva",
        ):
            with self.subTest(frase=frase):
                self.assertEqual(self.violazioni(frase + "\n"), [], frase)


class IlPerimetro(unittest.TestCase):
    def test_il_piano_puo_nominare_i_marcatori_che_vieta(self) -> None:
        """Il documento che decide la regola sta fuori dal perimetro.

        Senza questa esclusione la tabella dei marcatori misurati -- che li
        cita uno per uno -- renderebbe rosso il gate che quella tabella
        istituisce.
        """
        self.assertIn("docs/PIANO-4.0.0.md", gate.ESCLUSI_ESATTI)
        percorsi = {p.relative_to(gate.RADICE).as_posix() for p in gate.file_commentabili()}
        self.assertNotIn("docs/PIANO-4.0.0.md", percorsi)

    def test_il_codice_di_terzi_non_e_nostro(self) -> None:
        self.assertTrue(any(p.startswith("vendor/") for p in gate.ESCLUSI_PREFISSO))
        percorsi = {p.relative_to(gate.RADICE).as_posix() for p in gate.file_commentabili()}
        self.assertFalse([p for p in percorsi if p.startswith("vendor/")])

    def test_il_perimetro_non_e_vuoto(self) -> None:
        """Senza, «nessuna violazione» sarebbe vero anche di zero file."""
        self.assertGreater(len(gate.file_commentabili()), 200)


class L_alberoRealeEPulito(unittest.TestCase):
    def test_nessuna_violazione_nell_albero(self) -> None:
        trovate = gate.violazioni(gate.file_commentabili())
        self.assertEqual(trovate, [], "\n".join(trovate[:20]))


if __name__ == "__main__":
    unittest.main()
