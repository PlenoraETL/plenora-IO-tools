"""Sonde sulla coerenza fra la matrice di distribuzione e i lock.

# Il difetto che queste sonde chiudono

La prima stesura del lock portava **tre** numeri per lo stesso fatto -- 55, 56
e 57 dipendenze interne -- sparsi fra il lock, la matrice e il rapporto, e due
erano sbagliati. Nessun gate se ne accorgeva: erano prosa, e la prosa non si
riconcilia da sola.

La cura non e' rileggere meglio. E' che un numero derivato stia in **un posto
solo** -- quello che lo misura -- e che cio' che compare in due posti sia
confrontato da qualcosa che diventa rosso quando divergono.
"""

from __future__ import annotations

import json
import pathlib
import re
import unittest

RADICE = pathlib.Path(__file__).resolve().parent.parent
MATRICE = RADICE / "assurance" / "registries" / "distribuzione-matrice.json"
LOCK_LINUX = RADICE / "scripts" / "linux-gdal-lock.json"
LOCK_WINDOWS = RADICE / "scripts" / "windows-gdal-lock.json"
LOCK_MACOS = RADICE / "scripts" / "macos-gdal-lock.json"
LOCK_PER_PIATTAFORMA = {
    "linux-x86_64": LOCK_LINUX,
    "windows-x86_64": LOCK_WINDOWS,
    "macos-aarch64": LOCK_MACOS,
}

CHECKER = RADICE / "scripts" / "check-linux-gdal-runtime.py"
RADICI_RS = RADICE / "crates" / "plenora-io-cli" / "src" / "radici.rs"


def carica(percorso: pathlib.Path) -> dict:
    return json.loads(percorso.read_text(encoding="utf-8"))


def docset() -> list[pathlib.Path]:
    """I documenti canonici, presi da chi li definisce.

    Elencarli qui a mano sarebbe una copia del perimetro, e le copie divergono:
    un documento nuovo resterebbe fuori da questa sonda senza che nessuno se ne
    accorga. `check_docset.py` e' il posto dove il docset e' deciso, e da li'
    si legge."""
    sorgente = (RADICE / "scripts" / "check_docset.py").read_text(encoding="utf-8")
    blocco = sorgente.split("CANONICI = [")[1].split("]")[0]
    return [RADICE / relativo for relativo in re.findall(r'"([^"]+)"', blocco)]



