"""Prove che richiedono binari e GDAL: discovery nel job Rust di ogni PR."""
import io
import subprocess
import unittest
from contextlib import redirect_stdout, redirect_stderr
from scripts import check_release_contract as gate, strict_json
from scripts import check_asan_filegdb as asan_gate

class SondaDellaMisuraVera(unittest.TestCase):
    def test_il_gate_e_verde_sull_albero_corrente(self):
        uscita, errori = io.StringIO(), io.StringIO()
        with redirect_stdout(uscita), redirect_stderr(errori):
            codice = asan_gate.main([])
        self.assertEqual(codice, 0, errori.getvalue())

class SondeRinvioControQualificazione(unittest.TestCase):
    """Le due facce del rinvio, provate sui comandi che il registro dichiara.

    Un rinvio che passasse anche in qualificazione sarebbe un permesso, non una
    registrazione: la misura scaduta risulterebbe valida e nessuno la
    rifarebbe. Un rinvio che bloccasse anche lo sviluppo fermerebbe il lavoro
    fino alla campagna, che e' proprio cio' che si e' deciso di rinviare.

    Le prove non nominano i comandi: li **leggono dal registro**, cosi' che
    cambiarli li' senza cambiare il comportamento non lasci queste prove a
    verificare un comando che nessuno esegue piu'.

    # I due regimi, e perche' l'atteso non e' fisso

    Queste prove pretendevano il rinvio: sviluppo verde con `DIFFERITO`,
    qualificazione rossa. Era giusto finche' un rinvio era dichiarato, e cioe'
    fino al candidato finale -- dove le misure si rifanno, le dichiarazioni si
    chiudono e tutte e tre si invertono insieme. Un oracolo che e' lo stato
    transitorio del repository si spegne da solo proprio quando conta.

    L'atteso si legge percio' dai registri: se un rinvio e' dichiarato valgono
    le due facce di prima; se non ce n'e' nessuno, la qualificazione dev'essere
    **verde** e nessuna riga puo' dire `DIFFERITO`. Il secondo ramo non e' piu'
    debole del primo: dice che una misura rifatta non lascia dietro di se' una
    dichiarazione che nessuno rilegge.

    Che il meccanismo funzioni resta provato **in entrambi i regimi** da
    `SondeDelRinvioSuUnRegistroCostruito` in `test_check_profondita_fuzz`, che
    il rinvio se lo costruisce invece di cercarlo.
    """

    CONDIZIONI = ("profondita-fuzz-rimisurata", "confine-asan-rimisurato")

    @staticmethod
    def _registro() -> dict:
        return strict_json.loads(gate.REGISTRO.read_text(encoding="utf-8"))

    @staticmethod
    def _rinvio_dichiarato(identita: str) -> bool:
        """Se **oggi**, nei registri, un rinvio e' dichiarato per la condizione.

        Si chiede ai registri e non all'uscita del comando: chiedere al comando
        renderebbe la prova circolare, perche' e' proprio il suo esito che si
        vuole verificare.
        """
        from scripts import check_asan_filegdb as asan
        from scripts import check_profondita_fuzz as profondita

        if identita == "profondita-fuzz-rimisurata":
            return any(
                profondita.leggi_registro(bersaglio).get("rimisurazione_dovuta")
                is not None
                for bersaglio in profondita.BERSAGLI.values()
            )
        registro = profondita.leggi_registro(profondita.BERSAGLI["filegdb_reader"])
        return registro.get(asan.CHIAVE_RINVIO) is not None

    def _comando(self, identita: str) -> list[str]:
        voce = next(
            c
            for c in self._registro()["autorizzazione_di_release"]["condizioni"]
            if c["id"] == identita
        )
        self.assertEqual(voce["verifica"]["tipo"], "gate")
        return voce["verifica"]["comando"]

    def test_in_sviluppo_il_rinvio_passa(self) -> None:
        # Lo stesso gate senza `--qualifica`: e' quello che gira in CI e nel
        # checkpoint, e deve restare verde mentre la campagna e' rinviata.
        for identita in self.CONDIZIONI:
            comando = [v for v in self._comando(identita) if v != "--qualifica"]
            with self.subTest(condizione=identita):
                esito = subprocess.run(
                    comando, cwd=gate.ROOT, capture_output=True, text=True, check=False
                )
                # Su una copia di lavoro con CRLF l'impronta non e' calcolabile,
                # e il gate lo dice invece di misurare male. Non e' l'oggetto di
                # questa prova: si salta **solo** su quel messaggio, cosi' un
                # rosso vero non ci si nasconde dentro.
                if "non sarebbe riproducibile su un checkout pulito" in esito.stderr:
                    self.skipTest(
                        "copia di lavoro con fine riga non normalizzati: "
                        "l'impronta si misura su un checkout pulito, come in CI"
                    )
                self.assertEqual(esito.returncode, 0, esito.stderr[-600:])
                if self._rinvio_dichiarato(identita):
                    self.assertIn("DIFFERITO", esito.stdout, esito.stdout[-600:])
                else:
                    # Nessun rinvio dichiarato: la riga che lo annuncerebbe non
                    # puo' esserci. Una misura rifatta non lascia dietro di se'
                    # una dichiarazione che nessun gate rilegge.
                    self.assertNotIn("DIFFERITO", esito.stdout, esito.stdout[-600:])

    def test_in_qualificazione_il_rinvio_ferma(self) -> None:
        for identita in self.CONDIZIONI:
            comando = self._comando(identita)
            self.assertIn("--qualifica", comando)
            with self.subTest(condizione=identita):
                esito = subprocess.run(
                    comando, cwd=gate.ROOT, capture_output=True, text=True, check=False
                )
                if self._rinvio_dichiarato(identita):
                    self.assertNotEqual(esito.returncode, 0, esito.stdout[-600:])
                else:
                    # Senza rinvii la qualificazione non ha niente da rifiutare:
                    # pretenderla rossa vorrebbe dire pretendere che la release
                    # non si possa qualificare mai.
                    self.assertEqual(esito.returncode, 0, esito.stderr[-600:])

    def test_il_comando_di_rilascio_rifiuta_e_dice_quali(self) -> None:
        """La via vera: `verifica_condizione`, la stessa che `--release` chiama.

        Invocare qui l'intero `--release` vorrebbe dire rieseguire ogni prova di
        ogni invariante -- minuti di corsa per verificare due righe. La funzione
        che le condizioni le esegue e' questa, ed e' il punto in cui un rinvio
        deve diventare un rifiuto.
        """
        documento = self._registro()
        for identita in self.CONDIZIONI:
            condizione = next(
                c
                for c in documento["autorizzazione_di_release"]["condizioni"]
                if c["id"] == identita
            )
            with self.subTest(condizione=identita):
                motivi = gate.verifica_condizione(condizione, documento)
                if self._rinvio_dichiarato(identita):
                    self.assertTrue(
                        motivi, "un rinvio non puo' autorizzare il rilascio"
                    )
                else:
                    self.assertFalse(motivi, motivi)

    def test_le_due_condizioni_sono_obbligatorie(self) -> None:
        # Senza, la via piu' breve al verde sarebbe toglierle dal registro.
        for identita in self.CONDIZIONI:
            self.assertIn(identita, gate.CONDIZIONI_OBBLIGATORIE)


