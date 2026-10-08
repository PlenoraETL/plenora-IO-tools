"""La busta d'errore e i modelli rifiutano cio' che il protocollo non dichiara.

Prima l'SDK controllava soltanto la **presenza** delle chiavi, e dove un
valore mancava o era di un tipo inatteso ripiegava su una risposta: un
`retry.kind` assente o sconosciuto diceva «ritentabile», un `remote_effect`
`null` diceva «nessun effetto remoto da temere», un `details` stringa
sollevava `AttributeError` invece di un errore di protocollo. Sono tutte
failure aperte, e qui ciascuna e' riprodotta e chiusa.

Le sonde non eseguono un binario: costruiscono la busta e la danno a
`failure_from_envelope`, cosi' girano anche dove gli script con shebang non si
eseguono.
"""

from __future__ import annotations

import unittest

from plenora_io import (
    CommandFailed,
    ErrorEnvelope,
    Field,
    LayerSummary,
    ProtocolError,
)
from plenora_io import errors
from plenora_io.errors import failure_from_envelope
from plenora_io.models import FidelityReason, LossReport


def errore_sano(**campi):
    errore = {
        "code": "X",
        "category": "internal",
        "phase": "read",
        "remote_effect": "none",
        "retry": {"kind": "never"},
        "message": "un messaggio qualunque",
    }
    errore.update(campi)
    return errore


def solleva(**campi) -> CommandFailed:
    return failure_from_envelope(
        {"status": "error", "error": errore_sano(**campi)}, 5, ["x"]
    )


class IlRitentativoEChiuso(unittest.TestCase):
    """`retry.kind` e' un vocabolario chiuso: fuori, e' `ProtocolError`."""

    def test_kind_assente_null_o_sconosciuto_non_e_ritentabile(self) -> None:
        # Il difetto originale: `retry.get("kind") != "never"` era vero per
        # tutti e tre, e l'SDK diceva «ritentabile» a una busta che non lo
        # diceva affatto.
        for retry in ({}, {"kind": None}, {"kind": "riprova_pure"}, {"kind": 3}):
            with self.subTest(retry=retry):
                with self.assertRaises(ProtocolError):
                    solleva(retry=retry)

    def test_retry_non_oggetto_e_protocollo_non_attributeerror(self) -> None:
        for retry in (None, "never", ["never"], 0):
            with self.subTest(retry=retry):
                with self.assertRaises(ProtocolError):
                    solleva(retry=retry)

    def test_after_pretende_un_ritardo_intero_nei_limiti(self) -> None:
        for retry in (
            {"kind": "after"},
            {"kind": "after", "delay_ms": None},
            {"kind": "after", "delay_ms": "10"},
            {"kind": "after", "delay_ms": True},
            {"kind": "after", "delay_ms": 1.5},
            {"kind": "after", "delay_ms": -1},
            {"kind": "after", "delay_ms": errors.RITARDO_MASSIMO_MS + 1},
        ):
            with self.subTest(retry=retry):
                with self.assertRaises(ProtocolError):
                    solleva(retry=retry)

    def test_chiavi_in_piu_sono_rifiutate(self) -> None:
        # Lo schema e' `additionalProperties: false`: un `delay_ms` su `safe`
        # direbbe un ritardo che il tipo non promette.
        for retry in (
            {"kind": "safe", "delay_ms": 5},
            {"kind": "never", "altro": 1},
            {"kind": "after", "delay_ms": 5, "altro": 1},
        ):
            with self.subTest(retry=retry):
                with self.assertRaises(ProtocolError):
                    solleva(retry=retry)

    def test_ogni_tipo_valido_passa_con_la_sua_risposta(self) -> None:
        # La controprova positiva: senza, «sempre rosso» sarebbe una difesa.
        attesi = {
            "never": False,
            "quarantine": False,
            "safe": True,
            "requires_idempotency_key": True,
            "requires_recovery": True,
        }
        for tipo, ritentabile in attesi.items():
            with self.subTest(tipo=tipo):
                errore = solleva(retry={"kind": tipo})
                self.assertIs(errore.retryable, ritentabile)
                self.assertIsNone(errore.retry_after_ms)
        dopo = solleva(retry={"kind": "after", "delay_ms": 0})
        self.assertTrue(dopo.retryable)
        self.assertEqual(dopo.retry_after_ms, 0)

    def test_il_vocabolario_e_quello_di_error_v1(self) -> None:
        self.assertEqual(
            set(errors.TIPI_DI_RITENTATIVO),
            {
                "never",
                "quarantine",
                "safe",
                "requires_idempotency_key",
                "requires_recovery",
                "after",
            },
        )
        self.assertEqual(
            set(errors.EFFETTI_REMOTI),
            {"none", "rolled_back", "partial", "committed", "unknown"},
        )


class LEffettoRemotoEChiuso(unittest.TestCase):
    """`remote_effect` fuori vocabolario non diventa «ritentare e' sicuro»."""

    def test_null_assente_o_sconosciuto_e_protocollo(self) -> None:
        for effetto in (None, "", "maybe", 0, ["none"]):
            with self.subTest(effetto=effetto):
                with self.assertRaises(ProtocolError):
                    solleva(remote_effect=effetto)

    def test_ogni_effetto_valido_ha_la_sua_decisione(self) -> None:
        for effetto, cautela in (
            ("none", False),
            ("rolled_back", False),
            ("partial", True),
            ("committed", True),
            ("unknown", True),
        ):
            with self.subTest(effetto=effetto):
                retry = {"kind": "never"}
                errore = solleva(remote_effect=effetto, retry=retry)
                self.assertIs(errore.must_assume_remote_committed, cautela)

    def test_effetto_ignoto_con_ritentativo_sicuro_e_contraddittorio(self) -> None:
        # Lo schema `error-v1`, nel suo `allOf`: con `unknown` solo `never`,
        # `quarantine` o `requires_recovery`.
        for retry in (
            {"kind": "safe"},
            {"kind": "requires_idempotency_key"},
            {"kind": "after", "delay_ms": 10},
        ):
            with self.subTest(retry=retry):
                with self.assertRaises(ProtocolError):
                    solleva(remote_effect="unknown", retry=retry)
        for tipo in ("never", "quarantine", "requires_recovery"):
            with self.subTest(tipo=tipo):
                solleva(remote_effect="unknown", retry={"kind": tipo})


