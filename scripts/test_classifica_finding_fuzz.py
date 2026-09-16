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
