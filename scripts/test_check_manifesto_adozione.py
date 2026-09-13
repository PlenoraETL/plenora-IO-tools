"""Le controprove del gate del manifesto di adozione.

Il gate ha una parte che vale piu' delle altre: il validatore di forma, scritto
qui invece di importare `jsonschema`. Un validatore fatto in casa che **salti**
i costrutti che non conosce direbbe verde su cio' che non ha guardato, ed e' il
modo esatto in cui un gate del genere si rompe senza farsi vedere. Le sonde
costruiscono percio' i documenti storti uno per uno.
"""

from __future__ import annotations

import hashlib
import json
import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import check_manifesto_adozione as gate  # noqa: E402

CONTRATTI = gate.RADICE / ".plenora-contracts"
SCHEMA = json.loads(
    (CONTRATTI / "schemas" / "adoption-manifest-v4.schema.json").read_text("utf-8")
)


def manifesto_minimo() -> dict:
    return {
        "schema_version": 4,
        "component": "plenora-io-tools",
        "contracts_source": {
            "repository": "https://github.com/PlenoraETL/plenora-contracts.git",
            "revision": "0" * 40,
        },
        "profile": "plenora-io-tools-profile-v1",
        "artifacts": [
            {
                "name": "plenora-io",
                "surface": "cli",
                "version": "4.0.0",
                "digest": "sha256:" + "a" * 64,
                "verification": ["python3 scripts/check_public_contracts.py"],
            }
        ],
        "contracts": [
            {
                "id": "plenora-cli-v2",
                "status": "conforming",
                "verification": ["python3 scripts/check_buste_v2.py"],
            }
        ],
        "deviations": [],
    }


class IlValidatoreDiForma(unittest.TestCase):
    def errori(self, documento: dict) -> list[str]:
        return gate.valida_forma(documento, SCHEMA)

    def test_il_minimo_e_valido(self) -> None:
        self.assertEqual(self.errori(manifesto_minimo()), [])

    def test_un_campo_obbligatorio_mancante(self) -> None:
        d = manifesto_minimo()
        del d["deviations"]
        self.assertTrue(any("deviations" in e for e in self.errori(d)))

    def test_una_chiave_non_prevista(self) -> None:
        d = manifesto_minimo()
        d["note"] = "una spiegazione"
        self.assertTrue(any("non previste" in e for e in self.errori(d)))

    def test_un_digest_di_forma_sbagliata(self) -> None:
        for storto in ("sha256:" + "a" * 63, "a" * 64, "SHA256:" + "a" * 64):
            with self.subTest(digest=storto):
                d = manifesto_minimo()
                d["artifacts"][0]["digest"] = storto
                self.assertTrue(any("digest" in e for e in self.errori(d)), storto)

    def test_una_superficie_fuori_dall_enum(self) -> None:
        d = manifesto_minimo()
        d["artifacts"][0]["surface"] = "grpc"
        self.assertTrue(any("surface" in e for e in self.errori(d)))

    def test_lo_schema_version_e_una_costante(self) -> None:
        d = manifesto_minimo()
        d["schema_version"] = 3
        self.assertTrue(any("schema_version" in e for e in self.errori(d)))

    def test_un_contratto_conforme_senza_verifica(self) -> None:
        """`conforming` senza `verification` e' un'affermazione senza prova.

        Lo schema la vieta con un `if/then`, ed e' il costrutto che un
        validatore ingenuo salterebbe: e' la sonda che dice se questo gate
        guarda davvero.
        """
        d = manifesto_minimo()
        del d["contracts"][0]["verification"]
        self.assertTrue(any("verification" in e for e in self.errori(d)), self.errori(d))

    def test_un_contratto_non_applicabile_con_verifica(self) -> None:
        """E il verso opposto: `not_applicable` **non** puo' portarne una.

        Verificare cio' che si e' dichiarato inapplicabile e' una contraddizione,
        e lo schema la chiude col ramo `else`.
        """
        d = manifesto_minimo()
        d["contracts"][0]["status"] = "not_applicable"
        self.assertTrue(any("verification" in e for e in self.errori(d)), self.errori(d))

    def test_una_deviazione_senza_artefatto_ne_superficie(self) -> None:
        """L'`anyOf` dello schema: almeno uno dei due va registrato."""
        d = manifesto_minimo()
        d["deviations"] = [
            {
                "rule": "X-001",
                "observed_behavior": "qualcosa",
                "tracking": "da qualche parte",
                "detectable_before_invocation": True,
            }
        ]
        self.assertTrue(any("anyOf" in e for e in self.errori(d)), self.errori(d))

    def test_un_costrutto_ignoto_e_un_errore_non_un_salto(self) -> None:
        """La proprieta' che rende affidabile un validatore fatto in casa.

        Se un giorno lo schema usasse `oneOf` o `patternProperties`, il gate
        deve diventare rosso e non passare oltre: un validatore che salta cio'
        che non capisce dice verde su cio' che non ha guardato.
        """
        errori = gate.valida_forma({"a": 1}, {"type": "object", "oneOf": [{}]})
        self.assertTrue(any("non conosce" in e for e in errori), errori)


