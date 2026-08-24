#!/usr/bin/env python3
"""Che cosa AddressSanitizer vede davvero nel target `filegdb_reader`.

# L'affermazione che questo gate esiste per impedire

«Il percorso FileGDB e' coperto da AddressSanitizer» e' vera a meta', e la meta'
falsa e' quella che conta. Il target e' compilato con `-Zsanitizer=address`, ma
GDAL no: `libgdal.so` e' una libreria di sistema, compilata da qualcun altro
senza strumentazione. Presentare una campagna verde come «GDAL non ha difetti di
memoria» sarebbe l'errore che il blocco `fuzz.filegdb` esisteva per evitare.

# Che cosa copre, e che cosa **non** copre

Copre per intero il **nostro** codice -- i driver, il core, il modello, il
wrapper Rust del fork governato di `gdal` -- perche' e' compilato con la
strumentazione: ogni accesso passa da un controllo sulla shadow memory, e
accessi fuori dai limiti, use-after-free, overflow di stack e di globali
diventano un abort con diagnostica.

Dentro GDAL **non copre gli accessi**. Il controllo lo inserisce il compilatore
in ogni load e store del codice che compila; codice non strumentato non consulta
la shadow memory, e i suoi accessi non vengono verificati. Un trabocco dentro un
buffer di GDAL, una lettura oltre i limiti di una sua struttura, un
use-after-free acceduto direttamente: nessuno lascia traccia.

Cio' che resta osservabile al confine e' piu' stretto, e va detto per quello che
e':

* gli errori dell'**allocatore**, perche' `malloc` e `free` sono intercettati
  dal runtime anche quando a chiamarli e' codice non strumentato: doppia
  liberazione, liberazione di un puntatore non allocato, dimensioni assurde;
* gli errori dentro le funzioni **intercettate** -- `memcpy`, `strcpy` e simili
  -- dove a controllare gli argomenti e' l'interceptor del runtime, non il
  codice chiamante;
* i **crash ordinari**, che libFuzzer riporta con i propri gestori di segnale.

Fuori da questi tre casi, un difetto di memoria dentro GDAL passa inosservato.
Il riferimento e' l'algoritmo di ASan:
<https://github.com/google/sanitizers/wiki/addresssanitizeralgorithm>.

# Perche' i numeri, e quali

«Non e' strumentata» e' una frase che invecchia: basta che qualcuno costruisca
GDAL da sorgente con `-fsanitize=address`, o che il link cambi, e la frase resta
scritta mentre il fatto cambia -- in meglio o in peggio.

Il fatto si misura **sulla libreria**: quanti simboli del runtime del sanitizer
compaiono in `libgdal.so`. Zero significa non strumentata; un numero diverso da
zero significa che la prosa qui sopra va riscritta. Il nostro binario, per
confronto, ne porta centinaia.

I contatori di copertura sono una **proprieta' diversa**, e questo file non li
confonde piu' con la strumentazione: dicono che il fuzzer non ha feedback dentro
GDAL, cioe' che le mutazioni non sono guidate da cio' che accade li'. Sono due
limiti distinti, e servono due misure.

# Perche' il gate rimisura invece di rileggere

Il livello 2 non riesegue la misura: rilegge l'artefatto. Un artefatto che
descrivesse una `libgdal` diversa da quella installata lascerebbe verde
l'invariante su un ambiente che nessuno ha guardato.

Il gate rimisura percio' **la libreria di questa macchina** a ogni esecuzione --
costa un `nm` -- e pretende che sia non strumentata. L'identita' della libreria
misurata resta registrata, e quando non coincide con quella locale il gate lo
dice: i numeri di copertura appartengono a quell'ambiente, la non-strumentazione
e' verificata qui.

# Uso

    python3 scripts/check_asan_filegdb.py
    python3 scripts/check_asan_filegdb.py --registra <misura.json>

Il secondo modo lo invoca `scripts/asan-filegdb.sh`, che e' il solo posto in cui
il binario strumentato viene interrogato.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))

from check_profondita_fuzz import (  # noqa: E402
    BERSAGLI,
    impronta_del_perimetro,
    leggi_registro,
    percorsi_del_perimetro,
)

ROOT = Path(__file__).resolve().parent.parent
BERSAGLIO = BERSAGLI["filegdb_reader"]
ARTEFATTO = ROOT / "assurance" / "asan-filegdb.json"

# La libreria che il target collega. Il nome sta qui perche' il gate deve
# accorgersi se un giorno sparisse: un binario che non collega piu' GDAL non sta
# esercitando FileGDB.
LIBRERIA = "libgdal"

# I fatti che la misura deve portare, e il valore che li rende il confine
# descritto dalla prosa. Non sono soglie: sono identita'.
ATTESI: dict[str, Any] = {
    # Il fatto centrale, e l'unico che dice qualcosa sulla **strumentazione**:
    # nessun simbolo del runtime del sanitizer dentro la libreria.
    "simboli_asan_nella_libreria": 0,
    # Proprieta' diversa, e altrettanto vera: nessun contatore di copertura da
    # una libreria condivisa, cioe' nessun feedback per il fuzzer dentro GDAL.
    "moduli_con_contatori": 1,
    "file_sorgente_gdal_strumentati": 0,
    "runtime_asan_nel_binario": True,
    "libreria_gdal_dentro_l_albero_di_build": False,
}

# Le affermazioni che l'artefatto deve portare per esteso. Un numero senza la
# frase che dice che cosa significa e' un numero che qualcuno rileggera' come
# gli fa comodo.
AFFERMAZIONI = (
    "copre_il_nostro_codice",
    "non_copre_gli_accessi_dentro_gdal",
    "copre_gli_errori_dell_allocatore",
    "copre_le_funzioni_intercettate",
    "non_guida_le_mutazioni_dentro_gdal",
)


class MisuraImpossibile(Exception):
    """La libreria locale non e' interrogabile: il gate non puo' concludere."""


# --- la misura locale, rifatta a ogni esecuzione ----------------------------


def _libreria_locale(percorso_registrato: str) -> Path:
    """La `libgdal` di **questa** macchina.

    Si preferisce il percorso registrato quando esiste, perche' e' quello che il
    processo misurato ha davvero caricato; altrimenti si chiede al linker
    dinamico, che e' cio' che il processo caricherebbe qui.
    """
    if percorso_registrato and Path(percorso_registrato).is_file():
        return Path(percorso_registrato)

    # `ldconfig` puo' non esistere affatto -- una macchina di sviluppo che non
    # e' Linux -- e in quel caso `subprocess` solleva invece di restituire.
    try:
        esito = subprocess.run(
            ["ldconfig", "-p"], capture_output=True, text=True, check=False
        )
    except OSError as errore:
        raise MisuraImpossibile(
            f"non si puo' interrogare il linker dinamico ({errore}): questo gate "
            "verifica la GDAL con cui il target verrebbe costruito **qui**, e va "
            "eseguito dove GDAL e' installata."
        ) from errore
    if esito.returncode == 0:
        for riga in esito.stdout.splitlines():
            if LIBRERIA in riga and "=>" in riga:
                candidato = Path(riga.split("=>")[-1].strip())
                if candidato.is_file():
                    return candidato
    raise MisuraImpossibile(
        f"{LIBRERIA} non trovata su questa macchina: questo gate verifica la "
        "GDAL con cui il target verrebbe costruito **qui**, e senza di essa non "
        "puo' concludere niente. Va eseguito dove GDAL e' installata."
    )


def simboli_asan(percorso: Path) -> int:
    """Quanti simboli del runtime del sanitizer porta un file ELF.

    E' la misura diretta della **strumentazione**, e non va confusa con i
    contatori di copertura: una libreria costruita con `-fsanitize=address`
    porta centinaia di `__asan_*`; una costruita senza non ne porta nessuno.
    """
    quanti = 0
    visto = False
    for argomenti in (["nm", "-D", str(percorso)], ["nm", str(percorso)]):
        try:
            esito = subprocess.run(argomenti, capture_output=True, text=True, check=False)
        except OSError:
            continue
        if esito.returncode == 0:
            visto = True
            quanti = max(quanti, sum(1 for r in esito.stdout.splitlines() if "__asan" in r))
    if not visto:
        # Zero perche' `nm` non ha risposto non e' zero perche' i simboli non ci
        # sono, e leggere il primo come il secondo direbbe «non strumentata» di
        # una libreria mai guardata.
        raise MisuraImpossibile(
            f"`nm` non ha saputo leggere {percorso}: senza, «zero simboli» "
            "sarebbe l'assenza della misura, non l'assenza dei simboli."
        )
    return quanti


def identita(percorso: Path) -> dict[str, str]:
    """Build-id e digest: due modi indipendenti di dire *quale* libreria."""
    try:
        esito = subprocess.run(
            ["readelf", "-n", str(percorso)], capture_output=True, text=True, check=False
        )
    except OSError:
        esito = None
    trovato = (
        re.search(r"Build ID:\s*([0-9a-f]+)", esito.stdout)
        if esito is not None and esito.returncode == 0
        else None
    )
    return {
        "percorso_risolto": str(percorso),
        "build_id": trovato.group(1) if trovato else "",
        "sha256": hashlib.sha256(percorso.read_bytes()).hexdigest(),
    }


# --- la verifica ------------------------------------------------------------


def verifica(misura: dict[str, Any]) -> list[str]:
    errori: list[str] = []

    if misura.get("target") != BERSAGLIO.nome:
        errori.append(
            f"la misura riguarda il target «{misura.get('target')}», questo gate "
            f"descrive «{BERSAGLIO.nome}»"
        )

    for chiave, atteso in ATTESI.items():
        ottenuto = misura.get(chiave)
        if ottenuto != atteso or isinstance(ottenuto, bool) is not isinstance(atteso, bool):
            errori.append(
                f"`{chiave}` vale «{ottenuto}», il confine descritto da questo "
                f"gate richiede «{atteso}». Se il fatto e' cambiato davvero, e' "
                "la prosa a dover cambiare con esso, non il numero da solo."
            )

    contatori = misura.get("contatori_di_copertura")
    if not isinstance(contatori, int) or isinstance(contatori, bool) or contatori <= 0:
        errori.append(
            f"`contatori_di_copertura` vale «{contatori}»: un binario senza "
            "contatori non e' strumentato per la copertura, e una campagna su di "
            "esso non sarebbe guidata da niente."
        )

    collegata = misura.get("libreria_collegata")
    if not isinstance(collegata, dict):
        errori.append("`libreria_collegata` assente: non si sa quale GDAL sia stata usata")
        collegata = {}
    else:
        soname = collegata.get("soname", "")
        if not isinstance(soname, str) or LIBRERIA not in soname:
            errori.append(
                f"`libreria_collegata.soname` e' «{soname}»: il target non collega "
                f"`{LIBRERIA}`, quindi non sta esercitando FileGDB."
            )
        for campo in ("percorso_risolto", "build_id", "sha256"):
            if not collegata.get(campo):
                errori.append(
                    f"`libreria_collegata.{campo}` assente: senza, la misura non "
                    "dice **quale** libreria ha guardato."
                )

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
                "corrente produrrebbe: va rifatta con "
                "`bash scripts/asan-filegdb.sh`."
            )

    # --- e infine la libreria di **questa** macchina -----------------------
    #
    # Rileggere l'artefatto non basta: descriverebbe una GDAL che qui potrebbe
    # non esserci. La non-strumentazione si rimisura, e costa un `nm`.
    try:
        locale = _libreria_locale(collegata.get("percorso_risolto", ""))
    except MisuraImpossibile as impossibile:
        return errori + [str(impossibile)]

    quanti = simboli_asan(locale)
    if quanti != 0:
        errori.append(
            f"la `{LIBRERIA}` di questa macchina ({locale}) porta {quanti} simboli "
            "del runtime di AddressSanitizer: **e' strumentata**. E' una buona "
            "notizia, e rende falsa la prosa di questo gate: va riscritta prima "
            "che l'invariante torni verde."
        )
    return errori


def _riga_di_esito(misura: dict[str, Any], locale: Path) -> str:
    collegata = misura.get("libreria_collegata", {})
    corrente = identita(locale)
    stessa = corrente["build_id"] and corrente["build_id"] == collegata.get("build_id")
    nota = (
        ""
        if stessa
        else (
            f"\n  nota: la libreria misurata (build-id {collegata.get('build_id')}) "
            f"non e' quella locale (build-id {corrente['build_id']}). I contatori "
            "appartengono all'ambiente di misura; la non-strumentazione e' "
            "verificata qui."
        )
    )
    return (
        f"confine ASan del target {BERSAGLIO.nome}: {misura['contatori_di_copertura']} "
        f"contatori su {misura['moduli_con_contatori']} modulo, e zero simboli del "
        f"runtime in {locale.name}. Il nostro codice e' strumentato, GDAL no.{nota}"
    )


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
                "misurato sul binario strumentato e sulla libreria che collega. "
                "Prodotto da scripts/asan-filegdb.sh; letto da "
                "scripts/check_asan_filegdb.py, che rimisura la libreria locale "
                "invece di credere a questo file."
            ),
            "target": BERSAGLIO.nome,
            "impronta_perimetro": impronta,
            **grezza,
            "che_cosa_significa": {
                "copre_il_nostro_codice": (
                    "i driver, il core, il modello e il wrapper Rust del fork "
                    "governato di `gdal` sono compilati con la strumentazione: "
                    "ogni load e store passa da un controllo sulla shadow memory, "
                    "e accessi fuori dai limiti, use-after-free e overflow di "
                    "stack o di globali diventano un abort con diagnostica."
                ),
                "non_copre_gli_accessi_dentro_gdal": (
                    "il controllo lo inserisce il compilatore nel codice che "
                    "compila. `libgdal.so` non e' strumentata, quindi i suoi "
                    "accessi non consultano la shadow memory e non vengono "
                    "verificati: un trabocco dentro un suo buffer, una lettura "
                    "oltre i limiti di una sua struttura o un use-after-free "
                    "acceduto direttamente non lasciano traccia."
                ),
                "copre_gli_errori_dell_allocatore": (
                    "`malloc` e `free` passano dal runtime del sanitizer anche "
                    "quando a chiamarli e' codice non strumentato: doppia "
                    "liberazione, liberazione di un puntatore non allocato e "
                    "dimensioni assurde restano osservabili."
                ),
                "copre_le_funzioni_intercettate": (
                    "dentro `memcpy`, `strcpy` e le altre funzioni intercettate a "
                    "controllare gli argomenti e' l'interceptor del runtime, non "
                    "il codice chiamante: li' un errore di GDAL viene visto."
                ),
                "non_guida_le_mutazioni_dentro_gdal": (
                    "`libgdal.so` non porta contatori di copertura -- proprieta' "
                    "distinta dalla strumentazione del sanitizer -- quindi il "
                    "fuzzer e' cieco dentro di essa: le mutazioni sono guidate "
                    "dalla copertura del percorso Rust."
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
            f"{documento.get('simboli_asan_nella_libreria')} simboli del runtime "
            f"nella libreria, {documento.get('contatori_di_copertura')} contatori "
            f"su {documento.get('moduli_con_contatori')} modulo"
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

    locale = _libreria_locale(misura.get("libreria_collegata", {}).get("percorso_risolto", ""))
    print(_riga_di_esito(misura, locale))
    return 0


if __name__ == "__main__":
    sys.exit(main())
