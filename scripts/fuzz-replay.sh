#!/bin/bash
# Replay deterministico di corpus, semi e artefatti su OGNI target (FZ-0).
#
# Non e' fuzzing: `-runs=0` esegue una volta ciascun input gia' esistente e
# esce, senza mutazioni. Due esecuzioni sulla stessa revisione e sullo stesso
# corpus danno lo stesso esito, quindi il replay serve da confronto prima/dopo
# una correzione — cosa che una campagna, che esplora, non puo' fare.
#
# La quarantena **non** e' letta apposta: un target in quarantena e' escluso
# dallo smoke, e il replay e' il posto dove deve continuare a essere osservato.
# Se non lo fosse, una correzione a monte non avrebbe modo di dimostrarsi.
#
# Uso: scripts/fuzz-replay.sh [target ...]
set -uo pipefail

cd "$(dirname "$0")/.."

# Stessa scelta esplicita della toolchain di fuzz-smoke.sh: -Zsanitizer richiede
# nightly, e senza una scelta esplicita lo script userebbe cio' che capita.
toolchain="${PLENORA_FUZZ_TOOLCHAIN:-nightly-2026-07-21}"
if ! rustup toolchain list | grep -q "^${toolchain}"; then
    echo "toolchain ${toolchain} non installata: e' quella che serve a -Zsanitizer=address" >&2
    exit 1
fi

rss_limit_mb="${PLENORA_FUZZ_RSS_MB:-2048}"
timeout_s="${PLENORA_FUZZ_TIMEOUT:-15}"

options=()
if [ -n "${PLENORA_FUZZ_TARGET_DIR:-}" ]; then
    options=(--target-dir "${PLENORA_FUZZ_TARGET_DIR}")
fi

if [ "$#" -gt 0 ]; then
    targets=("$@")
else
    mapfile -t targets < <(cargo +"${toolchain}" fuzz list)
fi
if [ "${#targets[@]}" -eq 0 ]; then
    echo "nessun target fuzz dichiarato in fuzz/Cargo.toml" >&2
    exit 1
fi

echo "=== build strumentata (${#targets[@]} target) ==="
if ! cargo +"${toolchain}" fuzz build "${options[@]}"; then
    echo "build strumentata fallita" >&2
    exit 1
fi

falliti=()
totale_input=0
for target in "${targets[@]}"; do
    # Ogni sorgente di input esistente entra nel replay: i semi versionati, il
    # corpus accumulato e gli artefatti dei crash. Gli artefatti soprattutto:
    # sono gli input che hanno prodotto un finding, e sono la prova che una
    # correzione funziona o non funziona.
    ingressi=()
    for cartella in "fuzz/seeds/${target}" "fuzz/corpus/${target}" "fuzz/artifacts/${target}"; do
        if [ -d "${cartella}" ] && [ -n "$(ls -A "${cartella}" 2>/dev/null)" ]; then
            ingressi+=("${cartella}")
        fi
    done
    if [ "${#ingressi[@]}" -eq 0 ]; then
        echo "=== ${target}: nessun input da rieseguire ==="
        continue
    fi

    quanti=$(find "${ingressi[@]}" -type f | wc -l)
    totale_input=$((totale_input + quanti))
    echo "=== ${target}: replay di ${quanti} input (${ingressi[*]}) ==="

    if ! cargo +"${toolchain}" fuzz run "${options[@]}" "${target}" "${ingressi[@]}" -- \
        -runs=0 \
        "-rss_limit_mb=${rss_limit_mb}" \
        "-timeout=${timeout_s}" \
        -artifact_prefix="fuzz/artifacts/${target}/"; then
        falliti+=("${target}")
    fi
done

echo
if [ "${#falliti[@]}" -ne 0 ]; then
    echo "replay fallito su ${#falliti[@]} target: ${falliti[*]}" >&2
    exit 1
fi
echo "replay completato: ${totale_input} input rieseguiti su ${#targets[@]} target, nessun crash"
