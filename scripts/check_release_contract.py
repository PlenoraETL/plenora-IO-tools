#!/usr/bin/env python3
"""Verifica il **contratto corrente**: invarianti che governano ancora il codice.

# Che cosa questo gate non fa piu'

La versione precedente validava la provenienza della release 1.0.0-rc.2
leggendo **quarantuno documenti Markdown** come database. Quei documenti erano
la cronaca di come si era arrivati a una release gia' emessa, e un gate che
verifica una cronaca non verifica il codice.

Il gate legge ora un solo registro strutturato,
`assurance/registries/release-contract-current.json`, in cui una voce esiste
**solo se un test o un gate corrente la verifica**. Un'affermazione senza
verifica corrente non viene importata come verita': diventa `release_blocking`.

# Due modalita', e il verde dell'una non e' il verde dell'altra

* senza argomenti — il registro e' ben formato **e le prove dei verificati
  sono state eseguite**. Non dice che la release sia autorizzabile, e lo
  stampa;
* `--release` — le condizioni congiunte dell'autorizzazione, fra cui l'assenza
  di voci `release_blocking`.

# Un registro vuoto non e' un contratto soddisfatto

La stesura precedente iterava `invarianti` e non aveva niente da dire su una
lista vuota: **cancellare una voce era il modo piu' rapido di ottenere un
verde**, e nessuna sonda lo vedeva. Il registro dichiara percio' uno schema, e
questo gate un **insieme obbligatorio** di identificatori. Chiudere un blocco
significa portarne lo `stato` a `verified`, mai eliminarne la voce; togliere un
nome da `INVARIANTI_OBBLIGATORI` resta possibile, ed e' il punto: e' un gesto
esplicito che compare in diff, non un'omissione che passa.

# Le condizioni di autorizzazione si eseguono, non si elencano

`autorizzazione_di_release.condizioni` era prosa — cinque frasi che nessuno
lanciava — mentre `--release` guardava soltanto due cose. Le condizioni sono
ora voci strutturate, ciascuna con la propria verifica (un gate da eseguire o
una funzione di questo modulo), e `--release` le esegue **tutte**. Anche il
loro insieme e' obbligatorio: togliere una condizione per ottenere un verde e'
rosso quanto lasciarla fallire.

# Una prova non e' un percorso che esiste

La stesura precedente controllava che il file citato da una prova fosse
presente. Un gate cancellato dal disco la faceva diventare rossa, ed era il
solo modo di accorgersene: uno strumento **presente e rotto**, un test
rinominato, un test sotto `#[ignore]`, un identificatore che non appartiene ad
alcun test — tutti passavano.

Le prove sono percio' tipizzate, e ogni tipo dice come si esegue:

* `test` — crate, configurazione, bersaglio del harness e identificatori
  esatti. Il test viene eseguito, deve comparire **una volta sola**
  nell'elenco del harness e passare;
* `gate` — comando strutturato, deduplicato fra invarianti e realmente
  eseguito: exit diverso da 0 significa invariante non verificato;
* `interna` — funzione di questo gate, eseguita in linea su un artefatto
  strutturato. Serve dove il comando sarebbe questo stesso gate;
* `esterna` — owner e artefatto. Lo stato non e' quello che la voce si
  attribuisce: e' **derivato dal contenuto dell'artefatto** dell'owner, e il
  campo `stato` del registro deve coincidere con esso. Senza evidenza — stato
  diverso da `passed` — un invariante non puo' risultare `verified`: e'
  bloccante.

Un elenco di test vuoto non e' una prova: il harness girerebbe e nessuna
identita' verrebbe cercata, cioe' un verde per assenza di domanda. Gli
identificatori sono percio' almeno uno, stringhe non vuote e distinti; e un
comando e' un argv, non una riga di shell dentro un solo argomento.

I bloccanti **non** si eseguono. Un bloccante puo' avere per definizione un
gate rosso, ed e' cio' che lo rende bloccante; cio' che deve avere e' `manca`,
la condizione che lo chiuderebbe.

E' la stessa separazione di ASSURANCE-N1, e per la stessa ragione: un verde che
significa due cose a seconda di chi lo legge e' la forma di falso verde che
questo repository ha incontrato piu' volte.

Il contratto del protocollo CLI resta verificato **nel merito**, non solo
nominato: `release/cli-protocol-v1.json` e' un artefatto strutturato, e la sua
validazione e' conservata qui.
"""

from __future__ import annotations

import argparse
import functools
import hashlib
import json
import math
import re
import subprocess
import sys
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))

# Lo stesso lettore che usa ASSURANCE-N1: due definizioni di «test
# eseguito» divergerebbero, e divergerebbero in silenzio.
from check_assurance_n1_prove import (  # noqa: E402
    BERSAGLI,
    BERSAGLIO_PREDEFINITO,
    CONFIGURAZIONI,
    esegui_harness,
)

# E lo stesso lettore del debito: «gruppo aperto» e' definito una volta sola,
# in `check_assurance_n1`. Ricontare qui le disposizioni aperte creerebbe la
# seconda definizione che il conteggio esiste per legare.
from check_assurance_n1 import (  # noqa: E402
    carica as carica_gruppi_n1,
    debito as debito_n1,
)

# Le altre fonti dello stato, per la stessa ragione: l'allowlist del docset e
# il censimento dei costruttori legacy sono definiti dove vivono, e qui si
# leggono invece di essere ricopiati.
from check_docset import (  # noqa: E402
    BASELINE_DOCSET,
    CANONICI,
    OPERATIVI,
)
from check_errori_redatti import (  # noqa: E402
    MIGRATI,
    verifica as censimento_errori,
)

# I nomi degli archivi si leggono da dove li scrive il costruttore. Ricavarli
# qui con una formula darebbe una seconda regola di denominazione, e le due
# divergerebbero al primo cambio di forma -- con il gate che continua a
# pretendere quattro nomi che nessuno produce piu'.
import distribuzione  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
REGISTRO = ROOT / "assurance" / "registries" / "release-contract-current.json"
CLI_PROTOCOL_V1 = ROOT / "release" / "cli-protocol-v1.json"

# Tre stati, e il terzo non e' una scorciatoia per il primo. `differita` dice
# che la capacita' **non e' richiesta da questa release** e **non e'
# verificata**: toglie il blocco senza produrre una verifica, ed e' per questo
# lo stato piu' pericoloso del registro. Le sonde che lo circondano esistono
# perche' la distanza fra «non richiesta qui» e «vera» e' una riga di JSON.
STATI = {"verified", "release_blocking", "differita"}
CAMPI = {"id", "superficie", "invariante", "prova", "stato"}

# Chi puo' essere differita, e nessun altro. Se lo stato fosse applicabile a
# qualunque voce, la via piu' breve al verde sarebbe differire il bloccante che
# non passa: l'elenco chiuso rende quella mossa un rosso invece che una scelta.
DIFFERIBILI = frozenset({"sistema.qualifica-cross-component"})

# Che cosa deve dire una voce differita. Non bastano una parola e un rinvio:
# servono chi ha deciso, che cosa la release **non promette** per effetto del
# rinvio, e che cosa lo farebbe tornare bloccante. Senza il non-promette un
# rinvio si legge come un dettaglio interno, mentre e' una capacita' in meno
# nelle mani di chi installa.
CAMPI_DELLA_DIFFERITA = frozenset(
    {"decisione", "non_promette", "condizione_di_ripristino"}
)

TIPI = {"test", "gate", "interna", "esterna"}
CAMPI_PER_TIPO = {
    "test": {"crate", "configurazione", "test"},
    "gate": {"comando"},
    "interna": {"funzione", "artefatto"},
    "esterna": {"owner", "artefatto", "stato"},
}

# Una prova esterna puo' rendere `verified` solo con questo stato: qualunque
# altro significa che l'evidenza non c'e', e un invariante senza evidenza e'
# bloccante, non vero.
STATO_ESTERNO_VALIDO = "passed"

SCHEMA_VERSIONE = 1

# --- l'insieme obbligatorio degli invarianti --------------------------------
#
# Un invariante che sparisce dal registro sparisce anche dalla tabella di
# `docs/RELEASE.md`: il documento risulta piu' verde senza che nulla sia
# cambiato nel codice. `struttura` non poteva accorgersene, perche' guarda una
# voce alla volta e su una lista vuota non ha voci da guardare.
#
# L'elenco e' chiuso e comprende anche i verificati: anche una garanzia che
# oggi passa, sparendo, smette di essere pretesa senza che nessuno lo decida.
INVARIANTI_OBBLIGATORI = frozenset(
    {
        "wire.error-v1.chiavi",
        "wire.error-v1.quartetto",
        "wire.cli-protocol-v1",
        "wire.catalog-producer",
        "errori.nessun-testo-runtime",
        "errori.static-non-promuovibile",
        "errori.tetto-messaggio",
        "budget.limiti-wkb",
        "budget.nessun-modello-legacy",
        "budget.permit-boundary",
        "difese.prevalidazione-decoder",
        "fallback.registro",
        "provenienza.fork-vendorizzati",
        "provenienza.pin",
        "identita.superficie-pubblica",
        "fuzz.quarantena",
        "copertura.rami-negativi",
        "copertura.variazione-fra-corse",
        # Il denominatore della soglia. Senza, bastava togliere
        # `--all-features` per far sparire dalla misura cinquecento righe
        # di produzione: non «scoperte», invisibili.
        "copertura.tier-gdb-nel-denominatore",
        "fuzz.reader-shapefile",
        "fuzz.semi-riproducibili",
        "fuzz.filegdb",
        # I due limiti dichiarati del target FileGDB. Stanno qui perche' un
        # invariante che si puo' **togliere** non e' un invariante: senza,
        # cancellare `fuzz.filegdb-confine-asan` dal registro avrebbe fatto
        # sparire in silenzio la sola affermazione che impedisce di leggere una
        # campagna verde come «GDAL non ha difetti di memoria».
        "fuzz.filegdb-confine-asan",
        "fuzz.fixture-filegdb",
        "fuzz.semi-filegdb",
        "wire.loss-report",
        "release.candidate-non-valida-per-head",
        "lotto.s10",
        "lotto.s11",
        "lotto.s12",
        "sistema.qualifica-cross-component",
        # I due rilievi lasciati dalla chiusura di `wire.loss-report`, e resi
        # bloccanti invece che raccomandati: un obbligo che vive solo nella
        # prosa della roadmap ricrea lo spazio non sorvegliato che quella
        # chiusura aveva appena trovato.
        "fuzz.profondita-riproducibile",
        "wire.clausole-confrontabili",
        "stato.fonti-legate",
    }
)

# --- le condizioni dell'autorizzazione --------------------------------------
CAMPI_CONDIZIONE = {"id", "descrizione", "verifica"}
TIPI_VERIFICA = {"gate", "interna"}

# Congiunte: nessuna implica le altre. L'insieme e' obbligatorio perche'
# altrimenti la via piu' breve al verde sarebbe cancellare la condizione che
# non passa, e il registro continuerebbe a dichiarare cinque condizioni nella
# propria prosa.
CONDIZIONI_OBBLIGATORIE = frozenset(
    {
        "nessun-bloccante",
        "debito-n1-a-zero",
        "decisione-scritta",
        "candidate-coerente",
        "qualifica-cross-component",
    }
)

STATO_CORRENTE = ROOT / "assurance" / "current-state.json"

# --- il congelamento, e le due revisioni ------------------------------------
#
# Il modello aveva un ciclo, e non era un blocco da chiudere: era una condizione
# che nessuna release poteva soddisfare.
#
# `condizione_candidate_coerente` pretendeva che il tag della release puntasse a
# **HEAD**. La decisione di rilascio, pero', e' `release_authorized: true` dentro
# un file versionato, e l'evidenza del livello 2 e' un altro file versionato:
# registrarle crea un commit, quel commit sposta HEAD, e il tag smette di
# puntarci. Soddisfare la prima condizione rompeva la seconda e viceversa. Il
# difetto non si sarebbe visto fino all'ultimo passo -- con il tag gia' creato --
# e la via d'uscita piu' comoda sarebbe stata spostare il tag sul commit
# dell'evidenza, cioe' far puntare la release a un albero che contiene
# l'attestazione di se stesso.
#
# Le revisioni sono percio' due, e fanno cose diverse.
#
# * `revisione_candidate` -- lo SHA **congelato**. Da li' sono costruiti binari,
#   SBOM e provenance, e li' punta il tag. Si scrive nello stato una volta, al
#   congelamento, ed e' un fatto: non si deriva da niente.
# * `revisione_assurance` -- il commit che registra evidenza e decisione. E'
#   **HEAD**, e non si scrive da nessuna parte. Un campo che dovesse contenere
#   lo SHA del commit che lo contiene non e' compilabile nel momento in cui lo
#   si scrive: l'unico modo di riempirlo sarebbe una cifra inventata, e una
#   cifra inventata accanto a una qualifica e' cio' che questo registro esiste
#   per impedire. Si deriva da git, a ogni corsa.
#
# Fra le due si puo' cambiare **solo** cio' che l'assurance produce. L'elenco e'
# un'allowlist -- default nega -- perche' una denylist dimentica la famiglia che
# nasce domani, e la dimenticherebbe in silenzio.
#
# Che cosa vi manca, e perche': `scripts/`, `.github/workflows/`, `crates/`,
# `Cargo.toml`, `Cargo.lock`, `vendor/`, `release/` e
# `assurance/registries/distribuzione-matrice.json`. Cioe' il codice, il lock,
# la macchina che costruisce, quella che verifica e il contratto di
# distribuzione: se cambiassero dopo il congelamento, l'albero qualificato e
# quello da cui gli artefatti sono stati costruiti sarebbero due, e il secondo
# non sarebbe stato misurato da nessuno.
#
# `release-contract-current.json` invece **e'** ammesso, ed e' la voce che
# merita una ragione: e' li' che gli invarianti passano a `verified` quando
# l'evidenza arriva, cioe' e' il prodotto dell'assurance. Ammetterlo non apre
# nulla, perche' le sue affermazioni non si autocertificano -- le riesegue
# `check_release_contract.py`, che sta in `scripts/` ed e' congelato.
ALLOWLIST_DOPO_IL_CONGELAMENTO = (
    "assurance/current-state.json",
    "assurance/evidence/",
    "assurance/registries/release-contract-current.json",
    "docs/RELEASE.md",
)

# --- che cosa lo stato copia dall'evidenza ---------------------------------
#
# `assurance/current-state.json` riporta i numeri di una corsa che vive in
# `assurance/evidence/`. Erano ricopiati a mano: una cifra sbagliata nella
# copia non era distinguibile da una misura diversa, e il documento generato
# avrebbe reso la copia con la stessa autorita' della fonte.
#
# Le coppie sono dichiarate qui invece che dedotte dai nomi: `quarantena` sta
# sotto `fuzz_smoke` nell'evidenza e sotto `fuzz` nello stato, e una
# corrispondenza indovinata sui nomi non lo saprebbe.
# --- ogni foglia dello stato e' classificata --------------------------------
#
# Il registro promette che «ogni numero» dello stato venga dalla propria fonte.
# Il validatore ne verificava tre famiglie — evidenza, ASSURANCE-N1, candidate —
# e le altre foglie stavano nel documento **senza che nulla le guardasse**:
# portare `componenti_a_zero` a 999 non produceva un solo errore, e la promessa
# restava scritta.
#
# Le foglie sono percio' classificate tutte, e in due modi soltanto: legata a
# una fonte che la riscrive, oppure dichiarata non derivabile con la ragione.
# Una foglia che non e' in nessuno dei due insiemi e' rossa — anche una
# aggiunta domani.
FOGLIE_LEGATE = frozenset(
    {
        # forma del documento
        "schema_version",
        "revisioni.baseline_documentale.sha",
        # l'evidenza della corsa misurata
        "revisioni.ultima_qualificata.sha",
        "ultima_misura.sha",
        "ultima_misura.evidenza",
        "ultima_misura.checkpoint.passi_eseguiti",
        "ultima_misura.checkpoint.passi_verdi",
        "ultima_misura.checkpoint.passi_omessi",
        "ultima_misura.checkpoint.passi_falliti",
        "ultima_misura.fuzz.replay_input",
        "ultima_misura.fuzz.replay_target",
        "ultima_misura.fuzz.replay_crash",
        "ultima_misura.fuzz.smoke_target_eseguiti",
        "ultima_misura.fuzz.smoke_target_totali",
        "ultima_misura.fuzz.smoke_finding",
        "ultima_misura.fuzz.quarantena",
        "ultima_misura.copertura.lcov_percentuale",
        "ultima_misura.copertura.lcov_righe_coperte",
        "ultima_misura.copertura.lcov_righe_strumentate",
        "ultima_misura.copertura.cargo_lines_percentuale",
        "ultima_misura.copertura.soglia",
        "ultima_misura.diagnostica_differenziale.baseline",
        "ultima_misura.diagnostica_differenziale.esito",
        # Le tre frasi. Erano classificate «prosa», cioe' fuori da ogni
        # confronto, mentre lo stato le **derivava** dall'evidenza: sostituirle
        # con testo arbitrario lasciava il gate verde, ed e' la forma di falso
        # verde piu' comoda -- una spiegazione che nessuno confronta con cio'
        # che spiega. Sono prosa, ma non sono di questo documento: sono
        # dell'evidenza, e qui vanno riportate identiche.
        "ultima_misura.diagnostica_differenziale.ragione",
        "ultima_misura.diagnostica_differenziale.che_cosa_sono_le_scoperte",
        "ultima_misura.copertura.misurata_con",
        # I quattro conteggi da cui l'esito differenziale e' calcolato. Stanno
        # qui perche' l'esito da solo non si puo' controllare: una percentuale
        # senza numeratore e denominatore e' una cifra, e questa corsa e' la
        # prima che ne produce di diversi da zero.
        "ultima_misura.diagnostica_differenziale.righe_cambiate_eseguibili",
        "ultima_misura.diagnostica_differenziale.coperte",
        "ultima_misura.diagnostica_differenziale.scoperte",
        "ultima_misura.diagnostica_differenziale.cambiate_non_eseguibili",
        # il registro di ASSURANCE-N1
        "aperto.assurance_n1.gruppi_totali",
        "aperto.assurance_n1.gruppi_aperti",
        # Cargo.toml e git
        "aperto.candidate_release.versione_workspace",
        "aperto.candidate_release.tag_previsto",
        "aperto.candidate_release.tag_creato",
        "aperto.candidate_release.tag_revisione",
        "aperto.candidate_release.tag_sulla_candidate",
        "aperto.candidate_release.assurance_entro_l_allowlist",
        # il registro del contratto corrente
        "aperto.assurance_n1.release_blocking",

        "aperto.loss_report.release_blocking",
        "aperto.loss_report.stato",
        "aperto.candidate_release.release_blocking",
        "aperto.lotti.s10",
        "aperto.lotti.s11",
        "aperto.lotti.s12",
        "aperto.lotti.qualifica_cross_component",
        "blocchi.fonte",
        # il censimento dei costruttori legacy, e git
        "chiuso.s9_errori_strutturati.stato",
        "chiuso.s9_errori_strutturati.censimento_costruttori_legacy",
        "chiuso.s9_errori_strutturati.componenti_a_zero",
        "chiuso.s9_errori_strutturati.qualificato_su",
        # la misura di profondita' del target shp_reader
        "chiuso.fuzz_reader_shapefile.stato",
        "chiuso.fuzz_reader_shapefile.release_blocking",
        "chiuso.fuzz_reader_shapefile.misura",
        "chiuso.fuzz_reader_shapefile.requisiti_di_profondita",
        # la misura di profondita' e il confine ASan del target FileGDB
        "chiuso.fuzz_filegdb.stato",
        "chiuso.fuzz_filegdb.release_blocking",
        "chiuso.fuzz_filegdb.misura",
        "chiuso.fuzz_filegdb.requisiti_di_profondita",
        "chiuso.fuzz_filegdb.confine_asan",
        "chiuso.fuzz_filegdb.contatori_di_copertura",
        "chiuso.fuzz_filegdb.file_sorgente_gdal_strumentati",
        "chiuso.fuzz_filegdb.simboli_asan_nella_libreria",
        # l'allowlist del docset
        "docset.markdown_canonici",
        "docset.markdown_operativi",
        "docset.verificato_da",
    }
)

