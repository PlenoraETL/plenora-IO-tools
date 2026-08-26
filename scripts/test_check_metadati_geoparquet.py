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

# Il perimetro finto su cui girano le sonde: due campi, quanti bastano a
# distinguere «coperto» da «non coperto».
CAMPI_FINTI = {"version", "encoding"}

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
        """Il gate su un modulo finto, con un perimetro finto.

        Il perimetro viene dagli schemi ufficiali, e qui si inietta: provare la
        copertura su moduli inventati e' l'unico modo di provare la regola
        invece di provare il modulo di oggi.
        """
        return gate.verifica(produzione + sonde, campi=CAMPI_FINTI)[0]

    def test_un_modulo_coerente_e_verde(self) -> None:
        self.assertEqual(self.errori(), [])

    def test_un_campo_della_specifica_mai_provato_e_rosso(self) -> None:
        """Il caso per cui il gate esiste, e che la versione circolare **non
        poteva vedere**: un campo che la specifica definisce e che il driver non
        legge non entrava nemmeno nel suo perimetro, quindi risultava coperto.

        Qui il perimetro viene dagli schemi, e la lacuna si vede."""
        errori = gate.verifica(PRODUZIONE + SONDE, campi=CAMPI_FINTI | {"edges"})[0]
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
        """Il modo piu' sottile di rendere verde il gate, e ora impossibile per
        costruzione.

        Nella versione circolare il perimetro veniva dai `.get("...")` del
        modulo, quindi una sonda che ne dichiarava uno se lo trovava nel
        perimetro e lo copriva da sola. Ora il perimetro viene dagli schemi
        ufficiali: una sonda che nomina qualcosa che non e' un campo della
        specifica non trova un campo suo, ed e' rossa.
        """
        inventato = SONDE.replace(
            "    #[test]\n    fn documento_minimo_e_accettato() {}",
            '    #[test]\n    fn finto_e_accettato() { let _ = o.get("finto"); }',
        )
        errori = self.errori(sonde=inventato)
        self.assertTrue(any("non comincia con nessuno" in e for e in errori), errori)
        self.assertNotIn("finto", gate.verifica(PRODUZIONE + inventato, campi=CAMPI_FINTI)[1])

    def test_un_perimetro_vuoto_e_rosso(self) -> None:
        """Senza perimetro il gate misura il vuoto, e va rifatto invece che
        tolto."""
        errori = gate.verifica("fn niente() {}" + chr(10), campi=set())[0]
        self.assertTrue(any("nessun campo estratto" in e for e in errori), errori)

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

    def test_il_modulo_reale_e_coperto(self) -> None:
        """La controprova positiva: senza, «sempre rosso» sarebbe una difesa."""
        errori, copertura = gate.verifica()
        self.assertEqual(errori, [], errori)
        # Gli undici campi che gli schemi ufficiali definiscono, piu' il
        # documento stesso. Il numero non e' scritto qui a mano: viene
        # dall'autorita', e questa asserzione lo fissa perche' una sua
        # variazione sia una decisione e non un effetto.
        campi, guasti = gate.campi_dello_schema()
        self.assertEqual(guasti, [], guasti)
        self.assertEqual(len(campi), 11)
        self.assertEqual(len(copertura), 12)
        for campo, versi in copertura.items():
            self.assertTrue(versi["positiva"], campo)
            self.assertTrue(versi["negativa"], campo)


if __name__ == "__main__":
    unittest.main()
