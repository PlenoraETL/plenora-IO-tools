"""Sonde di ASSURANCE-N1.

La proprieta' che conta e' la **separazione delle due modalita'**: `--integrita`
puo' essere verde mentre il debito e' pieno, e non deve poter essere letto come
«la copertura negativa e' a posto».

E' la ragione per cui esistono due comandi invece di uno con una soglia: un
verde che significa due cose diverse a seconda di chi lo legge e' la forma di
falso verde che questa serie di checkpoint ha incontrato cinque volte.
"""

from __future__ import annotations

import unittest

from scripts import check_assurance_n1 as gate

APERTO = {
    "gruppo": "driver-x::f",
    "file": "scripts/check_assurance_n1.py",
    "righe": 3,
    "raggiunto_da_replay": 0,
    "disposizione": "test_tabellare",
    "nota": "ramo mai eseguito",
}
CHIUSO = {
    "gruppo": "driver-x::g",
    "file": "scripts/check_assurance_n1.py",
    "righe": 2,
    "raggiunto_da_replay": 0,
    "disposizione": "strutturale",
    "nota": "markup",
}


class SondeN1(unittest.TestCase):
    # --- integrita' del registro -------------------------------------------

    def test_un_registro_completo_e_integro(self) -> None:
        self.assertEqual(gate.integrita([APERTO, CHIUSO]), [])

    def test_una_disposizione_non_ammessa_e_rossa(self) -> None:
        voce = dict(APERTO, disposizione="vedremo")
        errori = gate.integrita([voce])
        self.assertTrue(any("non ammessa" in e for e in errori), errori)

    def test_una_disposizione_senza_nota_e_rossa(self) -> None:
        """Una casella riempita senza ragione non e' una disposizione."""
        voce = dict(APERTO, nota="")
        errori = gate.integrita([voce])
        self.assertTrue(any("senza nota" in e for e in errori), errori)

    def test_un_gruppo_duplicato_e_rosso(self) -> None:
        errori = gate.integrita([APERTO, dict(APERTO)])
        self.assertTrue(any("duplicata" in e for e in errori), errori)

    def test_un_file_sparito_e_rosso(self) -> None:
        """Una voce che sopravvive al proprio file tiene in vita un debito
        che non esiste piu', e gonfia il residuo."""
        voce = dict(APERTO, file="crates/driver-inesistente/src/lib.rs")
        errori = gate.integrita([voce])
        self.assertTrue(any("non esiste piu'" in e for e in errori), errori)

    def test_un_campo_mancante_e_rosso(self) -> None:
        voce = {k: v for k, v in APERTO.items() if k != "righe"}
        errori = gate.integrita([voce])
        self.assertTrue(any("campi mancanti" in e for e in errori), errori)

    # --- la separazione, che e' il punto ------------------------------------

    def test_il_debito_conta_solo_le_disposizioni_aperte(self) -> None:
        self.assertEqual([v["gruppo"] for v in gate.debito([APERTO, CHIUSO])], ["driver-x::f"])

    def test_un_registro_integro_puo_avere_debito_pieno(self) -> None:
        """La proprieta' decisiva.

        Un registro coerente **non** significa che i rami siano coperti. Se le
        due modalita' fossero una sola, questo caso darebbe verde.
        """
        gruppi = [APERTO, dict(APERTO, gruppo="driver-x::h")]
        self.assertEqual(gate.integrita(gruppi), [], "il registro e' coerente")
        self.assertEqual(len(gate.debito(gruppi)), 2, "e il debito e' pieno")

    def test_solo_disposizioni_chiuse_azzerano_il_debito(self) -> None:
        gruppi = [CHIUSO, dict(CHIUSO, gruppo="driver-x::i", disposizione="difensivo")]
        self.assertEqual(gate.integrita(gruppi), [])
        self.assertEqual(gate.debito(gruppi), [])


class SondeProva(unittest.TestCase):
    """«chiuso» deve nominare una prova, e la prova deve esistere.

    Senza questo vincolo bastava cambiare una riga del registro per far
    sparire un gruppo dal debito: il «semplice riallineamento» che
    ASSURANCE-N1 esiste per escludere.
    """

    # Un file **Rust** vero, con un test vero: il gate cerca `fn <nome>(` li'
    # dentro, quindi la fixture non puo' essere finta. La prima stesura usava
    # un file Python, dove i test si dichiarano con `def`: la sonda positiva
    # falliva, e la falsa diagnosi era «il gate rifiuta una prova vera».
    FILE = "crates/driver-filegdb/src/lib.rs"
    TEST_VERO = {
        "crate": "driver-filegdb",
        "test": "tests::create_without_gdal_feature_is_typed",
        "configurazione": "default",
        "esito": "coperto",
    }
    TEST_FINTO = dict(TEST_VERO, test="tests::un_test_che_non_esiste")

    def voce(self, **extra):
        base = {
            "gruppo": "driver-x::f",
            "file": self.FILE,
            "righe": 3,
            "raggiunto_da_replay": 0,
            "disposizione": "chiuso",
            "nota": "coperto da test",
        }
        base.update(extra)
        return base

    def test_chiuso_senza_prova_e_rosso(self) -> None:
        errori = gate.integrita([self.voce()])
        self.assertTrue(
            any("senza campo `prova`" in e for e in errori),
            f"un `chiuso` nudo e' passato: {errori}",
        )

    def test_una_prova_inesistente_e_rossa(self) -> None:
        errori = gate.integrita([self.voce(prova=[self.TEST_FINTO])])
        self.assertTrue(
            any("non ha un simbolo in" in e for e in errori),
            f"una prova fantasma e' passata: {errori}",
        )

    def test_una_prova_esistente_passa(self) -> None:
        """La controprova positiva: senza, «sempre rosso» sarebbe una difesa."""
        errori = gate.integrita([self.voce(prova=[self.TEST_VERO])])
        self.assertEqual(errori, [], f"una prova vera e' stata rifiutata: {errori}")

    def test_una_prova_su_un_gruppo_aperto_e_rossa(self) -> None:
        """Suggerirebbe una copertura che non conta nel debito."""
        errori = gate.integrita(
            [self.voce(disposizione="test_tabellare", prova=[self.TEST_VERO])]
        )
        self.assertTrue(
            any("non lo ammette" in e for e in errori),
            f"una prova su un gruppo aperto e' passata: {errori}",
        )

    def test_strutturale_e_difensivo_non_richiedono_prova(self) -> None:
        """Dicono che il ramo non e' esercitabile: un test non potrebbe esistere.

        La loro forza sta nella nota, che un revisore puo' contestare.
        """
        for disposizione in ("strutturale", "difensivo"):
            errori = gate.integrita([self.voce(disposizione=disposizione)])
            self.assertEqual(errori, [], f"{disposizione}: {errori}")

    def test_una_prova_malformata_e_rossa(self) -> None:
        errori = gate.integrita([self.voce(prova=self.TEST_VERO)])
        self.assertTrue(
            any("lista di voci" in e for e in errori),
            f"una prova non lista e' passata: {errori}",
        )

    def test_un_gruppo_chiuso_non_conta_nel_debito(self) -> None:
        chiuso = self.voce(prova=[self.TEST_VERO])
        aperto = self.voce(
            gruppo="driver-x::g",
            disposizione="test_tabellare",
            nota="ramo mai eseguito",
        )
        self.assertEqual([v["gruppo"] for v in gate.debito([chiuso, aperto])],
                         ["driver-x::g"])


if __name__ == "__main__":
    unittest.main()
