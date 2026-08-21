"""Sonde delle prove eseguite di ASSURANCE-N1.

La verifica e' separata dall'esecuzione apposta: le sonde possono dare in
pasto un'uscita di harness **sintetica** e provare tutti i modi in cui una
prova puo' essere finta, senza lanciare `cargo` per ciascuno.

I quattro modi che la versione precedente del gate lasciava passare — helper
senza `#[test]`, test `#[ignore]`, `cfg` inattivo, identita' inesistente —
hanno qui una sonda ciascuno.
"""

from __future__ import annotations

import unittest

from scripts import check_assurance_n1_prove as gate

USCITA = """
running 3 tests
test tests::n1_un_ramo_negativo ... ok
test tests::n1_una_precedenza ... ok
test tests::n1_saltato ... ignored

test result: ok. 2 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out
"""


def gruppo(**prova_extra):
    prova = {
        "crate": "driver-x",
        "test": "tests::n1_un_ramo_negativo",
        "configurazione": "default",
        "esito": "coperto",
    }
    prova.update(prova_extra)
    return {
        "gruppo": "driver-x::f",
        "file": "crates/driver-x/src/lib.rs",
        "righe": 3,
        "raggiunto_da_replay": 0,
        "disposizione": "chiuso",
        "nota": "coperto",
        "prova": [prova],
    }


class SondeAnalisi(unittest.TestCase):
    def test_l_uscita_del_harness_si_legge(self) -> None:
        esiti, duplicati = gate.analizza_uscita(USCITA)
        self.assertEqual(duplicati, [])
        self.assertEqual(esiti["tests::n1_un_ramo_negativo"], "ok")
        self.assertEqual(esiti["tests::n1_saltato"], "ignored")

    def test_un_identita_duplicata_e_rossa(self) -> None:
        """Due test omonimi renderebbero ambiguo quale chiude il gruppo."""
        doppia = USCITA + "test tests::n1_un_ramo_negativo ... ok\n"
        _, duplicati = gate.analizza_uscita(doppia)
        self.assertTrue(
            any("duplicata" in d for d in duplicati), f"duplicato non colto: {duplicati}"
        )


