"""Sonde negative del gate sulla quarantena vuota (FZ-0).

Un gate che non fallisce mai non e' un gate: qui la condizione verde e' un file
di soli commenti, che e' anche lo stato normale, quindi serve verificare che il
rosso arrivi davvero quando una riga compare.
"""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from scripts.check_quarantena_fuzz import righe_attive, verifica

INTESTAZIONE = """# Target con un finding aperto: la build strumentata resta obbligatoria,
# l'esecuzione nello smoke di CI no.
#
# Formato: <target> <spazi> <riferimento del finding>

"""


class SondeQuarantena(unittest.TestCase):
    def albero(self, contenuto: str | None) -> Path:
        radice = Path(tempfile.mkdtemp())
        self.addCleanup(self._rimuovi, radice)
        if contenuto is not None:
            percorso = radice / "fuzz" / "quarantine.txt"
            percorso.parent.mkdir(parents=True, exist_ok=True)
            percorso.write_text(contenuto, encoding="utf-8")
        return radice

    @staticmethod
    def _rimuovi(radice: Path) -> None:
        for percorso in sorted(radice.rglob("*"), reverse=True):
            if percorso.is_file():
                percorso.unlink()
            else:
                percorso.rmdir()
        radice.rmdir()

    def test_solo_intestazione_passa(self) -> None:
        """Lo stato normale: il file spiega il meccanismo e non quarantina nulla."""
        self.assertEqual(verifica(self.albero(INTESTAZIONE)), [])

    def test_file_assente_passa(self) -> None:
        self.assertEqual(verifica(self.albero(None)), [])

    def test_una_riga_attiva_fallisce(self) -> None:
        radice = self.albero(INTESTAZIONE + "ipc_reader  finding a monte non chiuso\n")
        errori = verifica(radice)
        self.assertTrue(errori)
        self.assertTrue(any("ipc_reader" in messaggio for messaggio in errori))

    def test_piu_righe_sono_tutte_elencate(self) -> None:
        radice = self.albero(
            INTESTAZIONE + "ipc_reader  uno\nxlsx_reader  due\ngpkg_reader  tre\n"
        )
        errori = verifica(radice)
        for atteso in ("ipc_reader", "xlsx_reader", "gpkg_reader"):
            self.assertTrue(
                any(atteso in messaggio for messaggio in errori),
                f"{atteso} non elencato",
            )

    def test_una_riga_indentata_non_si_nasconde(self) -> None:
        """Uno spazio davanti non la rende un commento."""
        radice = self.albero(INTESTAZIONE + "   ipc_reader  finding\n")
        self.assertTrue(verifica(radice))

    def test_un_commento_che_nomina_un_target_non_e_una_quarantena(self) -> None:
        radice = self.albero(INTESTAZIONE + "# ipc_reader era qui fino a FZ-0\n")
        self.assertEqual(verifica(radice), [])

    def test_righe_attive_ignora_vuote_e_commenti(self) -> None:
        self.assertEqual(righe_attive("\n\n# nota\n\n  # altra\n"), [])
        self.assertEqual(righe_attive("# nota\ntarget  motivo\n"), ["target  motivo"])


if __name__ == "__main__":
    unittest.main()
