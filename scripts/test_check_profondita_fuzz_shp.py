"""Sonde del gate di profondita' del target `shp_reader`.

Il gate e' cio' che tiene chiuso `fuzz.reader-shapefile`: se sbagliasse, direbbe
«il reader e' esercitato» di un target che compila, non crasha e non arriva al
parser -- che e' esattamente la situazione da cui il blocco veniva.

Le sonde provano le due direzioni. Che una misura completa e attuale sia verde,
e che **ogni** modo di renderla verde senza meritarselo sia rosso: requisito non
raggiunto, registro svuotato, nucleo mutilato, misura di un altro albero, misura
di un altro registro, corpus vuoto.
"""

from __future__ import annotations

import io
import json
import pathlib
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout

from scripts import check_profondita_fuzz_shp as gate

IMPRONTA = "f" * 64


def registro_minimo() -> dict:
    """Un registro valido, da cui le sonde tolgono un pezzo per volta."""
    return {
        "schema_version": 1,
        "target": "shp_reader",
        "artefatto": "assurance/profondita-fuzz-shapefile.json",
        "perimetro": {"percorsi": sorted(gate.PERIMETRO_OBBLIGATORIO)},
        "nucleo": sorted(gate.NUCLEO_OBBLIGATORIO),
        "funzioni": [
            {
                "id": identita,
                "segmenti": ["driver_shp", identita.replace(".", "_")],
                "perche": "perche' si'",
            }
            for identita in sorted(gate.NUCLEO_OBBLIGATORIO)
            if not identita.startswith("rifiuto.")
        ],
        "righe": [
            {
                "id": identita,
                "file": "crates/driver-shp/src/lib.rs",
                "ancora": identita,
                "perche": "perche' si'",
            }
            for identita in sorted(gate.NUCLEO_OBBLIGATORIO)
            if identita.startswith("rifiuto.")
        ],
    }


def misura_di(registro: dict, conteggio: int = 3) -> dict:
    voci, errori = gate.requisiti(registro)
    assert not errori, errori
    return {
        "target": registro["target"],
        "corpus": {"input": 5},
        "impronta_perimetro": IMPRONTA,
        "requisiti": [
            {
                "id": voce["id"],
                "famiglia": "funzione" if voce["famiglia"] == "funzioni" else "riga",
                "simboli": 1,
                "conteggio": conteggio,
            }
            for voce in voci
        ],
    }