class SondeVerifica(unittest.TestCase):
    def elenchi(self, uscita: str = USCITA):
        esiti, _ = gate.analizza_uscita(uscita)
        return {("driver-x", "default", "lib"): esiti}

    def test_una_prova_eseguita_e_passata_va_bene(self) -> None:
        """La controprova positiva: senza, «sempre rosso» sarebbe una difesa."""
        self.assertEqual(gate.verifica_prove([gruppo()], self.elenchi()), [])

    # --- i quattro modi in cui un simbolo non e' una prova ------------------

    def test_un_helper_senza_test_e_rosso(self) -> None:
        """Esiste come `fn`, ma il harness non lo esegue: non compare."""
        errori = gate.verifica_prove(
            [gruppo(test="tests::costruisci_fixture")], self.elenchi()
        )
        self.assertTrue(
            any("non compare fra i test eseguiti" in e for e in errori), errori
        )

    def test_un_test_ignorato_e_rosso(self) -> None:
        errori = gate.verifica_prove([gruppo(test="tests::n1_saltato")], self.elenchi())
        self.assertTrue(any("`#[ignore]`" in e for e in errori), errori)

    def test_un_cfg_inattivo_e_rosso(self) -> None:
        """Il test esiste nel sorgente ma non in questa configurazione.

        Si distingue dal caso «non esiste» soltanto per la configurazione, ed
        e' la ragione per cui la configurazione fa parte dell'identita'.
        """
        errori = gate.verifica_prove(
            [gruppo(configurazione="all-features")], self.elenchi()
        )
        self.assertTrue(any("nessuna misura" in e for e in errori), errori)

    def test_un_test_fallito_e_rosso(self) -> None:
        fallito = USCITA.replace("n1_un_ramo_negativo ... ok", "n1_un_ramo_negativo ... FAILED")
        errori = gate.verifica_prove([gruppo()], self.elenchi(fallito))
        self.assertTrue(any("non passa" in e for e in errori), errori)

    # --- coperto e irraggiungibile non sono la stessa cosa ------------------

    def test_irraggiungibile_senza_righe_e_guardia_e_rosso(self) -> None:
        errori = gate.verifica_prove(
            [gruppo(test="tests::n1_una_precedenza", esito="irraggiungibile")],
            self.elenchi(),
        )
        self.assertTrue(any("senza ['guardia', 'righe']" in e for e in errori), errori)

    def test_irraggiungibile_completa_va_bene(self) -> None:
        errori = gate.verifica_prove(
            [
                gruppo(
                    test="tests::n1_una_precedenza",
                    esito="irraggiungibile",
                    righe="392-394",
                    guardia="plenora-io-core::validate_write",
                )
            ],
            self.elenchi(),
        )
        self.assertEqual(errori, [], errori)

    def test_un_esito_inventato_e_rosso(self) -> None:
        errori = gate.verifica_prove([gruppo(esito="quasi")], self.elenchi())
        self.assertTrue(any("non ammesso" in e for e in errori), errori)

    def test_una_configurazione_inventata_e_rossa(self) -> None:
        errori = gate.verifica_prove([gruppo(configurazione="con-gdal")], self.elenchi())
        self.assertTrue(any("non ammessa" in e for e in errori), errori)

    def test_una_prova_senza_campi_e_rossa(self) -> None:
        voce = gruppo()
        del voce["prova"][0]["crate"]
        errori = gate.verifica_prove([voce], self.elenchi())
        self.assertTrue(any("campi mancanti" in e for e in errori), errori)

    # --- deduplicazione -----------------------------------------------------

    def test_le_coppie_da_misurare_non_si_ripetono(self) -> None:
        """Un test condiviso fra piu' gruppi si esegue una volta sola.

        Ripetere la misura non la rende piu' vera, e allunga il checkpoint.
        """
        due = [gruppo(), gruppo(test="tests::n1_una_precedenza")]
        self.assertEqual(gate.coppie_da_misurare(due), [("driver-x", "default", "lib")])

    def test_configurazioni_diverse_sono_misure_diverse(self) -> None:
        due = [gruppo(), gruppo(configurazione="all-features")]
        self.assertEqual(
            gate.coppie_da_misurare(due),
            [("driver-x", "default", "lib"), ("driver-x", "all-features", "lib")],
        )

    # --- il bersaglio del harness -------------------------------------------

    def test_bersagli_diversi_sono_misure_diverse(self) -> None:
        """`--lib` e `--bins` elencano test diversi dello stesso crate.

        Su un crate binario `--lib` non ne elenca nemmeno uno, e un elenco
        vuoto non e' un verde: il bersaglio fa parte dell'identita' della
        misura quanto la configurazione.
        """
        due = [gruppo(), gruppo(bersaglio="bins")]
        self.assertEqual(
            gate.coppie_da_misurare(due),
            [("driver-x", "default", "lib"), ("driver-x", "default", "bins")],
        )

    def test_un_bersaglio_inventato_e_rosso(self) -> None:
        errori = gate.verifica_prove([gruppo(bersaglio="esempi")], self.elenchi())
        self.assertTrue(any("bersaglio" in e for e in errori), errori)

    def test_il_comando_del_harness_e_uno_solo(self) -> None:
        """Lo condivide `check_release_contract.py`: due costruttori
        divergerebbero, e divergerebbero in silenzio."""
        self.assertEqual(
            gate.comando_test("plenora-io-cli", "all-features", "bins"),
            ["cargo", "test", "-p", "plenora-io-cli", "--all-features", "--bins"],
        )

    def test_i_gruppi_aperti_non_dichiarano_prove(self) -> None:
        aperto = gruppo()
        aperto["disposizione"] = "test_tabellare"
        self.assertEqual(gate.prove_dichiarate([aperto]), [])


if __name__ == "__main__":
    unittest.main()
