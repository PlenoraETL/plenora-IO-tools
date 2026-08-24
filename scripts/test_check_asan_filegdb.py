"""Sonde del gate che tiene misurato il confine di AddressSanitizer.

Il gate esiste per impedire una frase: «il percorso FileGDB e' coperto da
AddressSanitizer». E' vera a meta', e la meta' falsa e' quella che conta. Se il
gate sbagliasse, la frase tornerebbe vera per omissione -- e nessuna campagna
verde la smentirebbe, perche' una campagna verde e' esattamente cio' che quella
frase userebbe come prova.

Le sonde provano le due direzioni: che la misura vera sia verde, e che ogni modo
di raccontare un confine diverso da quello descritto sia rosso.

# Le due proprieta' non vanno confuse

**Strumentazione** e **feedback di copertura** sono cose diverse, e la prima
stesura del gate inferiva la prima dai secondi. Un binario puo' avere contatori
senza sanitizer e sanitizer senza contatori: le sonde qui sotto le muovono una
per volta, cosi' un gate che tornasse a confonderle diventerebbe rosso.
"""

from __future__ import annotations

import io
import json
import pathlib
import shutil
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout

from scripts import check_asan_filegdb as gate

IMPRONTA = "f" * 64


def misura_minima() -> dict:
    """La misura che descrive il confine vero, da cui le sonde tolgono un pezzo."""
    return {
        "target": "filegdb_reader",
        "impronta_perimetro": IMPRONTA,
        "libreria_collegata": {
            "soname": "libgdal.so.32",
            "percorso_risolto": "/lib/x86_64-linux-gnu/libgdal.so.32",
            "build_id": "0352ff263e6c85fc19040c196aeae35fdb431d7f",
            "sha256": "a" * 64,
        },
        "simboli_asan_nella_libreria": 0,
        "simboli_asan_nel_binario": 590,
        "runtime_asan_nel_binario": True,
        "libreria_gdal_dentro_l_albero_di_build": False,
        "moduli_con_contatori": 1,
        "contatori_di_copertura": 167812,
        "file_sorgente_gdal_strumentati": 0,
        "che_cosa_significa": {chiave: "prosa" for chiave in gate.AFFERMAZIONI},
    }


