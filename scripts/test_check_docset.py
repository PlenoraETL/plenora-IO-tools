"""Sonde del gate del docset.

Il gate ha sette doveri, e ciascuno puo' fallire in un modo diverso. Quello che
queste sonde fissano non e' che il docset attuale passi — lo si vede eseguendo
il gate — ma che **ogni controllo diventi rosso quando deve**.

L'eccezione piu' delicata e' che il gate stesso legge i documenti: li verifica,
non ne dipende. La sonda la fissa, cosi' estenderla ad altri script richiede di
scriverlo.

# Le sonde del perimetro non toccano questo repository

Provare che gli untracked entrino nel perimetro richiede un file untracked. Un
file creato qui dentro cambierebbe l'impronta dell'albero mentre il checkpoint
la sta misurando, e un checkpoint che si guasta da solo non e' una misura.
Le sonde costruiscono percio' un repository vero in una directory temporanea e
vi puntano `ROOT`: e' Git a rispondere, sul suo stesso comportamento, senza che
nulla appaia accanto ai sorgenti.
"""

from __future__ import annotations

import json
import pathlib
import subprocess
import tempfile
import re
import unittest
from unittest import mock

from scripts import check_docset as gate
from scripts import stato_release


class RepositoryFinto:
    """Un repository Git vero, usa e getta."""

    def __enter__(self) -> "RepositoryFinto":
        self._directory = tempfile.TemporaryDirectory()
        self.radice = pathlib.Path(self._directory.name)
        for comando in (
            ["git", "init", "-q"],
            ["git", "config", "user.email", "sonda@esempio"],
            ["git", "config", "user.name", "sonda"],
        ):
            subprocess.run(comando, cwd=self.radice, check=True)
        (self.radice / "scripts").mkdir()
        (self.radice / "docs").mkdir()
        return self

    def __exit__(self, *_: object) -> None:
        self._directory.cleanup()

    def committa(self, relativo: str, contenuto: str) -> None:
        (self.radice / relativo).write_text(contenuto, encoding="utf-8")
        subprocess.run(["git", "add", relativo], cwd=self.radice, check=True)
        subprocess.run(["git", "commit", "-qm", relativo], cwd=self.radice, check=True)

    def scrivi(self, relativo: str, contenuto: str) -> None:
        percorso = self.radice / relativo
        percorso.parent.mkdir(parents=True, exist_ok=True)
        percorso.write_text(contenuto, encoding="utf-8")


