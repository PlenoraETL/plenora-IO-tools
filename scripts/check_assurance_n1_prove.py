#!/usr/bin/env python3
"""Le prove di ASSURANCE-N1 sono **test eseguiti**, non riferimenti testuali.

# Che cosa questo gate aggiunge a `check_assurance_n1.py`

Quello verifica che il registro sia coerente e che una prova sia **nominata**.
La prima stesura cercava `fn <nome>(` nel file del gruppo, e non bastava: un
simbolo che esiste puo' essere

* un helper senza `#[test]`, che nessuno esegue;
* un test marcato `#[ignore]`, che il harness elenca e salta;
* un test sotto un `cfg` inattivo nella configurazione in cui si misura;
* un test che esiste con quel nome ma in un altro modulo.

Nessuno dei quattro casi copre un ramo, e tutti e quattro passavano.

Qui la prova e' verificata **eseguendola**: si lancia il harness per ogni
coppia `(crate, configurazione)` dichiarata, si legge l'elenco dei test
davvero eseguiti con il loro esito, e si pretende che ogni identita'
dichiarata compaia **una volta sola** e con esito `ok`.

# Perche' la configurazione fa parte dell'identita'

`--all-features` abilita `gdal-backend`, e il ramo stub di `driver-filegdb`
esiste solo **senza**. Un test che copre uno dei due non copre l'altro, e una
prova che non dice in quale configurazione gira e' ambigua esattamente dove
conta.

# Coperto e irraggiungibile non sono la stessa cosa

Un ramo puo' essere fermo perche' una guardia a monte lo rende inarrivabile
dall'API pubblica. Quel ramo **non e' coperto**, e presentarlo come tale
sarebbe la compensazione che ASSURANCE-N1 esiste per escludere.

Le prove con `esito: irraggiungibile` devono percio' dichiarare:

* `righe` — quali righe restano scoperte;
* `guardia` — quale controllo a monte rifiuta per primo.

Il test che le accompagna non copre quelle righe: prova la **precedenza**, cioe'
che sia la guardia dichiarata a rifiutare. E' un contratto piu' forte di una
nota, perche' rossa se la precedenza cambia.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
REGISTRO = ROOT / "assurance" / "registries" / "assurance-n1-copertura-negativa.json"

CONFIGURAZIONI = {"default": [], "all-features": ["--all-features"]}

# Il bersaglio fa parte dell'identita' della misura quanto la
# configurazione: `--lib` su un crate binario non elenca nulla, e un elenco
# vuoto non e' un verde.
BERSAGLI = {"lib": ["--lib"], "bins": ["--bins"], "tests": ["--tests"]}
BERSAGLIO_PREDEFINITO = "lib"
ESITI_PROVA = {"coperto", "irraggiungibile"}
CAMPI_PROVA = {"crate", "test", "configurazione", "esito"}
CAMPI_IRRAGGIUNGIBILE = {"righe", "guardia"}

# `test tests::nome ... ok` / `... ignored` / `... FAILED`
RIGA_ESITO = re.compile(r"^test\s+(?P<id>\S+)\s+\.\.\.\s+(?P<esito>ok|ignored|FAILED)\s*$")


def comando_test(
    crate: str, configurazione: str, bersaglio: str = BERSAGLIO_PREDEFINITO
) -> list[str]:
    """Il comando del harness, in un solo posto.

    Lo condivide `check_release_contract.py`: due costruttori del comando
    divergerebbero, e divergerebbero in silenzio — uno misurerebbe un
    bersaglio e l'altro ne dichiarerebbe un altro.
    """
    return [
        "cargo", "test", "-p", crate,
        *CONFIGURAZIONI[configurazione],
        *BERSAGLI[bersaglio],
    ]


def analizza_uscita(testo: str) -> tuple[dict[str, str], list[str]]:
    """`(test-id -> esito, errori)` dall'uscita del harness.

    Un'identita' che compare due volte e' un errore e non un dettaglio: due
    test omonimi in moduli diversi renderebbero ambiguo **quale** dei due
    chiude il gruppo, e il registro non potrebbe dirlo.
    """
    esiti: dict[str, str] = {}
    duplicati: list[str] = []
    for riga in testo.splitlines():
        trovato = RIGA_ESITO.match(riga.strip())
        if trovato is None:
            continue
        identita = trovato.group("id")
        if identita in esiti:
            duplicati.append(
                f"{identita}: identita' duplicata nell'elenco dei test eseguiti. "
                "Con due test omonimi il registro non puo' dire quale chiude il "
                "gruppo."
            )
            continue
        esiti[identita] = trovato.group("esito")
    return esiti, duplicati


def prove_dichiarate(gruppi: list[dict]) -> list[tuple[str, dict]]:
    """`(gruppo, prova)` per ogni prova dichiarata da un gruppo chiuso."""
    fuori: list[tuple[str, dict]] = []
    for voce in gruppi:
        if voce.get("disposizione") != "chiuso":
            continue
        for prova in voce.get("prova") or []:
            if isinstance(prova, dict):
                fuori.append((voce["gruppo"], prova))
    return fuori


def verifica_prove(
    gruppi: list[dict], elenchi: dict[tuple[str, str, str], dict[str, str]]
) -> list[str]:
    """Ogni prova dichiarata e' un test eseguito e passato.

    `elenchi` mappa `(crate, configurazione, bersaglio)` all'esito dei test
    eseguiti.
    """
    errori: list[str] = []
    for gruppo, prova in prove_dichiarate(gruppi):
        mancanti = CAMPI_PROVA - set(prova)
        if mancanti:
            errori.append(f"{gruppo}: prova con campi mancanti {sorted(mancanti)}")
            continue

        configurazione = prova["configurazione"]
        if configurazione not in CONFIGURAZIONI:
            errori.append(
                f"{gruppo}: configurazione «{configurazione}» non ammessa; "
                f"scegliere fra {sorted(CONFIGURAZIONI)}"
            )
            continue

        if prova["esito"] not in ESITI_PROVA:
            errori.append(
                f"{gruppo}: esito «{prova['esito']}» non ammesso; "
                f"scegliere fra {sorted(ESITI_PROVA)}"
            )
            continue

        if prova["esito"] == "irraggiungibile":
            senza = CAMPI_IRRAGGIUNGIBILE - set(prova)
            if senza:
                errori.append(
                    f"{gruppo}: prova «irraggiungibile» senza {sorted(senza)}. "
                    "Un ramo non coperto va dichiarato con le righe che restano "
                    "scoperte e con la guardia che rifiuta per prima, altrimenti "
                    "e' indistinguibile da un ramo coperto."
                )
                continue

        bersaglio = prova.get("bersaglio", BERSAGLIO_PREDEFINITO)
        if bersaglio not in BERSAGLI:
            errori.append(
                f"{gruppo}: bersaglio «{bersaglio}» non ammesso; "
                f"scegliere fra {sorted(BERSAGLI)}"
            )
            continue

        chiave = (prova["crate"], configurazione, bersaglio)
        elenco = elenchi.get(chiave)
        if elenco is None:
            errori.append(
                f"{gruppo}: nessuna misura per {prova['crate']} in configurazione "
                f"«{configurazione}» sul bersaglio «{bersaglio}». Una prova non "
                "misurata non e' una prova."
            )
            continue

        identita = prova["test"]
        esito = elenco.get(identita)
        if esito is None:
            errori.append(
                f"{gruppo}: «{identita}» non compare fra i test eseguiti di "
                f"{prova['crate']} ({configurazione}). Un simbolo che esiste ma "
                "non viene eseguito non copre niente: puo' essere un helper "
                "senza `#[test]` o un test sotto un `cfg` inattivo."
            )
        elif esito == "ignored":
            errori.append(
                f"{gruppo}: «{identita}» e' marcato `#[ignore]`. Il harness lo "
                "elenca e lo salta: chiude un gruppo senza eseguire nulla."
            )
        elif esito != "ok":
            errori.append(f"{gruppo}: «{identita}» non passa (esito «{esito}»)")
    return errori


def coppie_da_misurare(gruppi: list[dict]) -> list[tuple[str, str, str]]:
    """Le terne `(crate, configurazione, bersaglio)` distinte, senza ripetizioni.

    Un test condiviso fra piu' gruppi si esegue **una volta sola**: ripetere la
    misura non la rende piu' vera, e allunga il checkpoint di minuti che non
    aggiungono nulla.
    """
    viste: list[tuple[str, str, str]] = []
    for _, prova in prove_dichiarate(gruppi):
        chiave = (
            prova.get("crate"),
            prova.get("configurazione"),
            prova.get("bersaglio", BERSAGLIO_PREDEFINITO),
        )
        if None in chiave or chiave[1] not in CONFIGURAZIONI or chiave[2] not in BERSAGLI:
            continue
        if chiave not in viste:
            viste.append(chiave)
    return viste


def misura(
    coppie: list[tuple[str, str, str]]
) -> tuple[dict[tuple[str, str, str], dict[str, str]], list[str]]:
    elenchi: dict[tuple[str, str, str], dict[str, str]] = {}
    errori: list[str] = []
    for crate, configurazione, bersaglio in coppie:
        comando = comando_test(crate, configurazione, bersaglio)
        esecuzione = subprocess.run(
            comando, cwd=ROOT, capture_output=True, text=True, check=False
        )
        esiti, duplicati = analizza_uscita(esecuzione.stdout)
        errori.extend(f"{crate} ({configurazione}): {d}" for d in duplicati)
        if not esiti:
            errori.append(
                f"{crate} ({configurazione}): il harness non ha elencato alcun test. "
                "Senza elenco non si sa se le prove siano state eseguite, e un "
                "silenzio non va letto come un verde."
            )
        elenchi[(crate, configurazione, bersaglio)] = esiti
    return elenchi, errori


def main() -> int:
    argparse.ArgumentParser(description=__doc__).parse_args()
    gruppi = json.loads(REGISTRO.read_text(encoding="utf-8"))["gruppi"]

    coppie = coppie_da_misurare(gruppi)
    elenchi, errori = misura(coppie)
    errori.extend(verifica_prove(gruppi, elenchi))

    for messaggio in errori:
        print(messaggio, file=sys.stderr)
    if errori:
        return 1

    quante = len(prove_dichiarate(gruppi))
    coperte = sum(1 for _, p in prove_dichiarate(gruppi) if p.get("esito") == "coperto")
    print(
        f"prove ASSURANCE-N1 eseguite: {quante} su {len(coppie)} configurazioni; "
        f"{coperte} coprono un ramo, {quante - coperte} provano un'irraggiungibilita'"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
