"""Sonde del gate di profondita', su **entrambi** i bersagli.

Il gate e' cio' che tiene chiusi `fuzz.reader-shapefile` e `fuzz.filegdb`: se
sbagliasse, direbbe «il reader e' esercitato» di un target che compila, non
crasha e non arriva al parser -- che e' esattamente la situazione da cui i due
blocchi venivano.

Le sonde provano le due direzioni. Che una misura completa e attuale sia verde,
e che **ogni** modo di renderla verde senza meritarselo sia rosso: requisito non
raggiunto, registro svuotato, nucleo mutilato, misura di un altro albero, misura
di un altro registro, corpus vuoto.

# Perche' ogni sonda gira su tutti i bersagli

Il motore e' uno solo, e una sonda che ne provasse il comportamento su un
formato soltanto lascerebbe l'altro senza rete proprio dove la
generalizzazione ha introdotto il rischio. `bersagli()` le fa girare su
ciascuno: le proprieta' provate restano quelle di prima, il perimetro su cui
valgono e' piu' largo.
"""

from __future__ import annotations

import io
import json
import pathlib
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout

from scripts import check_profondita_fuzz as gate

IMPRONTA = "f" * 64

SHP = gate.BERSAGLI["shp_reader"]
GDB = gate.BERSAGLI["filegdb_reader"]


# Le sonde del motore girano sul bersaglio Shapefile.
#
# Non e' una scorciatoia: sono le stesse trentaquattro sonde che verificavano il
# gate prima che diventasse generico, e tenerle su quel bersaglio e' cio' che
# permette di dire che il comportamento non e' cambiato. Il bersaglio FileGDB ha
# la propria classe piu' sotto, e `SondeSuOgniBersaglio` prova su **tutti** le
# proprieta' che la generalizzazione ha introdotto.
bersaglio = SHP


def bersagli() -> list[gate.Bersaglio]:
    """Tutti i bersagli dichiarati, non un elenco scritto qui.

    Un bersaglio nuovo entra nelle sonde senza toccarle: se non ci entrasse, la
    prima cosa che nessuno proverebbe sarebbe proprio quella appena aggiunta.
    """
    return [gate.BERSAGLI[nome] for nome in sorted(gate.BERSAGLI)]


def registro_minimo(bersaglio: gate.Bersaglio = SHP) -> dict:
    """Un registro valido, da cui le sonde tolgono un pezzo per volta."""
    return {
        "schema_version": 1,
        "target": bersaglio.nome,
        "artefatto": f"assurance/profondita-{bersaglio.nome}.json",
        "perimetro": {"percorsi": sorted(bersaglio.perimetro_obbligatorio)},
        "nucleo": sorted(bersaglio.nucleo),
        # La famiglia viene dal gate, non da una convenzione sul nome: un
        # requisito di riga che non si chiamasse «rifiuto.*» finirebbe fra le
        # funzioni, e la fixture proverebbe un registro che il gate rifiuta.
        "funzioni": [
            {
                "id": identita,
                "segmenti": ["driver_shp", identita.replace(".", "_")],
                "perche": "perche' si'",
            }
            for identita, famiglia in sorted(bersaglio.famiglia_del_nucleo.items())
            if famiglia == "funzioni"
        ],
        "righe": [
            {
                "id": identita,
                "file": "crates/driver-shp/src/lib.rs",
                "ancora": identita,
                "perche": "perche' si'",
            }
            for identita, famiglia in sorted(bersaglio.famiglia_del_nucleo.items())
            if famiglia == "righe"
        ],
    }


