#!/usr/bin/env bash
# Checkpoint di livello 2 per S9 (design § 20).
#
# Esiste come script versionato, e non come comando improvvisato, per una
# ragione trovata sul campo: la prima esecuzione del 2026-08-20 girava da un
# harness ad hoc, e quell'harness aveva tre difetti che nessuno poteva vedere
# perche' non stava in nessun repository. Uno dei tre ha quasi fatto archiviare
# un finding reale.
#
# I tre difetti, e come sono chiusi qui:
#
#   1. lo smoke 13/13 veniva eseguito **due volte** — una dentro la batteria e
#      una a parte — per quaranta minuti che non aggiungevano nulla. Qui la
#      batteria **non** include `fuzz-smoke.sh`: lo smoke e' un passo dichiarato,
#      eseguito una volta sola;
#
#   2. il log di un gate rosso veniva troncato a sei righe, e i dettagli del
#      crash andavano persi. Qui **ogni** passo scrive un log completo su file,
#      il file resta, e il percorso viene stampato quando il passo fallisce;
#
#   3. il gate sulle esclusioni di copertura girava **prima** che il report LCOV
#      esistesse, quindi era rosso a ogni corsa per assenza dell'input. Un rosso
#      che si ripete sempre smette di essere letto: qui la copertura e' generata
#      prima, e il gate legge un report che c'e'.
#
# L'esito di ogni passo viene dal comando, mai da una pipe: nessun `| tail`
# decide se qualcosa e' andato bene.

set -u

cd "$(dirname "$0")/.."

LOG_DIR="${S9_CHECKPOINT_LOG_DIR:-/tmp/s9-checkpoint}"
mkdir -p "${LOG_DIR}"

passi=0
verdi=0
falliti=()
catena_rotta=""

# Esegue un passo, conserva il log per intero, e prende l'esito dal comando.
passo() {
    local nome="$1"
    shift
    local log="${LOG_DIR}/${nome}.log"
    passi=$((passi + 1))
    printf '  %-38s ' "${nome}"
    # L'esito si cattura **subito dopo il comando**, non dopo un `if`: un `if`
    # con condizione falsa e senza `else` restituisce 0, quindi `$?` letto dopo
    # il `fi` diceva sempre «exit 0» anche sui rossi. Trovato al checkpoint del
    # 2026-08-21, che ha stampato «ROSSO (exit 0)» -- una contraddizione che
    # rendeva inutile proprio il numero che serve a capire come si e' fallito.
    "$@" > "${log}" 2>&1
    local esito=$?
    if [ "${esito}" -eq 0 ]; then
        verdi=$((verdi + 1))
        echo "verde"
        return 0
    fi
    echo "ROSSO (exit ${esito}) — ${log}"
    falliti+=("${nome}")
    return "${esito}"
}

# Marca un passo come **non eseguito** perche' una sua precondizione e'
# fallita. Non e' verde: un passo che non e' girato non ha verificato niente, e
# contarlo fra i verdi e' il modo in cui un checkpoint si autoconvalida.
salta() {
    local nome="$1"
    local causa="$2"
    passi=$((passi + 1))
    printf '  %-38s ' "${nome}"
    echo "SALTATO (${causa} e' fallito)"
    falliti+=("${nome}(saltato)")
}

# Esegue un passo **solo se la catena non e' gia' rotta**.
#
# La prima versione faceva dipendere i passi della copertura dalla sola misura:
# se `coverage_export` falliva, il gate delle esclusioni e la soglia giravano lo
# stesso, su un file che poteva essere assente o vecchio. Ogni passo dipende ora
# dal **precedente**, non dal primo della catena.
#
# `catena_rotta` porta il nome del passo che l'ha spezzata, cosi' il motivo del
# salto e' quello vero e non un generico «a monte e' andato male».
passo_in_catena() {
    local nome="$1"
    shift
    if [ -n "${catena_rotta}" ]; then
        salta "${nome}" "${catena_rotta}"
        return 1
    fi
    if passo "${nome}" "$@"; then
        return 0
    fi
    catena_rotta="${nome}"
    return 1
}

