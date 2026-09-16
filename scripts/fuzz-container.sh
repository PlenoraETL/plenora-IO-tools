#!/bin/bash
# Wrapper host-side per eseguire replay e smoke dentro l'immagine di sviluppo,
# in modo che l'esecuzione **sopravviva** al client che l'ha avviata.
#
# ## Perche' esiste
#
# `scripts/fuzz-replay.sh` e `scripts/fuzz-smoke.sh` restano quello che sono:
# script foreground, invariati, ed e' la modalita' con cui girano in CI, dove
# il runner aspetta il processo e ne raccoglie l'esito. Questo file non li
# sostituisce e non li chiama in modo diverso — li lancia dentro un container
# **staccato**.
#
# La differenza conta quando il client ha un tetto di durata piu' corto della
# corsa. Uno smoke da quattordici target a sessanta secondi l'uno, piu' la build
# strumentata, supera i dieci minuti; un replay dell'intero corpus li ha
# superati due volte. In quei casi il client viene interrotto, e con lui il
# processo `docker run` in primo piano: il container muore, l'esecuzione e'
# persa, e — questa e' la parte che conta — **non c'e' nessun exit code**.
#
# Un'esecuzione interrotta senza exit code non e' verde e non e' rossa: non e'
# un esito. Trattarla come verde perche' «non si vedevano crash nel log» e' il
# modo in cui una campagna di fuzzing smette di dire qualcosa.
#
# ## Le tre proprieta' che il wrapper garantisce
#
# 1. **Il container sopravvive al client.** `docker run -d`: il ciclo di vita
#    e' del demone, non del terminale.
# 2. **L'exit code viene da Docker.** `docker inspect -f '{{.State.ExitCode}}'`,
#    mai dal log, mai da una pipe. Un `| tail` che restituisce zero mentre il
#    comando a monte fallisce e' un errore che questo repository ha gia' fatto,
#    ed e' registrato.
# 3. **Il container non viene rimosso prima di aver acquisito l'esito.** Niente
#    `--rm`: la rimozione e' esplicita e avviene **dopo** la lettura, in
#    `collect`. Un container rimosso automaticamente porta via con se' l'unica
#    fonte dell'esito.
# 5. **La corsa misura una revisione, e quella resta ferma.** Lo SHA viene
#    inciso nell'etichetta all'avvio, e `status` lo confronta con l'albero
#    corrente: senza, si diagnostica la corsa sbagliata, ed e' un errore gia'
#    fatto in questo ciclo.
#
#    L'etichetta pero' **non bastava**. Il container montava il checkout vivo:
#    una modifica durante la campagna entrava nelle compilazioni successive
#    mentre l'etichetta conservava lo SHA iniziale -- cioe' l'attribuzione
#    diventava falsa proprio dove sembrava piu' solida. La corsa gira ora su un
#    **clone isolato** della revisione, e l'albero di lavoro puo' muoversi senza
#    toccarla.
#
#    Isolati i sorgenti, non gli esiti: `fuzz/corpus`, `fuzz/artifacts` e
#    `assurance/evidence` restano montati dall'albero vivo. Il corpus e' un
#    ingresso che la campagna accresce, e gli altri due sono cio' che produce:
#    isolarli vorrebbe dire buttarli via alla fine.
# 4. **Il log sopravvive al container.** `collect` lo scrive **per intero** su
#    disco prima di rimuovere, e se non riesce a scriverlo **non rimuove**.
#    Prima ne stampava venti righe e poi cancellava: l'esito restava, il
#    racconto di come ci si era arrivati no. E' successo davvero -- una
#    campagna interrotta i cui log erano solo dentro il container -- ed e' la
#    ragione per cui questa proprieta' e' scritta qui e non lasciata a chi si
#    ricorda di salvarli.
#
# ## Uso
#
#   scripts/fuzz-container.sh start replay [target ...]
#   scripts/fuzz-container.sh start smoke [--seconds N] [target ...]
#   scripts/fuzz-container.sh status
#   scripts/fuzz-container.sh logs [righe]
#   scripts/fuzz-container.sh wait [secondi-di-attesa-massimi]
#   scripts/fuzz-container.sh collect [secondi-di-attesa-massimi]
#   scripts/fuzz-container.sh stop
#   scripts/fuzz-container.sh scarta
#
# `wait` attende e lascia il container in piedi: si puo' richiamare piu' volte,
# ed e' il modo di riprendere dopo un'interruzione del client. `collect`
# attende, **salva il log intero**, stampa l'esito, poi rimuove — ed e' l'unico
# comando che rimuove. La destinazione del log si sceglie con
# `PLENORA_FUZZ_LOG_DIR`; per difetto e' `campagne-log/` accanto al repository.
set -uo pipefail

