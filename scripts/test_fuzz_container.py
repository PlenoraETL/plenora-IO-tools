#!/usr/bin/env python3
"""Le sonde di `scripts/fuzz-container.sh`, con un Docker finto.

# Perche' un finto e non Docker

Perche' altrimenti queste proprieta' si potrebbero provare solo avendo
un'immagine, un volume e una campagna vera -- cioe' in pratica mai, ed e' il
motivo per cui il wrapper non aveva prove. Il finto risponde alle quattro
domande che il wrapper pone (`inspect` per l'esistenza, lo stato e l'exit code;
`logs` per il testo; `rm` per la rimozione) e registra che cosa gli e' stato
chiesto, cosi' una prova puo' dire non solo che cosa e' successo ma **in quale
ordine**.

La proprieta' centrale e' quella: il log si salva **prima** della rimozione, e
se non si salva la rimozione non avviene. Un log perso e' un'esecuzione di cui
resta il verdetto e non il racconto, ed e' successo davvero.
"""

from __future__ import annotations

import os
import pathlib
import subprocess
import tempfile
import unittest

RADICE = pathlib.Path(__file__).resolve().parents[1]
WRAPPER = RADICE / "scripts" / "fuzz-container.sh"

#: Il finto: risponde come Docker e annota ogni invocazione.
FINTO = """#!/bin/bash
echo "$@" >> "$TRACCIA"
case "$1 $2" in
  "container inspect")
    if [ "$3" = "-f" ]; then
      case "$4" in
        *Running*) echo "false" ;;
        *ExitCode*) echo "$ESITO_FINTO" ;;
        *plenora.revisione*)
          # Dopo `rm` l'etichetta non c'e' piu': e' il punto del difetto.
          if [ -f "$TRACCIA.rimosso" ]; then echo ""; else echo "$REVISIONE_FINTA"; fi ;;
        *) echo "" ;;
      esac
    fi
    exit 0 ;;
  "container logs")
    if [ "$LOG_FALLISCE" = "si" ]; then exit 1; fi
    printf '%s\\n' $LOG_FINTO
    exit 0 ;;
  "container rm") touch "$TRACCIA.rimosso"; exit 0 ;;
esac
exit 0
"""