class SondePerimetro(unittest.TestCase):
    def test_l_allowlist_ha_dodici_voci(self) -> None:
        """Il conteggio e' la difesa: l'allowlist si allarga per decisione.

        Cinque canonici e sette operativi. Sono cresciuti due volte il
        2026-09-04: con il terzo fork governato, che ridistribuisce anche un
        CHANGELOG e la propria licenza in Markdown -- contenuto di terzi, la cui
        integrita' la verifica il gate del fork sull'albero intero e non questo
        -- e con il README dell'SDK Python, che `pyproject.toml` dichiara
        `readme` e che finisce nei metadati del pacchetto: la stessa convenzione
        di percorso dei manifesti Cargo.

        Il 2026-09-05 e' cresciuto il lato **canonico**, che fino ad allora non
        si era mai mosso: `docs/INSTALL.md` risponde alla domanda di chi riceve
        il prodotto -- che cosa scarico, come verifico che sia arrivato intero,
        che cosa riscrivo passando dalla 1.x -- e gli altri quattro rispondono
        a domande di chi ci lavora. Che questa sonda sia diventata rossa e'
        precisamente il suo mestiere: un quinto documento canonico e' una
        decisione, e passa di qui.
        """
        self.assertEqual(len(gate.AMMESSI), 12)
        self.assertEqual(len(gate.CANONICI), 5)
        self.assertEqual(len(gate.OPERATIVI), 7)

    def test_un_nome_vivo_altrove_non_va_al_bando(self) -> None:
        """Il falso positivo che l'SDK ha rivelato.

        `_nomi_eliminati` confronta **nomi base** con **percorsi** ammessi, e i
        due non si incontrano mai: un `README.md` sparito da una sottodirectory
        metteva al bando il nome, e con esso ogni `readme = "README.md"` fuori
        da `vendor/` -- cioe' il modo in cui un manifesto Python o Cargo
        dichiara il proprio. E' lo stesso riguardo che le cartelle avevano gia'
        e ai nomi non era stato dato.
        """
        eliminati = gate._nomi_eliminati()
        for vivo in {"README.md", "CHANGELOG.md", "LICENSE.md"}:
            with self.subTest(nome=vivo):
                self.assertNotIn(re.escape(vivo), eliminati)

    def test_i_nomi_davvero_spariti_restano_al_bando(self) -> None:
        """La controprova: senza, «nessun falso positivo» sarebbe vero anche di
        un elenco vuoto."""
        eliminati = gate._nomi_eliminati()
        self.assertIn(re.escape("TRACEABILITY.md"), eliminati)
        self.assertIn(re.escape("IMPLEMENTATION_STATUS.md"), eliminati)

    def test_ogni_file_operativo_dichiara_la_propria_ragione(self) -> None:
        """Un'eccezione senza ragione e' un'eccezione permanente senza dirlo."""
        for percorso, ragione in gate.OPERATIVI.items():
            self.assertTrue(ragione.strip(), percorso)

    def test_i_canonici_e_gli_operativi_non_si_sovrappongono(self) -> None:
        self.assertEqual(set(gate.CANONICI) & set(gate.OPERATIVI), set())


class SondeUntracked(unittest.TestCase):
    """Il perimetro e' tracciati **unione** untracked non ignorati.

    La finestra che questo chiude si apriva esattamente dove serviva chiuderla:
    un nuovo lettore di Markdown, o un nuovo documento, e' untracked fino al
    primo `git add`, e chi lo scrive lancia il livello 1 **prima** di
    committare. Il gate sarebbe stato verde nel momento in cui l'errore era
    presente sul disco.
    """

    def test_un_lettore_non_tracciato_di_release_md_e_rosso(self) -> None:
        """La sonda che il perimetro esteso esiste per superare."""
        with RepositoryFinto() as finto:
            radice = finto.radice
            finto.scrivi(
                "scripts/lettore_nuovo.py",
                'testo = open("docs/RELEASE.md").read()\n',
            )
            with mock.patch.object(gate, "ROOT", radice):
                errori = gate.nessun_markdown_come_database()
        self.assertTrue(
            any("scripts/lettore_nuovo.py" in e for e in errori),
            f"il lettore non tracciato non e' stato visto: {errori}",
        )

    def test_lo_stesso_lettore_tracciato_resta_rosso(self) -> None:
        """La controprova: l'estensione aggiunge casi, non ne sposta."""
        with RepositoryFinto() as finto:
            radice = finto.radice
            finto.committa(
                "scripts/lettore_committato.py",
                'testo = open("docs/RELEASE.md").read()\n',
            )
            with mock.patch.object(gate, "ROOT", radice):
                errori = gate.nessun_markdown_come_database()
        self.assertTrue(any("scripts/lettore_committato.py" in e for e in errori), errori)

    def test_un_lettore_ignorato_resta_fuori(self) -> None:
        """`target/` e le copie di lavoro non sono materiale del repository.

        Includerli renderebbe il gate dipendente da cio' che c'e' sulla
        macchina di chi lo lancia — un rosso che nessun altro riproduce.
        """
        with RepositoryFinto() as finto:
            radice = finto.radice
            finto.committa(".gitignore", "scripts/scarti/\n")
            finto.scrivi(
                "scripts/scarti/lettore_ignorato.py",
                'testo = open("docs/RELEASE.md").read()\n',
            )
            with mock.patch.object(gate, "ROOT", radice):
                errori = gate.nessun_markdown_come_database()
        self.assertEqual(errori, [], errori)

    def test_un_markdown_non_tracciato_fuori_allowlist_e_rosso(self) -> None:
        with RepositoryFinto() as finto:
            radice = finto.radice
            finto.scrivi("docs/APPUNTI.md", "# appunti\n")
            with mock.patch.object(gate, "ROOT", radice):
                errori = gate.allowlist()
        self.assertTrue(any("docs/APPUNTI.md" in e for e in errori), errori)

    def test_l_unione_non_ripete_un_file(self) -> None:
        with RepositoryFinto() as finto:
            radice = finto.radice
            finto.committa("scripts/uno.py", "pass\n")
            finto.scrivi("scripts/due.py", "pass\n")
            with mock.patch.object(gate, "ROOT", radice):
                visti = gate.nel_perimetro("scripts/*.py")
                soli_tracciati = gate.tracciati("scripts/*.py")
        self.assertEqual(visti, ["scripts/due.py", "scripts/uno.py"])
        self.assertEqual(soli_tracciati, ["scripts/uno.py"])


