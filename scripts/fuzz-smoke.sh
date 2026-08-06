#!/bin/bash
# Smoke della campagna fuzz sotto AddressSanitizer.
#
# Non e' la campagna lunga (scripts/fuzz-campaign.sh): qui l'obiettivo e'
# dimostrare, a ogni revisione candidata, che ogni target compila strumentato e
# che i semi versionati piu' il corpus non producono crash. La durata per target
# e' volutamente breve; il valore sta nella copertura di TUTTI i target e nel
# fatto che il binario e' costruito con -Zsanitizer=address.
#
# Uso: scripts/fuzz-smoke.sh [secondi-per-target]
set -euo pipefail

cd "$(dirname "$0")/.."

duration="${1:-${PLENORA_FUZZ_SECONDS:-60}}"
rss_limit_mb="${PLENORA_FUZZ_RSS_MB:-2048}"
max_len="${PLENORA_FUZZ_MAX_LEN:-65536}"

# Fuori dalla CI la directory di build va spostata su un filesystem nativo:
# su un bind mount la build strumentata e' di un ordine di grandezza piu' lenta.
options=()
if [ -n "${PLENORA_FUZZ_TARGET_DIR:-}" ]; then
    options=(--target-dir "${PLENORA_FUZZ_TARGET_DIR}")
fi

# La lista dei target e' derivata dal manifest, non riscritta qui: un target
# nuovo entra nello smoke senza toccare questo script ne' la CI.
mapfile -t targets < <(cargo fuzz list)
if [ "${#targets[@]}" -eq 0 ]; then
    echo "nessun target fuzz dichiarato in fuzz/Cargo.toml" >&2
    exit 1
fi

echo "=== build strumentata (${#targets[@]} target) ==="
cargo fuzz build "${options[@]}"

# I semi versionati sono l'ingresso minimo perche' un target su formato
# contenitore superi il controllo del magic e raggiunga il parser vero.
for target in "${targets[@]}"; do
    mkdir -p "fuzz/corpus/${target}" "fuzz/artifacts/${target}"
    if [ -d "fuzz/seeds/${target}" ]; then
        cp -rf "fuzz/seeds/${target}/." "fuzz/corpus/${target}/"
    fi
done

failed=()
for target in "${targets[@]}"; do
    echo "=== ${target}: ${duration}s ==="
    if ! cargo fuzz run "${options[@]}" "${target}" -- \
        "-max_total_time=${duration}" \
        "-rss_limit_mb=${rss_limit_mb}" \
        "-max_len=${max_len}" \
        "-timeout=15" \
        "-print_final_stats=1" \
        "-artifact_prefix=fuzz/artifacts/${target}/"; then
        failed+=("${target}")
    fi
done

if [ "${#failed[@]}" -ne 0 ]; then
    echo "target con finding: ${failed[*]}" >&2
    exit 1
fi

echo "smoke fuzz completato senza finding su ${#targets[@]} target"
