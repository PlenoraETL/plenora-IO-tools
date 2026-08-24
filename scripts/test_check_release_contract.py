"""Sonde del contratto corrente.

Il gate ha due doveri opposti, e vanno provati entrambi: accorgersi che un
invariante si dichiari `verified` senza una prova che **passi**, e non impedire
che un blocco dichiari la propria condizione di chiusura.

La distinzione che queste sonde fissano e' la piu' facile da perdere:
`release_blocking` **puo'** avere una prova. Sono due casi diversi — un
meccanismo di verifica che esiste e oggi fallisce, e un meccanismo che non
esiste — e confonderli farebbe sparire il primo, che e' quello su cui si
lavora.

# Perche' le sonde del ramo `test` non lanciano `cargo`

La verifica ha due parti: che il harness venga **davvero eseguito**, e che la
sua uscita venga letta correttamente. La prima e' provata dalle sonde del ramo
`gate`, che lanciano processi veri e ne guardano l'exit code. La seconda e' la
parte che puo' sbagliare in silenzio — un test assente, `#[ignore]`, fallito o
omonimo letti tutti come verdi — e si prova alimentando il lettore con uscite
di harness costruite apposta. Lanciare `cargo test` per ottenere quelle uscite
aggiungerebbe minuti senza provare nulla di piu': l'esecuzione vera e' gia'
coperta.
"""

from __future__ import annotations

import contextlib
import io
import json
import pathlib
import tempfile
import unittest
from unittest import mock

from scripts import check_assurance_n1_prove as n1
from scripts import check_release_contract as gate

# Un comando che esiste, esce 0 e non tocca il repository.
VERDE = ["python3", "-c", "pass"]
ROSSO = ["python3", "-c", "raise SystemExit(3)"]


def voce(**extra):
    base = {
        "id": "wire.qualcosa",
        "superficie": "una superficie",
        "invariante": "un invariante scritto",
        "prova": {"tipo": "gate", "comando": VERDE},
        "stato": "verified",
    }
    base.update(extra)
    return base


def documento(*voci):
    return {"schema_version": 1, "invarianti": list(voci)}


def uscita(*righe: str) -> str:
    return "\n".join(f"test {r}" for r in righe) + "\n"


class SondeStruttura(unittest.TestCase):
    """Il registro e' ben formato. Non dice che le prove passino."""

    def test_un_registro_coerente_passa(self) -> None:
        """La controprova positiva: senza, «sempre rosso» sarebbe una difesa."""
        self.assertEqual(gate.struttura(documento(voce())), [])

    # --- primo dovere: `verified` senza verifica ---------------------------

    def test_verified_senza_prova_e_rosso(self) -> None:
        errori = gate.struttura(documento(voce(prova=None)))
        self.assertTrue(any("senza prova" in e for e in errori), errori)

    def test_un_tipo_di_prova_inventato_e_rosso(self) -> None:
        errori = gate.struttura(
            documento(voce(prova={"tipo": "intuizione", "comando": VERDE}))
        )
        self.assertTrue(any("tipo di prova" in e for e in errori), errori)

    def test_una_prova_senza_i_campi_del_proprio_tipo_e_rossa(self) -> None:
        """Ogni tipo dice come si esegue: senza i suoi campi non e' eseguibile."""
        errori = gate.struttura(
            documento(voce(prova={"tipo": "test", "crate": "plenora-io-model"}))
        )
        self.assertTrue(any("senza ['configurazione', 'test']" in e for e in errori), errori)

    def test_una_configurazione_inventata_e_rossa(self) -> None:
        errori = gate.struttura(
            documento(
                voce(
                    prova={
                        "tipo": "test",
                        "crate": "plenora-io-model",
                        "configurazione": "quasi-tutte",
                        "test": ["a"],
                    }
                )
            )
        )
        self.assertTrue(any("configurazione" in e for e in errori), errori)

    def test_un_bersaglio_inventato_e_rosso(self) -> None:
        """`--lib` su un crate binario non elenca nulla: il bersaglio e' parte
        dell'identita' della misura, e non si indovina."""
        errori = gate.struttura(
            documento(
                voce(
                    prova={
                        "tipo": "test",
                        "crate": "plenora-io-cli",
                        "configurazione": "default",
                        "bersaglio": "esempi",
                        "test": ["a"],
                    }
                )
            )
        )
        self.assertTrue(any("bersaglio" in e for e in errori), errori)

    def test_un_artefatto_che_non_esiste_e_rosso(self) -> None:
        errori = gate.struttura(
            documento(
                voce(
                    prova={
                        "tipo": "interna",
                        "funzione": "validate_cli_protocol_v1",
                        "artefatto": "release/mai-esistito.json",
                    }
                )
            )
        )
        self.assertTrue(any("assente" in e for e in errori), errori)

    def test_una_prova_esterna_senza_evidenza_non_puo_essere_verified(self) -> None:
        """Il caso per cui il tipo `esterna` esiste.

        Un invariante di proprieta' altrui non si chiude dichiarandolo: finche'
        l'owner non porta un esito `passed`, l'evidenza non c'e', e un
        invariante senza evidenza e' bloccante — non vero.
        """
        errori = gate.struttura(
            documento(
                voce(
                    prova={
                        "tipo": "esterna",
                        "owner": "plenora-contracts/conformance",
                        "artefatto": "release/system-rc-gate.json",
                        "stato": "not_run",
                    }
                )
            )
        )
        self.assertTrue(any("prova esterna in stato" in e for e in errori), errori)

    def test_verified_senza_invariante_scritto_e_rosso(self) -> None:
        errori = gate.struttura(documento(voce(invariante="")))
        self.assertTrue(any("senza invariante" in e for e in errori), errori)

    # --- secondo dovere: il blocco deve dire che cosa manca ----------------

    def test_release_blocking_senza_manca_e_rosso(self) -> None:
        errori = gate.struttura(
            documento(voce(stato="release_blocking", prova=None, sintesi="x"))
        )
        self.assertTrue(any("senza campo `manca`" in e for e in errori), errori)

    def test_release_blocking_senza_sintesi_e_rosso(self) -> None:
        """La riga con cui il blocco compare in `docs/RELEASE.md` vive nel
        registro: scriverla nella prosa creerebbe la seconda verita' che quella
        tabella esiste per impedire."""
        errori = gate.struttura(
            documento(voce(stato="release_blocking", prova=None, manca="niente"))
        )
        self.assertTrue(any("senza campo `sintesi`" in e for e in errori), errori)

    def test_release_blocking_puo_avere_una_prova(self) -> None:
        """La distinzione decisiva.

        ASSURANCE-N1 ha un gate che lo verifica **ed e' rosso**: il meccanismo
        esiste, l'invariante non e' ancora soddisfatto. Vietare la prova ai
        bloccanti confonderebbe questo caso con quello di una lacuna che non ha
        alcuno strumento — e i due si chiudono in modi diversi.
        """
        errori = gate.struttura(
            documento(voce(stato="release_blocking", manca="43 gruppi aperti", sintesi="rami negativi aperti"))
        )
        self.assertEqual(errori, [], errori)

    def test_release_blocking_senza_prova_va_bene(self) -> None:
        errori = gate.struttura(
            documento(voce(
                stato="release_blocking",
                prova=None,
                manca="nessuno strumento",
                sintesi="nessuno strumento",
            ))
        )
        self.assertEqual(errori, [], errori)

    # --- struttura ---------------------------------------------------------

    def test_uno_stato_inventato_e_rosso(self) -> None:
        errori = gate.struttura(documento(voce(stato="quasi")))
        self.assertTrue(any("non ammesso" in e for e in errori), errori)

    def test_un_identificatore_duplicato_e_rosso(self) -> None:
        errori = gate.struttura(documento(voce(), voce()))
        self.assertTrue(any("duplicata" in e for e in errori), errori)

    def test_campi_mancanti_sono_rossi(self) -> None:
        parziale = voce()
        del parziale["superficie"]
        errori = gate.struttura(documento(parziale))
        self.assertTrue(any("campi mancanti" in e for e in errori), errori)

    def test_il_debito_conta_solo_i_bloccanti(self) -> None:
        d = documento(
            voce(),
            voce(
                id="fuzz.lacuna",
                stato="release_blocking",
                prova=None,
                manca="niente strumento",
                sintesi="niente strumento",
            ),
        )
        self.assertEqual([v["id"] for v in gate.debito(d)], ["fuzz.lacuna"])


class SondeEsecuzioneGate(unittest.TestCase):
    """Il ramo che lancia processi veri e ne guarda l'exit code."""

    def test_un_gate_verde_passa(self) -> None:
        self.assertEqual(gate.esegui(documento(voce())), [])

    def test_un_gate_che_fallisce_e_rosso(self) -> None:
        """La differenza fra questo gate e la sua stesura precedente.

        Prima bastava che il file citato esistesse: uno strumento **presente e
        rotto** passava.
        """
        errori = gate.esegui(documento(voce(prova={"tipo": "gate", "comando": ROSSO})))
        self.assertTrue(any("esce con 3" in e for e in errori), errori)

    def test_i_comandi_aggiuntivi_sono_eseguiti(self) -> None:
        errori = gate.esegui(
            documento(
                voce(prova={"tipo": "gate", "comando": VERDE, "comandi_aggiuntivi": [ROSSO]})
            )
        )
        self.assertTrue(any("esce con 3" in e for e in errori), errori)

    def test_uno_stesso_comando_si_esegue_una_volta_sola(self) -> None:
        """Ripetere una misura non la rende piu' vera, e allunga il checkpoint."""
        with mock.patch.object(gate.subprocess, "run", wraps=gate.subprocess.run) as corse:
            gate.esegui(documento(voce(), voce(id="altro.invariante")))
        self.assertEqual(corse.call_count, 1)

    def test_i_bloccanti_non_si_eseguono(self) -> None:
        """Un bloccante puo' avere un gate rosso: e' cio' che lo rende tale."""
        errori = gate.esegui(
            documento(
                voce(
                    stato="release_blocking",
                    manca="lo strumento c'e' ed e' rosso",
                    sintesi="gate rosso",
                    prova={"tipo": "gate", "comando": ROSSO},
                )
            )
        )
        self.assertEqual(errori, [], errori)


