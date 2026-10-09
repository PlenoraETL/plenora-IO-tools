#!/usr/bin/env python3
"""Classifica il crash di un fuzz target: **noto** o **nuovo**.

# Il problema, scritto prima del meccanismo

> gestire finding noti senza disabilitare il bersaglio, senza nascondere
> difetti nuovi, e senza dichiarare completa una campagna che si e' interrotta.

La quarantena del progetto e' per **bersaglio**: mettere `geoparquet_reader` in
`fuzz/quarantine.txt` smetterebbe di esplorarlo, e il file stesso riserva quella
via ai finding dove «uno smoke che fallisce sempre non e' un gate, e' rumore».
Un finding solo non giustifica di smettere di cercarne altri.

# Perche' una lista di digest non basta

Perche' il fuzzer rigenera lo stesso difetto da un input diverso, o ne produce
una variante. La prova sta negli artefatti locali di `geoparquet_reader`: due
misurano 3.966 byte, la stessa dimensione del seme del finding, con digest tutti
diversi dal suo. Stessa famiglia, quattro digest. Una lista di digest li
riconoscerebbe uno per volta, e ogni variante sarebbe un rosso da triare a mano.

# Che cosa si riconosce invece

La **famiglia**, cioe' il punto in cui il difetto si manifesta:

* il modulo, col nome della crate spogliato della versione -- `parquet-59.3.0`
  e `parquet-60.0.0` sono la stessa crate, e un finding aperto a monte non
  smette di esserlo perche' il pin sale;
* la **forma** del messaggio, con le cifre ridotte a `N` -- «the len is 2 but
  the index is 2» e «the len is 56 but the index is 56» sono lo stesso difetto
  su due ingressi.

Il numero di riga **non** e' nella firma: si sposta fra versioni, e legarlo
renderebbe la voce stantia a ogni aggiornamento. Resta registrato come dato,
perche' chi rilegge la voce voglia vedere dove si era manifestato.

# Che cosa garantisce, e che cosa no

Garantisce di riconoscere un crash **compatibile** con una firma registrata. Non
garantisce che sia lo stesso difetto.

La distinzione non e' formale. Due difetti diversi possono produrre lo stesso
errore d'indice nello stesso modulo -- un conteggio incoerente e un offset
calcolato male finiscono entrambi su `index out of bounds`, e la firma non li
separa. «Noto» vuol dire percio' «gia' visto qualcosa che si presenta cosi'»,
non «gia' capito».

Da questo discendono due conseguenze, e sono il prezzo della garanzia piu'
debole:

* **ogni input si conserva**, anche quando la classificazione dice «noto»,
  insieme al referto che la sostiene. Se fosse identita' certa si potrebbe
  scartare il duplicato; essendo compatibilita', l'input e' l'unica cosa che
  permette di riesaminare la classificazione piu' tardi;
* la firma resta **stretta** e la corrispondenza **congiunta** -- modulo *e*
  forma del messaggio, per il bersaglio dichiarato -- perche' allargarla
  aumenterebbe i crash che finiscono nella stessa cesta senza aumentare di
  nulla cio' che si sa di loro. Un altro messaggio nello stesso modulo, lo
  stesso messaggio in un altro modulo, lo stesso crash su un altro bersaglio:
  tutti fuori. E un crash che non corrisponde a nessuna voce fa fallire lo
  smoke come prima.

# La seconda firma: l'esaurimento di memoria

Un'allocazione oltre `-malloc_limit_mb` (che libFuzzer pone uguale a
`-rss_limit_mb`) non e' un panico: libFuzzer stampa
`==N== ERROR: libFuzzer: out-of-memory (malloc(N))` e lo stack
dell'allocazione. La firma e' allora:

* il **tipo** `esaurimento-memoria`, distinto da `panico`: un panico e un
  esaurimento nello stesso modulo sono due finding;
* il **primo frame utile** dello stack, ridotto a modulo, come per il panico,
  e a **funzione**: il nome semplice, senza percorso, hash ne' parametri
  generici. Un frame inline arriva dal simbolizzatore col solo nome
  (`read_thrift_vec<...>`) e uno non inline col percorso intero; il modulo,
  che viene dal file, tiene la firma stretta lo stesso;
* la forma del messaggio, `out-of-memory (malloc(N))`.

# Come si legge lo stack, e perche' in modo conservativo

Un errore di lettura qui non deve poter produrre un falso «noto». Percio':

* l'intestazione deve combaciare con **l'intera** riga;
* lo stack va dal primo `#0` dopo l'intestazione alla prima riga vuota, a
  `SUMMARY:` o a un altro marcatore d'errore; ogni riga in mezzo deve avere la
  forma di una riga di stack, con i numeri consecutivi, altrimenti la firma e'
  **illeggibile**;
* si saltano **solo** i frame del runtime, per nome **esatto** e luogo
  riconosciuto: il sanitizer e l'intercettazione di `malloc` senza file, il
  sanitizer col solo nome del file, libFuzzer dal suo sorgente nel registro di
  cargo, la libreria standard che alloca dal sorgente della toolchain. Sono gli
  insiemi chiusi qui sotto, presi dall'uscita vera della CI. Un frame con un
  file del repository (`crates/`, `vendor/`, `fuzz/fuzz_targets/`) non si salta
  mai;
* il primo frame che non si salta deve avere un file: senza file la firma e'
  illeggibile, perche' firmare con il chiamante vorrebbe dire firmare un altro
  punto.

E i marcatori decidono prima dello stack. Un marcatore che non e' ne'
l'esaurimento da `malloc` ne' il segnale mortale di un panico -- un leak, un
errore di AddressSanitizer, un timeout, l'esaurimento misurato sull'RSS -- fa
il crash **nuovo**, mai «senza crash»; due marcatori nello stesso log pure.
Un'uscita senza marcatori ma con un codice diverso da zero e' illeggibile.

Per questo lo smoke cerca un `llvm-symbolizer` prima di correre: senza, ogni
esaurimento di memoria e' illeggibile, cioe' rosso.

# Perche' non dichiara completa una campagna interrotta

Perche' un finding noto **interrompe comunque** il bersaglio: libFuzzer si ferma
al primo crash, e il tempo restante non e' stato esplorato. La classificazione
dice «noto», non «completo». Chi chiama distingue i due stati, e l'esito lo
riporta: un bersaglio fermato a meta' non si conta fra quelli che hanno finito.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
REGISTRO = ROOT / "assurance" / "registries" / "finding-noti-fuzz.json"

CAMPI = (
    "id",
    "bersaglio",
    "modulo",
    "forma_del_messaggio",
    "dove_e_tracciato",
    "non_promette",
    "quando_si_toglie",
)

#: I tipi di crash che una voce puo' descrivere. Una voce senza `tipo` e' un
#: panico: e' la forma delle voci scritte prima che esistesse il secondo tipo.
TIPI = ("panico", "esaurimento-memoria")

#: L'intestazione di un esaurimento di memoria da `malloc`, sull'**intera**
#: riga: `==30580== ERROR: libFuzzer: out-of-memory (malloc(2315255472))`. Un
#: suffisso, o un prefisso diverso dal pid di libFuzzer, non e' questa riga.
ESAURIMENTO = re.compile(
    r"^==\d+== ERROR: libFuzzer: (?P<messaggio>out-of-memory \(malloc\(\d+\)\))$"
)

#: Il segno con cui libFuzzer chiude un panico Rust: `deadly signal`.
SEGNALE_MORTALE = re.compile(r"^==\d+== ERROR: libFuzzer: deadly signal$")

#: Ogni riga con cui libFuzzer o un sanitizer dichiarano un errore:
#: `==N== ERROR: libFuzzer: ...`, `==N==ERROR: AddressSanitizer: ...`,
#: `==N==ERROR: LeakSanitizer: ...`. Quelle che il classificatore non sa
#: leggere fanno il crash **nuovo**, mai «senza crash».
MARCATORE = re.compile(r"^==\d+==\s*ERROR:\s*\S")

#: Una riga di stack: `#N 0xINDIRIZZO`, poi `in <funzione> <dove>` oppure il
#: solo modulo fra parentesi. Una riga dentro lo stack che non e' cosi' rende
#: la firma illeggibile: non si salta.
RIGA_DI_STACK = re.compile(
    r"^\s+#(?P<numero>\d+) 0x[0-9a-fA-F]+"
    r"(?: in (?P<funzione>.+?) (?P<dove>\S+(?: \(BuildId: [0-9a-fA-F]+\))?)"
    r"| (?P<solo_modulo>\(\S+\+0x[0-9a-fA-F]+\)(?: \(BuildId: [0-9a-fA-F]+\))?))$"
)

#: `<percorso assoluto>:<riga>[:<colonna>]`
CON_FILE = re.compile(r"^(?P<percorso>/\S+?):(?P<riga>\d+)(?::\d+)?$")

#: `(<binario>+0x<offset>)`, con o senza BuildId: un frame senza file.
SENZA_FILE = re.compile(r"^\(\S+\+0x[0-9a-fA-F]+\)(?: \(BuildId: [0-9a-fA-F]+\))?$")

#: I frame del runtime che si saltano: nome **esatto** e luogo riconosciuto.
#: Nient'altro si salta. Sono quelli che la CI produce sull'input registrato
#: (corsa 37735627087): libFuzzer stampa lo stack da dentro il proprio gancio
#: su `malloc`, quindi prima vengono lui e il sanitizer, poi `malloc`, poi la
#: libreria standard che alloca, poi chi ha chiesto la memoria.
#:
#: Senza file: il sanitizer e l'intercettazione di `malloc`, che il binario
#: porta senza informazioni di riga.
RUNTIME_SENZA_FILE = frozenset(
    {
        "__sanitizer_print_stack_trace",
        "malloc",
        "calloc",
        "realloc",
        "__interceptor_malloc",
        "__interceptor_calloc",
        "__interceptor_realloc",
    }
)
#: Il sanitizer, che il simbolizzatore riporta col solo nome del file.
RUNTIME_SANITIZER = frozenset(
    {
        "__sanitizer::RunMallocHooks(void*, unsigned long)",
        "__asan::Allocator::Allocate(unsigned long, unsigned long, "
        "__sanitizer::BufferedStackTrace*, __asan::AllocType, bool)",
        "__asan::asan_malloc(unsigned long, __sanitizer::BufferedStackTrace*)",
    }
)
FILE_DEL_SANITIZER = frozenset(
    {"sanitizer_common.cpp", "asan_allocator.cpp", "asan_malloc_linux.cpp"}
)
#: libFuzzer, dal sorgente vendorizzato in `libfuzzer-sys` nel registro di cargo.
RUNTIME_LIBFUZZER = frozenset(
    {
        "fuzzer::PrintStackTrace()",
        "fuzzer::Fuzzer::HandleMalloc(unsigned long)",
        "fuzzer::MallocHook(void const volatile*, unsigned long)",
    }
)
PERCORSO_LIBFUZZER = re.compile(
    r"^/(?:[^/]+/)*registry/src/[^/]+/libfuzzer-sys-\d+\.\d+\.\d+/libfuzzer/Fuzzer\w+\.cpp$"
)
#: La libreria standard che alloca: nome semplice esatto, percorso del
#: sorgente della toolchain.
RUNTIME_STD = frozenset(
    {
        "alloc",
        "alloc_zeroed",
        "alloc_impl",
        "alloc_impl_runtime",
        "allocate",
        "allocate_in",
        "try_allocate_in",
        "with_capacity_in",
        "with_capacity",
    }
)
PERCORSO_STD = re.compile(r"^/rustc/[0-9a-f]{40}/library/(?:alloc|core|std)/src/")

#: Segmenti che dicono «codice di questo repository»: un frame che li porta
#: ferma sempre lo skip, qualunque nome abbia.
SEGMENTI_DEL_REPOSITORY = ("/crates/", "/vendor/", "/fuzz/fuzz_targets/")

#: `thread '<nome>' panicked at <percorso>:<riga>:<colonna>:`
PANICO = re.compile(r"panicked at ([^\s:]+(?:/[^\s:]+)*):(\d+):(\d+)")

#: Una crate nel registro di cargo: `nome-1.2.3` oppure `nome-1.2.3-rc.1`.
VERSIONE = re.compile(r"^(?P<nome>.+?)-\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$")

#: `Test unit written to fuzz/artifacts/<target>/crash-<sha1>`
#:
#: L'artefatto si prende da qui e non dal file piu' recente della directory: due
#: corse vicine, o una corsa parallela, e «il piu' recente» sarebbe l'input di
#: un'altra. Il referto deve nominare **questo** input.
ARTEFATTO = re.compile(r"Test unit written to (\S+)")


def modulo_normalizzato(percorso: str) -> str:
    """Il percorso col segmento di versione tolto dal nome della crate.

    `.../parquet-59.3.0/src/a/b.rs` diventa `parquet/src/a/b.rs`. Senza questo,
    ogni aggiornamento del pin renderebbe stantia una voce che descrive un
    difetto ancora aperto.
    """
    pezzi = percorso.replace("\\", "/").split("/")
    for indice, pezzo in enumerate(pezzi):
        trovato = VERSIONE.match(pezzo)
        if trovato is None:
            continue
        # Trovata la radice della crate: tutto cio' che sta prima e' il
        # percorso del registro di cargo sulla macchina che ha eseguito, e
        # cambia da macchina a macchina. Tenerlo renderebbe la firma dipendente
        # da dove gira il fuzzer.
        return "/".join([trovato.group("nome"), *pezzi[indice + 1 :]])
    # Nessun segmento con versione: e' codice nostro, e il percorso relativo
    # alla radice del repository e' gia' la forma stabile.
    relativo = "/".join(pezzi)
    radice = f"{ROOT.name}/"
    taglio = relativo.rfind(radice)
    return relativo[taglio + len(radice) :] if taglio != -1 else relativo


def forma_del_messaggio(messaggio: str) -> str:
    """Il messaggio con le cifre ridotte a `N`, cioe' la sua famiglia."""
    return re.sub(r"\d+", "N", messaggio).strip()


