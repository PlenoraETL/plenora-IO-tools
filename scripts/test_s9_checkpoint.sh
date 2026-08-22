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

# --- INFRA-7.1: l'impronta fallisce invece di restituire il vuoto -----------
#
# La versione precedente convogliava i tre comandi git in una pipe con
# `2>/dev/null`: se git falliva, la pipe riceveva zero byte e l'impronta era
# lo sha256 della stringa vuota -- **lo stesso valore di un albero pulito**.
#
# Trovato alla qualifica di `1c2707e`. Le quattro sonde coprono i quattro modi
# in cui la funzione puo' essere interrogata, e il punto e' che tre di essi
# davano prima lo stesso risultato.

# 1. Repository assente: deve **fallire**, non restituire un'impronta.
( cd /tmp && impronta_albero > /dev/null 2>&1 )
verifica "fuori da un worktree l'impronta fallisce" "1" "$?"

# 2. Errore interno di git: `rev-parse` risponde, `diff` no. Controllare il
#    solo `rev-parse` non basterebbe, ed e' la ragione per cui ogni comando ha
#    il proprio controllo.
FINTO="${S9_CHECKPOINT_LOG_DIR}/git-finto"
mkdir -p "${FINTO}"
{
    printf '#!/bin/sh\n'
    printf 'case "$*" in\n'
    printf '  *"diff --cached"*) exit 128 ;;\n'
    printf 'esac\n'
    printf 'exec /usr/bin/git "$@"\n'
} > "${FINTO}/git"
chmod +x "${FINTO}/git"
(
    cd "${ALBERO}" || exit 1
    PATH="${FINTO}:${PATH}" impronta_albero > /dev/null 2>&1
)
verifica "un git che fallisce internamente non produce impronta" "1" "$?"
rm -rf "${FINTO}"

# 3. Repository pulito: deve **riuscire**, e l'impronta non puo' valere lo
#    sha256 della stringa vuota -- altrimenti «pulito» e «git rotto»
#    resterebbero indistinguibili nel valore.
(
    cd "${ALBERO}" || exit 1
    git add -A > /dev/null 2>&1
    git commit -qm "pulito" > /dev/null 2>&1
)
pulita="$( ( cd "${ALBERO}" && impronta_albero ) )"
esito_pulita=$?
verifica "su un repository pulito l'impronta riesce" "0" "${esito_pulita}"
VUOTA=e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
if [ "${pulita}" = "${VUOTA}" ]; then
    verifica "e non vale lo sha della stringa vuota" "distinta" "vuota"
else
    verifica "e non vale lo sha della stringa vuota" "distinta" "distinta"
fi

# 4. Modifiche **ignorate**: possono cambiare senza muovere l'impronta. E' il
#    confine che il livello 2 richiede -- fuzz e copertura scrivono in
#    `target/` e `fuzz/corpus/` a ogni corsa.
(
    cd "${ALBERO}" || exit 1
    printf 'scarti/\n' > .gitignore
    git add .gitignore > /dev/null 2>&1
    git commit -qm gitignore > /dev/null 2>&1
    mkdir -p scarti
    printf 'primo artefatto\n' > scarti/a.tmp
)
prima_ignorata="$( ( cd "${ALBERO}" && impronta_albero ) )"
(
    cd "${ALBERO}" || exit 1
    printf 'artefatto diverso e piu lungo\n' > scarti/a.tmp
    printf 'e un secondo\n' > scarti/b.tmp
)
dopo_ignorata="$( ( cd "${ALBERO}" && impronta_albero ) )"
verifica "gli artefatti ignorati non muovono l'impronta" \
    "${prima_ignorata}" "${dopo_ignorata}"

# Controprova: un file **non** ignorato la muove. Senza, «non si muove mai»
# passerebbe per un confine.
( cd "${ALBERO}" && printf 'versionabile\n' > visibile.txt )
dopo_visibile="$( ( cd "${ALBERO}" && impronta_albero ) )"
if [ "${dopo_ignorata}" != "${dopo_visibile}" ]; then
    verifica "un file versionabile invece si'" "diversa" "diversa"
else
    verifica "un file versionabile invece si'" "diversa" "uguale"
fi
( cd "${ALBERO}" && rm -f visibile.txt )

