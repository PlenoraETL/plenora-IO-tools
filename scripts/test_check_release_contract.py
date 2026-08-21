"""Sonde del contratto corrente.

Il gate ha due doveri opposti, e vanno provati entrambi: accorgersi che un
invariante si dichiari `verified` senza una prova che esista, e non impedire
che un blocco dichiari la propria.

La distinzione che queste sonde fissano e' la piu' facile da perdere:
`release_blocking` **puo'** avere una prova. Sono due casi diversi — un
meccanismo di verifica che esiste e oggi fallisce, e un meccanismo che non
esiste — e confonderli farebbe sparire il primo, che e' quello su cui si
lavora.
"""

from __future__ import annotations

import unittest

from scripts import check_release_contract as gate


def voce(**extra):
    base = {
        "id": "wire.qualcosa",
        "superficie": "una superficie",
        "invariante": "un invariante scritto",
        "prova": {"tipo": "gate", "comando": "scripts/check_release_contract.py"},
        "stato": "verified",
    }
    base.update(extra)
    return base


def documento(*voci):
    return {"schema_version": 1, "invarianti": list(voci)}


class SondeRegistro(unittest.TestCase):
    def test_un_registro_coerente_passa(self) -> None:
        """La controprova positiva: senza, «sempre rosso» sarebbe una difesa."""
        self.assertEqual(gate.verifica_registro(documento(voce())), [])

    # --- primo dovere: `verified` senza verifica ---------------------------

    def test_verified_senza_prova_e_rosso(self) -> None:
        errori = gate.verifica_registro(documento(voce(prova=None)))
        self.assertTrue(any("senza prova" in e for e in errori), errori)

    def test_una_prova_che_non_esiste_e_rossa(self) -> None:
        """Una prova che sopravvive al proprio strumento verifica un
        invariante che nessuno controlla."""
        errori = gate.verifica_registro(
            documento(voce(prova={"tipo": "gate", "comando": "scripts/mai_esistito.py"}))
        )
        self.assertTrue(any("non esiste" in e for e in errori), errori)

    def test_un_artefatto_che_non_esiste_e_rosso(self) -> None:
        errori = gate.verifica_registro(
            documento(
                voce(
                    prova={
                        "tipo": "gate",
                        "comando": "scripts/check_release_contract.py",
                        "artefatto": "release/mai-esistito.json",
                    }
                )
            )
        )
        self.assertTrue(any("non esiste" in e for e in errori), errori)

    def test_un_tipo_di_prova_inventato_e_rosso(self) -> None:
        errori = gate.verifica_registro(
            documento(voce(prova={"tipo": "intuizione", "comando": "scripts/check_release_contract.py"}))
        )
        self.assertTrue(any("tipo di prova" in e for e in errori), errori)

    def test_verified_senza_invariante_scritto_e_rosso(self) -> None:
        errori = gate.verifica_registro(documento(voce(invariante="")))
        self.assertTrue(any("senza invariante" in e for e in errori), errori)

    # --- secondo dovere: il blocco deve dire che cosa manca ----------------

    def test_release_blocking_senza_manca_e_rosso(self) -> None:
        errori = gate.verifica_registro(
            documento(voce(stato="release_blocking", prova=None))
        )
        self.assertTrue(any("senza campo `manca`" in e for e in errori), errori)

    def test_release_blocking_puo_avere_una_prova(self) -> None:
        """La distinzione decisiva.

        ASSURANCE-N1 ha un gate che lo verifica **ed e' rosso**: il meccanismo
        esiste, l'invariante non e' ancora soddisfatto. Vietare la prova ai
        bloccanti confonderebbe questo caso con quello di una lacuna che non ha
        alcuno strumento — e i due si chiudono in modi diversi.
        """
        errori = gate.verifica_registro(
            documento(voce(stato="release_blocking", manca="43 gruppi aperti"))
        )
        self.assertEqual(errori, [], errori)

    def test_release_blocking_senza_prova_va_bene(self) -> None:
        errori = gate.verifica_registro(
            documento(voce(stato="release_blocking", prova=None, manca="nessuno strumento"))
        )
        self.assertEqual(errori, [], errori)

    # --- struttura ---------------------------------------------------------

    def test_uno_stato_inventato_e_rosso(self) -> None:
        errori = gate.verifica_registro(documento(voce(stato="quasi")))
        self.assertTrue(any("non ammesso" in e for e in errori), errori)

    def test_un_identificatore_duplicato_e_rosso(self) -> None:
        errori = gate.verifica_registro(documento(voce(), voce()))
        self.assertTrue(any("duplicata" in e for e in errori), errori)

    def test_campi_mancanti_sono_rossi(self) -> None:
        parziale = voce()
        del parziale["superficie"]
        errori = gate.verifica_registro(documento(parziale))
        self.assertTrue(any("campi mancanti" in e for e in errori), errori)

    def test_il_debito_conta_solo_i_bloccanti(self) -> None:
        d = documento(
            voce(),
            voce(id="fuzz.lacuna", stato="release_blocking", prova=None, manca="niente strumento"),
        )
        self.assertEqual([v["id"] for v in gate.debito(d)], ["fuzz.lacuna"])


class SondeProtocolloCli(unittest.TestCase):
    """Il protocollo CLI resta verificato **nel merito**, non solo nominato."""

    def documento_valido(self) -> dict:
        import json
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