class SondeMatrice(unittest.TestCase):
    def setUp(self) -> None:
        self.matrice = carica(MATRICE)
        self.lock = carica(LOCK_LINUX)

    def test_ogni_piattaforma_ha_la_propria_origine(self) -> None:
        """Una piattaforma senza origine e' una promessa senza runtime."""
        piattaforme = {p["id"] for p in self.matrice["piattaforme"]}
        origini = {o["piattaforma"] for o in self.matrice["contratto_gdal"]["origini"]}
        self.assertEqual(piattaforme, origini)

    def costruite(self) -> set[str]:
        return {
            p["id"]
            for p in self.matrice["piattaforme"]
            if p["stato_costruzione"] == "costruita"
        }

    def test_la_versione_gdal_e_una_sola_in_ogni_lock(self) -> None:
        """E' la precondizione perche' la capability sia la stessa ovunque: se
        due artefatti dichiarassero versioni diverse, porterebbero prodotti
        diversi sotto lo stesso nome.

        La pretesa vale su **ogni lock che esiste**, non solo su quelli gia'
        costruiti. Un lock e' una dichiarazione di che cosa si scarichera', e
        vale gia' prima che qualcuno lo materializzi: e' proprio nella finestra
        fra «dichiarato» e «costruito» che la divergenza precedente e'
        sopravvissuta, con Windows a 3.10.3 e Linux a 3.9.3.

        Legarla alle sole piattaforme costruite era troppo debole, e lo era in
        modo comodo: rendeva vera una sonda lasciando falso il repository."""
        dichiarata = self.matrice["contratto_gdal"]["versione"]
        trovati = 0
        for piattaforma, percorso in sorted(LOCK_PER_PIATTAFORMA.items()):
            if not percorso.exists():
                continue
            trovati += 1
            with self.subTest(piattaforma=piattaforma):
                self.assertEqual(carica(percorso)["gdal_version"], dichiarata)
        self.assertGreaterEqual(trovati, 1, "nessun lock trovato: la sonda non ha guardato niente")

    def test_ogni_piattaforma_non_costruita_porta_un_blocco(self) -> None:
        """La sonda che chiude la scappatoia.

        Senza, `stato_costruzione` sarebbe un interruttore per spegnere le
        pretese: basterebbe declassare una piattaforma per far passare una
        divergenza. Dichiararla non costruita costa quindi dire **che cosa la
        costruirebbe**, e chi legge trova il debito invece del silenzio."""
        con_blocco = {b["piattaforma"] for b in self.matrice["blocchi_aperti"]}
        non_costruite = {
            p["id"] for p in self.matrice["piattaforme"]
        } - self.costruite()
        self.assertEqual(
            non_costruite,
            non_costruite & con_blocco,
            f"senza blocco registrato: {sorted(non_costruite - con_blocco)}",
        )
        for blocco in self.matrice["blocchi_aperti"]:
            with self.subTest(piattaforma=blocco["piattaforma"]):
                for campo in ("blocco", "che_cosa_lo_chiude", "nel_frattempo"):
                    self.assertTrue(blocco.get(campo), f"«{campo}» vuoto")

    def test_i_binding_sono_della_serie_della_libreria_spedita(self) -> None:
        """Il difetto che la costruzione Linux ha fatto emergere.

        `gdal-sys` sceglie i binding pre-costruiti dalla versione che gli viene
        dichiarata. Dichiararne una diversa da quella spedita compila l'ABI di
        una serie contro la libreria di un'altra: funziona finche' funziona, e
        quando smette non lo dice. Su Linux la build si e' fermata da sola --
        non esistono binding per 3.10 -- e per questo il difetto si e' visto.

        Vale su ogni lock: un binding disallineato e' un difetto della
        dichiarazione, e non serve materializzarla per vederlo."""
        for piattaforma, percorso in sorted(LOCK_PER_PIATTAFORMA.items()):
            if not percorso.exists():
                continue
            with self.subTest(piattaforma=piattaforma):
                lock = carica(percorso)
                serie = lambda v: ".".join(v.split(".")[:2])
                self.assertEqual(
                    serie(lock["binding_version"]),
                    serie(lock["gdal_version"]),
                    f"{piattaforma}: binding {lock['binding_version']} contro libreria "
                    f"{lock['gdal_version']}",
                )

    def test_la_soglia_glibc_e_la_stessa_nei_due_posti(self) -> None:
        """La matrice la **dichiara**, il lock la fa **pretendere** al controllo.

        Comparire in due posti e' inevitabile -- l'una e' una promessa verso chi
        installa, l'altra e' una soglia che un programma applica -- e per questo
        vanno confrontate."""
        linux = next(p for p in self.matrice["piattaforme"] if p["id"] == "linux-x86_64")
        self.assertEqual(
            linux["glibc_dichiarata"],
            self.lock["contratto_di_verifica"]["glibc_massima_ammessa"],
        )

    def test_il_requisito_virtuale_sta_sotto_la_soglia_dichiarata(self) -> None:
        """Se la chiusura pretendesse piu' della soglia, la promessa sarebbe falsa
        prima ancora che qualcuno costruisca."""
        glibc = next(
            r for r in self.lock["requisiti_virtuali"] if r["nome"] == "__glibc"
        )
        def chiave(v: str) -> tuple[int, ...]:
            return tuple(int(x) for x in v.split("."))
        self.assertLessEqual(
            chiave(glibc["minimo_richiesto"]),
            chiave(self.lock["contratto_di_verifica"]["glibc_massima_ammessa"]),
        )

    def test_l_atteso_e_un_sottoinsieme_della_politica(self) -> None:
        """I due insiemi non sono indipendenti: cio' che il lock si aspetta di
        trovare fuori dall'albero dev'essere anche ammissibile.

        Una dipendenza attesa ma fuori politica sarebbe un'eccezione concessa
        dal lock a se stesso."""
        politica = set(
            re.findall(r'"([^"]+\.so[^"]*)"', CHECKER.read_text(encoding="utf-8").split("POLITICA_ABI = {")[1].split("}")[0])
        )
        per_profilo = self.lock["contratto_di_verifica"]["dipendenze_esterne_attese"]
        self.assertIsInstance(
            per_profilo, dict, "le attese sono per profilo: i due profili sono due prodotti"
        )
        self.assertEqual(set(per_profilo), {"base", "filegdb"})
        for profilo, attese in sorted(per_profilo.items()):
            with self.subTest(profilo=profilo):
                attese = set(attese)
                self.assertTrue(attese, "l'insieme atteso non puo' essere vuoto")
                self.assertTrue(
                    attese <= politica,
                    f"attese fuori dalla politica ABI: {sorted(attese - politica)}",
                )

    def test_la_matrice_non_ricopia_numeri_misurati(self) -> None:
        """La regola che il difetto ha prodotto.

        Un conteggio nella matrice e' una copia di qualcosa che un programma
        misura altrove, e le copie divergono. La matrice dice **che cosa** si
        verifica; i numeri stanno dove nascono."""
        origine = next(
            o
            for o in self.matrice["contratto_gdal"]["origini"]
            if o["piattaforma"] == "linux-x86_64"
        )
        self.assertNotIn("misure", origine)
        for chiave_vietata in ("chiusura_dt_needed", "glibc_massima_negli_elf_spediti"):
            self.assertNotIn(
                chiave_vietata,
                json.dumps(origine, ensure_ascii=False),
                f"«{chiave_vietata}» e' un numero misurato: sta nel lock o nel referto, non qui",
            )

    def test_il_lock_non_porta_misure_derivate(self) -> None:
        """Lo stesso, dall'altra parte: il lock dichiara il contratto, non gli
        esiti. Le misure vivono in `verifica-runtime.json`, accanto al prefisso
        che le ha prodotte."""
        self.assertNotIn("misure_alla_creazione", self.lock)

    def test_ogni_pacchetto_del_lock_e_verificabile(self) -> None:
        """Senza URL, dimensione e sha256 il costruttore dovrebbe fidarsi."""
        for pacchetto in self.lock["pacchetti"]:
            with self.subTest(pacchetto=pacchetto["nome"]):
                for campo in ("url", "dimensione", "sha256", "build", "subdir", "versione"):
                    self.assertIn(campo, pacchetto)
                self.assertEqual(len(pacchetto["sha256"]), 64)
                self.assertTrue(pacchetto["url"].startswith("https://"))

    def test_le_variabili_dichiarate_sono_quelle_che_il_binario_imposta(self) -> None:
        """Il lock dichiara che certi percorsi assoluti cotti nei binari sono
        innocui **perche' una variabile li copre**. Quella variabile la imposta
        `radici.rs`, e i due elenchi vivono in file diversi.

        Se una riga sparisse da `radici.rs` -- o vi cambiasse nome -- la
        classificazione nel lock resterebbe verde affermando una copertura che
        non c'e' piu'. E' la stessa forma del difetto dei tre conteggi: due
        verita' che nessuno confronta."""
        rust = RADICI_RS.read_text(encoding="utf-8")
        impostate = set(re.findall(r'variabile: "([A-Z_]+)"', rust))
        impostate |= set(re.findall(r'^const CATALOGO_XML: &str = "([A-Z_]+)"', rust, re.M))
        self.assertTrue(impostate, "nessuna variabile letta da radici.rs")

        dichiarate: set[str] = set()
        for regola in self.lock["contratto_di_verifica"]["percorsi_assoluti_ammessi"]:
            if regola["categoria"] == "coperto_da_variabile":
                dichiarate |= {v.strip() for v in regola["variabile"].split(",")}

        self.assertEqual(
            dichiarate,
            impostate,
            "il lock e radici.rs non nominano le stesse variabili: "
            f"solo nel lock {sorted(dichiarate - impostate)}, "
            f"solo in radici.rs {sorted(impostate - dichiarate)}",
        )

    def test_ogni_variabile_ha_un_bersaglio_nel_layout_installato(self) -> None:
        """Una variabile che puntasse a una directory che l'artefatto non
        spedisce sarebbe peggio del default: il default si riconosce come tale,
        una nostra variabile rotta si legge come configurazione voluta.

        A runtime `radici.rs` tace su cio' che non trova -- non puo' fare
        altro, e tacere e' la scelta giusta. Che l'artefatto lo spedisca
        dev'essere quindi una pretesa **del pacchetto**, ed e' questa."""
        rust = RADICI_RS.read_text(encoding="utf-8")
        relativi = re.findall(r'relativo: "([^"]+)"', rust)
        self.assertTrue(relativi, "nessuna directory letta da radici.rs")

        dichiarate = [
            voce["percorso"].rstrip("/")
            for voce in self.matrice["layout_installato"]["voci"]
        ]
        for relativo in relativi:
            with self.subTest(relativo=relativo):
                self.assertTrue(
                    any(
                        relativo == d or relativo.startswith(d + "/")
                        for d in dichiarate
                    ),
                    f"«{relativo}» non ricade in nessuna voce del layout dichiarato: {dichiarate}",
                )

    def test_ogni_lock_dichiara_i_virtuali_con_cui_e_stato_risolto(self) -> None:
        """Un lock risolto contro `__glibc >=2.35` non e' lo stesso risolto
        contro `>=2.36`, e conda deduce quei valori da chi esegue il solver.

        Senza dichiararli, lo stesso comando su due macchine puo' produrre due
        chiusure diverse e il lock non direbbe quale delle due e' la sua --
        cioe' proprio la cosa che un lock esiste per escludere."""
        for piattaforma, percorso in sorted(LOCK_PER_PIATTAFORMA.items()):
            if not percorso.exists():
                continue
            with self.subTest(piattaforma=piattaforma):
                self.assertIn("virtuali_alla_risoluzione", carica(percorso))

    def test_nessun_documento_nomina_una_versione_gdal_diversa(self) -> None:
        """La sonda che avrebbe colto la contraddizione dove e' sopravvissuta.

        I lock e la matrice erano gia' confrontati fra loro; a divergere in
        silenzio sono stati i **documenti**, che continuavano a dire 3.10.3 e
        binding 3.6.0 dopo che il contratto era cambiato. La prosa non si
        riconcilia da sola, e una versione scritta a mano in un documento e'
        una copia come tutte le altre.

        Una versione diversa resta ammessa dove la riga dice che e' passata: un
        documento deve poter raccontare che cosa c'era prima, purche' lo
        dichiari invece di affermarlo al presente."""
        contratto = self.matrice["contratto_gdal"]["versione"]
        binding = self.matrice["contratto_gdal"]["binding_rust"]["binding_version_dichiarata"]
        ammesse = {contratto, binding}
        # Il lookahead esclude una cifra o un altro segmento di versione, non
        # un punto qualunque: `(?![\w.])` lasciava sfuggire ogni versione a fine
        # frase, dove il punto fermo segue subito. La controprova l'ha trovato.
        schema = re.compile(r"(?<![\w.])3\.\d+\.\d+(?!\w|\.\d)")
        storica = re.compile(r"\b(era|prima|precedent|non piu|storic|dichiarava|forzava|mascherava)", re.I)
        for documento in docset():
            if not documento.exists():
                continue
            for numero, riga in enumerate(documento.read_text(encoding="utf-8").splitlines(), 1):
                for versione in schema.findall(riga):
                    if versione in ammesse or storica.search(riga):
                        continue
                    self.fail(
                        f"{documento.name}:{numero} nomina {versione} mentre il contratto e' "
                        f"{contratto}: «{riga.strip()[:90]}». Se e' una versione storica, la riga "
                        "deve dirlo."
                    )

    def test_nessun_documento_promette_una_piattaforma_fuori_perimetro(self) -> None:
        """Una promessa verso chi installa non si ritira dai registri e si
        lascia nei documenti.

        macOS e' uscito dal perimetro della v1 come **decisione di prodotto**.
        Un documento che continuasse a promettere artefatti, installazione o
        qualifica per macOS direbbe a chi legge una cosa che il progetto non
        mantiene -- ed e' peggio di non averla mai detta, perche' ha l'aria di
        un impegno.

        La sonda ammette il nome dove la riga dice che e' fuori, o dove parla
        di cio' che **non** si promette: il perimetro va spiegato, non taciuto."""
        fuori = {p["id"] for p in self.matrice["piattaforme_non_distribuite"]}
        self.assertIn("macos-aarch64", fuori)

        promessa = re.compile(
            r"\b(si distribuisce|artefatt|installazion|qualific|supporto)", re.I
        )
        esclusione = re.compile(
            r"(fuori|non si promett|non e' |non è |escluso|esce|uscit|"
            r"non distribuit|perimetro|decision|scope|storia git|conserva)",
            re.I,
        )
        for documento in docset():
            if not documento.exists():
                continue
            righe = documento.read_text(encoding="utf-8").splitlines()
            for numero, riga in enumerate(righe, 1):
                if "macos" not in riga.lower() and "mac os" not in riga.lower():
                    continue
                if not promessa.search(riga):
                    continue
                intorno = " ".join(righe[max(0, numero - 3) : numero + 2])
                self.assertTrue(
                    esclusione.search(intorno),
                    f"{documento.name}:{numero} sembra promettere qualcosa per macOS, che e' "
                    f"fuori dal perimetro: «{riga.strip()[:100]}»",
                )

    def test_la_firma_non_pretende_piu_developer_id(self) -> None:
        """Developer ID, notarizzazione e stapling sono usciti con macOS.

        Erano il pezzo piu' costoso della catena -- un certificato Apple, un
        servizio esterno da interrogare, e una ricevuta che sul deliverable non
        si poteva nemmeno attaccare al file -- e con macOS fuori scope non
        c'e' piu' nulla da decidere li'."""
        import importlib.util

        percorso = RADICE / "scripts" / "distribuzione.py"
        spec = importlib.util.spec_from_file_location("distribuzione", percorso)
        modulo = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(modulo)

        distribuite = {p["id"] for p in self.matrice["piattaforme"]}
        for piattaforma in modulo.POLITICA_DI_FIRMA:
            if piattaforma not in distribuite:
                continue
            regola = modulo.POLITICA_DI_FIRMA[piattaforma].get("candidate", {})
            with self.subTest(piattaforma=piattaforma):
                self.assertNotEqual(regola.get("meccanismo"), "developer-id")
                self.assertFalse(regola.get("notarizzazione"))
                self.assertFalse(regola.get("stapling"))

    def test_l_ordine_delle_operazioni_e_lo_stesso_nei_due_posti(self) -> None:
        """La matrice lo **dichiara**, `distribuzione.py` lo fa **applicare**.

        Comparire in due posti e' inevitabile -- l'uno e' una promessa verso chi
        legge, l'altro e' cio' che i programmi seguono -- e per questo vanno
        confrontati."""
        import importlib.util

        percorso = RADICE / "scripts" / "distribuzione.py"
        spec = importlib.util.spec_from_file_location("distribuzione", percorso)
        modulo = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(modulo)

        dal_codice = [f"{n}. {passo}: {perche}" for n, (passo, perche) in enumerate(modulo.ORDINE, 1)]
        self.assertEqual(self.matrice["firma"]["ordine_delle_operazioni"], dal_codice)

    def test_nessun_insieme_atteso_e_largo(self) -> None:
        """Un insieme largo non si accorge di cio' che smette di essere spedito
        e viene preso dal sistema -- che e' il difetto per cui l'insieme esatto
        esiste. Niente `C:\Windows\*`, niente `PATH`, niente jolly."""
        larghi = re.compile(r"[*?]|windows\\|\$\{|%PATH%|qualunque", re.I)
        for piattaforma, percorso in sorted(LOCK_PER_PIATTAFORMA.items()):
            if not percorso.exists():
                continue
            contratto = carica(percorso).get("contratto_di_verifica")
            if contratto is None:
                continue
            for chiave, valore in contratto.items():
                if not chiave.startswith(("dipendenze_", "dll_")) or chiave.endswith("nota"):
                    continue
                per_profilo = valore if isinstance(valore, dict) else {"-": valore}
                for profilo, nomi in per_profilo.items():
                    for nome in nomi:
                        with self.subTest(piattaforma=piattaforma, profilo=profilo, nome=nome):
                            self.assertIsNone(
                                larghi.search(nome),
                                f"«{nome}» e' un insieme largo, non un nome",
                            )

    def test_ogni_contratto_ha_un_insieme_per_profilo(self) -> None:
        """`base` e `filegdb` sono due prodotti: un insieme solo per entrambi
        attribuirebbe a un artefatto una misura fatta su un altro."""
        for piattaforma, percorso in sorted(LOCK_PER_PIATTAFORMA.items()):
            if not percorso.exists():
                continue
            contratto = carica(percorso).get("contratto_di_verifica")
            if contratto is None:
                continue
            with self.subTest(piattaforma=piattaforma):
                per_profilo = next(
                    (
                        v
                        for k, v in contratto.items()
                        if k.startswith(("dipendenze_esterne_attese", "dll_di_sistema_attese"))
                        and isinstance(v, dict)
                    ),
                    None,
                )
                self.assertIsNotNone(
                    per_profilo, "l'insieme atteso non e' diviso per profilo"
                )
                self.assertEqual(set(per_profilo), {"base", "filegdb"})

    def test_un_contratto_dichiara_da_quale_rilievo_viene(self) -> None:
        """Il digest lega il contratto alla misura.

        Senza, un insieme atteso sarebbe una lista di nomi senza provenienza:
        chi la rilegge non saprebbe da quale artefatto, su quale runner, con
        quale lock e' stata ricavata -- e non potrebbe rifare il conto.

        La pretesa vale dove il contratto **non** e' stato scritto insieme al
        primo lock: Linux e' stato misurato prima che questa regola esistesse,
        e riscriverne la storia sarebbe peggio che dichiararlo."""
        for piattaforma in ("windows-x86_64", "macos-aarch64"):
            percorso = LOCK_PER_PIATTAFORMA[piattaforma]
            if not percorso.exists():
                continue
            contratto = carica(percorso).get("contratto_di_verifica")
            if contratto is None:
                continue  # non ancora misurato: e' il blocco registrato
            with self.subTest(piattaforma=piattaforma):
                self.assertIn(
                    "rilievo_di_origine",
                    contratto,
                    "il contratto non dice da quale rilievo viene",
                )
                origine = contratto["rilievo_di_origine"]
                # Un digest **per profilo**: `base` e `filegdb` sono due
                # prodotti e hanno due rilievi, e un digest solo attribuirebbe
                # a un artefatto una misura fatta su un altro.
                self.assertEqual(
                    set(origine["sha256"]),
                    {"base", "filegdb"},
                    "il rilievo di origine non e' diviso per profilo",
                )
                for profilo, digesto in sorted(origine["sha256"].items()):
                    with self.subTest(profilo=profilo):
                        self.assertEqual(len(digesto), 64)
                self.assertNotEqual(
                    origine["sha256"]["base"],
                    origine["sha256"]["filegdb"],
                    "due profili con lo stesso digest: uno dei due rilievi non e' il suo",
                )
                self.assertTrue(origine.get("sha_sorgente"))

    def test_la_guardia_dei_percorsi_e_legata_alla_condizione(self) -> None:
        """«Zero percorsi assoluti» e' sospetto solo se si spedisce qualcosa.

        Dopo la rilocazione di conda un percorso c'e' sempre nelle librerie
        materializzate, ed e' per questo che zero fa rosso. Ma un binario Rust
        che non linka nulla di conda non ne contiene nessuno, e li' zero e' il
        valore giusto: legare la guardia al profilo l'avrebbe resa
        un'etichetta, legarla a «ci sono librerie interne» la lega a cio' che
        la rende sensata.

        La sonda guarda la condizione nel sorgente perche' e' quella la
        decisione: un `if` sul profilo tornerebbe a essere un'ipotesi."""
        sorgente = CHECKER.read_text(encoding="utf-8")
        self.assertIn("if interne and not per_categoria", sorgente)

    def test_il_runtime_e_atteso_su_entrambi_i_profili(self) -> None:
        """L'ipotesi che la prima corsa di scoperta ha smentito.

        Su Linux il profilo base non spedisce librerie, perche' `libgcc_s` e
        `libm` sono garantite dal sistema; su Windows importa
        `vcruntime140.dll`, che **non** e' un componente del sistema operativo.
        Pretendere il referto solo dal profilo pieno avrebbe lasciato passare
        un artefatto base che dipende da un runtime che non spedisce."""
        import importlib.util

        percorso = RADICE / "scripts" / "check-distribuzione-completa.py"
        spec = importlib.util.spec_from_file_location("gate", percorso)
        gate = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(gate)
        self.assertIn("runtime", gate.attese_per("base"))
        self.assertIn("runtime", gate.attese_per("filegdb"))
        self.assertNotIn(
            "elf_spediti",
            gate.VERIFICHE_ATTESE["runtime"]["misure_obbligatorie"],
            "un nome che nomina un formato in un contratto comune e' un'ipotesi travestita",
        )

    def test_lo_strumento_che_risolve_e_fissato(self) -> None:
        """Uno strumento che cambia da solo rende non riproducibile cio' che
        produce -- e cio' che produce e' proprio l'elenco che il lock fissa."""
        risolto = self.lock["risolto_con"]
        self.assertEqual(len(risolto["sha256"]), 64)
        self.assertTrue(risolto["url"].startswith("https://"))
        self.assertIsInstance(risolto["dimensione"], int)


if __name__ == "__main__":
    unittest.main()
