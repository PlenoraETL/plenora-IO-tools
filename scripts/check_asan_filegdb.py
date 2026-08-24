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

Il gate rimisura percio' **la libreria di questa macchina** a ogni esecuzione, e
pretende che sia non strumentata. L'identita' della libreria misurata resta
registrata, e quando non coincide con quella locale il gate lo dice: i numeri di
copertura appartengono a quell'ambiente, la non-strumentazione e' verificata qui.

# Quale libreria e' «quella di questa macchina»

Non quella scritta nell'artefatto. Un percorso registrato mesi fa puo' esistere
ancora mentre il loader ne sceglierebbe un'altra -- una `libgdal.so.35` accanto
alla `.32`, un `LD_LIBRARY_PATH`, un aggiornamento che ha lasciato il vecchio
file al suo posto -- e misurare quel file direbbe «non strumentata» di una
libreria che nessun processo caricherebbe piu'.

La domanda si gira percio' a chi la decide davvero. Il gate costruisce un
binario **corrente** con il tier GDB acceso e chiede a `ldd` quale `libgdal`
risolve: e' la libreria che il target caricherebbe se lo si costruisse adesso,
ed e' esattamente quella che viene misurata. Il percorso registrato serve solo a
dire se e' la stessa di allora.

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

# Il binario da cui si chiede al loader quale GDAL userebbe **oggi**. Non e' il
# target del fuzzing: quello richiede la toolchain nightly e il sanitizer, che
# per questa domanda non servono. E' un nostro binario con lo stesso tier acceso,
# quindi con la stessa dipendenza e lo stesso soname.
CRATE_FEATURE_ON = "plenora-io-cli"
FEATURE_GDB = "gdal-backend"

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


def _binario_feature_on() -> Path:
    """Un binario **costruito adesso** con il tier GDB acceso.

    Serve solo come domanda da porre al loader: un eseguibile che dipende da
    GDAL e' l'unico modo di far dire a `ldd` quale libreria verrebbe scelta con
    la configurazione di oggi. Costruirlo e' incrementale, e quando la build non
    riesce la conclusione giusta non e' «non strumentata» ma «non misurabile».
    """
    comando = [
        "cargo",
        "build",
        "--locked",
        "--quiet",
        "--package",
        CRATE_FEATURE_ON,
        "--features",
        FEATURE_GDB,
        "--message-format",
        "json-render-diagnostics",
    ]
    try:
        esito = subprocess.run(
            comando, cwd=str(ROOT), capture_output=True, text=True, check=False
        )
    except OSError as errore:
        raise MisuraImpossibile(
            f"`cargo` non e' invocabile ({errore}): senza un binario con il tier "
            "GDB acceso non si puo' chiedere al loader quale GDAL userebbe."
        ) from errore
    if esito.returncode != 0:
        coda = "\n".join(esito.stderr.strip().splitlines()[-5:])
        raise MisuraImpossibile(
            f"il binario con il tier GDB acceso non si costruisce qui (`cargo "
            f"build --package {CRATE_FEATURE_ON} --features {FEATURE_GDB}` e' "
            f"uscito con {esito.returncode}): questo gate verifica la GDAL con "
            "cui il target verrebbe costruito **qui**, e va eseguito dove GDAL "
            f"e' installata.\n{coda}"
        )

    binario: Path | None = None
    for riga in esito.stdout.splitlines():
        try:
            messaggio = json.loads(riga)
        except json.JSONDecodeError:
            continue
        eseguibile = messaggio.get("executable")
        if messaggio.get("reason") == "compiler-artifact" and eseguibile:
            candidato = Path(eseguibile)
            if candidato.is_file():
                binario = candidato
    if binario is None:
        raise MisuraImpossibile(
            f"`cargo build --package {CRATE_FEATURE_ON}` non ha prodotto nessun "
            "eseguibile: senza, non c'e' niente da interrogare."
        )
    return binario


def _righe_di_ldd(binario: Path) -> list[str]:
    """Che cosa il loader risolverebbe per quel binario, riga per riga."""
    try:
        esito = subprocess.run(
            ["ldd", str(binario)], capture_output=True, text=True, check=False
        )
    except OSError as errore:
        raise MisuraImpossibile(
            f"`ldd` non e' invocabile ({errore}): questo gate verifica la GDAL "
            "con cui il target verrebbe costruito **qui**, e va eseguito dove "
            "GDAL e' installata."
        ) from errore
    if esito.returncode != 0:
        raise MisuraImpossibile(
            f"`ldd {binario}` e' uscito con {esito.returncode}: non si sa quali "
            "librerie quel binario caricherebbe."
        )
    return esito.stdout.splitlines()


def _gdal_da_ldd(righe: list[str]) -> Path | None:
    """La `libgdal` risolta, se il loader ne ha risolta una.

    «Risolta» conta: `ldd` stampa anche le dipendenze che **non** trova, con un
    `not found` al posto del percorso, e prendere l'ultimo campo di quelle righe
    darebbe un nome di file inesistente.
    """
    for riga in righe:
        if LIBRERIA not in riga.lower() or "=>" not in riga:
            continue
        risolto = riga.split("=>", 1)[1].strip().split(" (")[0].strip()
        if not risolto or risolto == "not found":
            continue
        candidato = Path(risolto)
        if candidato.is_file():
            return candidato
    return None


def _libreria_locale() -> Path:
    """La `libgdal` che il loader sceglierebbe **oggi**.

    Non prende il percorso registrato nemmeno come suggerimento: quel file puo'
    esistere ancora mentre il loader ne sceglie un altro, e misurarlo direbbe
    «non strumentata» di una libreria che nessun processo caricherebbe piu'.
    """
    binario = _binario_feature_on()
    libreria = _gdal_da_ldd(_righe_di_ldd(binario))
    if libreria is None:
        raise MisuraImpossibile(
            f"il binario {binario}, costruito con `{FEATURE_GDB}`, non risolve "
            f"nessuna `{LIBRERIA}`: o GDAL non e' installata qui, o il tier GDB "
            "non collega piu' la libreria. In nessuno dei due casi questo gate "
            "puo' concludere qualcosa sulla strumentazione."
        )
    return libreria


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
        locale = _libreria_locale()
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

    locale = _libreria_locale()
    print(_riga_di_esito(misura, locale))
    return 0


if __name__ == "__main__":
    sys.exit(main())
