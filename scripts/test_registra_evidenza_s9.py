#!/usr/bin/env python3
"""Sonde sulla pubblicazione del verbale della campagna.

# Perche' questa parte ha bisogno di sonde

Il verbale non nasce piu' nell'albero: lo smoke lo scrive nella directory della
corsa, fuori, perche' un passo che scrive un file tracciato mentre il livello 2
verifica l'albero rende rosso `albero_invariato`. A portarlo dentro e' questo
registrar, **dopo** che la corsa e' finita.

Lo spostamento sposta anche il rischio. Prima il file c'era o non c'era, e il
gate della campagna lo leggeva dov'era sempre stato; ora c'e' un passaggio in
piu', e un passaggio che sbaglia in silenzio pubblicherebbe come qualifica di
questa corsa il verbale di un'altra. I tre rifiuti sono percio' provati uno per
uno: nessuno si deduce dagli altri.
"""

from __future__ import annotations

import importlib.util
import json
import pathlib
import tempfile
import unittest
from contextlib import redirect_stderr
import io

RADICE = pathlib.Path(__file__).resolve().parent.parent


def registrar():
    """Il modulo, caricato dal file: il nome porta trattini e non si importa."""
    percorso = RADICE / "scripts" / "registra-evidenza-s9.py"
    spec = importlib.util.spec_from_file_location("registra_evidenza_s9", percorso)
    modulo = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(modulo)
    return modulo


class SondeDellaPubblicazioneDelVerbale(unittest.TestCase):
    MISURATA = "a" * 40

    def setUp(self) -> None:
        self.gate = registrar()
        temporanea = tempfile.TemporaryDirectory()
        self.addCleanup(temporanea.cleanup)
        self.corsa = pathlib.Path(temporanea.name) / "corsa"
        self.corsa.mkdir()
        # La destinazione vive fuori dal repository: pubblicare davvero in
        # `assurance/evidence/` da una sonda lascerebbe l'albero sporco, e il
        # checkpoint lo vedrebbe mentre misura.
        self.destinazione = pathlib.Path(temporanea.name) / "evidenza" / "verbale.json"
        precedente = self.gate.VERBALE_PUBBLICATO
        self.gate.VERBALE_PUBBLICATO = self.destinazione
        self.addCleanup(
            setattr, self.gate, "VERBALE_PUBBLICATO", precedente
        )

    def _risultato(self, revisione: str | None = None) -> dict:
        return {"revisione_finale": revisione or self.MISURATA}

    def _scrivi(self, contenuto) -> None:
        percorso = self.corsa / self.gate.NOME_DEL_VERBALE
        if isinstance(contenuto, str):
            percorso.write_text(contenuto, encoding="utf-8")
        else:
            percorso.write_text(json.dumps(contenuto), encoding="utf-8")

    def _verbale(self, revisione: str) -> dict:
        return {
            "schema_version": 1,
            "revisione": revisione,
            "bersagli_dichiarati": ["shp_reader"],
            "secondi_per_bersaglio": 60,
            "hanno_finito": ["shp_reader"],
            "fermati_a_finding_noto": [],
            "falliti_su_finding_nuovo": [],
        }

    def _pubblica(self) -> list[str]:
        with redirect_stderr(io.StringIO()):
            return self.gate.pubblica_verbale(self.corsa, self._risultato())

    def test_il_verbale_della_corsa_viene_pubblicato_byte_per_byte(self) -> None:
        """I byte si copiano, non si rigenerano.

        Un verbale riscritto qui sarebbe un secondo documento che dice di essere
        il primo, e i digest di chi lo rilegge non tornerebbero.
        """
        self._scrivi(self._verbale(self.MISURATA))
        self.assertEqual(self._pubblica(), [])
        sorgente = (self.corsa / self.gate.NOME_DEL_VERBALE).read_bytes()
        self.assertEqual(self.destinazione.read_bytes(), sorgente)

    def test_un_verbale_assente_e_rosso(self) -> None:
        """Una campagna che non ha lasciato traccia non e' una campagna
        riuscita: e' l'assenza di una campagna."""
        motivi = self._pubblica()
        self.assertTrue(any("non c'e'" in m for m in motivi), motivi)
        self.assertFalse(self.destinazione.exists())

    def test_un_verbale_illeggibile_e_rosso(self) -> None:
        """C'e' un file e non dice niente: peggio che non averlo."""
        self._scrivi("{ questo non e' json")
        motivi = self._pubblica()
        self.assertTrue(any("non si legge" in m for m in motivi), motivi)
        self.assertFalse(self.destinazione.exists())

    def test_un_verbale_di_un_altra_revisione_e_rosso(self) -> None:
        """Il rifiuto che conta di piu'.

        Pubblicarlo attribuirebbe a questa corsa una campagna che non e' la sua,
        ed e' il ripiego piu' comodo: il file c'e', e' completo, e sembra una
        qualifica.
        """
        self._scrivi(self._verbale("b" * 40))
        motivi = self._pubblica()
        self.assertTrue(any("non e' la sua" in m for m in motivi), motivi)
        self.assertTrue(any("bbbbbbbbbbbb" in m for m in motivi), motivi)
        self.assertFalse(self.destinazione.exists())

    def test_un_verbale_senza_revisione_e_rosso(self) -> None:
        documento = self._verbale(self.MISURATA)
        del documento["revisione"]
        self._scrivi(documento)
        motivi = self._pubblica()
        self.assertTrue(any("non dichiara la revisione" in m for m in motivi), motivi)


if __name__ == "__main__":
    unittest.main()
