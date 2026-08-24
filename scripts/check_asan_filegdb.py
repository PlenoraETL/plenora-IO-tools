#!/usr/bin/env python3
"""Che cosa AddressSanitizer vede davvero nel target `filegdb_reader`.

# L'affermazione che questo gate esiste per impedire

«Il percorso FileGDB e' coperto da AddressSanitizer» e' vera a meta', e la meta'
falsa e' quella che conta. Il target e' compilato con `-Zsanitizer=address`, ma
GDAL no: `libgdal.so` e' una libreria di sistema, collegata dinamicamente,
compilata da qualcun altro senza strumentazione. Presentare una campagna verde
come «GDAL non ha difetti di memoria» sarebbe l'errore che il blocco
`fuzz.filegdb` esisteva per evitare.

Il confine c'e', ed e' netto. Questo gate lo tiene **misurato**, non dichiarato.

# Che cosa AddressSanitizer copre, e che cosa no

Copre per intero il **nostro** codice -- i driver, il core, il modello, il
wrapper Rust del fork governato di `gdal` -- perche' e' compilato con la
strumentazione: accessi fuori dai limiti, use-after-free, overflow di stack e di
globali diventano un abort con diagnostica.

Al confine con GDAL resta l'**intercettazione dell'allocatore**. `malloc` e
`free` passano dal runtime del sanitizer anche quando a chiamarli e' codice non
strumentato, quindi un accesso di GDAL che cada nella redzone di
un'allocazione ASan viene visto. E' una difesa reale, e non e' la stessa cosa.

Non copre gli accessi **interni** a GDAL: un trabocco dentro un buffer di stack
di GDAL, un accesso fuori limite dentro una sua struttura, un overflow che resti
dentro l'allocazione -- nessuno dei tre lascia traccia. E non c'e' copertura di
codice: `libgdal.so` non porta contatori, quindi il fuzzer e' **cieco** dentro
di essa e le mutazioni non sono guidate da cio' che succede li'.

# Perche' i numeri e non le parole

«Non e' strumentata» e' una frase che invecchia: basta che qualcuno costruisca
GDAL da sorgente dentro l'immagine, o che il link diventi statico, e la frase
resta scritta mentre il fatto cambia -- in meglio o in peggio, ma cambia.

L'artefatto porta percio' cio' che e' stato **misurato** sul binario: quale
`libgdal` e' stata collegata e da dove, quanti moduli portano contatori, quanti
contatori ci sono, e quanti file sorgente di GDAL compaiono nei dati di
copertura. Il gate rilegge quei numeri e pretende che raccontino il confine che
la prosa descrive.

# Uso

    python3 scripts/check_asan_filegdb.py
    python3 scripts/check_asan_filegdb.py --registra <misura.json>

Il secondo modo lo invoca `scripts/asan-filegdb.sh`, che e' il solo posto in cui
il binario viene interrogato.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))

from check_profondita_fuzz import (  # noqa: E402
    BERSAGLI,
    impronta_del_perimetro,
    percorsi_del_perimetro,
    leggi_registro,
)

ROOT = Path(__file__).resolve().parent.parent
BERSAGLIO = BERSAGLI["filegdb_reader"]
ARTEFATTO = ROOT / "assurance" / "asan-filegdb.json"

# La libreria che il target collega, e che **non** e' strumentata. Il nome sta
# qui perche' il gate deve accorgersi se un giorno sparisse: un binario che non
# collega piu' GDAL non sta esercitando FileGDB.
LIBRERIA = "libgdal"

# I fatti che la misura deve portare, e il valore che li rende il confine
# descritto dalla prosa. Non sono soglie: sono identita'.
#
# `moduli_con_contatori` a uno e' il fatto centrale: libFuzzer conta i moduli
# che portano contatori di copertura, e uno solo vuol dire che l'eseguibile e'
# strumentato e nessuna libreria condivisa lo e'. Due vorrebbe dire che qualcuno
# ha costruito GDAL con la strumentazione, e la prosa di questo file andrebbe
# riscritta -- in meglio.
ATTESI: dict[str, Any] = {
    "moduli_con_contatori": 1,
    "file_sorgente_gdal_strumentati": 0,
    "runtime_asan_collegato": True,
    "libreria_gdal_dentro_l_albero_di_build": False,
}

# Le affermazioni che l'artefatto deve portare per esteso. Un numero senza la
# frase che dice che cosa significa e' un numero che qualcuno rileggera' come
# gli fa comodo.
AFFERMAZIONI = (
    "copre_il_nostro_codice",
    "copre_l_intercettazione_dell_allocatore",
    "non_copre_gli_accessi_interni_a_gdal",
    "non_guida_le_mutazioni_dentro_gdal",
)


def verifica(misura: dict[str, Any]) -> list[str]:
    errori: list[str] = []

    if misura.get("target") != BERSAGLIO.nome:
        errori.append(
            f"la misura riguarda il target «{misura.get('target')}», questo gate "
            f"descrive «{BERSAGLIO.nome}»"
        )

    for chiave, atteso in ATTESI.items():
        ottenuto = misura.get(chiave)
        if ottenuto != atteso:
            errori.append(
                f"`{chiave}` vale «{ottenuto}», il confine descritto da questo "
                f"gate richiede «{atteso}». Se il fatto e' cambiato davvero, e' "
                "la prosa a dover cambiare con esso, non il numero da solo."
            )

    contatori = misura.get("contatori_di_copertura")
    if not isinstance(contatori, int) or isinstance(contatori, bool) or contatori <= 0:
        errori.append(
            f"`contatori_di_copertura` vale «{contatori}»: un binario senza "
            "contatori non e' strumentato, e una campagna su di esso non sarebbe "
            "guidata da niente."
        )

    collegata = misura.get("libreria_collegata")
    if not isinstance(collegata, dict):
        errori.append("`libreria_collegata` assente: non si sa quale GDAL sia stata usata")
    else:
        soname = collegata.get("soname", "")
        if not isinstance(soname, str) or LIBRERIA not in soname:
            errori.append(
                f"`libreria_collegata.soname` e' «{soname}»: il target non collega "
                f"`{LIBRERIA}`, quindi non sta esercitando FileGDB."
            )
        percorso = collegata.get("percorso_risolto", "")
        if not isinstance(percorso, str) or not percorso:
            errori.append("`libreria_collegata.percorso_risolto` assente")

    mancanti = sorted(set(AFFERMAZIONI) - set(misura.get("che_cosa_significa", {})))
    if mancanti:
        errori.append(
            f"`che_cosa_significa` senza {mancanti}. I numeri dicono dov'e' il "
            "confine; queste frasi dicono che cosa ci si puo' leggere, e senza di "
            "esse la misura verrebbe riletta come «GDAL e' coperto»."
        )

    registro = leggi_registro(BERSAGLIO)
    percorsi, problemi = percorsi_del_perimetro(BERSAGLIO, registro)
    errori.extend(problemi)
    if percorsi:
        attesa, problemi = impronta_del_perimetro(percorsi)
        errori.extend(problemi)
        if attesa and misura.get("impronta_perimetro") != attesa:
            errori.append(
                f"impronta del perimetro diversa: la misura dice "
                f"«{misura.get('impronta_perimetro')}», il working tree "
                f"«{attesa}». Il binario misurato non e' quello che il codice "
                "corrente produrrebbe: la misura va rifatta con "
                "`bash scripts/asan-filegdb.sh`."
            )
    return errori


def main(argv: list[str] | None = None) -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument("--registra", type=Path, help="la misura grezza da registrare")
    opzioni = argomenti.parse_args(argv)

    if opzioni.registra:
        grezza = json.loads(opzioni.registra.read_text(encoding="utf-8"))
        registro = leggi_registro(BERSAGLIO)
        percorsi, problemi = percorsi_del_perimetro(BERSAGLIO, registro)
        if problemi:
            for messaggio in problemi:
                print(messaggio, file=sys.stderr)
            return 1
        impronta, problemi = impronta_del_perimetro(percorsi)
        if problemi:
            for messaggio in problemi:
                print(messaggio, file=sys.stderr)
            return 1

        documento = {
            "schema_version": 1,
            "descrizione": (
                "Il confine di AddressSanitizer nel target `filegdb_reader`, "
                "misurato sul binario strumentato. Prodotto da "
                "scripts/asan-filegdb.sh; letto da "
                "scripts/check_asan_filegdb.py."
            ),
            "target": BERSAGLIO.nome,
            "impronta_perimetro": impronta,
            **grezza,
            "che_cosa_significa": {
                "copre_il_nostro_codice": (
                    "i driver, il core, il modello e il wrapper Rust del fork "
                    "governato di `gdal` sono compilati con la strumentazione: "
                    "accessi fuori dai limiti, use-after-free, overflow di stack e "
                    "di globali diventano un abort con diagnostica."
                ),
                "copre_l_intercettazione_dell_allocatore": (
                    "`malloc` e `free` passano dal runtime del sanitizer anche "
                    "quando a chiamarli e' codice non strumentato: un accesso di "
                    "GDAL che cada nella redzone di un'allocazione ASan viene "
                    "visto. E' una difesa reale, e non e' copertura."
                ),
                "non_copre_gli_accessi_interni_a_gdal": (
                    "un trabocco dentro un buffer di stack di GDAL, un accesso "
                    "fuori limite dentro una sua struttura, un overflow che resti "
                    "dentro l'allocazione: nessuno dei tre lascia traccia."
                ),
                "non_guida_le_mutazioni_dentro_gdal": (
                    "`libgdal.so` non porta contatori di copertura, quindi il "
                    "fuzzer e' cieco dentro di essa: le mutazioni sono guidate "
                    "dalla copertura del percorso Rust, non da cio' che succede "
                    "oltre il confine."
                ),
            },
        }
        ARTEFATTO.parent.mkdir(parents=True, exist_ok=True)
        ARTEFATTO.write_text(
            json.dumps(documento, ensure_ascii=False, indent=2) + "\n",
            encoding="utf-8",
            newline="\n",
        )
        print(
            f"confine ASan registrato in {ARTEFATTO.relative_to(ROOT).as_posix()}: "
            f"{documento.get('contatori_di_copertura')} contatori su "
            f"{documento.get('moduli_con_contatori')} modulo, "
            f"{documento.get('file_sorgente_gdal_strumentati')} file di GDAL strumentati"
        )
        return 0

    if not ARTEFATTO.exists():
        print(
            "assurance/asan-filegdb.json: misura assente. Si produce con "
            "`bash scripts/asan-filegdb.sh`.",
            file=sys.stderr,
        )
        return 1
    try:
        misura = json.loads(ARTEFATTO.read_text(encoding="utf-8"))
    except json.JSONDecodeError as errore:
        print(f"assurance/asan-filegdb.json: non e' JSON leggibile ({errore})", file=sys.stderr)
        return 1

    errori = verifica(misura)
    for messaggio in errori:
        print(messaggio, file=sys.stderr)
    if errori:
        return 1

    collegata = misura.get("libreria_collegata", {})
    print(
        f"confine ASan del target {BERSAGLIO.nome}: "
        f"{misura['contatori_di_copertura']} contatori su "
        f"{misura['moduli_con_contatori']} modulo, nessuno da "
        f"{collegata.get('soname')} ({collegata.get('percorso_risolto')}). "
        "Il nostro codice e' strumentato, GDAL no."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
