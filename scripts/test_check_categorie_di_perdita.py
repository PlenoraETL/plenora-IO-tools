#!/usr/bin/env python3
"""Sonde di `check_categorie_di_perdita.py`.

Un gate verde sul repository sano dice che oggi e' verde, non che domani
diventerebbe rosso. Ogni proprieta' che il gate afferma ha qui una sonda che la
viola e pretende il rosso, e le due che sono gia' costate un errore -- il
confine dei moduli di prova e la virgola finale -- ne hanno una loro.
"""

from __future__ import annotations

import copy
import unittest

from scripts import check_categorie_di_perdita as gate


def registro_sano() -> dict:
    return copy.deepcopy(gate.registro())


SORGENTE_SANA = [
    (
        "crates/finto/src/lib.rs",
        'const CATEGORIA: &str = "finta_categoria";\n'
        "fn produce(loss: &mut LossReport) {\n"
        "    loss.record(CATEGORIA, 1);\n"
        "}\n",
    )
]

REGISTRO_FINTO = {
    "schema_version": 1,
    "limite_di_lunghezza_byte": gate.LIMITE_ID_BYTE,
    "categorie": [{"id": "finta_categoria", "superficie": "finto", "forma": "costante"}],
    "vie_dinamiche_ammesse": [],
}


class IlRegistroReale(unittest.TestCase):
    def test_il_repository_passa(self):
        self.assertEqual(gate.verifica(), [])

    def test_ogni_identificatore_sta_nel_tetto(self):
        for voce in gate.registro()["categorie"]:
            self.assertLessEqual(len(voce["id"].encode("utf-8")), gate.LIMITE_ID_BYTE)


class LaFormaDelRegistro(unittest.TestCase):
    def test_schema_version_deve_essere_l_intero_uno(self):
        for valore in ("1", 1.0, True, 2, None):
            r = registro_sano()
            r["schema_version"] = valore
            self.assertTrue(
                any("schema_version" in e for e in gate.verifica(r)),
                f"{valore!r} non e' l'intero 1 e deve essere rifiutato",
            )

    def test_il_limite_dichiarato_deve_coincidere_con_quello_applicato(self):
        r = registro_sano()
        r["limite_di_lunghezza_byte"] = gate.LIMITE_ID_BYTE + 1
        self.assertTrue(any("divergono" in e for e in gate.verifica(r)))

    def test_una_voce_con_chiavi_diverse_e_rifiutata(self):
        r = registro_sano()
        r["categorie"][0] = {"id": "x", "superficie": "y"}
        self.assertTrue(any("chiavi diverse" in e for e in gate.verifica(r)))

    def test_identificatori_ripetuti_sono_rifiutati(self):
        r = registro_sano()
        r["categorie"].append(copy.deepcopy(r["categorie"][0]))
        self.assertTrue(any("ripetuti" in e for e in gate.verifica(r)))


class IlTettoInByte(unittest.TestCase):
    """Il tetto e' sui **byte UTF-8**, non sui caratteri."""

    def test_centoventotto_byte_passano_e_centoventinove_no(self):
        for lunghezza, atteso_rosso in ((gate.LIMITE_ID_BYTE, False), (gate.LIMITE_ID_BYTE + 1, True)):
            r = dict(REGISTRO_FINTO)
            r = copy.deepcopy(r)
            ident = "a" * lunghezza
            r["categorie"] = [{"id": ident, "superficie": "f", "forma": "costante"}]
            sorgente = [
                (
                    "crates/finto/src/lib.rs",
                    f'fn f(l: &mut LossReport) {{\n    l.record("{ident}", 1);\n}}\n',
                )
            ]
            errori = [e for e in gate.verifica(r, sorgente) if "byte UTF-8" in e]
            self.assertEqual(
                bool(errori), atteso_rosso, f"lunghezza {lunghezza}: {errori}"
            )

    def test_un_identificatore_unicode_si_misura_in_byte(self):
        # 64 «à» sono 64 caratteri e 128 byte: dentro il tetto. 65 lo superano
        # pur restando 65 caratteri, che e' meta' del tetto contato male.
        for caratteri, atteso_rosso in ((64, False), (65, True)):
            ident = "à" * caratteri
            r = copy.deepcopy(REGISTRO_FINTO)
            r["categorie"] = [{"id": ident, "superficie": "f", "forma": "costante"}]
            sorgente = [
                (
                    "crates/finto/src/lib.rs",
                    f'fn f(l: &mut LossReport) {{\n    l.record("{ident}", 1);\n}}\n',
                )
            ]
            errori = [e for e in gate.verifica(r, sorgente) if "byte UTF-8" in e]
            self.assertEqual(
                bool(errori),
                atteso_rosso,
                f"{caratteri} caratteri = {len(ident.encode())} byte: {errori}",
            )


