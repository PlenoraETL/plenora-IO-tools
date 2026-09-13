"""Le controprove della misura di dimensione.

Il rischio di un contatore non e' che sbagli di poco: e' che conti la cosa
sbagliata e nessuno se ne accorga, perche' il numero e' plausibile. Qui si
costruiscono i file di cui si conosce la risposta.
"""

from __future__ import annotations

import json
import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import code_size as gate  # noqa: E402


class IlConteggioDiUnFile(unittest.TestCase):
    def conta(self, contenuto: str) -> tuple[int, int]:
        with tempfile.TemporaryDirectory() as temporanea:
            percorso = pathlib.Path(temporanea) / "finto.rs"
            percorso.write_bytes(contenuto.encode("utf-8"))
            return gate.righe_del_file(percorso)

    def test_un_file_senza_prove_e_tutto_prodotto(self) -> None:
        self.assertEqual(self.conta("fn a() {}\nfn b() {}\n"), (2, 0))

    def test_il_modulo_di_prove_non_e_prodotto(self) -> None:
        prodotto, prove = self.conta(
            "fn a() {}\n"
            "\n"
            "#[cfg(test)]\n"
            "mod sonde {\n"
            "    #[test]\n"
            "    fn c() {}\n"
            "}\n"
        )
        self.assertEqual((prodotto, prove), (2, 5))

    def test_cio_che_segue_il_modulo_torna_prodotto(self) -> None:
        """Un file con le prove **in mezzo** non perde la coda.

        E' il difetto che un contatore scritto con «dalla prima occorrenza in
        poi e' prova» avrebbe: plausibile, e sbagliato di tutto cio' che sta
        sotto.
        """
        prodotto, prove = self.conta(
            "#[cfg(test)]\n"
            "mod sonde {\n"
            "    fn c() {}\n"
            "}\n"
            "fn dopo() {}\n"
            "fn ancora() {}\n"
        )
        self.assertEqual((prodotto, prove), (2, 4))

    def test_le_graffe_annidate_non_chiudono_presto(self) -> None:
        prodotto, prove = self.conta(
            "#[cfg(test)]\n"
            "mod sonde {\n"
            "    fn c() {\n"
            "        if vero { fai(); }\n"
            "    }\n"
            "}\n"
            "fn dopo() {}\n"
        )
        self.assertEqual((prodotto, prove), (1, 6))

    def test_la_forma_condizionata_a_una_feature_e_riconosciuta(self) -> None:
        prodotto, prove = self.conta(
            "#[cfg(all(test, feature = \"x\"))]\n"
            "mod sonde {\n"
            "    fn c() {}\n"
            "}\n"
            "fn dopo() {}\n"
        )
        self.assertEqual((prodotto, prove), (1, 4))

    def test_un_cfg_che_non_e_test_resta_prodotto(self) -> None:
        """`#[cfg(windows)]` non apre delle prove.

        Senza questa distinzione il codice condizionato a una piattaforma
        sparirebbe dal denominatore, e il prodotto sembrerebbe piu' piccolo
        proprio dove porta piu' rischio.
        """
        prodotto, prove = self.conta(
            "#[cfg(windows)]\nmod finestre {\n    fn c() {}\n}\n"
        )
        self.assertEqual((prodotto, prove), (4, 0))


class IlPerimetro(unittest.TestCase):
    def test_gli_strumenti_di_misura_non_sono_prodotto(self) -> None:
        self.assertEqual(gate.FUORI_DAL_PRODOTTO, {"plenora-bench", "plenora-fuzz"})
        nomi = {p.as_posix() for p in gate.sorgenti()}
        self.assertFalse([n for n in nomi if "plenora-bench" in n or "plenora-fuzz" in n])

    def test_il_perimetro_non_e_vuoto(self) -> None:
        self.assertGreater(len(gate.sorgenti()), 40)


class IlBudget(unittest.TestCase):
    def test_il_tetto_porta_la_sua_ragione(self) -> None:
        """Un numero senza ragione si alza senza pensarci."""
        registro = json.loads(gate.BUDGET.read_text(encoding="utf-8"))
        self.assertGreater(registro["tetto"]["righe_di_prodotto"], 0)
        self.assertGreater(len(registro["tetto"]["perche_questo_numero"]), 120)

    def test_la_misura_registrata_e_quella_vera(self) -> None:
        registro = json.loads(gate.BUDGET.read_text(encoding="utf-8"))
        self.assertEqual(registro["misura_corrente"]["prodotto"], gate.misura()["prodotto"])

    def test_il_prodotto_sta_nel_tetto(self) -> None:
        registro = json.loads(gate.BUDGET.read_text(encoding="utf-8"))
        self.assertLessEqual(
            gate.misura()["prodotto"], registro["tetto"]["righe_di_prodotto"]
        )


if __name__ == "__main__":
    unittest.main()