# Le foglie senza fonte, e perche' non ne hanno una. Non e' un'esenzione
# generica: ogni voce dice che cosa e', e una prosa che diventasse un numero
# andrebbe spostata fra le legate.
FOGLIE_DICHIARATE = {
    "descrizione": "prosa che spiega a che cosa serve il file",
    "release_authorized": (
        "la decisione di rilascio. E' l'unica che nessun gate puo' derivare, ed "
        "e' il motivo per cui esiste come campo scritto"
    ),
    "revisioni.baseline_documentale.significato": "prosa",
    "revisioni.ultima_qualificata.significato": "prosa",
    "revisioni.ultima_qualificata.nota": "prosa",
    "ultima_misura.checkpoint.riconciliazione": "prosa: descrive il metodo, non un numero",
    "ultima_misura.copertura.nota": "prosa",
    "chiuso.fuzz_reader_shapefile.nota": "prosa",
    "chiuso.fuzz_filegdb.nota": "prosa",
    "aperto.loss_report.decisioni_ratificate": (
        "le cinque decisioni che la ratifica del contratto LossReport ha "
        "chiuso. Non sono misurate da niente -- sono l'elenco di cio' su cui "
        "si e' deciso, non di cio' che si e' misurato -- e a verificare che "
        "quanto deciso sia anche applicato e' la `prova` di `wire.loss-report`"
    ),
    "aperto.lotti.perimetri.s10": "prosa: il perimetro del lotto",
    "aperto.lotti.perimetri.s11": "prosa: il perimetro del lotto",
    "aperto.lotti.perimetri.s12": "prosa: il perimetro del lotto",
    "aperto.candidate_release.versione_manifesto": (
        "fatto del manifesto della candidate, che non e' piu' nel repository: "
        "resta in git, e non c'e' una fonte viva che lo riscriva"
    ),
    "aperto.candidate_release.revisione_candidate": (
        "lo SHA **congelato** della candidate. E' una decisione -- si congela --"
        " e non si deriva da nessuna fonte viva: git puo' dire che esiste, non "
        "che sia quello scelto"
    ),
    "aperto.candidate_release.artefatti": (
        "i quattro archivi congelati, con nome, digest, dimensione e revisione. "
        "Vengono dalla corsa che li ha costruiti, non da un file del "
        "repository: la loro coerenza interna la verifica "
        "`_artefatti_fissati`, e che i byte pubblicati siano **questi** lo "
        "verifica `check-deliverable.py --contro-la-candidate` dopo la "
        "pubblicazione"
    ),
    "aperto.candidate_release.release_action_allowed": (
        "fatto del manifesto della candidate; vedi `versione_manifesto`"
    ),
    "aperto.candidate_release.nota": "prosa",
    "blocchi.nota": "prosa",
}

# --- che cosa lo stato ripete del registro ---------------------------------
#
# `aperto.<voce>.release_blocking` e' vero se e solo se l'invariante
# corrispondente e' `release_blocking` nel registro. Erano due scritture
# indipendenti della stessa cosa.
BLOCCANTI_DELLO_STATO = {
    ("aperto", "assurance_n1", "release_blocking"): "copertura.rami-negativi",
    ("chiuso", "fuzz_reader_shapefile", "release_blocking"): "fuzz.reader-shapefile",
    ("chiuso", "fuzz_filegdb", "release_blocking"): "fuzz.filegdb",
    ("aperto", "loss_report", "release_blocking"): "wire.loss-report",
    (
        "aperto",
        "candidate_release",
        "release_blocking",
    ): "release.candidate-non-valida-per-head",
}

# Le stesse voci dette a parole: `percorso -> (invariante, {stato: parola})`.
# Il vocabolario non e' uniforme — un lotto e' «aperto» e una qualifica e'
# «aperta» — e indovinarlo dal nome sarebbe la scorciatoia che rende il legame
# falso su un caso solo.
#
# La parola e' indicizzata per **stato**, non per «blocca o no». Con due sole
# parole, `differita` avrebbe dovuto prendere in prestito quella di uno degli
# altri due stati: «chiusa» avrebbe detto che la qualifica c'e', «aperta» che e'
# ancora pretesa, e nessuna delle due e' vera. Uno stato senza una parola sua e'
# rosso, e lo e' anche uno stato aggiunto domani.
ETICHETTE_DELLO_STATO = {
    ("aperto", "lotti", "s10"): (
        "lotto.s10",
        {"release_blocking": "aperto", "verified": "chiuso"},
    ),
    ("aperto", "lotti", "s11"): (
        "lotto.s11",
        {"release_blocking": "aperto", "verified": "chiuso"},
    ),
    ("aperto", "lotti", "s12"): (
        "lotto.s12",
        {"release_blocking": "aperto", "verified": "chiuso"},
    ),
    ("aperto", "lotti", "qualifica_cross_component"): (
        "sistema.qualifica-cross-component",
        {
            "release_blocking": "aperta",
            "verified": "chiusa",
            "differita": "differita",
        },
    ),
    ("chiuso", "fuzz_reader_shapefile", "stato"): (
        "fuzz.reader-shapefile",
        {"release_blocking": "aperto", "verified": "chiuso"},
    ),
    ("chiuso", "fuzz_filegdb", "stato"): (
        "fuzz.filegdb",
        {"release_blocking": "aperto", "verified": "chiuso"},
    ),
    ("aperto", "loss_report", "stato"): (
        "wire.loss-report",
        {"release_blocking": "non_ratificato", "verified": "ratificato"},
    ),
}

CAMPI_DALL_EVIDENZA = (
    (("checkpoint", "passi_eseguiti"), ("riconciliazione", "eseguiti")),
    (("checkpoint", "passi_verdi"), ("riconciliazione", "verdi")),
    (("checkpoint", "passi_omessi"), ("riconciliazione", "omessi")),
    (("checkpoint", "passi_falliti"), ("riconciliazione", "falliti")),
    (("fuzz", "replay_input"), ("misure", "fuzz_replay", "input")),
    (("fuzz", "replay_target"), ("misure", "fuzz_replay", "target")),
    (("fuzz", "replay_crash"), ("misure", "fuzz_replay", "crash")),
    (("fuzz", "smoke_target_eseguiti"), ("misure", "fuzz_smoke", "target_eseguiti")),
    (("fuzz", "smoke_target_totali"), ("misure", "fuzz_smoke", "target_totali")),
    (("fuzz", "smoke_finding"), ("misure", "fuzz_smoke", "finding")),
    (("fuzz", "quarantena"), ("misure", "fuzz_smoke", "quarantena")),
    (("copertura", "lcov_percentuale"), ("misure", "copertura", "lcov_percentuale")),
    (("copertura", "lcov_righe_coperte"), ("misure", "copertura", "lcov_righe_coperte")),
    (
        ("copertura", "lcov_righe_strumentate"),
        ("misure", "copertura", "lcov_righe_strumentate"),
    ),
    (
        ("copertura", "cargo_lines_percentuale"),
        ("misure", "copertura", "cargo_lines_percentuale"),
    ),
    (("copertura", "soglia"), ("misure", "copertura", "soglia")),
    (
        ("diagnostica_differenziale", "baseline"),
        ("misure", "diagnostica_differenziale", "base"),
    ),
    (
        ("diagnostica_differenziale", "esito"),
        ("misure", "diagnostica_differenziale", "esito"),
    ),
    (
        ("diagnostica_differenziale", "righe_cambiate_eseguibili"),
        ("misure", "diagnostica_differenziale", "righe_cambiate_eseguibili"),
    ),
    (
        ("diagnostica_differenziale", "coperte"),
        ("misure", "diagnostica_differenziale", "coperte"),
    ),
    (
        ("diagnostica_differenziale", "scoperte"),
        ("misure", "diagnostica_differenziale", "scoperte"),
    ),
    (
        ("diagnostica_differenziale", "cambiate_non_eseguibili"),
        ("misure", "diagnostica_differenziale", "cambiate_non_eseguibili"),
    ),
    # Le frasi si confrontano come i numeri. Una spiegazione che sopravvive alla
    # corsa che spiegava e' peggio di un numero sbagliato: il numero lo si
    # ricontrolla, la frase la si crede.
    (
        ("diagnostica_differenziale", "ragione"),
        ("misure", "diagnostica_differenziale", "ragione"),
    ),
    (
        ("diagnostica_differenziale", "che_cosa_sono_le_scoperte"),
        ("misure", "diagnostica_differenziale", "che_cosa_sono_le_scoperte"),
    ),
    (
        ("copertura", "misurata_con"),
        ("misure", "copertura", "misurata_con"),
    ),
)


def _percorsi(valore: Any) -> list[str]:
    """Un artefatto puo' essere uno o piu' percorsi; qui diventa sempre lista."""
    if valore is None:
        return []
    if isinstance(valore, str):
        return [valore]
    return list(valore)


def completezza(documento: dict[str, Any]) -> list[str]:
    """Il registro nel suo insieme: schema, voci obbligatorie, condizioni.

    Separata da `struttura` perche' guarda il **registro intero**. Quella
    verifica una voce alla volta, e su una lista vuota non ha voci da
    verificare: era la ragione per cui un registro svuotato passava.
    """
    errori: list[str] = []

    if documento.get("schema_version") != SCHEMA_VERSIONE:
        errori.append(
            f"schema_version «{documento.get('schema_version')}»: attesa "
            f"{SCHEMA_VERSIONE}. Uno schema non dichiarato rende ogni altra "
            "verifica un'ipotesi sul formato."
        )

    invarianti = documento.get("invarianti")
    if not isinstance(invarianti, list) or not invarianti:
        errori.append(
            "`invarianti` assente o vuoto. Un registro vuoto non e' un "
            "contratto soddisfatto: e' un contratto che non dice niente, e "
            "non ha nulla che possa fallire."
        )
        return errori

    presenti = {voce.get("id") for voce in invarianti if isinstance(voce, dict)}
    for mancante in sorted(INVARIANTI_OBBLIGATORI - presenti):
        errori.append(
            f"{mancante}: invariante obbligatorio assente dal registro. Un "
            "blocco si chiude portandone lo `stato` a `verified`, non "
            "eliminandone la voce."
        )

    errori.extend(_condizioni_ben_formate(documento))
    return errori


def _condizioni_ben_formate(documento: dict[str, Any]) -> list[str]:
    """Le condizioni dell'autorizzazione sono strutturate ed eseguibili."""
    errori: list[str] = []
    autorizzazione = documento.get("autorizzazione_di_release")
    if not isinstance(autorizzazione, dict):
        errori.append(
            "`autorizzazione_di_release` assente. Senza, `--release` non "
            "avrebbe condizioni da verificare, e il conteggio dei bloccanti "
            "sarebbe l'unica."
        )
        return errori

    condizioni = autorizzazione.get("condizioni")
    if not isinstance(condizioni, list) or not condizioni:
        errori.append("`autorizzazione_di_release.condizioni` assente o vuoto")
        return errori

    viste: set[str] = set()
    for condizione in condizioni:
        if not isinstance(condizione, dict):
            errori.append(
                f"condizione «{condizione}» non strutturata. Le condizioni "
                "erano prosa, e la prosa non si esegue."
            )
            continue
        identita = condizione.get("id", "<senza id>")
        mancanti = CAMPI_CONDIZIONE - set(condizione)
        if mancanti:
            errori.append(f"condizione {identita}: campi mancanti {sorted(mancanti)}")
            continue
        if identita in viste:
            errori.append(f"condizione {identita}: voce duplicata")
        viste.add(identita)

        verifica = condizione["verifica"]
        tipo = verifica.get("tipo") if isinstance(verifica, dict) else None
        if tipo not in TIPI_VERIFICA:
            errori.append(
                f"condizione {identita}: tipo di verifica «{tipo}» non "
                f"ammesso; {sorted(TIPI_VERIFICA)}"
            )
            continue
        if tipo == "gate":
            errori.extend(
                f"condizione {identita}: {m}" for m in _argv_valido(verifica.get("comando"))
            )
        else:
            funzione = verifica.get("funzione")
            if not callable(globals().get(funzione)):
                errori.append(
                    f"condizione {identita}: la funzione «{funzione}» non "
                    "esiste in questo gate. Una condizione che nomina una "
                    "verifica inesistente e' una frase, non un controllo."
                )

    for mancante in sorted(CONDIZIONI_OBBLIGATORIE - viste):
        errori.append(
            f"condizione obbligatoria «{mancante}» assente. Le condizioni sono "
            "congiunte: toglierne una e' il modo piu' rapido di ottenere un "
            "verde parziale e chiamarlo verde."
        )
    return errori


def _argv_valido(comando: Any) -> list[str]:
    """Un comando e' un argv non vuoto di stringhe non vuote.

    `["scripts/gate.py --release"]` **non** lo e': e' una riga di shell scritta
    dentro un solo argomento, e `subprocess` la cercherebbe come nome di file.
    """
    if not isinstance(comando, list) or not comando:
        return ["`comando` assente o vuoto"]
    if not all(isinstance(argomento, str) and argomento for argomento in comando):
        return ["`comando` contiene un argomento che non e' una stringa non vuota"]
    if any(carattere.isspace() for carattere in comando[0]):
        return [
            f"`comando` comincia con «{comando[0]}»: e' una riga di shell "
            "scritta dentro un solo argomento, e `subprocess` la cerchera' "
            "come nome di file."
        ]
    return []


def struttura(documento: dict[str, Any]) -> list[str]:
    """Il registro e' ben formato. Non dice che le prove passino."""
    errori: list[str] = []
    visti: set[str] = set()

    for voce in documento.get("invarianti", []):
        identita = voce.get("id", "<senza id>")
        mancanti = CAMPI - set(voce)
        if mancanti:
            errori.append(f"{identita}: campi mancanti {sorted(mancanti)}")
            continue
        if identita in visti:
            errori.append(f"{identita}: voce duplicata")
        visti.add(identita)

        stato = voce["stato"]
        if stato not in STATI:
            errori.append(f"{identita}: stato «{stato}» non ammesso; {sorted(STATI)}")
            continue

        prova = voce["prova"]
        if stato == "release_blocking":
            # Un bloccante puo' avere un gate rosso, o nessuna prova ancora.
            # Cio' che deve avere e' la **condizione di chiusura**: senza,
            # nessuno sa che cosa servirebbe per toglierlo.
            if not voce.get("manca"):
                errori.append(
                    f"{identita}: `release_blocking` senza campo `manca`. Un "
                    "blocco senza la sua condizione di chiusura non si puo' "
                    "chiudere."
                )
            # `sintesi` e' la riga con cui il blocco compare nella tabella di
            # `docs/RELEASE.md`. Vive qui, e non nel documento, perche' un
            # blocco nasce e muore nel registro: scriverla nella prosa
            # creerebbe una seconda verita' libera di divergere.
            if not voce.get("sintesi"):
                errori.append(
                    f"{identita}: `release_blocking` senza campo `sintesi`. La "
                    "tabella dello stato di release ha bisogno di una riga, e "
                    "scriverla a mano nel documento sarebbe la seconda verita' "
                    "che quella tabella esiste per impedire."
                )
            continue

        if stato == "differita":
            errori.extend(f"{identita}: {m}" for m in _differita_ben_dichiarata(voce))
            continue

        if not prova:
            errori.append(
                f"{identita}: `verified` senza prova. Un invariante senza "
                "verifica corrente e' `release_blocking`, non una verita'."
            )
            continue
        if not voce.get("invariante"):
            errori.append(f"{identita}: `verified` senza invariante scritto")

        tipo = prova.get("tipo")
        if tipo not in TIPI:
            errori.append(f"{identita}: tipo di prova «{tipo}» non ammesso; {sorted(TIPI)}")
            continue
        senza = CAMPI_PER_TIPO[tipo] - set(prova)
        if senza:
            errori.append(f"{identita}: prova «{tipo}» senza {sorted(senza)}")
            continue

        if tipo == "test":
            if prova["configurazione"] not in CONFIGURAZIONI:
                errori.append(
                    f"{identita}: configurazione «{prova['configurazione']}» non ammessa"
                )
            bersaglio = prova.get("bersaglio", BERSAGLIO_PREDEFINITO)
            if bersaglio not in BERSAGLI:
                errori.append(
                    f"{identita}: bersaglio «{bersaglio}» non ammesso; "
                    f"scegliere fra {sorted(BERSAGLI)}"
                )
            errori.extend(f"{identita}: {m}" for m in _elenco_di_test_valido(prova["test"]))
        if tipo == "gate":
            for comando in _comandi(prova):
                errori.extend(f"{identita}: {m}" for m in _argv_valido(comando))
        if tipo == "esterna" and prova["stato"] != STATO_ESTERNO_VALIDO:
            errori.append(
                f"{identita}: prova esterna in stato «{prova['stato']}» ma "
                f"invariante `verified`. Senza evidenza — stato "
                f"«{STATO_ESTERNO_VALIDO}» — un invariante e' bloccante, non vero."
            )
        for relativo in _percorsi(prova.get("artefatto")):
            if not (ROOT / relativo).exists():
                errori.append(f"{identita}: artefatto «{relativo}» assente")

    errori.extend(_prove_esterne(documento))
    return errori