class IDueVersi(unittest.TestCase):
    def test_una_categoria_prodotta_e_non_dichiarata_e_rossa(self):
        # Il registro resta non vuoto: svuotarlo del tutto sarebbe un altro
        # difetto, e la sonda arrossava per quello invece che per questo.
        sorgente = SORGENTE_SANA + [
            (
                "crates/altro/src/lib.rs",
                "fn f(l: &mut LossReport) {\n"
                '    l.record("mai_dichiarata", 1);\n'
                "}\n",
            )
        ]
        errori = gate.verifica(REGISTRO_FINTO, sorgente)
        self.assertTrue(any("assenti dal registro" in e for e in errori), errori)
        self.assertTrue(any("mai_dichiarata" in e for e in errori), errori)

    def test_una_voce_che_nessuno_produce_e_rossa(self):
        r = copy.deepcopy(REGISTRO_FINTO)
        r["categorie"].append({"id": "mai_prodotta", "superficie": "f", "forma": "costante"})
        errori = gate.verifica(r, SORGENTE_SANA)
        self.assertTrue(any("nessun sito produce" in e for e in errori), errori)

    def test_il_finto_sano_passa(self):
        self.assertEqual(gate.verifica(REGISTRO_FINTO, SORGENTE_SANA), [])


class LaViaDinamica(unittest.TestCase):
    def test_una_seconda_via_dinamica_e_rossa(self):
        sorgente = SORGENTE_SANA + [
            (
                "crates/altro/src/lib.rs",
                "fn f(l: &mut LossReport, n: &str) {\n"
                '    l.record(&format!("categoria {n}"), 1);\n'
                "}\n",
            )
        ]
        errori = gate.verifica(REGISTRO_FINTO, sorgente)
        self.assertTrue(any("non dichiarate" in e for e in errori), errori)

    def test_una_via_dichiarata_e_scomparsa_e_rossa(self):
        r = copy.deepcopy(REGISTRO_FINTO)
        r["vie_dinamiche_ammesse"] = [
            {"file": "crates/sparito/src/lib.rs", "espressione": '&format!("x {y}")'}
        ]
        errori = gate.verifica(r, SORGENTE_SANA)
        self.assertTrue(any("non trovate" in e for e in errori), errori)

    def test_la_via_dichiarata_e_presente_non_e_rossa(self):
        sorgente = SORGENTE_SANA + [
            (
                "crates/altro/src/lib.rs",
                "fn f(l: &mut LossReport, n: &str) {\n"
                '    l.record(&format!("categoria {n}"), 1);\n'
                "}\n",
            )
        ]
        r = copy.deepcopy(REGISTRO_FINTO)
        r["vie_dinamiche_ammesse"] = [
            {
                "file": "crates/altro/src/lib.rs",
                "espressione": '&format!("categoria {n}")',
            }
        ]
        self.assertEqual(gate.verifica(r, sorgente), [])


class LeDueLettureCheSonoGiaCostateUnErrore(unittest.TestCase):
    def test_il_confine_non_e_il_primo_cfg_test(self):
        righe = [
            "#[cfg(test)]",
            "fn aiutante() {}",
            "fn produzione() {}",
            "#[cfg(test)]",
            "mod tests {",
        ]
        self.assertEqual(
            gate.confine_delle_prove(righe),
            3,
            "il confine e' la coppia `#[cfg(test)]` + `mod`, non un attributo su un elemento",
        )

    def test_la_virgola_finale_non_e_un_argomento(self):
        testo = 'loss.record(\n    "categoria",\n    1,\n);'
        apertura = testo.index("(")
        self.assertEqual(gate.argomenti(testo, apertura), ['"categoria"', "1"])

    def test_gli_accenti_sopravvivono_alla_lettura_del_literal(self):
        # `unicode_escape` renderebbe «entità» come «entitÃ ».
        self.assertEqual(gate.testo_rust("entità"), "entità")
        self.assertEqual(gate.testo_rust(r"virgolette \" dentro"), 'virgolette " dentro')


class LaRisoluzione(unittest.TestCase):
    def test_un_metodo_chiuso_con_argomenti_non_e_dinamico(self):
        sorgente = [
            (
                "crates/finto/src/lib.rs",
                'const A: &str = "alfa";\n'
                'const B: &str = "beta";\n'
                "impl T {\n"
                "    const fn scegli(self, s: S) -> Option<&'static str> {\n"
                "        match (self, s) {\n"
                "            (Self::X, S::U) => Some(A),\n"
                "            (Self::Y, S::V) => Some(B),\n"
                "        }\n"
                "    }\n"
                "}\n"
                "fn f(l: &mut LossReport, t: T, s: S) {\n"
                "    let Some(categoria) = t.scegli(s) else {\n"
                "        return;\n"
                "    };\n"
                "    l.record(categoria, 1);\n"
                "}\n",
            )
        ]
        r = copy.deepcopy(REGISTRO_FINTO)
        r["categorie"] = [
            {"id": "alfa", "superficie": "f", "forma": "costante"},
            {"id": "beta", "superficie": "f", "forma": "costante"},
        ]
        self.assertEqual(gate.verifica(r, sorgente), [])

    def test_l_omonimo_a_quattro_argomenti_non_entra_nel_censimento(self):
        sorgente = SORGENTE_SANA + [
            (
                "crates/altro/src/lib.rs",
                "fn f(d: &mut ShpRowDiagnostics, i: usize, c: &str) {\n"
                "    d.record(i, c, None, None);\n"
                "}\n",
            )
        ]
        self.assertEqual(gate.verifica(REGISTRO_FINTO, sorgente), [])


if __name__ == "__main__":
    unittest.main(verbosity=1)
