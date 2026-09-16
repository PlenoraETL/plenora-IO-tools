#!/usr/bin/env python3
"""Le sonde del classificatore: che riconosca la famiglia e non di piu'.

Ogni prova guasta **una** proprieta' della firma e lascia le altre intatte: e'
il modo di far vedere che la corrispondenza e' congiunta, e che allentarla
nasconderebbe un finding nuovo dietro uno noto.
"""

from __future__ import annotations

import contextlib
import io
import json
import pathlib
import tempfile
import unittest

from scripts import classifica_finding_fuzz as gate

#: L'uscita vera della corsa che ha trovato il finding, ridotta all'essenziale.
CRASH = """==12345== ERROR: libFuzzer: deadly signal
thread '<unnamed>' panicked at \
/root/.cargo/registry/src/index.crates.io-1949cf8c/parquet-59.3.0/src/encodings/decoding/byte_stream_split_decoder.rs:61:38:
index out of bounds: the len is 2 but the index is 2
note: run with `RUST_BACKTRACE=1`
""".replace("\\\n", "")


def registro() -> dict:
    return json.loads(gate.REGISTRO.read_text(encoding="utf-8"))


class SondeDellaFirma(unittest.TestCase):
    def test_il_crash_registrato_e_riconosciuto(self) -> None:
        esito = gate.classifica("geoparquet_reader", CRASH, registro())
        self.assertEqual(esito["stato"], "noto")
        self.assertEqual(esito["id"], "arrow-rs-byte-stream-split-oob")

    def test_la_stessa_famiglia_con_altri_numeri_e_riconosciuta(self) -> None:
        # «the len is 2» e «the len is 56» sono lo stesso difetto su due
        # ingressi: e' il motivo per cui la firma riduce le cifre a `N`.
        altro = CRASH.replace("the len is 2 but the index is 2", "the len is 56 but the index is 56")
        self.assertEqual(
            gate.classifica("geoparquet_reader", altro, registro())["stato"], "noto"
        )

    def test_un_altra_versione_della_crate_e_riconosciuta(self) -> None:
        # Un difetto aperto a monte non smette di esserlo quando il pin sale.
        altro = CRASH.replace("parquet-59.3.0", "parquet-60.0.0")
        self.assertEqual(
            gate.classifica("geoparquet_reader", altro, registro())["stato"], "noto"
        )

    def test_la_firma_dice_compatibile_e_non_identico(self) -> None:
        """La garanzia e' piu' debole di quanto sembri, ed e' scritta.

        Modulo e forma del messaggio identificano un crash **compatibile** con
        una voce, non lo stesso difetto: un conteggio incoerente e un offset
        calcolato male finiscono entrambi su `index out of bounds` nello stesso
        modulo. Cio' che il classificatore dice a chi legge deve dirlo.
        """
        with tempfile.TemporaryDirectory() as temporanea:
            percorso = pathlib.Path(temporanea) / "uscita.txt"
            percorso.write_text(CRASH, encoding="utf-8")
            catturato = io.StringIO()
            with contextlib.redirect_stdout(catturato):
                gate.main(["geoparquet_reader", "--uscita", str(percorso)])
        detto = catturato.getvalue()
        self.assertIn("COMPATIBILE", detto)
        self.assertIn("non dimostra la stessa causa", detto)

    def test_un_altro_messaggio_nello_stesso_modulo_e_nuovo(self) -> None:
        altro = CRASH.replace(
            "index out of bounds: the len is 2 but the index is 2",
            "attempt to subtract with overflow",
        )
        self.assertEqual(
            gate.classifica("geoparquet_reader", altro, registro())["stato"], "nuovo"
        )

    def test_lo_stesso_messaggio_in_un_altro_modulo_e_nuovo(self) -> None:
        altro = CRASH.replace(
            "byte_stream_split_decoder.rs", "delta_byte_array_decoder.rs"
        )
        self.assertEqual(
            gate.classifica("geoparquet_reader", altro, registro())["stato"], "nuovo"
        )

    def test_lo_stesso_crash_su_un_altro_bersaglio_e_nuovo(self) -> None:
        # Una voce vale per il bersaglio che la dichiara: lo stesso modulo
        # raggiunto da un altro target e' un percorso che nessuno ha esaminato.
        self.assertEqual(
            gate.classifica("gpkg_reader", CRASH, registro())["stato"], "nuovo"
        )

    def test_senza_panico_non_c_e_crash(self) -> None:
        self.assertEqual(
            gate.classifica("geoparquet_reader", "tutto bene\n", registro())["stato"],
            "senza-crash",
        )

    def test_il_percorso_perde_versione_e_prefisso_del_registro(self) -> None:
        # La firma non deve dipendere da dove gira il fuzzer.
        normalizzato = gate.modulo_normalizzato(
            "/home/tizio/.cargo/registry/src/index.crates.io-abc/parquet-59.3.0/src/a/b.rs"
        )
        self.assertEqual(normalizzato, "parquet/src/a/b.rs")

    def test_un_modulo_nostro_resta_relativo_alla_radice(self) -> None:
        normalizzato = gate.modulo_normalizzato(
            f"/qualunque/{gate.ROOT.name}/crates/driver-shp/src/lib.rs"
        )
        self.assertEqual(normalizzato, "crates/driver-shp/src/lib.rs")


