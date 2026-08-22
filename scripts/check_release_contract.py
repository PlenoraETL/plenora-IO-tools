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
import json
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
    analizza_uscita,
    comando_test,
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

ROOT = Path(__file__).resolve().parent.parent
REGISTRO = ROOT / "assurance" / "registries" / "release-contract-current.json"
CLI_PROTOCOL_V1 = ROOT / "release" / "cli-protocol-v1.json"

STATI = {"verified", "release_blocking"}
CAMPI = {"id", "superficie", "invariante", "prova", "stato"}

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
        "fuzz.reader-shapefile",
        "fuzz.filegdb",
        "wire.loss-report",
        "release.candidate-non-valida-per-head",
        "lotto.s10",
        "lotto.s11",
        "lotto.s12",
        "sistema.qualifica-cross-component",
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
        # il registro di ASSURANCE-N1
        "aperto.assurance_n1.gruppi_totali",
        "aperto.assurance_n1.gruppi_aperti",
        # Cargo.toml e git
        "aperto.candidate_release.versione_workspace",
        "aperto.candidate_release.tag_previsto",
        "aperto.candidate_release.tag_creato",
        "aperto.candidate_release.tag_revisione",
        "aperto.candidate_release.tag_su_head",
        "aperto.candidate_release.qualifica_head",
        # il registro del contratto corrente
        "aperto.assurance_n1.release_blocking",
        "aperto.fuzz_reader_shapefile.release_blocking",
        "aperto.fuzz_reader_shapefile.stato",
        "aperto.fuzz_spike_filegdb.release_blocking",
        "aperto.fuzz_spike_filegdb.stato",
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
    "ultima_misura.diagnostica_differenziale.ragione": "prosa",
    "aperto.fuzz_reader_shapefile.nota": "prosa",
    "aperto.fuzz_spike_filegdb.nota": "prosa",
    "aperto.loss_report.decisioni_aperte": (
        "l'elenco delle decisioni aperte del contratto LossReport: non e' "
        "misurato da niente, ed e' cio' che una ratifica dovra' chiudere"
    ),
    "aperto.lotti.perimetri.s10": "prosa: il perimetro del lotto",
    "aperto.lotti.perimetri.s11": "prosa: il perimetro del lotto",
    "aperto.lotti.perimetri.s12": "prosa: il perimetro del lotto",
    "aperto.candidate_release.versione_manifesto": (
        "fatto del manifesto della candidate, che non e' piu' nel repository: "
        "resta in git, e non c'e' una fonte viva che lo riscriva"
    ),
    "aperto.candidate_release.revisione_manifesto": (
        "fatto del manifesto della candidate; vedi `versione_manifesto`"
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
    ("aperto", "fuzz_reader_shapefile", "release_blocking"): "fuzz.reader-shapefile",
    ("aperto", "fuzz_spike_filegdb", "release_blocking"): "fuzz.filegdb",
    ("aperto", "loss_report", "release_blocking"): "wire.loss-report",
    (
        "aperto",
        "candidate_release",
        "release_blocking",
    ): "release.candidate-non-valida-per-head",
}