class LeRegoleDiSostanza(unittest.TestCase):
    def test_un_contratto_applicabile_e_taciuto(self) -> None:
        d = manifesto_minimo()
        errori = gate.verifica(d, CONTRATTI, {})
        self.assertTrue(any("tace" in e for e in errori), errori)

    def test_non_applicabile_e_deviato_insieme(self) -> None:
        """«Non si applica» e «si applica e non lo soddisfo» non stanno insieme."""
        d = manifesto_minimo()
        d["contracts"] = [{"id": "plenora-runtime-binding-v1", "status": "not_applicable"}]
        d["deviations"] = [
            {
                "rule": "plenora-runtime-binding-v1 §3",
                "surface": "cli",
                "observed_behavior": "non c'e'",
                "tracking": "da nessuna parte",
                "detectable_before_invocation": True,
            }
        ]
        errori = gate.verifica(d, CONTRATTI, {})
        self.assertTrue(any("non possono valere insieme" in e for e in errori), errori)

    def test_un_digest_che_non_e_quello_dei_byte(self) -> None:
        with tempfile.TemporaryDirectory() as temporanea:
            file = pathlib.Path(temporanea) / "plenora-io"
            file.write_bytes(b"non sono l'artefatto del manifesto")
            d = manifesto_minimo()
            errori = gate.verifica(d, CONTRATTI, {"plenora-io": file})
            self.assertTrue(any("i byte dicono" in e for e in errori), errori)

    def test_il_digest_giusto_passa(self) -> None:
        with tempfile.TemporaryDirectory() as temporanea:
            file = pathlib.Path(temporanea) / "plenora-io"
            contenuto = b"i byte veri"
            file.write_bytes(contenuto)
            d = manifesto_minimo()
            d["artifacts"][0]["digest"] = (
                "sha256:" + hashlib.sha256(contenuto).hexdigest()
            )
            errori = gate.verifica(d, CONTRATTI, {"plenora-io": file})
            self.assertFalse([e for e in errori if "i byte dicono" in e], errori)

    def test_un_artefatto_senza_file_non_passa_per_assenza(self) -> None:
        """Un digest che nessuno ricalcola non e' una garanzia."""
        errori = gate.verifica(manifesto_minimo(), CONTRATTI, {})
        self.assertTrue(any("nessun file passato" in e for e in errori), errori)

    def test_il_pin_diverso_dalla_fonte(self) -> None:
        errori = gate.verifica(manifesto_minimo(), CONTRATTI, {})
        self.assertTrue(any("due adozioni" in e for e in errori), errori)


class IlProfiloComeFonte(unittest.TestCase):
    def test_i_contratti_si_leggono_dal_profilo(self) -> None:
        """L'elenco non e' ricopiato: si segue il rimando fino al documento."""
        trovati = gate.contratti_del_profilo(CONTRATTI)
        for atteso in (
            "plenora-public-surfaces-v1",
            "plenora-capabilities-v2",
            "plenora-error-v1",
            "plenora-cli-v2",
            "plenora-row-diagnostics-v1",
            "plenora-arrow-interchange-v1",
        ):
            self.assertIn(atteso, trovati)

    def test_il_manifesto_reale_li_copre_tutti(self) -> None:
        percorso = gate.RADICE / "contracts" / "adoption-manifest.json"
        if not percorso.is_file():
            self.skipTest("il manifesto non e' stato ancora prodotto")
        d = json.loads(percorso.read_text(encoding="utf-8"))
        taciuti = gate.contratti_del_profilo(CONTRATTI) - {c["id"] for c in d["contracts"]}
        self.assertEqual(taciuti, set())


if __name__ == "__main__":
    unittest.main()