class SondeDellaVerifica(unittest.TestCase):
    def setUp(self) -> None:
        # L'impronta vera legge il working tree e chiama git: qui interessa che
        # il **confronto** avvenga, non come si calcola il valore. Il calcolo ha
        # le sue sonde piu' sotto.
        precedente = gate.impronta_del_perimetro
        gate.impronta_del_perimetro = lambda percorsi: (IMPRONTA, [])
        self.addCleanup(setattr, gate, "impronta_del_perimetro", precedente)

    def test_una_misura_completa_e_attuale_e_verde(self) -> None:
        registro = registro_minimo()
        self.assertEqual(gate.verifica(registro, misura_di(registro)), [])

    def test_un_requisito_non_raggiunto_e_rosso(self) -> None:
        """E' il caso per cui il gate esiste: il target gira e non arriva."""
        registro = registro_minimo()
        misura = misura_di(registro)
        misura["requisiti"][0]["conteggio"] = 0
        errori = gate.verifica(registro, misura)
        self.assertTrue(any("non raggiunto dal replay" in m for m in errori), errori)

    def test_un_conteggio_che_non_e_un_intero_e_rosso(self) -> None:
        registro = registro_minimo()
        misura = misura_di(registro)
        misura["requisiti"][0]["conteggio"] = True
        self.assertTrue(
            any("non e' un intero" in m for m in gate.verifica(registro, misura))
        )

    def test_una_funzione_senza_simboli_corrispondenti_e_rossa(self) -> None:
        """Un conteggio positivo senza simboli e' una misura di niente: la
        funzione e' stata rinominata, o non e' stata compilata nel target."""
        registro = registro_minimo()
        misura = misura_di(registro)
        funzione = next(v for v in misura["requisiti"] if v["famiglia"] == "funzione")
        funzione["simboli"] = 0
        self.assertTrue(
            any("nessun simbolo corrisponde" in m for m in gate.verifica(registro, misura))
        )

    def test_un_registro_svuotato_e_rosso(self) -> None:
        """Zero requisiti, zero requisiti mancati: e' il verde per assenza di
        domanda, ed e' il primo modo in cui un gate cosi' si addomestica."""
        registro = registro_minimo()
        registro["funzioni"] = []
        errori = gate.verifica(registro, misura_di(registro_minimo()))
        self.assertTrue(any("assente o vuota" in m for m in errori), errori)

    def test_togliere_un_requisito_dal_nucleo_e_rosso(self) -> None:
        for identita in sorted(gate.NUCLEO_OBBLIGATORIO):
            with self.subTest(identita):
                registro = registro_minimo()
                for famiglia in ("funzioni", "righe"):
                    registro[famiglia] = [
                        v for v in registro[famiglia] if v["id"] != identita
                    ]
                registro["nucleo"] = [n for n in registro["nucleo"] if n != identita]
                errori = gate.verifica(registro, misura_di(registro_minimo()))
                self.assertTrue(
                    any("nucleo" in m for m in errori),
                    f"togliere «{identita}» deve essere rosso: {errori}",
                )

    def test_il_nucleo_dichiarato_deve_coincidere_con_quello_preteso(self) -> None:
        registro = registro_minimo()
        registro["nucleo"] = registro["nucleo"] + ["inventato"]
        self.assertTrue(
            any("nucleo" in m for m in gate.verifica(registro, misura_di(registro_minimo())))
        )

    def test_identita_ripetute_nel_registro_sono_rosse(self) -> None:
        registro = registro_minimo()
        registro["funzioni"].append(dict(registro["funzioni"][0]))
        self.assertTrue(
            any("ripetute" in m for m in gate.verifica(registro, misura_di(registro_minimo())))
        )

    def test_una_misura_di_un_altro_albero_e_rossa(self) -> None:
        """E' il modo in cui una misura committata invecchia: il codice cambia,
        il target smette di raggiungere, il JSON continua a dire di si'."""
        registro = registro_minimo()
        misura = misura_di(registro)
        misura["impronta_perimetro"] = "0" * 64
        errori = gate.verifica(registro, misura)
        self.assertTrue(any("impronta del perimetro diversa" in m for m in errori), errori)

    def test_una_misura_senza_impronta_e_rossa(self) -> None:
        registro = registro_minimo()
        misura = misura_di(registro)
        del misura["impronta_perimetro"]
        self.assertTrue(
            any("impronta del perimetro diversa" in m for m in gate.verifica(registro, misura))
        )

    def test_una_misura_di_un_registro_piu_piccolo_e_rossa(self) -> None:
        registro = registro_minimo()
        misura = misura_di(registro)
        tolto = misura["requisiti"].pop()
        errori = gate.verifica(registro, misura)
        self.assertTrue(any(tolto["id"] in m for m in errori), errori)

    def test_una_misura_che_osserva_cose_non_dichiarate_e_rossa(self) -> None:
        registro = registro_minimo()
        misura = misura_di(registro)
        misura["requisiti"].append({"id": "estraneo", "famiglia": "riga", "conteggio": 9})
        self.assertTrue(
            any("non dichiara" in m for m in gate.verifica(registro, misura))
        )

    def test_una_osservazione_ripetuta_e_rossa(self) -> None:
        registro = registro_minimo()
        misura = misura_di(registro)
        misura["requisiti"].append(dict(misura["requisiti"][0]))
        self.assertTrue(
            any("ripetuta" in m for m in gate.verifica(registro, misura))
        )

    def test_un_corpus_vuoto_e_rosso(self) -> None:
        registro = registro_minimo()
        misura = misura_di(registro)
        misura["corpus"] = {"input": 0}
        self.assertTrue(any("zero input" in m for m in gate.verifica(registro, misura)))

    def test_una_misura_di_un_altro_target_e_rossa(self) -> None:
        registro = registro_minimo()
        misura = misura_di(registro)
        misura["target"] = "shp_wkb"
        self.assertTrue(any("shp_wkb" in m for m in gate.verifica(registro, misura)))

    def test_un_perimetro_mutilato_e_rosso(self) -> None:
        for percorso in sorted(gate.PERIMETRO_OBBLIGATORIO):
            with self.subTest(percorso):
                registro = registro_minimo()
                registro["perimetro"]["percorsi"] = [
                    p for p in registro["perimetro"]["percorsi"] if p != percorso
                ]
                errori = gate.verifica(registro, misura_di(registro_minimo()))
                self.assertTrue(
                    any("perimetro senza" in m for m in errori),
                    f"togliere «{percorso}» deve essere rosso: {errori}",
                )

    def test_un_perimetro_assente_e_rosso(self) -> None:
        registro = registro_minimo()
        del registro["perimetro"]
        self.assertTrue(
            any("non scadrebbe mai" in m for m in gate.verifica(registro, misura_di(registro_minimo())))
        )