class SondeDelCollect(unittest.TestCase):
    def setUp(self) -> None:
        self.temporanea = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporanea.cleanup)
        self.radice = pathlib.Path(self.temporanea.name)
        self.finto = self.radice / "docker-finto"
        self.finto.write_text(FINTO, encoding="utf-8", newline="\n")
        self.finto.chmod(0o755)
        self.traccia = self.radice / "traccia.txt"
        self.revisione = "c" * 40
        self.log = self.radice / "log"

    def _collect(self, esito: str = "0", log_fallisce: str = "no"):
        ambiente = dict(os.environ)
        ambiente.update(
            {
                "PLENORA_DOCKER": str(self.finto),
                "PLENORA_FUZZ_LOG_DIR": str(self.log),
                "TRACCIA": str(self.traccia),
                "ESITO_FINTO": esito,
                "LOG_FALLISCE": log_fallisce,
                "LOG_FINTO": "prima seconda terza",
                "REVISIONE_FINTA": self.revisione,
            }
        )
        esecuzione = subprocess.run(
            ["bash", str(WRAPPER), "collect", "5"],
            capture_output=True,
            text=True,
            env=ambiente,
            check=False,
        )
        chiamate = (
            self.traccia.read_text(encoding="utf-8").splitlines()
            if self.traccia.exists()
            else []
        )
        return esecuzione, chiamate

    def test_il_log_intero_finisce_su_disco(self) -> None:
        esecuzione, _ = self._collect()
        self.assertEqual(esecuzione.returncode, 0, esecuzione.stderr)
        scritti = sorted(self.log.glob("*.log"))
        self.assertEqual(len(scritti), 1, scritti)
        testo = scritti[0].read_text(encoding="utf-8")
        for riga in ("prima", "seconda", "terza"):
            self.assertIn(riga, testo)

    def test_il_percorso_del_log_e_detto_a_chi_legge(self) -> None:
        # Un log salvato di cui nessuno sa il percorso e' un log perso con un
        # passaggio in piu'.
        esecuzione, _ = self._collect()
        self.assertIn("log completo in", esecuzione.stdout)

    def test_si_salva_prima_di_rimuovere(self) -> None:
        # L'ordine e' la proprieta': salvare dopo la rimozione non e' salvare.
        _, chiamate = self._collect()
        indice_log = next(i for i, c in enumerate(chiamate) if c.startswith("container logs"))
        indice_rm = next(i for i, c in enumerate(chiamate) if c.startswith("container rm"))
        self.assertLess(indice_log, indice_rm, chiamate)

    def test_se_il_log_non_si_salva_il_container_resta(self) -> None:
        """La prova che conta: senza log, niente rimozione.

        Rimuovere dopo aver fallito il salvataggio distruggerebbe l'unica copia
        rimasta, ed e' esattamente il modo in cui un log si perde.
        """
        esecuzione, chiamate = self._collect(log_fallisce="si")
        self.assertFalse(
            any(c.startswith("container rm") for c in chiamate),
            f"il container e' stato rimosso senza il log: {chiamate}",
        )
        self.assertIn("NON viene rimosso", esecuzione.stderr)

    def test_l_esito_del_container_e_restituito_e_non_viene_dal_log(self) -> None:
        # L'exit code viene da `inspect`, mai dal testo: un `| tail` che
        # restituisce zero mentre il comando a monte fallisce e' un errore gia'
        # fatto in questo repository.
        esecuzione, _ = self._collect(esito="3")
        self.assertEqual(esecuzione.returncode, 3, esecuzione.stdout)

    def test_anche_con_esito_rosso_il_log_si_salva(self) -> None:
        # E' il caso in cui il log serve di piu'.
        self._collect(esito="1")
        self.assertEqual(len(sorted(self.log.glob("*.log"))), 1)

    def test_due_corse_non_si_sovrascrivono(self) -> None:
        self._collect()
        self.traccia.unlink()
        # Il nome porta i secondi: due corse nello stesso secondo userebbero lo
        # stesso file, e la prova lo dice invece di fingere il contrario.
        import time

        time.sleep(1.1)
        self._collect()
        self.assertEqual(len(sorted(self.log.glob("*.log"))), 2)


if __name__ == "__main__":
    unittest.main()


class SondeDellaRevisioneDellaCorsa(unittest.TestCase):
    """La corsa dice quale revisione misura, e se l'albero si e' mosso.

    Non e' un errore che si siano mossi: una campagna lunga e un ramo che
    avanza convivono. E' un errore **non saperlo**, e leggere l'esito come se
    riguardasse l'albero che si ha davanti. E' uno degli errori che questo
    ciclo ha gia' fatto.
    """

    def setUp(self) -> None:
        SondeDelCollect.setUp(self)

    _collect = SondeDelCollect._collect

    def test_il_nome_del_log_porta_la_revisione(self) -> None:
        # Un log ritrovato mesi dopo deve dire da solo che cosa misurava.
        self._collect()
        scritti = sorted(self.log.glob("*.log"))
        self.assertEqual(len(scritti), 1, scritti)
        self.assertIn(self.revisione[:12], scritti[0].name)

    def test_collect_dice_se_l_albero_si_e_mosso(self) -> None:
        esecuzione, _ = self._collect()
        self.assertIn("DIVERSA dall'albero corrente", esecuzione.stdout)

    def test_una_corsa_senza_revisione_incisa_lo_dichiara(self) -> None:
        # I container avviati da una versione precedente del wrapper non hanno
        # l'etichetta: il wrapper lo dice invece di inventare una revisione.
        self.revisione = ""
        esecuzione, _ = self._collect()
        self.assertIn("NON incisa", esecuzione.stdout)

    def test_l_avvio_incide_la_revisione(self) -> None:
        # La si incide **all'avvio**: l'albero puo' muoversi mentre la campagna
        # gira, e dedurla dopo risponderebbe di un'altra.
        wrapper = WRAPPER.read_text(encoding="utf-8")
        self.assertIn("--label \"plenora.revisione=${revisione}\"", wrapper)
        self.assertIn("rev-parse HEAD", wrapper)


