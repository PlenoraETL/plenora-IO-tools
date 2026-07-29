#!/bin/bash
# Campagna riproducibile: sei target libFuzzer coverage-guided più il fuzzer
# strutturato stabile. Pensato per l'immagine definita da fuzz/Dockerfile.
set -euo pipefail

cd /work

duration="${1:-3600}"
toolchain="${PLENORA_FUZZ_TOOLCHAIN:-nightly-2026-07-21}"
rss_limit_mb="${PLENORA_FUZZ_RSS_MB:-2048}"
max_len="${PLENORA_FUZZ_MAX_LEN:-65536}"
run_id="$(date -u +%Y%m%dT%H%M%SZ)"
log_dir="/work/fuzz/logs/${run_id}"
artifact_dir="/work/fuzz/artifacts/${run_id}"
finding_dir="/work/fuzz-findings/${run_id}"
mkdir -p "${log_dir}" "${artifact_dir}" "${finding_dir}"
targets=(from_wkb geojson_reader wkt_parse kml_reader shp_wkb dxf_reader)

echo "=== build e seed corpus ==="
cargo +"${toolchain}" build --release -p plenora-fuzz --locked
./target/release/plenora-fuzz --export-corpus /work/fuzz/corpus
mkdir -p /work/fuzz/corpus/dxf_reader
cp /work/fuzz/seeds/dxf_reader/* /work/fuzz/corpus/dxf_reader/
for target in "${targets[@]}"; do
    cargo +"${toolchain}" --locked fuzz build "${target}"
done

declare -A pids=()
declare -A logs=()

start_target() {
    local target="$1"
    local log="${log_dir}/${target}.log"
    local artifacts="${artifact_dir}/${target}/"
    mkdir -p "${artifacts}"
    logs["${target}"]="${log}"
    echo "=== start ${target}: ${duration}s ==="
    cargo +"${toolchain}" --locked fuzz run "${target}" -- \
        "-max_total_time=${duration}" \
        "-rss_limit_mb=${rss_limit_mb}" \
        "-max_len=${max_len}" \
        "-timeout=15" \
        "-print_final_stats=1" \
        "-artifact_prefix=${artifacts}" \
        >"${log}" 2>&1 &
    pids["${target}"]=$!
}

start_structured() {
    local target="structured"
    local log="${log_dir}/${target}.log"
    logs["${target}"]="${log}"
    echo "=== start ${target}: ${duration}s ==="
    PLENORA_FUZZ_SECONDS="${duration}" \
    PLENORA_FUZZ_OUT="${finding_dir}" \
        ./target/release/plenora-fuzz >"${log}" 2>&1 &
    pids["${target}"]=$!
}

for target in "${targets[@]}"; do
    start_target "${target}"
done
start_structured

failed=0
for target in "${targets[@]}" structured; do
    status=0
    wait "${pids[${target}]}" || status=$?
    echo "=== end ${target}: exit ${status} ==="
    tail -n 30 "${logs[${target}]}"
    if [[ "${status}" -ne 0 ]]; then
        failed=1
    fi
done

echo "=== artifacts ==="
find "${artifact_dir}" "${finding_dir}" \
    -type f -printf '%p %s bytes\n' 2>/dev/null || true
echo "logs: ${log_dir}"
echo "artifacts: ${artifact_dir}"
echo "structured findings: ${finding_dir}"
exit "${failed}"
