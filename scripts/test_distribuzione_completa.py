"""Sonde sul gate finale e sulla politica di firma.

# Perche' il gate finale ha bisogno di sonde proprie

Perche' e' l'unica cosa che sta fra sei job e la conclusione «gli artefatti sono
verificati». Se il gate avesse un buco, il buco non lo troverebbe nessuno: i job
sarebbero verdi, il gate sarebbe verde, e la conclusione sarebbe falsa senza che
niente diventi rosso.

Le sonde lo mettono alla prova su referti costruiti apposta per essere
sbagliati, uno sbaglio alla volta.
"""

from __future__ import annotations

import importlib.util
import json
import pathlib
import shutil
import tempfile
import unittest

RADICE = pathlib.Path(__file__).resolve().parent.parent
GATE = RADICE / "scripts" / "check-distribuzione-completa.py"


def carica(percorso: pathlib.Path):
    spec = importlib.util.spec_from_file_location(percorso.stem.replace("-", "_"), percorso)
    modulo = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(modulo)
    return modulo


class SondeDelGateFinale(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        self.gate = carica(GATE)
        self.distribuzione = carica(RADICE / "scripts" / "distribuzione.py")

    def referto(self, piattaforma: str, profilo: str, verifica: str, **campi) -> None:
        misure = {
            # Anche il profilo base ha un referto `runtime`: su Windows
            # spedisce il runtime C, e su Linux dimostra con un conteggio che
            # non spedisce niente. Era un'ipotesi ereditata da Linux, e la
            # prima corsa di scoperta l'ha smentita.
            "runtime": {
                "binari_spediti": 56 if profilo == "filegdb" else 1,
                "dipendenze_esterne": ["libc.so.6"],
                "percorsi_assoluti_classificati": 29,
            },
            "licenze-artefatto": {"componenti_con_testo": 43},
            "relocation": {"librerie_dall_albero": 57 if profilo == "filegdb" else 1},
            "digest-manifesto": {
                "file_dichiarati": 145,
                "file_verificati": 145,
                "digest_divergenti": 0,
            },
            "provenance": {
                "archivio_sha256": "a" * 64,
                "revisione": "b" * 40,
                "lock_sha256": "c" * 64,
            },
            "smoke-profilo": {
                "filegdb_assente": True if profilo == "base" else None,
                "schema_riletto": True if profilo == "filegdb" else None,
                "firma": {"stato": "non_richiesta"},
            },
        }[verifica]
        documento = {
            "schema_referto": self.distribuzione.SCHEMA_REFERTO,
            "verifica": verifica,
            "piattaforma": piattaforma,
            "profilo": profilo,
            "canale": "prova",
            "esito": "verde",
            "misure": misure,
            "errori": [],
            "note": None,
        }
        documento.update(campi)
        nome = f"{piattaforma}-{profilo}-{verifica}.json"
        (self.tmp / nome).write_text(
            json.dumps(documento, ensure_ascii=False), encoding="utf-8"
        )

    def serie_completa(self, piattaforma: str = "linux-x86_64", canale: str = "prova") -> None:
        for profilo in self.gate.PROFILI:
            for verifica in self.gate.attese_per(profilo):
                self.referto(piattaforma, profilo, verifica, canale=canale)

    def esegui(self, canale: str = "prova", piattaforme=("linux-x86_64",)) -> list[str]:
        return self.gate.verifica(self.tmp, canale, piattaforme)

    def test_una_serie_completa_passa(self) -> None:
        self.serie_completa()
        self.assertEqual(self.esegui(), [])

    def test_un_referto_mancante_fa_rosso(self) -> None:
        """Un referto assente non e' un'omissione da tollerare: e' la
        differenza fra «verificato» e «non verificato»."""
        self.serie_completa()
        (self.tmp / "linux-x86_64-filegdb-relocation.json").unlink()
        errori = self.esegui()
        self.assertTrue(errori)
        self.assertIn("relocation", " ".join(errori))

    def test_una_misura_mancante_fa_rosso_anche_con_esito_verde(self) -> None:
        """Il cuore del gate.

        Un esito verde senza la misura che lo sostiene e' un'affermazione, ed e'
        esattamente la forma che ha un job verde per la ragione sbagliata: uno
        smoke che non ha trovato l'artefatto, un passo saltato per una
        condizione mai vera."""
        self.serie_completa()
        percorso = self.tmp / "linux-x86_64-filegdb-runtime.json"
        d = json.loads(percorso.read_text(encoding="utf-8"))
        del d["misure"]["binari_spediti"]
        percorso.write_text(json.dumps(d), encoding="utf-8")
        errori = self.esegui()
        self.assertTrue(errori)
        self.assertIn("binari_spediti", " ".join(errori))

    def test_il_profilo_base_deve_dimostrare_filegdb_assente(self) -> None:
        """Non basta che il base non usi FileGDB: deve dimostrare che manca.

        Un `base` costruito per sbaglio con la feature attiva porterebbe un
        runtime GDAL che il suo contratto non prevede, e nulla nel nome lo
        direbbe."""
        self.serie_completa()
        percorso = self.tmp / "linux-x86_64-base-smoke-profilo.json"
        d = json.loads(percorso.read_text(encoding="utf-8"))
        d["misure"]["filegdb_assente"] = False
        percorso.write_text(json.dumps(d), encoding="utf-8")
        errori = self.esegui()
        self.assertTrue(errori)
        self.assertIn("filegdb_assente", " ".join(errori))

    def test_una_provenance_senza_revisione_passa_su_una_prova(self) -> None:
        """Gli artefatti di prova esistono per essere misurati, e pretendere una
        revisione da una macchina senza `git` renderebbe impossibile
        costruirli. Resta **dichiarata**: `null` e' un valore, e chi legge sa
        distinguere una revisione assente da una sbagliata."""
        self.serie_completa()
        percorso = self.tmp / "linux-x86_64-filegdb-provenance.json"
        d = json.loads(percorso.read_text(encoding="utf-8"))
        d["misure"]["revisione"] = None
        percorso.write_text(json.dumps(d), encoding="utf-8")
        self.assertEqual(self.esegui(canale="prova"), [])

    def test_una_provenance_senza_revisione_fa_rosso_su_una_candidate(self) -> None:
        """Una provenance che non sa da quale revisione viene non lega niente:
        dice che esiste un archivio con un checksum, e quello lo dice gia' il
        checksum."""
        self.serie_completa(canale="candidate")
        percorso = self.tmp / "linux-x86_64-filegdb-provenance.json"
        d = json.loads(percorso.read_text(encoding="utf-8"))
        d["misure"]["revisione"] = None
        percorso.write_text(json.dumps(d), encoding="utf-8")
        errori = self.esegui(canale="candidate")
        self.assertTrue(errori)
        self.assertIn("revisione", " ".join(errori))

    def test_un_referto_di_prova_non_qualifica_una_candidate(self) -> None:
        self.serie_completa(canale="prova")
        errori = self.esegui(canale="candidate")
        self.assertTrue(errori)
        self.assertIn("prova", " ".join(errori))

    def test_una_piattaforma_senza_referti_fa_rosso(self) -> None:
        """Sei artefatti sono sei, e il gate li conta."""
        self.serie_completa("linux-x86_64")
        errori = self.esegui(piattaforme=("linux-x86_64", "windows-x86_64"))
        self.assertTrue(errori)
        self.assertIn("windows-x86_64", " ".join(errori))

    def test_due_referti_per_la_stessa_verifica_fanno_rosso(self) -> None:
        """Sceglierne uno sarebbe una decisione presa in silenzio."""
        self.serie_completa()
        d = json.loads(
            (self.tmp / "linux-x86_64-base-licenze-artefatto.json").read_text(encoding="utf-8")
        )
        (self.tmp / "un-altro-nome.json").write_text(json.dumps(d), encoding="utf-8")
        self.assertTrue(self.esegui())

    def test_un_referto_di_un_altro_schema_non_si_riconta(self) -> None:
        self.serie_completa()
        percorso = self.tmp / "linux-x86_64-filegdb-runtime.json"
        d = json.loads(percorso.read_text(encoding="utf-8"))
        d["schema_referto"] = 99
        percorso.write_text(json.dumps(d), encoding="utf-8")
        self.assertTrue(self.esegui())


class SondeDellaFirma(unittest.TestCase):
    """La decisione che doveva essere presa prima dei workflow.

    Inserirla dopo avrebbe cambiato byte, checksum, manifesti e provenance: il
    campo deve esistere prima del certificato, e l'ordine delle operazioni --
    assembla, firma, checksum, smoke -- deve essere gia' quello giusto.
    """

    def setUp(self) -> None:
        self.d = carica(RADICE / "scripts" / "distribuzione.py")
        self.gate = carica(GATE)
        self.tmp = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)

    def misura_completa(self, piattaforma: str) -> dict:
        pretese = self.d.POLITICA_DI_FIRMA[piattaforma]["candidate"]["misure_pretese"]
        return {p: f"valore per {p}" for p in pretese}

    def test_gli_artefatti_di_prova_non_sono_firmati(self) -> None:
        """Pretendere un certificato per costruire un artefatto di misura
        renderebbe impossibile lavorare senza segreti."""
        for piattaforma in ("linux-x86_64", "windows-x86_64"):
            with self.subTest(piattaforma=piattaforma):
                stato = self.d.stato_della_firma(piattaforma, "prova")
                self.assertEqual(stato["stato"], "non_richiesta")

    def test_senza_misura_lo_stato_non_e_un_si(self) -> None:
        """La correzione che conta.

        Prima lo stato veniva da un booleano «il materiale c'era», che diceva
        soltanto che il costruttore aveva avuto un certificato fra le mani. Ora
        viene da cio' che i verificatori nativi hanno **letto sui byte
        finali**, e non aver guardato ha uno stato proprio: `non_misurata` non
        e' `assente` e non e' `apposta`."""
        for piattaforma in ("windows-x86_64",):
            with self.subTest(piattaforma=piattaforma):
                stato = self.d.stato_della_firma(piattaforma, "candidate", misura=None)
                self.assertEqual(stato["stato"], "non_misurata")
                self.assertTrue(stato["misure_pretese"])

    def test_una_misura_incompleta_e_assente_non_apposta(self) -> None:
        """Una firma senza timestamp smette di valere quando scade il
        certificato, invece che quando scade il suo uso: manca qualcosa di
        preteso, e lo stato lo dice."""
        misura = self.misura_completa("windows-x86_64")
        del misura["timestamp"]
        stato = self.d.stato_della_firma("windows-x86_64", "candidate", misura=misura)
        self.assertEqual(stato["stato"], "assente")
        self.assertIn("timestamp", stato["mancanti"])

    def test_una_misura_completa_e_apposta(self) -> None:
        for piattaforma, meccanismo in (
            ("windows-x86_64", "authenticode"),
        ):
            with self.subTest(piattaforma=piattaforma):
                stato = self.d.stato_della_firma(
                    piattaforma, "candidate", misura=self.misura_completa(piattaforma)
                )
                self.assertEqual(stato["stato"], "apposta")
                self.assertEqual(stato["meccanismo"], meccanismo)
                self.assertEqual(stato["mancanti"], [])

    def test_il_contenitore_e_quello_della_piattaforma(self) -> None:
        """`tar.gz` su Linux, `zip` su Windows: un tar.gz non e' un formato che
        gli strumenti Windows aprano senza aiuto, e chi installa non deve
        procurarsi uno strumento per leggere un artefatto."""
        self.assertEqual(self.d.contenitore("linux-x86_64"), "tar.gz")
        self.assertEqual(self.d.contenitore("windows-x86_64"), "zip")

    def test_una_piattaforma_fuori_perimetro_non_ha_un_contenitore(self) -> None:
        """macOS e' fuori scope: chiedere il suo contenitore e' chiedere di un
        artefatto che la v1 non produce."""
        with self.assertRaises(SystemExit):
            self.d.contenitore("macos-aarch64")

    def test_linux_dichiara_di_non_avere_un_meccanismo(self) -> None:
        """Dichiararlo invece di lasciarlo implicito e' la differenza fra «non
        serve» e «ce ne siamo dimenticati»."""
        stato = self.d.stato_della_firma("linux-x86_64", "candidate")
        self.assertEqual(stato["stato"], "non_richiesta")
        self.assertTrue(stato["perche"])

    def test_una_piattaforma_sconosciuta_non_passa_in_silenzio(self) -> None:
        with self.assertRaises(SystemExit):
            self.d.stato_della_firma("solaris-sparc", "candidate")

    def test_l_ordine_completo_e_dichiarato(self) -> None:
        """Otto passi, e ognuno dipende dai byte del precedente.

        Il manifesto viene **dopo** la firma: scritto prima elencherebbe file
        che non esistono piu'. I checksum vengono dopo la notarizzazione, lo
        smoke dopo i checksum, e la provenance lega quel checksum."""
        passi = [p for p, _ in self.d.ORDINE]
        self.assertEqual(
            passi,
            [
                "payload",
                "firma",
                "manifesto",
                "archivio",
                "notarizzazione",
                "checksum",
                "smoke",
                "provenance",
            ],
        )

    def test_una_candidate_senza_firma_fa_rosso_al_gate(self) -> None:
        for profilo in self.gate.PROFILI:
            for verifica in self.gate.attese_per(profilo):
                misure = (
                    {"firma": self.d.stato_della_firma("windows-x86_64", "candidate")}
                    if verifica == "smoke-profilo"
                    else {}
                )
                if verifica == "smoke-profilo":
                    misure["filegdb_assente" if profilo == "base" else "schema_riletto"] = True
                for obbligatoria in self.gate.VERIFICHE_ATTESE[verifica]["misure_obbligatorie"]:
                    misure[obbligatoria] = 1
                (self.tmp / f"{profilo}-{verifica}.json").write_text(
                    json.dumps(
                        {
                            "schema_referto": self.d.SCHEMA_REFERTO,
                            "verifica": verifica,
                            "piattaforma": "windows-x86_64",
                            "profilo": profilo,
                            "canale": "candidate",
                            "esito": "verde",
                            "misure": misure,
                            "errori": [],
                        }
                    ),
                    encoding="utf-8",
                )
        errori = self.gate.verifica(self.tmp, "candidate", ("windows-x86_64",))
        self.assertTrue(errori)
        self.assertIn("authenticode", " ".join(errori))

    def test_uno_smoke_prima_della_firma_fa_rosso(self) -> None:
        """Un binario firmato e' un altro file: su macOS notarizzato e con lo
        stapling, su Windows con una sezione in piu'. Lo smoke va rifatto."""
        for profilo in self.gate.PROFILI:
            for verifica in self.gate.attese_per(profilo):
                misure = {}
                if verifica == "smoke-profilo":
                    misure["firma"] = {
                        **self.d.stato_della_firma(
                            "windows-x86_64",
                            "candidate",
                            misura=self.misura_completa("windows-x86_64"),
                        ),
                        "smoke_prima_della_firma": True,
                    }
                    misure["filegdb_assente" if profilo == "base" else "schema_riletto"] = True
                for obbligatoria in self.gate.VERIFICHE_ATTESE[verifica]["misure_obbligatorie"]:
                    misure[obbligatoria] = 1
                (self.tmp / f"{profilo}-{verifica}.json").write_text(
                    json.dumps(
                        {
                            "schema_referto": self.d.SCHEMA_REFERTO,
                            "verifica": verifica,
                            "piattaforma": "windows-x86_64",
                            "profilo": profilo,
                            "canale": "candidate",
                            "esito": "verde",
                            "misure": misure,
                            "errori": [],
                        }
                    ),
                    encoding="utf-8",
                )
        errori = self.gate.verifica(self.tmp, "candidate", ("windows-x86_64",))
        self.assertTrue(errori)
        self.assertIn("prima della firma", " ".join(errori))