class SondeDeiSegmenti(unittest.TestCase):
    """Il pattern e' costruito dai segmenti, non scritto a mano nel registro."""

    # Un simbolo v0 vero, preso dalla misura: porta i disambiguatori di crate,
    # che cambiano a ogni build e non devono comparire nel registro.
    APERTURA = (
        "_RNvXs1_Csic6bTMW85JE_10driver_shpNtB5_9ShpDriverNtNtCslQEjQUImrUs_"
        "15plenora_io_core6driver12FormatDriver4open"
    )
    SCHEMA = "_RNvCsic6bTMW85JE_10driver_shp16infer_shp_schema"

    def test_i_segmenti_trovano_il_simbolo_nonostante_i_disambiguatori(self) -> None:
        self.assertTrue(
            gate.pattern_dei_segmenti(["driver_shp", "ShpDriver", "open"]).search(
                self.APERTURA
            )
        )

    def test_i_segmenti_sono_lunghezza_e_nome(self) -> None:
        """E' la codifica v0: `10driver_shp`. Cercare `driver_shp` e basta
        troverebbe anche `12driver_shpx`, cioe' un'altra funzione."""
        self.assertTrue(
            gate.pattern_dei_segmenti(["infer_shp_schema"]).search(self.SCHEMA)
        )
        self.assertIsNone(
            gate.pattern_dei_segmenti(["infer_shp_schem"]).search(self.SCHEMA)
        )

    def test_l_ordine_dei_segmenti_conta(self) -> None:
        self.assertIsNone(
            gate.pattern_dei_segmenti(["ShpDriver", "driver_shp", "open"]).search(
                self.APERTURA
            )
        )

    def test_un_nome_che_non_c_e_non_si_trova(self) -> None:
        self.assertIsNone(
            gate.pattern_dei_segmenti(["driver_shp", "ShpDriver", "close"]).search(
                self.APERTURA
            )
        )