class SondeDelConfine(unittest.TestCase):
    def setUp(self) -> None:
        # L'impronta vera legge il working tree e chiama git; la libreria vera
        # sta sul disco di questa macchina. Qui interessa che il **confronto**
        # avvenga, non come si ottengono i valori: quelli hanno le loro sonde.
        precedente = gate.impronta_del_perimetro
        gate.impronta_del_perimetro = lambda percorsi: (IMPRONTA, [])
        self.addCleanup(setattr, gate, "impronta_del_perimetro", precedente)

        finta = pathlib.Path("/finta/libgdal.so.32")
        locale = gate._libreria_locale
        simboli = gate.simboli_asan
        gate._libreria_locale = lambda: finta
        gate.simboli_asan = lambda _percorso: 0
        self.addCleanup(setattr, gate, "_libreria_locale", locale)
        self.addCleanup(setattr, gate, "simboli_asan", simboli)

    def test_la_misura_vera_del_confine_e_verde(self) -> None:
        self.assertEqual(gate.verifica(misura_minima()), [])

    # --- la strumentazione, che e' il fatto centrale ----------------------

    def test_una_gdal_strumentata_non_passa_in_silenzio(self) -> None:
        """Se un giorno GDAL fosse costruita con la strumentazione sarebbe una
        **buona** notizia -- e questo gate dovrebbe comunque diventare rosso,
        perche' la prosa che descrive il confine andrebbe riscritta."""
        misura = misura_minima()
        misura["simboli_asan_nella_libreria"] = 412
        errori = gate.verifica(misura)
        self.assertTrue(any("simboli_asan_nella_libreria" in m for m in errori), errori)
        self.assertTrue(any("e' la prosa a dover cambiare" in m for m in errori))

    def test_una_gdal_locale_strumentata_e_rossa_anche_con_artefatto_pulito(self) -> None:
        """Il caso che il livello 2 non vedrebbe.

        Il checkpoint rilegge l'artefatto, non rifa' la misura: un artefatto
        che descrivesse una GDAL diversa da quella installata lascerebbe verde
        l'invariante su un ambiente che nessuno ha guardato. Il gate rimisura la
        libreria locale, e qui la sonda gliene mette davanti una strumentata.
        """
        gate.simboli_asan = lambda _percorso: 350
        errori = gate.verifica(misura_minima())
        self.assertTrue(any("e' strumentata" in m for m in errori), errori)
        self.assertTrue(any("va riscritta" in m for m in errori))

    def test_senza_gdal_locale_il_gate_non_conclude(self) -> None:
        """Un gate che verifica la GDAL di questa macchina, se non c'e', non
        deve dire di si': deve dire che non puo' dirlo."""

        def assente():
            raise gate.MisuraImpossibile("libgdal non trovata su questa macchina")

        gate._libreria_locale = assente
        errori = gate.verifica(misura_minima())
        self.assertTrue(any("non trovata" in m for m in errori), errori)

    # --- il feedback di copertura, che e' un'altra cosa -------------------

    def test_un_secondo_modulo_con_contatori_e_rosso(self) -> None:
        """Due moduli vorrebbero dire che una libreria condivisa porta
        contatori: il fuzzer non sarebbe piu' cieco dove la prosa dice che lo
        e'."""
        misura = misura_minima()
        misura["moduli_con_contatori"] = 2
        self.assertTrue(any("moduli_con_contatori" in m for m in gate.verifica(misura)))

    def test_sorgenti_di_gdal_nella_copertura_sono_rossi(self) -> None:
        misura = misura_minima()
        misura["file_sorgente_gdal_strumentati"] = 7
        self.assertTrue(
            any("file_sorgente_gdal_strumentati" in m for m in gate.verifica(misura))
        )

    def test_un_binario_senza_contatori_e_rosso(self) -> None:
        for valore in (0, -1, "molti", True, None):
            with self.subTest(valore=valore):
                misura = misura_minima()
                misura["contatori_di_copertura"] = valore
                self.assertTrue(
                    any("contatori_di_copertura" in m for m in gate.verifica(misura))
                )

    def test_i_contatori_non_dicono_niente_sulla_strumentazione(self) -> None:
        """La confusione che la prima stesura faceva, provata direttamente.

        Contatori in abbondanza **e** una libreria strumentata devono restare
        rossi: se il gate deducesse la seconda dai primi, questo caso passerebbe.
        """
        misura = misura_minima()
        misura["contatori_di_copertura"] = 999_999
        misura["simboli_asan_nella_libreria"] = 1
        self.assertTrue(
            any("simboli_asan_nella_libreria" in m for m in gate.verifica(misura))
        )

    # --- il resto -----------------------------------------------------------

    def test_il_runtime_asan_assente_dal_binario_e_rosso(self) -> None:
        """Il caso opposto, e il piu' pericoloso: un binario senza sanitizer che
        gira una campagna e non segnala niente."""
        misura = misura_minima()
        misura["runtime_asan_nel_binario"] = False
        self.assertTrue(any("runtime_asan_nel_binario" in m for m in gate.verifica(misura)))

    def test_un_binario_che_non_collega_gdal_e_rosso(self) -> None:
        misura = misura_minima()
        misura["libreria_collegata"]["soname"] = "libqualcosa.so.1"
        self.assertTrue(any("non collega" in m for m in gate.verifica(misura)))

    def test_una_libreria_senza_identita_e_rossa(self) -> None:
        """Senza build-id o digest la misura non dice **quale** libreria ha
        guardato, e il gate non puo' dire se e' la stessa."""
        for campo in ("percorso_risolto", "build_id", "sha256"):
            with self.subTest(campo):
                misura = misura_minima()
                misura["libreria_collegata"][campo] = ""
                errori = gate.verifica(misura)
                self.assertTrue(any(campo in m for m in errori), errori)

    def test_una_gdal_costruita_nell_albero_e_rossa(self) -> None:
        misura = misura_minima()
        misura["libreria_gdal_dentro_l_albero_di_build"] = True
        self.assertTrue(
            any("libreria_gdal_dentro_l_albero_di_build" in m for m in gate.verifica(misura))
        )

    def test_i_numeri_senza_le_frasi_sono_rossi(self) -> None:
        """Un numero senza la frase che dice che cosa significa e' un numero che
        qualcuno rileggera' come gli fa comodo."""
        for affermazione in gate.AFFERMAZIONI:
            with self.subTest(affermazione):
                misura = misura_minima()
                del misura["che_cosa_significa"][affermazione]
                errori = gate.verifica(misura)
                self.assertTrue(any(affermazione in m for m in errori), errori)

    def test_una_misura_di_un_altro_target_e_rossa(self) -> None:
        misura = misura_minima()
        misura["target"] = "shp_reader"
        self.assertTrue(any("shp_reader" in m for m in gate.verifica(misura)))

    def test_una_misura_di_un_altro_albero_e_rossa(self) -> None:
        gate.impronta_del_perimetro = lambda percorsi: ("0" * 64, [])
        errori = gate.verifica(misura_minima())
        self.assertTrue(any("impronta del perimetro diversa" in m for m in errori), errori)
        self.assertTrue(any("asan-filegdb.sh" in m for m in errori))