def funzione_normalizzata(nome: str) -> str:
    """Il nome semplice di una funzione Rust: senza percorso, hash e
    parametri generici.

    `parquet::parquet_thrift::read_thrift_vec::h0123456789abcdef`,
    `parquet::parquet_thrift::read_thrift_vec::<KeyValue, Slice>` e il frame
    inline `read_thrift_vec<KeyValue, Slice>` diventano tutti
    `read_thrift_vec`: la demangling legacy, la v0 e il nome che il
    simbolizzatore da' a una funzione inline. I parametri dipendono dal tipo
    letto, non dal punto in cui si alloca.
    """
    senza_hash = re.sub(r"::h[0-9a-f]{16}$", "", nome.strip())
    risultato: list[str] = []
    profondita = 0
    for carattere in senza_hash:
        if carattere == "<":
            profondita += 1
        elif carattere == ">":
            profondita = max(0, profondita - 1)
        elif profondita == 0:
            risultato.append(carattere)
    senza_generici = re.sub(r"::+$", "", re.sub(r"::::+", "::", "".join(risultato)))
    return senza_generici.rsplit("::", 1)[-1].strip()


def _frame_del_runtime(funzione: str, dove: str) -> bool:
    """Il frame e' del runtime -- sanitizer, libFuzzer, allocatore, libreria
    standard -- e si salta. Solo per nome **esatto** e luogo riconosciuto."""
    if any(segmento in dove for segmento in SEGMENTI_DEL_REPOSITORY):
        return False
    if SENZA_FILE.match(dove):
        return funzione in RUNTIME_SENZA_FILE
    if dove in FILE_DEL_SANITIZER:
        return funzione in RUNTIME_SANITIZER
    con_file = CON_FILE.match(dove)
    if con_file is None:
        return False
    percorso = con_file.group("percorso")
    if PERCORSO_LIBFUZZER.match(percorso):
        return funzione in RUNTIME_LIBFUZZER
    if PERCORSO_STD.match(percorso):
        return funzione_normalizzata(funzione) in RUNTIME_STD
    return False