class SondeEsecuzioneTest(unittest.TestCase):
    """Il ramo che legge l'uscita del harness.

    I quattro modi in cui un identificatore puo' non provare nulla — assente,
    `#[ignore]`, fallito, omonimo — passavano tutti nella stesura precedente.
    """

    def voce_test(self, *identita, **extra):
        prova = {
            "tipo": "test",
            "crate": "plenora-io-model",
            "configurazione": "default",
            "test": list(identita),
        }
        prova.update(extra)
        return voce(prova=prova)

    def esegui_con(self, testo: str, *voci, uscita_del_processo: int = 0):
        """Il runner e' condiviso: si sostituisce dove vive, non dove si usa."""
        finto = mock.Mock(returncode=uscita_del_processo, stdout=testo, stderr="")
        with mock.patch.object(n1.subprocess, "run", return_value=finto) as corsa:
            errori = gate.esegui(documento(*voci))
        return errori, corsa

    def test_un_test_eseguito_e_passato_va_bene(self) -> None:
        errori, _ = self.esegui_con(uscita("m::t ... ok"), self.voce_test("m::t"))
        self.assertEqual(errori, [], errori)

    def test_un_identificatore_assente_e_rosso(self) -> None:
        """Un simbolo che esiste ma non viene eseguito non verifica niente:
        puo' essere un helper senza `#[test]` o un `cfg` inattivo."""
        errori, _ = self.esegui_con(uscita("m::altro ... ok"), self.voce_test("m::t"))
        self.assertTrue(any("non compare fra i test eseguiti" in e for e in errori), errori)

    def test_un_test_ignorato_e_rosso(self) -> None:
        errori, _ = self.esegui_con(uscita("m::t ... ignored"), self.voce_test("m::t"))
        self.assertTrue(any("`#[ignore]`" in e for e in errori), errori)

    def test_un_test_fallito_e_rosso(self) -> None:
        errori, _ = self.esegui_con(uscita("m::t ... FAILED"), self.voce_test("m::t"))
        self.assertTrue(any("non passa" in e for e in errori), errori)

    def test_un_identificatore_omonimo_e_rosso(self) -> None:
        """Con due test omonimi il registro non puo' dire quale chiude la voce."""
        errori, _ = self.esegui_con(
            uscita("m::t ... ok", "m::t ... ok"), self.voce_test("m::t")
        )
        self.assertTrue(any("duplicata" in e for e in errori), errori)

    def test_un_elenco_vuoto_non_e_un_verde(self) -> None:
        errori, _ = self.esegui_con("", self.voce_test("m::t"))
        self.assertTrue(any("silenzio non va letto come un verde" in e for e in errori), errori)

    def test_un_harness_che_fallisce_e_rosso(self) -> None:
        """Il contratto usa lo stesso runner, e con esso lo stesso rifiuto."""
        errori, _ = self.esegui_con(
            uscita("m::t ... ok"), self.voce_test("m::t"), uscita_del_processo=9
        )
        self.assertTrue(any("esce con 9" in e for e in errori), errori)

    def test_il_bersaglio_dichiarato_finisce_nel_comando(self) -> None:
        """Il crate della CLI e' binario: `--lib` non elencherebbe nulla."""
        _, corsa = self.esegui_con(
            uscita("tests::t ... ok"),
            voce(
                prova={
                    "tipo": "test",
                    "crate": "plenora-io-cli",
                    "configurazione": "default",
                    "bersaglio": "bins",
                    "test": ["tests::t"],
                }
            ),
        )
        self.assertEqual(
            corsa.call_args.args[0],
            ["cargo", "test", "-p", "plenora-io-cli", "--bins"],
        )

    def test_una_coppia_si_misura_una_volta_sola(self) -> None:
        _, corsa = self.esegui_con(
            uscita("m::a ... ok", "m::b ... ok"),
            self.voce_test("m::a"),
            voce(id="altro.invariante", prova={
                "tipo": "test",
                "crate": "plenora-io-model",
                "configurazione": "default",
                "test": ["m::b"],
            }),
        )
        self.assertEqual(corsa.call_count, 1)


class SondeEsecuzioneInterna(unittest.TestCase):
    def test_una_funzione_inesistente_e_rossa(self) -> None:
        errori = gate.esegui(
            documento(
                voce(
                    prova={
                        "tipo": "interna",
                        "funzione": "validate_mai_scritta",
                        "artefatto": "release/cli-protocol-v1.json",
                    }
                )
            )
        )
        self.assertTrue(any("non esiste" in e for e in errori), errori)

    def test_la_funzione_dichiarata_e_eseguita_sull_artefatto(self) -> None:
        errori = gate.esegui(
            documento(
                voce(
                    prova={
                        "tipo": "interna",
                        "funzione": "validate_cli_protocol_v1",
                        "artefatto": "release/cli-protocol-v1.json",
                    }
                )
            )
        )
        self.assertEqual(errori, [], errori)


class SondeProveVuote(unittest.TestCase):
    """Una prova che non chiede niente e' verde per assenza di domanda.

    `"test": []` passava: il harness girava, nessuna identita' veniva cercata,
    e l'invariante risultava verificato. E' la stessa famiglia dell'elenco
    vuoto del harness — quello e' gia' rosso — vista dal lato del registro.
    """

    def prova_test(self, elenco, **extra):
        prova = {
            "tipo": "test",
            "crate": "plenora-io-model",
            "configurazione": "default",
            "test": elenco,
        }
        prova.update(extra)
        return gate.struttura(documento(voce(prova=prova)))

    def test_un_elenco_di_test_vuoto_e_rosso(self) -> None:
        errori = self.prova_test([])
        self.assertTrue(any("elenco vuoto" in e for e in errori), errori)

    def test_un_elenco_di_test_non_tipizzato_e_rosso(self) -> None:
        for storto in ([None], [42], [""], "m::t"):
            with self.subTest(storto=storto):
                errori = self.prova_test(storto)
                self.assertTrue(errori, storto)

    def test_un_identificatore_ripetuto_nel_registro_e_rosso(self) -> None:
        """Nominare due volte lo stesso test non lo esegue due volte."""
        errori = self.prova_test(["m::t", "m::t"])
        self.assertTrue(any("ripetuti" in e for e in errori), errori)

    def test_un_elenco_di_test_pieno_passa(self) -> None:
        self.assertEqual(self.prova_test(["m::a", "m::b"]), [])

    def test_un_comando_vuoto_e_rosso(self) -> None:
        errori = gate.struttura(documento(voce(prova={"tipo": "gate", "comando": []})))
        self.assertTrue(any("assente o vuoto" in e for e in errori), errori)

    def test_un_comando_che_e_una_riga_di_shell_e_rosso(self) -> None:
        errori = gate.struttura(
            documento(voce(prova={"tipo": "gate", "comando": ["python3 -c pass"]}))
        )
        self.assertTrue(any("riga di shell" in e for e in errori), errori)

    def test_un_comando_aggiuntivo_vuoto_e_rosso(self) -> None:
        errori = gate.struttura(
            documento(
                voce(prova={"tipo": "gate", "comando": VERDE, "comandi_aggiuntivi": [[]]})
            )
        )
        self.assertTrue(any("assente o vuoto" in e for e in errori), errori)


class SondeProvaEsterna(unittest.TestCase):
    """Lo stato di una qualifica esterna si legge dall'artefatto."""

    def esterna(self, dichiarato: str, **extra):
        prova = {
            "tipo": "esterna",
            "owner": "plenora-contracts/conformance",
            "artefatto": "release/system-rc-gate.json",
            "stato": dichiarato,
        }
        return gate.struttura(documento(voce(prova=prova, **extra)))

    def test_un_passed_autocertificato_e_rosso(self) -> None:
        """Il caso che rendeva il tipo `esterna` una dichiarazione.

        Bastava scrivere `passed` accanto a un artefatto che dice `not_run` per
        rendere `verified` un invariante di proprieta' altrui.
        """
        errori = self.esterna("passed")
        self.assertTrue(any("autocertificarlo" in e for e in errori), errori)

    def test_lo_stato_che_coincide_con_l_artefatto_passa(self) -> None:
        errori = self.esterna(
            "not_run",
            stato="release_blocking",
            manca="gate di sistema non superato",
            sintesi="gate di sistema non superato",
        )
        self.assertEqual(errori, [], errori)

    def test_anche_un_bloccante_che_si_autocertifica_e_rosso(self) -> None:
        """Un bloccante non produce un verde, ma la divergenza resta."""
        errori = self.esterna(
            "passed",
            stato="release_blocking",
            manca="gate di sistema non superato",
            sintesi="gate di sistema non superato",
        )
        self.assertTrue(any("autocertificarlo" in e for e in errori), errori)

    def test_un_artefatto_senza_evidence_non_deriva_uno_stato(self) -> None:
        prova = {
            "tipo": "esterna",
            "owner": "plenora-contracts/conformance",
            "artefatto": "release/cli-protocol-v1.json",
            "stato": "passed",
        }
        errori = gate.struttura(documento(voce(prova=prova)))
        self.assertTrue(any("non_derivabile" in e for e in errori), errori)


