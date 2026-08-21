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

import json
import unittest
from unittest import mock

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

    def esegui_con(self, testo: str, *voci):
        finto = mock.Mock(returncode=0, stdout=testo, stderr="")
        with mock.patch.object(gate.subprocess, "run", return_value=finto) as corsa:
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
        self.assertTrue(any("Un silenzio non e' un verde" in e for e in errori), errori)

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