def _esaurimento_osservato(righe: list[str], indice: int, artefatto: str) -> dict[str, str]:
    """La firma dell'esaurimento la cui intestazione sta a `righe[indice]`.

    Lo stack si legge dal primo `#0` dopo l'intestazione fino alla prima riga
    vuota, a `SUMMARY:` o a un altro marcatore d'errore. Dentro lo stack ogni
    riga deve essere una riga di stack; i frame del runtime si saltano, e il
    primo che non lo e' da' la firma -- purche' abbia un file. Ogni deviazione
    da questa forma da' `illeggibile`.
    """
    illeggibile = {"stato": "illeggibile", "artefatto": artefatto}
    messaggio = ESAURIMENTO.match(righe[indice])
    if messaggio is None:
        return illeggibile
    inizio = None
    for posizione in range(indice + 1, len(righe)):
        riga = righe[posizione]
        if MARCATORE.match(riga) or riga.startswith("SUMMARY:"):
            return illeggibile
        if re.match(r"^\s+#0 ", riga):
            inizio = posizione
            break
    if inizio is None:
        return illeggibile
    atteso = 0
    for riga in righe[inizio:]:
        if not riga.strip() or riga.startswith("SUMMARY:") or MARCATORE.match(riga):
            break
        frame = RIGA_DI_STACK.match(riga)
        if frame is None or int(frame.group("numero")) != atteso:
            return illeggibile
        atteso += 1
        funzione = frame.group("funzione")
        if funzione is None:
            # Il solo modulo, senza nome: nessuna funzione da firmare, e non
            # e' un frame del runtime riconosciuto.
            return illeggibile
        dove = frame.group("dove")
        if _frame_del_runtime(funzione, dove):
            continue
        con_file = CON_FILE.match(dove)
        if con_file is None:
            # Il primo frame utile senza file: firmare con il chiamante
            # vorrebbe dire firmare un altro punto.
            return illeggibile
        testo_messaggio = messaggio.group("messaggio")
        return {
            "stato": "letto",
            "tipo": "esaurimento-memoria",
            "modulo": modulo_normalizzato(con_file.group("percorso")),
            "funzione": funzione_normalizzata(funzione),
            "riga": con_file.group("riga"),
            "messaggio": testo_messaggio,
            "forma_del_messaggio": forma_del_messaggio(testo_messaggio),
            "artefatto": artefatto,
        }
    return illeggibile