class SondeDelRegistro(unittest.TestCase):
    def test_il_registro_vero_e_ben_formato(self) -> None:
        self.assertEqual(gate.registro_ben_formato(registro()), [])

    def test_ogni_campo_e_obbligatorio(self) -> None:
        # Una voce senza `non_promette` o senza `quando_si_toglie` non e' una
        # registrazione, e' un permesso a tempo indeterminato.
        for campo in gate.CAMPI:
            with self.subTest(campo=campo):
                documento = registro()
                del documento["finding"][0][campo]
                motivi = gate.registro_ben_formato(documento)
                self.assertTrue(any(campo in m for m in motivi), motivi)

    def test_due_voci_con_lo_stesso_id_sono_rosse(self) -> None:
        documento = registro()
        documento["finding"].append(dict(documento["finding"][0]))
        motivi = gate.registro_ben_formato(documento)
        self.assertTrue(any("ripetuto" in m for m in motivi), motivi)

    def test_un_registro_senza_elenco_e_rosso(self) -> None:
        self.assertTrue(gate.registro_ben_formato({"finding": "nessuno"}))


class SondeDeiCodiciDUscita(unittest.TestCase):
    """I tre stati hanno tre codici, e lo smoke ci si appoggia."""

    def _esegui(self, bersaglio: str, testo: str) -> int:
        with tempfile.TemporaryDirectory() as temporanea:
            percorso = pathlib.Path(temporanea) / "uscita.txt"
            percorso.write_text(testo, encoding="utf-8")
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(
                io.StringIO()
            ):
                return gate.main([bersaglio, "--uscita", str(percorso)])

    def test_senza_crash_esce_zero(self) -> None:
        self.assertEqual(self._esegui("geoparquet_reader", "tutto bene\n"), 0)

    def test_un_finding_noto_esce_tre_e_non_zero(self) -> None:
        # Non zero: un noto non e' un successo. Il bersaglio si e' fermato.
        self.assertEqual(self._esegui("geoparquet_reader", CRASH), 3)

    def test_un_finding_nuovo_esce_uno(self) -> None:
        altro = CRASH.replace("index out of bounds", "attempt to divide by zero")
        self.assertEqual(self._esegui("geoparquet_reader", altro), 1)

    def test_il_noto_dice_che_il_bersaglio_si_e_fermato(self) -> None:
        # E' la frase che impedisce di leggere la corsa come completa.
        with tempfile.TemporaryDirectory() as temporanea:
            percorso = pathlib.Path(temporanea) / "uscita.txt"
            percorso.write_text(CRASH, encoding="utf-8")
            catturato = io.StringIO()
            with contextlib.redirect_stdout(catturato):
                gate.main(["geoparquet_reader", "--uscita", str(percorso)])
        self.assertIn("si e\' fermato", catturato.getvalue())
        self.assertIn("non e\' stato esplorato", catturato.getvalue())


class SondaDelCollegamento(unittest.TestCase):
    def test_lo_smoke_classifica_e_distingue_il_codice_tre(self) -> None:
        """Il collegamento non deve poter sparire in silenzio.

        Se lo smoke smettesse di invocare il classificatore, o di distinguere il
        codice 3, un finding noto tornerebbe a far fallire la corsa e l'unica
        via sarebbe di nuovo la quarantena per bersaglio.
        """
        smoke = (gate.ROOT / "scripts" / "fuzz-smoke.sh").read_text(encoding="utf-8")
        self.assertIn("classifica_finding_fuzz.py", smoke)
        self.assertIn("3) noti+=", smoke)


if __name__ == "__main__":
    unittest.main()


