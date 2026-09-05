"""L'esecutore, e i due flussi che il protocollo tiene distinti.

Il binario e' finto per tutte le sonde qui: devono descrivere risposte che il
prodotto vero non produce a comando -- un errore scritto su stdout, un successo
senza busta, un output parziale prima di un errore terminale. Sono le
condizioni in cui l'esecutore deve fallire chiuso, e costruirle col binario vero
vorrebbe dire guastarlo.

Il prodotto vero lo esercitano le sonde di `test_client.py`.
"""

from __future__ import annotations

import os
import sys
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

from plenora_io import CommandFailed, NotFoundError, ProtocolError
from plenora_io.discovery import NOME
from plenora_io.process import Runner

BUSTA_ERRORE = (
    '{"status": "error", "protocol_version": 1, '
    '"contract": "plenora-io-error-v1", "error": {"code": "FORMAT_ERROR", '
    '"category": "not_found", "phase": "read", "remote_effect": "none", '
    '"retry": {"kind": "never"}, "message": "niente da leggere"}}'
)


@unittest.skipIf(sys.platform == "win32", "il finto e' uno script POSIX")
class LEsecutore(unittest.TestCase):
    def setUp(self) -> None:
        self._temporanea = TemporaryDirectory(prefix="plenora-sdk-process-")
        self.tmp = Path(self._temporanea.name)

    def tearDown(self) -> None:
        self._temporanea.cleanup()

    def runner(self, corpo: str, **kwargs) -> Runner:
        percorso = self.tmp / NOME
        percorso.write_text(
            "#!/usr/bin/env python3\nimport sys, json\n" + corpo, encoding="utf-8"
        )
        percorso.chmod(0o755)
        return Runner(percorso, **kwargs)

    # --- la strada che funziona -------------------------------------------

    def test_il_successo_si_legge_da_stdout(self) -> None:
        runner = self.runner('print(json.dumps({"status": "ok", "a": 1}))\n')
        self.assertEqual(runner.run(["catalog"]), {"status": "ok", "a": 1})

    def test_qualunque_cosa_su_stderr_con_successo_e_una_violazione(self) -> None:
        """Il v2 tace su stderr quando riesce, e l'SDK parla v2.

        La prima stesura tollerava l'avviso del protocollo legacy. Era una
        tolleranza implicita -- l'SDK non espone quel flag, e nessuno l'aveva
        chiesta -- e rendeva invisibile la sola forma in cui il v2 puo' sporcare
        quel flusso: un avviso non previsto, una traccia di debug, la riga di
        una libreria che scrive dove non deve.
        """
        for rumore in (
            "attenzione: protocollo legacy",
            "DEBUG: apro il file",
            " ",
        ):
            with self.subTest(rumore=rumore):
                runner = self.runner(
                    f"print({rumore!r}, file=sys.stderr)\n"
                    'print(json.dumps({"status": "ok"}))\n'
                )
                if not rumore.strip():
                    # Uno spazio solo non e' contenuto: il flusso e' vuoto per
                    # come lo si giudica, e il confine va detto invece che
                    # scoperto da chi ci inciampa.
                    self.assertEqual(runner.run(["convert"]), {"status": "ok"})
                    continue
                with self.assertRaises(ProtocolError) as preso:
                    runner.run(["convert"])
                self.assertIn("non mette niente", str(preso.exception))
                self.assertIn(rumore, str(preso.exception))

    def test_il_successo_pulito_resta_un_successo(self) -> None:
        """La controprova: senza, «stderr sporco e' un errore» sarebbe vero
        anche di un esecutore che rifiuta ogni successo."""
        runner = self.runner('print(json.dumps({"status": "ok", "a": 1}))\n')
        self.assertEqual(runner.run(["catalog"]), {"status": "ok", "a": 1})

    def test_stderr_sporco_non_conta_quando_il_comando_fallisce(self) -> None:
        """La regola vale sul **successo**: in caso d'errore stderr porta la
        busta, ed e' li' che si legge."""
        runner = self.runner(
            f"print({BUSTA_ERRORE!r}, file=sys.stderr)\nsys.exit(5)\n"
        )
        with self.assertRaises(NotFoundError):
            runner.run(["read", "x"])

    # --- l'errore, e la sua busta -----------------------------------------

    def test_l_errore_si_legge_da_stderr(self) -> None:
        runner = self.runner(
            f"print({BUSTA_ERRORE!r}, file=sys.stderr)\nsys.exit(5)\n"
        )
        with self.assertRaises(NotFoundError) as preso:
            runner.run(["read", "x"])
        self.assertEqual(preso.exception.exit_code, 5)
        self.assertEqual(preso.exception.envelope.phase, "read")
        self.assertEqual(preso.exception.argv, ["read", "x"])

    # --- le combinazioni che il protocollo non prevede ---------------------

    def test_un_errore_scritto_su_stdout_non_passa_per_successo(self) -> None:
        """Il difetto che la scelta del flusso per codice d'uscita chiude.

        Cercando un JSON prima su stdout e poi su stderr, questa busta sarebbe
        stata letta dal flusso sbagliato. `status` lo si guarda **dopo** aver
        scelto il flusso, quindi non avrebbe salvato niente: l'uscita e' zero, e
        una busta d'errore consegnata come successo e' cio' che ne sarebbe
        uscito.
        """
        runner = self.runner(f"print({BUSTA_ERRORE!r})\n")
        with self.assertRaises(ProtocolError) as preso:
            runner.run(["read", "x"])
        self.assertIn("stato «error»", str(preso.exception))

    def test_un_successo_su_stderr_con_uscita_diversa_da_zero(self) -> None:
        runner = self.runner(
            'print(json.dumps({"status": "ok"}), file=sys.stderr)\nsys.exit(1)\n'
        )
        with self.assertRaises(ProtocolError) as preso:
            runner.run(["catalog"])
        self.assertIn("porta una busta d'errore", str(preso.exception))

    def test_un_output_parziale_prima_di_un_errore_e_rifiutato(self) -> None:
        """Un output parziale consegnato prima di un errore terminale e' cio'
        che i target di fuzzing cercano, e un SDK che lo ignorasse lo
        lascerebbe consumare."""
        runner = self.runner(
            'print("riga parziale")\n'
            f"print({BUSTA_ERRORE!r}, file=sys.stderr)\nsys.exit(1)\n"
        )
        with self.assertRaises(ProtocolError) as preso:
            runner.run(["convert"])
        self.assertIn("output parziale", str(preso.exception))

    def test_il_silenzio_sul_flusso_atteso_e_nominato(self) -> None:
        for corpo, atteso in (
            ("pass\n", "stdout"),
            ('print("x")\nsys.exit(1)\n', "stderr"),
        ):
            with self.subTest(flusso=atteso):
                with self.assertRaises(ProtocolError) as preso:
                    self.runner(corpo).run(["catalog"])
                self.assertIn(f"su {atteso}", str(preso.exception))

    def test_un_json_malformato_dice_che_cosa_ha_letto(self) -> None:
        runner = self.runner('print("{non json")\n')
        with self.assertRaises(ProtocolError) as preso:
            runner.run(["catalog"])
        self.assertIn("non e' JSON", str(preso.exception))
        self.assertIn("{non json", str(preso.exception))

    def test_una_busta_che_non_e_un_oggetto(self) -> None:
        runner = self.runner("print(json.dumps([1, 2]))\n")
        with self.assertRaises(ProtocolError) as preso:
            runner.run(["catalog"])
        self.assertIn("non un oggetto", str(preso.exception))

    def test_il_timeout_dice_che_non_si_sa(self) -> None:
        """Un timeout non dice che il comando sia fallito: dice che non si sa.

        La distinzione conta per chi deve decidere se ripulire una destinazione
        parziale: un errore la dichiara, un timeout la lascia da accertare.
        """
        runner = self.runner("import time\ntime.sleep(5)\n", timeout=0.3)
        with self.assertRaises(ProtocolError) as preso:
            runner.run(["convert"])
        self.assertIn("non si sa", str(preso.exception))

    def test_un_binario_che_non_si_esegue(self) -> None:
        runner = Runner(self.tmp / "non-esiste")
        with self.assertRaises(ProtocolError) as preso:
            runner.run(["catalog"])
        self.assertIn("non si e' potuto eseguire", str(preso.exception))


