"""Sonde del gate che tiene onesta la capability `hostile_input_hardened`.

Il gate esiste per impedire una cosa sola: che un `true` costi un carattere. Se
il gate sbagliasse, la capability tornerebbe una parola -- e sarebbe una parola
che il catalogo pubblica, cioe' che qualcuno fuori di qui usa per decidere.

Le sonde muovono i tre modi in cui potrebbe diventare verde senza meritarlo: un
valore dichiarato senza il parser, un parser senza il valore, e un driver che
non dichiara affatto.
"""

from __future__ import annotations

import pathlib
import tempfile
import unittest

from scripts import check_capability_input_ostile as gate

DESCRITTORE = """\
static DESCRIPTOR: FormatDescriptor = FormatDescriptor::const_new(
    "prova",
    Runtime::PureRust,
    // `hostile_input_hardened`: {ragione}
    {valore},
    None,
);
"""


class SondeDellaCapability(unittest.TestCase):
    def albero(self, driver: dict[str, tuple[str, bool]]) -> pathlib.Path:
        """Un finto workspace: per ogni driver, il valore e cosa attraversa."""
        temporanea = tempfile.TemporaryDirectory()
        self.addCleanup(temporanea.cleanup)
        radice = pathlib.Path(temporanea.name)
        for nome, (corpo, valore) in driver.items():
            sorgenti = radice / "crates" / nome / "src"
            sorgenti.mkdir(parents=True)
            (sorgenti / "lib.rs").write_text(
                DESCRITTORE.format(ragione="prova", valore=str(valore).lower()) + corpo,
                encoding="utf-8",
            )
        return radice

    def test_un_driver_coerente_e_verde(self) -> None:
        radice = self.albero(
            {
                "driver-testo": ("fn leggi() { parse_wkt_bounded(t, l); }\n", True),
                "driver-binario": ("fn leggi() { decode_wkb(b, l); }\n", False),
            }
        )
        self.assertEqual(gate.verifica(radice), [])

    def test_un_true_senza_il_parser_e_rosso(self) -> None:
        """Il modo piu' economico di ottenere una capability: scriverla."""
        radice = self.albero({"driver-bugiardo": ("fn leggi() { }\n", True)})
        errori = gate.verifica(radice)
        self.assertTrue(any("costa un carattere" in e for e in errori), errori)

    def test_un_parser_senza_il_true_e_rosso(self) -> None:
        """La direzione opposta conta quanto la prima: una garanzia che non e'
        dichiarata e' una garanzia che nessun consumatore puo' usare."""
        radice = self.albero(
            {"driver-modesto": ("fn leggi() { parse_wkt_bounded(t, l); }\n", False)}
        )
        errori = gate.verifica(radice)
        self.assertTrue(any("nessun consumatore" in e for e in errori), errori)

    def test_un_driver_che_non_dichiara_e_rosso(self) -> None:
        temporanea = tempfile.TemporaryDirectory()
        self.addCleanup(temporanea.cleanup)
        radice = pathlib.Path(temporanea.name)
        sorgenti = radice / "crates" / "driver-muto" / "src"
        sorgenti.mkdir(parents=True)
        sorgenti.joinpath("lib.rs").write_text(
            "static DESCRIPTOR: FormatDescriptor = FormatDescriptor::const_new(\n"
            '    "muto",\n    false,\n);\n',
            encoding="utf-8",
        )
        errori = gate.verifica(radice)
        self.assertTrue(any("non e' dichiarata" in e for e in errori), errori)

    def test_l_elenco_dei_driver_viene_dal_descrittore(self) -> None:
        """Non dal nome della crate: `driver-common` si chiama come loro e non
        dichiara niente al catalogo, e un driver nuovo entra da solo."""
        temporanea = tempfile.TemporaryDirectory()
        self.addCleanup(temporanea.cleanup)
        radice = pathlib.Path(temporanea.name)
        for nome, corpo in (
            ("driver-common", "pub fn condiviso() {}\n"),
            (
                "formato-nuovo",
                DESCRITTORE.format(ragione="prova", valore="false"),
            ),
        ):
            sorgenti = radice / "crates" / nome / "src"
            sorgenti.mkdir(parents=True)
            (sorgenti / "lib.rs").write_text(corpo, encoding="utf-8")
        self.assertEqual(gate.crate_dei_driver(radice), ["formato-nuovo"])

    def test_una_chiamata_nei_soli_test_non_conta(self) -> None:
        """I test chiamano gli entry point per provarli, ed e' il loro mestiere:
        contarli come uso di produzione direbbe che un driver e' irrigidito
        perche' una sonda lo esercita."""
        radice = self.albero(
            {
                "driver-solo-in-prova": (
                    "mod tests {\n    fn sonda() { parse_wkt_bounded(t, l); }\n}\n",
                    False,
                )
            }
        )
        self.assertEqual(gate.verifica(radice), [])

    def test_gli_ingressi_progressivi_sono_quelli_del_lotto(self) -> None:
        """L'elenco e' chiuso: aggiungerne uno senza averlo reso progressivo
        sarebbe il modo di far passare un driver che non lo e'."""
        self.assertEqual(
            set(gate.INGRESSI_PROGRESSIVI),
            {"parse_wkt_bounded", "geometria_progressiva::analizza"},
        )

    def test_l_albero_vero_e_coerente(self) -> None:
        """La controprova positiva: senza, «sempre rosso» sarebbe una difesa."""
        self.assertEqual(gate.verifica(gate.ROOT), [])


if __name__ == "__main__":
    unittest.main()