# `docker` passa da una variabile per una ragione sola: le prove sostituiscono
# un finto al suo posto. Un wrapper che si puo' esercitare solo avendo Docker,
# un'immagine e una campagna vera non ha prove, e infatti non ne aveva.
DOCKER="${PLENORA_DOCKER:-docker}"

# Dove finiscono i log delle campagne. Fuori dal repository per difetto: un log
# di campagna e' un artefatto di corsa, non materiale versionato, e metterlo
# dentro l'albero lo farebbe comparire nei gate che contano cio' che c'e'.
DIRECTORY_LOG="${PLENORA_FUZZ_LOG_DIR:-}"

NOME="${PLENORA_FUZZ_CONTAINER:-plenora-fuzz}"
IMMAGINE="${PLENORA_FUZZ_IMAGE:-plenora-io-dev}"
VOLUME_CARGO="${PLENORA_FUZZ_CARGO_VOLUME:-plenora-io-cargo}"
VOLUME_TARGET="${PLENORA_FUZZ_TARGET_VOLUME:-plenora-io-fuzztarget}"

radice_repo() {
    local qui
    qui="$(cd "$(dirname "$0")/.." && pwd)"
    # Su Git Bash `docker` vuole un percorso Windows: `/c/Users/...` verrebbe
    # riscritto in `C:\Users\...` dalla conversione automatica di MSYS, che
    # rompe il bind mount. `pwd -W` da' la forma che Docker accetta; altrove
    # non esiste e il percorso POSIX va bene com'e'.
    (cd "${qui}" && pwd -W 2>/dev/null) || echo "${qui}"
}

# Dove vive il clone isolato della corsa. Sotto `campagne-log/`, che e' gia'
# ignorata: tutto cio' che una campagna produce sta in un posto solo.
directory_checkout() {
    local base="${PLENORA_FUZZ_CHECKOUT_DIR:-}"
    if [ -z "${base}" ]; then
        base="$(cd "$(dirname "$0")/.." && pwd)/campagne-log"
    fi
    echo "${base}/checkout-${1:0:12}"
}

# Prepara il clone della revisione e ne stampa il percorso, o fallisce.
#
# `--no-hardlinks` e non un worktree: un worktree porta un `.git` che rimanda al
# repository principale con un percorso **dell'host**, e dentro il container
# quel percorso non esiste. Il clone e' autonomo -- 69 MB e cinque secondi su
# questo repository -- e i gate che chiamano git funzionano dentro come fuori.
prepara_checkout() {
    local revisione="$1"
    local destinazione
    destinazione="$(directory_checkout "${revisione}")"
    if [ -d "${destinazione}/.git" ]; then
        echo "${destinazione}"
        return 0
    fi
    rm -rf "${destinazione}"
    mkdir -p "$(dirname "${destinazione}")" || return 1
    local sorgente
    sorgente="$(cd "$(dirname "$0")/.." && pwd)"
    git clone --local --no-hardlinks --quiet "${sorgente}" "${destinazione}" || return 1
    git -C "${destinazione}" checkout --detach --quiet "${revisione}" || return 1
    echo "${destinazione}"
}

esiste() {
    "${DOCKER}" container inspect "${NOME}" >/dev/null 2>&1
}

in_esecuzione() {
    [ "$("${DOCKER}" container inspect -f '{{.State.Running}}' "${NOME}" 2>/dev/null)" = "true" ]
}

esito() {
    "${DOCKER}" container inspect -f '{{.State.ExitCode}}' "${NOME}" 2>/dev/null
}