# Consente alle sonde di caricare solo le funzioni, senza eseguire il
# checkpoint. Un gate che non si puo' provare e' un gate di cui ci si fida
# perche' non si e' mai visto sbagliare.
if [ "${S9_CHECKPOINT_SOLO_FUNZIONI:-0}" = "1" ]; then
    return 0
fi

REVISIONE="$(git rev-parse HEAD)"
SPORCHI="$(git status --porcelain | wc -l)"

echo "=============================================================="
echo "S9 — checkpoint di livello 2"
echo "revisione:      ${REVISIONE}"
echo "albero:         ${SPORCHI} file non committati"
echo "log:            ${LOG_DIR}"
echo "=============================================================="

if [ "${SPORCHI}" -ne 0 ]; then
    echo
    echo "ALBERO SPORCO: la misura non sarebbe same-SHA." >&2
    echo "Un checkpoint su un albero che non coincide con la revisione" >&2
    echo "dichiarata e' peggio di nessun checkpoint: sembra un'evidenza." >&2
    exit 2
fi


echo
echo "--- 1. compilazione e test -----------------------------------"
# Le sonde del checkpoint stesso vengono per prime: se il gate che misura e'
# rotto, tutto cio' che segue e' una misura di cui non si sa niente.
passo sonde_checkpoint bash scripts/test_s9_checkpoint.sh
passo fmt cargo fmt --all -- --check
passo clippy cargo clippy --workspace --all-targets --all-features -- -D warnings
# `--all-features` abilita `gdal-backend`, quindi il percorso stub di
# driver-filegdb non veniva compilato da nessun passo. Il 2026-08-21 e'
# rimasto rotto per un'intera tranche, e a trovarlo e' stata la misura di
# copertura, che gira senza feature: un gate non dovrebbe dipendere da un
# altro per accorgersi di una compilazione fallita.
passo clippy_default cargo clippy --workspace --all-targets -- -D warnings
passo test cargo test --workspace --all-features
# Il set di feature **predefinito** va compilato a parte: `--all-features`
# abilita `gdal-backend`, e il percorso stub di driver-filegdb non veniva
# compilato da nessun passo di livello 1. Il 2026-08-21 e' rimasto rotto per
# un'intera tranche, e a trovarlo e' stata la misura di copertura -- che gira
# senza feature. Un passo non deve dipendere da un altro per accorgersi che
# qualcosa non compila.
passo test_default cargo test --workspace --all-targets

echo
echo "--- 2. gate del censimento e delle sonde ---------------------"
passo sonde_quartetto python3 -m unittest scripts.test_check_quartetto_sito
passo check_quartetto python3 scripts/check_quartetto_sito.py
passo sonde_errori_redatti python3 -m unittest scripts.test_check_errori_redatti
passo check_errori_redatti python3 scripts/check_errori_redatti.py

# `&'static str` garantisce la durata, non la provenienza: senza questo
# passo un `Box::leak` riporterebbe testo runtime dentro un messaggio
# curato, e il censimento resterebbe verde.
passo sonde_niente_leak python3 -m unittest scripts.test_check_niente_leak
passo check_niente_leak python3 scripts/check_niente_leak.py
passo sonde_wkb_limits python3 -m unittest scripts.test_check_wkb_limits_defaults
passo check_wkb_limits python3 scripts/check_wkb_limits_defaults.py
passo sonde_quarantena python3 -m unittest scripts.test_check_quarantena_fuzz
passo check_quarantena python3 scripts/check_quarantena_fuzz.py
passo sonde_prevalidazione python3 -m unittest scripts.test_check_prevalidazione_decoder
passo check_prevalidazione python3 scripts/check_prevalidazione_decoder.py
passo sonde_identita python3 -m unittest scripts.test_check_public_identity
passo check_identita python3 scripts/check_public_identity.py
passo sonde_release python3 -m unittest scripts.test_check_release_contract
passo check_release python3 scripts/check_release_contract.py --historical
passo sonde_patch python3 -m unittest scripts.test_check_patch_readiness
passo sonde_action_pins python3 -m unittest scripts.test_check_action_pins
passo check_action_pins python3 scripts/check_action_pins.py
passo sonde_toolchain python3 -m unittest scripts.test_check_toolchain_pins
passo check_toolchain python3 scripts/check_toolchain_pins.py
passo sonde_coverage_excl python3 -m unittest scripts.test_check_coverage_exclusions
passo sonde_filegdb python3 -m unittest scripts.test_check_filegdb_catalog
passo sonde_wkb_condiviso python3 -m unittest scripts.test_compare_shared_wkb_observations
passo corpus_condiviso python3 scripts/generate_shared_wkb_corpus.py --check
passo check_dependency_pins python3 scripts/check_dependency_pins.py
passo check_gdal_fork python3 scripts/check_gdal_fork.py
passo check_dxf_fork python3 scripts/check_dxf_fork.py
passo check_no_legacy_budget python3 scripts/check_no_legacy_budget.py
passo check_permit_boundary python3 scripts/check_permit_boundary.py
passo sonde_fallback python3 -m unittest scripts.test_check_assurance_fallbacks
passo assurance_fallbacks bash scripts/check_assurance_fallbacks.sh