class SondeControlli(unittest.TestCase):
    """Ogni controllo, sul repository reale, e' verde."""

    def test_tutti_i_controlli_passano(self) -> None:
        for nome, controllo in gate.CONTROLLI:
            with self.subTest(controllo=nome):
                self.assertEqual(controllo(), [], nome)

    def test_un_runbook_senza_rollback_e_rosso(self) -> None:
        """Costruire un archivio non dice come tornare alla versione prima."""
        sorgente = (gate.ROOT / "docs" / "RELEASE.md").read_text(encoding="utf-8")
        mutato = sorgente.replace("**Rollback.**", "**Ritorno.**", 1)
        with tempfile.TemporaryDirectory() as temporanea:
            radice = pathlib.Path(temporanea)
            (radice / "docs").mkdir()
            (radice / "docs" / "RELEASE.md").write_text(mutato, encoding="utf-8")
            with mock.patch.object(gate, "ROOT", radice):
                errori = gate.runbook_operativo()
        self.assertTrue(any("**rollback.**" in errore for errore in errori), errori)

    def test_un_runbook_che_attiva_prima_di_verificare_e_rosso(self) -> None:
        sorgente = (gate.ROOT / "docs" / "RELEASE.md").read_text(encoding="utf-8")
        mutato = sorgente.replace(
            "verificare il checksum",
            "rinominare la directory e poi verificare il checksum",
            1,
        )
        with tempfile.TemporaryDirectory() as temporanea:
            radice = pathlib.Path(temporanea)
            (radice / "docs").mkdir()
            (radice / "docs" / "RELEASE.md").write_text(mutato, encoding="utf-8")
            with mock.patch.object(gate, "ROOT", radice):
                errori = gate.runbook_operativo()
        self.assertTrue(any("prima di verificare" in errore for errore in errori), errori)