class SondeDellaConservazione(unittest.TestCase):
    """Ogni input si conserva, anche quando la classificazione dice «noto».

    Se fosse identita' certa si potrebbe scartare il duplicato. Essendo
    compatibilita', l'input e' l'unica cosa che permette di riesaminare la
    classificazione piu' tardi, ed e' il motivo per cui si conserva sempre.
    """

    def _corsa(self, testo: str, contenuto: bytes = b"input di prova\n"):
        temporanea = tempfile.TemporaryDirectory()
        self.addCleanup(temporanea.cleanup)
        radice = pathlib.Path(temporanea.name)
        artefatto = radice / "crash-abc"
        artefatto.write_bytes(contenuto)
        uscita = radice / "uscita.txt"
        uscita.write_text(
            testo.replace("ARTEFATTO", str(artefatto)), encoding="utf-8"
        )
        conserva = radice / "conservati"
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(
            io.StringIO()
        ):
            codice = gate.main(
                [
                    "geoparquet_reader",
                    "--uscita",
                    str(uscita),
                    "--conserva",
                    str(conserva),
                ]
            )
        return codice, conserva / "geoparquet_reader"

    CON_ARTEFATTO = CRASH + "Test unit written to ARTEFATTO\n"

    def test_un_crash_noto_conserva_input_e_referto(self) -> None:
        codice, cartella = self._corsa(self.CON_ARTEFATTO)
        self.assertEqual(codice, 3)
        self.assertEqual(len(list(cartella.glob("*.input"))), 1)
        self.assertEqual(len(list(cartella.glob("*.json"))), 1)

    def test_un_crash_nuovo_conserva_anche_lui(self) -> None:
        nuovo = self.CON_ARTEFATTO.replace(
            "index out of bounds", "attempt to divide by zero"
        )
        codice, cartella = self._corsa(nuovo)
        self.assertEqual(codice, 1)
        self.assertEqual(len(list(cartella.glob("*.input"))), 1)

    def test_il_referto_dice_che_e_compatibile_non_identico(self) -> None:
        _, cartella = self._corsa(self.CON_ARTEFATTO)
        referto = json.loads(
            next(cartella.glob("*.json")).read_text(encoding="utf-8")
        )
        self.assertEqual(referto["classificazione"], "noto")
        self.assertIn("compatibile", referto["che_cosa_significa"])
        self.assertIn("non che", referto["che_cosa_significa"])
        self.assertEqual(
            referto["finding_compatibile"], "arrow-rs-byte-stream-split-oob"
        )

    def test_il_nome_viene_dal_digest_e_non_si_duplica(self) -> None:
        # Due corse sullo stesso input scrivono lo stesso file invece di
        # accumulare copie; due input diversi non si sovrascrivono.
        _, cartella = self._corsa(self.CON_ARTEFATTO, b"identico\n")
        primo = sorted(v.name for v in cartella.iterdir())
        _, cartella = self._corsa(self.CON_ARTEFATTO, b"identico\n")
        self.assertEqual(sorted(v.name for v in cartella.iterdir()), primo)

    def test_l_artefatto_si_prende_dalla_corsa_non_dal_piu_recente(self) -> None:
        # «Il file piu' recente della directory» sarebbe l'input di un'altra
        # corsa quando due girano vicine. Senza la riga della corsa, il referto
        # dichiara di non avere l'input invece di indovinarlo.
        _, cartella = self._corsa(CRASH)
        referto = json.loads(
            next(cartella.glob("*.json")).read_text(encoding="utf-8")
        )
        self.assertIsNone(referto["input_conservato"])
        self.assertEqual(referto["artefatto_dichiarato_dalla_corsa"], "")


