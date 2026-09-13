"""Le controprove della disposizione delle prove, e del classificatore che usa.

Il classificatore e' la parte che porta rischio: sette gate gli chiedono se un
file sia prodotto o prove, e se rispondesse male sbaglierebbero tutti nello
stesso verso senza che nessuno lo veda. Qui si costruiscono alberi finti di cui
si conosce la risposta.
"""

from __future__ import annotations

import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import check_test_layout as gate  # noqa: E402
import perimetro_dei_sorgenti as perimetro  # noqa: E402


class UnCrateFinto(unittest.TestCase):
    """Un crate con i file che gli si danno, e la sua classificazione."""

    def classifica(self, file: dict[str, str]):
        temporanea = tempfile.TemporaryDirectory()
        self.addCleanup(temporanea.cleanup)
        crate = pathlib.Path(temporanea.name) / "finto"
        for nome, contenuto in file.items():
            percorso = crate / "src" / nome
            percorso.parent.mkdir(parents=True, exist_ok=True)
            percorso.write_bytes(contenuto.encode("utf-8"))
        return crate, perimetro.classifica(crate)


class IlClassificatore(UnCrateFinto):
    def test_un_modulo_di_prove_e_prove(self) -> None:
        crate, (prodotto, prove) = self.classifica(
            {"lib.rs": "#[cfg(test)]\nmod tests;\n", "tests.rs": "#[test]\nfn a() {}\n"}
        )
        self.assertEqual({p.name for p in prodotto}, {"lib.rs"})
        self.assertEqual({p.name for p in prove}, {"tests.rs"})

    def test_un_modulo_normale_e_prodotto(self) -> None:
        _, (prodotto, prove) = self.classifica(
            {"lib.rs": "mod motore;\n", "motore.rs": "pub fn a() {}\n"}
        )
        self.assertEqual({p.name for p in prodotto}, {"lib.rs", "motore.rs"})
        self.assertEqual(prove, set())

    def test_la_discendenza_di_un_modulo_di_prove_e_prove(self) -> None:
        """Un sottomodulo non ripete `#[cfg(test)]`, e resta prove.

        Senza la propagazione, `tests/aiutanti.rs` sarebbe prodotto: il suo
        `mod aiutanti;` non porta l'attributo, perche' il genitore lo porta
        gia'. E' il caso in cui un classificatore ingenuo comincia a contare
        fixture come codice spedito.
        """
        _, (prodotto, prove) = self.classifica(
            {
                "lib.rs": "#[cfg(test)]\nmod tests;\n",
                "tests.rs": "mod aiutanti;\n",
                "tests/aiutanti.rs": "pub fn fixture() {}\n",
            }
        )
        self.assertEqual({p.name for p in prodotto}, {"lib.rs"})
        self.assertEqual({p.name for p in prove}, {"tests.rs", "aiutanti.rs"})

    def test_la_forma_condizionata_a_una_feature_e_riconosciuta(self) -> None:
        _, (prodotto, prove) = self.classifica(
            {
                "lib.rs": '#[cfg(all(test, feature = "x"))]\nmod tests;\n',
                "tests.rs": "#[test]\nfn a() {}\n",
            }
        )
        self.assertEqual({p.name for p in prove}, {"tests.rs"})
        self.assertEqual({p.name for p in prodotto}, {"lib.rs"})

    def test_la_forma_con_mod_rs_e_riconosciuta(self) -> None:
        _, (prodotto, prove) = self.classifica(
            {
                "lib.rs": "#[cfg(test)]\nmod tests;\n",
                "tests/mod.rs": "#[test]\nfn a() {}\n",
            }
        )
        self.assertEqual({p.name for p in prove}, {"mod.rs"})

    def test_un_file_non_raggiunto_si_dichiara(self) -> None:
        """Un file che nessun `mod` nomina non e' ne' prodotto ne' prove.

        Serve che lo dica: classificarlo per difetto lo farebbe sparire da un
        conto o comparire nell'altro, e in tutti e due i casi in silenzio.
        """
        crate, _ = self.classifica({"lib.rs": "fn a() {}\n", "orfano.rs": "fn b() {}\n"})
        self.assertEqual(
            {p.name for p in perimetro.file_non_raggiunti(crate)}, {"orfano.rs"}
        )


class IlModuloInline(unittest.TestCase):
    def test_la_forma_con_la_graffa_e_una_violazione(self) -> None:
        trovati = gate.MODULO_INLINE.findall("#[cfg(test)]\nmod sonde {\n    fn a() {}\n}\n")
        self.assertEqual(trovati, ["sonde"])

    def test_la_forma_dichiarata_non_lo_e(self) -> None:
        self.assertEqual(gate.MODULO_INLINE.findall("#[cfg(test)]\nmod sonde;\n"), [])

    def test_la_dichiarazione_non_conta_fra_i_superstiti(self) -> None:
        """`mod tests;` e' la forma voluta, non un residuo.

        Contarla direbbe che ogni file separato e' un attributo di troppo, e il
        numero salirebbe proprio quando il lavoro e' stato fatto.
        """
        self.assertEqual(
            gate.ATTRIBUTO_SU_UN_ELEMENTO.findall("#[cfg(test)]\nmod tests;\n"), []
        )

    def test_un_attributo_su_un_elemento_conta(self) -> None:
        self.assertEqual(
            len(gate.ATTRIBUTO_SU_UN_ELEMENTO.findall("#[cfg(test)]\nfn aiutante() {}\n")),
            1,
        )


class L_alberoReale(unittest.TestCase):
    def test_nessun_modulo_inline_resta(self) -> None:
        violazioni, _, _ = gate.scansiona()
        self.assertEqual(violazioni, [], "\n".join(violazioni[:10]))

    def test_le_prove_sono_tante_quante_i_file(self) -> None:
        """Senza, «nessuna violazione» sarebbe vero anche di zero file di prove."""
        _, _, file_di_prove = gate.scansiona()
        self.assertGreater(file_di_prove, 40)


if __name__ == "__main__":
    unittest.main()
