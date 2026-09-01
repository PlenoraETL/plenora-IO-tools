"""Sonde sul verificatore nativo PE.

# Che cosa dimostrano, e che cosa no

Dimostrano che il verificatore **sa leggere** un PE: uno costruito qui byte per
byte, con una import table e una delay-import table. Se sbagliasse uno scarto o
un campo, queste sonde diventerebbero rosse.

Non dimostrano che l'artefatto Windows sia conforme. Quello lo dira' il
verificatore quando girera' su un artefatto vero, su un runner vero, e finche'
non succede la piattaforma resta `non_ancora_costruita` con il suo blocco. Sono
due affermazioni diverse, e confonderle sarebbe il modo piu' facile di
dichiarare verificato cio' che non lo e'.

# Il lettore Mach-O non c'e' piu'

E' uscito con macOS dal perimetro della v1. Un lettore che nessun artefatto
esercita invecchia senza che nessuno se ne accorga -- e quando servisse, gli
strumenti Apple e il formato saranno cambiati. La storia git lo conserva.

# Perche' binari sintetici e non file di prova scaricati

Perche' un file di prova sarebbe opaco: se una sonda diventasse rossa non si
saprebbe se ha trovato un difetto nel verificatore o una stranezza del file.
Costruendoli qui, ogni byte e' una decisione, e una sonda rossa nomina la
decisione che l'ha resa tale.
"""

from __future__ import annotations

import importlib.util
import pathlib
import shutil
import struct
import sys
import tempfile
import unittest

RADICE = pathlib.Path(__file__).resolve().parent.parent


def carica(nome: str):
    percorso = RADICE / "scripts" / nome
    spec = importlib.util.spec_from_file_location(percorso.stem.replace("-", "_"), percorso)
    modulo = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(modulo)
    return modulo


# --- un PE costruito a mano ------------------------------------------------


def costruisci_pe(
    percorso: pathlib.Path,
    *,
    macchina: int = 0x8664,
    importate: tuple[str, ...] = (),
    ritardate: tuple[str, ...] = (),
    firmato: bool = False,
) -> None:
    """Il PE minimo che le funzioni sotto prova devono saper leggere.

    Una sola sezione, che ospita sia le strutture delle directory sia i nomi. E'
    il minimo che regge le domande vere -- architettura, import, delay import --
    e non un byte di piu': cio' che non serve a una domanda non aiuta a
    rispondere e nasconde dove sta il difetto.
    """
    inizio_pe = 0x80
    rva_sezione = 0x1000
    offset_sezione = 0x400

    corpo = bytearray()
    posizioni: dict[str, int] = {}

    def deposita(testo: str) -> int:
        if testo not in posizioni:
            posizioni[testo] = len(corpo)
            corpo.extend(testo.encode("ascii") + b"\0")
        return rva_sezione + posizioni[testo]

    # Le due tabelle: 20 byte per voce, terminate da una voce di zeri. Il campo
    # «nome» sta a scarto 12 per gli import e a scarto 4 per i delay import, ed
    # e' l'unica differenza che conta.
    def tabella(nomi: tuple[str, ...], scarto_nome: int) -> tuple[int, int]:
        if not nomi:
            return 0, 0
        rva_nomi = [deposita(n) for n in nomi]
        inizio = len(corpo)
        for rva in rva_nomi:
            voce = bytearray(20)
            struct.pack_into("<I", voce, scarto_nome, rva)
            corpo.extend(voce)
        corpo.extend(bytes(20))
        return rva_sezione + inizio, (len(nomi) + 1) * 20

    rva_import, dim_import = tabella(importate, 12)
    rva_delay, dim_delay = tabella(ritardate, 4)

    dati = bytearray(offset_sezione)
    dati[0:2] = b"MZ"
    struct.pack_into("<I", dati, 0x3C, inizio_pe)
    dati[inizio_pe : inizio_pe + 4] = b"PE\0\0"
    dimensione_opzionale = 112 + 16 * 8
    struct.pack_into("<HH", dati, inizio_pe + 4, macchina, 1)
    struct.pack_into("<H", dati, inizio_pe + 20, dimensione_opzionale)
    inizio_opzionale = inizio_pe + 24
    struct.pack_into("<H", dati, inizio_opzionale, 0x20B)  # PE32+
    inizio_directory = inizio_opzionale + 112
    struct.pack_into("<II", dati, inizio_directory + 1 * 8, rva_import, dim_import)
    struct.pack_into("<II", dati, inizio_directory + 13 * 8, rva_delay, dim_delay)
    if firmato:
        # Directory 4: la Certificate Table. A differenza delle altre non porta
        # un RVA ma un offset nel file -- e per la domanda «e' firmato?» conta
        # che ci sia, non dove punti.
        struct.pack_into("<II", dati, inizio_directory + 4 * 8, 0x2000, 0x400)

    inizio_sezioni = inizio_opzionale + dimensione_opzionale
    intestazione = bytearray(40)
    intestazione[0:5] = b".text"
    struct.pack_into("<IIII", intestazione, 8, len(corpo), rva_sezione, len(corpo), offset_sezione)
    dati[inizio_sezioni : inizio_sezioni + 40] = intestazione

    percorso.write_bytes(bytes(dati) + bytes(corpo))


