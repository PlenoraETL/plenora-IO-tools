#!/usr/bin/env bash
# Sonde del checkpoint di livello 2 (INFRA-5).
#
# Il checkpoint e' il gate che decide se una revisione ha superato S9, ed e'
# stato scritto senza sonde. Ha avuto tre difetti in due corse:
#
#   1. stampava «ROSSO (exit 0)» -- un `if` con condizione falsa e senza `else`
#      restituisce 0, quindi `$?` letto dopo il `fi` non era mai quello del
#      comando;
#   2. i passi che dipendono dalla misura di copertura giravano anche quando la
#      misura era fallita, leggendo i dati di profiling della corsa precedente:
#      `coverage_soglia` ha detto «verde» su un albero che non era quello
#      dichiarato;
#   3. la diagnostica differenziale calcolava una percentuale sullo stesso
#      report stantio.
#
# Nessuno dei tre sarebbe sopravvissuto a una sonda negativa. Un gate che non si
# puo' provare e' un gate di cui ci si fida perche' non lo si e' mai visto
# sbagliare.
#
# Le sonde caricano **solo le funzioni** del checkpoint, con
# `S9_CHECKPOINT_SOLO_FUNZIONI=1`: eseguirlo per intero costerebbe un'ora e
# proverebbe altro.

set -u

RADICE="$(cd "$(dirname "$0")/.." && pwd)"
S9_CHECKPOINT_LOG_DIR="$(mktemp -d)"
export S9_CHECKPOINT_LOG_DIR
trap 'rm -rf "${S9_CHECKPOINT_LOG_DIR}"' EXIT

S9_CHECKPOINT_SOLO_FUNZIONI=1
export S9_CHECKPOINT_SOLO_FUNZIONI
# shellcheck source=/dev/null
. "${RADICE}/scripts/s9-checkpoint.sh"

sonde=0
rosse=0
USCITA="${S9_CHECKPOINT_LOG_DIR}/.uscita"
LETTA=""
ESITO=0

# Esegue nella shell corrente e cattura l'uscita su file.
#
# Con `$(...)` la funzione girerebbe in una **subshell** e le sue mutazioni dei
# contatori sparirebbero -- che e' proprio cio' che queste sonde devono
# osservare. La prima stesura lo faceva, e quattro sonde su dodici misuravano
# la subshell invece del gate.
esegui() {
    "$@" > "${USCITA}" 2>&1
    ESITO=$?
    LETTA="$(cat "${USCITA}")"
}

verifica() {
    local descrizione="$1" atteso="$2" ottenuto="$3"
    sonde=$((sonde + 1))
    if [ "${atteso}" = "${ottenuto}" ]; then
        printf '  ok    %s\n' "${descrizione}"
        return 0
    fi
    printf '  ROSSA %s: atteso «%s», ottenuto «%s»\n' "${descrizione}" "${atteso}" "${ottenuto}"
    rosse=$((rosse + 1))
}

echo "sonde del checkpoint S9"

# --- il verso positivo ------------------------------------------------------
passi=0
verdi=0
falliti=()
esegui passo comando_che_riesce true
verifica "un comando che riesce e' verde" "verde" "$(echo "${LETTA}" | awk '{print $NF}')"
verifica "un verde incrementa i verdi" "1" "${verdi}"
verifica "un verde non entra fra i falliti" "0" "${#falliti[@]}"

# --- il verso negativo: e' la sonda che i tre difetti richiedevano ----------
passi=0
verdi=0
falliti=()
esegui passo comando_che_fallisce sh -c 'exit 17'
esito_passo="${ESITO}"

verifica "un fallimento riporta l'exit code vero" \
    "ROSSO (exit 17)" \
    "$(echo "${LETTA}" | grep -o 'ROSSO (exit [0-9]*)')"
verifica "un fallimento non incrementa i verdi" "0" "${verdi}"
verifica "un fallimento entra fra i falliti" "1" "${#falliti[@]}"
verifica "passo() restituisce l'esito del comando" "17" "${esito_passo}"

# --- il passo saltato non e' un passo verde ---------------------------------
passi=0
verdi=0
falliti=()
esegui salta coverage_soglia coverage_misura
verifica "un passo saltato e' dichiarato tale" \
    "SALTATO (coverage_misura e' fallito)" \
    "$(echo "${LETTA}" | grep -o 'SALTATO (.*)')"
verifica "un passo saltato non e' verde" "0" "${verdi}"
verifica "un passo saltato entra fra i falliti" "1" "${#falliti[@]}"
verifica "un passo saltato conta fra i passi" "1" "${passi}"