class SondeDelloStop(unittest.TestCase):
    """`stop` ferma e lascia raccoglibile; buttare via si chiede per nome."""

    def setUp(self) -> None:
        SondeDelCollect.setUp(self)

    def _comando(self, *argomenti: str, in_esecuzione: bool = False):
        if in_esecuzione:
            # Il ramo che conta: una campagna viva che qualcuno interrompe. E'
            # li' che `rm --force` distruggeva, e una prova che esercitasse
            # solo il ramo «gia' fermo» non l'avrebbe visto -- infatti non lo
            # vedeva.
            self.finto.write_text(
                FINTO.replace('*Running*) echo "false"', '*Running*) echo "true"'),
                encoding="utf-8",
                newline="\n",
            )
            self.finto.chmod(0o755)
        ambiente = dict(os.environ)
        ambiente.update(
            {
                "PLENORA_DOCKER": str(self.finto),
                "PLENORA_FUZZ_LOG_DIR": str(self.log),
                "TRACCIA": str(self.traccia),
                "ESITO_FINTO": "0",
                "LOG_FALLISCE": "no",
                "LOG_FINTO": "riga",
                "REVISIONE_FINTA": self.revisione,
            }
        )
        esecuzione = subprocess.run(
            ["bash", str(WRAPPER), *argomenti],
            capture_output=True,
            text=True,
            env=ambiente,
            check=False,
        )
        chiamate = (
            self.traccia.read_text(encoding="utf-8").splitlines()
            if self.traccia.exists()
            else []
        )
        return esecuzione, chiamate

    def test_stop_non_rimuove_il_container(self) -> None:
        """Il rilievo: `stop` faceva `rm --force`.

        Fermava e distruggeva insieme, senza acquisire l'esito e senza salvare
        il log -- disfacendo a due righe di distanza tutto cio' che `collect`
        era stato scritto per conservare.
        """
        esecuzione, chiamate = self._comando("stop")
        self.assertFalse(
            any("container rm" in c for c in chiamate),
            f"stop ha rimosso il container: {chiamate}",
        )
        self.assertIn("collect", esecuzione.stdout)

    def test_stop_non_rimuove_nemmeno_una_corsa_viva(self) -> None:
        # E' il ramo dove il difetto stava davvero.
        esecuzione, chiamate = self._comando("stop", in_esecuzione=True)
        self.assertTrue(any("container stop" in c for c in chiamate), chiamate)
        self.assertFalse(
            any("container rm" in c for c in chiamate),
            f"stop ha distrutto una corsa viva: {chiamate}",
        )
        self.assertIn("collect", esecuzione.stdout)

    def test_scarta_rimuove_ma_lo_dice(self) -> None:
        # La decisione di perdere un'evidenza dev'essere detta, non essere il
        # comportamento per difetto di un comando che si chiama «stop».
        esecuzione, chiamate = self._comando("scarta")
        self.assertTrue(any("container rm" in c for c in chiamate), chiamate)
        self.assertIn("SENZA leggerne l'esito", esecuzione.stdout)

    def test_status_dice_la_revisione_anche_a_corsa_finita(self) -> None:
        """Il rilievo: il confronto stava solo nel ramo «in esecuzione».

        Cioe' dove nessuno conclude niente. A corsa finita si legge un esito, e
        un esito senza sapere a che cosa si riferisce e' la diagnosi sbagliata
        che aspetta di succedere.
        """
        esecuzione, _ = self._comando("status")
        self.assertIn("terminato con exit", esecuzione.stdout)
        self.assertIn("revisione della corsa", esecuzione.stdout)