class SondeDelleAncore(unittest.TestCase):
    """Un'ancora individua un ramo, o non individua niente."""

    def setUp(self) -> None:
        temporanea = tempfile.TemporaryDirectory()
        self.addCleanup(temporanea.cleanup)
        self.radice = pathlib.Path(temporanea.name)
        precedente = gate.ROOT
        gate.ROOT = self.radice
        self.addCleanup(setattr, gate, "ROOT", precedente)

        self.sorgente = self.radice / "sorgente.rs"
        self.sorgente.write_text(
            "fn a() {\n"
            '    rifiuta("messaggio");\n'
            "}\n"
            "#[cfg(test)]\n"
            "mod tests {\n"
            '    assert!(x.contains("messaggio"));\n'
            "}\n",
            encoding="utf-8",
            newline="\n",
        )

    def test_trova_la_riga_strumentata(self) -> None:
        numero, errori = gate.riga_dell_ancora("sorgente.rs", '"messaggio"', {2: 4})
        self.assertEqual((numero, errori), (2, []))

    def test_le_righe_di_un_mod_tests_non_confondono(self) -> None:
        """Il binario di fuzzing non compila `#[cfg(test)]`, quindi quelle righe
        non hanno dati di copertura: e' cio' che permette di usare come ancora
        un messaggio che il codice condivide con la sua sonda."""
        numero, errori = gate.riga_dell_ancora("sorgente.rs", "messaggio", {2: 1})
        self.assertEqual((numero, errori), (2, []))

    def test_un_ancora_ambigua_e_rossa(self) -> None:
        _, errori = gate.riga_dell_ancora("sorgente.rs", "messaggio", {2: 1, 6: 1})
        self.assertTrue(any("ambigua" in m for m in errori), errori)

    def test_un_ancora_scomparsa_e_rossa(self) -> None:
        _, errori = gate.riga_dell_ancora("sorgente.rs", "sparito", {2: 1})
        self.assertTrue(any("non esiste piu'" in m for m in errori), errori)

    def test_un_ancora_su_nessuna_riga_strumentata_e_rossa(self) -> None:
        """La distinzione conta: «il ramo non e' coperto» e «la misura riguarda
        un altro albero» sono due diagnosi diverse."""
        _, errori = gate.riga_dell_ancora("sorgente.rs", "messaggio", {})
        self.assertTrue(any("nessuna riga strumentata" in m for m in errori), errori)


class SondeDellaLettura(unittest.TestCase):
    def test_lcov_da_righe_per_file(self) -> None:
        lcov = "SF:/work/a.rs\nDA:1,3\nDA:2,0\nend_of_record\nSF:/work/b.rs\nDA:7,1\n"
        self.assertEqual(
            gate.righe_coperte(lcov),
            {"/work/a.rs": {1: 3, 2: 0}, "/work/b.rs": {7: 1}},
        )

    def test_un_export_senza_funzioni_non_e_una_misura(self) -> None:
        with self.assertRaises(gate.RegistroMalformato):
            gate.funzioni_coperte({"data": [{"files": []}]})

    def test_di_due_istanze_dello_stesso_simbolo_vale_la_maggiore(self) -> None:
        """`llvm-cov` elenca una funzione generica una volta per istanziazione:
        prendere l'ultima farebbe dipendere l'esito dall'ordine del file."""
        export = {
            "data": [
                {
                    "functions": [
                        {"name": "s", "count": 7},
                        {"name": "s", "count": 0},
                    ]
                }
            ]
        }
        self.assertEqual(gate.funzioni_coperte(export), {"s": 7})


class SondaDelRegistroVero(unittest.TestCase):
    """Il registro committato e la misura committata, letti come in CI."""

    def test_il_registro_e_ben_formato(self) -> None:
        voci, errori = gate.requisiti(gate.leggi_registro())
        self.assertEqual(errori, [])
        self.assertGreaterEqual(len(voci), len(gate.NUCLEO_OBBLIGATORIO))

    def test_il_perimetro_seleziona_dei_file(self) -> None:
        percorsi, errori = gate.percorsi_del_perimetro(gate.leggi_registro())
        self.assertEqual(errori, [])
        impronta, problemi = gate.impronta_del_perimetro(percorsi)
        self.assertEqual(problemi, [])
        self.assertRegex(impronta, "^[0-9a-f]{64}$")

    def test_il_gate_e_verde_sull_albero_corrente(self) -> None:
        uscita, errori = io.StringIO(), io.StringIO()
        with redirect_stdout(uscita), redirect_stderr(errori):
            codice = gate.main([])
        self.assertEqual(codice, 0, errori.getvalue())


if __name__ == "__main__":
    unittest.main()