class SondeCompletezza(unittest.TestCase):
    """Il registro nel suo insieme.

    `struttura` guarda una voce alla volta, e su una lista vuota non ha voci da
    guardare: **svuotare il registro era un verde**. Queste sonde provano la
    proprieta' opposta — che togliere qualcosa sia rosso — perche' e' la via
    piu' breve per un falso verde e non lascia traccia in nessun conteggio.
    """

    def registro(self) -> dict:
        return json.loads(gate.REGISTRO.read_text(encoding="utf-8"))

    def test_il_registro_reale_e_completo(self) -> None:
        """La controprova positiva: senza, «sempre rosso» sarebbe una difesa."""
        self.assertEqual(gate.completezza(self.registro()), [])

    def test_un_registro_vuoto_e_rosso(self) -> None:
        errori = gate.completezza({"schema_version": 1, "invarianti": []})
        self.assertTrue(any("non e' un contratto soddisfatto" in e for e in errori), errori)

    def test_un_registro_senza_invarianti_e_rosso(self) -> None:
        errori = gate.completezza({"schema_version": 1})
        self.assertTrue(any("assente o vuoto" in e for e in errori), errori)

    def test_uno_schema_non_dichiarato_e_rosso(self) -> None:
        documento = self.registro()
        del documento["schema_version"]
        errori = gate.completezza(documento)
        self.assertTrue(any("schema_version" in e for e in errori), errori)

    # --- la rimozione di un invariante obbligatorio ------------------------
    #
    # Uno per famiglia, e non un ciclo su tutto l'insieme: un ciclo proverebbe
    # la stessa riga di codice venticinque volte e chiamerebbe copertura la
    # ripetizione. Questi sono i quattro casi che la seconda lettura ha
    # chiesto di sondare per nome.

    def senza(self, identita: str) -> list[str]:
        documento = self.registro()
        documento["invarianti"] = [
            v for v in documento["invarianti"] if v["id"] != identita
        ]
        return gate.completezza(documento)

    def test_togliere_la_candidate_e_rosso(self) -> None:
        errori = self.senza("release.candidate-non-valida-per-head")
        self.assertTrue(
            any("release.candidate-non-valida-per-head" in e for e in errori), errori
        )

    def test_togliere_un_lotto_e_rosso(self) -> None:
        for lotto in ("lotto.s10", "lotto.s11", "lotto.s12"):
            with self.subTest(lotto=lotto):
                errori = self.senza(lotto)
                self.assertTrue(any(lotto in e for e in errori), errori)

    def test_togliere_la_copertura_negativa_e_rosso(self) -> None:
        errori = self.senza("copertura.rami-negativi")
        self.assertTrue(any("copertura.rami-negativi" in e for e in errori), errori)

    def test_togliere_la_qualifica_cross_component_e_rosso(self) -> None:
        errori = self.senza("sistema.qualifica-cross-component")
        self.assertTrue(
            any("sistema.qualifica-cross-component" in e for e in errori), errori
        )

    def test_togliere_un_verificato_e_rosso(self) -> None:
        """Anche una garanzia che oggi passa, sparendo, smette di essere pretesa."""
        errori = self.senza("wire.cli-protocol-v1")
        self.assertTrue(any("wire.cli-protocol-v1" in e for e in errori), errori)

    # --- le condizioni dell'autorizzazione ---------------------------------

    def test_togliere_una_condizione_e_rosso(self) -> None:
        documento = self.registro()
        documento["autorizzazione_di_release"]["condizioni"] = [
            c
            for c in documento["autorizzazione_di_release"]["condizioni"]
            if c["id"] != "decisione-scritta"
        ]
        errori = gate.completezza(documento)
        self.assertTrue(any("decisione-scritta" in e for e in errori), errori)

    def test_le_condizioni_in_prosa_sono_rosse(self) -> None:
        """Erano cinque frasi che nessuno eseguiva."""
        documento = self.registro()
        documento["autorizzazione_di_release"]["condizioni"] = [
            "nessun invariante `release_blocking` in questo registro"
        ]
        errori = gate.completezza(documento)
        self.assertTrue(any("non strutturata" in e for e in errori), errori)

    def test_una_condizione_senza_autorizzazione_e_rossa(self) -> None:
        documento = self.registro()
        del documento["autorizzazione_di_release"]
        errori = gate.completezza(documento)
        self.assertTrue(any("autorizzazione_di_release" in e for e in errori), errori)

    def test_una_funzione_di_condizione_inesistente_e_rossa(self) -> None:
        documento = self.registro()
        documento["autorizzazione_di_release"]["condizioni"][0]["verifica"] = {
            "tipo": "interna",
            "funzione": "condizione_mai_scritta",
        }
        errori = gate.completezza(documento)
        self.assertTrue(any("non esiste in questo gate" in e for e in errori), errori)

    def test_un_comando_di_condizione_in_una_riga_sola_e_rosso(self) -> None:
        """`["gate.py --release"]` non e' un argv: e' shell dentro un argomento."""
        documento = self.registro()
        documento["autorizzazione_di_release"]["condizioni"][1]["verifica"]["comando"] = [
            "python3 scripts/check_assurance_n1.py --release"
        ]
        errori = gate.completezza(documento)
        self.assertTrue(any("una riga di shell" in e for e in errori), errori)


class SondeCondizioni(unittest.TestCase):
    """Ogni condizione e' **eseguita**, e nessuna si legge da se stessa."""

    def registro(self) -> dict:
        return json.loads(gate.REGISTRO.read_text(encoding="utf-8"))

    def test_i_bloccanti_correnti_negano_la_prima_condizione(self) -> None:
        motivi = gate.condizione_nessun_bloccante(self.registro())
        self.assertTrue(motivi)
        self.assertTrue(any("restano bloccanti" in m for m in motivi), motivi)

    def test_senza_bloccanti_la_prima_condizione_e_soddisfatta(self) -> None:
        senza_blocchi = documento(voce())
        self.assertEqual(gate.condizione_nessun_bloccante(senza_blocchi), [])

    def test_la_decisione_scritta_non_e_presa(self) -> None:
        """`release_authorized` e' false: la condizione deve dirlo, non dedurlo."""
        motivi = gate.condizione_decisione_scritta(self.registro())
        self.assertTrue(any("decisione scritta" in m for m in motivi), motivi)

    def test_la_candidate_corrente_non_e_coerente(self) -> None:
        motivi = gate.condizione_candidate_coerente(self.registro())
        self.assertTrue(
            any("non qualifica il codice corrente" in m for m in motivi), motivi
        )

    def test_una_candidate_legata_al_nulla_non_soddisfa_la_condizione(self) -> None:
        """La condizione di `--release` usava lo stesso confronto per prefisso.

        Chiuderlo solo nel legame lascerebbe aperta la via che porta al verde:
        `revisione_manifesto = ""` e' prefisso di HEAD, quindi la candidate
        risultava coerente con la revisione corrente.
        """
        stato = json.loads(gate.STATO_CORRENTE.read_text(encoding="utf-8"))
        stato["aperto"]["candidate_release"]["revisione_manifesto"] = ""
        with mock.patch.object(gate, "_stato_corrente", return_value=(stato, [])):
            motivi = gate.condizione_candidate_coerente(self.registro())
        self.assertTrue(
            any("non risolve a un commit" in m for m in motivi), motivi
        )

    def test_la_qualifica_cross_component_non_e_superata(self) -> None:
        """L'esito viene dall'artefatto, non dal campo che la voce dichiara."""
        motivi = gate.condizione_qualifica_cross_component(self.registro())
        self.assertTrue(any("evidence.status" in m for m in motivi), motivi)

    def test_una_condizione_gate_esegue_davvero(self) -> None:
        rosso = {
            "id": "finta",
            "descrizione": "una condizione che fallisce",
            "verifica": {"tipo": "gate", "comando": ROSSO},
        }
        motivi = gate.verifica_condizione(rosso, self.registro())
        self.assertTrue(any("esce con 3" in m for m in motivi), motivi)

    def test_una_condizione_gate_verde_e_soddisfatta(self) -> None:
        verde = {
            "id": "finta",
            "descrizione": "una condizione che passa",
            "verifica": {"tipo": "gate", "comando": VERDE},
        }
        self.assertEqual(gate.verifica_condizione(verde, self.registro()), [])

    def test_release_e_rossa_sul_registro_corrente(self) -> None:
        """La controprova d'insieme: oggi la release non e' autorizzabile.

        `esegui` e' sostituito perche' lancerebbe `cargo` e i gate del
        workspace: qui si prova la **composizione** delle condizioni, e la
        loro esecuzione vera e' provata una per una qui sopra.
        """
        with mock.patch.object(gate, "esegui", return_value=[]):
            with contextlib.redirect_stderr(io.StringIO()) as riportato:
                esito = gate.main(["--release"])
        self.assertEqual(esito, 1)
        for identita in gate.CONDIZIONI_OBBLIGATORIE:
            self.assertIn(identita, riportato.getvalue())


class SondeStatoEsterno(unittest.TestCase):
    """Lo stato di una qualifica esterna si **deriva**, non si dichiara."""

    def test_l_artefatto_reale_non_dice_passed(self) -> None:
        stato, motivi = gate.stato_esterno_osservato("release/system-rc-gate.json")
        self.assertNotEqual(stato, "passed")
        self.assertTrue(motivi)

    def test_un_artefatto_assente_non_e_uno_stato(self) -> None:
        stato, motivi = gate.stato_esterno_osservato("release/mai-esistito.json")
        self.assertEqual(stato, "assente")
        self.assertTrue(motivi)