def _panico_osservato(testo: str, artefatto: str) -> dict[str, str] | None:
    """Il primo panico nel testo, ridotto a firma; `None` se non ce n'e'."""
    trovato = PANICO.search(testo)
    if trovato is None:
        return None
    # Il messaggio sta sulla riga **dopo** quella del panico: la riga del
    # panico finisce con i due punti, e prenderne la coda dava una stringa
    # vuota. Si salta percio' al primo a capo e si legge la riga seguente.
    coda = testo[trovato.end() :]
    a_capo = coda.find("\n")
    prima_riga = "" if a_capo == -1 else coda[a_capo + 1 :].split("\n", 1)[0]
    return {
        "stato": "letto",
        "tipo": "panico",
        "modulo": modulo_normalizzato(trovato.group(1)),
        "funzione": "",
        "riga": trovato.group(2),
        "messaggio": prima_riga.strip(),
        "forma_del_messaggio": forma_del_messaggio(prima_riga),
        "artefatto": artefatto,
    }


def osserva(testo: str, codice_uscita: int | None = None) -> dict[str, str]:
    """Che cosa dice l'uscita di una corsa: `senza-crash`, `illeggibile`,
    `altro` (un errore che nessuna firma descrive) o `letto` con la firma.

    I marcatori d'errore decidono, non la sola presenza di un panico o di uno
    stack:

    * nessun marcatore e nessun panico: senza crash -- ma se la corsa e'
      uscita con un codice diverso da zero, illeggibile;
    * un marcatore che non e' ne' l'esaurimento da `malloc` ne' il segnale
      mortale di un panico (un leak, un errore di AddressSanitizer, un timeout,
      l'esaurimento sull'RSS): `altro`, cioe' nuovo;
    * l'esaurimento e un altro marcatore qualsiasi: `altro` -- due errori nello
      stesso log non sono il finding registrato;
    * il segnale mortale senza un panico leggibile: illeggibile.
    """
    righe = [riga.rstrip("\r") for riga in testo.splitlines()]
    trovato = ARTEFATTO.search(testo)
    artefatto = trovato.group(1) if trovato else ""
    marcatori = [indice for indice, riga in enumerate(righe) if MARCATORE.match(riga)]
    esaurimenti = [i for i in marcatori if ESAURIMENTO.match(righe[i])]
    mortali = [i for i in marcatori if SEGNALE_MORTALE.match(righe[i])]
    altri = [i for i in marcatori if i not in esaurimenti and i not in mortali]

    if altri or (esaurimenti and (mortali or len(esaurimenti) > 1)):
        riga = righe[altri[0]] if altri else righe[marcatori[0]]
        messaggio = re.sub(r"^==\d+==\s*", "", riga).strip()
        return {
            "stato": "letto",
            "tipo": "altro",
            "modulo": "",
            "funzione": "",
            "riga": "",
            "messaggio": messaggio,
            "forma_del_messaggio": forma_del_messaggio(messaggio),
            "artefatto": artefatto,
        }
    if esaurimenti:
        return _esaurimento_osservato(righe, esaurimenti[0], artefatto)
    panico = _panico_osservato(testo, artefatto)
    if panico is not None:
        return panico
    if mortali or (codice_uscita is not None and codice_uscita != 0):
        return {"stato": "illeggibile", "artefatto": artefatto}
    return {"stato": "senza-crash"}


def crash_osservato(testo: str) -> dict[str, str] | None:
    """La firma del crash, o `None` se non ce n'e' una leggibile."""
    osservato = osserva(testo)
    return osservato if osservato["stato"] == "letto" else None


