"""Sonde del gate che tiene onesta la capability `hostile_input_hardened`.

Il gate esiste per impedire una cosa sola: che un `true` costi un carattere. Se
il gate sbagliasse, la capability tornerebbe una parola -- e sarebbe una parola
che il catalogo pubblica, cioe' che qualcuno fuori di qui usa per decidere.

Le sonde muovono i modi in cui potrebbe diventare verde senza meritarlo: un
valore dichiarato senza il parser, un parser senza il valore, un driver che non
dichiara affatto -- e i tre residui che *somigliano* a un attraversamento senza
esserlo, il nome dentro un commento, il nome dentro una stringa, il simbolo
soltanto importato. L'ultima e' la piu' importante: cancellare la chiamata vera
e lasciare l'import e il commento che la nominano e' il modo in cui una
garanzia si perde senza che nessun diff sembri toglierla.
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

    def prove_per(self, driver: dict[str, tuple[str, bool]]) -> dict:
        """La mappa delle prove che rende coerente un albero finto.

        In un albero finto i test veri non esistono, quindi la mappa va
        iniettata: senza, ogni sonda sarebbe rossa per la prova mancante e
        nessuna proverebbe la regola che intende provare.
        """
        return {
            nome: ("tests::una_prova",)
            for nome, (_, valore) in driver.items()
            if valore
        }

    def errori(self, driver: dict[str, tuple[str, bool]]) -> list[str]:
        """Il gate su un albero finto, con le prove che quell'albero implica."""
        return gate.verifica(self.albero(driver), self.prove_per(driver))

    def test_un_driver_coerente_e_verde(self) -> None:
        self.assertEqual(
            self.errori(
                {
                    "driver-testo": ("fn leggi() { parse_wkt_bounded(t, l); }\n", True),
                    "driver-binario": ("fn leggi() { decode_wkb(b, l); }\n", False),
                }
            ),
            [],
        )

    def test_un_true_senza_il_parser_e_rosso(self) -> None:
        """Il modo piu' economico di ottenere una capability: scriverla."""
        errori = self.errori({"driver-bugiardo": ("fn leggi() { }\n", True)})
        self.assertTrue(any("costa un carattere" in e for e in errori), errori)

    # --- cio' che il solo testo non puo' vedere ----------------------------
    #
    # Il controllo sintattico legge sorgenti, non binari: una chiamata dentro
    # `#[cfg(any())]`, dietro una feature spenta o nel corpo di una macro mai
    # invocata gli assomiglia in tutto. E' la ragione per cui la capability
    # poggia sulla prova **eseguita**, e queste sonde tengono ferma quella
    # ragione invece di lasciarla scritta in un commento.

    def test_una_chiamata_esclusa_dalla_compilazione_non_basta(self) -> None:
        """`#[cfg(any())]` e' sempre falso: quel codice non esiste nel binario.

        Il controllo sul testo la vede come una chiamata, e infatti da solo la
        accetterebbe. A fermarla e' la prova eseguita, che manca -- e nessun
        test potrebbe passare attraverso codice che non viene compilato.
        """
        driver = {
            "driver-escluso": (
                "#[cfg(any())]\nfn leggi() { parse_wkt_bounded(t, l); }\n",
                True,
            )
        }
        radice = self.albero(driver)
        # Il controllo sintattico, da solo, non la distingue: e' il limite che
        # il gate dichiara di avere.
        self.assertTrue(gate.osservato(radice, "driver-escluso"))
        # Il gate, invece, e' rosso: senza prova eseguita il `true` non regge.
        errori = gate.verifica(radice, {})
        self.assertTrue(any("prova **eseguita**" in e for e in errori), errori)

    def test_una_chiamata_dietro_una_feature_spenta_non_basta(self) -> None:
        driver = {
            "driver-condizionale": (
                '#[cfg(feature = "mai-accesa")]\n'
                "fn leggi() { geometria_progressiva::analizza(t, l); }\n",
                True,
            )
        }
        errori = gate.verifica(self.albero(driver), {})
        self.assertTrue(any("prova **eseguita**" in e for e in errori), errori)

    def test_una_prova_per_un_driver_che_non_dichiara_e_rossa(self) -> None:
        """La direzione opposta: la mappa e i descrittori dicono la stessa cosa."""
        driver = {"driver-modesto": ("fn leggi() { }\n", False)}
        errori = gate.verifica(
            self.albero(driver), {"driver-modesto": ("tests::una_prova",)}
        )
        self.assertTrue(any("devono dire la stessa cosa" in e for e in errori), errori)

    def test_una_prova_che_nomina_un_driver_inesistente_e_rossa(self) -> None:
        driver = {"driver-testo": ("fn leggi() { parse_wkt_bounded(t, l); }\n", True)}
        errori = gate.verifica(
            self.albero(driver),
            {
                "driver-testo": ("tests::una_prova",),
                "driver-fantasma": ("tests::un_altra",),
            },
        )
        self.assertTrue(any("driver-fantasma" in e for e in errori), errori)

    def test_una_voce_vuota_non_e_una_prova(self) -> None:
        """La via per tenersi un `true` cancellando cio' che lo giustifica.

        `crate in prove` era soddisfatto da `()`: la voce c'era, il gate la
        contava, e `cargo test -- --exact` senza nomi non filtra -- esegue tutto
        ed esce 0.
        """
        driver = {"driver-testo": ("fn leggi() { parse_wkt_bounded(t, l); }\n", True)}
        errori = gate.verifica(self.albero(driver), {"driver-testo": ()})
        self.assertTrue(any("voce delle prove e' vuota" in e for e in errori), errori)
        # E il driver risulta anche senza prova: la voce vuota non lo copre.
        self.assertTrue(any("prova **eseguita**" in e for e in errori), errori)

    def test_un_identita_ripetuta_e_rossa(self) -> None:
        """Il harness elenca ogni test una volta, il conteggio ne direbbe due."""
        driver = {"driver-testo": ("fn leggi() { parse_wkt_bounded(t, l); }\n", True)}
        errori = gate.verifica(
            self.albero(driver),
            {"driver-testo": ("tests::una_prova", "tests::una_prova")},
        )
        self.assertTrue(any("piu' di una volta" in e for e in errori), errori)

    def test_un_identita_vuota_e_rossa(self) -> None:
        driver = {"driver-testo": ("fn leggi() { parse_wkt_bounded(t, l); }\n", True)}
        errori = gate.verifica(self.albero(driver), {"driver-testo": ("",)})
        self.assertTrue(any("non valida" in e for e in errori), errori)

    def test_la_mappa_reale_e_ben_formata(self) -> None:
        """La controprova positiva: senza, «sempre rosso» sarebbe una difesa."""
        self.assertEqual(gate.mappa_valida(gate.PROVE_DELLA_CAPABILITY), [])

    def test_le_prove_reali_non_sono_quelle_del_cap_in_byte(self) -> None:
        """La distinzione che questo lotto esiste per fare.

        Il cap in byte esisteva prima del parser progressivo e scatta prima di
        deserializzare: un test che lo esercita resta verde anche rimettendo il
        parser vecchio. Le prove della capability devono esercitare il tetto sui
        **componenti**, e i loro nomi lo dicono.
        """
        for crate, nomi in gate.PROVE_DELLA_CAPABILITY.items():
            for nome in nomi:
                with self.subTest(driver=crate):
                    self.assertIn("componenti", nome)
                    self.assertNotIn("cell_bytes", nome)

    def test_le_prove_reali_coprono_i_driver_che_dichiarano(self) -> None:
        """La controprova positiva sulla mappa vera, non su un albero finto."""
        dichiarano = {
            crate
            for crate in gate.crate_dei_driver(gate.ROOT)
            if gate.dichiarato(gate.ROOT, crate)
        }
        self.assertEqual(dichiarano, set(gate.PROVE_DELLA_CAPABILITY))
        self.assertTrue(dichiarano, "senza driver che dichiarano non prova nulla")

    def test_il_nome_in_un_commento_non_e_un_attraversamento(self) -> None:
        """La controprova che la prima stesura non superava: un `true` con il
        solo nome del parser dentro un commento.

        Un gate del genere spiega le proprie ragioni nominando i simboli, e
        cosi' fa ogni file di questo repository: se il nome contasse ovunque
        comparisse, documentare costerebbe una capability."""
        radice = self.albero(
            {
                "driver-commentato": (
                    "// un giorno chiameremo parse_wkt_bounded(testo, limiti)\n"
                    "fn leggi() { }\n",
                    True,
                )
            }
        )
        errori = gate.verifica(radice, {"driver-commentato": ("tests::una_prova",)})
        self.assertTrue(any("non **chiama**" in e for e in errori), errori)

    def test_un_simbolo_solo_importato_non_e_un_attraversamento(self) -> None:
        """`use` porta il nome in scope, non lo esegue."""
        radice = self.albero(
            {
                "driver-importatore": (
                    "use driver_common::wkt_lossless::parse_wkt_bounded;\n"
                    "fn leggi() { }\n",
                    True,
                )
            }
        )
        errori = gate.verifica(radice, {"driver-importatore": ("tests::una_prova",)})
        self.assertTrue(any("non **chiama**" in e for e in errori), errori)

    def test_cancellare_la_chiamata_lasciando_i_residui_e_rosso(self) -> None:
        """Il caso che tiene insieme i due precedenti: la chiamata sparisce e
        restano l'import e il commento che la nominano. E' il modo in cui una
        garanzia si perde senza che nessun diff sembri toglierla."""
        con_chiamata = (
            "use driver_common::wkt_lossless::parse_wkt_bounded;\n"
            "// la cella WKT passa da parse_wkt_bounded\n"
            "fn leggi() { parse_wkt_bounded(testo, limiti); }\n"
        )
        prove = {"driver-vero": ("tests::una_prova",)}
        albero = self.albero({"driver-vero": (con_chiamata, True)})
        self.assertEqual(gate.verifica(albero, prove), [])

        senza_chiamata = (
            "use driver_common::wkt_lossless::parse_wkt_bounded;\n"
            "// la cella WKT passa da parse_wkt_bounded\n"
            "fn leggi() { }\n"
        )
        errori = gate.verifica(
            self.albero({"driver-vero": (senza_chiamata, True)}), prove
        )
        self.assertTrue(any("non **chiama**" in e for e in errori), errori)

    def test_una_stringa_che_nomina_il_parser_non_conta(self) -> None:
        radice = self.albero(
            {
                "driver-loquace": (
                    'fn messaggio() -> &str { "usa parse_wkt_bounded(x, y)" }\n',
                    True,
                )
            }
        )
        errori = gate.verifica(radice, {"driver-loquace": ("tests::una_prova",)})
        self.assertTrue(any("non **chiama**" in e for e in errori), errori)

    def test_un_parser_senza_il_true_e_rosso(self) -> None:
        """La direzione opposta conta quanto la prima: una garanzia che non e'
        dichiarata e' una garanzia che nessun consumatore puo' usare."""
        errori = self.errori(
            {"driver-modesto": ("fn leggi() { parse_wkt_bounded(t, l); }\n", False)}
        )
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
        errori = gate.verifica(radice, {})
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
        self.assertEqual(
            self.errori(
                {
                    "driver-solo-in-prova": (
                        "mod tests {\n    fn sonda() { parse_wkt_bounded(t, l); }\n}\n",
                        False,
                    )
                }
            ),
            [],
        )

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