class SondeStatoGenerato(unittest.TestCase):
    """Il blocco di stato e' reso dalla fonte, non riscritto a mano."""

    def stato(self) -> dict:
        return json.loads(gate.STATO.read_text(encoding="utf-8"))

    def registro(self) -> dict:
        return json.loads(stato_release.REGISTRO.read_text(encoding="utf-8"))

    def documento(self) -> str:
        return (gate.ROOT / "docs" / "RELEASE.md").read_text(encoding="utf-8")

    def test_il_renderer_rende_esattamente_i_campi_dichiarati(self) -> None:
        """L'elenco chiuso e' l'unica cosa che impedisca a un campo di
        scomparire in silenzio: un renderer che ne perde uno non lo dice."""
        self.assertEqual(
            list(stato_release.campi(self.stato(), self.registro())),
            list(stato_release.CAMPI_RICHIESTI),
        )

    def test_ogni_campo_richiesto_compare_nel_blocco(self) -> None:
        testo, errori = stato_release.blocco(self.stato(), self.registro())
        self.assertEqual(errori, [], errori)
        for etichetta, valore in stato_release.campi(
            self.stato(), self.registro()
        ).items():
            with self.subTest(campo=etichetta):
                self.assertIn(f"| {etichetta} | {valore} |", testo)

    def test_un_campo_reso_e_non_dichiarato_e_rosso(self) -> None:
        with mock.patch.object(
            stato_release, "CAMPI_RICHIESTI", stato_release.CAMPI_RICHIESTI[:-1]
        ):
            _, errori = stato_release.blocco(self.stato(), self.registro())
        self.assertTrue(any("non dichiarato" in e for e in errori), errori)

    def test_un_campo_dichiarato_e_non_reso_e_rosso(self) -> None:
        with mock.patch.object(
            stato_release,
            "CAMPI_RICHIESTI",
            stato_release.CAMPI_RICHIESTI + ("un campo mai reso",),
        ):
            _, errori = stato_release.blocco(self.stato(), self.registro())
        self.assertTrue(any("non reso" in e for e in errori), errori)

    def test_un_numero_divergente_e_rosso(self) -> None:
        """La sonda decisiva: se il confronto fosse vacuo, due verita'
        potrebbero divergere senza che nulla lo dicesse."""
        stato = self.stato()
        stato["ultima_misura"]["fuzz"]["replay_input"] = 999999
        testo, _ = stato_release.blocco(stato, self.registro())
        self.assertNotIn(testo, self.documento())

    def test_un_numero_sotto_l_etichetta_sbagliata_e_rosso(self) -> None:
        """Il caso che la ricerca per sottostringa non coglieva.

        Scambiare due valori lascia nel documento **le stesse cifre**: un gate
        che cercasse i numeri ovunque resterebbe verde.
        """
        stato = self.stato()
        copertura = stato["ultima_misura"]["copertura"]
        copertura["lcov_percentuale"], copertura["cargo_lines_percentuale"] = (
            copertura["cargo_lines_percentuale"],
            copertura["lcov_percentuale"],
        )
        testo, _ = stato_release.blocco(stato, self.registro())
        self.assertNotIn(testo, self.documento())

    def test_l_elenco_dei_blocchi_e_quello_del_registro(self) -> None:
        righe, errori = stato_release.blocchi(self.registro())
        self.assertEqual(errori, [], errori)
        self.assertEqual(
            [identita for identita, _ in righe],
            [
                v["id"]
                for v in self.registro()["invarianti"]
                if v["stato"] == "release_blocking"
            ],
        )

    def test_lo_stato_non_ripete_l_elenco_dei_blocchi(self) -> None:
        """La copia e' stata tolta, non solo confrontata.

        Finche' esisteva, il renderer verificava che coincidesse con il
        registro — e reggeva. Restava pero' una seconda rappresentazione degli
        stessi dati, da riscrivere a ogni blocco che nasce o muore. Questa
        sonda impedisce che rientri.
        """
        blocchi = self.stato()["blocchi"]
        self.assertNotIn("elenco", blocchi)
        self.assertNotIn("totale", blocchi)
        self.assertIn("fonte", blocchi)

    def test_il_conteggio_reso_segue_il_registro(self) -> None:
        """Il numero nel blocco non viene dallo stato: si conta nel registro."""
        registro = self.registro()
        registro["invarianti"] = [
            v for v in registro["invarianti"] if v["stato"] == "release_blocking"
        ][:2]
        self.assertEqual(
            stato_release.campi(self.stato(), registro)["blocchi"], "2"
        )

    def test_l_elenco_delle_differite_e_quello_del_registro(self) -> None:
        righe, errori = stato_release.differite(self.registro())
        self.assertEqual(errori, [], errori)
        self.assertEqual(
            [identita for identita, _, _ in righe],
            [
                v["id"]
                for v in self.registro()["invarianti"]
                if v["stato"] == "differita"
            ],
        )

    def test_una_differita_non_e_contata_fra_i_blocchi(self) -> None:
        """Le due tabelle contano insiemi disgiunti.

        Se una capacita' differita finisse fra i blocchi, il conteggio direbbe
        che la release e' piu' lontana di quanto sia; se una bloccante finisse
        fra le differite, direbbe che una verifica mancante e' una scelta di
        perimetro. Sono i due errori opposti, e nessuno dei due si vede a
        occhio in una tabella generata.
        """
        registro = self.registro()
        identita_differite = {
            v["id"] for v in registro["invarianti"] if v["stato"] == "differita"
        }
        self.assertTrue(identita_differite, "il registro non ha differite da provare")
        righe, _ = stato_release.blocchi(registro)
        self.assertFalse(identita_differite & {identita for identita, _ in righe})

    def test_il_conteggio_delle_differite_segue_il_registro(self) -> None:
        registro = self.registro()
        registro["invarianti"] = [
            v for v in registro["invarianti"] if v["stato"] == "differita"
        ]
        self.assertEqual(
            stato_release.campi(self.stato(), registro)["capacità differite"],
            str(len(registro["invarianti"])),
        )

    def test_una_differita_senza_non_promette_e_rossa(self) -> None:
        """Il campo che paga il rinvio deve arrivare fino al documento.

        Senza, la tabella delle differite direbbe soltanto che qualcosa e'
        stato rinviato, e «rinviato» senza «non promette» si legge come un
        dettaglio di pianificazione invece che come una capacita' in meno.
        """
        registro = self.registro()
        for voce in registro["invarianti"]:
            if voce["stato"] == "differita":
                voce["differita"].pop("non_promette", None)
                break
        _, errori = stato_release.differite(registro)
        self.assertTrue(any("non_promette" in e for e in errori), errori)

    def test_il_non_promette_del_registro_e_reso_nel_documento(self) -> None:
        """La prosa non e' riassunta: e' resa, e il confronto e' carattere per
        carattere. Cambiarla nel registro senza rigenerare il documento e'
        rosso, e cambiarla nel documento senza toccare il registro pure."""
        righe, _ = stato_release.differite(self.registro())
        self.assertTrue(righe, "il registro non ha differite da provare")
        for _, _, non_promette in righe:
            self.assertIn(non_promette, self.documento())

    def test_un_bloccante_senza_sintesi_e_rosso(self) -> None:
        registro = self.registro()
        for voce in registro["invarianti"]:
            if voce["stato"] == "release_blocking":
                voce.pop("sintesi", None)
                break
        _, errori = stato_release.blocchi(registro)
        self.assertTrue(any("senza `sintesi`" in e for e in errori), errori)

    def _con_documento(self, testo: str) -> list[str]:
        """`stato_coerente` su un `RELEASE.md` finto.

        Solo `ROOT` viene spostato: `STATO` e `REGISTRO` restano quelli veri,
        cosi' la sonda misura il confronto e non una fonte inventata.
        """
        with tempfile.TemporaryDirectory() as temporanea:
            radice = pathlib.Path(temporanea)
            (radice / "docs").mkdir()
            (radice / "docs" / "RELEASE.md").write_text(testo, encoding="utf-8")
            with mock.patch.object(gate, "ROOT", radice):
                return gate.stato_coerente()

    def test_il_documento_reale_coincide_con_la_fonte(self) -> None:
        self.assertEqual(self._con_documento(self.documento()), [])

    def test_un_marcatore_mancante_e_rosso(self) -> None:
        errori = self._con_documento(
            self.documento().replace(stato_release.CHIUSURA, "")
        )
        self.assertTrue(any("compare 0 volte" in e for e in errori), errori)

    def test_un_marcatore_ripetuto_e_rosso(self) -> None:
        errori = self._con_documento(self.documento() + stato_release.APERTURA)
        self.assertTrue(any("compare 2 volte" in e for e in errori), errori)

    def test_i_marcatori_invertiti_sono_rossi(self) -> None:
        testo = self.documento()
        senza = testo.replace(stato_release.APERTURA, "", 1).replace(
            stato_release.CHIUSURA, "", 1
        )
        errori = self._con_documento(
            senza + "\n" + stato_release.CHIUSURA + "\n" + stato_release.APERTURA + "\n"
        )
        self.assertTrue(any("invertiti" in e for e in errori), errori)

    def test_una_riga_ritoccata_a_mano_e_rossa(self) -> None:
        """Il blocco si confronta carattere per carattere: un ritocco che
        «suona giusto» non sopravvive.

        La riga da ritoccare si **prende dal blocco reso**, non si scrive qui:
        una sonda che nomina «| blocchi | 9 |» diventa rossa il giorno in cui
        nasce il decimo blocco, e la si aggiorna a mano invece di seguire la
        fonte.
        """
        reso = stato_release.campi(self.stato(), self.registro())["blocchi"]
        errori = self._con_documento(
            self.documento().replace(
                f"| blocchi | {reso} |", f"| blocchi | {int(reso) - 1} |"
            )
        )
        self.assertTrue(any("non coincide con la fonte" in e for e in errori), errori)