def registro_ben_formato(documento: Any) -> list[str]:
    """I motivi per cui il registro non e' leggibile; vuoto se lo e'."""
    if not isinstance(documento, dict) or not isinstance(
        documento.get("finding"), list
    ):
        return ["il registro non porta un elenco `finding`"]
    motivi: list[str] = []
    visti: set[str] = set()
    for indice, voce in enumerate(documento["finding"]):
        dove = f"finding[{indice}]"
        if not isinstance(voce, dict):
            motivi.append(f"{dove}: non e' un oggetto")
            continue
        for campo in CAMPI:
            if not isinstance(voce.get(campo), str) or not voce[campo].strip():
                motivi.append(f"{dove}: `{campo}` assente o vuoto")
        tipo = voce.get("tipo", "panico")
        if tipo not in TIPI:
            motivi.append(f"{dove}: `tipo` «{tipo}» sconosciuto")
        # Un esaurimento senza funzione si riconoscerebbe dal solo modulo: ogni
        # allocazione di quel file finirebbe nella stessa cesta.
        if tipo == "esaurimento-memoria" and (
            not isinstance(voce.get("funzione"), str) or not voce["funzione"].strip()
        ):
            motivi.append(f"{dove}: `funzione` assente o vuota")
        identita = voce.get("id")
        if isinstance(identita, str):
            if identita in visti:
                motivi.append(f"{dove}: `id` «{identita}» ripetuto")
            visti.add(identita)
    return motivi


def classifica(
    bersaglio: str, testo: str, documento: Any, codice_uscita: int | None = None
) -> dict[str, Any]:
    """`stato` fra `senza-crash`, `illeggibile`, `noto` e `nuovo`, con cio' che
    lo sostiene."""
    osservato = osserva(testo, codice_uscita)
    if osservato["stato"] == "senza-crash":
        return {"stato": "senza-crash"}
    if osservato["stato"] == "illeggibile":
        return {"stato": "illeggibile", "osservato": osservato}

    for voce in documento["finding"]:
        if voce["bersaglio"] != bersaglio:
            continue
        # Congiunta, e non «una delle due»: lo stesso messaggio in un altro
        # modulo e' un difetto diverso, e un altro messaggio nello stesso modulo
        # pure. Allentarla qui sarebbe il modo di nascondere un finding nuovo
        # dietro uno noto.
        if (
            voce.get("tipo", "panico") == osservato["tipo"]
            and voce["modulo"] == osservato["modulo"]
            and voce.get("funzione", "") == osservato["funzione"]
            and voce["forma_del_messaggio"] == osservato["forma_del_messaggio"]
        ):
            return {"stato": "noto", "id": voce["id"], "osservato": osservato}
    return {"stato": "nuovo", "osservato": osservato}


def _mostra(percorso: Path) -> str:
    """Il percorso come lo legge chi sta nel repository.

    Fuori dal repository si mostra intero: e' il caso delle prove, che scrivono
    in una directory temporanea, e un errore di rendering li' nasconderebbe
    l'errore vero che la prova sta cercando.
    """
    try:
        return percorso.relative_to(ROOT).as_posix()
    except ValueError:
        return str(percorso)


