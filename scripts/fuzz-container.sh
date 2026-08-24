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
#
# `wait` attende e lascia il container in piedi: si puo' richiamare piu' volte,
# ed e' il modo di riprendere dopo un'interruzione del client. `collect`
# attende, stampa l'esito, **poi** rimuove — ed e' l'unico comando che rimuove.
set -uo pipefail

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

esiste() {
    docker container inspect "${NOME}" >/dev/null 2>&1
}

in_esecuzione() {
    [ "$(docker container inspect -f '{{.State.Running}}' "${NOME}" 2>/dev/null)" = "true" ]
}

esito() {
    docker container inspect -f '{{.State.ExitCode}}' "${NOME}" 2>/dev/null
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
    echo "avvio ${modalita} in '${NOME}' (immagine ${IMMAGINE})"
    MSYS_NO_PATHCONV=1 docker run --detach --name "${NOME}" \
        --volume "${radice}:/work" \
        --volume "${VOLUME_CARGO}:/usr/local/cargo/registry" \
        --volume "${VOLUME_TARGET}:/fuzztarget" \
        --env PLENORA_FUZZ_TARGET_DIR=/fuzztarget \
        "${IMMAGINE}" bash "${script}" "$@" >/dev/null || return 1
    echo "avviato: segui con 'status', 'logs', 'wait'"
}

comando_status() {
    if ! esiste; then
        echo "nessun container '${NOME}'"
        return 3
    fi
    if in_esecuzione; then
        echo "in esecuzione da $(docker container inspect -f '{{.State.StartedAt}}' "${NOME}")"
        return 3
    fi
    local codice
    codice="$(esito)"
    echo "terminato con exit ${codice}"
    return "${codice}"
}

comando_logs() {
    local righe="${1:-40}"
    if ! esiste; then
        echo "nessun container '${NOME}'" >&2
        return 2
    fi
    docker container logs "${NOME}" 2>&1 | tail -n "${righe}"
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
    echo "--- coda del log ---"
    docker container logs "${NOME}" 2>&1 | tail -n 20
    docker container rm "${NOME}" >/dev/null
    echo "--- container rimosso, esito acquisito: ${codice} ---"
    return "${codice}"
}

comando_stop() {
    if ! esiste; then
        echo "nessun container '${NOME}'"
        return 0
    fi
    docker container rm --force "${NOME}" >/dev/null
    echo "container '${NOME}' rimosso senza leggerne l'esito"
}

case "${1:-}" in
    start) shift; comando_start "$@" ;;
    status) comando_status ;;
    logs) shift; comando_logs "$@" ;;
    wait) shift; comando_wait "$@" ;;
    collect) shift; comando_collect "$@" ;;
    stop) comando_stop ;;
    *)
        echo "uso: $0 {start replay|start smoke|status|logs|wait|collect|stop} [argomenti]" >&2
        exit 2
        ;;
esac