class SondeEccezione(unittest.TestCase):
    def test_solo_il_gate_del_docset_puo_leggere_i_documenti(self) -> None:
        """L'eccezione e' ristretta a un file, e va tenuta tale.

        Il gate legge i documenti per **verificarli** — collegamenti, blocco
        generato, raggiungibilita' — che e' l'opposto di dipendere dalla prosa.
        Se domani un altro script leggesse un documento, quella sarebbe la
        dipendenza che la regola vieta.

        `stato_release.py` non e' nell'insieme apposta: rende il blocco
        leggendo **solo JSON**, e non apre `RELEASE.md`. E' la ragione per cui
        la riscrittura sta in `check_docset`.
        """
        self.assertEqual(
            gate.VALIDATORI,
            {"scripts/check_docset.py", "scripts/test_check_docset.py"},
            "l'eccezione si e' allargata oltre il validatore e la sua sonda",
        )

    def test_i_trascrittori_sono_un_insieme_a_parte(self) -> None:
        """Leggere per **verificare** e leggere per **copiare** sono due cose.

        Un unico insieme le avrebbe fuse in «script che possono leggere i
        documenti», che e' la regola che questo gate non vuole. Il costruttore
        del pacchetto Python mette il README nella descrizione dei metadati:
        nella convenzione dei pacchetti Python la descrizione lunga *e'* il
        README, e riscriverla a mano produrrebbe due testi destinati a
        divergere.
        """
        self.assertEqual(
            gate.TRASCRITTORI,
            {"scripts/costruisci-pacchetto-python.py"},
            "anche l'insieme dei trascrittori si allarga per decisione",
        )
        self.assertFalse(
            gate.VALIDATORI & gate.TRASCRITTORI,
            "uno script e' l'uno o l'altro: chi verifica non trascrive",
        )

    def test_il_renderer_non_apre_il_docset(self) -> None:
        """La ragione per cui `stato_release` puo' restare fuori dall'insieme.

        Non e' una promessa: e' lo stesso controllo che il gate applica a tutti
        gli script, e il renderer vi e' sottoposto come gli altri.
        """
        errori = gate.nessun_markdown_come_database()
        self.assertEqual([e for e in errori if "stato_release" in e], [])

    def test_i_validatori_esistono(self) -> None:
        for relativo in gate.VALIDATORI:
            self.assertTrue((gate.ROOT / relativo).is_file(), relativo)

    def test_un_lettore_qualunque_non_e_ammesso(self) -> None:
        """La sonda decisiva: l'eccezione e' un insieme chiuso.

        Se `VALIDATORI` fosse ignorato e il controllo passasse tutto, questa
        resterebbe verde soltanto perche' non verifica nulla — quindi verifica
        il verso opposto: un nome che non e' nell'insieme non e' ammesso.
        """
        self.assertNotIn("scripts/check_release_contract.py", gate.VALIDATORI)
        self.assertNotIn("scripts/stato_release.py", gate.VALIDATORI)


if __name__ == "__main__":
    unittest.main()