def conserva(
    destinazione: Path, bersaglio: str, esito: dict[str, Any], voce: Any
) -> list[str]:
    """Scrive input e referto, e ritorna i percorsi scritti.

    Si conserva **sempre**, noto o nuovo. Un «noto» e' una compatibilita' di
    firma, non un'identita' dimostrata: buttare l'input perche' somiglia a uno
    gia' visto vorrebbe dire decidere che sono lo stesso difetto proprio nel
    momento in cui non lo si sa.

    Il nome viene dal digest dei byte, non dall'ora: due corse che trovano lo
    stesso input scrivono lo stesso file invece di accumularne copie, e due
    input diversi non si sovrascrivono mai.
    """
    osservato = esito["osservato"]
    cartella = destinazione / bersaglio
    cartella.mkdir(parents=True, exist_ok=True)

    byte: bytes | None = None
    percorso = osservato.get("artefatto") or ""
    if percorso:
        sorgente = Path(percorso)
        if not sorgente.is_absolute():
            sorgente = ROOT / sorgente
        if sorgente.is_file():
            byte = sorgente.read_bytes()

    nome = (
        hashlib.sha256(byte).hexdigest()[:16]
        if byte is not None
        else "input-non-trovato"
    )
    scritti: list[str] = []
    if byte is not None:
        ingresso = cartella / f"{nome}.input"
        ingresso.write_bytes(byte)
        scritti.append(_mostra(ingresso))

    referto = {
        "schema_version": 1,
        "bersaglio": bersaglio,
        "classificazione": esito["stato"],
        "che_cosa_significa": (
            "«noto» vuol dire che il crash e' **compatibile** con la firma "
            "registrata -- stesso modulo, stessa forma del messaggio -- non che "
            "sia lo stesso difetto. Due difetti diversi possono presentarsi "
            "cosi'. L'input e' conservato qui perche' la classificazione si "
            "possa riesaminare."
        ),
        "firma_osservata": {
            "tipo": osservato.get("tipo", ""),
            "funzione": osservato.get("funzione", ""),
            "modulo": osservato.get("modulo", ""),
            "riga": osservato.get("riga", ""),
            "messaggio": osservato.get("messaggio", ""),
            "forma_del_messaggio": osservato.get("forma_del_messaggio", ""),
        },
        "artefatto_dichiarato_dalla_corsa": percorso,
        "input_conservato": scritti[0] if scritti else None,
    }
    if esito["stato"] == "noto":
        referto["finding_compatibile"] = esito["id"]
        referto["dove_e_tracciato"] = voce["dove_e_tracciato"]

    documento = cartella / f"{nome}.json"
    documento.write_text(
        json.dumps(referto, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
        newline="\n",
    )
    scritti.append(_mostra(documento))
    return scritti


def verifica_campagna(
    percorso: Path, revisione_attesa: str | None = None
) -> list[str]:
    """I motivi per cui l'ultima campagna non e' **completa**; vuoto se lo e'.

    Un bersaglio fermato a un finding noto ha smesso di esplorare: libFuzzer non
    riparte dopo un crash, e il tempo restante non e' stato usato. Lo smoke
    esce 0 perche' lo sviluppo prosegua sugli altri bersagli -- ed e' giusto --
    ma quello 0 non deve diventare «campagna completata» piu' in la' nella
    catena. Questa funzione e' il punto in cui i due si separano, e gira nella
    qualificazione finale.
    """
    if not percorso.exists():
        return [
            f"{percorso.name} assente: nessuna campagna registrata, e "
            "l'assenza di un verbale non e' una campagna riuscita"
        ]
    try:
        documento = json.loads(percorso.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as guasto:
        return [f"{percorso.name}: non si legge ({guasto})"]

    # La revisione prima di tutto: se il verbale parla di un altro albero, cio'
    # che dice degli esiti non riguarda questo, e leggerlo sarebbe peggio che
    # non averlo. Le domande pero' sono **due**, e non hanno la stessa risposta.
    dichiarata = documento.get("revisione")
    if not isinstance(dichiarata, str) or not dichiarata:
        return [
            f"{percorso.name}: non dichiara la revisione su cui la campagna e' "
            "girata. Un verbale senza revisione qualifica qualunque albero, e "
            "non e' quello che una campagna prova."
        ]

    if revisione_attesa is not None:
        # La domanda del **checkpoint**: «e' il verbale di questa corsa?». Qui
        # l'uguaglianza e' la risposta giusta, e la regola dell'allowlist
        # sarebbe troppo larga -- accetterebbe il verbale di un antenato, cioe'
        # proprio il ripiego su una campagna precedente che il livello 2 non
        # deve poter fare. Il verbale lo produce la corsa, fuori dall'albero, e
        # il passo lo consuma da li'.
        if dichiarata != revisione_attesa:
            return [
                f"{percorso.name}: la campagna dichiara «{dichiarata[:12]}» "
                f"mentre la corsa misura «{revisione_attesa[:12]}». Il "
                "checkpoint consuma il verbale prodotto da se stesso e non "
                "ripiega su quello gia' versionato."
            ]
    else:
        # La domanda del **rilascio**: «questa campagna qualifica ancora
        # quest'albero?». La regola non e' l'uguaglianza con HEAD -- lo e'
        # stata, e rendeva il verbale impossibile da registrare, perche'
        # committarlo sposta HEAD. E' quella del congelamento, e vive in
        # `check_release_contract` insieme all'allowlist: discendenza piu' diff
        # ammessa.
        #
        # L'import e' locale perche' questo modulo scrive il verbale **durante**
        # lo smoke, dove il gate del contratto non serve e non deve poter
        # rompere la scrittura.
        sys.path.insert(0, str(Path(__file__).resolve().parent))
        from check_release_contract import evidenza_ancora_valida  # noqa: PLC0415

        motivi = evidenza_ancora_valida(dichiarata)
        if motivi:
            return [f"{percorso.name}: {motivo}" for motivo in motivi]

    dichiarati = documento.get("bersagli_dichiarati")
    if not isinstance(dichiarati, list) or not dichiarati:
        return [
            f"{percorso.name}: non dichiara quali bersagli esistessero. Senza, "
            "una corsa su un sottoinsieme sarebbe indistinguibile da una su "
            "tutti."
        ]

    fermati = documento.get("fermati_a_finding_noto")
    if not isinstance(fermati, list):
        return [f"{percorso.name}: non dichiara quali bersagli si siano fermati"]
    falliti = documento.get("falliti_su_finding_nuovo") or []
    if falliti:
        return [
            "la campagna non e' completa: "
            f"{', '.join(sorted(falliti))} sono falliti su un crash che nessuna "
            "voce riconosce"
        ]
    if fermati:
        return [
            "la campagna non e' completa: "
            f"{', '.join(sorted(fermati))} si sono fermati a un finding noto e "
            "non hanno esplorato il tempo restante"
        ]
    finiti = documento.get("hanno_finito") or []
    if not finiti:
        return [f"{percorso.name}: nessun bersaglio ha finito il proprio tempo"]
    mancanti = sorted(set(dichiarati) - set(finiti))
    if mancanti:
        # Ci si arriva con un sottoinsieme richiesto, o con un target in
        # quarantena: in nessuno dei due casi la campagna e' quella dichiarata.
        return [
            "la campagna non copre i bersagli dichiarati: "
            f"{', '.join(mancanti)} non hanno finito il proprio tempo"
        ]
    return []


VERBALE = ROOT / "assurance" / "evidence" / "fuzz-smoke-ultima.json"

#: Dove **questa** corsa scrive il proprio verbale.
#:
#: Per difetto l'albero dell'assurance, che e' dove il verbale vive come
#: evidenza citata dallo stato. Il checkpoint lo dirotta fuori dall'albero con
#: `PLENORA_VERBALE_CAMPAGNA`, e non e' una comodita': lo smoke e' un passo del
#: livello 2, il verbale e' un file **tracciato**, e scriverlo durante la corsa
#: rende rosso `albero_invariato`. Un checkpoint che modifica l'albero che sta
#: qualificando non qualifica niente -- e' la stessa ragione per cui
#: `fuzz-profondita.sh` non e' un passo del checkpoint, scritta nel suo commento
#: da molto prima che questa scattasse.
#:
#: Cambia il **momento** della registrazione, non il requisito: la campagna deve
#: restare completa e attribuibile alla revisione su cui e' girata, e
#: `registra-evidenza-s9.py` pubblica poi quel verbale nell'albero.
VARIABILE_VERBALE = "PLENORA_VERBALE_CAMPAGNA"


def percorso_del_verbale() -> Path:
    """Il file su cui questa corsa scrive, che non e' sempre quello versionato."""
    fuori = os.environ.get(VARIABILE_VERBALE)
    return Path(fuori) if fuori else VERBALE


def revisione_corrente() -> str | None:
    """Lo SHA di HEAD, o `None` se git non risponde."""
    try:
        esito = subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
    except OSError:
        return None
    return esito.stdout.strip() if esito.returncode == 0 else None


def voci_dei_fermati(voci: list[str]) -> dict[str, str]:
    """`bersaglio=id` in un dizionario; una coppia malformata e' un errore."""
    risultato: dict[str, str] = {}
    for coppia in voci:
        bersaglio, uguale, identita = coppia.partition("=")
        if not uguale or not bersaglio or not identita:
            raise ValueError(f"voce di un fermato malformata: «{coppia}»")
        risultato[bersaglio] = identita
    return risultato


def scrivi_verbale(
    secondi: int,
    finiti: list[str],
    fermati: list[str],
    falliti: list[str],
    dichiarati: list[str],
    voci: dict[str, str] | None = None,
) -> None:
    """Il verbale della corsa: su che cosa ha girato, e com'e' finita.

    Lo costruisce questo modulo e non lo shell: un JSON assemblato a colpi di
    espansione di array e' illeggibile, e soprattutto non si prova. Qui ha una
    firma, e le sue proprieta' hanno regressioni.

    # Perche' porta la revisione e i bersagli dichiarati

    Perche' altrimenti un verbale **completo** di ieri qualificherebbe l'albero
    di oggi. Una campagna vale per il codice su cui e' girata, ed e' la stessa
    ragione per cui le misure di profondita' portano l'impronta del perimetro.
    Qui la revisione basta: la qualifica pretende gia' un albero pulito allo
    SHA atteso, quindi due SHA uguali sono due alberi uguali.

    I bersagli dichiarati servono alla seconda meta' della stessa domanda. Lo
    smoke sa girare su un **sottoinsieme**, e una corsa su un target solo puo'
    finire senza fermate: «nessuno si e' fermato» sarebbe vero e direbbe
    pochissimo. Il verbale registra percio' cio' che `cargo fuzz list`
    dichiarava, e la qualifica pretende che i finiti siano tutti.
    """
    destinazione = percorso_del_verbale()
    destinazione.parent.mkdir(parents=True, exist_ok=True)
    destinazione.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "descrizione": (
                    "L'esito dell'ultima corsa di `scripts/fuzz-smoke.sh`, per "
                    "bersaglio. Un bersaglio fermato a un crash compatibile con "
                    "un finding noto non ha esplorato il tempo restante: lo "
                    "smoke esce 0 perche' lo sviluppo prosegua sugli altri, e "
                    "questo verbale tiene separata la campagna completa da "
                    "quella interrotta."
                ),
                "revisione": revisione_corrente(),
                "bersagli_dichiarati": sorted(dichiarati),
                "secondi_per_bersaglio": secondi,
                "hanno_finito": sorted(finiti),
                "fermati_a_finding_noto": sorted(fermati),
                # Quale voce ha fermato ciascuno: un arresto «noto» si legge
                # come tale, con il finding a cui somiglia, e non solo come un
                # numero di bersagli fermati.
                "voci_dei_fermati": dict(sorted((voci or {}).items())),
                "falliti_su_finding_nuovo": sorted(falliti),
            },
            indent=2,
            ensure_ascii=False,
        )
        + "\n",
        encoding="utf-8",
        newline="\n",
    )
    print(f"verbale della corsa in {_mostra(destinazione)}")