comando_start() {
    local modalita="${1:?modalita: replay | smoke}"
    shift
    local script
    case "${modalita}" in
        replay) script="scripts/fuzz-replay.sh" ;;
        smoke) script="scripts/fuzz-smoke.sh" ;;
        *)
            echo "modalita' sconosciuta: ${modalita} (attese: replay, smoke)" >&2
            return 2
            ;;
    esac

    # Un container gia' presente non viene sovrascritto: potrebbe essere una
    # corsa viva, o una finita di cui nessuno ha ancora letto l'esito.
    # Rimuoverlo per far posto significherebbe buttare via proprio la cosa che
    # questo wrapper esiste per conservare.
    if esiste; then
        if in_esecuzione; then
            echo "container '${NOME}' gia' in esecuzione: usa 'wait' o 'stop'" >&2
        else
            echo "container '${NOME}' fermo con esito $(esito): leggilo con 'collect'" >&2
        fi
        return 2
    fi

    local radice
    radice="$(radice_repo)"
    # La revisione si incide all'avvio, non si deduce dopo: l'albero puo'
    # muoversi mentre la campagna gira, e `git rev-parse` fra un'ora
    # risponderebbe di un'altra.
    local revisione
    revisione="$(git -C "$(dirname "$0")/.." rev-parse HEAD 2>/dev/null || echo sconosciuta)"
    if [ "${revisione}" = "sconosciuta" ]; then
        echo "git non risolve HEAD: una campagna che non sa che cosa misura non si avvia" >&2
        return 1
    fi
    # Un albero sporco non si isola: il clone parte da HEAD e non porterebbe le
    # modifiche non committate. Misurare una revisione che non contiene il
    # lavoro che si ha davanti e' una diagnosi che va male dopo, non subito.
    if [ -n "$(git -C "$(dirname "$0")/.." status --porcelain 2>/dev/null)" ]; then
        echo "albero di lavoro sporco: committa o metti da parte prima di avviare," >&2
        echo "o la campagna misurerebbe una revisione diversa da cio' che vedi" >&2
        return 1
    fi

    local checkout
    if ! checkout="$(prepara_checkout "${revisione}")"; then
        echo "il clone isolato della revisione non si e' preparato" >&2
        return 1
    fi
    echo "avvio ${modalita} in '${NOME}' su ${revisione:0:12} (immagine ${IMMAGINE})"
    echo "sorgenti isolati in ${checkout}"
    MSYS_NO_PATHCONV=1 "${DOCKER}" run --detach --name "${NOME}" \
        --label "plenora.revisione=${revisione}" \
        --volume "${checkout}:/work" \
        --volume "${radice}/fuzz/corpus:/work/fuzz/corpus" \
        --volume "${radice}/fuzz/artifacts:/work/fuzz/artifacts" \
        --volume "${radice}/assurance/evidence:/work/assurance/evidence" \
        --volume "${VOLUME_CARGO}:/usr/local/cargo/registry" \
        --volume "${VOLUME_TARGET}:/fuzztarget" \
        --env PLENORA_FUZZ_TARGET_DIR=/fuzztarget \
        "${IMMAGINE}" bash "${script}" "$@" >/dev/null || return 1
    echo "avviato: segui con 'status', 'logs', 'wait'"
}

# La revisione incisa all'avvio, o la stringa vuota.
revisione_della_corsa() {
    "${DOCKER}" container inspect -f '{{index .Config.Labels "plenora.revisione"}}' \
        "${NOME}" 2>/dev/null
}

# Dice su che cosa gira la corsa, e se l'albero nel frattempo si e' mosso.
#
# Non e' un errore che si siano mossi: una campagna lunga e un ramo che avanza
# convivono. E' un errore **non saperlo**, e leggere l'esito come se
# riguardasse l'albero che si ha davanti.
riga_della_revisione() {
    local incisa corrente
    incisa="$(revisione_della_corsa)"
    if [ -z "${incisa}" ] || [ "${incisa}" = "sconosciuta" ]; then
        echo "revisione della corsa: NON incisa (container avviato da una versione precedente del wrapper)"
        return 0
    fi
    corrente="$(git -C "$(dirname "$0")/.." rev-parse HEAD 2>/dev/null || echo "")"
    if [ "${incisa}" = "${corrente}" ]; then
        echo "revisione della corsa: ${incisa:0:12}, uguale all'albero corrente"
    else
        echo "revisione della corsa: ${incisa:0:12}, DIVERSA dall'albero corrente (${corrente:0:12}): l'esito non riguarda cio' che hai davanti"
    fi
}

