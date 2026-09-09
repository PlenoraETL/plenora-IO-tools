"""Prove del gate sui pin: quelli esatti, e quelli condivisi con fuzz/."""

from __future__ import annotations

import unittest

from scripts.check_dependency_pins import (
    pin_condivisi,
    pin_condivisi_del_repository,
    versione_dichiarata,
)


def prodotto(**dipendenze: object) -> dict:
    return {"workspace": {"dependencies": dict(dipendenze)}}


def fuzz(**dipendenze: object) -> dict:
    return {"dependencies": dict(dipendenze)}


class PinCondivisiTests(unittest.TestCase):
    """Il confronto fra `Cargo.toml` e `fuzz/Cargo.toml`.

    Le due direzioni sono entrambe necessarie e nessuna delle due basta da sola:
    un controllo che accetta tutto passa la prima, uno che rifiuta tutto passa
    la seconda.
    """

    def test_accetta_manifesti_coerenti(self) -> None:
        self.assertEqual(
            [],
            pin_condivisi(
                prodotto(**{"geo-types": "=0.7.20", "tempfile": "=3.27.0"}),
                fuzz(**{"geo-types": "=0.7.20", "tempfile": "=3.27.0"}),
            ),
        )

    def test_rifiuta_una_divergenza_nominando_dipendenza_e_valori(self) -> None:
        """Il caso reale del 2026-09-09, ridotto al minimo.

        Il messaggio deve bastare da solo: chi lo legge nel registro della CI
        non ha i due manifesti sott'occhio, e deve sapere *quale* dipendenza e
        *quali* due valori senza aprirli.
        """
        errori = pin_condivisi(
            prodotto(**{"geo-types": "=0.7.20"}),
            fuzz(**{"geo-types": "=0.7.19"}),
        )
        self.assertEqual(1, len(errori))
        self.assertIn("geo-types", errori[0])
        self.assertIn("=0.7.20", errori[0])
        self.assertIn("=0.7.19", errori[0])

    def test_una_divergenza_non_nasconde_l_altra(self) -> None:
        errori = pin_condivisi(
            prodotto(**{"geo-types": "=0.7.20", "tempfile": "=3.27.0"}),
            fuzz(**{"geo-types": "=0.7.19", "tempfile": "=3.20.0"}),
        )
        self.assertEqual(2, len(errori))

    def test_confronta_anche_le_dichiarazioni_in_tabella(self) -> None:
        """`geo-types = "=0.7.20"` e `{ version = "=0.7.20" }` dicono lo stesso.

        Una dichiarazione con feature o `default-features` diventa una tabella,
        e leggere solo la forma breve renderebbe il confronto cieco proprio
        sulle dipendenze piu' configurate.
        """
        self.assertEqual(
            [],
            pin_condivisi(
                prodotto(**{"geo-types": {"version": "=0.7.20", "features": ["serde"]}}),
                fuzz(**{"geo-types": "=0.7.20"}),
            ),
        )
        self.assertEqual(
            1,
            len(
                pin_condivisi(
                    prodotto(**{"geo-types": {"version": "=0.7.20"}}),
                    fuzz(**{"geo-types": {"version": "=0.7.19"}}),
                )
            ),
        )

    def test_una_dipendenza_solo_di_fuzz_non_e_una_divergenza(self) -> None:
        """`libfuzzer-sys` non esiste nel prodotto, ed e' giusto cosi'."""
        self.assertEqual(
            [],
            pin_condivisi(
                prodotto(**{"geo-types": "=0.7.20"}),
                fuzz(**{"geo-types": "=0.7.20", "libfuzzer-sys": "=0.4.13"}),
            ),
        )

    def test_confronta_anche_le_dev_dependencies_di_fuzz(self) -> None:
        self.assertEqual(
            1,
            len(
                pin_condivisi(
                    prodotto(**{"tempfile": "=3.27.0"}),
                    {"dev-dependencies": {"tempfile": "=3.20.0"}},
                )
            ),
        )

    def test_un_path_o_un_git_non_si_confrontano_per_versione(self) -> None:
        """Senza una versione da entrambi i lati non c'e' niente da confrontare.

        Tacere qui non e' una scappatoia: `validate_dependency` esamina gia'
        quelle dichiarazioni per conto suo, ed e' quello il posto giusto.
        """
        self.assertIsNone(versione_dichiarata({"path": "../vendor/dxf"}))
        self.assertEqual(
            [],
            pin_condivisi(
                prodotto(**{"dxf": {"path": "vendor/dxf"}}),
                fuzz(**{"dxf": {"path": "../vendor/dxf"}}),
            ),
        )

    def test_un_prodotto_senza_dipendenze_di_workspace_e_un_errore(self) -> None:
        """Se la sezione sparisse il confronto diventerebbe vuoto e verde.

        Un gate che passa perche' non ha trovato niente da guardare e' peggio
        di un gate assente, percio' l'assenza e' un fallimento.
        """
        self.assertEqual(1, len(pin_condivisi({}, fuzz(**{"geo-types": "=0.7.20"}))))


class RepositoryTests(unittest.TestCase):
    def test_i_manifesti_di_questo_repository_sono_coerenti(self) -> None:
        divergenze, condivisi = pin_condivisi_del_repository()
        self.assertEqual([], divergenze)
        self.assertGreater(
            condivisi,
            0,
            "nessun pin condiviso trovato: il confronto non sta guardando niente",
        )


if __name__ == "__main__":
    unittest.main()
