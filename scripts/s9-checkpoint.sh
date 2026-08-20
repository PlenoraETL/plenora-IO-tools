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

passi=0
verdi=0
falliti=()

# Esegue un passo, conserva il log per intero, e prende l'esito dal comando.
passo() {
    local nome="$1"
    shift
    local log="${LOG_DIR}/${nome}.log"
    passi=$((passi + 1))
    printf '  %-38s ' "${nome}"
    if "$@" > "${log}" 2>&1; then
        verdi=$((verdi + 1))
        echo "verde"
        return 0
    fi
    local esito=$?
    echo "ROSSO (exit ${esito}) — ${log}"
    falliti+=("${nome}")
    return "${esito}"
}

echo
echo "--- 1. compilazione e test -----------------------------------"
passo fmt cargo fmt --all -- --check
passo clippy cargo clippy --workspace --all-targets --all-features -- -D warnings
passo test cargo test --workspace --all-features

echo
echo "--- 2. gate del censimento e delle sonde ---------------------"
passo sonde_errori_redatti python3 -m unittest scripts.test_check_errori_redatti
passo check_errori_redatti python3 scripts/check_errori_redatti.py
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
passo coverage_misura cargo llvm-cov --workspace --all-targets --locked --no-report
passo coverage_export cargo llvm-cov report --lcov --output-path "${LOG_DIR}/lcov.info" \
    --ignore-filename-regex "${ESCLUSIONI}"
# Ora il report esiste: il gate legge qualcosa invece di lamentarne l'assenza.
passo check_coverage_exclusions python3 scripts/check_coverage_exclusions.py \
    --lcov "${LOG_DIR}/lcov.info"
passo coverage_soglia cargo llvm-cov report --summary-only \
    --ignore-filename-regex "${ESCLUSIONI}" --fail-under-lines 80

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
if [ -n "${S9_CHECKPOINT_BASE:-}" ]; then
    python3 scripts/coverage_diff.py --lcov "${LOG_DIR}/lcov.info" \
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