comando_status() {
    if ! esiste; then
        echo "nessun container '${NOME}'"
        return 3
    fi
    if in_esecuzione; then
        echo "in esecuzione da $("${DOCKER}" container inspect -f '{{.State.StartedAt}}' "${NOME}")"
        riga_della_revisione
        return 3
    fi
    local codice
    codice="$(esito)"
    echo "terminato con exit ${codice}"
    # La revisione si dice **anche** qui, ed e' il momento in cui serve di piu':
    # a corsa finita si legge un esito, e un esito senza sapere a che cosa si
    # riferisce e' la diagnosi sbagliata che aspetta di succedere. Stava solo
    # nel ramo «in esecuzione», cioe' dove nessuno conclude niente.
    riga_della_revisione
    return "${codice}"
}

comando_logs() {
    local righe="${1:-40}"
    if ! esiste; then
        echo "nessun container '${NOME}'" >&2
        return 2
    fi
    "${DOCKER}" container logs "${NOME}" 2>&1 | tail -n "${righe}"
}

# `si` solo quando `comando_wait` ha letto un exit code vero dal demone.
#
# Serve perche' il valore di ritorno di `wait` **e'** l'exit code del
# container, e quindi non puo' anche significare «non c'e' nessun container» o
# «l'attesa e' scaduta»: un container che esce con 2 e l'assenza del container
# darebbero lo stesso numero. La prima stesura li confondeva, e la conseguenza
# era che un container fallito con 2 non veniva mai rimosso da `collect`.
#
# E' la stessa classe di errore che questo wrapper esiste per chiudere — un
# esito che significa due cose — e non diventa accettabile per il fatto di
# stare nello strumento invece che nella misura.
ESITO_ACQUISITO="no"

# Attende la fine, senza rimuovere. Ritorna l'exit code del container quando
# c'e'; altrimenti lascia `ESITO_ACQUISITO=no` e ritorna un codice fuori dallo
# spazio degli esiti che ci interessano.
comando_wait() {
    local massimo="${1:-3600}"
    ESITO_ACQUISITO="no"
    if ! esiste; then
        echo "nessun container '${NOME}'" >&2
        return 125
    fi
    local trascorsi=0
    while in_esecuzione; do
        if [ "${trascorsi}" -ge "${massimo}" ]; then
            echo "ancora in esecuzione dopo ${massimo}s: richiama 'wait' per continuare" >&2
            return 124
        fi
        sleep 5
        trascorsi=$((trascorsi + 5))
    done
    local codice
    codice="$(esito)"
    ESITO_ACQUISITO="si"
    echo "terminato con exit ${codice} dopo circa ${trascorsi}s"
    return "${codice}"
}

# Attende, stampa la coda del log e l'esito, **poi** rimuove. E' l'unico
# comando che rimuove, e lo fa solo dopo aver acquisito l'exit code: se
# l'attesa scade il container resta, perche' l'esito non e' ancora noto.
comando_collect() {
    local massimo="${1:-3600}"
    comando_wait "${massimo}"
    local codice=$?
    # La decisione di rimuovere dipende da `ESITO_ACQUISITO`, non dal numero:
    # il numero e' l'esito del container, e usarlo anche come stato del wrapper
    # e' cio' che rendeva 2 ambiguo.
    if [ "${ESITO_ACQUISITO}" != "si" ]; then
        return "${codice}"
    fi
    local destinazione
    if ! destinazione="$(salva_log)"; then
        echo "il log non si e' potuto salvare: il container NON viene rimosso, " >&2
        echo "cosi' resta l'unica copia. Esito acquisito: ${codice}" >&2
        return "${codice}"
    fi
    riga_della_revisione
    echo "--- coda del log ---"
    tail -n 20 "${destinazione}"
    echo "--- log completo in ${destinazione} ---"
    "${DOCKER}" container rm "${NOME}" >/dev/null
    pulisci_checkout
    echo "--- container rimosso, esito acquisito: ${codice} ---"
    return "${codice}"
}

