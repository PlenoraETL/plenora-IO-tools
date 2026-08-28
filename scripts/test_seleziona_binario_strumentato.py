#!/usr/bin/env python3
"""Sonde della selezione del binario strumentato.

Le quattro condizioni che `fuzz.profondita-riproducibile` pretende sono
affermazioni, e un'affermazione senza una sonda che la violi non e' verificata.
Qui ciascuna ha la propria, e la piu' importante e' la seconda: con l'arresto al
primo successo -- la forma da cui questo modulo viene -- passava, e passava
proprio nel caso in cui la misura sarebbe stata di un altro binario.
"""

from __future__ import annotations

import io
import pathlib
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout

from scripts import seleziona_binario_strumentato as selezione


def impronte(mappa: dict[str, str]):
    """Un'impronta finta: il contenuto dichiarato dalla sonda, e non un file."""

    def impronta(percorso: str) -> str:
        return mappa[percorso]

    return impronta


class LEnumerazione(unittest.TestCase):
    """L'ordine dei candidati e' dell'albero, non del filesystem."""

    def setUp(self) -> None:
        temporanea = tempfile.TemporaryDirectory()
        self.addCleanup(temporanea.cleanup)
        self.radice = pathlib.Path(temporanea.name)

    def crea(self, relativo: str, contenuto: bytes = b"x") -> pathlib.Path:
        percorso = self.radice / relativo
        percorso.parent.mkdir(parents=True, exist_ok=True)
        percorso.write_bytes(contenuto)
        return percorso

    def test_i_candidati_escono_ordinati(self) -> None:
        # Creati in ordine inverso apposta: se l'enumerazione ereditasse
        # l'ordine di creazione -- come fa `readdir` su piu' di un filesystem --
        # questa sonda lo vedrebbe.
        for relativo in ("zeta/shp_reader", "media/shp_reader", "alfa/shp_reader"):
            self.crea(relativo)
        trovati = selezione.candidati([self.radice], "shp_reader")
        self.assertEqual(trovati, sorted(trovati))
        self.assertEqual(len(trovati), 3)

    def test_l_enumerazione_e_ripetibile(self) -> None:
        for relativo in ("uno/shp_reader", "due/sotto/shp_reader"):
            self.crea(relativo)
        self.assertEqual(
            selezione.candidati([self.radice], "shp_reader"),
            selezione.candidati([self.radice], "shp_reader"),
        )

    def test_un_nome_diverso_non_e_un_candidato(self) -> None:
        """`shp_reader.d` e' il file delle dipendenze, non un binario."""
        self.crea("uno/shp_reader")
        self.crea("uno/shp_reader.d")
        self.assertEqual(len(selezione.candidati([self.radice], "shp_reader")), 1)

    def test_lo_stesso_file_da_due_radici_e_un_candidato_solo(self) -> None:
        """Radici che si contengono darebbero due candidati per un file solo, e
        il confronto di impronte fra un file e se stesso direbbe sempre di si'."""
        self.crea("dentro/shp_reader")
        trovati = selezione.candidati([self.radice, self.radice / "dentro"], "shp_reader")
        self.assertEqual(len(trovati), 1)

    def test_una_radice_inesistente_non_e_un_errore(self) -> None:
        """`fuzz/target` non c'e' finche' nessuno ha costruito i target."""
        self.crea("uno/shp_reader")
        self.assertEqual(
            len(selezione.candidati([self.radice, self.radice / "mai-creata"], "shp_reader")),
            1,
        )

    def test_l_impronta_e_del_contenuto(self) -> None:
        uno = self.crea("uno/shp_reader", b"identico")
        due = self.crea("due/shp_reader", b"identico")
        tre = self.crea("tre/shp_reader", b"diverso")
        self.assertEqual(
            selezione.impronta_del_file(uno), selezione.impronta_del_file(due)
        )
        self.assertNotEqual(
            selezione.impronta_del_file(uno), selezione.impronta_del_file(tre)
        )


class SiVerificanoTuttiICandidati(unittest.TestCase):
    """La condizione che l'arresto al primo successo rendeva invisibile."""

    def test_la_compatibilita_e_chiesta_a_ognuno(self) -> None:
        chiesti: list[str] = []

        def compatibile(candidato: str) -> bool:
            chiesti.append(candidato)
            return candidato == "a"

        scelto = selezione.scelta(["a", "b", "c"], compatibile, impronte({"a": "1"}))
        self.assertEqual(scelto, "a")
        self.assertEqual(
            chiesti,
            ["a", "b", "c"],
            "fermarsi al primo successo nasconde per costruzione l'esistenza di "
            "un secondo binario compatibile",
        )

    def test_un_secondo_compatibile_e_diverso_dopo_il_primo_e_rosso(self) -> None:
        """E' il caso in cui la forma vecchia sbagliava, e sbagliava in silenzio.

        Il primo candidato dell'ordine e' compatibile: `break` avrebbe scelto
        lui e non avrebbe mai guardato il secondo, che e' un binario diverso e
        accetta lo stesso profdata.
        """
        with self.assertRaises(selezione.SelezioneImpossibile) as raccolto:
            selezione.scelta(
                ["a", "b"], lambda _: True, impronte({"a": "1111", "b": "2222"})
            )
        self.assertIn("byte-identici", str(raccolto.exception))


