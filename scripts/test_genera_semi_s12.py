"""Sonde dei semi del lotto S12.

Un seme e' utile quando **raggiunge** cio' che dovrebbe esercitare. Che ci
arrivi lo dice la misura di profondita', che gira sui semi versionati e
pretende ogni requisito dichiarato; qui si verifica l'altra meta': che il seme
sia il caso che il suo nome promette, e che `--verifica` si accorga di un blob
che questo file non ha prodotto.
"""

from __future__ import annotations

import io
import json
import pathlib
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout

from scripts import genera_semi_s12 as gate


class SondeDeiSemiWkt(unittest.TestCase):
    def test_ogni_dimensionalita_ha_il_suo_seme(self) -> None:
        semi = gate.semi_wkt()
        self.assertIn("POINT (1 2)", semi.values())
        self.assertIn("POINT Z (1 2 3)", semi.values())
        self.assertIn("POINT M (1 2 3)", semi.values())
        self.assertIn("POINT ZM (1 2 3 4)", semi.values())

    def test_le_due_sintassi_di_multipoint_sono_entrambe_presenti(self) -> None:
        """Sono lo stesso oggetto scritto in due modi, e un parser puo'
        accettarne uno solo senza che nessun test se ne accorga."""
        semi = gate.semi_wkt()
        self.assertEqual(semi["multipunto-nudo"], "MULTIPOINT (1 2,3 4)")
        self.assertEqual(semi["multipunto-fra-parentesi"], "MULTIPOINT ((1 2),(3 4))")

    def test_il_suffisso_attaccato_e_un_seme(self) -> None:
        """`POINTZ` e' la forma che la sonda comparativa ha trovato mancante:
        senza un seme, tornerebbe a mancare senza rumore."""
        self.assertEqual(gate.semi_wkt()["punto-suffisso-attaccato"], "POINTZ (1 2 3)")

    def test_il_seme_dell_annidamento_supera_il_tetto(self) -> None:
        """Fermarsi al tetto proverebbe il caso ammesso, non quello rifiutato."""
        seme = gate.semi_wkt()["tetto-annidamento"]
        self.assertGreater(seme.count("GEOMETRYCOLLECTION"), gate.PROFONDITA_WKT)

    def test_i_semi_di_rifiuto_sono_davvero_malformati(self) -> None:
        semi = gate.semi_wkt()
        self.assertEqual(semi["rifiuto-punto-vuoto"], "POINT EMPTY")
        self.assertTrue(semi["rifiuto-troncato"].count("(") > semi["rifiuto-troncato"].count(")"))
        self.assertTrue(semi["rifiuto-testo-residuo"].endswith("))"))


class SondeDeiSemiGeoJson(unittest.TestCase):
    def test_ogni_seme_e_una_feature_collection(self) -> None:
        """E' cio' che il target legge: una geometria nuda non attraverserebbe
        il lettore delle feature."""
        for nome, testo in gate.semi_geojson().items():
            with self.subTest(nome):
                if nome == "rifiuto-troncato":
                    continue
                documento = json.loads(testo)
                self.assertEqual(documento["type"], "FeatureCollection")
                self.assertEqual(len(documento["features"]), 1)

    def test_le_chiavi_invertite_sono_davvero_invertite(self) -> None:
        """In JSON le chiavi non hanno ordine, ed e' la ragione per cui
        l'albero delle coordinate esiste: senza questo seme, l'ordine inverso
        non sarebbe mai attraversato."""
        geometria = json.loads(gate.semi_geojson()["chiavi-invertite"])["features"][0][
            "geometry"
        ]
        self.assertEqual(list(geometria)[0], "coordinates")

    def test_il_seme_dell_annidamento_supera_il_tetto_della_campagna(self) -> None:
        """Il tetto della campagna e' 32, non 64: oltre i sessantadue livelli
        e' `serde_json` a rifiutare per primo, e il nostro non morderebbe."""
        seme = gate.semi_geojson()["tetto-annidamento"]
        self.assertGreater(
            seme.count("GeometryCollection"), gate.PROFONDITA_GEOJSON
        )
        self.assertLess(seme.count("GeometryCollection"), 62)

    def test_il_seme_troncato_non_e_json(self) -> None:
        with self.assertRaises(json.JSONDecodeError):
            json.loads(gate.semi_geojson()["rifiuto-troncato"])


class SondeDellaVerifica(unittest.TestCase):
    """`--verifica` deve accorgersi di cio' che non ha prodotto."""

    def setUp(self) -> None:
        temporanea = tempfile.TemporaryDirectory()
        self.addCleanup(temporanea.cleanup)
        radice = pathlib.Path(temporanea.name)
        self.wkt = radice / "wkt"
        self.geojson = radice / "geojson"
        precedenti = (gate.SEMI_WKT, gate.SEMI_GEOJSON)
        gate.SEMI_WKT = self.wkt
        gate.SEMI_GEOJSON = self.geojson
        self.addCleanup(setattr, gate, "SEMI_WKT", precedenti[0])
        self.addCleanup(setattr, gate, "SEMI_GEOJSON", precedenti[1])

    def esegui(self, verifica: bool) -> tuple[int, str]:
        uscita, errori = io.StringIO(), io.StringIO()
        with redirect_stdout(uscita), redirect_stderr(errori):
            codice = gate.main(["--verifica"] if verifica else [])
        return codice, uscita.getvalue() + errori.getvalue()

    def test_i_semi_appena_scritti_si_verificano(self) -> None:
        self.assertEqual(self.esegui(False)[0], 0)
        codice, testo = self.esegui(True)
        self.assertEqual(codice, 0, testo)
        self.assertIn("byte a byte", testo)

    def test_un_seme_assente_e_rosso(self) -> None:
        codice, testo = self.esegui(True)
        self.assertEqual(codice, 1)
        self.assertIn("seme assente", testo)

    def test_un_seme_modificato_e_rosso(self) -> None:
        self.esegui(False)
        percorso = next(self.wkt.glob(f"{gate.PREFISSO}*"))
        percorso.write_bytes(percorso.read_bytes() + b" ")
        codice, testo = self.esegui(True)
        self.assertEqual(codice, 1)
        self.assertIn("dalla specifica", testo)

    def test_un_seme_orfano_col_prefisso_e_rosso(self) -> None:
        self.esegui(False)
        (self.geojson / f"{gate.PREFISSO}inventato.geojson").write_bytes(b"{}")
        codice, testo = self.esegui(True)
        self.assertEqual(codice, 1)
        self.assertIn("che questo file non dichiara", testo)

    def test_i_semi_precedenti_restano_fuori_dal_perimetro(self) -> None:
        """I due semi storici di `wkt_parse` sono precedenti a questo file:
        dichiararli senza averli derivati vorrebbe dire firmare byte che non ho
        prodotto."""
        self.esegui(False)
        (self.wkt / "polygon-con-anello-vuoto.wkt").write_bytes(b"polygon(EMPTY)")
        self.assertEqual(self.esegui(True)[0], 0)


if __name__ == "__main__":
    unittest.main()