class SondeFontiLegate(unittest.TestCase):
    """`current-state.json` non e' una fonte: e' una **giunzione**.

    Riporta numeri che vivono altrove e li rende a `docs/RELEASE.md` con la
    stessa autorita' con cui li renderebbe la fonte. Erano ricopiati a mano, e
    una cifra sbagliata nella copia era indistinguibile da una misura diversa.
    """

    def stato(self) -> dict:
        return json.loads(gate.STATO_CORRENTE.read_text(encoding="utf-8"))

    def test_lo_stato_reale_coincide_con_le_proprie_fonti(self) -> None:
        self.assertEqual(gate.validate_stato_corrente(self.stato()), [])

    # --- la promessa vale per **ogni** foglia -----------------------------
    #
    # Il registro prometteva che ogni numero venisse dalla propria fonte, e il
    # validatore ne verificava tre famiglie. Le altre foglie stavano nel
    # documento senza che nulla le guardasse: portare `componenti_a_zero` a 999
    # non produceva un errore, e la promessa restava scritta.
    #
    # Questa sonda non prova un campo scelto a mano: prova **tutti** quelli
    # dichiarati legati. Una foglia messa in `FOGLIE_LEGATE` senza una verifica
    # che la copra fa rossa questa sonda, quindi l'elenco non puo' mentire.

    @staticmethod
    def muta(valore):
        if isinstance(valore, bool):
            return not valore
        if isinstance(valore, int):
            return valore + 999
        if isinstance(valore, float):
            return valore + 999.0
        if isinstance(valore, str):
            return valore + "-non-derivato"
        if isinstance(valore, list):
            return []
        return "non-derivato"

    @staticmethod
    def imposta(documento: dict, percorso: list[str], valore) -> None:
        nodo = documento
        for chiave in percorso[:-1]:
            nodo = nodo[chiave]
        nodo[percorso[-1]] = valore

    def test_ogni_foglia_legata_e_davvero_verificata(self) -> None:
        for foglia in sorted(gate.FOGLIE_LEGATE):
            with self.subTest(foglia=foglia):
                percorso = foglia.split(".")
                stato = self.stato()
                nodo = stato
                for chiave in percorso[:-1]:
                    nodo = nodo[chiave]
                self.imposta(stato, percorso, self.muta(nodo[percorso[-1]]))
                self.assertNotEqual(
                    gate.validate_stato_corrente(stato),
                    [],
                    f"«{foglia}» e' dichiarata legata ma nessuna verifica la copre",
                )

    def test_una_foglia_nuova_non_classificata_e_rossa(self) -> None:
        """Il caso futuro: un campo aggiunto e mai collegato."""
        stato = self.stato()
        stato["ultima_misura"]["copertura"]["percentuale_inventata"] = 99.9
        errori = gate.validate_stato_corrente(stato)
        self.assertTrue(any("non classificata" in e for e in errori), errori)

    def test_una_foglia_dichiarata_ma_sparita_e_rossa(self) -> None:
        """Una classificazione che descrive un campo che non c'e' piu'."""
        stato = self.stato()
        del stato["blocchi"]["nota"]
        errori = gate.validate_stato_corrente(stato)
        self.assertTrue(any("dichiarata ma assente" in e for e in errori), errori)

    def test_le_due_classificazioni_non_si_sovrappongono(self) -> None:
        self.assertEqual(gate.FOGLIE_LEGATE & set(gate.FOGLIE_DICHIARATE), set())

    def test_ogni_foglia_dichiarata_porta_la_propria_ragione(self) -> None:
        senza = [f for f, r in gate.FOGLIE_DICHIARATE.items() if not r]
        self.assertEqual(senza, [])

    # --- le famiglie che la seconda lettura ha trovato scoperte -----------

    def test_il_censimento_s9_viene_dal_censimento(self) -> None:
        for chiave in ("componenti_a_zero", "censimento_costruttori_legacy"):
            with self.subTest(chiave=chiave):
                stato = self.stato()
                stato["chiuso"]["s9_errori_strutturati"][chiave] = 999
                errori = gate.validate_stato_corrente(stato)
                self.assertTrue(any(chiave in e for e in errori), errori)

    def test_la_profondita_del_fuzzing_viene_dalla_misura(self) -> None:
        """Il numero dei requisiti raggiunti non e' leggibile a occhio, ed e' per
        questo il posto piu' facile in cui scriverne uno piu' bello."""
        stato = self.stato()
        stato["chiuso"]["fuzz_reader_shapefile"]["requisiti_di_profondita"] = 999
        errori = gate.validate_stato_corrente(stato)
        self.assertTrue(
            any("requisiti_di_profondita" in e for e in errori), errori
        )

    def test_la_misura_citata_e_quella_che_il_gate_legge(self) -> None:
        """Puntare lo stato a un altro file lo renderebbe verde su una misura
        che nessun gate guarda."""
        stato = self.stato()
        stato["chiuso"]["fuzz_reader_shapefile"]["misura"] = "assurance/altra.json"
        errori = gate.validate_stato_corrente(stato)
        self.assertTrue(any("il registro dei requisiti dichiara" in e for e in errori), errori)

    def test_i_numeri_del_filegdb_vengono_dalle_due_misure(self) -> None:
        """Quattro numeri, nessuno leggibile a occhio, e il piu' comodo da
        scrivere sarebbe proprio quello che dice che GDAL e' coperto."""
        casi = {
            "requisiti_di_profondita": 999,
            "contatori_di_copertura": 999,
            "file_sorgente_gdal_strumentati": 999,
        }
        for chiave, valore in casi.items():
            with self.subTest(chiave):
                stato = self.stato()
                stato["chiuso"]["fuzz_filegdb"][chiave] = valore
                errori = gate.validate_stato_corrente(stato)
                self.assertTrue(any(chiave in e for e in errori), errori)

    def test_il_confine_asan_citato_e_quello_che_il_gate_legge(self) -> None:
        stato = self.stato()
        stato["chiuso"]["fuzz_filegdb"]["confine_asan"] = "assurance/altro.json"
        errori = gate.validate_stato_corrente(stato)
        self.assertTrue(any("misura del confine sta in" in e for e in errori), errori)

    def test_una_qualifica_su_una_revisione_inesistente_e_rossa(self) -> None:
        stato = self.stato()
        stato["chiuso"]["s9_errori_strutturati"]["qualificato_su"] = "0" * 40
        errori = gate.validate_stato_corrente(stato)
        self.assertTrue(any("non risolve a un commit" in e for e in errori), errori)

    def test_l_ascendenza_da_head_non_e_una_qualifica(self) -> None:
        """Il difetto che questo legame chiude.

        Il campo era verificato come **antenato di HEAD**, e il commit radice
        del repository e' antenato di tutto: passava. Passava qualunque
        revisione della storia, cioe' la verifica non distingueva una qualifica
        da una parentela.
        """
        radice = gate._git("rev-list", "--max-parents=0", "HEAD").split()[-1]
        self.assertTrue(
            gate._git_riesce("merge-base", "--is-ancestor", radice, "HEAD"),
            "la radice deve essere antenato di HEAD, altrimenti la sonda non prova niente",
        )
        stato = self.stato()
        stato["chiuso"]["s9_errori_strutturati"]["qualificato_su"] = radice
        errori = gate.validate_stato_corrente(stato)
        self.assertTrue(
            any("la corsa di livello 2 registrata dallo stato" in e for e in errori),
            errori,
        )

    def test_una_qualifica_senza_il_passo_del_censimento_e_rossa(self) -> None:
        """Una corsa che non ha misurato il censimento non attesta la chiusura."""
        finta = self.evidenza_con()
        finta["artefatti"] = dict(finta["artefatti"])
        finta["artefatti"]["manifest"] = {
            nome: digest
            for nome, digest in finta["artefatti"]["manifest"].items()
            if nome != gate.LOG_DEL_CENSIMENTO
        }
        with mock.patch.object(gate, "_evidenza", return_value=finta):
            errori = gate.validate_stato_corrente(self.stato())
        self.assertTrue(
            any(gate.LOG_DEL_CENSIMENTO in e for e in errori), errori
        )

    def test_i_conteggi_del_docset_vengono_dall_allowlist(self) -> None:
        for chiave in ("markdown_canonici", "markdown_operativi"):
            with self.subTest(chiave=chiave):
                stato = self.stato()
                stato["docset"][chiave] = 999
                errori = gate.validate_stato_corrente(stato)
                self.assertTrue(any(chiave in e for e in errori), errori)

    def test_un_blocco_negato_dallo_stato_e_rosso(self) -> None:
        """`release_blocking: false` accanto a un invariante che blocca."""
        stato = self.stato()
        stato["aperto"]["loss_report"]["release_blocking"] = False
        errori = gate.validate_stato_corrente(stato)
        self.assertTrue(any("wire.loss-report" in e for e in errori), errori)

    def test_un_lotto_dichiarato_chiuso_e_rosso(self) -> None:
        stato = self.stato()
        stato["aperto"]["lotti"]["s11"] = "chiuso"
        errori = gate.validate_stato_corrente(stato)
        self.assertTrue(any("lotti.s11" in e for e in errori), errori)

    def test_la_fonte_dei_blocchi_e_il_registro(self) -> None:
        stato = self.stato()
        stato["blocchi"]["fonte"] = "assurance/registries/un-altro.json"
        errori = gate.validate_stato_corrente(stato)
        self.assertTrue(any("blocchi.fonte" in e for e in errori), errori)

    # --- evidenza ----------------------------------------------------------

    def test_un_numero_che_non_viene_dall_evidenza_e_rosso(self) -> None:
        stato = self.stato()
        stato["ultima_misura"]["copertura"]["lcov_percentuale"] = 99.9
        errori = gate.validate_stato_corrente(stato)
        self.assertTrue(any("lcov_percentuale" in e for e in errori), errori)

    def test_un_conteggio_di_passi_ritoccato_e_rosso(self) -> None:
        stato = self.stato()
        stato["ultima_misura"]["checkpoint"]["passi_falliti"] = 0
        stato["ultima_misura"]["checkpoint"]["passi_verdi"] = 58
        errori = gate.validate_stato_corrente(stato)
        self.assertTrue(any("passi_verdi" in e for e in errori), errori)

    def test_un_evidenza_che_descrive_un_altra_revisione_e_rossa(self) -> None:
        """Una revisione che git non risolve non identifica un albero."""
        stato = self.stato()
        stato["ultima_misura"]["sha"] = "0000000"
        errori = gate.validate_stato_corrente(stato)
        self.assertTrue(any("non risolve a un commit" in e for e in errori), errori)

    def test_un_evidenza_che_non_nomina_la_revisione_e_rossa(self) -> None:
        """Il nome dell'evidenza lega la corsa alla revisione.

        La revisione qui **esiste** — e' la baseline documentale — quindi si
        risolve, e cio' che resta scoperto e' il nome del file.
        """
        stato = self.stato()
        stato["ultima_misura"]["sha"] = gate.BASELINE_DOCSET
        errori = gate.validate_stato_corrente(stato)
        self.assertTrue(any("non nomina la revisione" in e for e in errori), errori)

    def test_senza_evidenza_i_numeri_non_hanno_una_corsa(self) -> None:
        stato = self.stato()
        del stato["ultima_misura"]["evidenza"]
        errori = gate.validate_stato_corrente(stato)
        self.assertTrue(any("non hanno una corsa" in e for e in errori), errori)

    # --- una revisione si risolve, non si confronta per prefisso ----------
    #
    # `startswith` e l'appartenenza a un nome di file accettano la **stringa
    # vuota**: e' prefisso di tutto e sottostringa di tutto. `ultima_misura.sha
    # = ""` passava ogni controllo, e con esso `ultima_qualificata.sha = ""`;
    # `revisione_manifesto = ""` passava accanto a `qualifica_head: true`, che
    # e' una qualifica fabbricata dichiarata su niente.

    def test_uno_sha_vuoto_non_e_una_revisione(self) -> None:
        stato = self.stato()
        stato["ultima_misura"]["sha"] = ""
        stato["revisioni"]["ultima_qualificata"]["sha"] = ""
        errori = gate.validate_stato_corrente(stato)
        self.assertTrue(any("ultima_misura.sha" in e for e in errori), errori)
        self.assertTrue(any("ultima_qualificata.sha" in e for e in errori), errori)

    def test_uno_sha_troppo_corto_non_e_una_revisione(self) -> None:
        """git rifiuta un prefisso piu' corto di quattro caratteri."""
        stato = self.stato()
        stato["ultima_misura"]["sha"] = "c9"
        errori = gate.validate_stato_corrente(stato)
        self.assertTrue(any("non risolve a un commit" in e for e in errori), errori)

    def test_una_candidate_legata_al_nulla_non_qualifica_head(self) -> None:
        stato = self.stato()
        stato["aperto"]["candidate_release"]["revisione_manifesto"] = ""
        stato["aperto"]["candidate_release"]["qualifica_head"] = True
        errori = gate.validate_stato_corrente(stato)
        self.assertTrue(any("revisione_manifesto" in e for e in errori), errori)
        self.assertTrue(any("qualifica_head" in e for e in errori), errori)

    def test_revisione_risolta_rifiuta_cio_che_non_e_un_commit(self) -> None:
        for storto in ("", "c9", "0" * 40, "non-una-revisione", None, 7):
            with self.subTest(storto=storto):
                self.assertIsNone(gate.revisione_risolta(storto))

    def test_revisione_risolta_restituisce_lo_sha_intero(self) -> None:
        risolta = gate.revisione_risolta("HEAD")
        self.assertIsNotNone(risolta)
        self.assertEqual(len(risolta), 40)
        self.assertEqual(gate.revisione_risolta(risolta[:8]), risolta)

    # --- l'esito dell'evidenza si confronta per intero --------------------

    def evidenza_con(self, **extra):
        """L'evidenza **che lo stato indica**, con i campi sostituiti.

        Il percorso non si scrive qui: una sonda che nomina un'evidenza
        diventerebbe rossa al checkpoint successivo, e verrebbe aggiornata a
        mano ogni volta invece di seguire la fonte.
        """
        relativo = self.stato()["ultima_misura"]["evidenza"]
        documento = json.loads((gate.ROOT / relativo).read_text(encoding="utf-8"))
        documento.update(extra)
        return documento

    def test_un_esito_che_contiene_la_frase_non_e_la_frase(self) -> None:
        """«S9 checkpoint level 2 passed, con riserva» conteneva la frase."""
        finta = self.evidenza_con(esito=f"{gate.ESITO_LIVELLO_2}, con riserva")
        with mock.patch.object(gate, "_evidenza", return_value=finta):
            errori = gate.validate_stato_corrente(self.stato())
        self.assertTrue(any("pretende esattamente" in e for e in errori), errori)

    def test_un_evidenza_di_livello_1_non_puo_essere_l_ultima_misura(self) -> None:
        finta = self.evidenza_con(esito="S9 livello 1 verificato")
        with mock.patch.object(gate, "_evidenza", return_value=finta):
            errori = gate.validate_stato_corrente(self.stato())
        self.assertTrue(any("pretende esattamente" in e for e in errori), errori)

    def test_l_evidenza_reale_porta_l_esito_esatto(self) -> None:
        """La controprova positiva: senza, «sempre rosso» sarebbe una difesa."""
        with mock.patch.object(gate, "_evidenza", return_value=self.evidenza_con()):
            self.assertEqual(gate.validate_stato_corrente(self.stato()), [])

    def test_la_baseline_differenziale_viene_dall_evidenza(self) -> None:
        """La prosa nominava una baseline diversa da quella della corsa."""
        stato = self.stato()
        stato["ultima_misura"]["diagnostica_differenziale"]["baseline"] = "0fb799d"
        errori = gate.validate_stato_corrente(stato)
        self.assertTrue(any("diagnostica_differenziale.baseline" in e for e in errori), errori)

    # --- ASSURANCE-N1 ------------------------------------------------------

    def test_un_conteggio_n1_ritoccato_e_rosso(self) -> None:
        stato = self.stato()
        stato["aperto"]["assurance_n1"]["gruppi_aperti"] = 3
        errori = gate.validate_stato_corrente(stato)
        self.assertTrue(any("gruppi_aperti" in e for e in errori), errori)

    def test_un_totale_n1_ritoccato_e_rosso(self) -> None:
        stato = self.stato()
        stato["aperto"]["assurance_n1"]["gruppi_totali"] = 50
        errori = gate.validate_stato_corrente(stato)
        self.assertTrue(any("gruppi_totali" in e for e in errori), errori)

    # --- candidate ---------------------------------------------------------

    def test_un_tag_dichiarato_inesistente_e_rosso(self) -> None:
        """Il difetto che questo legame ha trovato: `v1.0.1` esisteva."""
        stato = self.stato()
        stato["aperto"]["candidate_release"]["tag_creato"] = False
        errori = gate.validate_stato_corrente(stato)
        self.assertTrue(any("tag_creato" in e for e in errori), errori)

    def test_una_qualifica_di_head_fabbricata_e_rossa(self) -> None:
        stato = self.stato()
        stato["aperto"]["candidate_release"]["qualifica_head"] = True
        errori = gate.validate_stato_corrente(stato)
        self.assertTrue(any("qualifica_head" in e for e in errori), errori)

    def test_una_versione_di_workspace_inventata_e_rossa(self) -> None:
        stato = self.stato()
        stato["aperto"]["candidate_release"]["versione_workspace"] = "2.0.0"
        errori = gate.validate_stato_corrente(stato)
        self.assertTrue(any("versione_workspace" in e for e in errori), errori)

    def test_un_tag_che_si_dichiara_su_head_e_rosso(self) -> None:
        stato = self.stato()
        stato["aperto"]["candidate_release"]["tag_su_head"] = True
        errori = gate.validate_stato_corrente(stato)
        self.assertTrue(any("tag_su_head" in e for e in errori), errori)