class SondeDellIsolamento(unittest.TestCase):
    """I sorgenti della corsa sono un clone fermo, non il checkout vivo.

    L'etichetta con lo SHA non bastava: il container montava l'albero di
    lavoro, e una modifica durante la campagna entrava nelle compilazioni
    successive mentre l'etichetta conservava lo SHA iniziale. L'attribuzione
    diventava falsa proprio dove sembrava piu' solida.
    """

    def setUp(self) -> None:
        self.temporanea = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporanea.cleanup)
        self.radice = pathlib.Path(self.temporanea.name)

        # Un repository vero, minuscolo: il clone dev'essere esercitato, non
        # simulato, o la prova non direbbe niente del comando che gira davvero.
        self.repo = self.radice / "repo"
        (self.repo / "scripts").mkdir(parents=True)
        (self.repo / "fuzz" / "corpus").mkdir(parents=True)
        (self.repo / "fuzz" / "artifacts").mkdir(parents=True)
        (self.repo / "assurance" / "evidence").mkdir(parents=True)
        (self.repo / "scripts" / "fuzz-container.sh").write_bytes(
            WRAPPER.read_bytes()
        )
        (self.repo / "scripts" / "fuzz-smoke.sh").write_text("#!/bin/bash\n", encoding="utf-8")
        (self.repo / "sorgente.txt").write_text("prima\n", encoding="utf-8")
        for comando in (
            ["git", "init", "--quiet"],
            ["git", "config", "user.email", "prova@esempio"],
            ["git", "config", "user.name", "Prova"],
            ["git", "add", "-A"],
            ["git", "commit", "--quiet", "-m", "base"],
        ):
            subprocess.run(comando, cwd=self.repo, check=True, capture_output=True)

        self.finto = self.radice / "docker-finto"
        self.finto.write_text(FINTO, encoding="utf-8", newline="\n")
        self.finto.chmod(0o755)
        self.traccia = self.radice / "traccia.txt"

    def _start(self):
        ambiente = dict(os.environ)
        ambiente.update(
            {
                "PLENORA_DOCKER": str(self.finto),
                "PLENORA_FUZZ_CHECKOUT_DIR": str(self.radice / "checkout"),
                "TRACCIA": str(self.traccia),
                "ESITO_FINTO": "0",
                "LOG_FALLISCE": "no",
                "LOG_FINTO": "riga",
                "REVISIONE_FINTA": "",
            }
        )
        # `container inspect` senza `-f` decide se il container «esiste»: il
        # finto esce 0 sempre, quindi qui si finge che non ci sia.
        finto_assente = self.radice / "docker-assente"
        finto_assente.write_text(
            FINTO.replace(
                '  "container inspect")',
                '  "container inspect")\n    if [ "$3" != "-f" ]; then exit 1; fi',
            ),
            encoding="utf-8",
            newline="\n",
        )
        finto_assente.chmod(0o755)
        ambiente["PLENORA_DOCKER"] = str(finto_assente)
        esecuzione = subprocess.run(
            ["bash", str(self.repo / "scripts" / "fuzz-container.sh"), "start", "smoke"],
            capture_output=True,
            text=True,
            env=ambiente,
            check=False,
        )
        chiamate = (
            self.traccia.read_text(encoding="utf-8").splitlines()
            if self.traccia.exists()
            else []
        )
        return esecuzione, chiamate

    def test_i_sorgenti_montati_sono_il_clone_non_l_albero_vivo(self) -> None:
        esecuzione, chiamate = self._start()
        self.assertEqual(esecuzione.returncode, 0, esecuzione.stderr)
        run = next((c for c in chiamate if c.startswith("run ")), "")
        self.assertIn("checkout", run, run)
        self.assertNotIn(f"{self.repo}:/work", run, run)

    def test_il_corpus_e_gli_esiti_restano_dell_albero_vivo(self) -> None:
        # Isolati i sorgenti, non gli esiti: il corpus e' un ingresso che la
        # campagna accresce, e artefatti ed evidenze sono cio' che produce.
        _, chiamate = self._start()
        run = next((c for c in chiamate if c.startswith("run ")), "")
        for relativo in ("fuzz/corpus", "fuzz/artifacts", "assurance/evidence"):
            with self.subTest(percorso=relativo):
                self.assertIn(f"{relativo}:/work/{relativo}", run.replace("\\", "/"))

    def test_il_clone_contiene_la_revisione_e_non_le_modifiche_dopo(self) -> None:
        self._start()
        cloni = list((self.radice / "checkout").glob("checkout-*"))
        self.assertEqual(len(cloni), 1, cloni)
        self.assertEqual(
            (cloni[0] / "sorgente.txt").read_text(encoding="utf-8"), "prima\n"
        )

    def test_un_albero_sporco_non_avvia(self) -> None:
        """Il clone parte da HEAD, quindi non porterebbe le modifiche.

        Misurare una revisione che non contiene il lavoro che si ha davanti e'
        una diagnosi che va male dopo, non subito: meglio rifiutare.
        """
        (self.repo / "sorgente.txt").write_text("dopo\n", encoding="utf-8")
        esecuzione, chiamate = self._start()
        self.assertNotEqual(esecuzione.returncode, 0)
        self.assertIn("albero di lavoro sporco", esecuzione.stderr)
        self.assertFalse(any(c.startswith("run ") for c in chiamate), chiamate)