# Scrive il log **intero** su disco e ne stampa il percorso, o fallisce.
#
# Il nome porta la data e il nome del container: due campagne non si
# sovrascrivono, e chi rilegge sa quale corsa sta guardando senza aprirlo.
#
# Fallisce rumorosamente invece di ripiegare su un percorso qualunque. Un
# salvataggio che riesce sempre, da qualche parte, e' la stessa cosa del non
# salvare: nessuno sa dove guardare.
salva_log() {
    local cartella="${DIRECTORY_LOG}"
    if [ -z "${cartella}" ]; then
        cartella="$(cd "$(dirname "$0")/.." && pwd)/campagne-log"
    fi
    mkdir -p "${cartella}" || return 1
    local incisa destinazione
    incisa="$(revisione_della_corsa)"
    [ -n "${incisa}" ] || incisa="sconosciuta"
    # Il nome porta la revisione: un log ritrovato mesi dopo deve dire da solo
    # che cosa misurava, senza dipendere da chi si ricorda di averlo prodotto.
    destinazione="${cartella}/${NOME}-${incisa:0:12}-$(date -u +%Y%m%dT%H%M%SZ).log"
    "${DOCKER}" container logs "${NOME}" > "${destinazione}" 2>&1 || return 1
    [ -s "${destinazione}" ] || [ -f "${destinazione}" ] || return 1
    echo "${destinazione}"
}

# Ferma la corsa e la lascia **raccoglibile**.
#
# Faceva `rm --force`: fermava e distruggeva insieme, senza acquisire l'esito e
# senza salvare il log. Era la via che disfaceva tutto cio' che `collect` era
# stato scritto per conservare, e stava a due righe di distanza.
#
# Fermare e leggere sono due decisioni, e chi ferma una campagna vuole quasi
# sempre sapere com'era andata fin li'. Chi invece vuole buttare via deve dirlo:
# `scarta` esiste per quello, e il suo nome non si digita per sbaglio.
# Rimuove il clone isolato, se c'e'. Si chiama **dopo** aver acquisito l'esito:
# i sorgenti di una corsa servono finche' qualcuno potrebbe volerli rileggere.
pulisci_checkout() {
    local incisa destinazione
    incisa="$(revisione_della_corsa)"
    [ -n "${incisa}" ] || return 0
    destinazione="$(directory_checkout "${incisa}")"
    [ -d "${destinazione}" ] || return 0
    rm -rf "${destinazione}" && echo "sorgenti isolati rimossi: ${destinazione}"
}

comando_stop() {
    if ! esiste; then
        echo "nessun container '${NOME}'"
        return 0
    fi
    if in_esecuzione; then
        "${DOCKER}" container stop "${NOME}" >/dev/null
        echo "container '${NOME}' fermato: l'esito e il log si leggono con 'collect'"
    else
        echo "container '${NOME}' gia' fermo: l'esito e il log si leggono con 'collect'"
    fi
}

# Butta via container e clone senza leggerne l'esito. Esiste perche' la
# decisione di perdere un'evidenza sia **detta**, invece di essere il modo in
# cui `stop` si comportava per difetto.
comando_scarta() {
    if esiste; then
        "${DOCKER}" container rm --force "${NOME}" >/dev/null
        echo "container '${NOME}' rimosso SENZA leggerne l'esito ne' salvarne il log"
    else
        echo "nessun container '${NOME}'"
    fi
    pulisci_checkout
}

case "${1:-}" in
    start) shift; comando_start "$@" ;;
    status) comando_status ;;
    logs) shift; comando_logs "$@" ;;
    wait) shift; comando_wait "$@" ;;
    collect) shift; comando_collect "$@" ;;
    stop) comando_stop ;;
    scarta) comando_scarta ;;
    *)
        echo "uso: $0 {start replay|start smoke|status|logs|wait|collect|stop|scarta} [argomenti]" >&2
        exit 2
        ;;
esac