echo
echo "--- 3. catalogo FileGDB reale --------------------------------"
catalogo_filegdb() {
    local json="${LOG_DIR}/catalog.json"
    cargo run --quiet -p plenora-io-cli --features gdal-backend --locked -- catalog > "${json}" || return
    python3 scripts/check_filegdb_catalog.py < "${json}"
}
passo check_filegdb_catalog catalogo_filegdb

echo
echo "--- 4. fuzz: replay deterministico, poi smoke ----------------"
# Il replay viene **prima**: rigioca semi, corpus e artefatti gia' noti, ed e'
# deterministico. Lo smoke cerca input nuovi, e sessanta secondi di mutazioni
# non ritrovano quello che il replay ritrova sempre. Invertirli significa
# scoprire una regressione nota solo per fortuna.
passo fuzz_replay bash scripts/fuzz-replay.sh
passo fuzz_smoke bash scripts/fuzz-smoke.sh

echo
echo "--- 5. copertura, poi il suo gate ----------------------------"
ESCLUSIONI='(^|/)(plenora-bench|plenora-fuzz|plenora-io-cli)/src/.*\.rs$'
export PLENORA_CROSS_FS_TEST_ROOT="${PLENORA_CROSS_FS_TEST_ROOT:-/dev/shm}"
# I dati di profiling precedenti vanno rimossi **prima** di misurare. Senza,
# una misura fallita lascia in piedi quelli della corsa precedente, e i passi
# che seguono li leggono senza sapere che descrivono un altro albero: e'
# successo il 2026-08-21, e `coverage_soglia` ha detto «verde» su un albero che
# non era quello dichiarato.
LCOV="${LOG_DIR}/lcov.info"
# Il percorso del report parte **vuoto**: se l'export fallisce, non deve restare
# in piedi il file di una corsa precedente per i passi che seguono.
rm -f "${LCOV}"

catena_rotta=""
passo_in_catena coverage_pulizia cargo llvm-cov clean --workspace
passo_in_catena coverage_misura cargo llvm-cov --workspace --all-targets --locked --no-report
passo_in_catena coverage_export cargo llvm-cov report --lcov --output-path "${LCOV}" \
    --ignore-filename-regex "${ESCLUSIONI}"
# Un export che finisce con esito zero e un file vuoto e' un caso che nessuno
# guarda finche' non succede: qui e' un passo, non un'assunzione.
passo_in_catena coverage_report_non_vuoto test -s "${LCOV}"
passo_in_catena check_coverage_exclusions python3 scripts/check_coverage_exclusions.py \
    --lcov "${LCOV}"