def misura_di(
    registro: dict, conteggio: int = 3, bersaglio: gate.Bersaglio = SHP
) -> dict:
    voci, errori = gate.requisiti(bersaglio, registro)
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
        self.assertEqual(gate.verifica(bersaglio, registro, misura_di(registro)), [])

    def test_un_requisito_non_raggiunto_e_rosso(self) -> None:
        """E' il caso per cui il gate esiste: il target gira e non arriva."""
        registro = registro_minimo()
        misura = misura_di(registro)
        misura["requisiti"][0]["conteggio"] = 0
        errori = gate.verifica(bersaglio, registro, misura)
        self.assertTrue(any("non raggiunto dal replay" in m for m in errori), errori)

    def test_un_conteggio_che_non_e_un_intero_e_rosso(self) -> None:
        registro = registro_minimo()
        misura = misura_di(registro)
        misura["requisiti"][0]["conteggio"] = True
        self.assertTrue(
            any("non e' un intero" in m for m in gate.verifica(bersaglio, registro, misura))
        )

    def test_una_funzione_senza_simboli_corrispondenti_e_rossa(self) -> None:
        """Un conteggio positivo senza simboli e' una misura di niente: la
        funzione e' stata rinominata, o non e' stata compilata nel target."""
        registro = registro_minimo()
        misura = misura_di(registro)
        funzione = next(v for v in misura["requisiti"] if v["famiglia"] == "funzione")
        funzione["simboli"] = 0
        self.assertTrue(
            any("nessun simbolo corrisponde" in m for m in gate.verifica(bersaglio, registro, misura))
        )

    def test_un_registro_svuotato_e_rosso(self) -> None:
        """Zero requisiti, zero requisiti mancati: e' il verde per assenza di
        domanda, ed e' il primo modo in cui un gate cosi' si addomestica."""
        registro = registro_minimo()
        registro["funzioni"] = []
        errori = gate.verifica(bersaglio, registro, misura_di(registro_minimo()))
        self.assertTrue(any("assente o vuota" in m for m in errori), errori)

    def test_togliere_un_requisito_dal_nucleo_e_rosso(self) -> None:
        for identita in sorted(bersaglio.nucleo):
            with self.subTest(identita):
                registro = registro_minimo()
                for famiglia in ("funzioni", "righe"):
                    registro[famiglia] = [
                        v for v in registro[famiglia] if v["id"] != identita
                    ]
                registro["nucleo"] = [n for n in registro["nucleo"] if n != identita]
                errori = gate.verifica(bersaglio, registro, misura_di(registro_minimo()))
                self.assertTrue(
                    any("nucleo" in m for m in errori),
                    f"togliere «{identita}» deve essere rosso: {errori}",
                )

    def test_spostare_un_requisito_del_nucleo_di_famiglia_e_rosso(self) -> None:
        """Un ramo del sorgente e un simbolo eseguito non sono la stessa prova.

        `prevalidazione.conteggi-del-multipunto` e' un requisito di **riga**:
        riscritto come funzione passerebbe a nome di un simbolo che esiste
        comunque, e il ramo resterebbe mai percorso con il gate verde.
        """
        for identita, famiglia in sorted(bersaglio.famiglia_del_nucleo.items()):
            with self.subTest(identita):
                registro = registro_minimo()
                altra = "righe" if famiglia == "funzioni" else "funzioni"
                registro[famiglia] = [v for v in registro[famiglia] if v["id"] != identita]
                registro[altra] = registro[altra] + [
                    {
                        "id": identita,
                        "segmenti": ["driver_shp", "inventata"],
                        "file": "crates/driver-shp/src/lib.rs",
                        "ancora": identita,
                        "perche": "perche' si'",
                    }
                ]
                errori = gate.verifica(bersaglio, registro, misura_di(registro_minimo()))
                self.assertTrue(
                    any("atteso fra le" in m for m in errori),
                    f"spostare «{identita}» deve essere rosso: {errori}",
                )

    def test_il_nucleo_e_derivato_dalle_famiglie(self) -> None:
        """Due elenchi da tenere allineati a mano divergono."""
        self.assertEqual(bersaglio.nucleo, frozenset(bersaglio.famiglia_del_nucleo))
        self.assertEqual(set(bersaglio.famiglia_del_nucleo.values()), {"funzioni", "righe"})

    def test_il_nucleo_dichiarato_deve_coincidere_con_quello_preteso(self) -> None:
        registro = registro_minimo()
        registro["nucleo"] = registro["nucleo"] + ["inventato"]
        self.assertTrue(
            any("nucleo" in m for m in gate.verifica(bersaglio, registro, misura_di(registro_minimo())))
        )

    def test_identita_ripetute_nel_registro_sono_rosse(self) -> None:
        registro = registro_minimo()
        registro["funzioni"].append(dict(registro["funzioni"][0]))
        self.assertTrue(
            any("ripetute" in m for m in gate.verifica(bersaglio, registro, misura_di(registro_minimo())))
        )

    def test_una_misura_di_un_altro_albero_e_rossa(self) -> None:
        """E' il modo in cui una misura committata invecchia: il codice cambia,
        il target smette di raggiungere, il JSON continua a dire di si'."""
        registro = registro_minimo()
        misura = misura_di(registro)
        misura["impronta_perimetro"] = "0" * 64
        errori = gate.verifica(bersaglio, registro, misura)
        self.assertTrue(any("impronta del perimetro diversa" in m for m in errori), errori)

    def test_una_misura_senza_impronta_e_rossa(self) -> None:
        registro = registro_minimo()
        misura = misura_di(registro)
        del misura["impronta_perimetro"]
        self.assertTrue(
            any("impronta del perimetro diversa" in m for m in gate.verifica(bersaglio, registro, misura))
        )

    def test_una_misura_di_un_registro_piu_piccolo_e_rossa(self) -> None:
        registro = registro_minimo()
        misura = misura_di(registro)
        tolto = misura["requisiti"].pop()
        errori = gate.verifica(bersaglio, registro, misura)
        self.assertTrue(any(tolto["id"] in m for m in errori), errori)

    def test_una_misura_che_osserva_cose_non_dichiarate_e_rossa(self) -> None:
        registro = registro_minimo()
        misura = misura_di(registro)
        misura["requisiti"].append({"id": "estraneo", "famiglia": "riga", "conteggio": 9})
        self.assertTrue(
            any("non dichiara" in m for m in gate.verifica(bersaglio, registro, misura))
        )

    def test_una_osservazione_ripetuta_e_rossa(self) -> None:
        registro = registro_minimo()
        misura = misura_di(registro)
        misura["requisiti"].append(dict(misura["requisiti"][0]))
        self.assertTrue(
            any("ripetuta" in m for m in gate.verifica(bersaglio, registro, misura))
        )

    def test_un_corpus_vuoto_e_rosso(self) -> None:
        registro = registro_minimo()
        misura = misura_di(registro)
        misura["corpus"] = {"input": 0}
        self.assertTrue(any("zero input" in m for m in gate.verifica(bersaglio, registro, misura)))

    def test_una_misura_di_un_altro_target_e_rossa(self) -> None:
        registro = registro_minimo()
        misura = misura_di(registro)
        misura["target"] = "shp_wkb"
        self.assertTrue(any("shp_wkb" in m for m in gate.verifica(bersaglio, registro, misura)))

    def test_un_perimetro_mutilato_e_rosso(self) -> None:
        for percorso in sorted(bersaglio.perimetro_obbligatorio):
            with self.subTest(percorso):
                registro = registro_minimo()
                registro["perimetro"]["percorsi"] = [
                    p for p in registro["perimetro"]["percorsi"] if p != percorso
                ]
                errori = gate.verifica(bersaglio, registro, misura_di(registro_minimo()))
                self.assertTrue(
                    any("perimetro senza" in m for m in errori),
                    f"togliere «{percorso}» deve essere rosso: {errori}",
                )

    def test_un_perimetro_assente_e_rosso(self) -> None:
        registro = registro_minimo()
        del registro["perimetro"]
        self.assertTrue(
            any("non scadrebbe mai" in m for m in gate.verifica(bersaglio, registro, misura_di(registro_minimo())))
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
        voci, errori = gate.requisiti(bersaglio, gate.leggi_registro(bersaglio))
        self.assertEqual(errori, [])
        self.assertGreaterEqual(len(voci), len(bersaglio.nucleo))

    def test_il_perimetro_seleziona_dei_file(self) -> None:
        percorsi, errori = gate.percorsi_del_perimetro(bersaglio, gate.leggi_registro(bersaglio))
        self.assertEqual(errori, [])
        impronta, problemi = gate.impronta_del_perimetro(percorsi)
        self.assertEqual(problemi, [])
        self.assertRegex(impronta, "^[0-9a-f]{64}$")

    def test_il_gate_e_verde_sull_albero_corrente(self) -> None:
        uscita, errori = io.StringIO(), io.StringIO()
        with redirect_stdout(uscita), redirect_stderr(errori):
            codice = gate.main([bersaglio.nome])
        self.assertEqual(codice, 0, errori.getvalue())


class SondeSuOgniBersaglio(unittest.TestCase):
    """Le proprieta' che la generalizzazione ha introdotto, su tutti i bersagli.

    Un motore solo con due configurazioni sposta il rischio: non piu' «il codice
    e' sbagliato», ma «la configurazione di **quel** bersaglio e' sbagliata».
    Queste sonde girano su ciascuno, cosi' il bersaglio aggiunto per ultimo non
    e' quello senza rete.
    """

    def setUp(self) -> None:
        precedente = gate.impronta_del_perimetro
        gate.impronta_del_perimetro = lambda percorsi: (IMPRONTA, [])
        self.addCleanup(setattr, gate, "impronta_del_perimetro", precedente)

    def test_una_misura_completa_e_verde_su_ogni_bersaglio(self) -> None:
        for corrente in bersagli():
            with self.subTest(corrente.nome):
                registro = registro_minimo(corrente)
                misura = misura_di(registro, bersaglio=corrente)
                self.assertEqual(gate.verifica(corrente, registro, misura), [])

    def test_la_misura_viene_davvero_consumata(self) -> None:
        """Il requisito non raggiunto deve **fermare** il gate.

        E' la proprieta' per cui la misura esiste: se un conteggio a zero
        passasse, l'artefatto sarebbe un documento che nessuno legge, e il gate
        direbbe «raggiunto» di un ramo mai percorso.
        """
        for corrente in bersagli():
            for indice in range(len(corrente.famiglia_del_nucleo)):
                with self.subTest(bersaglio=corrente.nome, requisito=indice):
                    registro = registro_minimo(corrente)
                    misura = misura_di(registro, bersaglio=corrente)
                    misura["requisiti"][indice]["conteggio"] = 0
                    errori = gate.verifica(corrente, registro, misura)
                    self.assertTrue(
                        any("non raggiunto dal replay" in m for m in errori), errori
                    )

    def test_ogni_bersaglio_ha_un_nucleo_e_un_perimetro_non_vuoti(self) -> None:
        for corrente in bersagli():
            with self.subTest(corrente.nome):
                self.assertTrue(corrente.famiglia_del_nucleo)
                self.assertTrue(corrente.perimetro_obbligatorio)
                self.assertEqual(corrente.nucleo, frozenset(corrente.famiglia_del_nucleo))
                self.assertEqual(
                    set(corrente.famiglia_del_nucleo.values()), {"funzioni", "righe"}
                )

    def test_il_registro_di_un_bersaglio_non_vale_per_un_altro(self) -> None:
        """Due registri e un motore solo: leggerne uno per il bersaglio
        sbagliato verificherebbe un formato a nome di un altro."""
        registro = registro_minimo(GDB)
        errori = gate.verifica(SHP, registro, misura_di(registro, bersaglio=GDB))
        self.assertTrue(
            any("lo ha aperto come" in m for m in errori), errori
        )

    def test_i_bersagli_non_condividono_registro_ne_artefatto(self) -> None:
        """Due bersagli che scrivessero nello stesso file si sovrascriverebbero,
        e l'ultimo a misurare cancellerebbe la prova dell'altro."""
        registri = [corrente.registro for corrente in bersagli()]
        self.assertEqual(len(set(registri)), len(registri))


class SondeDellaFormaDellaMisura(unittest.TestCase):
    """Una misura malformata deve fallire chiusa, e per «malformata» si intende
    anche cio' che somiglia a un numero senza esserlo.

    Le tre sonde qui sotto nascono da altrettanti stati che il gate accettava
    mentre l'invariante dichiarava di rifiutarli. Nessuno dei tre era visibile
    dai casi positivi: un conteggio positivo si ottiene da qualunque riga
    eseguita, e `True` e' un intero per Python.
    """

    def setUp(self) -> None:
        precedente = gate.impronta_del_perimetro
        gate.impronta_del_perimetro = lambda percorsi: (IMPRONTA, [])
        self.addCleanup(setattr, gate, "impronta_del_perimetro", precedente)

    def test_una_famiglia_scambiata_e_rossa(self) -> None:
        """Una funzione osservata come riga risponde a una domanda diversa.

        Il registro dice se un requisito e' un **ramo del sorgente** o un
        **simbolo eseguito**; la misura deve dire la stessa cosa. Senza il
        confronto restava il solo conteggio, e un conteggio positivo non
        distingue le due prove.
        """
        for corrente in bersagli():
            for indice, atteso in enumerate(
                {"funzioni": "funzione", "righe": "riga"}[f]
                for _, f in sorted(corrente.famiglia_del_nucleo.items())
            ):
                with self.subTest(bersaglio=corrente.nome, requisito=indice):
                    registro = registro_minimo(corrente)
                    misura = misura_di(registro, bersaglio=corrente)
                    voce = misura["requisiti"][indice]
                    voce["famiglia"] = "riga" if voce["famiglia"] == "funzione" else "funzione"
                    errori = gate.verifica(corrente, registro, misura)
                    self.assertTrue(
                        any("non rispondono alla stessa domanda" in m for m in errori),
                        f"scambiare la famiglia deve essere rosso: {errori}",
                    )

    def test_un_corpus_booleano_non_e_un_numero_di_input(self) -> None:
        """`bool` e' sottotipo di `int`: `true` passava per «un input»."""
        for valore in (True, False):
            with self.subTest(valore=valore):
                registro = registro_minimo()
                misura = misura_di(registro)
                misura["corpus"]["input"] = valore
                errori = gate.verifica(bersaglio, registro, misura)
                self.assertTrue(
                    any("su quanti input" in m for m in errori), errori
                )

    def test_un_conteggio_di_simboli_booleano_e_rosso(self) -> None:
        registro = registro_minimo()
        misura = misura_di(registro)
        funzione = next(v for v in misura["requisiti"] if v["famiglia"] == "funzione")
        funzione["simboli"] = True
        errori = gate.verifica(bersaglio, registro, misura)
        self.assertTrue(any("non e' un conteggio" in m for m in errori), errori)


class SondeDelPerimetroDichiarato(unittest.TestCase):
    """Il perimetro deve contenere cio' che decide **quale** binario si misura.

    I sorgenti non bastano: feature, versioni e logica di build cambiano il
    binario senza toccare una riga di codice, e una misura che sopravvivesse a
    quei cambiamenti descriverebbe qualcos'altro.
    """

    def test_ogni_bersaglio_comprende_manifesti_e_lockfile(self) -> None:
        for corrente in bersagli():
            with self.subTest(corrente.nome):
                perimetro = corrente.perimetro_obbligatorio
                self.assertIn(
                    "fuzz/Cargo.lock",
                    perimetro,
                    "il workspace di fuzzing e' detached: le versioni con cui il "
                    "target viene costruito stanno nel suo lockfile",
                )
                self.assertTrue(
                    any(p.endswith("/Cargo.toml") and p.startswith("crates/") for p in perimetro),
                    "il manifesto del driver decide quali feature entrano nel target",
                )

    def test_il_perimetro_del_filegdb_comprende_la_build_del_wrapper(self) -> None:
        """`vendor/gdal/build.rs` decide **contro quale** libreria si collega."""
        perimetro = GDB.perimetro_obbligatorio
        for atteso in ("vendor/gdal/build.rs", "vendor/gdal/Cargo.toml", "vendor/gdal/src"):
            self.assertIn(atteso, perimetro)


class SondeDelBersaglioFileGDB(unittest.TestCase):
    """Il bersaglio aggiunto con la generalizzazione, e i suoi modi di fallire.

    Le tre proprieta' che contano: la misura viene consumata, una misura
    invecchiata fallisce chiuso, una misura malformata pure. Senza la prima
    l'artefatto sarebbe ornamentale; senza le altre due sopravvivrebbe al codice
    che descrive.
    """

    def registro(self) -> dict:
        return registro_minimo(GDB)

    def test_il_registro_vero_e_ben_formato(self) -> None:
        voci, errori = gate.requisiti(GDB, gate.leggi_registro(GDB))
        self.assertEqual(errori, [])
        self.assertGreaterEqual(len(voci), len(GDB.nucleo))

    def test_il_perimetro_vero_comprende_il_wrapper_gdal(self) -> None:
        """`vendor/gdal/src` e' l'unica parte del percorso GDAL che la copertura
        vede: fuori dal perimetro, la misura sopravviverebbe a una sua
        riscrittura."""
        percorsi, errori = gate.percorsi_del_perimetro(GDB, gate.leggi_registro(GDB))
        self.assertEqual(errori, [])
        self.assertIn("vendor/gdal/src", percorsi)
        self.assertIn("fuzz/fixtures/filegdb", percorsi)

    def test_una_misura_invecchiata_fallisce_chiuso(self) -> None:
        registro = self.registro()
        misura = misura_di(registro, bersaglio=GDB)
        misura["impronta_perimetro"] = "0" * 64
        errori = gate.verifica(GDB, registro, misura)
        self.assertTrue(any("impronta del perimetro diversa" in m for m in errori), errori)
        self.assertTrue(
            any("fuzz-profondita.sh filegdb_reader" in m for m in errori),
            "il messaggio deve dire come rifarla, e per **quale** bersaglio",
        )

    def test_una_misura_malformata_fallisce_chiuso(self) -> None:
        precedente = gate.impronta_del_perimetro
        gate.impronta_del_perimetro = lambda percorsi: (IMPRONTA, [])
        self.addCleanup(setattr, gate, "impronta_del_perimetro", precedente)

        registro = self.registro()
        casi = {
            "senza requisiti": lambda m: m.update(requisiti=[]),
            "requisiti non lista": lambda m: m.update(requisiti="tutti"),
            "senza corpus": lambda m: m.pop("corpus"),
            "osservazione senza id": lambda m: m["requisiti"].append({"conteggio": 1}),
            "conteggio non intero": lambda m: m["requisiti"][0].update(conteggio="molti"),
        }
        for nome, rompi in casi.items():
            with self.subTest(nome):
                misura = misura_di(registro, bersaglio=GDB)
                rompi(misura)
                self.assertNotEqual(
                    gate.verifica(GDB, registro, misura),
                    [],
                    f"«{nome}» deve fallire chiuso",
                )

    def test_spostare_un_requisito_del_nucleo_e_rosso(self) -> None:
        for identita, famiglia in sorted(GDB.famiglia_del_nucleo.items()):
            with self.subTest(identita):
                registro = self.registro()
                altra = "righe" if famiglia == "funzioni" else "funzioni"
                registro[famiglia] = [v for v in registro[famiglia] if v["id"] != identita]
                registro[altra] = registro[altra] + [
                    {
                        "id": identita,
                        "segmenti": ["driver_filegdb", "inventata"],
                        "file": "crates/driver-filegdb/src/lib.rs",
                        "ancora": identita,
                        "perche": "perche' si'",
                    }
                ]
                errori = gate.verifica(GDB, registro, misura_di(self.registro(), bersaglio=GDB))
                self.assertTrue(
                    any("atteso fra le" in m for m in errori),
                    f"spostare «{identita}» deve essere rosso: {errori}",
                )

    def test_il_gate_e_verde_sull_albero_corrente(self) -> None:
        uscita, errori = io.StringIO(), io.StringIO()
        with redirect_stdout(uscita), redirect_stderr(errori):
            codice = gate.main(["filegdb_reader"])
        self.assertEqual(codice, 0, errori.getvalue())


if __name__ == "__main__":
    unittest.main()
