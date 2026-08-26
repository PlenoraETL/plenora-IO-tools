"""Sonde del gate che tiene onesta la validazione dei metadati GeoParquet.

Il gate afferma una cosa sola e grossa: che **ogni** campo che il driver legge
sia provato nei due versi. Se sbagliasse, l'invariante `lotto.s10` direbbe
«validato per intero» misurando una parte, ed e' esattamente il difetto che il
lotto esiste per chiudere -- prima di S10 il lettore consultava cinque campi su
undici e nessuno se ne accorgeva.

Le sonde muovono i modi in cui il gate potrebbe diventare verde senza meritarlo:
un campo nuovo senza prove, un verso solo, un nome che non dice che cosa prova,
e -- il piu' sottile -- una sonda che si inventa un campo per poi coprirlo da
sola.
"""

from __future__ import annotations

import unittest
from unittest import mock

from scripts import check_metadati_geoparquet as gate

PRODUZIONE = '''
fn versione(o: &Map) -> Result<&'static str> {
    let d = stringa_obbligatoria(o, "version")?;
}
fn colonna(o: &Map) -> Result<()> {
    let _ = o.get("encoding");
}
'''

VERSIONI_NEL_MODULO = '''pub const VERSIONI_SUPPORTATE: [&str; 2] = ["1.0.0", "1.1.0"];
'''

SONDE = '''
#[cfg(test)]
mod sonde {
    #[test]
    fn version_dei_due_schemi_e_accettata() {}

    #[test]
    fn version_di_un_altro_schema_e_valida_e_non_supportata() {}

    #[test]
    fn encoding_wkb_e_accettato() {}

    #[test]
    fn encoding_sconosciuto_e_non_conforme() {}

    #[test]
    fn documento_minimo_e_accettato() {}

    #[test]
    fn documento_che_non_e_json_e_non_conforme() {}
}
'''


