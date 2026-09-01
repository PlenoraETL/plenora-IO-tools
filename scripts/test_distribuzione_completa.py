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
            "runtime": {
                "elf_spediti": 56,
                "dipendenze_esterne": ["libc.so.6"],
                "percorsi_assoluti_classificati": 29,
            },
            "licenze-artefatto": {"componenti_con_testo": 43},
            "relocation": {"librerie_dall_albero": 57},
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
        del d["misure"]["elf_spediti"]
        percorso.write_text(json.dumps(d), encoding="utf-8")
        errori = self.esegui()
        self.assertTrue(errori)
        self.assertIn("elf_spediti", " ".join(errori))

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
        for piattaforma in ("linux-x86_64", "windows-x86_64", "macos-aarch64"):
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
        for piattaforma in ("windows-x86_64", "macos-aarch64"):
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
            ("macos-aarch64", "developer-id"),
        ):
            with self.subTest(piattaforma=piattaforma):
                stato = self.d.stato_della_firma(
                    piattaforma, "candidate", misura=self.misura_completa(piattaforma)
                )
                self.assertEqual(stato["stato"], "apposta")
                self.assertEqual(stato["meccanismo"], meccanismo)
                self.assertEqual(stato["mancanti"], [])

    def test_macos_pretende_la_notarizzazione_e_non_lo_stapling(self) -> None:
        """La correzione: Apple notarizza uno ZIP, ma `stapler` attacca la
        ricevuta solo ad app bundle, DMG e PKG. Il deliverable e' uno ZIP di
        una CLI rilocabile, quindi niente stapling -- e **la prima verifica di
        Gatekeeper richiedera' rete**. Va detto a chi installa, invece che
        lasciato scoprire a lui."""
        stato = self.d.stato_della_firma(
            "macos-aarch64", "candidate", misura=self.misura_completa("macos-aarch64")
        )
        self.assertTrue(stato["notarizzazione"])
        self.assertFalse(stato["stapling"])
        self.assertEqual(stato["smoke_dopo"], "la notarizzazione")
        self.assertIn("rete", stato["perche_niente_stapling"])
        self.assertIn("notarizzato", stato["misure_pretese"])

    def test_il_contenitore_macos_e_uno_zip(self) -> None:
        """La notarizzazione accetta ZIP; un tar.gz non e' un formato che gli
        strumenti Apple sappiano ispezionare."""
        self.assertEqual(self.d.contenitore("macos-aarch64"), "zip")
        self.assertEqual(self.d.contenitore("linux-x86_64"), "tar.gz")

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
                            "macos-aarch64",
                            "candidate",
                            misura=self.misura_completa("macos-aarch64"),
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
                            "piattaforma": "macos-aarch64",
                            "profilo": profilo,
                            "canale": "candidate",
                            "esito": "verde",
                            "misure": misure,
                            "errori": [],
                        }
                    ),
                    encoding="utf-8",
                )
        errori = self.gate.verifica(self.tmp, "candidate", ("macos-aarch64",))
        self.assertTrue(errori)
        self.assertIn("prima della firma", " ".join(errori))


if __name__ == "__main__":
    unittest.main()