def main(argv: list[str] | None = None) -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument(
        "bersaglio", nargs="?", help="il fuzz target che ha girato"
    )
    argomenti.add_argument(
        "--uscita",
        type=Path,
        help="il file con l'output della corsa, stdout e stderr insieme",
    )
    argomenti.add_argument(
        "--conserva",
        type=Path,
        help=(
            "dove scrivere input e referto del crash: si conserva sempre, "
            "noto o nuovo, perche' «noto» e' una compatibilita' e non "
            "un'identita'"
        ),
    )
    argomenti.add_argument(
        "--scrivi-verbale",
        type=int,
        metavar="SECONDI",
        help="scrive il verbale della corsa; con --finiti e --fermati",
    )
    argomenti.add_argument("--finiti", nargs="*", default=[], help="chi ha finito")
    argomenti.add_argument(
        "--fermati", nargs="*", default=[], help="chi si e' fermato a un crash noto"
    )
    argomenti.add_argument(
        "--falliti", nargs="*", default=[], help="chi e' fallito su un crash nuovo"
    )
    argomenti.add_argument(
        "--voci",
        nargs="*",
        default=[],
        help="per ogni fermato, `bersaglio=id` della voce che l'ha fermato",
    )
    argomenti.add_argument(
        "--codice-uscita",
        type=int,
        help=(
            "il codice d'uscita della corsa: diverso da zero senza alcun "
            "marcatore d'errore nell'uscita vuol dire illeggibile, non «senza "
            "crash»"
        ),
    )
    argomenti.add_argument(
        "--voce-nota",
        type=Path,
        help="se il crash e' noto, scrive qui l'id della voce",
    )
    argomenti.add_argument(
        "--dichiarati",
        nargs="*",
        default=[],
        help="i bersagli che `cargo fuzz list` dichiara, cioe' il perimetro intero",
    )
    argomenti.add_argument(
        "--revisione-della-corsa",
        help=(
            "lo SHA che la corsa sta misurando: con questo il verbale deve "
            "dichiarare esattamente quella revisione, ed e' come il "
            "checkpoint pretende il proprio invece di uno precedente"
        ),
    )
    argomenti.add_argument(
        "--verifica-campagna",
        type=Path,
        help=(
            "il verbale di una corsa dello smoke: rossa se un bersaglio si e' "
            "fermato a un finding noto. E' la modalita' della qualificazione "
            "finale, dove «interrotta» non vale come «completata»"
        ),
    )
    opzioni = argomenti.parse_args(argv)

    if opzioni.scrivi_verbale is not None:
        try:
            voci = voci_dei_fermati(opzioni.voci)
        except ValueError as guasto:
            print(str(guasto), file=sys.stderr)
            return 2
        senza_voce = sorted(set(opzioni.fermati) - set(voci))
        if senza_voce:
            print(
                f"fermati senza la voce che li ha fermati: {', '.join(senza_voce)}",
                file=sys.stderr,
            )
            return 2
        scrivi_verbale(
            opzioni.scrivi_verbale,
            opzioni.finiti,
            opzioni.fermati,
            opzioni.falliti,
            opzioni.dichiarati,
            voci,
        )
        return 0

    if opzioni.verifica_campagna is not None:
        motivi = verifica_campagna(
            opzioni.verifica_campagna, opzioni.revisione_della_corsa
        )
        for motivo in motivi:
            print(motivo, file=sys.stderr)
        if motivi:
            return 1
        print("campagna fuzz completa: nessun bersaglio fermato a un finding noto")
        return 0

    if opzioni.bersaglio is None or opzioni.uscita is None:
        print(
            "servono il bersaglio e `--uscita`, oppure `--verifica-campagna`",
            file=sys.stderr,
        )
        return 2

    if not REGISTRO.exists():
        print(f"{REGISTRO}: registro assente", file=sys.stderr)
        return 2
    documento = json.loads(REGISTRO.read_text(encoding="utf-8"))
    motivi = registro_ben_formato(documento)
    if motivi:
        for motivo in motivi:
            print(f"registro dei finding noti: {motivo}", file=sys.stderr)
        return 2

    if not opzioni.uscita.is_file():
        print(f"{opzioni.uscita}: uscita della corsa assente", file=sys.stderr)
        return 2
    testo = opzioni.uscita.read_text(encoding="utf-8", errors="replace")
    esito = classifica(opzioni.bersaglio, testo, documento, opzioni.codice_uscita)

    # I tre stati hanno tre codici d'uscita, perche' chi chiama deve poterli
    # distinguere senza rileggere il testo: 0 nessun crash, 3 crash noto,
    # 1 crash nuovo. Un noto non e' un successo, ed e' il motivo per cui non
    # esce 0.
    if esito["stato"] == "senza-crash":
        print(f"{opzioni.bersaglio}: nessun crash nell'uscita")
        return 0
    if esito["stato"] == "illeggibile":
        if opzioni.conserva is not None:
            for scritto in conserva(opzioni.conserva, opzioni.bersaglio, esito, None):
                print(f"{opzioni.bersaglio}: conservato {scritto}")
        print(
            f"{opzioni.bersaglio}: crash ILLEGGIBILE -- la corsa e' fallita ma "
            "nessuna firma si legge (stack non simbolizzato o fuori forma, un "
            "frame senza file prima del primo utile, un'uscita diversa da zero "
            "senza marcatori). Non e' un finding noto.",
            file=sys.stderr,
        )
        return 1
    voce = (
        next(v for v in documento["finding"] if v["id"] == esito["id"])
        if esito["stato"] == "noto"
        else None
    )
    if opzioni.conserva is not None:
        for scritto in conserva(opzioni.conserva, opzioni.bersaglio, esito, voce):
            print(f"{opzioni.bersaglio}: conservato {scritto}")

    if esito["stato"] == "noto":
        if opzioni.voce_nota is not None:
            opzioni.voce_nota.write_text(esito["id"] + "\n", encoding="utf-8")
        print(
            f"{opzioni.bersaglio}: crash COMPATIBILE con il finding noto "
            f"«{esito['id']}» ({voce['dove_e_tracciato']}). Compatibile non "
            f"vuol dire identico: la firma non dimostra la stessa causa, e "
            f"l'input e' conservato per poterlo riesaminare. "
            f"{voce['non_promette']} Il bersaglio si e' fermato qui: il tempo "
            f"restante non e' stato esplorato."
        )
        return 3
    osservato = esito["osservato"]
    dove = osservato["modulo"]
    if osservato["funzione"]:
        dove = f"{osservato['funzione']} ({dove})"
    print(
        f"{opzioni.bersaglio}: finding NUOVO ({osservato['tipo']}) in {dove}"
        f":{osservato['riga']} -- {osservato['messaggio']}",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
