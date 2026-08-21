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
esegui passo_pesante fuzz_replay true
verifica "livello 1: un passo pesante non entra fra i passi" "0" "${passi}"
verifica "livello 1: e' contato fra gli omessi" "1" "${omessi}"
verifica "livello 1: non e' contato fra i falliti" "0" "${#falliti[@]}"

# 2. Un passo pesante **fallito** al livello 1 non puo' rossare, perche' non
#    gira affatto: e' la proprieta' che rende la modalita' usabile.
passi=0
omessi=0
falliti=()
esegui passo_pesante fuzz_smoke false
verifica "livello 1: un pesante rotto non viene eseguito" "0" "${#falliti[@]}"

# 3. Al livello 2 lo stesso passo gira e rossa. Senza questa sonda la
#    modalita' potrebbe omettere **sempre**, e nessuno se ne accorgerebbe.
passi=0
verdi=0
omessi=0
falliti=()
LIVELLO=2
esegui passo_pesante coverage_misura false
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

# --- INFRA-7a: l'elenco dei passi pesanti e' chiuso -------------------------
#
# Senza elenco chiuso, marcare per sbaglio un gate come pesante lo farebbe
# sparire dal livello 1 in silenzio: lo stesso difetto che la modalita' esiste
# per chiudere, reintrodotto dalla porta di servizio.

passi=0
verdi=0
omessi=0
falliti=()
LIVELLO=1
esegui passo_pesante fuzz_smoke true
verifica "un nome autorizzato e' omesso" "1" "${omessi}"
verifica "e non e' fra i falliti" "0" "${#falliti[@]}"

passi=0
verdi=0
omessi=0
falliti=()
esegui passo_pesante un_gate_qualunque true
verifica "un decimo nome e' rifiutato" "1" "${#falliti[@]}"
verifica "il rifiuto non lo conta fra gli omessi" "0" "${omessi}"
verifica \
    "il rifiuto e' motivato nel nome del fallito" \
    "un_gate_qualunque(non autorizzato)" \
    "${falliti[0]}"

# Il rifiuto riguarda la **dichiarazione**, quindi vale anche al livello 2,
# dove il passo verrebbe comunque eseguito.
passi=0
verdi=0
omessi=0
falliti=()
LIVELLO=2
esegui passo_pesante un_gate_qualunque true
verifica "il rifiuto vale anche al livello 2" "1" "${#falliti[@]}"
verifica "e il passo non viene eseguito" "0" "${verdi}"

# I nove autorizzati sono esattamente nove, e sono quelli.
verifica "l'elenco chiuso ha nove nomi" "9" "${#PASSI_PESANTI[@]}"
for atteso in fuzz_replay fuzz_smoke coverage_pulizia coverage_misura \
    coverage_export coverage_report_non_vuoto check_coverage_exclusions \
    coverage_soglia_dal_report coverage_soglia_controprova; do
    if e_pesante_autorizzato "${atteso}"; then
        verifica "«${atteso}» e' autorizzato" "si" "si"
    else
        verifica "«${atteso}» e' autorizzato" "si" "no"
    fi
done
LIVELLO=2

# --- INFRA-7b: l'impronta dell'albero ---------------------------------------
#
# `git status --porcelain | wc -l` conta le righe: un passo che modifica un
# file **gia' sporco** lascia il conteggio identico. La sonda decisiva e'
# proprio quella, e senza di essa la difesa sarebbe apparente.

ALBERO="${S9_CHECKPOINT_LOG_DIR}/albero-finto"
rm -rf "${ALBERO}"
mkdir -p "${ALBERO}"
(
    cd "${ALBERO}" || exit 1
    git init -q .
    git config user.email prova@example.com
    git config user.name Prova
    echo "originale" > tracciato.txt
    echo "altro" > secondo.txt
    git add -A
    git commit -qm base
    # Un file **gia' sporco** prima della misura, come nell'incidente.
    echo "modificato a mano" > tracciato.txt
    echo "non tracciato" > nuovo.txt
)

impronta_in() { ( cd "$1" && impronta_albero ); }

sporchi_prima="$( ( cd "${ALBERO}" && git status --porcelain | wc -l ) | tr -d ' ' )"
prima="$(impronta_in "${ALBERO}")"

# La sentinella gia' sporca viene modificata: il conteggio non si muove.
echo "modificato da un passo" > "${ALBERO}/tracciato.txt"
sporchi_dopo="$( ( cd "${ALBERO}" && git status --porcelain | wc -l ) | tr -d ' ' )"
dopo="$(impronta_in "${ALBERO}")"

verifica "il conteggio dei file sporchi NON si accorge" "${sporchi_prima}" "${sporchi_dopo}"
if [ "${prima}" != "${dopo}" ]; then
    verifica "l'impronta invece si' (file gia' sporco)" "diversa" "diversa"