class SondeEvidenzaCoerente(unittest.TestCase):
    """L'evidenza e' coerente **con se stessa**, prima che con lo stato.

    Il legame copiava i campi dichiarati e ne confrontava uno solo: la
    revisione finale. Un'evidenza che dichiarava una `revisione_iniziale`
    diversa da quella finale, o due impronte diverse, restava accettata — e lo
    stato le restava fedele. Una copia fedele di un documento che si
    contraddice non e' una verifica.
    """

    def stato(self) -> dict:
        return json.loads(gate.STATO_CORRENTE.read_text(encoding="utf-8"))

    def evidenza(self) -> dict:
        return gate._evidenza(self.stato()["ultima_misura"]["evidenza"])

    def errori_con(self, muta) -> list[str]:
        finta = self.evidenza()
        muta(finta)
        with mock.patch.object(gate, "_evidenza", return_value=finta):
            return gate.validate_stato_corrente(self.stato())

    def test_l_evidenza_reale_e_coerente(self) -> None:
        """La controprova positiva: senza, «sempre rosso» sarebbe una difesa."""
        self.assertEqual(gate.evidenza_coerente(self.evidenza(), None), [])

    # --- revisione ---------------------------------------------------------

    def test_due_revisioni_diverse_nella_stessa_corsa_sono_rosse(self) -> None:
        """HEAD si e' mosso durante la corsa: la misura descrive un albero e
        l'esito ne nomina un altro."""
        radice = gate._git("rev-list", "--max-parents=0", "HEAD").split()[-1]
        errori = self.errori_con(
            lambda e: e["corsa"].update(revisione_iniziale=radice)
        )
        self.assertTrue(
            any("corsa.revisione_iniziale" in m for m in errori), errori
        )

    def test_una_revisione_di_corsa_che_non_si_risolve_e_rossa(self) -> None:
        for campo in ("revisione_iniziale", "revisione_finale"):
            with self.subTest(campo=campo):
                errori = self.errori_con(lambda e, c=campo: e["corsa"].update({c: ""}))
                self.assertTrue(
                    any(f"corsa.{campo}` non si risolve" in m for m in errori), errori
                )

    def test_una_corsa_su_un_altra_revisione_e_rossa(self) -> None:
        """Entrambe coerenti fra loro, ma non con la revisione misurata."""
        altra = gate.revisione_risolta(gate.BASELINE_DOCSET)
        errori = self.errori_con(
            lambda e: e["corsa"].update(revisione_iniziale=altra, revisione_finale=altra)
        )
        self.assertTrue(
            any("la revisione misurata" in m for m in errori), errori
        )

    # --- impronta ----------------------------------------------------------

    def test_due_impronte_diverse_sono_rosse(self) -> None:
        """Un passo ha scritto nell'albero che stava verificando."""
        errori = self.errori_con(lambda e: e["corsa"].update(impronta_finale="a" * 64))
        self.assertTrue(any("ha scritto nell'albero" in m for m in errori), errori)

    def test_un_impronta_che_non_e_uno_sha256_e_rossa(self) -> None:
        for valore in ("", "pulito", "A" * 64, "abc123"):
            with self.subTest(valore=valore):
                errori = self.errori_con(
                    lambda e, v=valore: e["corsa"].update(
                        impronta_iniziale=v, impronta_finale=v
                    )
                )
                self.assertTrue(any("non e' uno sha256" in m for m in errori), errori)

    # --- riconciliazione ---------------------------------------------------

    def test_un_conteggio_dichiarato_a_parte_e_rosso(self) -> None:
        """I sei conteggi tornavano fra loro, e nessuno chiedeva da dove
        venissero. Ora l'elenco e' la fonte e il conteggio ne e' il riassunto."""
        for campo, valore in (
            ("verdi", 56),
            ("falliti", 1),
            ("omessi", 1),
            ("duplicati", 1),
            ("eseguiti", 58),
            ("identificatori_distinti", 56),
            ("verdi", "molti"),
        ):
            with self.subTest(campo=campo, valore=valore):
                errori = self.errori_con(
                    lambda e, c=campo, v=valore: e["riconciliazione"].update({c: v})
                )
                self.assertTrue(
                    any("l'elenco dei passi ne conta" in m for m in errori), errori
                )

    def test_un_elenco_vuoto_non_e_un_verde(self) -> None:
        errori = self.errori_con(lambda e: e["riconciliazione"].update(passi=[]))
        self.assertTrue(any("assente o vuoto" in m for m in errori), errori)

    # --- misure ------------------------------------------------------------

    def test_una_percentuale_che_non_e_il_proprio_rapporto_e_rossa(self) -> None:
        errori = self.errori_con(
            lambda e: e["misure"]["copertura"].update(lcov_percentuale=99.0)
        )
        self.assertTrue(any("lcov_percentuale" in m for m in errori), errori)

    def test_piu_righe_coperte_che_strumentate_e_rosso(self) -> None:
        errori = self.errori_con(
            lambda e: e["misure"]["copertura"].update(lcov_righe_coperte=99999)
        )
        self.assertTrue(any("righe coperte" in m for m in errori), errori)

    def test_una_copertura_sotto_soglia_arrotondata_e_rossa(self) -> None:
        """Il caso che l'arrotondamento nascondeva.

        79 999/100 000 fa 79,999%, che arrotondato a due decimali e' «80,00%»:
        la percentuale dichiarata raggiungeva la soglia dell'80% e il rapporto
        no. La soglia si confronta con il rapporto.
        """
        errori = self.errori_con(
            lambda e: e["misure"]["copertura"].update(
                lcov_righe_coperte=79999,
                lcov_righe_strumentate=100000,
                lcov_percentuale=80.0,
                soglia=80.0,
            )
        )
        self.assertTrue(any("sotto la soglia" in m for m in errori), errori)

    def test_una_copertura_cargo_a_zero_e_rossa(self) -> None:
        """`coverage_soglia_controprova` gira con `--fail-under-lines`."""
        errori = self.errori_con(
            lambda e: e["misure"]["copertura"].update(cargo_lines_percentuale=0)
        )
        self.assertTrue(
            any("cargo_lines_percentuale" in m for m in errori), errori
        )

    def test_una_percentuale_non_finita_e_rossa(self) -> None:
        """`inf` e `nan` superano ogni confronto, o nessuno."""
        for valore in (float("inf"), float("nan"), float("-inf"), "molta"):
            with self.subTest(valore=valore):
                errori = self.errori_con(
                    lambda e, v=valore: e["misure"]["copertura"].update(
                        lcov_percentuale=v
                    )
                )
                self.assertTrue(any("copertura" in m for m in errori), errori)

    def test_zero_righe_strumentate_non_e_una_copertura(self) -> None:
        errori = self.errori_con(
            lambda e: e["misure"]["copertura"].update(
                lcov_righe_coperte=0, lcov_righe_strumentate=0
            )
        )
        self.assertTrue(any("strumentate" in m for m in errori), errori)

    # --- la diagnostica differenziale --------------------------------------
    #
    # I suoi numeri erano **copiati**: lo stato li prendeva dall'evidenza e il
    # gate confrontava le due copie. Due documenti modificati insieme con numeri
    # incompatibili restavano verdi, perche' nessuno chiedeva che i conteggi
    # tornassero fra loro.

    def test_coperte_piu_scoperte_devono_fare_le_cambiate(self) -> None:
        errori = self.errori_con(
            lambda e: e["misure"]["diagnostica_differenziale"].update(coperte=1)
        )
        self.assertTrue(any("non c'e' un terzo stato" in m for m in errori), errori)

    def test_la_percentuale_differenziale_e_il_proprio_rapporto(self) -> None:
        errori = self.errori_con(
            lambda e: e["misure"]["diagnostica_differenziale"].update(esito="99.99%")
        )
        self.assertTrue(
            any("diagnostica_differenziale.esito` vale 99.99" in m for m in errori),
            errori,
        )

    def test_un_esito_che_non_e_una_percentuale_ne_n_d_e_rosso(self) -> None:
        for valore in ("buono", "", "92,56%", "92.56"):
            with self.subTest(valore=valore):
                errori = self.errori_con(
                    lambda e, v=valore: e["misure"][
                        "diagnostica_differenziale"
                    ].update(esito=v)
                )
                self.assertTrue(
                    any("diagnostica_differenziale.esito" in m for m in errori), errori
                )

    def test_n_d_con_righe_da_misurare_e_rosso(self) -> None:
        """`n/d` e' la risposta quando non c'e' niente da misurare. Con righe
        eseguibili cambiate vorrebbe dire che la diagnostica ha misurato e non
        ha concluso."""
        errori = self.errori_con(
            lambda e: e["misure"]["diagnostica_differenziale"].update(esito="n/d")
        )
        self.assertTrue(any("e' una percentuale" in m for m in errori), errori)

    def test_una_percentuale_senza_denominatore_e_rossa(self) -> None:
        errori = self.errori_con(
            lambda e: e["misure"]["diagnostica_differenziale"].update(
                righe_cambiate_eseguibili=0, coperte=0, scoperte=0
            )
        )
        self.assertTrue(any("senza denominatore" in m for m in errori), errori)

    def test_conteggi_differenziali_non_interi_sono_rossi(self) -> None:
        for campo in (
            "righe_cambiate_eseguibili",
            "coperte",
            "scoperte",
            "cambiate_non_eseguibili",
        ):
            with self.subTest(campo=campo):
                errori = self.errori_con(
                    lambda e, c=campo: e["misure"]["diagnostica_differenziale"].update(
                        {c: True}
                    )
                )
                self.assertTrue(
                    any(f"diagnostica_differenziale.{campo}" in m for m in errori),
                    errori,
                )

    def test_l_elenco_delle_scoperte_deve_contarle_tutte(self) -> None:
        errori = self.errori_con(
            lambda e: e["misure"]["diagnostica_differenziale"].update(
                righe_scoperte=e["misure"]["diagnostica_differenziale"][
                    "righe_scoperte"
                ][:-1]
            )
        )
        self.assertTrue(any("posizioni, `scoperte`" in m for m in errori), errori)

    def test_un_elenco_di_scoperte_con_ripetizioni_e_rosso(self) -> None:
        """Una riga contata due volte gonfia l'elenco senza gonfiare il
        conteggio: e' il modo in cui i due smetterebbero di dire la stessa
        cosa."""

        def ripeti(e):
            elenco = e["misure"]["diagnostica_differenziale"]["righe_scoperte"]
            e["misure"]["diagnostica_differenziale"]["righe_scoperte"] = [
                elenco[0]
            ] + elenco[:-1]

        errori = self.errori_con(ripeti)
        self.assertTrue(any("ripete" in m for m in errori), errori)

    def test_una_campagna_di_fuzzing_a_zero_non_e_un_verde(self) -> None:
        casi = {
            "replay senza input": lambda e: e["misure"]["fuzz_replay"].update(input=0),
            "nessun target di replay": lambda e: e["misure"]["fuzz_replay"].update(
                target=0, target_totali=0
            ),
            "nessun target di smoke": lambda e: e["misure"]["fuzz_smoke"].update(
                target_eseguiti=0, target_totali=0
            ),
        }
        for nome, muta in casi.items():
            with self.subTest(caso=nome):
                errori = self.errori_con(muta)
                self.assertTrue(any("zero" in m for m in errori), errori)

    def test_un_conteggio_di_fuzzing_che_non_e_intero_e_rosso(self) -> None:
        errori = self.errori_con(
            lambda e: e["misure"]["fuzz_replay"].update(input="tanti")
        )
        self.assertTrue(any("intero non negativo" in m for m in errori), errori)

    def test_un_crash_o_un_finding_contraddicono_l_esito(self) -> None:
        casi = {
            "crash": lambda e: e["misure"]["fuzz_replay"].update(crash=1),
            "finding": lambda e: e["misure"]["fuzz_smoke"].update(finding=1),
            "quarantena": lambda e: e["misure"]["fuzz_smoke"].update(quarantena=1),
            "smoke parziale": lambda e: e["misure"]["fuzz_smoke"].update(
                target_eseguiti=12
            ),
        }
        for nome, muta in casi.items():
            with self.subTest(caso=nome):
                self.assertNotEqual(self.errori_con(muta), [])

    # --- l'elenco dei passi, e il manifest che ne discende ----------------

    def test_un_manifest_ridotto_e_rosso(self) -> None:
        """Il difetto che l'elenco esiste per chiudere.

        Il manifest poteva essere ridotto a due file mentre la riconciliazione
        continuava a dichiarare 57/57: i contatori dicevano quanti, e nessun
        artefatto diceva quali.
        """
        def muta(evidenza):
            manifest = {
                nome: digest
                for nome, digest in evidenza["artefatti"]["manifest"].items()
                if nome in (gate.RISULTATO_DELLA_CORSA, "check_errori_redatti.log")
            }
            evidenza["artefatti"]["manifest"] = manifest
            evidenza["artefatti"]["file"] = len(manifest)
            evidenza["artefatti"]["digest_manifest"] = gate.digest_del_manifest(manifest)

        errori = self.errori_con(muta)
        self.assertTrue(any("mancano" in m for m in errori), errori)

    def test_un_artefatto_estraneo_al_manifest_e_rosso(self) -> None:
        """L'insieme e' chiuso: i log dei passi piu' quattro artefatti noti."""
        def muta(evidenza):
            manifest = evidenza["artefatti"]["manifest"]
            manifest["passo_mai_esistito.log"] = "0" * 64
            evidenza["artefatti"]["file"] = len(manifest)
            evidenza["artefatti"]["digest_manifest"] = gate.digest_del_manifest(manifest)

        errori = self.errori_con(muta)
        self.assertTrue(any("non appartengono" in m for m in errori), errori)

    def test_senza_elenco_dei_passi_e_rosso(self) -> None:
        errori = self.errori_con(lambda e: e["riconciliazione"].pop("passi"))
        self.assertTrue(any("assente o vuoto" in m for m in errori), errori)

    def test_i_conteggi_si_derivano_dall_elenco(self) -> None:
        """Un conteggio dichiarato diverso da quello che l'elenco produce."""
        errori = self.errori_con(lambda e: e["riconciliazione"].update(verdi=56))
        self.assertTrue(
            any("l'elenco dei passi ne conta" in m for m in errori), errori
        )

    def test_log_nullo_solo_per_i_due_passi_in_linea(self) -> None:
        casi = {
            "un passo con log dichiara null": lambda e: e["riconciliazione"]["passi"][
                0
            ].update(log=None),
            "un passo in linea dichiara un log": lambda e: [
                v for v in e["riconciliazione"]["passi"] if v["id"] == "albero_invariato"
            ][0].update(log="albero_invariato.log"),
        }
        for nome, muta in casi.items():
            with self.subTest(caso=nome):
                errori = self.errori_con(muta)
                self.assertTrue(any("atteso" in m for m in errori), errori)

    def test_un_log_che_non_segue_l_identita_e_rosso(self) -> None:
        errori = self.errori_con(
            lambda e: e["riconciliazione"]["passi"][0].update(log="un_altro.log")
        )
        self.assertTrue(any("atteso" in m for m in errori), errori)

    def test_un_passo_non_verde_contraddice_l_esito(self) -> None:
        for esito in ("omesso", "rosso", "saltato"):
            with self.subTest(esito=esito):
                errori = self.errori_con(
                    lambda e, x=esito: e["riconciliazione"]["passi"][3].update(esito=x)
                )
                self.assertTrue(any("ha esito" in m for m in errori), errori)

    def test_i_due_passi_in_linea_devono_esserci(self) -> None:
        """Senza, la corsa non ha verificato di aver misurato un albero solo."""
        for identita in sorted(gate.passi_del_checkpoint()[1]):
            with self.subTest(passo=identita):
                errori = self.errori_con(
                    lambda e, i=identita: e["riconciliazione"].__setitem__(
                        "passi",
                        [v for v in e["riconciliazione"]["passi"] if v["id"] != i],
                    )
                )
                self.assertTrue(any(identita in m for m in errori), errori)

    # --- l'insieme dei passi e' quello del registro canonico -------------
    #
    # Il verificatore controllava perfettamente cio' che l'evidenza dichiarava,
    # e non sapeva che `fmt` dovesse esistere: togliere il gate, la sua voce, il
    # suo log, e aggiornare contatori, numero di artefatti e digest, passava.
    # Una rimozione **coordinata** e' esattamente cio' che un gate coerente con
    # se stesso non puo' vedere.

    @staticmethod
    def senza_passo(evidenza, identita):
        conti = evidenza["riconciliazione"]
        conti["passi"] = [v for v in conti["passi"] if v["id"] != identita]
        for campo in ("identificatori_distinti", "eseguiti", "verdi"):
            conti[campo] = len(conti["passi"])
        manifest = {
            nome: digest
            for nome, digest in evidenza["artefatti"]["manifest"].items()
            if nome != f"{identita}.log"
        }
        evidenza["artefatti"]["manifest"] = manifest
        evidenza["artefatti"]["file"] = len(manifest)
        evidenza["artefatti"]["digest_manifest"] = gate.digest_del_manifest(manifest)

    def test_una_rimozione_coordinata_di_un_passo_e_rossa(self) -> None:
        errori = self.errori_con(lambda e: self.senza_passo(e, "fmt"))
        self.assertTrue(
            any("dichiarati dal registro dei passi" in m for m in errori), errori
        )

    def test_una_rinomina_coordinata_e_rossa(self) -> None:
        """Il totale resta 57: un passo esce e uno entra."""
        def muta(evidenza):
            for voce in evidenza["riconciliazione"]["passi"]:
                if voce["id"] == "fmt":
                    voce["id"] = "formattazione"
                    voce["log"] = "formattazione.log"
            manifest = evidenza["artefatti"]["manifest"]
            manifest["formattazione.log"] = manifest.pop("fmt.log")
            evidenza["artefatti"]["digest_manifest"] = gate.digest_del_manifest(manifest)

        errori = self.errori_con(muta)
        self.assertTrue(any("['fmt']" in m for m in errori), errori)
        self.assertTrue(any("formattazione" in m for m in errori), errori)

    def test_un_identificatore_aggiunto_e_rosso(self) -> None:
        def muta(evidenza):
            conti = evidenza["riconciliazione"]
            conti["passi"].append(
                {"id": "passo_nuovo", "esito": "verde", "log": "passo_nuovo.log"}
            )
            for campo in ("identificatori_distinti", "eseguiti", "verdi"):
                conti[campo] = len(conti["passi"])
            manifest = evidenza["artefatti"]["manifest"]
            manifest["passo_nuovo.log"] = "0" * 64
            evidenza["artefatti"]["file"] = len(manifest)
            evidenza["artefatti"]["digest_manifest"] = gate.digest_del_manifest(manifest)

        errori = self.errori_con(muta)
        self.assertTrue(any("non sono nel registro" in m for m in errori), errori)

    def test_un_campo_in_piu_nella_voce_e_rosso(self) -> None:
        """`{id, esito, log, extra}` passava: il sottoschema e' chiuso."""
        errori = self.errori_con(
            lambda e: e["riconciliazione"]["passi"][0].update(extra=1)
        )
        self.assertTrue(any("attesi esattamente" in m for m in errori), errori)

    def test_due_passi_scambiati_sono_rossi(self) -> None:
        """L'ordine e' dichiarato, e presenza ed estranei non lo vedono.

        Scambiare due passi lascia l'insieme identico: e' l'ordine a rendere
        leggibile una corsa — `sonde_checkpoint` per primo perche' se il gate
        che misura e' rotto tutto cio' che segue e' una misura di cui non si sa
        niente, la copertura dopo il fuzzing perche' legge il profdata che
        quello ha prodotto.
        """
        def muta(evidenza):
            passi = evidenza["riconciliazione"]["passi"]
            passi[0], passi[1] = passi[1], passi[0]

        errori = self.errori_con(muta)
        self.assertTrue(any("ordine divergente" in m for m in errori), errori)
        self.assertTrue(any("posizione 1" in m for m in errori), errori)

    def test_l_ordine_divergente_nomina_la_prima_posizione(self) -> None:
        """Ci si ferma alla prima: uno spostamento le stamperebbe quasi tutte."""
        def muta(evidenza):
            passi = evidenza["riconciliazione"]["passi"]
            passi[3], passi[4] = passi[4], passi[3]

        errori = self.errori_con(muta)
        divergenze = [m for m in errori if "ordine divergente" in m]
        self.assertEqual(len(divergenze), 1, errori)
        self.assertIn("posizione 4", divergenze[0])

    def test_l_ordine_reale_coincide_col_registro(self) -> None:
        dichiarati, _ = gate.passi_del_checkpoint()
        passi = self.evidenza()["riconciliazione"]["passi"]
        self.assertEqual([v["id"] for v in passi], list(dichiarati))

    def test_l_insieme_reale_coincide_col_registro(self) -> None:
        """La controprova positiva, e la sola cosa che lega i due elenchi."""
        dichiarati, senza_log = gate.passi_del_checkpoint()
        passi = self.evidenza()["riconciliazione"]["passi"]
        self.assertEqual([v["id"] for v in passi], list(dichiarati))
        self.assertEqual({v["id"] for v in passi if v["log"] is None}, set(senza_log))

    def test_l_elenco_reale_copre_il_manifest(self) -> None:
        """La controprova positiva, sui numeri di questa corsa."""
        evidenza = self.evidenza()
        passi, errori = gate._passi_dichiarati(evidenza)
        self.assertEqual(errori, [], errori)
        self.assertEqual(len(passi), 57)
        self.assertEqual(gate._manifest_legato_ai_passi(evidenza, passi), [])
        con_log = {v["log"] for v in passi if v["log"]}
        self.assertEqual(len(con_log), 55)
        self.assertEqual(
            set(evidenza["artefatti"]["manifest"]),
            con_log | set(gate.ARTEFATTI_NON_DI_PASSO),
        )

    # --- artefatti ---------------------------------------------------------

    def test_un_manifest_ritoccato_non_produce_il_proprio_digest(self) -> None:
        """Il digest si **ricalcola** dal manifest, e la forma vive nel gate.

        La prosa dell'evidenza dice «percorsi ordinati piu' sha256 del
        contenuto» e non fissa i delimitatori: non determina un valore. Un
        digest che nessuno puo' ricalcolare e' una stringa.
        """
        errori = self.errori_con(
            lambda e: e["artefatti"]["manifest"].update({"finto.log": "0" * 64})
        )
        self.assertTrue(any("digest_manifest" in m for m in errori), errori)

    def test_un_manifest_di_valori_che_non_sono_digest_e_rosso(self) -> None:
        """Il digest aggregato si ricalcola anche su un manifest di «x».

        Legava l'insieme senza dire che le voci fossero impronte: il manifest
        improntava la propria forma invece del contenuto della corsa.
        """
        def muta(evidenza):
            evidenza["artefatti"]["manifest"] = {
                percorso: "x" for percorso in evidenza["artefatti"]["manifest"]
            }
            evidenza["artefatti"]["digest_manifest"] = gate.digest_del_manifest(
                evidenza["artefatti"]["manifest"]
            )

        errori = self.errori_con(muta)
        self.assertTrue(any("-> sha256" in m for m in errori), errori)

    def test_una_voce_di_manifest_che_esce_dalla_corsa_e_rossa(self) -> None:
        """`../fuori.log` nomina un file che la corsa non ha prodotto.

        Il digest si ricalcola su qualunque insieme di stringhe, quindi restava
        coerente con se stesso: e' il manifest a dover nominare solo cio' che
        sta dentro la directory di corsa.
        """
        def muta(evidenza):
            manifest = evidenza["artefatti"]["manifest"]
            manifest["../fuori.log"] = "0" * 64
            evidenza["artefatti"]["file"] = len(manifest)
            evidenza["artefatti"]["digest_manifest"] = gate.digest_del_manifest(manifest)

        errori = self.errori_con(muta)
        self.assertTrue(any("canonico" in m for m in errori), errori)

    def test_due_voci_che_normalizzano_allo_stesso_percorso_sono_rosse(self) -> None:
        """Su un volume che non distingue le maiuscole sono un file solo."""
        def muta(evidenza):
            manifest = evidenza["artefatti"]["manifest"]
            manifest["FMT.log"] = manifest["fmt.log"]
            evidenza["artefatti"]["file"] = len(manifest)
            evidenza["artefatti"]["digest_manifest"] = gate.digest_del_manifest(manifest)

        errori = self.errori_con(muta)
        self.assertTrue(any("normalizzano" in m for m in errori), errori)

    def test_un_manifest_senza_il_risultato_della_corsa_e_rosso(self) -> None:
        """E' l'ultimo file che il checkpoint scrive."""
        def muta(evidenza):
            manifest = evidenza["artefatti"]["manifest"]
            manifest.pop(gate.RISULTATO_DELLA_CORSA)
            evidenza["artefatti"]["file"] = len(manifest)
            evidenza["artefatti"]["digest_manifest"] = gate.digest_del_manifest(manifest)

        errori = self.errori_con(muta)
        self.assertTrue(
            any(gate.RISULTATO_DELLA_CORSA in m for m in errori), errori
        )

    def test_il_conteggio_dei_file_segue_il_manifest(self) -> None:
        errori = self.errori_con(lambda e: e["artefatti"].update(file=1))
        self.assertTrue(any("artefatti.file" in m for m in errori), errori)

    def test_il_digest_reale_si_ricalcola(self) -> None:
        evidenza = self.evidenza()
        self.assertEqual(
            gate.digest_del_manifest(evidenza["artefatti"]["manifest"]),
            evidenza["artefatti"]["digest_manifest"],
        )