class ICampiDellaBusta(unittest.TestCase):
    def test_i_campi_testuali_sono_stringhe_non_vuote(self) -> None:
        for campo in ("code", "category", "phase", "message"):
            for valore in (None, 7, "", ["x"]):
                with self.subTest(campo=campo, valore=valore):
                    with self.assertRaises(ProtocolError):
                        solleva(**{campo: valore})

    def test_details_null_o_non_oggetto_e_protocollo(self) -> None:
        # `null` diventava `{}` in silenzio, una stringa `AttributeError`.
        for dettagli in (None, "x", [], 3):
            with self.subTest(dettagli=dettagli):
                with self.assertRaises(ProtocolError):
                    solleva(details=dettagli)

    def test_row_diagnostics_non_oggetto_e_protocollo(self) -> None:
        for diagnostica in (None, "x", []):
            with self.subTest(diagnostica=diagnostica):
                with self.assertRaises(ProtocolError):
                    solleva(details={"row_diagnostics": diagnostica})

    def test_details_valido_passa(self) -> None:
        self.assertIsNone(solleva(details={}).envelope.row_diagnostics)
        documento = {"contract": "plenora-row-diagnostics-v1"}
        errore = solleva(details={"row_diagnostics": documento})
        self.assertEqual(errore.envelope.row_diagnostics, documento)

    def test_la_busta_non_oggetto_e_protocollo(self) -> None:
        for documento in (None, [], "x"):
            with self.subTest(documento=documento):
                with self.assertRaises(ProtocolError):
                    ErrorEnvelope.from_json(documento)

    def test_anche_la_costruzione_diretta_valida(self) -> None:
        # La validazione sta in `__post_init__`: un adattatore che costruisce
        # la busta a mano non scavalca il vocabolario.
        with self.assertRaises(ProtocolError):
            ErrorEnvelope(
                code="X",
                category="io",
                phase="read",
                remote_effect="forse",
                retry={"kind": "never"},
                message="m",
            )

    def test_i_messaggi_non_ricopiano_il_valore(self) -> None:
        segreto = "valore-che-non-deve-uscire"
        for campi in (
            {"remote_effect": segreto},
            {"retry": {"kind": segreto}},
            {"phase": 12, "message": segreto},
        ):
            with self.subTest(campi=campi):
                with self.assertRaises(ProtocolError) as preso:
                    solleva(**campi)
                self.assertNotIn(segreto, str(preso.exception))


class IModelliControllanoITipi(unittest.TestCase):
    """`_pretendi` controllava la presenza delle chiavi, non il tipo."""

    def test_un_intero_null_stringa_o_booleano_e_rifiutato(self) -> None:
        sano = {"id": 0, "name": "a", "field_count": 1, "geometry_crs": "EPSG:4326"}
        LayerSummary.from_json(sano)
        for valore in (None, "1", True, 1.0):
            with self.subTest(valore=valore):
                with self.assertRaises(ProtocolError):
                    LayerSummary.from_json({**sano, "field_count": valore})

    def test_un_booleano_intero_e_rifiutato(self) -> None:
        sano = {"name": "a", "type": "Utf8", "nullable": True, "geometry": False}
        Field.from_json(sano)
        for valore in (None, 1, "true"):
            with self.subTest(valore=valore):
                with self.assertRaises(ProtocolError):
                    Field.from_json({**sano, "nullable": valore})

    def test_un_documento_non_oggetto_e_protocollo(self) -> None:
        for documento in (None, [], "x", 3):
            with self.subTest(documento=documento):
                with self.assertRaises(ProtocolError):
                    Field.from_json(documento)

    def test_gli_opzionali_presenti_non_ammettono_null(self) -> None:
        sano = {"code": "c", "detail": "d"}
        self.assertIsNone(FidelityReason.from_json(sano).field_index)
        FidelityReason.from_json({**sano, "field_index": 0, "layer_index": 0})
        for campo in FidelityReason.OPZIONALI:
            with self.subTest(campo=campo):
                with self.assertRaises(ProtocolError):
                    FidelityReason.from_json({**sano, campo: None})

    def test_gli_elementi_di_un_elenco_hanno_il_loro_tipo(self) -> None:
        omesse = {
            "categorie_omesse": 0,
            "ragioni_omesse": 0,
            "esempi_omessi": 0,
            "omesse_per_byte": 0,
        }
        sano = {
            "lossless": True,
            "counts": [],
            "esempi": [],
            "troncato": False,
            "omesse": omesse,
            "omesse_esatte": True,
        }
        LossReport.from_json(sano)
        for voce in (None, "x", 3):
            with self.subTest(voce=voce):
                with self.assertRaises(ProtocolError):
                    LossReport.from_json({**sano, "counts": [voce]})


if __name__ == "__main__":
    unittest.main()