def _differita_ben_dichiarata(voce: dict[str, Any]) -> list[str]:
    """Le sonde che impediscono a «non richiesta» di diventare «verificata».

    Uno stato che toglie un blocco senza verificare niente e' una leva, e una
    leva senza sorveglianza si usa. Qui se ne sorvegliano tre lati.

    * **Chi**: solo gli identificatori di `DIFFERIBILI`. Altrimenti la via piu'
      corta al verde sarebbe differire il bloccante che non passa.
    * **Che cosa si smette di promettere**: il rinvio deve dirlo per esteso, e
      la riga finisce nella tabella generata di `docs/RELEASE.md`. Un rinvio che
      non nomina la promessa che ritira si legge come un dettaglio interno.
    * **Che non stia coprendo un'evidenza**: se l'artefatto dell'owner dicesse
      `passed`, la voce sarebbe verificata e non differita, e tenerla differita
      nasconderebbe una qualifica riuscita esattamente come il caso opposto ne
      inventerebbe una.

    L'ultimo lato e' quello che chiude il passaggio da «non richiesta per questa
    release» a «verificata». Non si attraversa cambiando una parola: `verified`
    continua a pretendere l'artefatto `passed` -- lo impone `struttura` sul tipo
    `esterna` -- e `differita` lo vieta. Fra i due stati non esiste un valore
    dell'artefatto che li renda entrambi ammissibili, quindi non esiste una
    scrittura che promuova la voce senza toccare l'evidenza.
    """
    identita = voce.get("id")
    errori: list[str] = []
    if identita not in DIFFERIBILI:
        errori.append(
            f"stato `differita` non ammesso; le sole voci differibili sono "
            f"{sorted(DIFFERIBILI)}. Differire e' togliere un blocco senza "
            "verificare niente: aperto a chiunque, sarebbe la via piu' breve "
            "al verde."
        )
    if not voce.get("sintesi"):
        errori.append(
            "`differita` senza campo `sintesi`. La tabella delle capacita' "
            "differite ha bisogno di una riga, e una capacita' rinviata che non "
            "compare da nessuna parte e' una capacita' rinviata in silenzio."
        )
    dichiarazione = voce.get("differita")
    if not isinstance(dichiarazione, dict):
        errori.append(
            "`differita` senza il blocco `differita`. Servono la decisione, "
            "cio' che la release non promette, e la condizione di ripristino."
        )
        return errori
    for campo in sorted(CAMPI_DELLA_DIFFERITA - set(dichiarazione)):
        errori.append(f"blocco `differita` senza `{campo}`")
    for campo in sorted(CAMPI_DELLA_DIFFERITA & set(dichiarazione)):
        valore = dichiarazione[campo]
        if not isinstance(valore, str) or not valore.strip():
            errori.append(
                f"blocco `differita`: `{campo}` non e' prosa non vuota. Un "
                "campo vuoto soddisfa la forma e non dice niente."
            )
    prova = voce.get("prova")
    if not isinstance(prova, dict):
        errori.append(
            "`differita` senza prova. Il rinvio deve dire **da dove** "
            "arriverebbe l'evidenza: senza, non c'e' nulla da ripristinare e "
            "la condizione di ripristino non e' verificabile da nessuno."
        )
        return errori
    if prova.get("tipo") == "esterna":
        for relativo in _percorsi(prova.get("artefatto")):
            derivato, _ = stato_esterno_osservato(relativo)
            if derivato == STATO_ESTERNO_VALIDO:
                errori.append(
                    f"«{relativo}» dice «{STATO_ESTERNO_VALIDO}»: l'evidenza "
                    "esiste, e allora la voce e' `verified`, non `differita`. "
                    "Restare differita nasconderebbe una qualifica riuscita."
                )
    return errori


def _elenco_di_test_valido(elenco: Any) -> list[str]:
    """Una prova `test` nomina almeno un test, e ciascuno una volta sola.

    `"test": []` passava: il harness girava, nessuna identita' veniva cercata,
    e l'invariante risultava verificato per **assenza di domanda**. E' la stessa
    famiglia dell'elenco vuoto del harness, dal lato del registro invece che da
    quello dell'uscita.
    """
    if not isinstance(elenco, list) or not elenco:
        return [
            "prova `test` con elenco vuoto. Il harness girerebbe e nessuna "
            "identita' verrebbe cercata: un verde per assenza di domanda."
        ]
    if not all(isinstance(nome, str) and nome for nome in elenco):
        return ["prova `test` con un identificatore che non e' una stringa non vuota"]
    ripetuti = sorted({nome for nome in elenco if elenco.count(nome) > 1})
    if ripetuti:
        return [
            f"prova `test` con identificatori ripetuti {ripetuti}. Nominare due "
            "volte lo stesso test non lo esegue due volte: gonfia l'elenco e "
            "nient'altro."
        ]
    return []


def _prove_esterne(documento: dict[str, Any]) -> list[str]:
    """Lo stato di una prova esterna e' quello dell'**artefatto**.

    Il campo `stato` restava una dichiarazione: nessuno lo confrontava con cio'
    che l'artefatto dice, e uno stato che nessuno confronta e' lo stato che
    prima o poi si scrive da solo. Scrivere `passed` accanto a un artefatto che
    dice `not_run` sarebbe bastato a rendere `verified` un invariante di
    proprieta' altrui.

    Il confronto vale anche per i **bloccanti**. Li' un `passed` autocertificato
    non produce un verde, ma resta una divergenza fra due fonti, e la seconda
    lettura la troverebbe come ha trovato questa.
    """
    errori: list[str] = []
    for voce in documento.get("invarianti", []):
        prova = voce.get("prova")
        if not isinstance(prova, dict) or prova.get("tipo") != "esterna":
            continue
        percorsi = _percorsi(prova.get("artefatto"))
        if not percorsi:
            errori.append(f"{voce.get('id')}: prova esterna senza artefatto da leggere")
            continue
        for relativo in percorsi:
            derivato, _ = stato_esterno_osservato(relativo)
            if prova.get("stato") != derivato:
                errori.append(
                    f"{voce.get('id')}: la prova dichiara lo stato "
                    f"«{prova.get('stato')}», «{relativo}» ne descrive uno "
                    f"«{derivato}». Lo stato di una qualifica esterna si legge "
                    "dall'artefatto: dichiararlo qui sarebbe autocertificarlo."
                )
    return errori


def _comandi(prova: dict[str, Any]) -> list[list[str]]:
    return [prova["comando"], *prova.get("comandi_aggiuntivi", [])]


def esegui(documento: dict[str, Any]) -> list[str]:
    """Esegue le prove degli invarianti `verified`.

    I bloccanti non si eseguono: possono avere un gate rosso per definizione,
    ed e' cio' che li rende bloccanti. Eseguirli renderebbe il gate rosso su
    una condizione gia' dichiarata, e un rosso che si ripete smette di essere
    letto.
    """
    errori: list[str] = []
    verificati = [v for v in documento.get("invarianti", []) if v.get("stato") == "verified"]

    # --- gate: deduplicati, perche' ripetere una misura non la rende piu' vera
    visti: dict[tuple[str, ...], str] = {}
    for voce in verificati:
        prova = voce["prova"]
        if prova.get("tipo") != "gate":
            continue
        for comando in _comandi(prova):
            chiave = tuple(comando)
            if chiave in visti:
                continue
            visti[chiave] = voce["id"]
            esito = subprocess.run(comando, cwd=ROOT, capture_output=True, text=True, check=False)
            if esito.returncode != 0:
                errori.append(
                    f"{voce['id']}: la prova «{' '.join(comando)}» esce con "
                    f"{esito.returncode}. Un invariante la cui verifica fallisce "
                    "non e' verificato."
                )

    # --- test: eseguiti una volta per coppia, l'identita' deve comparire
    per_coppia: dict[tuple[str, str, str], list[dict[str, Any]]] = {}
    for voce in verificati:
        prova = voce["prova"]
        if prova.get("tipo") != "test":
            continue
        chiave = (
            prova["crate"],
            prova["configurazione"],
            prova.get("bersaglio", BERSAGLIO_PREDEFINITO),
        )
        per_coppia.setdefault(chiave, []).append(voce)

    for (crate, configurazione, bersaglio), voci in per_coppia.items():
        eseguiti, trovati = esegui_harness(crate, configurazione, bersaglio)
        errori.extend(trovati)
        for voce in voci:
            for identita in voce["prova"]["test"]:
                risultato = eseguiti.get(identita)
                if risultato is None:
                    errori.append(
                        f"{voce['id']}: «{identita}» non compare fra i test "
                        f"eseguiti di {crate} ({configurazione}, {bersaglio}). Un simbolo che "
                        "esiste ma non viene eseguito non verifica niente."
                    )
                elif risultato == "ignored":
                    errori.append(f"{voce['id']}: «{identita}» e' marcato `#[ignore]`")
                elif risultato != "ok":
                    errori.append(f"{voce['id']}: «{identita}» non passa («{risultato}»)")

    # --- interna: la funzione di questo gate, in linea
    for voce in verificati:
        prova = voce["prova"]
        if prova.get("tipo") != "interna":
            continue
        funzione = globals().get(prova["funzione"])
        if funzione is None:
            errori.append(f"{voce['id']}: la funzione «{prova['funzione']}» non esiste")
            continue
        documento_artefatto = json.loads((ROOT / prova["artefatto"]).read_text(encoding="utf-8"))
        errori.extend(f"{voce['id']}: {m}" for m in funzione(documento_artefatto))

    return errori


def debito(documento: dict[str, Any]) -> list[dict[str, Any]]:
    return [v for v in documento.get("invarianti", []) if v.get("stato") == "release_blocking"]


# --- le condizioni dell'autorizzazione, eseguite ----------------------------
#
# Ogni funzione restituisce i **motivi per cui la condizione non e'
# soddisfatta**: lista vuota significa soddisfatta. Nessuna di esse legge il
# campo con cui la condizione si dichiara: le leve stanno nelle fonti.


def _stato_corrente() -> tuple[dict[str, Any] | None, list[str]]:
    if not STATO_CORRENTE.exists():
        return None, [
            f"{STATO_CORRENTE.relative_to(ROOT).as_posix()}: fonte strutturata "
            "dello stato assente"
        ]
    return json.loads(STATO_CORRENTE.read_text(encoding="utf-8")), []


def _git(*argomenti: str) -> str | None:
    """L'uscita di un comando git, o `None` se non ha acquisito."""
    esito = subprocess.run(
        ["git", *argomenti], cwd=ROOT, capture_output=True, text=True, check=False
    )
    if esito.returncode != 0:
        return None
    return esito.stdout.strip()


# L'esito atteso da un'evidenza di livello 2. Il confronto e' **esatto**: una
# sottostringa accetterebbe «S9 checkpoint level 2 passed, ma con riserva», e un
# testo che contiene la frase non e' la frase.
ESITO_LIVELLO_2 = "S9 checkpoint level 2 passed"


@functools.lru_cache(maxsize=None)
def _revisione_risolta(revisione: str) -> str | None:
    return _git("rev-parse", "--verify", revisione + "^{commit}") or None


def revisione_risolta(revisione: Any) -> str | None:
    """Lo SHA completo del commit designato, o `None` se non ne designa uno.

    Il confronto fra revisioni non si fa con `startswith` ne' con
    l'appartenenza a un nome di file. La stringa vuota e' prefisso di
    qualunque cosa e sottostringa di qualunque cosa: `ultima_misura.sha = ""`
    passava ogni controllo, e cosi' `revisione_manifesto = ""` accanto a
    `qualifica_head: true`.

    Qui ogni revisione viene **risolta da git**, e il confronto e' fra SHA
    completi. Una revisione vuota, troppo corta, ambigua o assente dalla storia
    non risolve, e non c'e' un prefisso che possa cavarsela.

    Il risultato e' memoizzato: un gate non muove HEAD mentre gira, e ogni
    altra revisione che risolve e' un commit, cioe' immutabile. Senza, le sonde
    dell'evidenza risolvevano la stessa revisione una volta per sonda -- una
    settantina di `git rev-parse` che su un repository montato da fuori si
    sentono.
    """
    if not isinstance(revisione, str) or not revisione:
        return None
    return _revisione_risolta(revisione)


def _git_riesce(*argomenti: str) -> bool:
    """`True` se il comando esce con 0.

    Serve dove l'esito **e'** l'uscita e non il testo: `merge-base
    --is-ancestor` non stampa niente, e `_git` restituirebbe la stringa vuota
    sia sul successo sia sul fallimento.
    """
    return (
        subprocess.run(
            ["git", *argomenti], cwd=ROOT, capture_output=True, check=False
        ).returncode
        == 0
    )


def versione_workspace() -> str | None:
    """La versione di `[workspace.package]`, letta da `Cargo.toml`."""
    dentro = False
    for riga in (ROOT / "Cargo.toml").read_text(encoding="utf-8").splitlines():
        nuda = riga.strip()
        if nuda.startswith("["):
            dentro = nuda == "[workspace.package]"
            continue
        if dentro:
            trovato = re.match(r'version\s*=\s*"([^"]+)"', nuda)
            if trovato:
                return trovato.group(1)
    return None


def stato_esterno_osservato(relativo: str) -> tuple[str, list[str]]:
    """`(stato, motivi)` di una qualifica esterna, **derivati dal contenuto**.

    Non si legge il campo con cui la voce si dichiara: un `passed` scritto
    accanto a un artefatto che dice `not_run` e' autocertificazione, ed e'
    esattamente la forma di falso verde che il tipo `esterna` esiste per
    escludere.
    """
    percorso = ROOT / relativo
    if not percorso.exists():
        return "assente", [f"{relativo}: artefatto della prova esterna assente"]
    try:
        documento = json.loads(percorso.read_text(encoding="utf-8"))
    except json.JSONDecodeError as errore:
        return "illeggibile", [f"{relativo}: non e' JSON leggibile ({errore})"]

    evidenza = documento.get("evidence")
    dichiarato = evidenza.get("status") if isinstance(evidenza, dict) else None
    if not isinstance(dichiarato, str):
        return "non_derivabile", [
            f"{relativo}: nessun `evidence.status` da cui derivare lo stato. "
            "Senza, l'unica fonte sarebbe il campo che la voce scrive su se "
            "stessa."
        ]

    motivi: list[str] = []
    if documento.get("status") != "satisfied":
        motivi.append(
            f"{relativo}: `status` e' «{documento.get('status')}», non «satisfied»"
        )
    if dichiarato != STATO_ESTERNO_VALIDO:
        motivi.append(f"{relativo}: `evidence.status` e' «{dichiarato}»")
    aperti = documento.get("open_blockers") or []
    if aperti:
        motivi.append(
            f"{relativo}: l'owner dichiara {len(aperti)} blocchi ancora aperti"
        )
    senza_revisione = [
        componente.get("name")
        for componente in documento.get("components", [])
        if not componente.get("revision")
    ]
    if senza_revisione:
        motivi.append(
            f"{relativo}: revisione non fissata per {senza_revisione}. Una "
            "catena qualificata senza le revisioni non dice su che cosa e' "
            "girata."
        )

    if motivi:
        return (
            dichiarato if dichiarato != STATO_ESTERNO_VALIDO else "non_superata"
        ), motivi
    return STATO_ESTERNO_VALIDO, []


def _ammesso_dopo_il_congelamento(percorso: str) -> bool:
    """Un percorso della diff e' fra quelli che l'assurance puo' produrre.

    Le voci che finiscono con `/` sono prefissi di **directory**, e il confronto
    tiene la barra: senza, `assurance/evidence-falsa/x.json` sarebbe passato per
    somiglianza del nome. E' il modo in cui un'allowlist per prefisso si allarga
    senza che nessuno lo decida.
    """
    for voce in ALLOWLIST_DOPO_IL_CONGELAMENTO:
        if voce.endswith("/"):
            if percorso.startswith(voce):
                return True
        elif percorso == voce:
            return True
    return False


@functools.lru_cache(maxsize=None)
def _discende_da(antenato: str, discendente: str) -> bool:
    """L'ascendenza fra due revisioni, chiesta una volta per coppia.

    Come la diff qui sotto: dipende dall'albero, non dal documento, e la stessa
    domanda serve alla condizione di release e al legame dello stato.
    """
    return _git_riesce("merge-base", "--is-ancestor", antenato, discendente)


@functools.lru_cache(maxsize=None)
def _uscita_della_diff(congelata: str, fino_a: str) -> str | None:
    """La diff fra due revisioni, misurata una volta per processo.

    Dipende dall'albero e non dal documento, ed e' cara: fra una candidate e
    l'assurance ci possono essere centinaia di file, e la stessa diff serve alla
    condizione di release **e** al legame dello stato alle proprie fonti.
    Rifarla per ogni campo che ne deriva ripeterebbe lo stesso confronto senza
    cambiarne l'esito -- che e' la ragione per cui anche `_censimento` e
    `_revisione_risolta` sono memorizzate.

    Le sonde che sostituiscono `_git` devono svuotarla: una risposta finta
    memorizzata sopravviverebbe al `mock.patch` che l'ha prodotta.
    """
    return _git("diff", "--name-only", f"{congelata}..{fino_a}")


def cambiamenti_dopo_il_congelamento(
    congelata: str, fino_a: str = "HEAD"
) -> tuple[list[str], list[str]]:
    """`(percorsi fuori allowlist, errori)` fra la candidate e l'assurance.

    Un `git diff` che non acquisisce non e' una diff vuota. Restituire una lista
    vuota di percorsi fuori allowlist si leggerebbe come «niente e' cambiato», e
    sarebbe un verde per assenza di domanda proprio sul confronto che regge
    tutto il congelamento: gli errori tornano separati dai percorsi perche' i
    due casi non si somigliano.
    """
    uscita = _uscita_della_diff(congelata, fino_a)
    if uscita is None:
        return [], [
            f"git non produce la diff fra «{congelata[:7]}» e «{fino_a}»: senza "
            "quel confronto non si sa che cosa sia cambiato dopo il "
            "congelamento, e non saperlo non e' averlo verificato."
        ]
    percorsi = [riga.strip() for riga in uscita.splitlines() if riga.strip()]
    return [p for p in percorsi if not _ammesso_dopo_il_congelamento(p)], []