# --- INFRA-7b: la revisione va riletta, non ristampata ----------------------
#
# Un commit durante la corsa lascia l'albero **invariato** e sposta HEAD: la
# misura descriverebbe un albero e l'esito ne nominerebbe un altro. Fino al
# 2026-08-21 la coda ristampava la variabile acquisita in testa, quindi «SHA
# iniziale e finale identici» era vero per costruzione.
(
    cd "${ALBERO}" || exit 1
    git add -A > /dev/null 2>&1
    # `-q` non zittisce «nothing to commit»: la sonda precedente puo' aver
    # gia' committato tutto, e il rumore si confonderebbe con l'esito.
    git commit -qm "stato di partenza" > /dev/null 2>&1 || true
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

# --- il risultato: atomico e persistente -------------------------------------
#
# L'esito viveva solo sullo stdout. Una corsa e' gia' stata scartata per intero
# perche' il container girava con `--rm`: il verdetto e' stato osservato, le
# misure sono sparite, e non si combinano il verdetto di una corsa e i numeri
# di un'altra.
#
# Due proprieta', e vanno provate separatamente: che il risultato **esista su
# disco**, e che chi lo legge non possa mai vederne meta'.
RISULTATI="$(mktemp -d)"

passi=7
verdi=5
omessi=2
falliti=(alfa beta)
REVISIONE="0000000000000000000000000000000000000000"
REVISIONE_FINE="${REVISIONE}"
IMPRONTA_INIZIO="abcdef"
IMPRONTA_FINE="abcdef"
SPORCHI=0
RISULTATO="${RISULTATI}/risultato.json"

scrivi_risultato non_superato
verifica "scrivere il risultato riesce" "0" "$?"
verifica "il risultato esiste su disco" "si" "$([ -f "${RISULTATO}" ] && echo si || echo no)"
verifica "e' JSON leggibile" "ok" \
    "$(python3 -c 'import json,sys;json.load(open(sys.argv[1]));print("ok")' "${RISULTATO}" 2>&1 | tail -1)"
verifica "porta l'esito della corsa" "non_superato" \
    "$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["esito"])' "${RISULTATO}")"
verifica "porta i passi ricostruiti" "7 5 2" \
    "$(python3 -c 'import json,sys;d=json.load(open(sys.argv[1]));print(d["passi"],d["verdi"],d["omessi"])' "${RISULTATO}")"
verifica "porta i nomi dei passi rossi" "alfa beta" \
    "$(python3 -c 'import json,sys;print(" ".join(json.load(open(sys.argv[1]))["falliti"]))' "${RISULTATO}")"
verifica "non lascia file parziali" "0" \
    "$(find "${RISULTATI}" -name '*.parziale.*' | wc -l | tr -d ' ')"

# Una seconda scrittura sostituisce la prima: il file dice l'ultimo stato noto
# della corsa, non il primo.
scrivi_risultato superato
verifica "una scrittura successiva sostituisce" "superato" \
    "$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["esito"])' "${RISULTATO}")"

# La sonda dell'atomicita': se la produzione del JSON si interrompe a meta', la
# destinazione deve conservare il **contenuto precedente completo**. Scrivere
# direttamente sulla destinazione la lascerebbe troncata, e un JSON troncato
# non e' distinguibile da un JSON scritto male.
_risultato_intero="$(cat "${RISULTATO}")"
_risultato_json_originale="$(declare -f _risultato_json)"
_risultato_json() {
    printf '{\n  "schema_version": 1,\n  "esi'
    return 1
}
scrivi_risultato interrotto
verifica "una produzione interrotta non scrive" "1" "$?"
verifica "la destinazione resta quella di prima" "${_risultato_intero}" "$(cat "${RISULTATO}")"
verifica "e nessun parziale sopravvive" "0" \
    "$(find "${RISULTATI}" -name '*.parziale.*' | wc -l | tr -d ' ')"
eval "${_risultato_json_originale}"

# Una destinazione non scrivibile **fallisce**, invece di far credere che il
# risultato sia stato registrato.
RISULTATO="${RISULTATI}/non/esiste/risultato.json"
scrivi_risultato superato
verifica "una destinazione impossibile e' rossa" "1" "$?"

# --- ogni uscita terminale registra il proprio esito ------------------------
#
# Il file veniva scritto `in_corso` all'avvio, e un livello 2 su albero sporco
# usciva con 2 **senza passare da `concludi`**: il risultato restava `in_corso`
# mentre la causa era nota. «E' morta a meta'» e «e' stata rifiutata in
# partenza» si chiudono in modi diversi, ed e' la distinzione per cui questo
# file esiste.
RISULTATO="${RISULTATI}/terminale.json"
( concludi albero_sporco 2 ) > /dev/null 2>&1
verifica "un'uscita terminale esce con il proprio codice" "2" "$?"
verifica "e registra un esito terminale, non «in_corso»" "albero_sporco" \
    "$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["esito"])' "${RISULTATO}")"

( concludi superato 0 ) > /dev/null 2>&1
verifica "e il codice 0 quando la corsa e' passata" "0" "$?"
verifica "con l'esito della corsa" "superato" \
    "$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["esito"])' "${RISULTATO}")"

# L'elenco delle `exit` nude e' **chiuso**. Le tre ammesse sono quelle che non
# possono registrare niente, perche' e' la scrittura del risultato a essere
# fallita: `exit "${codice}"` dentro `concludi`, il suo ripiego quando la
# scrittura non riesce, e il rifiuto in testa alla corsa. Ogni altra uscita
# deve passare da `concludi`, e aggiungerne una fa rossa questa sonda invece di
# lasciare un `in_corso` su disco.
nude="$(grep -cE '^[[:space:]]*exit ' "${RADICE}/scripts/s9-checkpoint.sh")"
verifica "nessuna uscita terminale fuori da concludi" "3" "${nude}"

rm -rf "${RISULTATI}"

echo
if [ "${rosse}" -ne 0 ]; then
    echo "sonde: $((sonde - rosse))/${sonde} — ROSSE: ${rosse}"
    exit 1
fi
echo "sonde: ${sonde}/${sonde}"
