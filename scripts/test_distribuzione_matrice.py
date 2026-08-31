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
CHECKER = RADICE / "scripts" / "check-linux-gdal-runtime.py"


def carica(percorso: pathlib.Path) -> dict:
    return json.loads(percorso.read_text(encoding="utf-8"))


class SondeMatrice(unittest.TestCase):
    def setUp(self) -> None:
        self.matrice = carica(MATRICE)
        self.lock = carica(LOCK_LINUX)

    def test_ogni_piattaforma_ha_la_propria_origine(self) -> None:
        """Una piattaforma senza origine e' una promessa senza runtime."""
        piattaforme = {p["id"] for p in self.matrice["piattaforme"]}
        origini = {o["piattaforma"] for o in self.matrice["contratto_gdal"]["origini"]}
        self.assertEqual(piattaforme, origini)

    def test_la_versione_gdal_e_una_sola(self) -> None:
        """E' la precondizione perche' la capability sia la stessa ovunque.

        Windows e Linux la dichiarano ciascuno nel proprio lock: se divergessero,
        i due artefatti porterebbero prodotti diversi con lo stesso nome."""
        dichiarata = self.matrice["contratto_gdal"]["versione"]
        self.assertEqual(self.lock["gdal_version"], dichiarata)
        if LOCK_WINDOWS.exists():
            self.assertEqual(carica(LOCK_WINDOWS)["gdal_version"], dichiarata)

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
        attese = set(self.lock["contratto_di_verifica"]["dipendenze_esterne_attese"])
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

    def test_lo_strumento_che_risolve_e_fissato(self) -> None:
        """Uno strumento che cambia da solo rende non riproducibile cio' che
        produce -- e cio' che produce e' proprio l'elenco che il lock fissa."""
        risolto = self.lock["risolto_con"]
        self.assertEqual(len(risolto["sha256"]), 64)
        self.assertTrue(risolto["url"].startswith("https://"))
        self.assertIsInstance(risolto["dimensione"], int)


if __name__ == "__main__":
    unittest.main()
