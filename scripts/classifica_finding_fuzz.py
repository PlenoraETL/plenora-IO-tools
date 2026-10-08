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
`ERROR: libFuzzer: out-of-memory (malloc(N))` e lo stack dell'allocazione.
La firma e' allora:

* il **tipo** `esaurimento-memoria`, distinto da `panico`: un panico e un
  esaurimento nello stesso modulo sono due finding;
* il **primo frame che non e' l'allocatore** -- `malloc`, i frame della
  libreria standard (percorsi sotto `/rustc/`), il runtime del sanitizer --
  ridotto a modulo, come per il panico, e a **funzione**, col suffisso
  `::h<hash>` e i parametri generici tolti;
* la forma del messaggio, `out-of-memory (malloc(N))`.

La corrispondenza resta congiunta: tipo, modulo, funzione e forma, per il
bersaglio dichiarato. Restano **fuori**, e quindi rossi:

* l'esaurimento misurato sull'RSS (`out-of-memory (used: ...)`), che arriva dal
  thread che sorveglia la memoria e non porta lo stack di un'allocazione: non
  c'e' una firma da leggere;
* uno stack non simbolizzato, cioe' indirizzi senza nomi: senza nomi non c'e'
  firma, e un crash senza firma e' **illeggibile**, mai «noto».

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

#: `ERROR: libFuzzer: out-of-memory (malloc(2315255472))`
ESAURIMENTO = re.compile(r"ERROR: libFuzzer: (out-of-memory \(malloc\(\d+\)\))")

#: Un frame simbolizzato di AddressSanitizer:
#: `#12 0x55d0c3b0f2a1 in <funzione> <percorso>:<riga>:<colonna>`.
FRAME = re.compile(
    r"^\s*#\d+\s+0x[0-9a-fA-F]+\s+in\s+(?P<funzione>.+?)\s+"
    r"(?P<percorso>/\S+?):(?P<riga>\d+)(?::\d+)?\s*$"
)

#: Le funzioni che sono l'allocatore e non chi alloca.
ALLOCATORE = (
    "malloc",
    "calloc",
    "realloc",
    "posix_memalign",
    "aligned_alloc",
    "__interceptor_",
    "__rust_",
    "__rdl_",
    "__rg_",
)

#: I segni di un crash che libFuzzer o il sanitizer hanno riportato. Se il
#: testo ne porta uno e nessuna firma si legge, il crash e' **illeggibile**.
SEGNI_DI_CRASH = (
    "ERROR: libFuzzer:",
    "ERROR: AddressSanitizer:",
    "panicked at",
)

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
    """Il nome di una funzione Rust senza hash e senza parametri generici.

    `parquet::parquet_thrift::read_thrift_vec::h0123456789abcdef` e
    `parquet::parquet_thrift::read_thrift_vec::<KeyValue, Slice>` diventano
    entrambi `parquet::parquet_thrift::read_thrift_vec`: il primo e' la
    demangling legacy, il secondo la v0, e i parametri dipendono dal tipo
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
    return re.sub(r"::+$", "", re.sub(r"::::+", "::", "".join(risultato)))


def _e_allocatore(funzione: str, percorso: str) -> bool:
    """Un frame dell'allocatore, della libreria standard o del sanitizer."""
    normalizzato = percorso.replace("\\", "/")
    if "/rustc/" in normalizzato or "compiler-rt" in normalizzato:
        return True
    return funzione.startswith(ALLOCATORE)


def _esaurimento_osservato(testo: str) -> dict[str, str] | None:
    """L'esaurimento di memoria da `malloc`, ridotto a firma; `None` se non
    c'e' o se lo stack non e' simbolizzato."""
    trovato = ESAURIMENTO.search(testo)
    if trovato is None:
        return None
    for riga in testo[trovato.end() :].splitlines():
        frame = FRAME.match(riga)
        if frame is None:
            continue
        funzione = frame.group("funzione")
        percorso = frame.group("percorso")
        if _e_allocatore(funzione, percorso):
            continue
        artefatto = ARTEFATTO.search(testo)
        messaggio = trovato.group(1)
        return {
            "tipo": "esaurimento-memoria",
            "modulo": modulo_normalizzato(percorso),
            "funzione": funzione_normalizzata(funzione),
            "riga": frame.group("riga"),
            "messaggio": messaggio,
            "forma_del_messaggio": forma_del_messaggio(messaggio),
            "artefatto": artefatto.group(1) if artefatto else "",
        }
    return None


def crash_osservato(testo: str) -> dict[str, str] | None:
    """Il primo panico nel testo, o l'esaurimento di memoria, ridotto a
    firma; `None` se non ce n'e' una leggibile."""
    trovato = PANICO.search(testo)
    if trovato is None:
        return _esaurimento_osservato(testo)
    # Il messaggio sta sulla riga **dopo** quella del panico: la riga del
    # panico finisce con i due punti, e prenderne la coda dava una stringa
    # vuota. Si salta percio' al primo a capo e si legge la riga seguente.
    coda = testo[trovato.end() :]
    a_capo = coda.find("\n")
    prima_riga = "" if a_capo == -1 else coda[a_capo + 1 :].split("\n", 1)[0]
    artefatto = ARTEFATTO.search(testo)
    return {
        "tipo": "panico",
        "modulo": modulo_normalizzato(trovato.group(1)),
        "funzione": "",
        "riga": trovato.group(2),
        "messaggio": prima_riga.strip(),
        "forma_del_messaggio": forma_del_messaggio(prima_riga),
        "artefatto": artefatto.group(1) if artefatto else "",
    }


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


def classifica(bersaglio: str, testo: str, documento: Any) -> dict[str, Any]:
    """`stato` fra `senza-crash`, `noto` e `nuovo`, con cio' che lo sostiene."""
    osservato = crash_osservato(testo)
    if osservato is None:
        # Un crash c'e' ma la firma non si legge: stack non simbolizzato,
        # esaurimento misurato sull'RSS, un errore del sanitizer. Non e' «senza
        # crash», e non e' «noto».
        if any(segno in testo for segno in SEGNI_DI_CRASH):
            return {"stato": "illeggibile"}
        return {"stato": "senza-crash"}

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
            "tipo": osservato["tipo"],
            "funzione": osservato["funzione"],
            "modulo": osservato["modulo"],
            "riga": osservato["riga"],
            "messaggio": osservato["messaggio"],
            "forma_del_messaggio": osservato["forma_del_messaggio"],
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
    esito = classifica(opzioni.bersaglio, testo, documento)

    # I tre stati hanno tre codici d'uscita, perche' chi chiama deve poterli
    # distinguere senza rileggere il testo: 0 nessun crash, 3 crash noto,
    # 1 crash nuovo. Un noto non e' un successo, ed e' il motivo per cui non
    # esce 0.
    if esito["stato"] == "senza-crash":
        print(f"{opzioni.bersaglio}: nessun crash nell'uscita")
        return 0
    if esito["stato"] == "illeggibile":
        print(
            f"{opzioni.bersaglio}: crash ILLEGGIBILE -- c'e' un crash ma "
            "nessuna firma si legge (stack non simbolizzato, esaurimento "
            "misurato sull'RSS, errore del sanitizer). Non e' un finding noto.",
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