# Le stesse voci dette a parole: `percorso -> (invariante, parola se blocca,
# parola se non blocca)`. Il vocabolario non e' uniforme — un lotto e' «aperto»
# e una qualifica e' «aperta» — e indovinarlo dal nome sarebbe la scorciatoia
# che rende il legame falso su un caso solo.
ETICHETTE_DELLO_STATO = {
    ("aperto", "lotti", "s10"): ("lotto.s10", "aperto", "chiuso"),
    ("aperto", "lotti", "s11"): ("lotto.s11", "aperto", "chiuso"),
    ("aperto", "lotti", "s12"): ("lotto.s12", "aperto", "chiuso"),
    ("aperto", "lotti", "qualifica_cross_component"): (
        "sistema.qualifica-cross-component",
        "aperta",
        "chiusa",
    ),
    ("aperto", "fuzz_reader_shapefile", "stato"): (
        "fuzz.reader-shapefile",
        "aperto",
        "chiuso",
    ),
    ("aperto", "fuzz_spike_filegdb", "stato"): ("fuzz.filegdb", "aperto", "chiuso"),
    ("aperto", "loss_report", "stato"): (
        "wire.loss-report",
        "non_ratificato",
        "ratificato",
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
        comando = comando_test(crate, configurazione, bersaglio)
        esito = subprocess.run(comando, cwd=ROOT, capture_output=True, text=True, check=False)
        eseguiti, duplicati = analizza_uscita(esito.stdout)
        errori.extend(f"{crate} ({configurazione}, {bersaglio}): {d}" for d in duplicati)
        if not eseguiti:
            errori.append(
                f"{crate} ({configurazione}, {bersaglio}): il harness non ha elencato alcun "
                "test. Un silenzio non e' un verde."
            )
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
    """Il manifesto della candidate descrive **la revisione corrente**.

    Versione del workspace, SHA di HEAD e tag non si leggono dallo stato: si
    leggono da `Cargo.toml` e da git. Uno stato che dichiarasse la coerenza
    senza averla renderebbe la condizione una copia di se stessa.
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

    head = _git("rev-parse", "HEAD")
    revisione = candidate.get("revisione_manifesto")
    if head is None:
        motivi.append("git: HEAD non leggibile")
    elif not isinstance(revisione, str) or not head.startswith(revisione):
        motivi.append(
            f"la candidate e' legata a «{revisione}», HEAD e' «{(head or '')[:7]}»: "
            "quel manifesto non qualifica il codice corrente"
        )

    atteso = f"v{versione}" if versione else None
    puntato = _git("rev-parse", "--verify", atteso + "^{commit}") if atteso else None
    if atteso is None:
        pass
    elif puntato is None:
        motivi.append(f"il tag «{atteso}» non esiste")
    elif head is not None and puntato != head:
        motivi.append(f"il tag «{atteso}» punta a «{puntato[:7]}», non a HEAD")

    if candidate.get("release_action_allowed") is not True:
        motivi.append("`release_action.allowed` non e' consentita")
    return motivi


def condizione_qualifica_cross_component(documento: dict[str, Any]) -> list[str]:
    """L'esito della catena, letto **dall'artefatto dell'owner esterno**.

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


def _misura_legata_all_evidenza(stato: dict[str, Any]) -> list[str]:
    """I numeri dello stato vengono dall'evidenza della corsa che li ha prodotti."""
    errori: list[str] = []
    misura = stato.get("ultima_misura")
    if not isinstance(misura, dict):
        return ["`ultima_misura` assente"]

    relativo = misura.get("evidenza")
    if not isinstance(relativo, str):
        return ["`ultima_misura.evidenza` assente: i numeri non hanno una corsa da cui venire"]
    percorso = ROOT / relativo
    if not percorso.exists():
        return [f"«{relativo}»: evidenza assente"]

    sha = misura.get("sha")
    if not isinstance(sha, str) or sha not in Path(relativo).name:
        errori.append(
            f"«{relativo}» non nomina la revisione misurata «{sha}». Il nome "
            "dell'evidenza e' cio' che lega la corsa alla revisione."
        )

    evidenza = json.loads(percorso.read_text(encoding="utf-8"))
    finale = _dentro(evidenza, ("corsa", "revisione_finale"))
    if not isinstance(finale, str) or not (isinstance(sha, str) and finale.startswith(sha)):
        errori.append(
            f"«{relativo}» descrive la revisione «{finale}», lo stato dichiara "
            f"«{sha}». Un'evidenza vale per l'albero misurato e per nessun altro."
        )

    qualificata = _dentro(stato, ("revisioni", "ultima_qualificata", "sha"))
    if "level 2 passed" in str(evidenza.get("esito", "")) and qualificata != sha:
        errori.append(
            f"`revisioni.ultima_qualificata` dice «{qualificata}» ma l'ultima "
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

    head = _git("rev-parse", "HEAD")
    revisione = candidate.get("revisione_manifesto")
    su_head = bool(head and isinstance(revisione, str) and head.startswith(revisione))
    if candidate.get("qualifica_head") is not su_head:
        errori.append(
            f"`candidate_release.qualifica_head` vale "
            f"«{candidate.get('qualifica_head')}»: HEAD e' «{(head or '')[:7]}» e "
            f"il manifesto e' legato a «{revisione}»"
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
    corta = puntato[:7] if puntato else None
    if candidate.get("tag_revisione") != corta:
        errori.append(
            f"`candidate_release.tag_revisione` vale "
            f"«{candidate.get('tag_revisione')}», il tag «{atteso}» punta a «{corta}»"
        )
    if candidate.get("tag_su_head") is not (puntato is not None and puntato == head):
        errori.append(
            f"`candidate_release.tag_su_head` vale "
            f"«{candidate.get('tag_su_head')}» ma il tag punta a «{corta}» e HEAD "
            f"e' «{(head or '')[:7]}»"
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
    blocca = {
        v.get("id"): v.get("stato") == "release_blocking"
        for v in registro.get("invarianti", [])
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

    for percorso, (identita, se_blocca, se_no) in ETICHETTE_DELLO_STATO.items():
        if identita not in blocca:
            errori.append(f"`{'.'.join(percorso)}`: «{identita}» non e' nel registro")
            continue
        atteso = se_blocca if blocca[identita] else se_no
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

    qualificato = dichiarato.get("qualificato_su")
    if not isinstance(qualificato, str) or not qualificato:
        errori.append("`chiuso.s9_errori_strutturati.qualificato_su` assente")
    elif not _git_riesce("merge-base", "--is-ancestor", qualificato, "HEAD"):
        errori.append(
            f"`chiuso.s9_errori_strutturati.qualificato_su` dichiara "
            f"«{qualificato}», che git non riconosce come antenato di HEAD. Una "
            "qualifica su una revisione che non e' nella storia corrente non "
            "qualifica niente."
        )
    return errori


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
        + _forma_legata(stato)
        + _misura_legata_all_evidenza(stato)
        + _conteggi_n1_legati_al_registro(stato)
        + _candidate_legata_alle_fonti(stato)
        + _registro_legato(stato)
        + _censimento_s9_legato(stato)
        + _docset_legato(stato)
    )


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
    catalog_fields = ["available", "required_feature"]
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
        "required_feature": {
            "type": ["string", "null"],
            "filegdb": "gdal-backend",
            "other_drivers": None,
        },
    }:
        errors.append("cli-protocol-v1: semantica campi driver inattesa")

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
        print(
            f"release autorizzabile: {totali} invarianti, nessun blocco, "
            f"{len(documento['autorizzazione_di_release']['condizioni'])} "
            "condizioni verificate."
        )
        return 0

    print(
        f"contratto corrente coerente: {totali} invarianti, "
        f"{totali - len(bloccanti)} verificati, {len(bloccanti)} bloccanti."
    )
    print("  Le prove dei verificati sono state ESEGUITE: gate con exit 0,")
    print("  test elencati dal harness una volta sola e passati. Non dice")
    print("  che la release sia autorizzabile: per quello serve --release.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