class SondeDelPerimetro(unittest.TestCase):
    """Il perimetro viene da una decisione, non dall'assenza di un job.

    La riduzione da sei artefatti a quattro e' la conseguenza di una scelta --
    macOS e' fuori dallo scope dei deployment server della v1 -- e non di un
    job che qualcuno ha tolto. Nel conteggio le due cose si somigliano; per il
    resto non si somigliano per niente, e queste sonde esistono per non farle
    confondere.
    """

    def setUp(self) -> None:
        self.gate = carica(GATE)
        self.matrice_percorso = (
            RADICE / "assurance" / "registries" / "distribuzione-matrice.json"
        )
        self.originale = self.matrice_percorso.read_bytes()
        self.addCleanup(self.matrice_percorso.write_bytes, self.originale)

    def scrivi(self, matrice: dict) -> None:
        self.matrice_percorso.write_bytes(
            json.dumps(matrice, ensure_ascii=False, indent=2).encode("utf-8")
        )

    def test_il_perimetro_e_due_piattaforme_e_una_fuori(self) -> None:
        distribuite, fuori = self.gate.perimetro()
        self.assertEqual(set(distribuite), {"linux-x86_64", "windows-x86_64"})
        self.assertEqual(set(fuori), {"macos-aarch64"})

    def test_ogni_piattaforma_nota_sta_in_una_delle_due_liste(self) -> None:
        """Se una uscisse da entrambe, sarebbe uscita dal perimetro senza che
        nessuno lo dicesse -- ed e' esattamente il caso che il gate non deve
        lasciar passare."""
        distribuite, fuori = self.gate.perimetro()
        self.assertEqual(
            set(self.gate.PIATTAFORME_NOTE),
            set(distribuite) | set(fuori),
        )

    def test_una_piattaforma_sparita_ferma_il_gate(self) -> None:
        """La sonda che il committente ha chiesto.

        Togliere macOS dalle distribuite **senza** dichiararla fuori scope
        ridurrebbe gli artefatti attesi esattamente come la decisione, e il
        conteggio tornerebbe. Non deve tornare: il gate si ferma e dice che
        manca una decisione."""
        matrice = json.loads(self.originale)
        del matrice["piattaforme_non_distribuite"]
        self.scrivi(matrice)
        with self.assertRaises(SystemExit) as contesto:
            self.gate.perimetro()
        self.assertIn("senza decisione", str(contesto.exception))
        self.assertIn("macos-aarch64", str(contesto.exception))

    def test_una_decisione_senza_motivazione_non_e_una_decisione(self) -> None:
        """Dichiararlo costa dire perche'. Un campo vuoto sarebbe un modo di
        non prendere la decisione facendo finta di averla presa."""
        for campo in ("decisione", "perche", "che_cosa_la_ribalterebbe"):
            with self.subTest(campo=campo):
                matrice = json.loads(self.originale)
                matrice["piattaforme_non_distribuite"][0][campo] = ""
                self.scrivi(matrice)
                with self.assertRaises(SystemExit) as contesto:
                    self.gate.perimetro()
                self.assertIn(campo, str(contesto.exception))

    def test_non_si_puo_pretendere_una_piattaforma_fuori_perimetro(self) -> None:
        """Chiedere referti a macOS ora sarebbe chiedere una promessa che la v1
        non fa."""
        distribuite, _ = self.gate.perimetro()
        self.assertNotIn("macos-aarch64", distribuite)

    def test_gli_artefatti_attesi_sono_quattro(self) -> None:
        """Due piattaforme per due profili. E' il numero che la matrice
        dichiara, e viene dalla stessa fonte da cui il gate lo calcola."""
        distribuite, _ = self.gate.perimetro()
        self.assertEqual(len(distribuite) * len(self.gate.PROFILI), 4)
        matrice = json.loads(self.originale)
        self.assertIn("quattro", matrice["perimetro"]["artefatti_attesi"])


