"""Sonde dello snapshot dei quartetti.

Il gate ha tre doveri: leggere il quartetto giusto da ogni forma di costruttore,
accorgersi di un cambio, e non accorgersi di cio' che non e' un cambio --
spostamenti e riformattazioni, che sono la ragione per cui l'identita' e'
`percorso::funzione` e non `path:riga`.

La sonda che conta di piu' e' quella sul difetto reale della tranche 2: un
`new(Schema, Validate, ...)` diventato `schema_redatto` sposta il codice da
`Generic` a `Schema` **senza cambiare una riga di assi**. Un gate basato sul
diff delle varianti enum non lo vedrebbe.
"""

from __future__ import annotations

import unittest

from scripts import check_quartetto_sito as gate


class SondeQuartetto(unittest.TestCase):
    def quartetti(self, sorgente: str) -> dict[str, list[str]]:
        import pathlib
        import tempfile

        percorso = pathlib.Path(tempfile.mkdtemp()) / "lib.rs"
        percorso.write_text(sorgente, encoding="utf-8")
        self.addCleanup(lambda: percorso.unlink(missing_ok=True))
        return gate.quartetti_del_file(percorso)

    # --- primo dovere: leggere il quartetto da ogni forma -------------------

    def test_un_costruttore_di_famiglia_porta_il_suo_quartetto(self) -> None:
        q = self.quartetti(
            "fn f() -> PlenoraIoError {\n"
            "    PlenoraIoError::schema_redatto(&PublicMessage::Curated(\"x\"))\n"
            "}\n"
        )
        self.assertEqual(q, {"f": ["Schema/Validate/Schema/Never"]})

    def test_new_porta_generic_per_costruzione(self) -> None:
        """`new` non nomina il codice: lo mette a `Generic`.

        E' il fatto che ha reso invisibile il difetto della tranche 2.
        """
        q = self.quartetti(
            "fn f() -> PlenoraIoError {\n"
            "    PlenoraIoError::new(ErrorCategory::Schema, ErrorPhase::Validate,\n"
            "        RemoteEffect::None, RetryDisposition::Never, \"x\")\n"
            "}\n"
        )
        self.assertEqual(q, {"f": ["esplicito/esplicito/Generic/esplicito"]})

    def test_redatto_porta_il_codice_del_primo_argomento(self) -> None:
        q = self.quartetti(
            "fn f() -> PlenoraIoError {\n"
            "    PlenoraIoError::redatto(IoErrorCode::Io, ErrorCategory::Io,\n"
            "        ErrorPhase::Read, RemoteEffect::None, RetryDisposition::Never, &m)\n"
            "}\n"
        )
        self.assertEqual(q, {"f": ["esplicito/esplicito/Io/esplicito"]})

    # --- il difetto vero ----------------------------------------------------

    def test_new_sostituito_da_una_famiglia_cambia_il_quartetto(self) -> None:
        """La regressione della tranche 2, in miniatura.

        Categoria, fase, effetto e retry restano gli stessi; il diff non mostra
        una riga di assi cambiata. Solo il codice si sposta.
        """
        prima = self.quartetti(
            "fn f() -> PlenoraIoError {\n"
            "    PlenoraIoError::new(ErrorCategory::Schema, ErrorPhase::Validate,\n"
            "        RemoteEffect::None, RetryDisposition::Never, \"x\")\n"
            "}\n"
        )
        dopo = self.quartetti(
            "fn f() -> PlenoraIoError {\n"
            "    PlenoraIoError::schema_redatto(&PublicMessage::Curated(\"x\"))\n"
            "}\n"
        )
        self.assertNotEqual(prima, dopo)
        self.assertEqual(gate.confronta({"x": prima}, {"x": dopo})[0].count("cambiato"), 1)

    def test_redatto_con_generic_conserva_il_quartetto_di_new(self) -> None:
        """La correzione: `redatto` riceve il codice, quindi lo si conserva."""
        prima = self.quartetti(
            "fn f() -> PlenoraIoError {\n"
            "    PlenoraIoError::new(ErrorCategory::Schema, ErrorPhase::Validate,\n"
            "        RemoteEffect::None, RetryDisposition::Never, \"x\")\n"
            "}\n"
        )
        dopo = self.quartetti(
            "fn f() -> PlenoraIoError {\n"
            "    PlenoraIoError::redatto(IoErrorCode::Generic, ErrorCategory::Schema,\n"
            "        ErrorPhase::Validate, RemoteEffect::None, RetryDisposition::Never, &m)\n"
            "}\n"
        )
        self.assertEqual(prima, dopo)
        self.assertEqual(gate.confronta({"x": prima}, {"x": dopo}), [])

    # --- il cambio COMPENSATO: la sonda che il multiinsieme non superava ----

    @staticmethod
    def _sorgente(primo: str, secondo: str) -> str:
        """Una funzione con due rami d'errore, in ordine."""
        return "\n".join(
            [
                "fn f(a: bool) -> PlenoraIoError {",
                "    if a {",
                f"        return PlenoraIoError::{primo}(&m);",
                "    }",
                f"    PlenoraIoError::{secondo}(&m)",
                "}",
                "",
            ]
        )

    def test_due_siti_che_si_scambiano_il_quartetto_sono_rossi(self) -> None:
        """La sonda decisiva.

        Stessa funzione, due costruttori con quartetti diversi, scambiati fra
        loro. Il multiinsieme e' identico: con `sorted()` questo gate sarebbe
        rimasto **verde**, e «preservato sito per sito» sarebbe stata una frase
        invece di una proprieta'.

        Non e' un caso di scuola: due rami d'errore adiacenti che si scambiano
        il costruttore e' esattamente cio' che una sostituzione meccanica
        sbagliata produce.
        """
        prima = self.quartetti(self._sorgente("schema_redatto", "crs_redatto"))
        dopo = self.quartetti(self._sorgente("crs_redatto", "schema_redatto"))

        self.assertEqual(
            sorted(prima["f"]),
            sorted(dopo["f"]),
            "la premessa della sonda: come multiinsieme sono indistinguibili",
        )
        self.assertNotEqual(prima, dopo, "come sequenza ordinata devono differire")
        errori = gate.confronta({"x": prima}, {"x": dopo})
        self.assertTrue(any("cambiato" in e for e in errori), errori)

    def test_uno_scambio_fra_limite_e_contratto_e_rosso(self) -> None:
        """Seconda coppia, per non provare la proprieta' su un caso solo."""
        prima = self.quartetti(self._sorgente("limite_redatto", "contratto_redatto"))
        dopo = self.quartetti(self._sorgente("contratto_redatto", "limite_redatto"))

        self.assertEqual(sorted(prima["f"]), sorted(dopo["f"]))
        self.assertNotEqual(prima, dopo)
        self.assertTrue(gate.confronta({"x": prima}, {"x": dopo}))

    # --- terzo dovere: non accendersi su cio' che non e' un cambio ----------

    def test_uno_spostamento_non_e_un_cambio(self) -> None:
        """L'identita' e' `percorso::funzione`: e' la lezione di INFRA-1."""
        prima = self.quartetti(
            "fn f() -> PlenoraIoError {\n"
            "    PlenoraIoError::limite_redatto(&m)\n"
            "}\n"
        )
        dopo = self.quartetti(
            "// una riga di commento in piu'\n"
            "// e un'altra\n"
            "fn f() -> PlenoraIoError {\n"
            "    PlenoraIoError::limite_redatto(\n"
            "        &m,\n"
            "    )\n"
            "}\n"
        )
        self.assertEqual(prima, dopo)

    def test_il_codice_di_test_non_conta(self) -> None:
        """Nei test la via legacy resta lecita finche' la si smantella."""
        q = self.quartetti(
            "fn f() -> PlenoraIoError {\n"
            "    PlenoraIoError::limite_redatto(&m)\n"
            "}\n"
            "\n"
            "#[cfg(test)]\n"
            "mod tests {\n"
            "    #[test]\n"
            "    fn t() {\n"
            "        let _ = PlenoraIoError::Contract(\"x\".to_owned());\n"
            "    }\n"
            "}\n"
        )
        self.assertEqual(q, {"f": ["ResourceLimit/Validate/LimitExceeded/Never"]})

    # --- secondo dovere: le forme di cambio -------------------------------

    def test_un_sito_nuovo_e_rosso(self) -> None:
        errori = gate.confronta({}, {"x": {"f": ["Schema/Validate/Schema/Never"]}})
        self.assertTrue(any("sito nuovo" in e for e in errori), errori)

    def test_un_sito_sparito_e_rosso(self) -> None:
        errori = gate.confronta({"x": {"f": ["Schema/Validate/Schema/Never"]}}, {})
        self.assertTrue(any("sito sparito" in e for e in errori), errori)

    def test_una_costruzione_in_piu_nella_stessa_funzione_e_rossa(self) -> None:
        """Due errori dove ce n'era uno cambiano cosa la funzione puo' emettere."""
        errori = gate.confronta(
            {"x": {"f": ["Schema/Validate/Schema/Never"]}},
            {"x": {"f": ["Schema/Validate/Schema/Never", "Crs/Validate/Crs/Never"]}},
        )
        self.assertTrue(any("cambiato" in e for e in errori), errori)


if __name__ == "__main__":
    unittest.main()