def artefatti_attesi(versione: str) -> list[str]:
    """I nomi degli archivi del perimetro, derivati dalla matrice.

    Il perimetro viene da `perimetro.distribuite`, non dai job che esistono: un
    artefatto in meno perche' qualcuno ha tolto un job e' un buco, uno in meno
    perche' la piattaforma e' fuori scope e' una decisione, e nel conteggio si
    somigliano.
    """
    matrice = json.loads(
        (ROOT / "assurance" / "registries" / "distribuzione-matrice.json").read_text(
            encoding="utf-8"
        )
    )
    piattaforme = matrice["perimetro"]["distribuite"]
    profili = [p["id"] for p in matrice["profili"]]
    return sorted(
        f"{distribuzione.nome_archivio(versione, piattaforma, profilo)}"
        f".{distribuzione.contenitore(piattaforma)}"
        for piattaforma in piattaforme
        for profilo in profili
    )


def _tag_sulla_candidate(
    candidate: dict[str, Any],
    congelata: str,
    versione: str,
    risolvi=None,
) -> list[str]:
    """Il tag della release punta alla **candidate**, non a HEAD.

    E' la meta' della correzione che si vede da fuori. Prima si pretendeva HEAD,
    e HEAD dopo l'assurance e' il commit che porta l'evidenza: un tag li' sopra
    qualificherebbe un albero diverso da quello da cui gli artefatti sono usciti.
    """
    risolvi = risolvi or revisione_risolta
    atteso = f"v{versione}"
    puntato = risolvi(atteso)
    if puntato is None:
        return [f"il tag «{atteso}» non esiste"]
    if puntato != congelata:
        return [
            f"il tag «{atteso}» punta a «{puntato[:7]}», non alla revisione "
            f"congelata «{congelata[:7]}». Il tag nomina cio' da cui si e' "
            "costruito, non cio' che lo attesta."
        ]
    return []


def _artefatti_fissati(
    candidate: dict[str, Any], congelata: str, versione: str
) -> list[str]:
    """I quattro archivi congelati: nomi del perimetro, digest, revisione.

    Fissare i digest e' cio' che rende verificabile la promessa piu' difficile
    della release -- che i byte pubblicati siano **quelli** misurati, non una
    ricostruzione equivalente. Due costruzioni della stessa revisione possono
    differire di un byte, e checksum e provenance del secondo archivio sarebbero
    coerenti fra loro: nessun gate del canale se ne accorgerebbe. Il confronto
    con il digest congelato e' l'unico posto in cui la differenza diventa
    visibile invece che opinabile.
    """
    fissati = candidate.get("artefatti")
    attesi = artefatti_attesi(versione)
    if not isinstance(fissati, list) or not fissati:
        return [
            "la candidate non fissa nessun artefatto. Un elenco vuoto non e' "
            f"una qualifica: sono attesi {len(attesi)} archivi, con il proprio "
            "digest e la revisione da cui escono."
        ]

    errori: list[str] = []
    per_nome: dict[str, dict[str, Any]] = {}
    for voce in fissati:
        if not isinstance(voce, dict) or not isinstance(voce.get("nome"), str):
            errori.append(f"artefatto senza nome leggibile: {voce!r}")
            continue
        if voce["nome"] in per_nome:
            errori.append(f"artefatto «{voce['nome']}» dichiarato due volte")
        per_nome[voce["nome"]] = voce

    for nome in attesi:
        if nome not in per_nome:
            errori.append(
                f"«{nome}» non e' fra gli artefatti congelati. Il perimetro "
                "viene dalla matrice, e un archivio che manca e' un artefatto "
                "che nessuno ha qualificato."
            )
    for nome in sorted(set(per_nome) - set(attesi)):
        errori.append(
            f"«{nome}» non e' nel perimetro della matrice: un artefatto in piu' "
            "non e' un artefatto qualificato in piu'."
        )

    for nome in sorted(set(per_nome) & set(attesi)):
        voce = per_nome[nome]
        digesto = voce.get("sha256")
        if not isinstance(digesto, str) or not IMPRONTA.match(digesto):
            errori.append(f"«{nome}»: `sha256` non e' un digest sha256")
        dimensione = voce.get("dimensione")
        if not isinstance(dimensione, int) or isinstance(dimensione, bool) or dimensione <= 0:
            errori.append(f"«{nome}»: `dimensione` non e' un intero positivo")
        if voce.get("revisione") != congelata:
            errori.append(
                f"«{nome}»: `revisione` vale «{str(voce.get('revisione'))[:7]}», "
                f"la candidate e' congelata su «{congelata[:7]}». Un artefatto "
                "costruito da un'altra revisione non qualifica questa."
            )

    digesti = [
        voce.get("sha256")
        for nome, voce in per_nome.items()
        if nome in attesi and isinstance(voce.get("sha256"), str)
    ]
    ripetuti = sorted({d for d in digesti if digesti.count(d) > 1})
    if ripetuti:
        errori.append(
            f"digest ripetuto fra artefatti diversi: {[d[:12] for d in ripetuti]}. "
            "Due archivi con gli stessi byte sono lo stesso archivio con due "
            "nomi, e uno dei due non e' stato costruito."
        )
    return errori


def condizione_nessun_bloccante(documento: dict[str, Any]) -> list[str]:
    bloccanti = debito(documento)
    if not bloccanti:
        return []
    motivi = [f"{voce['id']}: {voce['manca']}" for voce in bloccanti]
    motivi.append(
        f"{len(bloccanti)} invarianti su {len(documento['invarianti'])} restano bloccanti"
    )
    return motivi


def condizione_decisione_scritta(documento: dict[str, Any]) -> list[str]:
    """L'unica condizione che nessun gate puo' derivare.

    `release_authorized` e' una decisione scritta, non l'esito automatico di
    caselle verdi: qui si verifica che sia stata presa, non la si calcola.
    """
    stato, errori = _stato_corrente()
    if errori:
        return errori
    if stato.get("release_authorized") is not True:
        return [
            "`release_authorized` non e' true in assurance/current-state.json: "
            "e' una decisione scritta, e non e' stata presa"
        ]
    return []


def condizione_candidate_coerente(documento: dict[str, Any]) -> list[str]:
    """La candidate congelata, il tag che la nomina, e cio' che le e' successo dopo.

    Versione, revisione congelata, tag e artefatti non si leggono dallo stato
    come dichiarazioni: si confrontano con `Cargo.toml`, con git e con la
    matrice di distribuzione. Uno stato che dichiarasse la coerenza senza averla
    renderebbe la condizione una copia di se stessa.

    Le cinque cose che verifica, e perche' sono cinque e non una:

    1. la versione del manifesto e' quella del workspace;
    2. il tag `v<versione>` punta alla **revisione congelata**, non a HEAD;
    3. i quattro artefatti del perimetro sono fissati con digest e revisione, e
       la revisione e' quella congelata;
    4. fra la revisione congelata e HEAD e' cambiato **solo** cio' che
       l'assurance produce;
    5. `release_action.allowed` e' consentita.

    La quarta e' la correzione. Prima si pretendeva `tag == HEAD`, e siccome
    registrare evidenza e decisione sposta HEAD, quella pretesa e la condizione
    `decisione-scritta` non potevano essere vere insieme. Ora HEAD puo' andare
    avanti rispetto alla candidate -- **deve** andare avanti, perche' l'evidenza
    va scritta da qualche parte -- purche' porti solo evidenza, stato e
    documentazione generata. Ogni altra cosa che cambi dopo il congelamento
    rimette in discussione gli artefatti, che sono gia' stati costruiti.
    """
    stato, errori = _stato_corrente()
    if errori:
        return errori
    candidate = stato.get("aperto", {}).get("candidate_release")
    if not isinstance(candidate, dict):
        return ["assurance/current-state.json: `aperto.candidate_release` assente"]

    motivi: list[str] = []
    versione = versione_workspace()
    if versione is None:
        motivi.append("Cargo.toml: versione di `[workspace.package]` non leggibile")
    elif candidate.get("versione_manifesto") != versione:
        motivi.append(
            f"la candidate dichiara la versione «{candidate.get('versione_manifesto')}», "
            f"il workspace e' a «{versione}»"
        )

    dichiarata = candidate.get("revisione_candidate")
    congelata = revisione_risolta(dichiarata)
    if congelata is None:
        motivi.append(
            f"la candidate e' congelata su «{dichiarata}», che git non risolve "
            "a un commit: quel manifesto non qualifica alcuna revisione"
        )
        return motivi + _coda_della_candidate(candidate)

    if versione is not None:
        motivi.extend(_tag_sulla_candidate(candidate, congelata, versione))
        motivi.extend(_artefatti_fissati(candidate, congelata, versione))

    # HEAD e' `revisione_assurance`: si deriva, non si scrive.
    testa = revisione_risolta("HEAD")
    if testa is None:
        motivi.append("git: HEAD non si risolve a un commit")
    elif testa != congelata:
        if not _discende_da(congelata, testa):
            motivi.append(
                f"HEAD «{testa[:7]}» non discende dalla revisione congelata "
                f"«{congelata[:7]}». L'assurance viene **dopo** la candidate: "
                "un HEAD che non la contiene attesta un altro albero."
            )
        else:
            fuori, guasti = cambiamenti_dopo_il_congelamento(congelata)
            motivi.extend(guasti)
            if fuori:
                motivi.append(
                    f"dopo il congelamento «{congelata[:7]}» sono cambiati file "
                    f"che l'assurance non produce: {fuori}. Gli artefatti sono "
                    "gia' stati costruiti da quella revisione, e cio' che li "
                    "produce o li verifica non puo' muoversi dopo."
                )

    motivi.extend(_coda_della_candidate(candidate))
    return motivi


def _coda_della_candidate(candidate: dict[str, Any]) -> list[str]:
    if candidate.get("release_action_allowed") is not True:
        return ["`release_action.allowed` non e' consentita"]
    return []


def condizione_qualifica_cross_component(documento: dict[str, Any]) -> list[str]:
    """La catena e' **superata**, oppure **dichiaratamente differita**.

    La condizione non e' stata tolta quando la 2.0.0 ha smesso di pretendere la
    qualifica di sistema. Toglierla avrebbe accorciato di uno l'insieme chiuso
    delle condizioni e lasciato il rinvio senza nessuno che lo guardasse, che e'
    la forma di verde piu' comoda: la condizione che non passa sparisce, e il
    documento resta a dire che le condizioni sono cinque. Cambia invece **cio'
    che verifica**.

    Restano due sole vie, e la seconda non e' piu' economica della prima.

    * `verified` — l'artefatto dell'owner dice `passed`. L'esito si legge di
      la', non dal campo con cui la voce si descrive.
    * `differita` — la voce e' fra le differibili, porta il proprio
      non-promette, e l'artefatto **non** dice `passed`. Costa una
      dichiarazione esplicita di cio' che la release smette di promettere, e
      quella dichiarazione finisce nella tabella generata di `docs/RELEASE.md`:
      non e' un campo che si possa mettere e dimenticare.

    `release_blocking` non soddisfa la condizione, ed e' il caso in cui la
    qualifica e' ancora pretesa e non c'e'.

    L'artefatto non e' nominato qui: si prende dalla prova dell'invariante che
    lo governa, cosi' un cambio di percorso non lascia questa condizione a
    guardare un file che nessuno aggiorna piu'.
    """
    voce = next(
        (
            v
            for v in documento.get("invarianti", [])
            if v.get("id") == "sistema.qualifica-cross-component"
        ),
        None,
    )
    if voce is None:
        return ["`sistema.qualifica-cross-component` assente dal registro"]
    if voce.get("stato") == "differita":
        return [
            f"`sistema.qualifica-cross-component`: {m}"
            for m in _differita_ben_dichiarata(voce)
        ]
    percorsi = _percorsi((voce.get("prova") or {}).get("artefatto"))
    if not percorsi:
        return [
            "`sistema.qualifica-cross-component`: nessun artefatto da cui "
            "leggere l'esito della qualifica"
        ]
    motivi: list[str] = []
    for relativo in percorsi:
        _, trovati = stato_esterno_osservato(relativo)
        motivi.extend(trovati)
    return motivi


def verifica_condizione(
    condizione: dict[str, Any], documento: dict[str, Any]
) -> list[str]:
    """I motivi per cui una condizione non e' soddisfatta; vuoto se lo e'."""
    verifica = condizione["verifica"]
    if verifica["tipo"] == "gate":
        comando = verifica["comando"]
        esito = subprocess.run(
            comando, cwd=ROOT, capture_output=True, text=True, check=False
        )
        if esito.returncode != 0:
            return [f"«{' '.join(comando)}» esce con {esito.returncode}"]
        return []
    return globals()[verifica["funzione"]](documento)


def _dentro(documento: Any, percorso: tuple[str, ...]) -> Any:
    """Il valore in fondo a un percorso di chiavi, o `None` se si interrompe."""
    corrente = documento
    for chiave in percorso:
        if not isinstance(corrente, dict) or chiave not in corrente:
            return None
        corrente = corrente[chiave]
    return corrente


DIRECTORY_EVIDENZE = ROOT / "assurance" / "evidence"
CARTELLA_DELLE_EVIDENZE = "assurance/evidence"

# Un segmento che non nomina un figlio.
SEGMENTI_VIETATI = {"", ".", ".."}


def percorso_canonico(valore: Any) -> str | None:
    """Il percorso, se e' un relativo canonico in forma POSIX; `None` altrimenti.

    Un percorso veniva accettato come «stringa non vuota», e il confronto fra
    percorsi si faceva sul solo nome del file. Due scritture diverse dello
    stesso file passavano — `assurance/evidence/../evidence/x.json` — e una
    voce del manifest poteva uscire dalla directory di corsa con `../fuori.log`
    senza che nulla lo dicesse: il digest si ricalcola su qualunque insieme di
    stringhe, quindi restava coerente con se stesso.

    Canonico significa: relativo, separato da `/`, senza segmenti vuoti, `.` o
    `..`, senza NUL, senza backslash e senza due punti — cosi' una sola
    scrittura nomina un file, e nessuna esce dall'albero che la contiene.
    """
    if not isinstance(valore, str) or not valore:
        return None
    if "\0" in valore or "\\" in valore or ":" in valore:
        return None
    if valore.startswith("/"):
        return None
    if any(segmento in SEGMENTI_VIETATI for segmento in valore.split("/")):
        return None
    return valore


def _sola_evidenza_corrente(stato: dict[str, Any]) -> list[str]:
    """`assurance/evidence/` contiene **la sola** evidenza citata dallo stato.

    Le evidenze precedenti restavano nell'albero senza che alcun gate le
    leggesse. Costavano piu' di quanto rendessero: la loro sola presenza
    invitava a confronti fra corse — e un confronto fra corse **non e'
    ricostruibile** da un albero in cui quelle corse non ci sono piu' o non
    sono verificabili. Una di esse portava un digest anteriore alla forma
    canonica, quindi non ricalcolabile; un'altra dichiarava di non fornire
    numeri e veniva citata lo stesso.

    Git conserva la storia. L'albero di lavoro dice che cosa vale **oggi**.
    """
    relativo = percorso_canonico(_dentro(stato, ("ultima_misura", "evidenza")))
    if relativo is None:
        return [
            "`ultima_misura.evidenza` assente o non canonico: non c'e' "
            "un'evidenza corrente che si possa nominare in un modo solo"
        ]
    if Path(relativo).parent.as_posix() != CARTELLA_DELLE_EVIDENZE:
        return [
            f"`ultima_misura.evidenza` vale «{relativo}», che non sta in "
            f"`{CARTELLA_DELLE_EVIDENZE}/`"
        ]

    attesa = Path(relativo).name
    # `iterdir` e non `glob("*.json")`: filtrare per estensione e' una
    # whitelist travestita da controllo di contenuto. Un `checkpoint-old.json.bak`,
    # una sottodirectory o un `.orig` lasciato da un merge restavano invisibili,
    # e «contiene la corrente e nient'altro» era vero solo dei `.json`.
    presenti = sorted(voce.name for voce in DIRECTORY_EVIDENZE.iterdir())
    if presenti != [attesa]:
        return [
            f"assurance/evidence contiene {presenti}, e deve contenere "
            f"esattamente [{attesa!r}]. Un'evidenza che nessun gate legge e' un "
            "documento che nessuno rilegge, e la sua presenza invita a "
            "confronti fra corse che l'albero non permette di ricostruire. La "
            "storia sta in git."
        ]

    percorso = DIRECTORY_EVIDENZE / attesa
    if percorso.is_symlink():
        return [
            f"assurance/evidence/{attesa} e' un link. Un'evidenza deve essere "
            "il file che dichiara di essere: un link punta a un contenuto che "
            "l'albero non registra, e che puo' cambiare senza che il repository "
            "lo mostri."
        ]
    if not percorso.is_file():
        return [f"assurance/evidence/{attesa} non e' un file regolare"]
    return []


def _evidenza(relativo: str) -> dict[str, Any]:
    """L'evidenza di una corsa, letta dal disco.

    Sta in una funzione perche' le sonde devono poterla sostituire: scrivere
    un'evidenza finta dentro il repository per provarne una malformata
    lascerebbe un file non tracciato, e l'impronta dell'albero lo vedrebbe
    proprio mentre il checkpoint la misura.
    """
    return json.loads((ROOT / relativo).read_text(encoding="utf-8"))


# L'impronta dell'albero e' uno sha256 in minuscolo.
IMPRONTA = re.compile(r"^[0-9a-f]{64}$")


def digest_del_manifest(manifest: dict[str, str]) -> str:
    """Il digest del manifest degli artefatti, **ricalcolato**.

    La forma e' dichiarata qui e non dedotta dalla prosa dell'evidenza: quella
    dice «percorsi relativi ordinati piu' sha256 del contenuto» e non fissa i
    delimitatori, quindi non determina un valore. Un digest che nessuno puo'
    ricalcolare e' una stringa, non un'impronta.

    `percorso` NUL `sha256` LF, in ordine di percorso.
    """
    accumulatore = hashlib.sha256()
    for percorso in sorted(manifest):
        accumulatore.update(percorso.encode("utf-8"))
        accumulatore.update(b"\0")
        accumulatore.update(str(manifest[percorso]).encode("ascii"))
        accumulatore.update(b"\n")
    return accumulatore.hexdigest()