class SondeDelLettorePe(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        self.pe = carica("check-windows-runtime.py")

    def test_legge_l_architettura(self) -> None:
        x64 = self.tmp / "x64.exe"
        costruisci_pe(x64, macchina=0x8664)
        self.assertEqual(self.pe.architettura(x64), self.pe.MACCHINA_X86_64)

        x86 = self.tmp / "x86.exe"
        costruisci_pe(x86, macchina=0x014C)
        self.assertNotEqual(self.pe.architettura(x86), self.pe.MACCHINA_X86_64)

    def test_legge_gli_import_normali(self) -> None:
        f = self.tmp / "a.exe"
        costruisci_pe(f, importate=("KERNEL32.dll", "gdal.dll"))
        normali, ritardati = self.pe.importazioni(f)
        self.assertEqual(normali, {"kernel32.dll", "gdal.dll"})
        self.assertEqual(ritardati, set())

    def test_legge_anche_i_delay_import(self) -> None:
        """La ragione per cui questo verificatore esiste.

        Una DLL che comparisse solo fra i delay import sfuggirebbe a chi
        guardasse la sola import table, e si manifesterebbe molto dopo -- in
        esecuzione, su una macchina che non ce l'ha."""
        f = self.tmp / "b.exe"
        costruisci_pe(f, importate=("kernel32.dll",), ritardate=("bcrypt.dll",))
        normali, ritardati = self.pe.importazioni(f)
        self.assertEqual(normali, {"kernel32.dll"})
        self.assertEqual(ritardati, {"bcrypt.dll"}, "i delay import non sono stati letti")

    def test_i_nomi_si_confrontano_senza_maiuscole(self) -> None:
        """Windows non distingue le maiuscole: trattare `KERNEL32.dll` e
        `kernel32.dll` come diversi sarebbe un rosso che non significa niente."""
        f = self.tmp / "c.exe"
        costruisci_pe(f, importate=("KeRnEl32.DLL",))
        normali, _ = self.pe.importazioni(f)
        self.assertEqual(normali, {"kernel32.dll"})
        self.assertLessEqual(normali, self.pe.POLITICA_ABI)

    def test_un_file_che_non_e_un_pe_non_passa_in_silenzio(self) -> None:
        f = self.tmp / "no.exe"
        f.write_bytes(b"non sono un PE")
        with self.assertRaises(self.pe.PeMalformato):
            self.pe.architettura(f)

    def test_la_chiusura_separa_interne_ed_esterne(self) -> None:
        albero = self.tmp / "albero"
        (albero / "bin").mkdir(parents=True)
        costruisci_pe(albero / "bin" / "gdal.dll", importate=("kernel32.dll",))
        costruisci_pe(
            albero / "bin" / "plenora-io.exe",
            importate=("gdal.dll",),
            ritardate=("bcrypt.dll",),
        )
        interne, esterne, ritardate = self.pe.chiusura(
            albero / "bin" / "plenora-io.exe", albero
        )
        self.assertEqual(set(interne), {"gdal.dll"})
        self.assertEqual(esterne, {"kernel32.dll", "bcrypt.dll"})
        self.assertEqual(ritardate, {"bcrypt.dll"})

    def test_riconosce_un_pe_firmato_e_uno_no(self) -> None:
        """La parte della misura che si legge dai byte.

        `firma.stato` non deve venire da «il costruttore aveva un certificato»:
        quello direbbe soltanto che qualcuno ne ha avuto uno fra le mani. La
        presenza della firma si misura qui; l'identita' del firmatario e il
        timestamp li dice Windows, e fuori da Windows restano **non misurati**
        -- che non e' «non firmato» e non e' «va bene»."""
        senza = self.tmp / "senza.exe"
        costruisci_pe(senza, importate=("kernel32.dll",))
        self.assertFalse(self.pe.ha_tabella_dei_certificati(senza))

        con = self.tmp / "con.exe"
        costruisci_pe(con, importate=("kernel32.dll",), firmato=True)
        self.assertTrue(self.pe.ha_tabella_dei_certificati(con))

    def test_i_due_livelli_della_misura_non_si_confondono(self) -> None:
        """La struttura dice una cosa, il sistema ne dice un'altra piu' forte.

        `ha_tabella_dei_certificati` legge i byte: dice che una firma **c'e'**.
        Su Windows `Get-AuthenticodeSignature` chiede al sistema se quella firma
        sia **valida**, e su questo PE -- costruito qui, con una tabella che
        punta a byte che non ci sono -- la risposta giusta e' no.

        Le due risposte sono diverse apposta, e la seconda e' quella che conta:
        un artefatto con una tabella dei certificati e una firma non valida non
        e' un artefatto firmato. Fuori da Windows la seconda domanda resta
        **non misurata**, che non e' «non firmato» e non e' «va bene»."""
        f = self.tmp / "m.exe"
        costruisci_pe(f, importate=("kernel32.dll",), firmato=True)
        self.assertTrue(
            self.pe.ha_tabella_dei_certificati(f), "la tabella c'e', e si legge dai byte"
        )

        misura = self.pe.misura_della_firma(f)
        if self.pe.sys.platform == "win32":
            self.assertFalse(
                misura["firmato"],
                "il sistema deve rifiutare una tabella che non porta una firma vera",
            )
            self.assertIn("stato_authenticode", misura)
        else:
            self.assertTrue(misura["firmato"], "senza il sistema resta la misura strutturale")
            self.assertIsNone(misura["firmatario"])
            self.assertIsNone(misura["timestamp"])
            self.assertIn("non_misurabile_qui", misura)

    def test_i_percorsi_di_costruzione_si_trovano_in_ascii_e_utf16(self) -> None:
        """I binari Windows portano entrambe le codifiche: cercarne una sola e'
        un modo di trovarne meno di quante ce ne sono."""
        f = self.tmp / "d.bin"
        prefisso = "C:\\lavoro\\prefisso"
        f.write_bytes(
            b"..." + f"{prefisso}\\share\\gdal".encode("ascii")
            + b"\0\0" + f"{prefisso}\\lib\\x".encode("utf-16-le")
        )
        trovati = self.pe.percorsi_assoluti(f, prefisso)
        self.assertTrue(any("share" in t for t in trovati), trovati)
        self.assertTrue(any("lib" in t for t in trovati), trovati)


class SondeDellaScoperta(unittest.TestCase):
    """La prima corsa Windows scopre, e non qualifica.

    L'insieme delle DLL di sistema attese non si scrive a tavolino: dipende da
    come conda-forge ha compilato GDAL per win-64, e l'unico modo di saperlo e'
    guardare un artefatto vero. La corsa di scoperta lo misura e si ferma.

    Che si **fermi** e' il punto. Una corsa che potesse diventare verde da sola
    scriverebbe il proprio contratto, e un contratto scritto da cio' che deve
    verificare non verifica niente.
    """

    def setUp(self) -> None:
        self.tmp = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        self.pe = carica("check-windows-runtime.py")

    def artefatto(self, profilo: str) -> pathlib.Path:
        import json

        albero = self.tmp / f"plenora-io-0.0.0-prova-windows-x86_64-{profilo}"
        (albero / "bin").mkdir(parents=True)
        importate = ("kernel32.dll", "api-ms-win-crt-runtime-l1-1-0.dll")
        if profilo == "filegdb":
            costruisci_pe(
                albero / "bin" / "gdal.dll",
                importate=("kernel32.dll", "advapi32.dll"),
                ritardate=("bcrypt.dll",),
            )
            importate = ("gdal.dll", *importate)
        costruisci_pe(albero / "bin" / "plenora-io.exe", importate=importate)
        with (albero / "bin" / "plenora-io.exe").open("ab") as f:
            f.write(b"C:\\lavoro\\prefisso\\share\\gdal\x00")
        (albero / "MANIFEST.json").write_text(
            json.dumps(
                {
                    "piattaforma": "windows-x86_64",
                    "profilo": profilo,
                    "canale": "prova",
                    "versione": "0.0.0-prova",
                    "prefisso_di_costruzione": "C:\\lavoro\\prefisso",
                }
            ),
            encoding="utf-8",
        )
        return albero

    def scopri(self, profilo: str):
        import json
        import subprocess

        albero = self.artefatto(profilo)
        rilievo = self.tmp / "discovery" / f"windows-{profilo}.json"
        esito = subprocess.run(
            [
                sys.executable,
                str(RADICE / "scripts" / "check-windows-runtime.py"),
                "--albero", str(albero),
                "--discovery", str(rilievo),
            ],
            capture_output=True,
            text=True,
        )
        return esito, json.loads(rilievo.read_text(encoding="utf-8"))

    def test_la_scoperta_termina_rossa(self) -> None:
        """Il rosso non e' un difetto trovato: e' l'assenza di una revisione
        umana, e va distinto da un verde che non ha verificato niente."""
        esito, rilievo = self.scopri("filegdb")
        self.assertNotEqual(esito.returncode, 0, "la scoperta non deve poter passare")
        self.assertTrue(rilievo["non_qualificante"])
        self.assertIn("contratto", rilievo["perche_rosso"])

    def test_la_scoperta_non_tocca_il_lock(self) -> None:
        """Non modifica lock ne' repository: cio' che misura diventa un
        contratto solo passando per una rilettura e per un commit."""
        lock = RADICE / "scripts" / "windows-gdal-lock.json"
        prima = lock.read_bytes()
        self.scopri("base")
        self.assertEqual(lock.read_bytes(), prima)

    def test_registra_cio_che_serve_a_scrivere_il_contratto(self) -> None:
        """Import, delay-import, DLL interne, API-set, DLL esterne, percorsi
        incorporati, architettura -- e da dove viene la misura."""
        _, rilievo = self.scopri("filegdb")
        misure = rilievo["misure"]
        for atteso in (
            "import_normali",
            "import_ritardati",
            "dll_interne",
            "api_set",
            "dll_esterne",
            "percorsi_incorporati",
            "architetture",
        ):
            self.assertIn(atteso, misure, f"il rilievo non porta «{atteso}»")
        provenienza = rilievo["provenienza_della_misura"]
        for atteso in ("runner", "sistema", "sha_sorgente", "lock_sha256"):
            self.assertIn(atteso, provenienza)
        self.assertEqual(len(provenienza["lock_sha256"]), 64)

    def test_i_due_profili_producono_due_rilievi_distinti(self) -> None:
        """`base` e `filegdb` sono due prodotti: usare la misura dell'uno per
        l'altro attribuirebbe a un artefatto una misura fatta su un altro."""
        _, base = self.scopri("base")
        _, filegdb = self.scopri("filegdb")
        self.assertEqual(base["artefatto"]["profilo"], "base")
        self.assertEqual(filegdb["artefatto"]["profilo"], "filegdb")
        self.assertNotEqual(base["misure"]["dll_interne"], filegdb["misure"]["dll_interne"])
        self.assertIn("gdal.dll", filegdb["misure"]["dll_interne"])
        self.assertEqual(base["misure"]["dll_interne"], [])

    def test_le_api_set_non_finiscono_fra_le_dll(self) -> None:
        """Un'API-set e' un nome che il caricatore traduce, non un file che il
        sistema fornisce. Metterla fra le DLL funzionerebbe e direbbe una cosa
        falsa -- e farebbe cercare in `bin/` qualcosa che per costruzione non
        esiste."""
        _, rilievo = self.scopri("base")
        self.assertIn("api-ms-win-crt-runtime-l1-1-0.dll", rilievo["misure"]["api_set"])
        self.assertNotIn("api-ms-win-crt-runtime-l1-1-0.dll", rilievo["misure"]["dll_esterne"])

    def test_i_delay_import_sono_registrati_a_parte(self) -> None:
        """Una DLL che comparisse solo fra i delay import si manifesterebbe in
        esecuzione, molto dopo. Il rilievo la porta in entrambi gli elenchi."""
        _, rilievo = self.scopri("filegdb")
        self.assertIn("bcrypt.dll", rilievo["misure"]["import_ritardati"])
        self.assertIn("bcrypt.dll", rilievo["misure"]["dll_esterne"])


class SondeDellaClassificazione(unittest.TestCase):
    """Le quattro classi, e la sola che blocca."""

    def setUp(self) -> None:
        self.pe = carica("check-windows-runtime.py")

    def test_le_quattro_classi(self) -> None:
        interne = {"gdal.dll": pathlib.Path("bin/gdal.dll")}
        attese = {"kernel32.dll"}
        casi = {
            "gdal.dll": "interna",
            "api-ms-win-crt-runtime-l1-1-0.dll": "api_set",
            "ext-ms-win-qualcosa-l1-1-0.dll": "api_set",
            "kernel32.dll": "abi_windows",
            "qualcosa-di-ignoto.dll": "inattesa",
        }
        for nome, atteso in casi.items():
            with self.subTest(nome=nome):
                self.assertEqual(
                    self.pe.classifica_dipendenza(nome, interne, attese), atteso
                )

    def test_una_libreria_spedita_e_interna_anche_se_somiglia_a_una_di_sistema(self) -> None:
        """L'ordine delle domande conta: cio' che si spedisce e' cio' che si
        carica, e il nome non decide."""
        interne = {"kernel32.dll": pathlib.Path("bin/kernel32.dll")}
        self.assertEqual(
            self.pe.classifica_dipendenza("kernel32.dll", interne, {"kernel32.dll"}),
            "interna",
        )

    def test_il_runtime_c_va_spedito_non_ammesso(self) -> None:
        """La distinzione che la prima corsa di scoperta ha reso concreta.

        `vcruntime140.dll` non e' un componente di Windows: e' il runtime C
        ridistribuibile di Visual Studio. Il runner lo possiede perche' ci gira
        Visual Studio; una macchina di destinazione pulita potrebbe non averlo,
        e l'artefatto non partirebbe con un errore che parla di una DLL
        mancante invece che di cio' che manca davvero."""
        for nome in ("vcruntime140.dll", "vcruntime140_1.dll", "msvcp140.dll"):
            with self.subTest(nome=nome):
                self.assertNotIn(nome, self.pe.POLITICA_ABI)
                self.assertIn(nome, self.pe.DA_SPEDIRE_NON_AMMETTERE)
                self.assertEqual(
                    self.pe.classifica_dipendenza(nome, {}, set()), "inattesa"
                )

    def test_le_tre_dll_di_sistema_sono_in_politica(self) -> None:
        """Classificate come componenti del sistema operativo, non perche' il
        runner le abbia: `bcryptprimitives` e' CNG dai tempi di Vista,
        `odbc32` e' il Driver Manager ODBC, `wsock32` e' la Winsock 1.1 che
        Windows conserva per compatibilita'."""
        for nome in ("bcryptprimitives.dll", "odbc32.dll", "wsock32.dll"):
            with self.subTest(nome=nome):
                self.assertIn(nome, self.pe.POLITICA_ABI)

    def test_la_politica_dichiara_il_proprio_limite(self) -> None:
        """Le voci sono classificate sulla documentazione dei componenti di
        Windows, non su una misura fatta su un'installazione pulita: un runner
        di GitHub non e' una baseline."""
        sorgente = (RADICE / "scripts" / "check-windows-runtime.py").read_text(encoding="utf-8")
        blocco = sorgente[: sorgente.index("POLITICA_ABI = {")]
        self.assertIn("baseline", blocco.lower())
        self.assertIn("installazione pulita", blocco)

    def test_inattesa_non_significa_probabilmente_va_bene(self) -> None:
        """E' la classe che resta, ed e' quella che blocca: significa che
        nessuno ha deciso che cosa sia quella dipendenza."""
        self.assertEqual(self.pe.classifica_dipendenza("ignota.dll", {}, set()), "inattesa")
        self.assertIn("inattesa", self.pe.CATEGORIE)


class SondeSulPerimetro(unittest.TestCase):
    """Che cosa il verificatore pretende di avere prima di dire qualcosa."""

    def setUp(self) -> None:
        self.pe = carica("check-windows-runtime.py")

    def test_un_contratto_esiste_solo_se_viene_da_un_rilievo(self) -> None:
        """La pretesa giusta, che ho dovuto correggere.

        La prima formulazione vietava un contratto di verifica finche' la
        piattaforma non fosse **costruita**. Era una scorciatoia: cio' che va
        vietato non e' un contratto su una piattaforma non ancora verificata --
        e' un contratto **scritto a tavolino**, cioe' una soglia mai misurata
        che passa per misurata.

        Ora il contratto Windows esiste e viene da una corsa di scoperta reale,
        con il digest del rilievo per ciascun profilo. La piattaforma resta non
        costruita, perche' il contratto non e' ancora stato **applicato**: sono
        due cose diverse, e la prima sonda le confondeva."""
        import json

        for nome in ("windows-gdal-lock.json",):
            percorso = RADICE / "scripts" / nome
            if not percorso.exists():
                continue
            with self.subTest(lock=nome):
                lock = json.loads(percorso.read_text(encoding="utf-8"))
                contratto = lock.get("contratto_di_verifica")
                if contratto is None:
                    continue  # non ancora misurato: e' il blocco registrato
                origine = contratto.get("rilievo_di_origine")
                self.assertIsNotNone(
                    origine,
                    "il contratto non dice da quale rilievo viene: sarebbe una soglia "
                    "scritta a tavolino, e passerebbe per misurata",
                )
                self.assertTrue(origine.get("sha_sorgente"))
                self.assertEqual(set(origine["sha256"]), {"base", "filegdb"})

    def test_gli_insiemi_attesi_vengono_dalla_misura(self) -> None:
        """Il contratto porta cio' che la scoperta ha visto, non cio' che
        sarebbe ragionevole aspettarsi.

        `base` chiede tre DLL e `filegdb` quindici; le tre del base compaiono
        tutte fra le quindici, e non per caso -- `bcryptprimitives.dll` e' li'
        perche' la libreria standard di Rust la usa, ed e' una dipendenza del
        nostro binario e non di GDAL."""
        import json

        percorso = RADICE / "scripts" / "windows-gdal-lock.json"
        contratto = json.loads(percorso.read_text(encoding="utf-8")).get("contratto_di_verifica")
        if contratto is None:
            self.skipTest("il contratto non e' ancora stato scritto")
        attese = contratto["dll_di_sistema_attese"]
        self.assertLessEqual(set(attese["base"]), set(attese["filegdb"]))
        self.assertIn("bcryptprimitives.dll", attese["base"])
        # E ogni attesa e' ammissibile: un insieme che ammettesse cio' che la
        # politica non ammette sarebbe un'eccezione concessa a se stesso.
        for profilo, nomi in sorted(attese.items()):
            with self.subTest(profilo=profilo):
                self.assertLessEqual(set(nomi), self.pe.POLITICA_ABI)


if __name__ == "__main__":
    unittest.main()