class SondeSolaEvidenza(unittest.TestCase):
    """L'albero di lavoro conserva la sola evidenza corrente."""

    def stato(self) -> dict:
        return json.loads(gate.STATO_CORRENTE.read_text(encoding="utf-8"))

    def test_l_albero_reale_ne_ha_una_sola(self) -> None:
        self.assertEqual(gate._sola_evidenza_corrente(self.stato()), [])
        presenti = sorted(p.name for p in gate.DIRECTORY_EVIDENZE.glob("*.json"))
        self.assertEqual(len(presenti), 1, presenti)

    def test_un_evidenza_precedente_rimasta_e_rossa(self) -> None:
        """Una corsa vecchia nell'albero invita a un confronto fra corse.

        La directory viene sostituita con una temporanea **fuori dal
        repository**: crearne una qui dentro lascerebbe un file non tracciato, e
        l'impronta dell'albero lo vedrebbe mentre il checkpoint la misura.
        """
        stato = self.stato()
        attesa = pathlib.Path(stato["ultima_misura"]["evidenza"]).name
        with tempfile.TemporaryDirectory() as temporanea:
            radice = pathlib.Path(temporanea)
            (radice / attesa).write_text("{}", encoding="utf-8")
            (radice / "checkpoint-0000000.json").write_text("{}", encoding="utf-8")
            with mock.patch.object(gate, "DIRECTORY_EVIDENZE", radice):
                errori = gate._sola_evidenza_corrente(stato)
        self.assertTrue(any("checkpoint-0000000.json" in m for m in errori), errori)

    def test_un_file_che_non_e_json_resta_visibile(self) -> None:
        """`glob("*.json")` era una whitelist di estensione travestita.

        `checkpoint-old.json.bak`, una sottodirectory o un `.orig` lasciato da
        un merge non comparivano: «contiene la corrente e nient'altro» era vero
        soltanto dei `.json`.
        """
        casi = ("checkpoint-old.json.bak", "checkpoint-388afcb.json.orig")
        for nome in casi:
            with self.subTest(voce=nome):
                stato = self.stato()
                attesa = pathlib.Path(stato["ultima_misura"]["evidenza"]).name
                with tempfile.TemporaryDirectory() as temporanea:
                    radice = pathlib.Path(temporanea)
                    (radice / attesa).write_text("{}", encoding="utf-8")
                    (radice / nome).write_text("{}", encoding="utf-8")
                    with mock.patch.object(gate, "DIRECTORY_EVIDENZE", radice):
                        errori = gate._sola_evidenza_corrente(stato)
                self.assertTrue(any(nome in m for m in errori), errori)

    def test_una_sottodirectory_resta_visibile(self) -> None:
        stato = self.stato()
        attesa = pathlib.Path(stato["ultima_misura"]["evidenza"]).name
        with tempfile.TemporaryDirectory() as temporanea:
            radice = pathlib.Path(temporanea)
            (radice / attesa).write_text("{}", encoding="utf-8")
            (radice / "archivio").mkdir()
            with mock.patch.object(gate, "DIRECTORY_EVIDENZE", radice):
                errori = gate._sola_evidenza_corrente(stato)
        self.assertTrue(any("archivio" in m for m in errori), errori)

    def test_un_evidenza_che_e_un_link_e_rossa(self) -> None:
        """Un link punta a un contenuto che l'albero non registra."""
        stato = self.stato()
        attesa = pathlib.Path(stato["ultima_misura"]["evidenza"]).name
        with tempfile.TemporaryDirectory() as temporanea:
            radice = pathlib.Path(temporanea)
            bersaglio = radice.parent / "bersaglio.json"
            bersaglio.write_text("{}", encoding="utf-8")
            try:
                (radice / attesa).symlink_to(bersaglio)
            except (OSError, NotImplementedError) as impedimento:
                self.skipTest(f"symlink non creabili qui: {impedimento}")
            with mock.patch.object(gate, "DIRECTORY_EVIDENZE", radice):
                errori = gate._sola_evidenza_corrente(stato)
            bersaglio.unlink()
        self.assertTrue(any("e' un link" in m for m in errori), errori)

    def test_la_sola_corrente_passa(self) -> None:
        stato = self.stato()
        attesa = pathlib.Path(stato["ultima_misura"]["evidenza"]).name
        with tempfile.TemporaryDirectory() as temporanea:
            radice = pathlib.Path(temporanea)
            (radice / attesa).write_text("{}", encoding="utf-8")
            with mock.patch.object(gate, "DIRECTORY_EVIDENZE", radice):
                self.assertEqual(gate._sola_evidenza_corrente(stato), [])