def evidenza_coerente(evidenza: dict[str, Any], atteso: str | None) -> list[str]:
    """L'evidenza e' coerente **con se stessa**, prima che con lo stato.

    Il legame copiava i campi dichiarati e ne confrontava uno solo: la
    revisione finale. Sostituire `revisione_iniziale` con il commit radice, o
    `impronta_finale` con un altro valore, non produceva alcun errore —
    l'evidenza si contraddiceva e lo stato le restava fedele.

    `atteso` e' lo SHA completo della revisione misurata, o `None` se non si e'
    potuto risolvere: in quel caso le revisioni si confrontano fra loro, che e'
    comunque una contraddizione osservabile.
    """
    errori: list[str] = []

    # --- la corsa descrive una sola revisione, e un solo albero ------------
    iniziale = revisione_risolta(_dentro(evidenza, ("corsa", "revisione_iniziale")))
    finale = revisione_risolta(_dentro(evidenza, ("corsa", "revisione_finale")))
    for nome, risolta in (("revisione_iniziale", iniziale), ("revisione_finale", finale)):
        if risolta is None:
            errori.append(
                f"`corsa.{nome}` non si risolve a un commit: una corsa che non "
                "nomina una revisione non descrive un albero"
            )
    if iniziale is not None and finale is not None and iniziale != finale:
        errori.append(
            f"`corsa.revisione_iniziale` e' «{iniziale[:7]}» e "
            f"`corsa.revisione_finale` e' «{finale[:7]}»: HEAD si e' mosso "
            "durante la corsa, e la misura descrive un albero mentre l'esito ne "
            "nomina un altro."
        )
    if atteso is not None:
        for nome, risolta in (
            ("revisione_iniziale", iniziale),
            ("revisione_finale", finale),
        ):
            if risolta is not None and risolta != atteso:
                errori.append(
                    f"`corsa.{nome}` e' «{risolta[:7]}», la revisione misurata "
                    f"e' «{atteso[:7]}»"
                )

    partenza = _dentro(evidenza, ("corsa", "impronta_iniziale"))
    arrivo = _dentro(evidenza, ("corsa", "impronta_finale"))
    for nome, valore in (("impronta_iniziale", partenza), ("impronta_finale", arrivo)):
        if not isinstance(valore, str) or not IMPRONTA.match(valore):
            errori.append(f"`corsa.{nome}` non e' uno sha256: «{valore}»")
    if isinstance(partenza, str) and isinstance(arrivo, str) and partenza != arrivo:
        errori.append(
            f"`corsa.impronta_iniziale` e' «{partenza[:8]}» e "
            f"`corsa.impronta_finale` e' «{arrivo[:8]}»: un passo ha scritto "
            "nell'albero che stava verificando, e la misura non vale per "
            "l'albero dichiarato."
        )

    errori.extend(_riconciliazione_coerente(evidenza))
    errori.extend(_misure_coerenti(evidenza))
    errori.extend(_artefatti_coerenti(evidenza))
    return errori


# Un digest di file e' uno sha256 in minuscolo, come l'impronta dell'albero.
SHA256 = re.compile(r"^[0-9a-f]{64}$")

# Il risultato della corsa e' l'ultimo file che il checkpoint scrive. Un
# manifest che non lo elenca descrive una corsa che non e' arrivata in fondo,
# o una directory raccolta altrove.
RISULTATO_DELLA_CORSA = "risultato.json"


def _intero(valore: Any) -> bool:
    """Un intero vero: `True` e' un `int` per Python, e non e' un conteggio."""
    return isinstance(valore, int) and not isinstance(valore, bool) and valore >= 0


def _numero(valore: Any) -> bool:
    """Un numero **finito**: `inf` e `nan` superano ogni confronto o nessuno."""
    return (
        isinstance(valore, (int, float))
        and not isinstance(valore, bool)
        and math.isfinite(valore)
    )


# Il registro canonico dei passi, condiviso con il checkpoint: quello confronta
# cio' che ha eseguito, questo confronta cio' che l'evidenza descrive.
#
# Senza, il verificatore controllava perfettamente cio' che l'evidenza
# dichiarava e non sapeva che `fmt` dovesse esistere: togliere un gate e
# aggiornare coerentemente contatori, artefatti e digest passava. `len == 57`
# non basterebbe — un passo tolto e uno aggiunto lasciano il totale fermo.
#
# Il percorso e' **relativo** perche' il registro va letto dove la corsa lo ha
# letto: alla revisione che l'evidenza misura, non nel working tree. Vedi
# `registro_della_revisione`.
REGISTRO_DEI_PASSI_RELATIVO = "assurance/registries/passi-del-checkpoint.json"

# Il registro dei requisiti di profondita' del fuzzing: `_profondita_legata` ne
# legge il percorso dell'artefatto invece di riscriverlo, cosi' lo stato non puo'
# puntare a un file diverso da quello che il gate della profondita' legge.
REGISTRO_DI_PROFONDITA = (
    ROOT / "assurance" / "registries" / "profondita-fuzz-shapefile.json"
)
REGISTRO_DI_PROFONDITA_FILEGDB = (
    ROOT / "assurance" / "registries" / "profondita-fuzz-filegdb.json"
)
# Il confine di AddressSanitizer, misurato sul binario. Lo stato ne ripete due
# numeri, e ripeterli senza confrontarli sarebbe il modo in cui il piu' comodo
# dei due finirebbe per essere quello scritto.
MISURA_ASAN_FILEGDB = ROOT / "assurance" / "asan-filegdb.json"


# Lo schema del registro dei passi. E' **chiuso**, e in entrambe le direzioni:
# una voce e' esattamente `{id, log}`, l'identita' e' una stringa non vuota e
# `log` e' un booleano.
#
# Non e' pedanteria. Il registro decide che cosa un'evidenza deve contenere, e
# la lettura permissiva accettava `log: 1`, `log: "false"`, un campo in piu' che
# nessuno guarda e uno `schema_version` sbagliato: `not v["log"]` su `"false"`
# e' falso, cioe' quella voce avrebbe preteso un log dove il registro voleva
# dire il contrario, e l'evidenza sarebbe stata giudicata contro una regola che
# nessuno ha scritto.
SCHEMA_DEL_REGISTRO = 1
CAMPI_DI_VOCE_DEL_REGISTRO = frozenset({"id", "log"})


def registro_dal_testo(
    grezzo: bytes, dove: str
) -> tuple[tuple[str, ...], frozenset[str], str | None]:
    """`(identita' in ordine, quelle senza log, errore)` da un registro serio.

    `dove` compare nei messaggi e non altrove: e' la revisione, o il percorso,
    di cui si sta parlando.
    """
    try:
        testo = grezzo.decode("utf-8")
    except UnicodeDecodeError as errore:
        return (), frozenset(), f"il registro dei passi {dove} non e' UTF-8: {errore}"
    try:
        documento = json.loads(testo)
    except json.JSONDecodeError as errore:
        return (), frozenset(), f"il registro dei passi {dove} non e' JSON: {errore}"
    if not isinstance(documento, dict):
        return (), frozenset(), f"il registro dei passi {dove} non e' un oggetto"

    versione = documento.get("schema_version")
    # `type(...) is int`, non `isinstance` e non il solo `==`: in Python `True`
    # e `1.0` sono entrambi uguali a 1, quindi `schema_version: true` e
    # `schema_version: 1.0` passavano per la versione 1. Un JSON che dichiara un
    # booleano dove va un intero non e' la versione 1: e' un documento di cui non
    # si sa niente.
    if type(versione) is not int or versione != SCHEMA_DEL_REGISTRO:
        return (), frozenset(), (
            f"il registro dei passi {dove} dichiara `schema_version` "
            f"«{versione}», atteso {SCHEMA_DEL_REGISTRO}. Uno schema diverso si "
            "legge con regole diverse, e leggerlo con queste sarebbe leggerlo "
            "male senza accorgersene."
        )

    voci = documento.get("passi")
    if not isinstance(voci, list) or not voci:
        return (), frozenset(), (
            f"il registro dei passi {dove} non dichiara un elenco `passi` non vuoto"
        )

    ordine: list[str] = []
    senza_log: set[str] = set()
    for posizione, voce in enumerate(voci, 1):
        if not isinstance(voce, dict) or set(voce) != CAMPI_DI_VOCE_DEL_REGISTRO:
            return (), frozenset(), (
                f"il registro dei passi {dove}, voce {posizione}: campi "
                f"{sorted(voce) if isinstance(voce, dict) else voce}, attesi "
                f"esattamente {sorted(CAMPI_DI_VOCE_DEL_REGISTRO)}"
            )
        identita = voce["id"]
        if not isinstance(identita, str) or not identita:
            return (), frozenset(), (
                f"il registro dei passi {dove}, voce {posizione}: `id` non e' "
                f"una stringa non vuota ma «{identita}»"
            )
        log = voce["log"]
        # `isinstance(1, bool)` e' falso, quindi `log: 1` non passa di qui: e'
        # esattamente il caso che la lettura permissiva accettava.
        if not isinstance(log, bool):
            return (), frozenset(), (
                f"il registro dei passi {dove}, voce {posizione} «{identita}»: "
                f"`log` non e' un booleano ma «{log}» ({type(log).__name__})"
            )
        if identita in ordine:
            return (), frozenset(), (
                f"il registro dei passi {dove} dichiara «{identita}» due volte. "
                "Due passi omonimi scriverebbero lo stesso log, e il manifest ne "
                "conterebbe uno."
            )
        ordine.append(identita)
        if not log:
            senza_log.add(identita)
    return tuple(ordine), frozenset(senza_log), None


def _registro_corrente_ben_formato(_stato: dict[str, Any]) -> list[str]:
    """Il registro del working tree passa lo **stesso** schema di quelli storici.

    Senza, la validazione stretta varrebbe per i registri del passato e non per
    quello che la prossima corsa riconciliera': la forma sarebbe verificata
    dappertutto tranne dove si scrive.
    """
    percorso = ROOT / REGISTRO_DEI_PASSI_RELATIVO
    if not percorso.exists():
        return [f"{REGISTRO_DEI_PASSI_RELATIVO}: assente dal working tree"]
    _, _, guasto = registro_dal_testo(
        percorso.read_bytes(), f"in {REGISTRO_DEI_PASSI_RELATIVO}"
    )
    return [] if guasto is None else [guasto]


@functools.lru_cache(maxsize=8)
def registro_della_revisione(
    revisione: str,
) -> tuple[tuple[str, ...], frozenset[str], str | None]:
    """`(identita' in ordine, quelle senza log, errore)` **alla revisione data**.

    # Perche' non dal working tree

    Un'evidenza e' il verbale di una corsa passata, e una corsa passata non puo'
    aver eseguito un passo introdotto dopo. Confrontarla con il registro di HEAD
    faceva percio' due cose sbagliate insieme: dichiarava incoerente
    un'evidenza che era esatta quando e' stata scritta, e rendeva il registro
    **immutabile**, perche' aggiungere un passo rendeva rosso questo gate,
    quindi rossi livello 1 e livello 2, quindi impossibile produrre l'evidenza
    nuova che avrebbe sciolto il nodo. Il registro e' rimasto fermo non per
    scelta ma per costruzione.

    Il registro viene percio' letto **dalla revisione che l'evidenza misura**,
    che e' un commit e quindi immutabile. Cio' che il checkpoint fa a ogni corsa
    non cambia: riconcilia cio' che ha eseguito con il registro del suo
    worktree, per insieme, ordine e log.

    # Nessun ripiego

    Se la revisione non si risolve, se il file non esiste a quella revisione, o
    se non e' un JSON con la forma attesa, la risposta e' un errore. Ripiegare
    sul registro corrente vorrebbe dire verificare un verbale contro un altro
    documento e chiamarlo verificato, che e' peggio del non averlo verificato:
    ne ha l'aspetto.
    """
    esito = subprocess.run(
        ["git", "show", f"{revisione}:{REGISTRO_DEI_PASSI_RELATIVO}"],
        cwd=ROOT,
        capture_output=True,
        check=False,
    )
    if esito.returncode != 0:
        return (), frozenset(), (
            f"il registro dei passi non e' leggibile alla revisione "
            f"«{revisione[:7]}»: `git show` esce con {esito.returncode}. Senza "
            "il registro di quella revisione non c'e' niente contro cui "
            "verificare l'elenco, e il registro corrente descrive un'altra corsa."
        )
    return registro_dal_testo(esito.stdout, f"alla revisione «{revisione[:7]}»")

# Gli artefatti che una corsa produce e che non sono il log di un passo.
# L'elenco e' chiuso: il manifest e' esattamente i log dei passi piu' questi, e
# un file in piu' o in meno e' rosso.
#
# `coverage_diff.log` ne fa parte, quindi la diagnostica differenziale e'
# **obbligatoria** per un'evidenza di livello 2: un livello 2 lanciato senza
# `S9_CHECKPOINT_BASE` salta quel passo e non puo' qualificare. Un'evidenza
# descrive una corsa completa.
# Gli artefatti che non sono il log di un passo, e **chi li scrive**.
#
# Erano un insieme piatto, e l'insieme e' andato alla deriva: `4e9d3d3` ha
# aggiunto la misura di copertura dedicata alla CLI, con essa i file
# `lcov-completo.info` e `coverage_diff_cli.log`, e questa costante e' rimasta
# ferma. Nessuno se ne e' accorto perche' nessuna evidenza di livello 2 e' stata
# scritta da allora: il difetto stava su una strada che non veniva percorsa, e
# si e' manifestato al primo uso -- come un rosso, non come un verde falso, che
# e' il verso fortunato.
#
# Ora l'attesa si **deriva**. Un artefatto e' atteso se il passo che lo scrive
# compare fra i passi della corsa: cosi' un'evidenza prodotta prima che quel
# passo esistesse resta valida, e un passo aggiunto domani porta con se' il
# proprio artefatto senza che nessuno debba ricordarsi di questa riga.
ARTEFATTO_DEL_PASSO = {
    "check_filegdb_catalog": "catalog.json",
    "coverage_export": "lcov.info",
    "coverage_export_cli": "lcov-completo.info",
}

# I log della diagnostica differenziale, scritti **solo** quando la diagnostica
# gira: `s9-checkpoint.sh` li produce dentro il ramo che pretende
# `S9_CHECKPOINT_BASE`.
#
# Sono condizionali in tutte e due le direzioni. Pretenderli sempre rendeva
# irregistrabile una corsa senza base; ammetterli sempre lascerebbe passare
# un'evidenza che porta i log di una misura che dichiara di non aver fatto.
#
# # Il discriminante e' la base, **non** l'esito
#
# La prima stesura di questa correzione usava `esito != "n/d"`, e i dati l'hanno
# smentita subito: l'evidenza `c96ffac` dichiara `n/d` e porta `coverage_diff.log`.
# Non e' una contraddizione, sono due cose diverse che `n/d` non distingue --
# «la diagnostica non e' stata eseguita» e «e' stata eseguita e non aveva righe
# da misurare, perche' il cambio stava fuori dal suo perimetro». Il secondo caso
# produce i log e non produce una percentuale.
#
# Cio' che decide se i file esistono e' `S9_CHECKPOINT_BASE`, che l'evidenza
# registra come `diagnostica_differenziale.base`.
LOG_DELLA_DIAGNOSTICA = "coverage_diff.log"

# Il secondo log della diagnostica esiste solo dove esiste la misura di
# copertura della CLI: lo scrive lo stesso ramo, sullo stesso `lcov-completo.info`
# che `coverage_export_cli` produce. Un'evidenza anteriore a quel passo ha la
# base impostata e **un** log solo, ed e' corretta cosi'.
LOG_DIAGNOSTICA_DEL_PASSO = {"coverage_export_cli": "coverage_diff_cli.log"}

CAMPI_DI_PASSO = {"id", "esito", "log"}
ESITO_DI_PASSO_SUPERATO = "verde"


def _passi_dichiarati(evidenza: dict[str, Any]) -> tuple[list[dict[str, Any]], list[str]]:
    """L'elenco dei passi, verificato voce per voce.

    I contatori dicono **quanti**; l'elenco dice **quali**. Senza, il manifest
    poteva essere ridotto a due file mentre la riconciliazione continuava a
    dichiarare 57/57: non c'era niente da confrontare.
    """
    passi = _dentro(evidenza, ("riconciliazione", "passi"))
    if not isinstance(passi, list) or not passi:
        return [], [
            "`riconciliazione.passi` assente o vuoto: senza l'elenco dei passi "
            "i conteggi non hanno nulla da cui derivare, e il manifest nulla a "
            "cui essere legato"
        ]

    # Il registro contro cui confrontare e' quello **della corsa**, non quello
    # di adesso. `evidenza_coerente` ha gia' preteso che revisione iniziale e
    # finale coincidano, quindi nominare la finale nomina l'unica revisione
    # che la corsa descrive.
    revisione = revisione_risolta(_dentro(evidenza, ("corsa", "revisione_finale")))
    if revisione is None:
        return passi, [
            "`corsa.revisione_finale` non si risolve a un commit: senza la "
            "revisione misurata non si sa quale registro dei passi la corsa "
            "abbia riconciliato, e l'elenco non e' verificabile."
        ]
    dichiarati, senza_log, guasto = registro_della_revisione(revisione)
    if guasto is not None:
        return passi, [f"`riconciliazione.passi`: {guasto}"]

    errori: list[str] = []
    for voce in passi:
        # Il sottoschema e' **chiuso**: `{id, esito, log, extra}` passava, e un
        # campo che nessuno guarda e' un campo che qualcuno prima o poi legge.
        if not isinstance(voce, dict) or set(voce) != CAMPI_DI_PASSO:
            errori.append(
                f"`riconciliazione.passi`: voce con campi "
                f"{sorted(voce) if isinstance(voce, dict) else voce}, attesi "
                f"esattamente {sorted(CAMPI_DI_PASSO)}"
            )
            continue
        identita = voce["id"]
        if not isinstance(identita, str) or not identita:
            errori.append(f"`riconciliazione.passi`: identita' non valida: «{identita}»")
            continue
        if voce["esito"] != ESITO_DI_PASSO_SUPERATO:
            errori.append(
                f"`riconciliazione.passi`: «{identita}» ha esito "
                f"«{voce['esito']}» accanto a un esito «{ESITO_LIVELLO_2}»"
            )
        atteso = None if identita in senza_log else f"{identita}.log"
        if voce["log"] != atteso:
            errori.append(
                f"`riconciliazione.passi`: «{identita}» dichiara log "
                f"«{voce['log']}», atteso «{atteso}»"
            )
    if errori:
        return passi, errori

    identita = [voce["id"] for voce in passi]
    ripetute = sorted({nome for nome in identita if identita.count(nome) > 1})
    if ripetute:
        errori.append(
            f"`riconciliazione.passi`: identita' ripetute {ripetute}. Due passi "
            "omonimi scriverebbero lo stesso log, e il manifest ne conterebbe uno."
        )

    # L'insieme e' **esatto**, non un conteggio: un passo tolto e uno aggiunto
    # lascerebbero il totale fermo.
    mancanti = [nome for nome in dichiarati if nome not in identita]
    estranei = [nome for nome in identita if nome not in dichiarati]
    if mancanti:
        errori.append(
            f"`riconciliazione.passi`: {mancanti} sono dichiarati dal registro "
            f"dei passi alla revisione «{revisione[:7]}» e non compaiono. Una "
            "corsa che non li esegue misura un altro insieme, e i suoi conteggi "
            "sono coerenti con se stessi."
        )
    if estranei:
        errori.append(
            f"`riconciliazione.passi`: {estranei} non sono nel registro dei "
            f"passi alla revisione «{revisione[:7]}». Un passo che nessuno ha "
            "dichiarato non e' verificato da nessuno."
        )

    # L'**ordine** e' dichiarato dal registro, e va confrontato: presenza,
    # estranei e duplicati non lo vedono. Scambiare due passi lasciava
    # l'insieme identico, e l'ordine e' cio' che rende leggibile una corsa —
    # `sonde_checkpoint` per primo perche' se il gate che misura e' rotto tutto
    # cio' che segue e' una misura di cui non si sa niente, la copertura dopo
    # il fuzzing perche' legge il profdata che quello ha prodotto.
    #
    # Il confronto e' sequenziale e si ferma alla **prima** posizione
    # divergente: elencarle tutte, dopo uno scambio, ne stamperebbe due che
    # dicono la stessa cosa, e dopo uno spostamento le stamperebbe quasi tutte.
    if not errori and identita != list(dichiarati):
        for posizione, (osservata, attesa) in enumerate(zip(identita, dichiarati), 1):
            if osservata != attesa:
                errori.append(
                    f"`riconciliazione.passi`: ordine divergente alla posizione "
                    f"{posizione}: «{osservata}» dove il registro dichiara "
                    f"«{attesa}»"
                )
                break
    return passi, errori