class SondeDellaCampagnaInterrotta(unittest.TestCase):
    """Il codice 3 resta «interrotta» fino alla qualificazione finale.

    Lo smoke esce 0 perche' lo sviluppo prosegua sugli altri bersagli. Quello 0
    non deve pero' diventare «campagna completata»: la CI e il passo
    `fuzz_smoke` del checkpoint leggono l'esito, non la riga stampata, e senza
    il verbale un'interruzione nota sarebbe indistinguibile da un successo.
    """

    def _verbale(self, **campi) -> list[str]:
        documento = {
            "revisione": gate.revisione_corrente(),
            "bersagli_dichiarati": ["shp_reader"],
            "hanno_finito": ["shp_reader"],
            "fermati_a_finding_noto": [],
            "falliti_su_finding_nuovo": [],
        }
        documento.update(campi)
        with tempfile.TemporaryDirectory() as temporanea:
            percorso = pathlib.Path(temporanea) / "verbale.json"
            percorso.write_text(json.dumps(documento), encoding="utf-8")
            return gate.verifica_campagna(percorso)

    def test_una_corsa_senza_fermate_e_completa(self) -> None:
        self.assertEqual(self._verbale(), [])

    def test_un_bersaglio_fermato_non_e_una_campagna_completa(self) -> None:
        motivi = self._verbale(fermati_a_finding_noto=["geoparquet_reader"])
        self.assertTrue(any("non e' completa" in m for m in motivi), motivi)
        self.assertTrue(any("geoparquet_reader" in m for m in motivi), motivi)

    def test_un_bersaglio_fallito_non_e_una_campagna_completa(self) -> None:
        motivi = self._verbale(falliti_su_finding_nuovo=["gpkg_reader"])
        self.assertTrue(any("non e' completa" in m for m in motivi), motivi)

    def test_nessun_bersaglio_finito_non_e_una_campagna(self) -> None:
        # Un insieme vuoto soddisfa ogni «nessuno si e' fermato».
        motivi = self._verbale(hanno_finito=[])
        self.assertTrue(motivi)

    def test_senza_verbale_la_campagna_non_e_riuscita(self) -> None:
        # L'assenza di un verbale non e' una campagna riuscita: e' l'assenza di
        # una campagna.
        with tempfile.TemporaryDirectory() as temporanea:
            percorso = pathlib.Path(temporanea) / "non-c-e.json"
            motivi = gate.verifica_campagna(percorso)
        self.assertTrue(any("assente" in m for m in motivi), motivi)

    def test_un_verbale_muto_sulle_fermate_e_rosso(self) -> None:
        motivi = self._verbale(fermati_a_finding_noto=None)
        self.assertTrue(any("non dichiara" in m for m in motivi), motivi)

    def test_un_verbale_di_un_altra_revisione_non_qualifica_questa(self) -> None:
        """La conseguenza che il verbale, da solo, non impediva.

        Una campagna vale per il codice su cui e' girata. Senza la revisione,
        un verbale **completo** di ieri qualificherebbe l'albero di oggi -- ed
        e' la stessa famiglia della misura di profondita' che porta l'impronta
        del perimetro.
        """
        motivi = self._verbale(revisione="0" * 40)
        self.assertTrue(any("non qualifica questa" in m for m in motivi), motivi)
        self.assertTrue(any("000000000000" in m for m in motivi), motivi)

    def test_un_verbale_senza_revisione_e_rosso(self) -> None:
        motivi = self._verbale(revisione=None)
        self.assertTrue(any("non dichiara la revisione" in m for m in motivi), motivi)

    def test_una_corsa_su_un_sottoinsieme_non_e_la_campagna(self) -> None:
        """«Nessuno si e' fermato» su un bersaglio solo e' vero e dice poco.

        Lo smoke sa girare su un sottoinsieme, e un target in quarantena esce
        comunque dai finiti. In nessuno dei due casi la campagna e' quella
        dichiarata, e il verbale porta `cargo fuzz list` per poterlo dire.
        """
        motivi = self._verbale(
            bersagli_dichiarati=["shp_reader", "gpkg_reader", "kml_reader"]
        )
        self.assertTrue(any("non copre i bersagli" in m for m in motivi), motivi)
        self.assertTrue(any("gpkg_reader" in m for m in motivi), motivi)

    def test_un_verbale_senza_i_bersagli_dichiarati_e_rosso(self) -> None:
        motivi = self._verbale(bersagli_dichiarati=[])
        self.assertTrue(
            any("non dichiara quali bersagli esistessero" in m for m in motivi), motivi
        )

    def test_lo_smoke_passa_il_perimetro_dichiarato(self) -> None:
        smoke = (gate.ROOT / "scripts" / "fuzz-smoke.sh").read_text(encoding="utf-8")
        self.assertIn("--dichiarati", smoke)
        self.assertIn('${dichiarati[@]+"${dichiarati[@]}"}', smoke)

    def test_la_condizione_di_rilascio_legge_quel_verbale(self) -> None:
        """La catena, per intero: registro, condizione obbligatoria, comando.

        Se la condizione sparisse dal registro, o smettesse di essere
        obbligatoria, un'interruzione nota tornerebbe a passare come campagna
        completa -- che e' precisamente cio' che il codice 3 esiste per
        impedire.
        """
        from scripts import check_release_contract as contratto

        self.assertIn("campagna-fuzz-completa", contratto.CONDIZIONI_OBBLIGATORIE)
        registro = json.loads(
            (
                gate.ROOT
                / "assurance"
                / "registries"
                / "release-contract-current.json"
            ).read_text(encoding="utf-8")
        )
        voce = next(
            c
            for c in registro["autorizzazione_di_release"]["condizioni"]
            if c["id"] == "campagna-fuzz-completa"
        )
        self.assertEqual(
            voce["verifica"]["comando"],
            [
                "python3",
                "scripts/classifica_finding_fuzz.py",
                "--verifica-campagna",
                "assurance/evidence/fuzz-smoke-ultima.json",
            ],
        )

    def test_lo_smoke_scrive_il_verbale_e_non_conta_i_falliti(self) -> None:
        smoke = (gate.ROOT / "scripts" / "fuzz-smoke.sh").read_text(encoding="utf-8")
        self.assertIn("--scrivi-verbale", smoke)
        self.assertIn("--conserva", smoke)
        # I falliti escono dai «finiti»: contarli fra i completi renderebbe il
        # verbale piu' generoso della corsa.
        self.assertIn('${failed[@]+"${failed[@]}"}; do', smoke)