class SondeDelGate(unittest.TestCase):
    def errori(self, produzione: str = PRODUZIONE, sonde: str = SONDE) -> list[str]:
        """Il gate su un modulo finto.

        Il perimetro dichiarato viene dal descrittore vero, che qui non c'entra:
        si finge coerente con il modulo finto, cosi' le sonde della copertura
        provano la copertura e quelle del perimetro provano il perimetro.
        """
        with mock.patch.object(gate, "versione_dichiarata", return_value="1.1.0"):
            return gate.verifica(VERSIONI_NEL_MODULO + produzione + sonde)[0]

    def test_un_modulo_coerente_e_verde(self) -> None:
        self.assertEqual(self.errori(), [])

    def test_un_campo_letto_e_mai_provato_e_rosso(self) -> None:
        """Il caso per cui il gate esiste: un campo aggiunto senza sonde."""
        con_edges = PRODUZIONE + '\nfn bordi(o: &Map) { let _ = o.get("edges"); }\n'
        errori = self.errori(produzione=con_edges)
        self.assertTrue(any("«edges»" in e for e in errori), errori)
        self.assertEqual(
            len([e for e in errori if "«edges»" in e]),
            2,
            "manca in entrambi i versi, e il gate lo dice due volte",
        )

    def test_un_verso_solo_non_basta(self) -> None:
        """Con la sola negativa il gate direbbe che il validatore rifiuta tutto."""
        senza_positiva = SONDE.replace("fn encoding_wkb_e_accettato", "fn encoding_wkb_e_non_conforme")
        errori = self.errori(sonde=senza_positiva)
        self.assertTrue(
            any("«encoding» non ha una prova positiva" in e for e in errori), errori
        )

        senza_negativa = SONDE.replace(
            "fn encoding_sconosciuto_e_non_conforme", "fn encoding_sconosciuto_e_accettato"
        )
        errori = self.errori(sonde=senza_negativa)
        self.assertTrue(
            any("«encoding» non ha una prova negativa" in e for e in errori), errori
        )

    def test_una_sonda_senza_verso_nel_nome_e_rossa(self) -> None:
        """Un nome che non dice se accetta o rifiuta lascia smettere di provare."""
        muta = SONDE.replace("fn encoding_wkb_e_accettato", "fn encoding_wkb_funziona")
        errori = self.errori(sonde=muta)
        self.assertTrue(any("non dichiara il proprio verso" in e for e in errori), errori)

    def test_una_sonda_che_non_nomina_un_campo_e_rossa(self) -> None:
        muta = SONDE.replace("fn encoding_wkb_e_accettato", "fn qualcosa_e_accettato")
        errori = self.errori(sonde=muta)
        self.assertTrue(any("non comincia con nessuno" in e for e in errori), errori)

    def test_una_sonda_non_puo_inventarsi_un_campo(self) -> None:
        """Il modo piu' sottile di rendere verde il gate: dichiarare un campo
        dentro le sonde e coprirlo da soli.

        L'estrazione si ferma a `mod sonde` proprio per questo: cio' che il gate
        misura viene da cio' che il driver legge in produzione, e le sonde
        costruiscono documenti -- se contassero, il perimetro se lo
        sceglierebbero loro.
        """
        inventato = SONDE.replace(
            "    #[test]\n    fn documento_minimo_e_accettato() {}",
            '    #[test]\n    fn finto_e_accettato() { let _ = o.get("finto"); }',
        )
        errori = self.errori(sonde=inventato)
        # `finto` non entra fra i campi, quindi la sonda che lo nomina non
        # trova un campo suo...
        self.assertTrue(any("non comincia con nessuno" in e for e in errori), errori)
        # ...e il campo inventato non compare nella copertura.
        self.assertNotIn("finto", gate.verifica(PRODUZIONE + inventato)[1])

    def test_un_modulo_che_non_interroga_niente_e_rosso(self) -> None:
        """Se il modulo cambia forma, il gate misura il vuoto e deve dirlo."""
        with mock.patch.object(gate, "versione_dichiarata", return_value="1.1.0"):
            errori = gate.verifica("fn niente() {}" + chr(10))[0]
        self.assertTrue(any("nessun campo interrogato" in e for e in errori), errori)

    def test_i_versi_sono_un_elenco_chiuso(self) -> None:
        """Aggiungerne uno qui senza pensarci sarebbe il modo di far passare un
        nome che non dice niente."""
        self.assertEqual(
            set(gate.POSITIVI),
            {"_e_accettato", "_e_accettata", "_sono_accettati", "_sono_accettate"},
        )
        self.assertEqual(
            set(gate.NEGATIVI),
            {
                "_e_non_conforme",
                "_non_supportata",
                "_non_supportato",
                "_non_supportati",
                "_non_e_esprimibile",
            },
        )

    # --- il perimetro dichiarato e quello applicato ----------------------

    def test_un_perimetro_dichiarato_diverso_da_quello_applicato_e_rosso(self) -> None:
        """`spec_version_supported` e' un'affermazione pubblica: chi legge il
        catalogo decide su di essa, e un perimetro dichiarato diverso da quello
        applicato e' peggio di nessun perimetro."""
        con_versioni = VERSIONI_NEL_MODULO + PRODUZIONE
        with mock.patch.object(gate, "versione_dichiarata", return_value="2.0.0"):
            errori = gate.perimetro_dichiarato(con_versioni)
        self.assertTrue(any("2.0.0" in e and "1.1.0" in e for e in errori), errori)

        with mock.patch.object(gate, "versione_dichiarata", return_value="1.1.0"):
            self.assertEqual(gate.perimetro_dichiarato(con_versioni), [])

    def test_un_perimetro_non_dichiarato_e_rosso(self) -> None:
        con_versioni = VERSIONI_NEL_MODULO + PRODUZIONE
        with mock.patch.object(gate, "versione_dichiarata", return_value=None):
            errori = gate.perimetro_dichiarato(con_versioni)
        self.assertTrue(any("non dichiara" in e for e in errori), errori)

    def test_senza_versioni_nel_modulo_il_gate_e_rosso(self) -> None:
        """Se il modulo cambia forma, il confronto non ha piu' un termine."""
        errori = gate.perimetro_dichiarato(PRODUZIONE)
        self.assertTrue(any("VERSIONI_SUPPORTATE" in e for e in errori), errori)

    def test_il_perimetro_reale_e_coerente(self) -> None:
        """La controprova positiva sui file veri."""
        self.assertEqual(gate.versioni_accettate(gate.sorgente()), ["1.0.0", "1.1.0"])
        self.assertEqual(gate.versione_dichiarata(), "1.1.0")
        self.assertEqual(gate.perimetro_dichiarato(gate.sorgente()), [])

    def test_il_modulo_reale_e_coperto(self) -> None:
        """La controprova positiva: senza, «sempre rosso» sarebbe una difesa."""
        errori, copertura = gate.verifica()
        self.assertEqual(errori, [], errori)
        # Gli undici campi della specifica, piu' il documento stesso.
        self.assertEqual(len(copertura), 12)
        for campo, versi in copertura.items():
            self.assertTrue(versi["positiva"], campo)
            self.assertTrue(versi["negativa"], campo)


if __name__ == "__main__":
    unittest.main()
