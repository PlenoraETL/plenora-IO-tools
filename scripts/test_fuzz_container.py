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
        *plenora.revisione*) echo "$REVISIONE_FINTA" ;;
        *) echo "" ;;
      esac
    fi
    exit 0 ;;
  "container logs")
    if [ "$LOG_FALLISCE" = "si" ]; then exit 1; fi
    printf '%s\\n' $LOG_FINTO
    exit 0 ;;
  "container rm") exit 0 ;;
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