else
    verifica "l'impronta invece si' (file gia' sporco)" "diversa" "uguale"
fi

# Anche un untracked **gia' presente** che cambia contenuto: l'elenco dei nomi
# resterebbe identico.
echo "modificato da un passo" > "${ALBERO}/tracciato.txt"
prima="$(impronta_in "${ALBERO}")"
echo "contenuto diverso" > "${ALBERO}/nuovo.txt"
dopo="$(impronta_in "${ALBERO}")"
if [ "${prima}" != "${dopo}" ]; then
    verifica "l'impronta vede un untracked riscritto" "diversa" "diversa"
else
    verifica "l'impronta vede un untracked riscritto" "diversa" "uguale"
fi

# E un albero fermo da' la stessa impronta: senza questa, «sempre diversa»
# passerebbe per una difesa.
prima="$(impronta_in "${ALBERO}")"
dopo="$(impronta_in "${ALBERO}")"
verifica "un albero fermo da' impronta stabile" "${prima}" "${dopo}"

# --- INFRA-7b: l'impronta regge i binari ------------------------------------
#
# Senza `--binary`, git stampa «Binary files a/x and b/x differ»: **la stessa
# riga** per due modifiche diverse. L'impronta non distinguerebbe un binario
# corrotto da uno sostituito.
(
    cd "${ALBERO}" || exit 1
    printf 'PK\003\004\000\001\002' > binario.bin
    git add binario.bin
    git commit -qm binario
)
printf 'PK\003\004\377\376\375' > "${ALBERO}/binario.bin"
prima="$(impronta_in "${ALBERO}")"
printf 'PK\003\004\001\002\003' > "${ALBERO}/binario.bin"
dopo="$(impronta_in "${ALBERO}")"
if [ "${prima}" != "${dopo}" ]; then
    verifica "due modifiche binarie diverse danno impronte diverse" "diversa" "diversa"
else
    verifica "due modifiche binarie diverse danno impronte diverse" "diversa" "uguale"
fi

# Due untracked distinti non devono collidere per come sono concatenati: il
# percorso e il contenuto sono delimitati, non incollati.
(
    cd "${ALBERO}" || exit 1
    printf 'b' > 'a'
    printf '' > 'ab'
)
uno="$(impronta_in "${ALBERO}")"
(
    cd "${ALBERO}" || exit 1
    rm -f 'a' 'ab'
    printf '' > 'a'
    printf 'b' > 'ab'
)
due="$(impronta_in "${ALBERO}")"
if [ "${uno}" != "${due}" ]; then
    verifica "due untracked non collidono per concatenazione" "diversa" "diversa"
else
    verifica "due untracked non collidono per concatenazione" "diversa" "uguale"
fi

# --- INFRA-7b: la revisione va riletta, non ristampata ----------------------
#
# Un commit durante la corsa lascia l'albero **invariato** e sposta HEAD: la
# misura descriverebbe un albero e l'esito ne nominerebbe un altro. Fino al
# 2026-08-21 la coda ristampava la variabile acquisita in testa, quindi «SHA
# iniziale e finale identici» era vero per costruzione.
(
    cd "${ALBERO}" || exit 1
    git add -A
    git commit -qm "stato di partenza"
)
revisione_prima="$( ( cd "${ALBERO}" && git rev-parse HEAD ) )"
impronta_prima="$(impronta_in "${ALBERO}")"

# Un commit **vuoto**: HEAD si muove, l'albero no.
( cd "${ALBERO}" && git commit -q --allow-empty -m "commit durante la corsa" )

revisione_dopo="$( ( cd "${ALBERO}" && git rev-parse HEAD ) )"
impronta_dopo="$(impronta_in "${ALBERO}")"

verifica "un commit vuoto NON muove l'impronta" "${impronta_prima}" "${impronta_dopo}"
if [ "${revisione_prima}" != "${revisione_dopo}" ]; then
    verifica "ma muove HEAD, e va colto a parte" "diversa" "diversa"
else
    verifica "ma muove HEAD, e va colto a parte" "diversa" "uguale"
fi

# La lettura deve venire da `git`, non da una variabile: se la coda dello
# script ristampasse `REVISIONE`, questo confronto sarebbe sempre verde.
REVISIONE="${revisione_prima}"
REVISIONE_FINE="$( ( cd "${ALBERO}" && git rev-parse HEAD ) )"
if [ "${REVISIONE_FINE}" != "${REVISIONE}" ]; then
    verifica "rileggere HEAD rivela lo spostamento" "rivelato" "rivelato"
else
    verifica "rileggere HEAD rivela lo spostamento" "rivelato" "nascosto"
fi

rm -rf "${ALBERO}"

echo
if [ "${rosse}" -ne 0 ]; then
    echo "sonde: $((sonde - rosse))/${sonde} — ROSSE: ${rosse}"
    exit 1
fi
echo "sonde: ${sonde}/${sonde}"
