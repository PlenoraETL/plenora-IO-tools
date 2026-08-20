"""ASSURANCE-N1: censimento della copertura negativa mancante.

# Che cos'e'

Un elenco **nominale** dei rami d'errore che nessuna verifica esegue: ne' i test
unitari ne' il replay deterministico del fuzzer. Ogni voce ha una disposizione
dichiarata, e nessuna puo' restare senza.

Non e' debito causato da S9. La verifica su `effc4ab` ha stabilito che questi
rami erano gia' scoperti prima della migrazione: S9 li ha **resi visibili**,
perche' un errore da una riga e' diventato quattro e la diagnostica
differenziale ha cominciato a guardare. La distinzione conta perche' decide chi
paga: S9 garantisce cio' che ha cambiato, e cio' che ha solo illuminato va
censito qui invece di sparire.

# Le due modalita', e perche' sono separate

    --integrita   il registro e' coerente con il codice
    --release     il debito e' a zero

**Un registro coerente non significa che i rami siano coperti.** Tenere insieme
le due cose produrrebbe un verde che si legge come «la copertura negativa e'
a posto» mentre dice soltanto «l'elenco e' scritto bene» -- ed e' la forma di
falso verde che questa serie di checkpoint ha incontrato cinque volte.

`--integrita` puo' essere verde da subito, ed e' quello che gira in CI: serve a
impedire che il censimento si sfaldi mentre lo si lavora.

`--release` e' rosso finche' resta un gruppo senza copertura, e **blocca la
qualifica finale**. Non ha senso farlo girare a ogni commit; ha senso che nessun
candidato di release passi senza.

# Disposizioni ammesse

    test_tabellare   una classe di equivalenza chiusa da un test parametrico
    fixture          un input costruito a mano che raggiunge il ramo
    seme_fuzz        un seme versionato, per i rami che vivono dietro un parser
    strutturale      markup o righe non eseguibili: nessuna prova dovuta
    difensivo        ramo raggiungibile solo se una dipendenza cambia
    chiuso           gia' coperto, con il test che lo copre
"""

from __future__ import annotations

import argparse
import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
REGISTRO = ROOT / "docs" / "assurance" / "ASSURANCE_N1_copertura_negativa.json"

DISPOSIZIONI_APERTE = {"test_tabellare", "fixture", "seme_fuzz"}
DISPOSIZIONI_CHIUSE = {"strutturale", "difensivo", "chiuso"}
DISPOSIZIONI = DISPOSIZIONI_APERTE | DISPOSIZIONI_CHIUSE

CAMPI = {"gruppo", "file", "righe", "raggiunto_da_replay", "disposizione", "nota"}


def carica() -> list[dict]:
    if not REGISTRO.exists():
        print(f"{REGISTRO}: registro assente.", file=sys.stderr)
        raise SystemExit(2)
    return json.loads(REGISTRO.read_text(encoding="utf-8"))["gruppi"]


def integrita(gruppi: list[dict]) -> list[str]:
    """Il registro e' coerente: non dice che i rami siano coperti."""
    errori: list[str] = []
    visti: set[str] = set()
    for voce in gruppi:
        nome = voce.get("gruppo", "<senza nome>")
        mancanti = CAMPI - set(voce)
        if mancanti:
            errori.append(f"{nome}: campi mancanti {sorted(mancanti)}")
        if nome in visti:
            errori.append(f"{nome}: voce duplicata")
        visti.add(nome)
        disposizione = voce.get("disposizione")
        if disposizione not in DISPOSIZIONI:
            errori.append(
                f"{nome}: disposizione «{disposizione}» non ammessa; "
                f"scegliere fra {sorted(DISPOSIZIONI)}"
            )
        if not voce.get("nota"):
            # Una disposizione senza ragione e' una casella riempita.
            errori.append(f"{nome}: disposizione senza nota")
        percorso = voce.get("file")
        if percorso and not (ROOT / percorso).exists():
            errori.append(f"{nome}: il file {percorso} non esiste piu'")
    return errori


def debito(gruppi: list[dict]) -> list[dict]:
    return [v for v in gruppi if v.get("disposizione") in DISPOSIZIONI_APERTE]


def main() -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    modo = argomenti.add_mutually_exclusive_group(required=True)
    modo.add_argument("--integrita", action="store_true", help="il registro e' coerente")
    modo.add_argument("--release", action="store_true", help="il debito e' a zero")
    opzioni = argomenti.parse_args()

    gruppi = carica()

    if opzioni.integrita:
        errori = integrita(gruppi)
        for messaggio in errori:
            print(messaggio, file=sys.stderr)
        if errori:
            return 1
        aperti = len(debito(gruppi))
        print(
            f"ASSURANCE-N1 integro: {len(gruppi)} gruppi, tutti con disposizione.\n"
            f"  ATTENZIONE: {aperti} gruppi sono ancora senza copertura.\n"
            f"  Questo esito dice che il REGISTRO e' coerente, non che i rami\n"
            f"  siano coperti. Per quello serve --release."
        )
        return 0

    # --release
    errori = integrita(gruppi)
    if errori:
        print("il registro non e' integro: --release non e' interpretabile.", file=sys.stderr)
        for messaggio in errori:
            print(messaggio, file=sys.stderr)
        return 2

    aperti = debito(gruppi)
    if aperti:
        print(
            f"ASSURANCE-N1: {len(aperti)} gruppi senza copertura, su {len(gruppi)}.",
            file=sys.stderr,
        )
        for voce in aperti:
            print(
                f"  {voce['gruppo']:56s} {voce['disposizione']:14s} "
                f"righe={voce['righe']}",
                file=sys.stderr,
            )
        print(
            "\nLa qualifica di release non puo' passare finche' il debito non e' a zero.",
            file=sys.stderr,
        )
        return 1

    print(f"ASSURANCE-N1 a zero: {len(gruppi)} gruppi, nessuno senza copertura.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
