#!/bin/bash
# Campagna riproducibile: cinque target libFuzzer coverage-guided più il fuzzer
# strutturato stabile. Pensato per l'immagine definita da fuzz/Dockerfile.
set -euo pipefail

cd /work

duration="${1:-3600}"
toolchain="${PLENORA_FUZZ_TOOLCHAIN:-nightly-2026-07-21}"
rss_limit_mb="${PLENORA_FUZZ_RSS_MB:-2048}"
max_len="${PLENORA_FUZZ_MAX_LEN:-65536}"
run_id="$(date -u +%Y%m%dT%H%M%SZ)"
log_dir="/work/fuzz/logs/${run_id}"
mkdir -p "${log_dir}" /work/fuzz-findings

echo "=== build e seed corpus ==="
cargo +"${toolchain}" build --release -p plenora-fuzz
./target/release/plenora-fuzz --export-corpus /work/fuzz/corpus
for target in from_wkb geojson_reader wkt_parse kml_reader shp_wkb; do
    cargo +"${toolchain}" fuzz build "${target}"
done

declare -A pids=()
declare -A logs=()

start_target() {
    local target="$1"
    local log="${log_dir}/${target}.log"
    logs["${target}"]="${log}"
    echo "=== start ${target}: ${duration}s ==="
    cargo +"${toolchain}" fuzz run "${target}" -- \
        "-max_total_time=${duration}" \
        "-rss_limit_mb=${rss_limit_mb}" \
        "-max_len=${max_len}" \
        "-timeout=15" \
        "-print_final_stats=1" \
        >"${log}" 2>&1 &
    pids["${target}"]=$!
}

start_structured() {
    local target="structured"
    local log="${log_dir}/${target}.log"
    logs["${target}"]="${log}"
    echo "=== start ${target}: ${duration}s ==="
    PLENORA_FUZZ_SECONDS="${duration}" \
    PLENORA_FUZZ_OUT="/work/fuzz-findings" \
        ./target/release/plenora-fuzz >"${log}" 2>&1 &
    pids["${target}"]=$!
}

for target in from_wkb geojson_reader wkt_parse kml_reader shp_wkb; do
    start_target "${target}"
done
start_structured

failed=0
for target in from_wkb geojson_reader wkt_parse kml_reader shp_wkb structured; do
    status=0
    wait "${pids[${target}]}" || status=$?
    echo "=== end ${target}: exit ${status} ==="
    tail -n 30 "${logs[${target}]}"
    if [[ "${status}" -ne 0 ]]; then
        failed=1
    fi
done

echo "=== artifacts ==="
find /work/fuzz/artifacts /work/fuzz-findings \
    -type f -printf '%p %s bytes\n' 2>/dev/null || true
echo "logs: ${log_dir}"
exit "${failed}"