# --- la catena si spezza al primo fallimento --------------------------------
passi=0
verdi=0
falliti=()
catena_rotta=""
esegui passo_in_catena primo true
verifica "il primo passo della catena gira" "verde" "$(echo "${LETTA}" | awk '{print $NF}')"

esegui passo_in_catena secondo sh -c 'exit 5'
verifica "un fallimento rompe la catena" "secondo" "${catena_rotta}"
verifica "il fallimento riporta il suo exit"     "ROSSO (exit 5)"     "$(echo "${LETTA}" | grep -o 'ROSSO (exit [0-9]*)')"

esegui passo_in_catena terzo true
verifica "il passo dopo la rottura non gira"     "SALTATO (secondo e' fallito)"     "$(echo "${LETTA}" | grep -o 'SALTATO (.*)')"
verifica "il passo saltato nomina chi ha rotto la catena, non il primo" \
    "secondo" \
    "$(echo "${LETTA}" | grep -o 'SALTATO ([^ ]*' | cut -d'(' -f2)"
verifica "dopo la rottura i verdi restano uno" "1" "${verdi}"
verifica "i due passi non verdi sono fra i falliti" "2" "${#falliti[@]}"

# Il caso che INFRA-6 chiude: un export che riesce lasciando un file vuoto.
passi=0
verdi=0
falliti=()
catena_rotta=""
VUOTO="${S9_CHECKPOINT_LOG_DIR}/vuoto.info"
: > "${VUOTO}"
esegui passo_in_catena report_non_vuoto test -s "${VUOTO}"
verifica "un report vuoto rompe la catena" "report_non_vuoto" "${catena_rotta}"

# --- il log resta, per intero -----------------------------------------------
passi=0
verdi=0
falliti=()
passo comando_verboso sh -c 'echo prima; echo dettaglio-che-serve; echo dopo; exit 3' \
    > /dev/null 2>&1
righe_log="$(wc -l < "${S9_CHECKPOINT_LOG_DIR}/comando_verboso.log" | tr -d ' ')"
verifica "il log di un rosso e' conservato per intero" "3" "${righe_log}"

# --- la modalita' di livello 1 ----------------------------------------------
#
# La modalita' esiste perche' una batteria composta a mano diverge dal
# checkpoint in silenzio. Se la modalita' stessa non fosse provata, avremmo
# spostato il problema invece di chiuderlo.

# 1. Al livello 1 un passo pesante e' **omesso**, e omesso non e' fallito.
passi=0
verdi=0
omessi=0
falliti=()
LIVELLO=1
esegui passo_pesante misura_costosa true
verifica "livello 1: un passo pesante non entra fra i passi" "0" "${passi}"
verifica "livello 1: e' contato fra gli omessi" "1" "${omessi}"
verifica "livello 1: non e' contato fra i falliti" "0" "${#falliti[@]}"

# 2. Un passo pesante **fallito** al livello 1 non puo' rossare, perche' non
#    gira affatto: e' la proprieta' che rende la modalita' usabile.
passi=0
omessi=0
falliti=()
esegui passo_pesante misura_che_fallirebbe false
verifica "livello 1: un pesante rotto non viene eseguito" "0" "${#falliti[@]}"

# 3. Al livello 2 lo stesso passo gira e rossa. Senza questa sonda la
#    modalita' potrebbe omettere **sempre**, e nessuno se ne accorgerebbe.
passi=0
verdi=0
omessi=0
falliti=()
LIVELLO=2
esegui passo_pesante misura_che_fallisce false
verifica "livello 2: lo stesso passo viene eseguito" "1" "${passi}"
verifica "livello 2: e ne raccoglie il rosso" "1" "${#falliti[@]}"
verifica "livello 2: nessun passo omesso" "0" "${omessi}"

# 4. Un passo **normale** gira in entrambe le modalita': il livello 1 omette
#    fuzz e copertura, non i gate. E' la proprieta' che l'incidente su `fmt`
#    ha reso necessaria.
passi=0
verdi=0
falliti=()
LIVELLO=1
esegui passo gate_qualunque true
verifica "livello 1: un passo normale gira lo stesso" "1" "${verdi}"
LIVELLO=2

echo
if [ "${rosse}" -ne 0 ]; then
    echo "sonde: $((sonde - rosse))/${sonde} — ROSSE: ${rosse}"
    exit 1
fi
echo "sonde: ${sonde}/${sonde}"