def _manifest_legato_ai_passi(
    evidenza: dict[str, Any], passi: list[dict[str, Any]]
) -> list[str]:
    """Il manifest e' **esattamente** cio' che la corsa dichiarata ha scritto.

    Non «i log dei passi piu' un elenco fisso»: l'elenco fisso e' andato alla
    deriva una volta e lo rifarebbe. L'attesa si deriva da due cose che
    l'evidenza gia' porta -- quali passi sono stati eseguiti, e se la
    diagnostica differenziale ha misurato -- e da nient'altro.
    """
    manifest = _dentro(evidenza, ("artefatti", "manifest"))
    if not isinstance(manifest, dict):
        return []

    identita = {voce["id"] for voce in passi}
    attesi = {voce["log"] for voce in passi if voce.get("log")}
    attesi.add(RISULTATO_DELLA_CORSA)
    attesi.update(
        artefatto
        for passo, artefatto in ARTEFATTO_DEL_PASSO.items()
        if passo in identita
    )
    # I log della diagnostica: presenti se la base c'era, assenti se no. Non si
    # legge il campo per sapere se «tollerarli»: si legge per sapere quali file
    # quella corsa **deve** avere prodotto.
    if _dentro(evidenza, ("misure", "diagnostica_differenziale", "base")):
        attesi.add(LOG_DELLA_DIAGNOSTICA)
        attesi.update(
            log
            for passo, log in LOG_DIAGNOSTICA_DEL_PASSO.items()
            if passo in identita
        )

    presenti = set(manifest)
    mancanti = sorted(attesi - presenti)
    estranei = sorted(presenti - attesi)

    errori: list[str] = []
    if mancanti:
        errori.append(
            f"`artefatti.manifest`: mancano {len(mancanti)} artefatti che i "
            f"passi dichiarano, fra cui {mancanti[:3]}. Un manifest ridotto "
            "lascia la riconciliazione a dichiarare passi di cui non resta "
            "traccia."
        )
    if estranei:
        # Il messaggio distingue il caso che si sbaglia piu' facilmente: i log
        # della diagnostica dentro un'evidenza che non dichiara una base. Dire
        # genericamente «estranei» manderebbe a cercare un file di troppo,
        # mentre il difetto sta nel campo che dichiara la base.
        # Solo quando la base **manca davvero**: con la base impostata un log
        # della diagnostica in piu' e' un orfano come gli altri -- segue il
        # proprio passo, non la base -- e accusare il campo sbagliato manderebbe
        # a cercare dove non c'e' niente.
        tutti_i_log = {LOG_DELLA_DIAGNOSTICA} | set(LOG_DIAGNOSTICA_DEL_PASSO.values())
        senza_base = not _dentro(
            evidenza, ("misure", "diagnostica_differenziale", "base")
        )
        della_diagnostica = sorted(set(estranei) & tutti_i_log) if senza_base else []
        if della_diagnostica:
            errori.append(
                f"`artefatti.manifest`: porta {della_diagnostica}, ma "
                "`diagnostica_differenziale.base` e' vuota. Una corsa che non "
                "dichiara una base non ha eseguito la diagnostica e non puo' "
                "portarne i log: uno dei due campi mente, e non si sa quale."
            )
        altri = sorted(set(estranei) - set(della_diagnostica))
        if altri:
            errori.append(
                f"`artefatti.manifest`: {len(altri)} artefatti non appartengono "
                f"ad alcun passo della corsa, fra cui {altri[:3]}"
            )
    return errori


def _riconciliazione_coerente(evidenza: dict[str, Any]) -> list[str]:
    """I conteggi **si derivano dall'elenco**, non si accettano a parte.

    Erano sei numeri verificati fra loro: bastava che tornassero, e nessuno
    chiedeva da dove venissero. Ora l'elenco dei passi e' la fonte, i conteggi
    ne sono il riassunto, e il riassunto deve coincidere con cio' che riassume.

    Un'evidenza che dichiara `S9 checkpoint level 2 passed` con un passo
    fallito, uno omesso o un identificatore duplicato dice due cose opposte, e
    quella che si legge e' la prima riga.
    """
    conti = evidenza.get("riconciliazione")
    if not isinstance(conti, dict):
        return ["`riconciliazione` assente"]

    passi, errori = _passi_dichiarati(evidenza)
    if errori:
        return errori

    identita = [voce["id"] for voce in passi]
    derivati = {
        "identificatori_distinti": len(set(identita)),
        "eseguiti": len(passi),
        "verdi": sum(1 for v in passi if v["esito"] == ESITO_DI_PASSO_SUPERATO),
        "omessi": sum(1 for v in passi if v["esito"] == "omesso"),
        "falliti": sum(
            1 for v in passi if v["esito"] not in (ESITO_DI_PASSO_SUPERATO, "omesso")
        ),
        "duplicati": len(identita) - len(set(identita)),
    }
    errori.extend(
        f"`riconciliazione.{campo}` dichiara «{conti.get(campo)}», l'elenco dei "
        f"passi ne conta «{valore}»"
        for campo, valore in derivati.items()
        if conti.get(campo) != valore
    )
    errori.extend(_manifest_legato_ai_passi(evidenza, passi))
    return errori


def _misure_coerenti(evidenza: dict[str, Any]) -> list[str]:
    """Le misure non si contraddicono, e non contraddicono l'esito.

    Il sottoschema accettava evidenze **semanticamente impossibili**: una
    copertura cargo a zero accanto a un checkpoint superato, un replay di zero
    input, e una copertura reale sotto soglia arrotondata a una percentuale che
    la soglia la raggiunge — 79,999% dichiarato «80,00%» contro una soglia
    dell'80% passava, perche' la soglia veniva confrontata con la percentuale
    arrotondata invece che con il rapporto.
    """
    return (
        _copertura_coerente(evidenza)
        + _fuzz_coerente(evidenza)
        + _differenziale_coerente(evidenza)
    )


def _differenziale_coerente(evidenza: dict[str, Any]) -> list[str]:
    """I conteggi della diagnostica differenziale si riconciliano fra loro.

    Erano **copiati**: lo stato li prendeva dall'evidenza e il gate confrontava
    le due copie, quindi due documenti modificati insieme con numeri
    incompatibili -- 224 coperte e 18 scoperte su 300 righe cambiate, o una
    percentuale che non e' il loro rapporto -- restavano verdi. Un numero
    confrontato solo con la propria copia non e' verificato: e' ripetuto.

    Finche' la diagnostica dichiara `n/d` non c'e' niente da riconciliare, e
    quel caso resta ammesso: e' cio' che una corsa scrive quando la base non e'
    impostata o quando nessuna riga cambiata e' eseguibile.
    """
    diagnostica = _dentro(evidenza, ("misure", "diagnostica_differenziale"))
    if not isinstance(diagnostica, dict):
        return ["`misure.diagnostica_differenziale` assente"]

    esito = diagnostica.get("esito")
    if not isinstance(esito, str) or not esito:
        return [
            f"`diagnostica_differenziale.esito` non e' una stringa: «{esito}». "
            "O e' una percentuale, o e' «n/d» con la sua ragione."
        ]

    cambiate = diagnostica.get("righe_cambiate_eseguibili")
    coperte = diagnostica.get("coperte")
    scoperte = diagnostica.get("scoperte")
    non_eseguibili = diagnostica.get("cambiate_non_eseguibili")
    conteggi = {
        "righe_cambiate_eseguibili": cambiate,
        "coperte": coperte,
        "scoperte": scoperte,
        "cambiate_non_eseguibili": non_eseguibili,
    }
    errori = [
        f"`diagnostica_differenziale.{campo}` non e' un intero non negativo: «{valore}»"
        for campo, valore in conteggi.items()
        if not _intero(valore)
    ]
    if errori:
        return errori

    if coperte + scoperte != cambiate:
        errori.append(
            f"`diagnostica_differenziale`: {coperte} coperte piu' {scoperte} "
            f"scoperte fanno {coperte + scoperte}, non le {cambiate} righe "
            "cambiate ed eseguibili. Una riga cambiata ed eseguibile e' coperta "
            "o non lo e'; non c'e' un terzo stato."
        )

    # `righe_scoperte` e' **obbligatorio** quando la diagnostica produce una
    # percentuale, e la prima stesura lo riconciliava soltanto se presente:
    # cancellarlo dall'evidenza non rendeva rosso niente, cioe' bastava
    # toglierlo per non doverlo far tornare. Un elenco che si puo' omettere non
    # e' una prova di che cosa resta scoperto.
    #
    # Con `esito: n/d` non c'e' niente da elencare, e pretenderlo direbbe di
    # righe che nessuno ha misurato.
    elencate = diagnostica.get("righe_scoperte")
    if elencate is None and esito != "n/d":
        errori.append(
            "`diagnostica_differenziale.righe_scoperte` assente accanto a un "
            f"esito «{esito}»: i conteggi dicono **quante** righe restano "
            "scoperte, l'elenco dice **quali**, e senza l'affermazione non si "
            "puo' controllare."
        )
    elif elencate is not None:
        if not isinstance(elencate, list) or not all(
            isinstance(v, str) and v for v in elencate
        ):
            errori.append(
                "`diagnostica_differenziale.righe_scoperte` non e' un elenco di "
                "posizioni: senza, l'affermazione su che cosa resta scoperto non "
                "si puo' controllare."
            )
        else:
            if len(set(elencate)) != len(elencate):
                ripetute = sorted({v for v in elencate if elencate.count(v) > 1})
                errori.append(
                    f"`diagnostica_differenziale.righe_scoperte` ripete {ripetute}: "
                    "una riga contata due volte gonfia l'elenco senza gonfiare il "
                    "conteggio, ed e' il modo in cui i due smetterebbero di dire "
                    "la stessa cosa."
                )
            if len(elencate) != scoperte:
                errori.append(
                    f"`diagnostica_differenziale.righe_scoperte` elenca "
                    f"{len(elencate)} posizioni, `scoperte` ne dichiara {scoperte}"
                )

    if esito == "n/d":
        # Ammesso, ma non a caso: `n/d` con righe eseguibili cambiate vorrebbe
        # dire che la diagnostica ha misurato e non ha concluso.
        if cambiate:
            errori.append(
                f"`diagnostica_differenziale.esito` e' «n/d» con {cambiate} righe "
                "cambiate ed eseguibili: se ci sono righe da misurare, il "
                "risultato e' una percentuale."
            )
        return errori

    trovato = re.fullmatch(r"(\d+(?:\.\d+)?)%", esito)
    if not trovato:
        errori.append(
            f"`diagnostica_differenziale.esito` vale «{esito}»: ne' una "
            "percentuale ne' «n/d»"
        )
        return errori
    if cambiate == 0:
        errori.append(
            f"`diagnostica_differenziale.esito` vale «{esito}» su zero righe "
            "cambiate ed eseguibili: una percentuale senza denominatore"
        )
        return errori

    dichiarata = float(trovato.group(1))
    vera = coperte / cambiate * 100
    # Come per la copertura: il confronto e' sulla distanza, per non dipendere
    # dal modo di arrotondare a meta' esatta.
    if abs(vera - dichiarata) > 0.005 + 1e-9:
        errori.append(
            f"`diagnostica_differenziale.esito` vale {dichiarata}%, ma "
            f"{coperte}/{cambiate} fa {vera:.4f}%"
        )
    return errori


def _copertura_coerente(evidenza: dict[str, Any]) -> list[str]:
    copertura = _dentro(evidenza, ("misure", "copertura"))
    if not isinstance(copertura, dict):
        return ["`misure.copertura` assente"]

    errori: list[str] = []
    coperte = copertura.get("lcov_righe_coperte")
    strumentate = copertura.get("lcov_righe_strumentate")
    percentuale = copertura.get("lcov_percentuale")
    cargo = copertura.get("cargo_lines_percentuale")
    soglia = copertura.get("soglia")

    if not _numero(soglia) or not 0 < soglia <= 100:
        return [f"`copertura.soglia` non e' una percentuale: «{soglia}»"]
    if not (_intero(coperte) and _intero(strumentate)):
        return ["`copertura`: righe coperte e strumentate devono essere interi non negativi"]
    if strumentate == 0:
        return [
            "`copertura.lcov_righe_strumentate` e' zero: una copertura senza "
            "righe strumentate non e' una misura, e ogni percentuale su di essa "
            "sarebbe indefinita"
        ]
    if coperte > strumentate:
        return [
            f"`copertura`: {coperte} righe coperte su {strumentate} strumentate"
        ]

    vera = coperte / strumentate * 100
    if not _numero(percentuale):
        errori.append(f"`copertura.lcov_percentuale` non e' un numero finito: «{percentuale}»")
    # La percentuale dichiarata dev'essere il rapporto **arrotondato a due
    # decimali**. Il confronto e' sulla distanza invece che su `round`, per non
    # dipendere dal modo di arrotondare a meta' esatta.
    elif abs(vera - percentuale) > 0.005 + 1e-9:
        errori.append(
            f"`copertura.lcov_percentuale` vale {percentuale}, ma "
            f"{coperte}/{strumentate} fa {vera:.4f}%"
        )

    # La soglia si confronta con il **rapporto**, non con la percentuale
    # dichiarata: arrotondare 79,999% a 80,00% non porta la copertura sopra
    # soglia, sposta solo la cifra che si legge.
    if vera < soglia:
        errori.append(
            f"`copertura`: {coperte}/{strumentate} fa {vera:.4f}%, sotto la "
            f"soglia {soglia}%, accanto a un esito superato. La soglia si "
            "confronta con il rapporto, non con la percentuale arrotondata."
        )

    if not _numero(cargo) or not 0 <= cargo <= 100:
        errori.append(
            f"`copertura.cargo_lines_percentuale` non e' una percentuale: «{cargo}»"
        )
    elif cargo < soglia:
        errori.append(
            f"`copertura.cargo_lines_percentuale` vale {cargo}%, sotto la soglia "
            f"{soglia}%. Il passo `coverage_soglia_controprova` gira con "
            "`--fail-under-lines`, quindi un esito superato non puo' portarla."
        )
    return errori


def _fuzz_coerente(evidenza: dict[str, Any]) -> list[str]:
    errori: list[str] = []
    replay = _dentro(evidenza, ("misure", "fuzz_replay"))
    smoke = _dentro(evidenza, ("misure", "fuzz_smoke"))
    if not isinstance(replay, dict) or not isinstance(smoke, dict):
        return ["`misure.fuzz_replay` o `misure.fuzz_smoke` assenti"]

    campi = {
        "fuzz_replay": (replay, ("input", "target", "target_totali", "crash")),
        "fuzz_smoke": (
            smoke,
            ("target_eseguiti", "target_totali", "finding", "quarantena"),
        ),
    }
    for nome, (blocco, chiavi) in campi.items():
        for chiave in chiavi:
            if not _intero(blocco.get(chiave)):
                errori.append(
                    f"`{nome}.{chiave}` non e' un intero non negativo: "
                    f"«{blocco.get(chiave)}»"
                )
    if errori:
        return errori

    # Un conteggio a zero non e' un verde: e' una campagna che non e' girata.
    if replay["input"] == 0:
        errori.append(
            "`fuzz_replay.input` e' zero: un replay che non rigioca niente non "
            "trova niente, e il suo silenzio non e' un verde"
        )
    for nome, blocco, chiave in (
        ("fuzz_replay", replay, "target_totali"),
        ("fuzz_smoke", smoke, "target_totali"),
    ):
        if blocco[chiave] == 0:
            errori.append(f"`{nome}.{chiave}` e' zero: non esistono target da esercitare")

    if replay["crash"] != 0:
        errori.append(
            f"`fuzz_replay.crash` vale {replay['crash']} accanto a un esito superato"
        )
    if replay["target"] != replay["target_totali"]:
        errori.append(
            f"`fuzz_replay`: {replay['target']} target su "
            f"{replay['target_totali']} — il replay li rigioca tutti"
        )
    if smoke["finding"] != 0:
        errori.append(
            f"`fuzz_smoke.finding` vale {smoke['finding']} accanto a un esito superato"
        )
    if smoke["quarantena"] != 0:
        errori.append(
            f"`fuzz_smoke.quarantena` vale {smoke['quarantena']}: la quarantena "
            "dev'essere vuota"
        )
    if smoke["target_eseguiti"] != smoke["target_totali"]:
        errori.append(
            f"`fuzz_smoke`: {smoke['target_eseguiti']} target eseguiti su "
            f"{smoke['target_totali']}"
        )
    return errori