class SondeDellaRisoluzione(unittest.TestCase):
    """*Quale* libgdal viene misurata.

    La prima stesura preferiva il percorso scritto nell'artefatto quando esiste
    ancora. Un file puo' restare al suo posto mentre il loader ne sceglie un
    altro -- una `.so.35` installata accanto alla `.32`, un `LD_LIBRARY_PATH` --
    e in quel caso il gate diceva «non strumentata» di una libreria che nessun
    processo caricherebbe piu'.
    """

    def test_il_percorso_risolto_e_quello_che_dice_il_loader(self) -> None:
        radice = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, radice, True)
        vera = radice / "libgdal.so.35"
        vera.write_bytes(b"ELF")
        righe = [
            "\tlinux-vdso.so.1 (0x00007ffd)",
            f"\tlibgdal.so.35 => {vera} (0x00007f0000000000)",
            "\tlibc.so.6 => /lib/x86_64-linux-gnu/libc.so.6 (0x00007f0000001000)",
        ]
        self.assertEqual(gate._gdal_da_ldd(righe), vera)

    def test_una_dipendenza_non_risolta_non_e_una_libreria(self) -> None:
        """`ldd` stampa anche cio' che **non** trova: prendere l'ultimo campo di
        quelle righe darebbe il nome di un file inesistente."""
        self.assertIsNone(gate._gdal_da_ldd(["\tlibgdal.so.32 => not found"]))

    def test_un_binario_che_non_collega_gdal_non_da_nessuna_libreria(self) -> None:
        self.assertIsNone(
            gate._gdal_da_ldd(["\tlibc.so.6 => /lib/x86_64-linux-gnu/libc.so.6 (0x7f)"])
        )

    def test_il_percorso_registrato_non_viene_preferito(self) -> None:
        """Il fatto che il gate esiste per non dare per buono: si misura la
        libreria che il loader sceglie oggi, non quella di allora."""
        radice = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, radice, True)
        storica = radice / "libgdal.so.32"
        storica.write_bytes(b"ELF vecchio")
        odierna = radice / "libgdal.so.35"
        odierna.write_bytes(b"ELF nuovo")

        guardate: list[pathlib.Path] = []
        precedenti = (
            gate._binario_feature_on,
            gate._righe_di_ldd,
            gate.simboli_asan,
            gate.impronta_del_perimetro,
        )
        gate._binario_feature_on = lambda: radice / "plenora-io"
        gate._righe_di_ldd = lambda _binario: [f"\tlibgdal.so.35 => {odierna} (0x7f)"]
        gate.simboli_asan = lambda percorso: guardate.append(percorso) or 0
        gate.impronta_del_perimetro = lambda percorsi: (IMPRONTA, [])
        self.addCleanup(setattr, gate, "_binario_feature_on", precedenti[0])
        self.addCleanup(setattr, gate, "_righe_di_ldd", precedenti[1])
        self.addCleanup(setattr, gate, "simboli_asan", precedenti[2])
        self.addCleanup(setattr, gate, "impronta_del_perimetro", precedenti[3])

        misura = misura_minima()
        misura["libreria_collegata"]["percorso_risolto"] = str(storica)
        self.assertEqual(gate.verifica(misura), [])
        self.assertEqual(guardate, [odierna])