class SondeDellaPuliziaDelClone(unittest.TestCase):
    """Il clone si rimuove davvero, e il percorso si acquisisce prima.

    L'etichetta vive nel container: leggerla dopo `rm` non restituisce niente,
    e il clone restava sul disco mentre il wrapper diceva di aver pulito. E' lo
    stesso ordine che `collect` gia' rispettava per l'esito, e che la pulizia
    non rispettava.
    """

    def setUp(self) -> None:
        SondeDelCollect.setUp(self)
        self.revisione = "d" * 40
        self.clone = self.radice / "checkout" / f"checkout-{self.revisione[:12]}"
        self.clone.mkdir(parents=True)
        (self.clone / "segno.txt").write_text("ci sono\n", encoding="utf-8")

    def _comando(self, *argomenti: str):
        ambiente = dict(os.environ)
        ambiente.update(
            {
                "PLENORA_DOCKER": str(self.finto),
                "PLENORA_FUZZ_LOG_DIR": str(self.log),
                "PLENORA_FUZZ_CHECKOUT_DIR": str(self.radice / "checkout"),
                "TRACCIA": str(self.traccia),
                "ESITO_FINTO": "0",
                "LOG_FALLISCE": "no",
                "LOG_FINTO": "riga",
                "REVISIONE_FINTA": self.revisione,
            }
        )
        return subprocess.run(
            ["bash", str(WRAPPER), *argomenti],
            capture_output=True,
            text=True,
            env=ambiente,
            check=False,
        )

    def test_collect_rimuove_anche_il_clone(self) -> None:
        esecuzione = self._comando("collect", "5")
        self.assertFalse(
            self.clone.exists(),
            f"il clone e' rimasto dopo collect: {esecuzione.stdout}",
        )
        self.assertIn("sorgenti isolati rimossi", esecuzione.stdout)

    def test_scarta_rimuove_anche_il_clone(self) -> None:
        esecuzione = self._comando("scarta")
        self.assertFalse(self.clone.exists(), esecuzione.stdout)