def _artefatti_coerenti(evidenza: dict[str, Any]) -> list[str]:
    """Il manifest e il suo digest si verificano a vicenda.

    Il digest aggregato si ricalcolava, e questo bastava a legare l'insieme —
    ma non a dire che le voci fossero digest: un manifest di `"x"` produce un
    aggregato coerente con se stesso. Un'impronta di file e' uno sha256, e
    senza quel vincolo il manifest impronta la propria forma invece del proprio
    contenuto.
    """
    artefatti = evidenza.get("artefatti")
    if not isinstance(artefatti, dict):
        return ["`artefatti` assente"]
    manifest = artefatti.get("manifest")
    if not isinstance(manifest, dict) or not manifest:
        return ["`artefatti.manifest` assente o vuoto"]

    errori: list[str] = []
    storti = sorted(
        str(percorso)
        for percorso, digest in manifest.items()
        if percorso_canonico(percorso) is None
        or not isinstance(digest, str)
        or not SHA256.match(digest)
    )
    if storti:
        errori.append(
            f"`artefatti.manifest`: {len(storti)} voci non sono «percorso "
            f"relativo canonico -> sha256», fra cui {storti[:3]}. Un `..` "
            "farebbe uscire un artefatto dalla directory di corsa, e il digest "
            "resterebbe coerente con se stesso."
        )
    # Due scritture che il filesystem risolve nello stesso file: su un volume
    # che non distingue le maiuscole `Fmt.log` e `fmt.log` sono un artefatto
    # solo, e il manifest ne conterebbe due.
    normalizzati: dict[str, list[str]] = {}
    for percorso in manifest:
        if isinstance(percorso, str):
            normalizzati.setdefault(percorso.casefold(), []).append(percorso)
    collisioni = sorted(v for v in normalizzati.values() if len(v) > 1)
    if collisioni:
        errori.append(
            f"`artefatti.manifest`: voci che normalizzano allo stesso percorso: "
            f"{collisioni[:3]}"
        )
    if RISULTATO_DELLA_CORSA not in manifest:
        errori.append(
            f"`artefatti.manifest` non elenca «{RISULTATO_DELLA_CORSA}»: e' "
            "l'ultimo file che la corsa scrive, e senza di esso il manifest "
            "descrive una corsa che non e' arrivata in fondo."
        )
    if artefatti.get("file") != len(manifest):
        errori.append(
            f"`artefatti.file` vale {artefatti.get('file')}, il manifest ne "
            f"elenca {len(manifest)}"
        )
    atteso = digest_del_manifest(manifest)
    if artefatti.get("digest_manifest") != atteso:
        errori.append(
            f"`artefatti.digest_manifest` vale "
            f"«{str(artefatti.get('digest_manifest'))[:12]}…», il manifest ne "
            f"produce «{atteso[:12]}…». Un digest che non si ricalcola dal "
            "proprio manifest non impronta niente."
        )
    return errori


def _misura_legata_all_evidenza(stato: dict[str, Any]) -> list[str]:
    """I numeri dello stato vengono dall'evidenza della corsa che li ha prodotti."""
    errori: list[str] = []
    misura = stato.get("ultima_misura")
    if not isinstance(misura, dict):
        return ["`ultima_misura` assente"]

    relativo = percorso_canonico(misura.get("evidenza"))
    if relativo is None:
        return [
            "`ultima_misura.evidenza` assente o non canonico: i numeri non "
            "hanno una corsa da cui venire, o ne hanno una che si puo' nominare "
            "in piu' modi"
        ]
    percorso = ROOT / relativo
    if not percorso.exists():
        return [f"«{relativo}»: evidenza assente"]

    evidenza = _evidenza(relativo)

    # `ultima_misura` e' per definizione l'ultima misura di **livello 2
    # superata**. Un'evidenza con un esito diverso non puo' occupare quel
    # posto, e la frase si confronta per intero: cercarla dentro il testo
    # accetterebbe un esito che la contiene e la contraddice.
    if evidenza.get("esito") != ESITO_LIVELLO_2:
        errori.append(
            f"«{relativo}» dichiara l'esito «{evidenza.get('esito')}», e "
            f"`ultima_misura` pretende esattamente «{ESITO_LIVELLO_2}»."
        )

    sha = misura.get("sha")
    risolta = revisione_risolta(sha)
    if risolta is None:
        errori.append(
            f"`ultima_misura.sha` vale «{sha}», che git non risolve a un "
            "commit. Una revisione che non si risolve non identifica un albero."
        )
    elif not isinstance(sha, str) or sha not in Path(relativo).name:
        errori.append(
            f"«{relativo}» non nomina la revisione misurata «{sha}». Il nome "
            "dell'evidenza e' cio' che lega la corsa alla revisione."
        )

    # L'evidenza dev'essere coerente **con se stessa** prima che con lo stato:
    # un'evidenza che si contraddice non diventa vera perche' lo stato la copia
    # fedelmente.
    errori.extend(f"«{relativo}»: {m}" for m in evidenza_coerente(evidenza, risolta))

    dichiarata = _dentro(stato, ("revisioni", "ultima_qualificata", "sha"))
    qualificata = revisione_risolta(dichiarata)
    if qualificata is None:
        errori.append(
            f"`revisioni.ultima_qualificata.sha` vale «{dichiarata}», che git "
            "non risolve a un commit"
        )
    elif risolta is not None and qualificata != risolta:
        errori.append(
            f"`revisioni.ultima_qualificata` dice «{dichiarata}» ma l'ultima "
            f"misura di livello 2 e' su «{sha}»"
        )

    for nello_stato, nell_evidenza in CAMPI_DALL_EVIDENZA:
        dichiarato = _dentro(misura, nello_stato)
        osservato = _dentro(evidenza, nell_evidenza)
        if dichiarato != osservato:
            errori.append(
                f"`ultima_misura.{'.'.join(nello_stato)}` vale «{dichiarato}», "
                f"«{relativo}» ne registra «{osservato}»"
            )
    return errori


def _conteggi_n1_legati_al_registro(stato: dict[str, Any]) -> list[str]:
    """I gruppi di ASSURANCE-N1 si contano nel registro di ASSURANCE-N1."""
    dichiarato = _dentro(stato, ("aperto", "assurance_n1"))
    if not isinstance(dichiarato, dict):
        return ["`aperto.assurance_n1` assente"]
    gruppi = carica_gruppi_n1()
    atteso = {"gruppi_totali": len(gruppi), "gruppi_aperti": len(debito_n1(gruppi))}
    return [
        f"`aperto.assurance_n1.{chiave}` vale «{dichiarato.get(chiave)}», il "
        f"registro di ASSURANCE-N1 ne conta «{valore}»"
        for chiave, valore in atteso.items()
        if dichiarato.get(chiave) != valore
    ]


def _candidate_legata_alle_fonti(stato: dict[str, Any]) -> list[str]:
    """La candidate si confronta con Cargo.toml e con git, non con se stessa.

    Lo stato dichiarava `tag_creato: false` mentre `v1.0.1` esisteva e puntava
    a `c490f82`: una copia scritta a mano che nessuno confrontava con git.

    Le due domande sul tag e sull'assurance sono cambiate con il modello a due
    revisioni. Non si chiede piu' se il tag stia su HEAD -- pretenderlo rendeva
    la release impossibile -- ma se stia sulla **revisione congelata**; e non si
    chiede se HEAD sia la candidate, ma se cio' che le e' successo dopo stia
    dentro l'allowlist dell'assurance.
    """
    candidate = _dentro(stato, ("aperto", "candidate_release"))
    if not isinstance(candidate, dict):
        return ["`aperto.candidate_release` assente"]

    errori: list[str] = []
    versione = versione_workspace()
    if candidate.get("versione_workspace") != versione:
        errori.append(
            f"`candidate_release.versione_workspace` vale "
            f"«{candidate.get('versione_workspace')}», Cargo.toml dichiara «{versione}»"
        )

    head = revisione_risolta("HEAD")
    dichiarata = candidate.get("revisione_candidate")
    congelata = revisione_risolta(dichiarata)
    if congelata is None:
        errori.append(
            f"`candidate_release.revisione_candidate` vale «{dichiarata}», che "
            "git non risolve a un commit. Una candidate congelata su una "
            "revisione che non esiste non qualifica niente."
        )

    atteso = f"v{candidate.get('versione_manifesto')}"
    if candidate.get("tag_previsto") != atteso:
        errori.append(
            f"`candidate_release.tag_previsto` vale "
            f"«{candidate.get('tag_previsto')}», dalla versione del manifesto "
            f"segue «{atteso}»"
        )

    puntato = _git("rev-parse", "--verify", atteso + "^{commit}")
    if candidate.get("tag_creato") is not (puntato is not None):
        errori.append(
            f"`candidate_release.tag_creato` vale «{candidate.get('tag_creato')}» "
            f"ma git {'trova' if puntato else 'non trova'} il tag «{atteso}»"
        )
    dichiarato = candidate.get("tag_revisione")
    if puntato is None:
        if dichiarato is not None:
            errori.append(
                f"`candidate_release.tag_revisione` vale «{dichiarato}» ma il "
                f"tag «{atteso}» non esiste"
            )
    elif revisione_risolta(dichiarato) != puntato:
        errori.append(
            f"`candidate_release.tag_revisione` vale «{dichiarato}», il tag "
            f"«{atteso}» punta a «{puntato[:7]}»"
        )

    sulla_candidate = (
        puntato is not None and congelata is not None and puntato == congelata
    )
    if candidate.get("tag_sulla_candidate") is not sulla_candidate:
        errori.append(
            f"`candidate_release.tag_sulla_candidate` vale "
            f"«{candidate.get('tag_sulla_candidate')}» ma il tag punta a "
            f"«{(puntato or 'nessun commit')[:7]}» e la candidate e' congelata "
            f"su «{(congelata or 'nessun commit')[:7]}»"
        )

    # `assurance_entro_l_allowlist` e' la forma leggibile della quarta
    # condizione: HEAD e' la candidate stessa, oppure la contiene e non ha
    # toccato altro che cio' che l'assurance produce.
    entro = False
    if congelata is not None and head is not None:
        if head == congelata:
            entro = True
        elif _discende_da(congelata, head):
            fuori, guasti = cambiamenti_dopo_il_congelamento(congelata)
            entro = not fuori and not guasti
    if candidate.get("assurance_entro_l_allowlist") is not entro:
        errori.append(
            f"`candidate_release.assurance_entro_l_allowlist` vale "
            f"«{candidate.get('assurance_entro_l_allowlist')}»: HEAD e' "
            f"«{(head or '')[:7]}», la candidate e' congelata su "
            f"«{(congelata or '')[:7]}», e la diff fra le due "
            f"{'sta' if entro else 'non sta'} dentro l'allowlist "
            "dell'assurance"
        )
    return errori


def _registro_legato(stato: dict[str, Any]) -> list[str]:
    """I blocchi che lo stato ripete sono quelli del registro.

    `release_blocking: true` accanto a una voce, e la parola «aperto» che la
    descrive, erano scritture indipendenti da quelle del registro: potevano
    dire il contrario e nessuno le confrontava.
    """
    errori: list[str] = []
    registro = json.loads(REGISTRO.read_text(encoding="utf-8"))
    stato_di = {v.get("id"): v.get("stato") for v in registro.get("invarianti", [])}
    blocca = {
        identita: valore == "release_blocking" for identita, valore in stato_di.items()
    }

    for percorso, identita in BLOCCANTI_DELLO_STATO.items():
        if identita not in blocca:
            errori.append(f"`{'.'.join(percorso)}`: «{identita}» non e' nel registro")
            continue
        if _dentro(stato, percorso) is not blocca[identita]:
            errori.append(
                f"`{'.'.join(percorso)}` vale «{_dentro(stato, percorso)}», il "
                f"registro dichiara «{identita}» "
                f"{'bloccante' if blocca[identita] else 'non bloccante'}"
            )

    for percorso, (identita, parole) in ETICHETTE_DELLO_STATO.items():
        if identita not in stato_di:
            errori.append(f"`{'.'.join(percorso)}`: «{identita}» non e' nel registro")
            continue
        atteso = parole.get(stato_di[identita])
        if atteso is None:
            errori.append(
                f"`{'.'.join(percorso)}`: il registro dichiara «{identita}» in "
                f"stato «{stato_di[identita]}», e questa etichetta non ha una "
                "parola per quello stato. Prenderla in prestito da un altro "
                "stato direbbe una cosa diversa da quella che il registro dice."
            )
            continue
        if _dentro(stato, percorso) != atteso:
            errori.append(
                f"`{'.'.join(percorso)}` vale «{_dentro(stato, percorso)}», dal "
                f"registro segue «{atteso}»"
            )

    fonte = _dentro(stato, ("blocchi", "fonte"))
    atteso = REGISTRO.relative_to(ROOT).as_posix()
    if fonte != atteso:
        errori.append(f"`blocchi.fonte` vale «{fonte}», il registro e' «{atteso}»")
    return errori


@functools.lru_cache(maxsize=1)
def _censimento() -> tuple[list[str], dict[str, int]]:
    """Il censimento dei costruttori legacy, misurato una volta per processo.

    Dipende dall'albero e non dal documento: ripeterlo per ogni campo che ne
    deriva rifarebbe la stessa scansione di tutti i sorgenti Rust senza
    cambiarne l'esito.
    """
    return censimento_errori(ROOT)


def _censimento_s9_legato(stato: dict[str, Any]) -> list[str]:
    """Il censimento dei costruttori legacy si conta dove viene misurato.

    `componenti_a_zero` e `censimento_costruttori_legacy` erano due numeri
    scritti a mano accanto a un gate che li produce a ogni corsa.
    """
    dichiarato = _dentro(stato, ("chiuso", "s9_errori_strutturati"))
    if not isinstance(dichiarato, dict):
        return ["`chiuso.s9_errori_strutturati` assente"]

    errori: list[str] = []
    problemi, per_crate = _censimento()
    if problemi:
        return [
            "il censimento dei costruttori legacy non e' interpretabile: "
            f"{len(problemi)} anomalie riportate da check_errori_redatti"
        ]
    residui = sum(per_crate.values())

    atteso = {
        "censimento_costruttori_legacy": residui,
        "componenti_a_zero": len(MIGRATI),
        "stato": "chiuso" if residui == 0 else "aperto",
    }
    errori.extend(
        f"`chiuso.s9_errori_strutturati.{chiave}` vale "
        f"«{dichiarato.get(chiave)}», il censimento ne misura «{valore}»"
        for chiave, valore in atteso.items()
        if dichiarato.get(chiave) != valore
    )

    errori.extend(_qualifica_s9_attestata(stato, dichiarato))
    return errori


# Il passo del checkpoint che misura il censimento. La sua presenza nel
# manifest di un'evidenza e' cio' che rende quella corsa un'attestazione della
# chiusura, invece di una corsa che parla d'altro.
LOG_DEL_CENSIMENTO = "check_errori_redatti.log"


def _qualifica_s9_attestata(
    stato: dict[str, Any], dichiarato: dict[str, Any]
) -> list[str]:
    """La chiusura di S9 e' attestata da un'evidenza, non da un'ascendenza.

    Il campo era verificato come **antenato di HEAD**, e l'ascendenza non
    dimostra una qualifica: il commit radice del repository e' antenato di
    tutto, e passava. Qualunque revisione della storia passava.

    Una qualifica e' una **corsa**, e la corsa deve esistere: qui `qualificato_su`
    e' la revisione dell'ultima misura di livello 2 registrata dallo stato, e
    l'evidenza di quella corsa deve contenere il passo che misura il censimento.
    Cosi' la chiusura e' attestata da un artefatto, non da una relazione di
    parentela.
    """
    qualificato = dichiarato.get("qualificato_su")
    risolta = revisione_risolta(qualificato)
    if risolta is None:
        return [
            f"`chiuso.s9_errori_strutturati.qualificato_su` vale "
            f"«{qualificato}», che git non risolve a un commit"
        ]

    misurata = revisione_risolta(_dentro(stato, ("ultima_misura", "sha")))
    if misurata is not None and risolta != misurata:
        return [
            f"`chiuso.s9_errori_strutturati.qualificato_su` dichiara "
            f"«{qualificato}», ma la corsa di livello 2 registrata dallo stato "
            f"e' su «{misurata[:7]}». Una qualifica senza la propria evidenza e' "
            "un'affermazione, e l'ascendenza da HEAD non ne e' una prova."
        ]

    relativo = _dentro(stato, ("ultima_misura", "evidenza"))
    if not isinstance(relativo, str) or not (ROOT / relativo).exists():
        return [
            "`chiuso.s9_errori_strutturati.qualificato_su`: l'evidenza della "
            "corsa che dovrebbe attestarla non e' leggibile"
        ]
    manifest = _dentro(_evidenza(relativo), ("artefatti", "manifest")) or {}
    if LOG_DEL_CENSIMENTO not in manifest:
        return [
            f"«{relativo}» non elenca «{LOG_DEL_CENSIMENTO}» fra gli artefatti: "
            "quella corsa non ha misurato il censimento, quindi non attesta la "
            "chiusura di S9."
        ]
    return []


def _docset_legato(stato: dict[str, Any]) -> list[str]:
    """I conteggi del docset si contano nell'allowlist del docset."""
    dichiarato = _dentro(stato, ("docset",))
    if not isinstance(dichiarato, dict):
        return ["`docset` assente"]
    atteso = {
        "markdown_canonici": len(CANONICI),
        "markdown_operativi": len(OPERATIVI),
        "verificato_da": "scripts/check_docset.py",
    }
    errori = [
        f"`docset.{chiave}` vale «{dichiarato.get(chiave)}», l'allowlist di "
        f"check_docset ne dichiara «{valore}»"
        for chiave, valore in atteso.items()
        if dichiarato.get(chiave) != valore
    ]
    if not (ROOT / "scripts" / "check_docset.py").exists():
        errori.append("`docset.verificato_da`: il gate citato non esiste")
    return errori


