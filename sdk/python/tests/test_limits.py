"""I tetti tipizzati, e la loro corrispondenza con la CLI."""

from __future__ import annotations

import re
import unittest
from datetime import timedelta
from pathlib import Path

from plenora_io import Limits

RADICE = Path(__file__).resolve().parents[3]
CLI = RADICE / "crates" / "plenora-io-cli" / "src" / "main.rs"


class ITetti(unittest.TestCase):
    def test_nessun_tetto_non_produce_argomenti(self) -> None:
        """Il prodotto ha i propri valori predefiniti, e riscriverli qui
        vorrebbe dire mantenerli in due posti."""
        self.assertEqual(Limits().to_argv(), [])
        self.assertFalse(Limits())

    def test_un_tetto_solo(self) -> None:
        self.assertEqual(Limits(max_rows=10).to_argv(), ["--max-rows", "10"])
        self.assertTrue(Limits(max_rows=10))

    def test_la_durata_diventa_millisecondi(self) -> None:
        """`deadline` e' un `timedelta` perche' e' un tempo: un intero nudo
        lascerebbe indovinare l'unita', che nella CLI e' il millisecondo e nelle
        librerie Python quasi sempre il secondo."""
        self.assertEqual(
            Limits(deadline=timedelta(seconds=1.5)).to_argv(),
            ["--deadline-ms", "1500"],
        )
        self.assertEqual(
            Limits(deadline=timedelta(milliseconds=250)).to_argv(),
            ["--deadline-ms", "250"],
        )

    def test_l_ordine_e_stabile(self) -> None:
        """Due chiamate con gli stessi limiti producono la stessa riga, che e'
        cio' che rende confrontabili due esecuzioni."""
        limiti = Limits(memory_bytes=1024, max_rows=5, deadline=timedelta(seconds=1))
        self.assertEqual(limiti.to_argv(), Limits(
            deadline=timedelta(seconds=1), max_rows=5, memory_bytes=1024
        ).to_argv())
        self.assertEqual(
            limiti.to_argv(),
            ["--deadline-ms", "1000", "--max-rows", "5", "--memory-bytes", "1024"],
        )

    def test_zero_non_e_assente(self) -> None:
        """`None` vuol dire «non lo passo», `0` vuol dire «zero», e il prodotto
        li tratta diversamente: `--limit 0` produce una busta con zero righe."""
        self.assertEqual(Limits(max_rows=0).to_argv(), ["--max-rows", "0"])


class ITettiEsistonoNellaCli(unittest.TestCase):
    """Il confronto che impedisce all'SDK di offrire opzioni che non esistono.

    Un tetto che l'SDK offre e la CLI non conosce e' un comando che fallira'
    sull'uso; uno che la CLI accetta e l'SDK non offre e' un tetto raggiungibile
    solo scrivendo la riga a mano, cioe' facendo a mano il lavoro per cui l'SDK
    esiste.
    """

    def opzioni_della_cli(self) -> set[str]:
        sorgente = CLI.read_text(encoding="utf-8")
        riga = re.search(r'const OPZIONI_AMMESSE: &str = "(.*?)";', sorgente, re.S)
        self.assertIsNotNone(riga, "`OPZIONI_AMMESSE` non si trova")
        return {pezzo.strip() for pezzo in riga.group(1).split(",") if pezzo.strip()}

    @unittest.skipUnless(CLI.is_file(), "la CLI sta nel repository")
    def test_ogni_tetto_dell_sdk_e_un_opzione_della_cli(self) -> None:
        ammesse = self.opzioni_della_cli()
        for opzione in Limits.opzioni():
            with self.subTest(opzione=opzione):
                self.assertIn(opzione, ammesse)

    @unittest.skipUnless(CLI.is_file(), "la CLI sta nel repository")
    def test_ogni_tetto_della_cli_e_offerto_dall_sdk(self) -> None:
        """Il verso inverso, con le eccezioni **dichiarate**.

        Non tutte le opzioni della CLI sono tetti: alcune scelgono il layer o il
        formato, e hanno un parametro proprio nei metodi del client. Sono
        elencate qui perche' un elenco che si allunga da solo sarebbe il modo in
        cui «non e' un tetto» diventa «me ne sono dimenticato».
        """
        non_sono_tetti = {
            "--assume-crs",  # parametro di ogni metodo che legge
            "--layer",  # parametro di validate()
            "--limit",  # parametro di validate()
            "--in-opt",  # il dizionario `options`
            "--out-opt",  # servira' a convert()
            "--opt",  # idem
            "--durable",  # servira' a convert()
            "--version",  # e' un comando, non un'opzione
        }
        offerte = set(Limits.opzioni())
        mancanti = self.opzioni_della_cli() - offerte - non_sono_tetti
        self.assertEqual(mancanti, set())


if __name__ == "__main__":
    unittest.main()