@unittest.skipIf(sys.platform == "win32", "il finto e' uno script POSIX")
class LaGerarchiaDegliErrori(unittest.TestCase):
    """La classe si sceglie dalla **categoria**, mai dal testo."""

    def setUp(self) -> None:
        self._temporanea = TemporaryDirectory(prefix="plenora-sdk-errori-")
        self.tmp = Path(self._temporanea.name)

    def tearDown(self) -> None:
        self._temporanea.cleanup()

    def solleva(self, **campi):
        errore = {
            "code": "X",
            "category": "internal",
            "phase": "read",
            "remote_effect": "none",
            "retry": {"kind": "never"},
            "message": "un messaggio qualunque",
        }
        errore.update(campi)
        busta = {"status": "error", "error": errore}
        percorso = self.tmp / NOME
        percorso.write_text(
            "#!/usr/bin/env python3\nimport sys, json\n"
            f"print(json.dumps({busta!r}), file=sys.stderr)\nsys.exit(1)\n",
            encoding="utf-8",
        )
        percorso.chmod(0o755)
        with self.assertRaises(CommandFailed) as preso:
            Runner(percorso).run(["x"])
        return preso.exception

    def test_ogni_categoria_ha_la_propria_classe(self) -> None:
        from plenora_io import errors

        for categoria, classe in errors.CATEGORIE.items():
            with self.subTest(categoria=categoria):
                self.assertIsInstance(self.solleva(category=categoria), classe)

    def test_una_categoria_sconosciuta_non_e_un_guasto(self) -> None:
        """Le regole di compatibilita' consentono di estendere un vocabolario
        chiuso: un SDK che si rifiutasse di leggere la busta trasformerebbe
        un'estensione in un guasto."""
        errore = self.solleva(category="una_categoria_futura")
        self.assertIs(type(errore), CommandFailed)
        self.assertEqual(errore.envelope.category, "una_categoria_futura")

    def test_il_ritentativo_e_il_suo_ritardo(self) -> None:
        mai = self.solleva(retry={"kind": "never"})
        self.assertFalse(mai.retryable)
        self.assertIsNone(mai.retry_after_ms)

        dopo = self.solleva(retry={"kind": "after", "delay_ms": 2750})
        self.assertTrue(dopo.retryable)
        self.assertEqual(dopo.retry_after_ms, 2750)

        sicuro = self.solleva(retry={"kind": "safe"})
        self.assertTrue(sicuro.retryable)
        # `None` non vuol dire «subito»: vuol dire che il prodotto non l'ha
        # detto, e chi riprova sceglie da se'.
        self.assertIsNone(sicuro.retry_after_ms)

    def test_un_ritentativo_cieco_non_e_sicuro_in_due_stati(self) -> None:
        """La proprieta' dice che cosa **fare**, non che cosa e' successo.

        `committed` e `unknown` portano alla stessa decisione -- non ripetere
        alla cieca -- e sono due fatti diversi. Il nome dice la decisione.
        """
        for effetto, atteso in (
            ("none", False),
            ("rolled_back", False),
            ("partial", False),
            ("committed", True),
            ("unknown", True),
        ):
            with self.subTest(effetto=effetto):
                errore = self.solleva(remote_effect=effetto)
                self.assertEqual(errore.must_assume_remote_committed, atteso)

    def test_il_valore_originale_resta_leggibile(self) -> None:
        """La controprova che rende onesto il nome.

        Si chiamava `remote_committed`, e davanti a `unknown` restituiva `True`
        come se il commit fosse accertato: chi la leggeva imparava dal nome un
        fatto sbagliato. I due stati devono restare **distinguibili** per chi
        deve decidere se verificare lo stato remoto invece di riprovare.
        """
        commesso = self.solleva(remote_effect="committed")
        ignoto = self.solleva(remote_effect="unknown")

        self.assertEqual(
            commesso.must_assume_remote_committed,
            ignoto.must_assume_remote_committed,
            "la decisione e' la stessa",
        )
        self.assertNotEqual(
            commesso.envelope.remote_effect,
            ignoto.envelope.remote_effect,
            "e i due fatti restano diversi: e' cio' che il vecchio nome perdeva",
        )
        self.assertEqual(ignoto.envelope.remote_effect, "unknown")

    def test_il_nome_vecchio_non_esiste_piu(self) -> None:
        """Lasciarlo come alias terrebbe in vita l'affermazione sbagliata, e
        chi lo usasse non saprebbe mai di dover guardare altrove."""
        self.assertFalse(hasattr(self.solleva(), "remote_committed"))

    def test_il_messaggio_non_e_un_asse_su_cui_reagire(self) -> None:
        """Due errori con lo **stesso** testo e categorie diverse danno classi
        diverse: e' la proprieta' che rende inutile leggere la stringa."""
        uno = self.solleva(category="not_found", message="uguale")
        due = self.solleva(category="conflict", message="uguale")
        self.assertNotEqual(type(uno), type(due))
        self.assertEqual(uno.envelope.message, due.envelope.message)


if __name__ == "__main__":
    unittest.main()