def _profondita_legata(stato: dict[str, Any]) -> list[str]:
    """La profondita' del fuzzing si conta dove viene misurata.

    `requisiti_di_profondita` era il posto piu' facile in cui scrivere un numero
    piu' bello di quello vero: nessuno lo confrontava con la misura, e la misura
    non e' leggibile a occhio. Qui il numero viene dall'artefatto, e il percorso
    dell'artefatto dal registro dei requisiti -- cosi' non e' nemmeno possibile
    puntare lo stato a un file diverso da quello che il gate legge.
    """
    dichiarato = _dentro(stato, ("chiuso", "fuzz_reader_shapefile"))
    if not isinstance(dichiarato, dict):
        return ["`chiuso.fuzz_reader_shapefile` assente"]

    try:
        registro = json.loads(REGISTRO_DI_PROFONDITA.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as errore:
        return [f"{REGISTRO_DI_PROFONDITA.name}: non leggibile ({errore})"]

    relativo = registro.get("artefatto")
    errori: list[str] = []
    if dichiarato.get("misura") != relativo:
        errori.append(
            f"`chiuso.fuzz_reader_shapefile.misura` vale "
            f"«{dichiarato.get('misura')}», il registro dei requisiti dichiara "
            f"«{relativo}»"
        )
        return errori

    try:
        misura = json.loads((ROOT / relativo).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as errore:
        return [f"{relativo}: non leggibile ({errore})"]

    quanti = len(misura.get("requisiti", []))
    if dichiarato.get("requisiti_di_profondita") != quanti:
        errori.append(
            f"`chiuso.fuzz_reader_shapefile.requisiti_di_profondita` vale "
            f"«{dichiarato.get('requisiti_di_profondita')}», la misura ne porta "
            f"{quanti}"
        )
    return errori


def _profondita_filegdb_legata(stato: dict[str, Any]) -> list[str]:
    """I numeri del FileGDB si contano dove vengono misurati.

    Sono quattro, e nessuno e' leggibile a occhio: quanti requisiti di
    profondita' la misura porta, quanti contatori di copertura ha il binario,
    quanti file sorgente di GDAL sono strumentati, e dove stanno le due misure.
    Un numero che nessuno confronta e' il numero che prima o poi si scrive da
    solo -- e qui il piu' comodo da scrivere sarebbe proprio quello che dice che
    GDAL e' coperto.
    """
    dichiarato = _dentro(stato, ("chiuso", "fuzz_filegdb"))
    if not isinstance(dichiarato, dict):
        return ["`chiuso.fuzz_filegdb` assente"]

    errori: list[str] = []
    try:
        registro = json.loads(
            REGISTRO_DI_PROFONDITA_FILEGDB.read_text(encoding="utf-8")
        )
    except (OSError, json.JSONDecodeError) as errore:
        return [f"{REGISTRO_DI_PROFONDITA_FILEGDB.name}: non leggibile ({errore})"]

    relativo = registro.get("artefatto")
    if dichiarato.get("misura") != relativo:
        return [
            f"`chiuso.fuzz_filegdb.misura` vale «{dichiarato.get('misura')}», il "
            f"registro dei requisiti dichiara «{relativo}»"
        ]
    try:
        misura = json.loads((ROOT / relativo).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as errore:
        return [f"{relativo}: non leggibile ({errore})"]

    quanti = len(misura.get("requisiti", []))
    if dichiarato.get("requisiti_di_profondita") != quanti:
        errori.append(
            f"`chiuso.fuzz_filegdb.requisiti_di_profondita` vale "
            f"«{dichiarato.get('requisiti_di_profondita')}», la misura ne porta "
            f"{quanti}"
        )

    atteso_asan = MISURA_ASAN_FILEGDB.relative_to(ROOT).as_posix()
    if dichiarato.get("confine_asan") != atteso_asan:
        errori.append(
            f"`chiuso.fuzz_filegdb.confine_asan` vale "
            f"«{dichiarato.get('confine_asan')}», la misura del confine sta in "
            f"«{atteso_asan}»"
        )
        return errori
    try:
        asan = json.loads(MISURA_ASAN_FILEGDB.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as errore:
        return errori + [f"{atteso_asan}: non leggibile ({errore})"]

    for chiave in (
        "contatori_di_copertura",
        "file_sorgente_gdal_strumentati",
        # Il numero che dice se GDAL e' strumentata. E' il piu' facile da
        # scrivere sbagliato, perche' zero e' cio' che ci si aspetta.
        "simboli_asan_nella_libreria",
    ):
        if dichiarato.get(chiave) != asan.get(chiave):
            errori.append(
                f"`chiuso.fuzz_filegdb.{chiave}` vale «{dichiarato.get(chiave)}», "
                f"la misura del confine dice «{asan.get(chiave)}»"
            )
    return errori


def _forma_legata(stato: dict[str, Any]) -> list[str]:
    """Schema e baseline documentale."""
    errori: list[str] = []
    if stato.get("schema_version") != SCHEMA_VERSIONE:
        errori.append(
            f"`schema_version` vale «{stato.get('schema_version')}», attesa "
            f"{SCHEMA_VERSIONE}"
        )
    baseline = _dentro(stato, ("revisioni", "baseline_documentale", "sha"))
    if baseline != BASELINE_DOCSET:
        errori.append(
            f"`revisioni.baseline_documentale.sha` vale «{baseline}», la "
            f"baseline del docset e' «{BASELINE_DOCSET}»"
        )
    return errori


def _foglie(nodo: Any, prefisso: tuple[str, ...] = ()) -> list[tuple[str, ...]]:
    """Ogni percorso terminale del documento. Una lista e' una foglia."""
    if isinstance(nodo, dict):
        trovate: list[tuple[str, ...]] = []
        for chiave, valore in nodo.items():
            trovate.extend(_foglie(valore, prefisso + (chiave,)))
        return trovate
    return [prefisso]


def _classificazione(stato: dict[str, Any]) -> list[str]:
    """Ogni foglia dello stato e' legata a una fonte, o dichiarata non derivabile.

    E' la parte che mancava, e la sua assenza rendeva falsa la promessa scritta
    nel registro. Il validatore verificava tre famiglie di campi; le altre
    stavano nel documento senza che nulla le guardasse — portare
    `componenti_a_zero` a 999 non produceva un solo errore.

    Classificare **tutte** le foglie chiude anche il caso futuro: un campo
    aggiunto domani e non collegato e' rosso, invece di entrare in silenzio
    sotto una promessa che non lo copre.
    """
    presenti = {".".join(percorso) for percorso in _foglie(stato)}
    dichiarate = set(FOGLIE_DICHIARATE)
    errori = [
        f"`{foglia}`: foglia non classificata. O la si lega a una fonte che la "
        "riscrive, o la si dichiara in `FOGLIE_DICHIARATE` con la ragione per "
        "cui non ne ha una. La promessa «ogni numero viene dalla sua fonte» non "
        "si allarga da sola a cio' che qualcuno aggiunge."
        for foglia in sorted(presenti - FOGLIE_LEGATE - dichiarate)
    ]
    errori.extend(
        f"`{foglia}`: dichiarata ma assente dallo stato. Una classificazione "
        "che descrive un campo che non c'e' piu' non classifica niente."
        for foglia in sorted((FOGLIE_LEGATE | dichiarate) - presenti)
    )
    return errori


def validate_stato_corrente(stato: dict[str, Any]) -> list[str]:
    """`assurance/current-state.json` non e' una fonte: e' una **giunzione**.

    Riporta numeri che vivono altrove — l'evidenza di una corsa, il registro di
    ASSURANCE-N1, `Cargo.toml`, git — e li rende a `docs/RELEASE.md` con la
    stessa autorita' con cui li renderebbe la fonte. Erano ricopiati a mano: una
    cifra sbagliata nella copia era indistinguibile da una misura diversa, e la
    copia sopravviveva alla fonte.

    Qui ogni valore copiato viene **riconfrontato con la propria fonte**. Lo
    stato resta il posto dove i numeri stanno insieme; smette di essere il posto
    dove possono divergere.
    """
    return (
        _classificazione(stato)
        + _sola_evidenza_corrente(stato)
        + _forma_legata(stato)
        + _misura_legata_all_evidenza(stato)
        + _conteggi_n1_legati_al_registro(stato)
        + _candidate_legata_alle_fonti(stato)
        + _registro_legato(stato)
        + _censimento_s9_legato(stato)
        + _profondita_legata(stato)
        + _profondita_filegdb_legata(stato)
        + _docset_legato(stato)
        + _registro_corrente_ben_formato(stato)
    )


def _record_di_impatto(documento: dict[str, Any], campo: str) -> list[str]:
    """Un campo additivo senza il suo record d'impatto non e' additivo: e' nuovo.

    La regola del contratto dice `add_optional_field: allowed_with_change_impact_record`.
    Senza questo controllo la prima meta' della regola valeva e la seconda no --
    il campo si aggiungeva e il record restava una buona intenzione. Il record
    deve dire **quale** campo, quale regola invoca e che cosa succede a chi
    legge il catalogo senza conoscerlo.
    """
    voci = documento.get("change_impact")
    if not isinstance(voci, list):
        return [
            "cli-protocol-v1: `change_impact` assente. La regola "
            "`add_optional_field` lo pretende, e un campo aggiunto senza record "
            "e' un campo aggiunto senza avvertire nessuno."
        ]
    for voce in voci:
        if not isinstance(voce, dict) or voce.get("field") != campo:
            continue
        mancanti = [
            chiave
            for chiave in ("kind", "protocol_version", "impatto_sui_consumatori")
            if not voce.get(chiave)
        ]
        if mancanti:
            return [
                f"cli-protocol-v1: il record d'impatto di `{campo}` non dice "
                f"{mancanti}. Un record che non dice l'impatto e' una riga."
            ]
        if voce.get("kind") != "add_optional_field":
            return [
                f"cli-protocol-v1: il record di `{campo}` invoca la regola "
                f"«{voce.get('kind')}», che non e' quella che consente "
                "l'aggiunta di un campo opzionale."
            ]
        return []
    return [
        f"cli-protocol-v1: nessun record d'impatto per `{campo}`, che il "
        "contratto elenca fra i campi additivi."
    ]


def validate_cli_protocol_v1(document: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    if document.get("manifest_version") != 1:
        errors.append("cli-protocol-v1: manifest_version inattesa")
    if document.get("component") != "plenora-IO-tools":
        errors.append("cli-protocol-v1: componente inatteso")
    if document.get("protocol_version") != 1:
        errors.append("cli-protocol-v1: protocol_version inattesa")
    if document.get("status") != "frozen_for_1_0":
        errors.append("cli-protocol-v1: stato inatteso")
    if document.get("compatibility_scope") != "cli_json_only":
        errors.append("cli-protocol-v1: superficie non limitata alla CLI JSON")

    rust_api = document.get("rust_api", {})
    if rust_api != {
        "status": "internal_unstable",
        "semver_guarantee": False,
        "crates_publish": False,
        "reason": (
            "R15.4.1 prevede l'estrazione dei tipi di confine da "
            "plenora-io-model; una garanzia pubblica 1.x renderebbe "
            "quell'estrazione una rottura."
        ),
    }:
        errors.append("cli-protocol-v1: stato API Rust inatteso")

    expected_contracts = {
        "error": "plenora-io-error-v1",
        "catalog": "plenora-io-catalog-v1",
        "inspect": "plenora-io-inspect-v1",
        "layers": "plenora-io-layers-v1",
        "read": "plenora-io-read-v1",
        "convert": "plenora-io-convert-v1",
    }
    envelopes = document.get("envelopes", {})
    if set(envelopes) != set(expected_contracts):
        errors.append("cli-protocol-v1: devono essere dichiarate sei buste")
    for name, contract in expected_contracts.items():
        if envelopes.get(name, {}).get("contract") != contract:
            errors.append(f"cli-protocol-v1: contratto inatteso per {name}")

    error_envelope = envelopes.get("error", {})
    if error_envelope.get("optional_error_fields") != ["row_diagnostics"]:
        errors.append("cli-protocol-v1: campi errore opzionali inattesi")
    if error_envelope.get("row_diagnostics_semantics") != {
        "contract": "plenora-row-diagnostics-v1",
        "present_when": (
            "read_row_scoped_rejections_are_observed_or_write_row_scoped_"
            "rejections_are_observed_after_exact_input_total_declaration"
        ),
        "missing_write_input_total": (
            "contract_precondition_error_without_row_diagnostics"
        ),
        "absent_for_other_errors": True,
    }:
        errors.append("cli-protocol-v1: semantica row diagnostics inattesa")
    if error_envelope.get("emitted_error_codes") != [
        "CANCELLED",
        "INVALID_ROW_DIAGNOSTICS",
    ]:
        errors.append("cli-protocol-v1: token errore emessi inattesi")
    if error_envelope.get("exit_codes") != {
        "data_mapping": 2,
        "cancelled_by_caller": 130,
    }:
        errors.append("cli-protocol-v1: exit code additivi inattesi")

    catalog = envelopes.get("catalog", {})
    # I campi additivi del catalogo, in ordine. `hostile_input_hardened` e'
    # entrato con il lotto S12: la regola `add_optional_field` del contratto lo
    # consente **con un record d'impatto**, e il record e' preteso qui sotto --
    # senza, la regola sarebbe una frase e l'aggiunta un fatto compiuto.
    catalog_fields = [
        "available",
        "hostile_input_hardened",
        "required_feature",
        "spec_version_supported",
    ]
    if catalog.get("optional_driver_fields") != catalog_fields:
        errors.append("cli-protocol-v1: campi catalogo additivi opzionali inattesi")
    if catalog.get("current_producer") != {
        "required_driver_fields": catalog_fields,
    }:
        errors.append("cli-protocol-v1: campi obbligatori del producer corrente inattesi")
    if "required_driver_fields" in catalog:
        errors.append("cli-protocol-v1: producer v1 legacy resi incompatibili")
    if catalog.get("driver_field_semantics") != {
        "available": {
            "type": "boolean",
            "true_when": "runtime_probe_satisfies_descriptor",
        },
        "hostile_input_hardened": {
            "type": "boolean",
            "true_when": (
                "every_geometry_text_the_driver_parses_is_bounded_during_the_parse"
            ),
            "false_means": "not_declared_not_unsafe",
            "nota": (
                "true dice che i tetti del bordo -- byte, componenti, "
                "profondita' -- si applicano mentre il testo viene consumato, "
                "non dopo che l'albero e' stato costruito. false non dice "
                "«insicuro»: dice che questa garanzia non e' dichiarata, e un "
                "driver binario ne ha altre che questo booleano non riassume."
            ),
            "verificata_da": "scripts/check_capability_input_ostile.py",
        },
        "required_feature": {
            "type": ["string", "null"],
            "filegdb": "gdal-backend",
            "other_drivers": None,
        },
        # Entrato con il lotto S10, con la stessa regola e lo stesso record.
        "spec_version_supported": {
            "type": "string_or_null",
            "value_when_declared": (
                "the_highest_specification_version_the_driver_reads_in_full"
            ),
            "null_means": (
                "the_format_is_not_versioned_in_a_way_the_driver_can_declare"
            ),
            "nota": (
                "dice dove il supporto si ferma. Senza, un consumatore che vede "
                "un formato nel catalogo non ha modo di sapere se una versione "
                "successiva della sua specifica sarebbe letta: lo dedurrebbe, e "
                "si sbaglierebbe. Un driver che dichiara una versione rifiuta "
                "quelle oltre con un errore di funzionalita' **non supportata**, "
                "distinto da quello di metadati non conformi: il primo dice che "
                "il file va bene e noi no, il secondo che il file e' sbagliato, "
                "e mandare chi legge a correggere un file corretto e' il danno "
                "che la distinzione evita."
            ),
            "esempio": "geoparquet dichiara 1.1.0: legge 1.0.0 e 1.1.0, e 2.0 no.",
            "verificata_da": "scripts/check_metadati_geoparquet.py",
        },
    }:
        errors.append("cli-protocol-v1: semantica campi driver inattesa")
    errors.extend(_record_di_impatto(document, "drivers[].hostile_input_hardened"))
    errors.extend(_record_di_impatto(document, "drivers[].spec_version_supported"))

    convert = envelopes.get("convert", {})
    required_convert = {
        "conversion_fidelity",
        "read_fidelity",
        "write_fidelity",
        "read_loss",
        "write_loss",
    }
    if not required_convert.issubset(set(convert.get("required_top_level", []))):
        errors.append("cli-protocol-v1: osservabilità convert incompleta")
    if convert.get("forbidden_legacy_fields") != ["loss"]:
        errors.append("cli-protocol-v1: campo legacy loss non vietato")
    return errors


def main(argv: list[str] | None = None) -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument(
        "--release",
        action="store_true",
        help="rossa se una sola condizione dell'autorizzazione non e' soddisfatta",
    )
    opzioni = argomenti.parse_args(argv)

    if not REGISTRO.exists():
        print(f"{REGISTRO}: registro assente.", file=sys.stderr)
        return 2
    documento = json.loads(REGISTRO.read_text(encoding="utf-8"))

    # `completezza` viene per prima: se il registro e' vuoto o mutilato, gli
    # errori delle singole voci descriverebbero cio' che resta invece della
    # causa.
    errori = completezza(documento)
    if not errori:
        errori = struttura(documento)
    if not errori:
        errori = esegui(documento)

    for messaggio in errori:
        print(messaggio, file=sys.stderr)
    if errori:
        return 1

    bloccanti = debito(documento)
    totali = len(documento["invarianti"])
    if opzioni.release:
        # Le condizioni sono **congiunte** e vengono eseguite tutte, anche dopo
        # la prima che fallisce: fermarsi darebbe un elenco parziale di cio'
        # che manca, e chi legge crederebbe che il resto sia a posto.
        mancate: list[str] = []
        for condizione in documento["autorizzazione_di_release"]["condizioni"]:
            motivi = verifica_condizione(condizione, documento)
            for motivo in motivi:
                print(f"{condizione['id']}: {motivo}", file=sys.stderr)
            if motivi:
                mancate.append(f"{condizione['id']}: {condizione['descrizione']}")

        if mancate:
            print("", file=sys.stderr)
            print("release non autorizzabile:", file=sys.stderr)
            for motivo in mancate:
                print(f"  - {motivo}", file=sys.stderr)
            print("", file=sys.stderr)
            print(
                "Le condizioni sono congiunte: nessuna implica le altre, e un "
                "verde parziale non e' un verde.",
                file=sys.stderr,
            )
            return 1
        differite = [
            v for v in documento["invarianti"] if v.get("stato") == "differita"
        ]
        print(
            f"release autorizzabile: {totali} invarianti, nessun blocco, "
            f"{len(documento['autorizzazione_di_release']['condizioni'])} "
            "condizioni verificate."
        )
        # Autorizzare non e' verificare tutto: le differite restano non
        # verificate, e la riga che dice «autorizzabile» e' l'ultima in cui
        # convenga tacerlo.
        for voce in differite:
            print(
                f"  DIFFERITO {voce['id']}: NON verificato. "
                f"{voce['differita']['non_promette']}"
            )
        return 0

    # I verificati si **contano**, non si ricavano per differenza. Finche' gli
    # stati erano due, `totali - bloccanti` era esatto; con il terzo, la stessa
    # sottrazione ha riportato la voce differita fra i verificati -- cioe' ha
    # detto «verificata» proprio della sola voce che il registro dichiara non
    # verificata. Una riga di sommario e' dove si guarda per sapere come sta il
    # contratto, ed e' il posto peggiore in cui tenere quella confusione.
    verificati = [v for v in documento["invarianti"] if v.get("stato") == "verified"]
    differite = [v for v in documento["invarianti"] if v.get("stato") == "differita"]
    print(
        f"contratto corrente coerente: {totali} invarianti, "
        f"{len(verificati)} verificati, {len(bloccanti)} bloccanti, "
        f"{len(differite)} differiti."
    )
    print("  Le prove dei verificati sono state ESEGUITE: gate con exit 0,")
    print("  test elencati dal harness una volta sola e passati. Non dice")
    print("  che la release sia autorizzabile: per quello serve --release.")
    for voce in differite:
        print(
            f"  DIFFERITO {voce['id']}: non richiesto da questa release e NON "
            f"verificato. {voce['differita']['non_promette']}"
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