class SondeDelRiusoDelClone(unittest.TestCase):
    """Un clone si riusa **solo se e' ancora quello che dice di essere**.

    Bastava che esistesse `.git`: un sorgente toccato dentro il clone veniva
    compilato al posto di quello della revisione, con l'etichetta che
    continuava a nominarla. E' lo stesso difetto del mount vivo, un livello
    piu' in basso.
    """

    def setUp(self) -> None:
        SondeDellIsolamento.setUp(self)

    _start = SondeDellIsolamento._start

    def _clone(self) -> pathlib.Path:
        cloni = list((self.radice / "checkout").glob("checkout-*"))
        self.assertEqual(len(cloni), 1, cloni)
        return cloni[0]

    def test_un_clone_sporco_viene_rifatto(self) -> None:
        self._start()
        clone = self._clone()
        (clone / "sorgente.txt").write_text("manomesso\n", encoding="utf-8")

        esecuzione, chiamate = self._start()
        self.assertEqual(esecuzione.returncode, 0, esecuzione.stderr)
        self.assertEqual(
            (self._clone() / "sorgente.txt").read_text(encoding="utf-8"),
            "prima\n",
            "il clone manomesso e' stato riusato",
        )
        self.assertIn("non riusabile", esecuzione.stderr)
        self.assertTrue(any(c.startswith("run ") for c in chiamate), chiamate)

    def test_un_clone_su_un_altra_revisione_viene_rifatto(self) -> None:
        # Stesso nome di directory, contenuto di un'altra revisione: succede
        # riusando un percorso, e il controllo e' sulla revisione vera.
        self._start()
        clone = self._clone()
        (clone / "sorgente.txt").write_text("seconda\n", encoding="utf-8")
        for comando in (
            # Il clone non eredita l'identita' locale: senza, `commit` esce 128
            # e la prova fallirebbe per una ragione che non sta esaminando.
            ["git", "config", "user.email", "prova@esempio"],
            ["git", "config", "user.name", "Prova"],
            ["git", "add", "-A"],
            ["git", "commit", "--quiet", "-m", "altra"],
        ):
            subprocess.run(comando, cwd=clone, check=True, capture_output=True)

        esecuzione, _ = self._start()
        self.assertEqual(esecuzione.returncode, 0, esecuzione.stderr)
        self.assertEqual(
            (self._clone() / "sorgente.txt").read_text(encoding="utf-8"), "prima\n"
        )

    def test_un_clone_pulito_e_alla_revisione_giusta_si_riusa(self) -> None:
        # La controprova positiva: senza, «riclona sempre» sarebbe una difesa
        # che costa cinque secondi a ogni avvio e non prova niente.
        self._start()
        segno = self._clone() / ".segno-del-riuso"
        segno.write_text("x", encoding="utf-8")
        # Un file non tracciato **sporca** l'albero: va tolto dall'indice della
        # prova, o si misurerebbe il contrario di cio' che si vuole.
        (self._clone() / ".git" / "info").mkdir(exist_ok=True)
        (self._clone() / ".git" / "info" / "exclude").write_text(
            ".segno-del-riuso\n", encoding="utf-8"
        )
        esecuzione, _ = self._start()
        self.assertEqual(esecuzione.returncode, 0, esecuzione.stderr)
        self.assertTrue(segno.exists(), "il clone pulito e' stato rifatto senza bisogno")


class SondaDelPercorsoPerDocker(unittest.TestCase):
    """La forma del percorso che il mount riceve.

    Su Git Bash `MSYS_NO_PATHCONV=1` disattiva la riscrittura automatica,
    quindi la forma giusta va data. `radice_repo` lo faceva gia'; il clone e' un
    secondo percorso che finisce in un bind mount, e senza la stessa
    conversione il mount si rompe.

    **Limite dichiarato**: queste prove girano su Linux, dove `pwd -W` non
    esiste e il ramo che conta non si esercita. Qui si verifica che la
    conversione ci sia e che sia applicata al mount, non che produca la forma
    giusta su Windows.
    """

    def test_il_mount_del_clone_passa_dalla_conversione(self) -> None:
        wrapper = WRAPPER.read_text(encoding="utf-8")
        self.assertIn('percorso_per_docker "${checkout}"', wrapper)

    def test_su_linux_il_mount_riceve_il_percorso_del_clone(self) -> None:
        # Non si sorgenta il wrapper per chiamarne una funzione: in fondo ha un
        # `case` che gira e uscirebbe. Si guarda invece cio' che il mount
        # riceve davvero, che e' la domanda vera.
        sonda = SondeDellIsolamento("run")
        sonda.setUp()
        self.addCleanup(sonda.temporanea.cleanup)
        _, chiamate = SondeDellIsolamento._start(sonda)
        run = next((c for c in chiamate if c.startswith("run ")), "")
        cloni = list((sonda.radice / "checkout").glob("checkout-*"))
        self.assertEqual(len(cloni), 1, cloni)
        self.assertIn(f"{cloni[0]}:/work", run)