class SondePercorsiCanonici(unittest.TestCase):
    """Un percorso si scrive in un modo solo, e non esce dal proprio albero.

    Un percorso era accettato come «stringa non vuota», e due percorsi si
    confrontavano sul nome del file: `assurance/evidence/../evidence/x.json`
    passava, e una voce di manifest poteva uscire dalla directory di corsa con
    `../fuori.log` mentre il digest restava coerente con se stesso — si
    ricalcola su qualunque insieme di stringhe.
    """

    def stato(self) -> dict:
        return json.loads(gate.STATO_CORRENTE.read_text(encoding="utf-8"))

    def test_riconosce_i_relativi_canonici(self) -> None:
        for buono in ("fmt.log", "a/b.log", "assurance/evidence/x.json"):
            with self.subTest(percorso=buono):
                self.assertEqual(gate.percorso_canonico(buono), buono)

    def test_rifiuta_cio_che_non_e_canonico(self) -> None:
        storti = [
            "",
            ".",
            "..",
            "/assoluto",
            "a//b",
            "a/./b",
            "a/../b",
            "assurance/evidence/../evidence/x.json",
            "C:/x",
            "a" + chr(92) + "b",
            "a" + chr(0) + "b",
            None,
            7,
            [],
        ]
        for storto in storti:
            with self.subTest(percorso=storto):
                self.assertIsNone(gate.percorso_canonico(storto))

    def test_un_evidenza_nominata_in_due_modi_e_rossa(self) -> None:
        stato = self.stato()
        stato["ultima_misura"]["evidenza"] = (
            "assurance/evidence/../evidence/"
            + pathlib.Path(stato["ultima_misura"]["evidenza"]).name
        )
        errori = gate.validate_stato_corrente(stato)
        self.assertTrue(any("non canonico" in m for m in errori), errori)

    def test_un_evidenza_fuori_dalla_propria_cartella_e_rossa(self) -> None:
        stato = self.stato()
        stato["ultima_misura"]["evidenza"] = "assurance/checkpoint-388afcb.json"
        errori = gate.validate_stato_corrente(stato)
        self.assertTrue(
            any(gate.CARTELLA_DELLE_EVIDENZE in m for m in errori), errori
        )