class SondeDelGateNelWorkflow(unittest.TestCase):
    """Che il gate finale **giri**, e non soltanto esista.

    E' il difetto che c'e' stato: il gate era scritto, aveva le proprie sonde,
    e nel workflow non c'era nessun job che lo eseguisse. Quattro job verdi si
    leggevano come «la distribuzione e' completa», che e' un'altra
    affermazione: ogni job sa di se stesso, e nessuno contava se i quattro
    artefatti attesi ci fossero tutti con tutte le loro verifiche.

    Un referto mancante non rende rosso nessun job -- semplicemente non c'e' --
    ed e' la forma di falso verde piu' facile da non vedere, perche' si
    manifesta come assenza. Uno strumento che nessuno esegue e' la stessa cosa,
    un gradino piu' in su.
    """

    def setUp(self) -> None:
        self.testo = (RADICE / ".github" / "workflows" / "distribuzione.yml").read_text(
            encoding="utf-8"
        )

    def test_esiste_un_job_che_esegue_il_gate(self) -> None:
        self.assertRegex(self.testo, r"(?m)^  gate:$", "nessun job `gate` nel workflow")
        self.assertIn(
            "scripts/check-distribuzione-completa.py --referti",
            self.testo,
            "il job `gate` non esegue il gate finale",
        )

    def test_il_gate_aspetta_entrambi_i_costruttori(self) -> None:
        """Un gate che girasse prima conterebbe i referti di una corsa a
        meta', e li troverebbe mancanti per la ragione sbagliata."""
        self.assertIn("needs: [linux, windows]", self.testo)

    def test_un_costruttore_rosso_non_diventa_una_distribuzione_verde(self) -> None:
        """`if: always()` serve a far parlare il gate anche quando qualcosa e'
        fallito. Senza leggere l'esito dei costruttori, pero', trasformerebbe
        un costruttore rosso in una corsa verde -- che e' il contrario di cio'
        per cui esiste."""
        self.assertIn("needs.linux.result", self.testo)
        self.assertIn("needs.windows.result", self.testo)

    def test_i_referti_arrivano_tutti_al_gate(self) -> None:
        """Il pattern deve prendere ogni artefatto di referti: prenderne una
        parte darebbe un conteggio incompleto che il gate leggerebbe come
        assenza."""
        self.assertIn("pattern: referti-*", self.testo)