class ICompatibiliDevonoEssereIdentici(unittest.TestCase):
    def test_due_compatibili_diversi_sono_rossi(self) -> None:
        with self.assertRaises(selezione.SelezioneImpossibile) as raccolto:
            selezione.scelta(
                ["primo", "secondo"],
                lambda _: True,
                impronte({"primo": "aaaa", "secondo": "bbbb"}),
            )
        messaggio = str(raccolto.exception)
        self.assertIn("primo", messaggio)
        self.assertIn("secondo", messaggio)

    def test_tre_compatibili_con_due_impronte_sono_rossi(self) -> None:
        """Due su tre identici non bastano: la terza copia e' un'altra misura."""
        with self.assertRaises(selezione.SelezioneImpossibile):
            selezione.scelta(
                ["a", "b", "c"],
                lambda _: True,
                impronte({"a": "1", "b": "1", "c": "2"}),
            )

    def test_un_incompatibile_diverso_non_conta(self) -> None:
        """Solo i **compatibili** devono essere identici fra loro.

        Sotto `target/` vivono build che col profdata non c'entrano -- quella
        con AddressSanitizer, per dirne una -- e pretendere che siano identiche
        a quella misurata renderebbe il gate rosso sempre.
        """
        scelto = selezione.scelta(
            ["asan", "buono"],
            lambda candidato: candidato == "buono",
            impronte({"asan": "aaaa", "buono": "bbbb"}),
        )
        self.assertEqual(scelto, "buono")


class LaSceltaFraCopieIdentiche(unittest.TestCase):
    def test_fra_copie_identiche_la_scelta_e_canonica(self) -> None:
        identiche = impronte({"alfa": "uguale", "beta": "uguale", "gamma": "uguale"})
        for elenco in (
            ["alfa", "beta", "gamma"],
            ["gamma", "beta", "alfa"],
            ["beta", "gamma", "alfa"],
        ):
            with self.subTest(elenco=elenco):
                self.assertEqual(
                    selezione.scelta(elenco, lambda _: True, identiche),
                    "alfa",
                    "la scelta non deve dipendere dall'ordine in cui l'elenco arriva",
                )

    def test_un_solo_compatibile_si_sceglie(self) -> None:
        self.assertEqual(
            selezione.scelta(
                ["a", "b"], lambda candidato: candidato == "b", impronte({"b": "1"})
            ),
            "b",
        )

    def test_nessun_candidato_e_rosso(self) -> None:
        with self.assertRaises(selezione.SelezioneImpossibile) as raccolto:
            selezione.scelta([], lambda _: True, impronte({}))
        self.assertIn("nessun candidato", str(raccolto.exception))

    def test_nessun_compatibile_e_rosso(self) -> None:
        with self.assertRaises(selezione.SelezioneImpossibile) as raccolto:
            selezione.scelta(["a", "b"], lambda _: False, impronte({}))
        self.assertIn("accetta il profdata", str(raccolto.exception))


class LaCompatibilitaVera(unittest.TestCase):
    """Il predicato che interroga `llvm-cov`, con un finto al posto suo."""

    def setUp(self) -> None:
        temporanea = tempfile.TemporaryDirectory()
        self.addCleanup(temporanea.cleanup)
        self.radice = pathlib.Path(temporanea.name)

    def finto(self, corpo: str) -> str:
        percorso = self.radice / "llvm-cov-finto"
        percorso.write_text(f"#!/bin/sh\n{corpo}\n", encoding="utf-8", newline="\n")
        percorso.chmod(0o755)
        return str(percorso)

    def test_un_export_riuscito_e_non_vuoto_e_compatibile(self) -> None:
        compatibile = selezione.compatibilita_con_llvm_cov(
            self.finto('echo "{}"'), "profdata", None
        )
        self.assertTrue(compatibile("qualunque"))

    def test_un_export_fallito_non_e_compatibile(self) -> None:
        compatibile = selezione.compatibilita_con_llvm_cov(
            self.finto("exit 1"), "profdata", None
        )
        self.assertFalse(compatibile("qualunque"))

    def test_un_export_riuscito_e_vuoto_non_e_compatibile(self) -> None:
        """Un export che esce con zero e non scrive niente non e' una misura:
        accettarlo qui sposterebbe il rosso dove il candidato scelto non e' piu'
        ricostruibile."""
        compatibile = selezione.compatibilita_con_llvm_cov(
            self.finto("exit 0"), "profdata", None
        )
        self.assertFalse(compatibile("qualunque"))

    def test_lo_stderr_di_ogni_tentativo_finisce_nel_log(self) -> None:
        log = self.radice / "selezione.log"
        compatibile = selezione.compatibilita_con_llvm_cov(
            self.finto('echo "non combacia" >&2; exit 1'), "profdata", log
        )
        compatibile("primo")
        compatibile("secondo")
        testo = log.read_text(encoding="utf-8")
        self.assertIn("primo", testo)
        self.assertIn("secondo", testo)
        self.assertIn("non combacia", testo)


class LaLineaDiComando(unittest.TestCase):
    def test_senza_radici_e_un_errore_d_uso(self) -> None:
        uscita, errori = io.StringIO(), io.StringIO()
        with redirect_stdout(uscita), redirect_stderr(errori):
            codice = selezione.main(
                ["shp_reader", "--llvm-cov", "/bin/false", "--instr-profile", "p"]
            )
        self.assertEqual(codice, 2)
        self.assertIn("radice", errori.getvalue())

    def test_nessun_candidato_esce_rosso_e_non_stampa_un_percorso(self) -> None:
        uscita, errori = io.StringIO(), io.StringIO()
        with redirect_stdout(uscita), redirect_stderr(errori):
            codice = selezione.main(
                [
                    "un_target_che_non_esiste",
                    "--radice",
                    "scripts",
                    "--llvm-cov",
                    "/bin/false",
                    "--instr-profile",
                    "p",
                ]
            )
        self.assertEqual(codice, 1)
        self.assertEqual(uscita.getvalue(), "")


if __name__ == "__main__":
    unittest.main()