class SondeRegistroReale(unittest.TestCase):
    def registro(self) -> dict:
        return json.loads(gate.REGISTRO.read_text(encoding="utf-8"))

    def test_il_registro_reale_e_ben_formato(self) -> None:
        self.assertEqual(gate.struttura(self.registro()), [])

    def test_ogni_bloccante_dichiara_la_propria_condizione_di_chiusura(self) -> None:
        senza = [v["id"] for v in gate.debito(self.registro()) if not v.get("manca")]
        self.assertEqual(senza, [])


class SondeProtocolloCli(unittest.TestCase):
    """Il protocollo CLI resta verificato **nel merito**, non solo nominato."""

    def documento_valido(self) -> dict:
        return json.loads(gate.CLI_PROTOCOL_V1.read_text(encoding="utf-8"))

    def test_il_manifesto_reale_e_valido(self) -> None:
        self.assertEqual(gate.validate_cli_protocol_v1(self.documento_valido()), [])

    def test_una_busta_mancante_e_rossa(self) -> None:
        documento = self.documento_valido()
        documento["envelopes"].pop("convert")
        errori = gate.validate_cli_protocol_v1(documento)
        self.assertTrue(any("sei buste" in e for e in errori), errori)

    def test_una_garanzia_semver_sull_api_rust_e_rossa(self) -> None:
        """L'API Rust resta interna: prometterla sarebbe una rottura futura."""
        documento = self.documento_valido()
        documento["rust_api"]["semver_guarantee"] = True
        errori = gate.validate_cli_protocol_v1(documento)
        self.assertTrue(any("API Rust" in e for e in errori), errori)


if __name__ == "__main__":
    unittest.main()