class SondeDelRuntimeNativo(unittest.TestCase):
    """Che il campo dica quello che il suo nome promette.

    `runtime_nativo` era `{"presente": false}` sul profilo base e voleva dire
    «non spedisce GDAL». Diceva pero' «non spedisce runtime nativo», che sul
    base Windows e' falso: `vcruntime140.dll` la spedisce, e la prima corsa di
    scoperta l'ha trovata proprio perche' non spedirla era un difetto.

    Un campo il cui nome promette piu' di quanto misura non fa rosso da nessuna
    parte: chi lo legge conclude qualcosa, la conclusione e' sbagliata, e
    nessun controllo se ne accorge perche' il campo *e'* coerente con se stesso.
    """

    def setUp(self) -> None:
        self.d = carica(RADICE / "scripts" / "distribuzione.py")

    def file(self, *nomi: str) -> list[dict]:
        return [{"percorso": f"bin/{n}", "sha256": "0" * 64, "byte": 1} for n in nomi]

    def test_il_base_windows_dichiara_il_runtime_c_che_spedisce(self) -> None:
        gdal = {"presente": False, "perche": "profilo base"}
        r = self.d.runtime_nativo(
            "windows-x86_64", self.file("plenora-io.exe", "vcruntime140.dll"), gdal
        )
        self.assertFalse(r["gdal"]["presente"])
        self.assertTrue(
            r["c_ridistribuibile"]["presente"],
            "il base Windows spedisce vcruntime140.dll: dichiararlo assente e' falso",
        )
        self.assertEqual(r["c_ridistribuibile"]["file"], ["vcruntime140.dll"])

    def test_il_base_linux_non_ne_dichiara_nessuno(self) -> None:
        """Il rovescio: dove non si spedisce niente, il campo dice zero e non
        sparisce. Una piattaforma assente dalla tabella si leggerebbe come «non
        ci ho pensato»."""
        r = self.d.runtime_nativo(
            "linux-x86_64", self.file("plenora-io"), {"presente": False}
        )
        self.assertFalse(r["c_ridistribuibile"]["presente"])
        self.assertEqual(r["c_ridistribuibile"]["file"], [])

    def test_i_due_componenti_sono_indipendenti(self) -> None:
        """GDAL c'e' solo nel profilo pieno; il runtime C c'e' su Windows in
        tutti e due. Un campo solo non puo' dire due cose diverse."""
        gdal = {"presente": True, "versione": "3.9.3"}
        r = self.d.runtime_nativo(
            "windows-x86_64",
            self.file("plenora-io.exe", "gdal.dll", "vcruntime140.dll", "msvcp140.dll"),
            gdal,
        )
        self.assertTrue(r["gdal"]["presente"])
        self.assertEqual(
            r["c_ridistribuibile"]["file"], ["msvcp140.dll", "vcruntime140.dll"]
        )

    def test_si_misura_dai_file_e_non_dal_profilo(self) -> None:
        """Il profilo e' cio' che volevamo costruire; l'elenco dei file e' cio'
        che abbiamo costruito, ed e' l'unico dei due che si accorge di un passo
        saltato. Se il runtime C sparisse dal payload, il campo lo direbbe."""
        r = self.d.runtime_nativo(
            "windows-x86_64", self.file("plenora-io.exe"), {"presente": True}
        )
        self.assertFalse(r["c_ridistribuibile"]["presente"])

    def test_il_verificatore_pe_legge_la_stessa_tabella(self) -> None:
        """Tre copie a mano divergerebbero, e la divergenza non farebbe rosso:
        il verificatore rifiuterebbe una DLL che il manifesto non nomina, o il
        contrario, e ciascuno dei due sarebbe internamente coerente."""
        verificatore = carica(RADICE / "scripts" / "check-windows-runtime.py")
        self.assertEqual(
            verificatore.DA_SPEDIRE_NON_AMMETTERE,
            self.d.RUNTIME_C_RIDISTRIBUIBILE["windows-x86_64"],
        )
        # L'uguaglianza da sola non basta: due copie a mano sono uguali finche'
        # qualcuno non tocca una delle due. Si pretende che **derivi**.
        sorgente = (RADICE / "scripts" / "check-windows-runtime.py").read_text(
            encoding="utf-8"
        )
        self.assertIn(
            'distribuzione.RUNTIME_C_RIDISTRIBUIBILE["windows-x86_64"]', sorgente
        )
        self.assertNotIn(
            '"vcruntime140.dll":', sorgente, "l'elenco e' tornato a essere una copia"
        )

    def test_la_matrice_non_torna_a_un_booleano(self) -> None:
        """La matrice portava `contiene_runtime_nativo`, con lo stesso difetto
        del manifesto e nello stesso verso."""
        matrice = json.loads(
            (
                RADICE / "assurance" / "registries" / "distribuzione-matrice.json"
            ).read_text(encoding="utf-8")
        )
        for profilo in matrice["profili"]:
            with self.subTest(profilo=profilo["id"]):
                self.assertNotIn("contiene_runtime_nativo", profilo)
                self.assertIn("gdal", profilo["runtime_nativo"])
                self.assertIn("c_ridistribuibile", profilo["runtime_nativo"])


if __name__ == "__main__":
    unittest.main()