class SondeDellaMisuraDiretta(unittest.TestCase):
    """`simboli_asan` discrimina davvero fra strumentato e non.

    Senza questa sonda, `simboli_asan` potrebbe restituire zero per un errore di
    invocazione -- un `nm` assente, un percorso sbagliato -- e il gate leggerebbe
    quello zero come «non strumentata».
    """

    def test_il_nostro_binario_strumentato_porta_simboli_del_runtime(self) -> None:
        misura = json.loads(gate.ARTEFATTO.read_text(encoding="utf-8"))
        self.assertGreater(
            misura["simboli_asan_nel_binario"],
            100,
            "un binario costruito con -Zsanitizer=address porta centinaia di "
            "simboli del runtime; se ne porta pochi, la misura non sta guardando "
            "il binario giusto",
        )
        self.assertEqual(misura["simboli_asan_nella_libreria"], 0)


def gdal_locale_disponibile() -> bool:
    """Questo gate misura la GDAL di **questa** macchina.

    Dove non c'e' -- una macchina di sviluppo che non e' Linux -- la sonda che
    esegue il gate per intero non ha niente da misurare e si salta. In CI e nel
    container GDAL c'e', ed e' li' che quella sonda conta.
    """
    try:
        gate._libreria_locale()
    except gate.MisuraImpossibile:
        return False
    return True


class SondaDellaMisuraVera(unittest.TestCase):
    """L'artefatto committato, letto come in CI."""

    @unittest.skipUnless(
        gdal_locale_disponibile(), "GDAL non installata: il gate non puo' concludere"
    )
    def test_il_gate_e_verde_sull_albero_corrente(self) -> None:
        uscita, errori = io.StringIO(), io.StringIO()
        with redirect_stdout(uscita), redirect_stderr(errori):
            codice = gate.main([])
        self.assertEqual(codice, 0, errori.getvalue())

    def test_la_misura_porta_tutte_le_affermazioni(self) -> None:
        misura = json.loads(gate.ARTEFATTO.read_text(encoding="utf-8"))
        for affermazione in gate.AFFERMAZIONI:
            self.assertIn(affermazione, misura["che_cosa_significa"])
            self.assertGreater(len(misura["che_cosa_significa"][affermazione]), 40)

    def test_la_misura_non_promette_la_redzone(self) -> None:
        """La prima stesura affermava che un accesso di GDAL nella redzone di
        un'allocazione ASan venisse visto. E' falso: il controllo lo inserisce il
        compilatore, e codice non strumentato non consulta la shadow memory."""
        testo = gate.ARTEFATTO.read_text(encoding="utf-8").lower()
        self.assertNotIn("redzone", testo)
        misura = json.loads(gate.ARTEFATTO.read_text(encoding="utf-8"))
        self.assertIn(
            "non consultano la shadow memory",
            misura["che_cosa_significa"]["non_copre_gli_accessi_dentro_gdal"],
        )

    def test_la_misura_dice_come_e_stata_presa(self) -> None:
        """Un numero di cui non si sa come e' stato ottenuto non si puo'
        ricontrollare, e ricontrollarlo e' l'unico modo di fidarsene."""
        misura = json.loads(gate.ARTEFATTO.read_text(encoding="utf-8"))
        self.assertIn("come_sono_stati_contati", misura)
        self.assertIn("come_e_stata_misurata", misura["libreria_collegata"])
        # Il metodo deve nominare la misura diretta, non i contatori.
        self.assertIn("__asan", misura["come_sono_stati_contati"])


if __name__ == "__main__":
    unittest.main()
