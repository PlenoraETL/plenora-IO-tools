#!/usr/bin/env python3
"""Le prove di confine del lotto S12 sono **eseguite**, e nominate.

# Perche' esiste

Il tetto sui componenti -- centomila coordinate -- non e' raggiungibile dai
target di fuzzing: non stanno in un input da quattro kilobyte, che e' il cap del
harness. Sotto fuzzing quel ramo non e' esercitato, e i registri delle misure di
profondita' lo dicono invece di lasciarlo intendere.

A provarlo restano le sonde unitarie, che lo provano **al confine**: `n`
componenti passano, `n+1` no. Ma una sonda che nessuno nomina e' una sonda che
si puo' cancellare: `cargo test --workspace` resterebbe verde, e il checkpoint
pure. Il tetto tornerebbe una frase nel commento di una funzione.

Qui le sonde sono nominate una per una e **eseguite**: ciascuna deve comparire
nell'elenco del harness una volta sola e passare. Cancellarne una, rinominarla o
metterla dietro un `#[ignore]` rende rosso questo gate, e con lui l'invariante
`lotto.s12`.

E' la stessa disciplina di `check_assurance_n1_prove.py`, per la stessa ragione:
un riferimento testuale non e' una prova, un'esecuzione si'.

# Uso

    python3 scripts/check_prove_di_confine.py
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# Le sonde che provano cio' che il fuzzing non raggiunge. La chiave e' la crate,
# il valore le identita' esatte: `--exact` non basta da solo, perche' un filtro
# che non trova niente lascia `cargo test` a zero e con exit 0.
PROVE: dict[str, tuple[str, ...]] = {
    "driver-common": (
        "wkt_progressivo::sonde::il_tetto_sui_componenti_e_esatto",
        "wkt_progressivo::sonde::l_annidamento_oltre_il_tetto_e_rifiutato",
        "wkt_progressivo::sonde::la_coda_non_vuota_e_rifiutata_come_sintassi",
    ),
    "driver-geojson": (
        "geometria_progressiva::sonde::il_tetto_sui_componenti_e_esatto",
        "geometria_progressiva::sonde::una_lista_di_numeri_non_cresce_oltre_una_posizione",
        "geometria_progressiva::sonde::oltre_una_certa_profondita_rifiuta_serde_e_non_noi",
        # L'ultima via per entrare nell'albero senza pagare: una lista vuota
        # non era ne' una posizione ne' un elenco addebitato, e `[[],[],...]`
        # cresceva finche' il solo cap in byte non lo fermava. La sonda prova
        # che l'analisi si ferma sulla **prima**.
        "geometria_progressiva::sonde::una_lista_vuota_ferma_l_analisi_alla_prima",
    ),
}

RIGA_DI_ESITO = re.compile(r"^test (?P<nome>\S+) \.\.\. (?P<esito>\w+)")


def esegui(crate: str, nomi: tuple[str, ...], cosa: str = "sonda") -> list[str]:
    """Esegue i test nominati e pretende che ognuno passi, una volta sola.

    `cosa` nomina cio' che si sta eseguendo, perche' questa funzione la usa
    anche il gate della capability: li' non sono «sonde di confine» ma «prove
    della capability», e un messaggio che dicesse la parola sbagliata manderebbe
    a cercare nel file sbagliato. La **regola** e' la stessa, e averla in due
    copie vorrebbe dire vederle divergere.
    """
    # Un elenco vuoto non e' un filtro vuoto: `cargo test -- --exact` senza nomi
    # non filtra niente, esegue **tutto** ed esce 0. Chiamarlo con zero nomi
    # certificherebbe una prova che nessuno ha nominato.
    if not nomi:
        return [
            f"{crate}: nessuna identita' da eseguire. `--exact` senza nomi non "
            "filtra: eseguirebbe l'intera suite e uscirebbe 0, cioe' direbbe "
            "verde senza aver guardato cio' che doveva."
        ]
    ripetute = sorted({n for n in nomi if nomi.count(n) > 1})
    if ripetute:
        return [
            f"{crate}: le identita' {ripetute} sono dichiarate piu' di una volta. "
            "Il harness ne elenca una, e il conteggio ne annuncerebbe due."
        ]
    comando = ["cargo", "test", "-p", crate, "--lib", "--locked", "--", "--exact", *nomi]
    try:
        esito = subprocess.run(
            comando, cwd=str(ROOT), capture_output=True, text=True, check=False
        )
    except OSError as errore:
        return [f"{crate}: `cargo` non e' invocabile ({errore})"]

    visti: dict[str, str] = {}
    for riga in esito.stdout.splitlines():
        trovato = RIGA_DI_ESITO.match(riga)
        if trovato:
            nome = trovato.group("nome")
            if nome in visti:
                return [f"{crate}: {cosa} «{nome}» compare due volte nell'elenco"]
            visti[nome] = trovato.group("esito")

    errori: list[str] = []
    if esito.returncode != 0:
        coda = "\n".join(esito.stdout.strip().splitlines()[-4:])
        errori.append(
            f"{crate}: il harness esce con {esito.returncode}. Un harness che "
            f"fallisce non certifica cio' che ha elencato prima di "
            f"fallire.\n{coda}"
        )
    for nome in nomi:
        stato = visti.get(nome)
        if stato is None:
            errori.append(
                f"{crate}: {cosa} «{nome}» non e' stata eseguita. Un filtro "
                "che non trova niente lascia `cargo test` a zero test e a exit "
                "0: il silenzio non e' un verde."
            )
        elif stato != "ok":
            errori.append(f"{crate}: {cosa} «{nome}» esce con «{stato}»")
    return errori


def verifica() -> list[str]:
    errori: list[str] = []
    for crate, nomi in PROVE.items():
        errori.extend(esegui(crate, nomi))
    return errori


def main() -> int:
    errori = verifica()
    for messaggio in errori:
        print(messaggio, file=sys.stderr)
    if errori:
        return 1
    quante = sum(len(n) for n in PROVE.values())
    print(
        f"prove di confine eseguite: {quante} sonde su {len(PROVE)} crate, "
        "ognuna elencata dal harness una volta sola e passata. Provano al "
        "confine cio' che il cap del harness impedisce di raggiungere sotto "
        "fuzzing."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
