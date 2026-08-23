#!/usr/bin/env python3
"""Le sonde che tolgono lo scheduling dalla misura di copertura **passano**.

# Che cosa questo gate impedisce

La copertura misurata su un sorgente Rust strumentato invariato cambiava fra
due corse. La causa e' dimostrata: alcuni rami si eseguono solo quando una
corsa fra thread va in un certo modo — uno scambio atomico che perde, un canale
bounded pieno al momento del tentativo, un osservatore che parte prima del
produttore — e senza una sonda che li esercita **per costruzione** quelle righe
sono coperte o no a seconda dello scheduling.

Le sonde che le esercitano sono la correzione. Toglierne una riporterebbe la
variazione, e nessun conteggio se ne accorgerebbe: una soglia sulla copertura
non distingue una riga che cambia stato da mille che restano ferme, ed e'
precisamente il motivo per cui la variazione e' rimasta invisibile finche'
nessuno l'ha guardata riga per riga.

# Perche' non basta che i test esistano

Vale qui la stessa distinzione di ASSURANCE-N1: un simbolo che esiste puo'
essere un helper senza `#[test]`, un test sotto `#[ignore]`, o un test sotto un
`cfg` inattivo. Le sonde sono percio' verificate **eseguendole**, con lo stesso
lettore che usa il resto del repository — due definizioni di «test eseguito»
divergerebbero, e divergerebbero in silenzio.

# Che cosa questo gate non fa

Non misura la copertura e non ha soglie. La riproducibilita' riga per riga si
dimostra con campagne ripetute, che costano un'ora e vivono in un'evidenza; qui
si verifica che cio' che la rende riproducibile sia ancora in piedi.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

# Lo stesso lettore di ASSURANCE-N1 e del contratto corrente.
from check_assurance_n1_prove import (  # noqa: E402
    BERSAGLI,
    BERSAGLIO_PREDEFINITO,
    CONFIGURAZIONI,
    esegui_harness,
)

ROOT = pathlib.Path(__file__).resolve().parent.parent
REGISTRO = ROOT / "assurance" / "registries" / "sonde-deterministiche.json"

CAMPI = {
    "ramo",
    "perche_dipendeva_dallo_scheduling",
    "seam",
    "crate",
    "configurazione",
    "bersaglio",
    "test",
    "che_cosa_provano",
}


def struttura(gruppi: list[dict]) -> list[str]:
    """Ogni gruppo dice quale ramo, perche' dipendeva, e che cosa prova."""
    errori: list[str] = []
    visti: set[str] = set()
    for voce in gruppi:
        ramo = voce.get("ramo", "<senza ramo>")
        mancanti = CAMPI - set(voce)
        if mancanti:
            errori.append(f"{ramo}: campi mancanti {sorted(mancanti)}")
            continue
        if ramo in visti:
            errori.append(f"{ramo}: voce duplicata")
        visti.add(ramo)

        if voce["configurazione"] not in CONFIGURAZIONI:
            errori.append(f"{ramo}: configurazione «{voce['configurazione']}» non ammessa")
        if voce["bersaglio"] not in BERSAGLI:
            errori.append(f"{ramo}: bersaglio «{voce['bersaglio']}» non ammesso")

        identita = voce["test"]
        if not isinstance(identita, list) or not identita:
            errori.append(
                f"{ramo}: nessuna sonda. Un ramo dichiarato senza la sonda che "
                "lo esercita e' un ramo che torna a dipendere dallo scheduling."
            )
        elif not all(isinstance(nome, str) and nome for nome in identita):
            errori.append(f"{ramo}: identificatore che non e' una stringa non vuota")
        elif len(set(identita)) != len(identita):
            errori.append(f"{ramo}: identificatori ripetuti")

        for campo in ("perche_dipendeva_dallo_scheduling", "seam", "che_cosa_provano"):
            if not voce[campo]:
                errori.append(
                    f"{ramo}: `{campo}` vuoto. Una sonda senza la ragione per cui "
                    "esiste e' una sonda che qualcuno togliera'."
                )
    return errori


def esegui(gruppi: list[dict]) -> list[str]:
    """Esegue il harness una volta per terna e verifica ogni identita'."""
    per_terna: dict[tuple[str, str, str], list[dict]] = {}
    for voce in gruppi:
        chiave = (
            voce["crate"],
            voce["configurazione"],
            voce.get("bersaglio", BERSAGLIO_PREDEFINITO),
        )
        per_terna.setdefault(chiave, []).append(voce)

    errori: list[str] = []
    for (crate, configurazione, bersaglio), voci in per_terna.items():
        eseguiti, trovati = esegui_harness(crate, configurazione, bersaglio)
        errori.extend(trovati)
        if not eseguiti:
            continue
        for voce in voci:
            for identita in voce["test"]:
                risultato = eseguiti.get(identita)
                if risultato is None:
                    errori.append(
                        f"{voce['ramo']}: «{identita}» non compare fra i test "
                        f"eseguiti di {crate}. Il ramo torna a dipendere dallo "
                        "scheduling, e la copertura a cambiare fra due corse."
                    )
                elif risultato == "ignored":
                    errori.append(f"{voce['ramo']}: «{identita}» e' marcato `#[ignore]`")
                elif risultato != "ok":
                    errori.append(f"{voce['ramo']}: «{identita}» non passa («{risultato}»)")
    return errori


def main(argv: list[str] | None = None) -> int:
    argparse.ArgumentParser(description=__doc__).parse_args(argv)

    if not REGISTRO.exists():
        print(f"{REGISTRO}: registro assente.", file=sys.stderr)
        return 2
    gruppi = json.loads(REGISTRO.read_text(encoding="utf-8"))["gruppi"]
    if not gruppi:
        print(
            "registro vuoto: senza sonde dichiarate non c'e' niente che tenga "
            "lo scheduling fuori dalla misura.",
            file=sys.stderr,
        )
        return 1

    errori = struttura(gruppi)
    if not errori:
        errori = esegui(gruppi)

    for messaggio in errori:
        print(messaggio, file=sys.stderr)
    if errori:
        return 1

    quante = sum(len(voce["test"]) for voce in gruppi)
    print(
        f"sonde deterministiche eseguite: {quante} su {len(gruppi)} rami che "
        "dipendevano dallo scheduling."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
