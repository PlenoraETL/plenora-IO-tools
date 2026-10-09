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
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
import unittest.mock

from scripts import classifica_finding_fuzz as gate

#: L'uscita vera della corsa che ha trovato il finding, ridotta all'essenziale.
CRASH = """==12345== ERROR: libFuzzer: deadly signal
thread '<unnamed>' panicked at \
/home/runner/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/parquet-59.3.0/src/encodings/decoding/byte_stream_split_decoder.rs:61:38:
index out of bounds: the len is 2 but the index is 2
note: run with `RUST_BACKTRACE=1`
""".replace("\\\n", "")


#: La provenienza della corsa vera: la radice del registro di cargo e il
#: commit della toolchain del fuzzing. Lo smoke li passa al classificatore.
RADICE = "/home/runner/.cargo/registry/src"
COMMIT = "87e5904f5eb6398af6b22eac2802c78934260c48"
CONTESTO = gate.Contesto(RADICE, COMMIT)
PROVENIENZA = ["--radice-registry", RADICE, "--rustc-commit", COMMIT]


def registro() -> dict:
    return json.loads(gate.REGISTRO.read_text(encoding="utf-8"))


class SondeDellaFirma(unittest.TestCase):
    def test_il_crash_registrato_e_riconosciuto(self) -> None:
        esito = gate.classifica("geoparquet_reader", CRASH, registro(), contesto=CONTESTO)
        self.assertEqual(esito["stato"], "noto")
        self.assertEqual(esito["id"], "arrow-rs-byte-stream-split-oob")

    def test_la_stessa_famiglia_con_altri_numeri_e_riconosciuta(self) -> None:
        # «the len is 2» e «the len is 56» sono lo stesso difetto su due
        # ingressi: e' il motivo per cui la firma riduce le cifre a `N`.
        altro = CRASH.replace("the len is 2 but the index is 2", "the len is 56 but the index is 56")
        self.assertEqual(
            gate.classifica("geoparquet_reader", altro, registro(), contesto=CONTESTO)["stato"], "noto"
        )

    def test_un_altra_versione_della_crate_non_e_riconosciuta(self) -> None:
        # La firma vale per la versione che il lockfile del fuzz fissa: con un
        # aggiornamento della crate la voce va rivalidata, non ereditata.
        altro = CRASH.replace("parquet-59.3.0", "parquet-60.0.0")
        self.assertEqual(
            gate.classifica("geoparquet_reader", altro, registro(), contesto=CONTESTO)["stato"], "nuovo"
        )

    def test_senza_provenienza_un_panico_non_e_noto(self) -> None:
        self.assertEqual(
            gate.classifica("geoparquet_reader", CRASH, registro())["stato"], "nuovo"
        )

    def test_un_panico_senza_segnale_mortale_e_illeggibile(self) -> None:
        senza = CRASH.split("\n", 1)[1]
        self.assertEqual(
            gate.classifica("geoparquet_reader", senza, registro(), contesto=CONTESTO)["stato"],
            "illeggibile",
        )

    def test_due_panici_sono_nuovi(self) -> None:
        doppio = CRASH + CRASH.split("\n", 1)[1]
        self.assertEqual(
            gate.classifica("geoparquet_reader", doppio, registro(), contesto=CONTESTO)["stato"],
            "nuovo",
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
                gate.main(["geoparquet_reader", "--uscita", str(percorso), *PROVENIENZA])
        detto = catturato.getvalue()
        self.assertIn("COMPATIBILE", detto)
        self.assertIn("non dimostra la stessa causa", detto)

    def test_un_altro_messaggio_nello_stesso_modulo_e_nuovo(self) -> None:
        altro = CRASH.replace(
            "index out of bounds: the len is 2 but the index is 2",
            "attempt to subtract with overflow",
        )
        self.assertEqual(
            gate.classifica("geoparquet_reader", altro, registro(), contesto=CONTESTO)["stato"], "nuovo"
        )

    def test_lo_stesso_messaggio_in_un_altro_modulo_e_nuovo(self) -> None:
        altro = CRASH.replace(
            "byte_stream_split_decoder.rs", "delta_byte_array_decoder.rs"
        )
        self.assertEqual(
            gate.classifica("geoparquet_reader", altro, registro(), contesto=CONTESTO)["stato"], "nuovo"
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
                return gate.main([bersaglio, "--uscita", str(percorso), *PROVENIENZA])

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
                gate.main(["geoparquet_reader", "--uscita", str(percorso), *PROVENIENZA])
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
        self.assertIn('3)\n            noti+=("${target}")', smoke)


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
                    *PROVENIENZA,
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
            "illeggibili": [],
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

    def test_un_verbale_senza_illeggibili_e_rosso(self) -> None:
        """Uno smoke che non scriveva gli illeggibili contava finito anche un
        bersaglio uscito con 0 senza aver girato: il suo verbale non qualifica."""
        motivi = self._verbale(illeggibili=None)
        self.assertTrue(motivi)

    def test_un_bersaglio_illeggibile_non_e_una_campagna_completa(self) -> None:
        motivi = self._verbale(illeggibili=["shp_reader"])
        self.assertTrue(motivi)

    def test_un_verbale_muto_sulle_fermate_e_rosso(self) -> None:
        motivi = self._verbale(fermati_a_finding_noto=None)
        self.assertTrue(any("non dichiara" in m for m in motivi), motivi)

    def test_un_verbale_di_un_altra_revisione_non_qualifica_questa(self) -> None:
        """La conseguenza che il verbale, da solo, non impediva.

        Una campagna vale per il codice su cui e' girata. Senza la revisione,
        un verbale **completo** di ieri qualificherebbe l'albero di oggi -- ed
        e' la stessa famiglia della misura di profondita' che porta l'impronta
        del perimetro.

        La regola non e' piu' l'uguaglianza con HEAD -- che rendeva il verbale
        impossibile da registrare, perche' committarlo sposta HEAD. E'
        discendenza **piu'** diff dentro l'allowlist, e vive in
        `check_release_contract.evidenza_ancora_valida` insieme all'allowlist
        stessa, che non si duplica. Qui si prova che la campagna la **usa** e
        che il motivo nomina il verbale; le quattro direzioni della regola hanno
        le proprie regressioni dove la regola sta.
        """
        motivi = self._verbale(revisione="0" * 40)
        self.assertTrue(motivi)
        self.assertTrue(any("000000000000" in m for m in motivi), motivi)
        self.assertTrue(
            any(m.startswith("verbale.json:") for m in motivi), motivi
        )

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
        self.assertIn('${failed[@]+"${failed[@]}"} ${illeggibili[@]+"${illeggibili[@]}"}; do', smoke)


class SondeDelVerbaleFuoriDallAlbero(unittest.TestCase):
    """Il verbale della corsa si scrive dove la corsa dice, e il checkpoint
    consuma **il proprio**.

    # Il difetto che queste sonde chiudono

    Lo smoke e' un passo del livello 2, e il verbale e' un file **tracciato**.
    Scriverlo durante la corsa fa cambiare l'albero che la corsa sta
    verificando, e `albero_invariato` diventa rosso: un checkpoint che modifica
    l'albero che qualifica non qualifica niente. Si e' visto alla prima corsa
    completa, dopo che il verbale era stato tracciato per renderlo registrabile.

    Cambia il **momento** della registrazione, non il requisito: la campagna
    resta completa e attribuita alla revisione eseguita, e a pubblicarla
    nell'albero e' `registra-evidenza-s9.py`, dopo.
    """

    def _scrivi(self, destinazione: pathlib.Path | None) -> None:
        with contextlib.ExitStack() as pila:
            if destinazione is not None:
                pila.enter_context(
                    unittest.mock.patch.dict(
                        "os.environ", {gate.VARIABILE_VERBALE: str(destinazione)}
                    )
                )
            with contextlib.redirect_stdout(io.StringIO()):
                gate.scrivi_verbale(
                    60, ["shp_reader"], [], [], ["shp_reader"]
                )

    def test_la_corsa_scrive_fuori_dall_albero(self) -> None:
        """Produzione esterna: il file nasce dove la variabile dice."""
        with tempfile.TemporaryDirectory() as temporanea:
            fuori = pathlib.Path(temporanea) / "corsa" / "fuzz-smoke-ultima.json"
            self._scrivi(fuori)
            self.assertTrue(fuori.is_file(), "il verbale non e' stato scritto fuori")
            documento = json.loads(fuori.read_text(encoding="utf-8"))
            self.assertEqual(documento["hanno_finito"], ["shp_reader"])

    def test_l_albero_resta_invariato(self) -> None:
        """La proprieta' per cui la variabile esiste.

        Il file versionato non viene toccato: e' cio' che tiene verde
        `albero_invariato` mentre la campagna gira dentro il checkpoint.
        """
        prima = (
            gate.VERBALE.read_bytes() if gate.VERBALE.exists() else None
        )
        with tempfile.TemporaryDirectory() as temporanea:
            self._scrivi(pathlib.Path(temporanea) / "fuzz-smoke-ultima.json")
        dopo = gate.VERBALE.read_bytes() if gate.VERBALE.exists() else None
        self.assertEqual(prima, dopo, "la corsa ha toccato il verbale versionato")

    def test_senza_variabile_scrive_dove_vive_l_evidenza(self) -> None:
        """La via ordinaria non cambia: fuori dal checkpoint il verbale sta
        dove lo stato lo cita.

        L'ambiente si **azzera**, invece di darlo per azzerato. Dentro il
        checkpoint la variabile e' esportata per tutta la corsa: una sonda che
        la desse per assente misurerebbe l'ambiente in cui gira invece della
        proprieta' che verifica, e sarebbe verde da sola e rossa li'. E' il
        difetto che questa riga ha avuto, e a mostrarlo e' stato il livello 2.
        """
        with unittest.mock.patch.dict(os.environ):
            os.environ.pop(gate.VARIABILE_VERBALE, None)
            self.assertEqual(gate.percorso_del_verbale(), gate.VERBALE)

    def _verifica(self, documento, revisione_attesa, crea=True) -> list[str]:
        with tempfile.TemporaryDirectory() as temporanea:
            percorso = pathlib.Path(temporanea) / "fuzz-smoke-ultima.json"
            if crea:
                percorso.write_text(json.dumps(documento), encoding="utf-8")
            return gate.verifica_campagna(percorso, revisione_attesa)

    def _completo(self, revisione: str) -> dict:
        return {
            "revisione": revisione,
            "bersagli_dichiarati": ["shp_reader"],
            "secondi_per_bersaglio": 60,
            "hanno_finito": ["shp_reader"],
            "fermati_a_finding_noto": [],
            "falliti_su_finding_nuovo": [],
            "illeggibili": [],
        }

    def test_il_verbale_della_corsa_passa(self) -> None:
        self.assertEqual(self._verifica(self._completo("a" * 40), "a" * 40), [])

    def test_un_verbale_mancante_e_rosso(self) -> None:
        """Assente non e' «riuscita»: e' l'assenza di una campagna."""
        motivi = self._verifica(None, "a" * 40, crea=False)
        self.assertTrue(any("assente" in m for m in motivi), motivi)

    def test_un_verbale_precedente_non_vale_per_questa_corsa(self) -> None:
        """Il ripiego che il passo esiste per impedire.

        Con la regola del rilascio -- discendenza piu' diff ammessa -- il
        verbale di un antenato passerebbe. Dentro il checkpoint la domanda e'
        un'altra: «e' il verbale di **questa** corsa?», e li' l'uguaglianza e'
        la risposta giusta.
        """
        motivi = self._verifica(self._completo("b" * 40), "a" * 40)
        self.assertTrue(any("non ripiega" in m for m in motivi), motivi)
        self.assertTrue(any("bbbbbbbbbbbb" in m for m in motivi), motivi)

    def test_una_campagna_interrotta_resta_incompleta(self) -> None:
        """La completezza si legge dal **contenuto**, non dall'exit code.

        Lo smoke esce 0 anche quando un bersaglio si e' fermato a un finding
        noto: dedurre la completezza da quello 0 renderebbe il livello 2 piu'
        generoso del gate del rilascio.
        """
        documento = self._completo("a" * 40)
        documento["fermati_a_finding_noto"] = ["geoparquet_reader"]
        documento["hanno_finito"] = []
        motivi = self._verifica(documento, "a" * 40)
        self.assertTrue(any("non e' completa" in m for m in motivi), motivi)
        self.assertTrue(any("geoparquet_reader" in m for m in motivi), motivi)

    def test_il_checkpoint_consuma_il_verbale_della_corsa(self) -> None:
        """Il legame fra lo script e questa regola, perche' non si sciolga.

        Il passo deve leggere il file della corsa e pretendere la revisione
        misurata: senza l'uno o l'altro, un verbale gia' versionato passerebbe
        al posto suo.
        """
        script = (gate.ROOT / "scripts" / "s9-checkpoint.sh").read_text(
            encoding="utf-8"
        )
        self.assertIn(f"export {gate.VARIABILE_VERBALE}=", script)
        self.assertIn("--verifica-campagna", script)
        self.assertIn("--revisione-della-corsa", script)


#: L'uscita di libFuzzer sull'input di 3966 byte, **com'e'**: intestazione,
#: stack simbolizzato e coda, presi dalla corsa di CI 37735627087 (i frame fino
#: al #28, che la corsa ha stampato). Prima i frame di chi riporta l'errore --
#: il sanitizer e libFuzzer, che stampa da dentro il proprio gancio su
#: `malloc` --, poi `malloc`, poi la libreria standard che alloca, poi
#: `read_thrift_vec`, inline, col solo nome.
ESAURIMENTO = (
    gate.ROOT / "scripts" / "fixtures" / "fuzz" / "oom-parquet-footer.txt"
).read_text(encoding="utf-8")

#: La riga del primo frame utile, nella forma inline della corsa vera.
PRIMO_UTILE = (
    "in read_thrift_vec<parquet::file::metadata::KeyValue, "
    "parquet::parquet_thrift::ThriftSliceInputProtocol> "
)


def _frame(numero: int) -> str:
    """La riga del frame `#numero` del campione."""
    return next(
        riga for riga in ESAURIMENTO.splitlines() if riga.lstrip().startswith(f"#{numero} ")
    )


class SondeDellEsaurimento(unittest.TestCase):
    """La seconda firma: stretta come la prima, e illeggibile o nuova dove
    l'uscita non ha la forma attesa."""

    def _stato(self, testo: str, bersaglio: str = "geoparquet_reader") -> str:
        return gate.classifica(bersaglio, testo, registro(), contesto=CONTESTO)["stato"]

    def test_l_esaurimento_registrato_e_riconosciuto(self) -> None:
        esito = gate.classifica("geoparquet_reader", ESAURIMENTO, registro(), contesto=CONTESTO)
        self.assertEqual(esito["stato"], "noto")
        self.assertEqual(esito["id"], "parquet-footer-lista-thrift-oom")
        self.assertEqual(esito["osservato"]["tipo"], "esaurimento-memoria")
        self.assertEqual(esito["osservato"]["funzione"], "read_thrift_vec")
        self.assertEqual(esito["osservato"]["modulo"], "parquet/src/parquet_thrift.rs")

    def test_un_altra_dimensione_e_un_frame_non_inline_sono_la_stessa_famiglia(self) -> None:
        for nome in (
            "parquet::parquet_thrift::read_thrift_vec::h4444444444444444",
            "parquet::parquet_thrift::read_thrift_vec::<parquet::file::metadata::KeyValue, "
            "parquet::parquet_thrift::ThriftSliceInputProtocol>",
        ):
            altro = ESAURIMENTO.replace("malloc(2315255472)", "malloc(48000000000)").replace(
                PRIMO_UTILE, f"in {nome} "
            )
            self.assertEqual(self._stato(altro), "noto", nome)

    def test_un_altra_funzione_nello_stesso_modulo_e_nuova(self) -> None:
        altro = ESAURIMENTO.replace(PRIMO_UTILE, "in read_bytes_owned ")
        self.assertEqual(self._stato(altro), "nuovo")

    def test_la_stessa_funzione_in_un_altro_modulo_e_nuova(self) -> None:
        altro = ESAURIMENTO.replace(
            "parquet-59.3.0/src/parquet_thrift.rs:724:19",
            "parquet-59.3.0/src/file/page_index/index_reader.rs:88:5",
        )
        self.assertEqual(self._stato(altro), "nuovo")

    def test_la_variante_dei_row_group_resta_nuova(self) -> None:
        """La stessa prenotazione dei row group non passa da `read_thrift_vec`:
        il primo frame utile e' `parquet_metadata_from_bytes`, che nessuna voce
        registra. Non e' stata osservata, e resta rossa."""
        senza = [riga for riga in ESAURIMENTO.splitlines() if "read_thrift_vec" not in riga]
        # I numeri dei frame restano consecutivi.
        rinumerate = []
        numero = 0
        for riga in senza:
            if riga.lstrip().startswith("#"):
                resto = riga.lstrip().split(" ", 1)[1]
                riga = f"    #{numero} {resto}"
                numero += 1
            rinumerate.append(riga)
        esito = gate.classifica("geoparquet_reader", "\n".join(rinumerate), registro(), contesto=CONTESTO)
        self.assertEqual(esito["stato"], "nuovo")
        self.assertEqual(esito["osservato"]["funzione"], "parquet_metadata_from_bytes")

    def test_lo_stesso_esaurimento_su_un_altro_bersaglio_e_nuovo(self) -> None:
        self.assertEqual(self._stato(ESAURIMENTO, "ipc_reader"), "nuovo")

    def test_un_panico_nello_stesso_punto_non_e_l_esaurimento(self) -> None:
        panico = (
            "==1== ERROR: libFuzzer: deadly signal\n"
            "thread '<unnamed>' panicked at /home/runner/.cargo/registry/src/"
            "index.crates.io-1949cf8c6b5b557f/parquet-59.3.0/src/parquet_thrift.rs:724:19:\n"
            "out-of-memory (malloc(2315255472))\n"
        )
        self.assertEqual(self._stato(panico), "nuovo")

    # --- i controesempi della revisione: ognuno era un falso «noto» o un
    # --- «senza crash» con il classificatore precedente.

    def test_una_funzione_che_comincia_per_malloc_non_e_l_allocatore(self) -> None:
        """`malloc_buffer` non e' `malloc`: lo skip per prefisso lo saltava, e
        la firma passava al chiamante."""
        altro = ESAURIMENTO.replace(
            "    #18 0x5560ef18a922 " + PRIMO_UTILE.rstrip(),
            "    #18 0x5560ef18a922 in malloc_buffer "
            "/home/runner/work/plenora-IO-tools/plenora-IO-tools/crates/driver-geoparquet/src/lib.rs:10:5\n"
            "    #19 0x5560ef18a922 " + PRIMO_UTILE.rstrip(),
        )
        altro = _rinumera(altro)
        esito = gate.classifica("geoparquet_reader", altro, registro(), contesto=CONTESTO)
        self.assertEqual(esito["stato"], "nuovo")
        self.assertEqual(esito["osservato"]["funzione"], "malloc_buffer")

    def test_un_percorso_che_contiene_compiler_rt_non_e_il_runtime(self) -> None:
        """Un checkout in una cartella che contiene `compiler-rt` non fa di
        codice nostro un frame del sanitizer."""
        altro = ESAURIMENTO.replace(
            "    #18 0x5560ef18a922 " + PRIMO_UTILE.rstrip(),
            "    #18 0x5560ef18a922 in alloca_qui "
            "/home/compiler-rt/plenora-IO-tools/crates/driver-geoparquet/src/lib.rs:10:5\n"
            "    #19 0x5560ef18a922 " + PRIMO_UTILE.rstrip(),
        )
        esito = gate.classifica("geoparquet_reader", _rinumera(altro), registro(), contesto=CONTESTO)
        self.assertEqual(esito["stato"], "nuovo")
        self.assertEqual(esito["osservato"]["funzione"], "alloca_qui")

    def test_un_frame_senza_indirizzo_e_illeggibile(self) -> None:
        altro = ESAURIMENTO.replace(_frame(12), "    #12 in allocate /rustc/x/alloc.rs:1:1")
        self.assertEqual(self._stato(altro), "illeggibile")

    def test_un_frame_senza_numero_e_illeggibile(self) -> None:
        altro = ESAURIMENTO.replace(_frame(12), _frame(12).replace("#12 ", ""))
        self.assertEqual(self._stato(altro), "illeggibile")

    def test_un_frame_senza_file_prima_del_primo_utile_e_illeggibile(self) -> None:
        """Senza file si salta solo un simbolo **esatto** del runtime
        (`malloc (/bin/x+0x1)`). `malloc_qui (/bin/x+0x1)` comincia come
        `malloc` ma non lo e': prima del primo utile ferma la firma, mentre
        il classificatore precedente lo saltava per prefisso."""
        altro = ESAURIMENTO.replace(
            "    #18 0x5560ef18a922 " + PRIMO_UTILE.rstrip(),
            "    #18 0x5560ef18a922 in malloc_qui (/bin/x+0x1)\n"
            "    #19 0x5560ef18a922 " + PRIMO_UTILE.rstrip(),
        )
        self.assertEqual(self._stato(_rinumera(altro)), "illeggibile")

    def test_un_intestazione_con_un_suffisso_non_e_l_esaurimento(self) -> None:
        altro = ESAURIMENTO.replace(
            "out-of-memory (malloc(2315255472))",
            "out-of-memory (malloc(2315255472)) e altro",
        )
        self.assertEqual(self._stato(altro), "nuovo")

    def test_un_timeout_dopo_il_summary_non_e_il_finding_registrato(self) -> None:
        """Due errori nello stesso log non sono il finding registrato: lo
        stack si legge fino a `SUMMARY:`, ma il secondo marcatore resta."""
        altro = ESAURIMENTO + "==42463== ERROR: libFuzzer: timeout after 15 seconds\n"
        self.assertEqual(self._stato(altro), "nuovo")

    def test_un_leak_e_nuovo_e_mai_senza_crash(self) -> None:
        leak = (
            "==7==ERROR: LeakSanitizer: detected memory leaks\n\n"
            "Direct leak of 64 byte(s) in 1 object(s) allocated from:\n"
            "    #0 0x1 in malloc (/bin/x+0x1)\n\n"
            "SUMMARY: AddressSanitizer: 64 byte(s) leaked in 1 allocation(s).\n"
        )
        self.assertEqual(self._stato(leak), "nuovo")

    def test_un_errore_di_address_sanitizer_e_nuovo(self) -> None:
        asan = "==7==ERROR: AddressSanitizer: heap-buffer-overflow on address 0x1\n"
        self.assertEqual(self._stato(asan), "nuovo")

    def test_l_esaurimento_sull_rss_e_nuovo(self) -> None:
        rss = (
            "==1== ERROR: libFuzzer: out-of-memory (used: 2100Mb; exceeds: 2048Mb)\n"
            "   To change the out-of-memory limit use -rss_limit_mb=<N>\n"
            "SUMMARY: libFuzzer: out-of-memory\n"
        )
        self.assertEqual(self._stato(rss), "nuovo")

    def test_uno_stack_non_simbolizzato_e_illeggibile(self) -> None:
        """L'uscita vera della CI senza `llvm-symbolizer`: indirizzi senza
        nomi. Senza nomi non c'e' firma, e un crash senza firma non e' noto."""
        crudo = "\n".join(
            riga.split(" in ", 1)[0]
            + " (/home/runner/fuzz/target/release/geoparquet_reader+0x17ba1d1)"
            if riga.lstrip().startswith("#")
            else riga
            for riga in ESAURIMENTO.splitlines()
        )
        self.assertEqual(self._stato(crudo), "illeggibile")

    def test_un_uscita_diversa_da_zero_senza_marcatori_e_illeggibile(self) -> None:
        esito = gate.classifica(
            "geoparquet_reader", "build fallita\n", registro(), codice_uscita=101
        )
        self.assertEqual(esito["stato"], "illeggibile")
        self.assertEqual(
            gate.classifica("geoparquet_reader", "tutto bene\n", registro(), 0)["stato"],
            "senza-crash",
        )

    def test_illeggibile_esce_quattro(self) -> None:
        crudo = ESAURIMENTO.split("    #0", 1)[0]
        with tempfile.TemporaryDirectory() as temporanea:
            percorso = pathlib.Path(temporanea) / "uscita.txt"
            percorso.write_text(crudo, encoding="utf-8")
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(
                io.StringIO()
            ):
                self.assertEqual(
                    gate.main(["geoparquet_reader", "--uscita", str(percorso)]), 4
                )

    def test_il_noto_scrive_l_id_della_voce(self) -> None:
        with tempfile.TemporaryDirectory() as temporanea:
            radice = pathlib.Path(temporanea)
            uscita = radice / "uscita.txt"
            uscita.write_text(ESAURIMENTO, encoding="utf-8")
            voce = radice / "voce.txt"
            with contextlib.redirect_stdout(io.StringIO()):
                codice = gate.main(
                    [
                        "geoparquet_reader",
                        "--uscita",
                        str(uscita),
                        "--voce-nota",
                        str(voce),
                        *PROVENIENZA,
                    ]
                )
            self.assertEqual(codice, 3)
            self.assertEqual(
                voce.read_text(encoding="utf-8").strip(),
                "parquet-footer-lista-thrift-oom",
            )

    # --- il terzo giro della revisione --------------------------------------

    def test_i_frame_senza_nome_della_coda_vera_sono_righe_di_stack(self) -> None:
        """La coda vera dello stack porta frame senza nome con due spazi davanti
        al modulo (`libc`), e frame con il solo nome del modulo (`__rust_try`):
        sono righe di stack valide, e la firma resta quella del frame utile."""
        coda = (
            "    #29 0x55af77e04623 in __rust_try driver_geoparquet.b2f642e7786b642f-cgu.0
"
            "    #30 0x7fd13d02a1c9  (/lib/x86_64-linux-gnu/libc.so.6+0x2a1c9) "
            "(BuildId: a4a7992a8e66555c8141ab2a08a8465ff6e0ea65)
"
            "    #31 0x7fd13d02a28a in __libc_start_main (/lib/x86_64-linux-gnu/libc.so.6+0x2a28a) "
            "(BuildId: a4a7992a8e66555c8141ab2a08a8465ff6e0ea65)
"
        )
        altro = ESAURIMENTO.replace(_frame(28) + "
", _frame(28) + "
" + coda)
        self.assertEqual(self._stato(altro), "noto")

    def test_una_coda_malformata_dopo_il_frame_utile_e_illeggibile(self) -> None:
        """Lo stack si valida tutto prima di sceglierne un frame: una riga fuori
        forma dopo il primo utile non si ignora."""
        altro = ESAURIMENTO.replace(_frame(25), _frame(25).replace("#25 ", "#25"))
        self.assertEqual(self._stato(altro), "illeggibile")
        buco = ESAURIMENTO.replace(_frame(25) + "\n", "")
        self.assertEqual(self._stato(buco), "illeggibile")

    def test_due_segnali_mortali_con_un_panico_noto_sono_nuovi(self) -> None:
        doppio = (
            "==12345== ERROR: libFuzzer: deadly signal\n"
            + CRASH.split("\n", 1)[1]
            + "==12345== ERROR: libFuzzer: deadly signal\n"
        )
        self.assertEqual(
            gate.classifica("geoparquet_reader", CRASH, registro(), contesto=CONTESTO)["stato"],
            "noto",
        )
        self.assertEqual(
            gate.classifica("geoparquet_reader", doppio, registro(), contesto=CONTESTO)["stato"],
            "nuovo",
        )

    def test_parquet_di_un_altra_versione_non_e_noto(self) -> None:
        """La firma porta la versione dal percorso del registro, confrontata con
        il pin del lockfile del fuzz: con un bump la voce va rivalidata."""
        altro = ESAURIMENTO.replace("parquet-59.3.0", "parquet-60.0.0")
        esito = gate.classifica("geoparquet_reader", altro, registro(), contesto=CONTESTO)
        self.assertEqual(esito["stato"], "nuovo")
        self.assertEqual(esito["osservato"]["versione"], "60.0.0")

    def test_una_copia_locale_di_parquet_non_e_mai_nota(self) -> None:
        """Un sorgente fuori dal registro -- una copia nel checkout -- ha lo
        stesso modulo relativo, ma non e' la crate fissata."""
        altro = ESAURIMENTO.replace(
            "/home/runner/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/"
            "parquet-59.3.0/src/parquet_thrift.rs",
            f"/home/runner/work/{gate.ROOT.name}/{gate.ROOT.name}/parquet/src/parquet_thrift.rs",
        )
        esito = gate.classifica("geoparquet_reader", altro, registro(), contesto=CONTESTO)
        self.assertEqual(esito["stato"], "nuovo")
        self.assertEqual(esito["osservato"]["modulo"], "parquet/src/parquet_thrift.rs")

    def test_un_registro_falso_non_e_il_registro(self) -> None:
        """Un percorso che somiglia a un registro, fuori dalla radice dichiarata,
        non fa saltare libFuzzer ne' attribuisce parquet."""
        falso = gate.Contesto("/home/altro/.cargo/registry/src", COMMIT)
        self.assertEqual(
            gate.classifica("geoparquet_reader", ESAURIMENTO, registro(), contesto=falso)[
                "stato"
            ],
            "nuovo",
        )

    def test_un_altro_commit_di_rustc_non_e_la_libreria_standard(self) -> None:
        diverso = gate.Contesto(RADICE, "0" * 40)
        esito = gate.classifica("geoparquet_reader", ESAURIMENTO, registro(), contesto=diverso)
        self.assertEqual(esito["stato"], "nuovo")
        self.assertEqual(esito["osservato"]["funzione"], "alloc")

    def test_senza_provenienza_niente_e_noto(self) -> None:
        """Senza radice del registro e commit, nessun frame di libFuzzer o della
        libreria standard si salta: la firma e' il primo frame di libFuzzer."""
        esito = gate.classifica("geoparquet_reader", ESAURIMENTO, registro())
        self.assertEqual(esito["stato"], "nuovo")
        self.assertEqual(esito["osservato"]["funzione"], "PrintStackTrace()")

    def test_un_uscita_vuota_con_zero_non_e_una_corsa_finita(self) -> None:
        conclusa = (
            "#100 DONE cov: 1 ft: 1 corp: 1/1b\n"
            "Done 100 runs in 60 second(s)\n"
            "stat::number_of_executed_units: 100\n"
        )
        self.assertTrue(gate.conclusione_regolare(conclusa))
        self.assertFalse(gate.conclusione_regolare(""))
        self.assertFalse(gate.conclusione_regolare("Done 100 runs in 60 second(s)\n"))
        self.assertFalse(gate.conclusione_regolare(conclusa + conclusa))

    def test_una_voce_di_esaurimento_senza_funzione_e_rossa(self) -> None:
        documento = registro()
        voce = next(
            v for v in documento["finding"] if v["id"] == "parquet-footer-lista-thrift-oom"
        )
        del voce["funzione"]
        self.assertTrue(gate.registro_ben_formato(documento))

    def test_un_tipo_sconosciuto_e_rosso(self) -> None:
        documento = registro()
        documento["finding"][0]["tipo"] = "timeout"
        self.assertTrue(gate.registro_ben_formato(documento))


def _rinumera(testo: str) -> str:
    """Rinumera i frame dello stack in ordine, dopo un inserimento."""
    righe = []
    numero = 0
    for riga in testo.splitlines():
        if re.match(r"^\s+#\d+ ", riga):
            riga = re.sub(r"#\d+ ", f"#{numero} ", riga, count=1)
            numero += 1
        righe.append(riga)
    return "\n".join(righe) + "\n"


class SondeDelleVociNelVerbale(unittest.TestCase):
    """Un arresto «noto» si legge come tale: bersaglio e voce, non un numero."""

    def test_il_verbale_porta_la_voce_di_ogni_fermato(self) -> None:
        with tempfile.TemporaryDirectory() as temporanea:
            destinazione = pathlib.Path(temporanea) / "verbale.json"
            with unittest.mock.patch.dict(
                "os.environ", {gate.VARIABILE_VERBALE: str(destinazione)}
            ), contextlib.redirect_stdout(io.StringIO()):
                codice = gate.main(
                    [
                        "--scrivi-verbale",
                        "60",
                        "--finiti",
                        "shp_reader",
                        "--fermati",
                        "geoparquet_reader",
                        "--voci",
                        "geoparquet_reader=parquet-footer-lista-thrift-oom",
                        "--dichiarati",
                        "shp_reader",
                        "geoparquet_reader",
                    ]
                )
            self.assertEqual(codice, 0)
            verbale = json.loads(destinazione.read_text(encoding="utf-8"))
        self.assertEqual(
            verbale["voci_dei_fermati"],
            {"geoparquet_reader": "parquet-footer-lista-thrift-oom"},
        )

    def test_un_fermato_senza_voce_non_si_scrive(self) -> None:
        with tempfile.TemporaryDirectory() as temporanea:
            destinazione = pathlib.Path(temporanea) / "verbale.json"
            with unittest.mock.patch.dict(
                "os.environ", {gate.VARIABILE_VERBALE: str(destinazione)}
            ), contextlib.redirect_stderr(io.StringIO()):
                codice = gate.main(
                    ["--scrivi-verbale", "60", "--fermati", "geoparquet_reader"]
                )
            self.assertEqual(codice, 2)
            self.assertFalse(destinazione.exists())

    def test_lo_smoke_passa_le_voci_e_le_stampa(self) -> None:
        smoke = (gate.ROOT / "scripts" / "fuzz-smoke.sh").read_text(encoding="utf-8")
        self.assertIn("--voce-nota", smoke)
        self.assertIn("--voci", smoke)
        self.assertIn('${voci[*]}', smoke)


def _bash() -> str | None:
    """La bash per la prova d'integrazione.

    Su Linux, quella del sistema. Su Windows `bash` puo' essere quella di WSL,
    che non e' l'ambiente dello smoke: lì la prova gira solo se
    `PLENORA_BASH` indica una bash vera (per esempio quella di Git).
    """
    esplicita = os.environ.get("PLENORA_BASH")
    if esplicita:
        return esplicita
    if os.name == "nt":
        return None
    return shutil.which("bash")


#: Un `cargo` finto: `fuzz list` dichiara due bersagli, `fuzz build` riesce,
#: `fuzz run` stampa l'uscita registrata ed esce 1 per `geoparquet_reader`;
#: per `shp_reader` stampa cio' che sta in `FINTO_SHP` (il riepilogo
#: conclusivo, o niente) ed esce 0.
CARGO_FINTO = """#!/usr/bin/env bash
shift  # +toolchain
if [ "$1 $2" = "fuzz list" ]; then
    printf 'geoparquet_reader\\nshp_reader\\n'
    exit 0
fi
if [ "$1 $2" = "fuzz build" ]; then
    exit 0
fi
if [ "$1 $2" = "fuzz run" ]; then
    if [ "$3" = "geoparquet_reader" ]; then
        cat "${FINTO_USCITA}"
        exit 1
    fi
    cat "${FINTO_SHP}"
    exit 0
fi
echo "cargo finto: $*" >&2
exit 2
"""

RUSTUP_FINTO = """#!/usr/bin/env bash
echo "nightly-2026-07-21-x86_64-unknown-linux-gnu"
"""

#: `rustc -vV` con il commit della toolchain della corsa vera.
RUSTC_FINTO = f"""#!/usr/bin/env bash
echo "rustc 1.99.0-nightly"
echo "commit-hash: {COMMIT}"
"""

CONCLUSA = (
    "#100\tDONE   cov: 1 ft: 1 corp: 1/1b exec/s: 1 rss: 1Mb\n"
    "Done 100 runs in 60 second(s)\n"
    "stat::number_of_executed_units: 100\n"
)


@unittest.skipIf(_bash() is None, "serve una bash: su Windows impostare PLENORA_BASH")
class SondaDellIntegrazioneDelloSmoke(unittest.TestCase):
    """Lo smoke vero, con un fuzz finto.

    Lo script gira con `set -euo pipefail`, e il codice 3 del classificatore lo
    interrompeva prima del `case`, del verbale e del riepilogo: un noto
    diventava un rosso senza traccia. Qui si esegue `fuzz-smoke.sh` per intero
    in un albero temporaneo -- lo script, il classificatore, il registro e il
    lockfile del fuzz copiati, `cargo`, `rustup` e `rustc` finti, `CARGO_HOME`
    puntato al registro della corsa vera.
    """

    def _smoke(self, uscita_shp: str) -> tuple[subprocess.CompletedProcess[str], pathlib.Path, tempfile.TemporaryDirectory[str]]:
        bash = _bash()
        assert bash is not None
        temporanea = tempfile.TemporaryDirectory()
        self.addCleanup(temporanea.cleanup)
        radice = pathlib.Path(temporanea.name)
        (radice / "scripts").mkdir()
        (radice / "assurance" / "registries").mkdir(parents=True)
        (radice / "fuzz").mkdir()
        for nome in ("fuzz-smoke.sh", "classifica_finding_fuzz.py"):
            shutil.copy(gate.ROOT / "scripts" / nome, radice / "scripts" / nome)
        shutil.copy(gate.REGISTRO, radice / "assurance" / "registries" / gate.REGISTRO.name)
        shutil.copy(gate.LOCK_DEL_FUZZ, radice / "fuzz" / "Cargo.lock")
        uscita = radice / "uscita-registrata.txt"
        uscita.write_text(ESAURIMENTO, encoding="utf-8", newline="\n")
        shp = radice / "uscita-shp.txt"
        shp.write_text(uscita_shp, encoding="utf-8", newline="\n")

        finti = radice / "bin"
        finti.mkdir()
        for nome, testo in (
            ("cargo", CARGO_FINTO),
            ("rustup", RUSTUP_FINTO),
            ("rustc", RUSTC_FINTO),
        ):
            (finti / nome).write_text(testo, encoding="utf-8", newline="\n")
            (finti / nome).chmod(0o755)
        # `python3` e' quello che sta eseguendo questa prova.
        (finti / "python3").write_text(
            f'#!/usr/bin/env bash\nexec "{pathlib.Path(sys.executable).as_posix()}" "$@"\n',
            encoding="utf-8",
            newline="\n",
        )
        (finti / "python3").chmod(0o755)

        ambiente = {
            chiave: valore
            for chiave, valore in os.environ.items()
            if chiave not in (gate.VARIABILE_VERBALE, "PLENORA_FUZZ_SECONDS")
        }
        ambiente["PATH"] = finti.as_posix() + os.pathsep + ambiente.get("PATH", "")
        ambiente["FINTO_USCITA"] = uscita.as_posix()
        ambiente["FINTO_SHP"] = shp.as_posix()
        ambiente["ASAN_SYMBOLIZER_PATH"] = "/finto/llvm-symbolizer"
        ambiente["CARGO_HOME"] = "/home/runner/.cargo"
        # Con la bash di Git su Windows, un percorso POSIX passato a un
        # eseguibile Windows verrebbe riscritto: la radice del registro non
        # sarebbe piu' quella dichiarata. Si esclude dalla conversione solo
        # quella; su Linux la variabile non ha effetto.
        ambiente["MSYS2_ARG_CONV_EXCL"] = "/home/runner"
        esito = subprocess.run(
            [bash, (radice / "scripts" / "fuzz-smoke.sh").as_posix(), "--seconds", "1"],
            cwd=radice,
            env=ambiente,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            check=False,
        )
        return esito, radice, temporanea

    def _verbale(self, radice: pathlib.Path) -> dict:
        return json.loads(
            (radice / "assurance" / "evidence" / "fuzz-smoke-ultima.json").read_text(
                encoding="utf-8"
            )
        )

    def test_un_noto_arriva_al_riepilogo_e_al_verbale(self) -> None:
        esito, radice, _ = self._smoke(CONCLUSA)
        detto = esito.stdout + esito.stderr
        self.assertEqual(esito.returncode, 0, detto)
        self.assertIn(
            "bersaglio=voce: geoparquet_reader=parquet-footer-lista-thrift-oom",
            esito.stdout,
        )
        verbale = self._verbale(radice)
        self.assertEqual(verbale["fermati_a_finding_noto"], ["geoparquet_reader"])
        self.assertEqual(
            verbale["voci_dei_fermati"],
            {"geoparquet_reader": "parquet-footer-lista-thrift-oom"},
        )
        self.assertEqual(verbale["hanno_finito"], ["shp_reader"])
        self.assertEqual(verbale["falliti_su_finding_nuovo"], [])
        self.assertEqual(verbale["illeggibili"], [])
        conservati = list(
            (radice / "assurance" / "evidence" / "finding-fuzz" / "geoparquet_reader").glob(
                "*.json"
            )
        )
        self.assertEqual(len(conservati), 1)

    def test_un_uscita_vuota_con_zero_e_illeggibile_e_la_campagna_e_rossa(self) -> None:
        """Un bersaglio che esce 0 senza stampare niente non ha finito: senza il
        riepilogo conclusivo di libFuzzer e' illeggibile, lo smoke esce 1 e la
        verifica della campagna e' rossa."""
        esito, radice, _ = self._smoke("")
        self.assertEqual(esito.returncode, 1, esito.stdout + esito.stderr)
        self.assertIn("target senza un esito leggibile: shp_reader", esito.stderr)
        verbale = self._verbale(radice)
        self.assertEqual(verbale["illeggibili"], ["shp_reader"])
        self.assertNotIn("shp_reader", verbale["hanno_finito"])
        motivi = gate.verifica_campagna(
            radice / "assurance" / "evidence" / "fuzz-smoke-ultima.json",
            verbale["revisione"],
        )
        self.assertTrue(motivi)