# La soglia si legge **dallo stesso file** delle esclusioni.
#
# La versione di cargo resta accanto, ma NON come conferma dello stesso numero:
# sulla corsa dell'8e64965 le due hanno dato 85,88% e 83,98% sugli stessi 38
# file. Non e' un errore di nessuna delle due -- i record `DA:` di LCOV e la
# colonna «Lines» di llvm-cov contano insiemi diversi di righe strumentate, e il
# denominatore differisce di 1.181 su 32.243.
#
# Sono quindi due **proiezioni diverse** dello stesso profdata, ed entrambe
# devono stare sopra la soglia. Chiamarle «la stessa misura» era un'imprecisione
# mia, e avrebbe reso illeggibile il giorno in cui una delle due si rompesse
# davvero.
passo_in_catena coverage_soglia_dal_report python3 scripts/check_coverage_threshold.py \
    --lcov "${LCOV}" --min-lines 80
passo_in_catena coverage_soglia_controprova cargo llvm-cov report --summary-only \
    --ignore-filename-regex "${ESCLUSIONI}" --fail-under-lines 80

if [ -z "${catena_rotta}" ]; then
    copertura_misurata=1
else
    copertura_misurata=0
fi

# --- diagnostica: la copertura delle sole righe cambiate ---------------------
#
# NON e' un gate e non ha soglia. La copertura totale non distingue due cose che
# vanno distinte dopo un refactor: la crescita meccanica del denominatore --
# messaggi curati su piu' righe, funzioni estratte -- da un ramo semantico nuovo
# che nessun test esercita. Guardando solo le righe cambiate, la prima risulta
# coperta e la seconda no.
#
# La prima esecuzione, sull'intervallo 107b7b5..effc4ab, ha dato 37,27% e ha
# trovato 22 righe mai eseguite dentro `classe_sqlite`: un vocabolario nuovo che
# nessun test attraversava. Non era crescita meccanica.
#
# Serve un riferimento: S9_CHECKPOINT_BASE e' la revisione dell'ultimo
# checkpoint **superato**, non l'ultimo commit. La differenza conta: fra un
# checkpoint e il successivo ci sono anche commit di infrastruttura e di
# documentazione, e sono parte del delta da qualificare. Prendere come base
# l'ultimo commit escluderebbe dalla misura proprio cio' che non e' ancora stato
# verificato.
#
# Senza, il passo si salta invece di misurare un intervallo arbitrario -- una
# diagnostica su un intervallo scelto a caso e' peggio di nessuna diagnostica,
# perche' ha comunque l'aria di un numero.
echo
echo "--- 6. diagnostica: copertura delle righe cambiate -----------"
if [ "${copertura_misurata}" -ne 1 ]; then
    echo "  saltata: la catena della copertura si e' rotta a «${catena_rotta}»."
    echo "  Un numero calcolato su un report assente o stantio somiglia a una"
    echo "  misura, e descrive un albero che non e' quello dichiarato."
elif [ -n "${S9_CHECKPOINT_BASE:-}" ]; then
    python3 scripts/coverage_diff.py --lcov "${LCOV}" \
        --base "${S9_CHECKPOINT_BASE}" --head "${REVISIONE}" \
        > "${LOG_DIR}/coverage_diff.log" 2>&1
    esito_diff=$?
    cat "${LOG_DIR}/coverage_diff.log"
    if [ "${esito_diff}" -ne 0 ]; then
        echo "  la diagnostica non ha potuto misurare (exit ${esito_diff});"
        echo "  non e' un gate, ma il suo silenzio non va letto come un verde."
    fi
else
    echo "  saltata: S9_CHECKPOINT_BASE non impostata."
    echo "  Impostala alla revisione dell'ultimo checkpoint superato."
fi

echo
echo "=============================================================="
echo "revisione verificata: ${REVISIONE}"
echo "passi: ${verdi}/${passi}"
if [ "${#falliti[@]}" -ne 0 ]; then
    echo "ROSSI: ${falliti[*]}"
    echo "esito: NON SUPERATO"
    echo "=============================================================="
    exit 1
fi
echo "esito: S9 checkpoint level 2 passed"
echo
echo "Questo esito NON autorizza una release e non promuove la readiness"
echo "di alcun componente ne' del sistema. Va registrato in un'evidenza S9"
echo "separata, mai in docs/assurance/SYSTEM_RC_GATE.md."
echo "=============================================================="
