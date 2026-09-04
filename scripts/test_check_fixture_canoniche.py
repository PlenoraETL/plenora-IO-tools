"""Sonde del gate che risponde dei byte delle fixture canoniche.

Il gate afferma che le fixture su cui converte la matrice cross-format sono
quelle di cui la review ha risposto. Se sbagliasse, i test attraverserebbero un
ingresso diverso da quello letto in review e continuerebbero a passare: non e'
un falso rosso, e' un falso verde, e non lo vedrebbe nessuno.

Le sonde muovono i modi in cui potrebbe diventare verde senza meritarlo: la
directory vuota, che soddisfa ogni digest per assenza di confronti; il registro
vuoto, che e' la stessa cosa dall'altro lato; una fixture sparita e una
comparsa, che sono opposte e vanno viste entrambe; e i byte cambiati sotto un
nome che resta lo stesso.
"""

from __future__ import annotations

import importlib.util
import json
import pathlib
import shutil
import sys
import tempfile
import unittest

RADICE = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(RADICE / "scripts"))


def carica():
    percorso = RADICE / "scripts" / "check-fixture-canoniche.py"
    spec = importlib.util.spec_from_file_location("check_fixture_canoniche", percorso)
    modulo = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(modulo)
    return modulo


class SondeDelGate(unittest.TestCase):
    def setUp(self) -> None:
        self.gate = carica()
        self.lavoro = pathlib.Path(tempfile.mkdtemp(prefix="fixture-canoniche-"))
        self.addCleanup(shutil.rmtree, self.lavoro, ignore_errors=True)
        self.fixture = self.lavoro / "canoniche"
        self.fixture.mkdir()
        self.registro = self.lavoro / "registro.json"
        self.gate.FIXTURE = self.fixture
        self.gate.REGISTRO = self.registro

    def scrivi(self, percorso: str, contenuto: bytes) -> None:
        file = self.fixture / percorso
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_bytes(contenuto)

    def dichiara(self, voci: list[dict] | None = None) -> None:
        """Scrive il registro. Senza argomento dichiara cio' che c'e'."""
        if voci is None:
            voci = self.gate.manifesto()
        self.registro.write_text(
            json.dumps({"schema_version": 1, "fixture": voci}, ensure_ascii=False),
            encoding="utf-8",
        )

    def popola(self) -> None:
        self.scrivi("canonico.csv", b"id,geom\nr1,POINT (1 2)\n")
        self.scrivi("canonico.gdb/a00000001.gdbtable", b"\x00\x01\x02")
        self.scrivi("canonico.gdb/gdb", b"\x03\x04")

    # --- la controprova positiva ------------------------------------------

    def test_l_albero_reale_e_verde(self) -> None:
        """Senza questa, «sempre rosso» sarebbe una difesa.

        Gira sul registro e sulle fixture veri, non su quelli finti degli altri
        casi: un gate che fosse rosso anche sull'albero buono non direbbe
        niente, e sarebbe il primo a essere disattivato.
        """
        vero = carica()
        self.assertEqual(vero.verifica(), [])

    def test_un_albero_popolato_e_dichiarato_e_verde(self) -> None:
        self.popola()
        self.dichiara()
        self.assertEqual(self.gate.verifica(), [])

    # --- i modi di diventare verde senza merito ---------------------------

    def test_la_directory_vuota_e_rossa(self) -> None:
        """Il caso limite: nessun file, quindi nessun digest smentito."""
        self.dichiara(
            [{"percorso": "canonico.csv", "byte": 3, "sha256": "0" * 64}]
        )
        errori = self.gate.verifica()
        self.assertTrue(errori)
        self.assertIn("nessuna fixture sull'albero", " ".join(errori))

    def test_il_registro_vuoto_e_rosso(self) -> None:
        """La stessa assenza di confronti, ottenuta dall'altro lato."""
        self.popola()
        self.dichiara([])
        errori = self.gate.verifica()
        self.assertTrue(errori)
        self.assertIn("`fixture` assente o vuoto", " ".join(errori))

    def test_una_fixture_sparita_e_rossa(self) -> None:
        self.popola()
        self.dichiara()
        (self.fixture / "canonico.csv").unlink()
        errori = self.gate.verifica()
        self.assertTrue(any("assente dall'albero" in e for e in errori), errori)

    def test_una_fixture_comparsa_e_rossa(self) -> None:
        """L'opposto della precedente, e non e' la stessa prova.

        Un gate che verificasse soltanto i digest dichiarati sarebbe verde qui:
        i file elencati combaciano tutti. Cio' che non combacia e' l'insieme.
        """
        self.popola()
        self.dichiara()
        self.scrivi("intrusa.csv", b"id\n1\n")
        errori = self.gate.verifica()
        self.assertTrue(any("non dichiarato" in e for e in errori), errori)

    def test_un_membro_del_filegdb_cambiato_e_rosso(self) -> None:
        """La directory e' trentatre' file, e uno solo basta a cambiarla."""
        self.popola()
        self.dichiara()
        self.scrivi("canonico.gdb/gdb", b"\x03\x05")
        errori = self.gate.verifica()
        self.assertTrue(any("digest diverso" in e for e in errori), errori)

    def test_i_byte_cambiati_sono_rossi(self) -> None:
        self.popola()
        self.dichiara()
        self.scrivi("canonico.csv", b"id,geom\nr1,POINT (9 9)\n")
        errori = self.gate.verifica()
        self.assertTrue(errori)

    def test_un_digest_malformato_e_rosso(self) -> None:
        """Un registro che non si sa leggere non e' un registro soddisfatto."""
        self.popola()
        self.dichiara([{"percorso": "canonico.csv", "byte": 3, "sha256": "corto"}])
        errori = self.gate.verifica()
        self.assertTrue(any("digest a 64 cifre" in e for e in errori), errori)

    def test_lo_stesso_percorso_due_volte_e_rosso(self) -> None:
        self.popola()
        voce = {"percorso": "canonico.csv", "byte": 3, "sha256": "0" * 64}
        self.dichiara([voce, dict(voce)])
        errori = self.gate.verifica()
        self.assertTrue(any("due volte" in e for e in errori), errori)

    def test_il_registro_assente_e_rosso(self) -> None:
        self.popola()
        errori = self.gate.verifica()
        self.assertTrue(any("registro assente" in e for e in errori), errori)


if __name__ == "__main__":
    unittest.main()
