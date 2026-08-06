#!/bin/bash
# Campagna riproducibile: tutti i target libFuzzer coverage-guided più il fuzzer
# strutturato stabile. Pensato per l'immagine definita da fuzz/Dockerfile.
# La lista dei target è derivata dal manifest, non riscritta qui.
#
# I target girano tutti in parallelo: il fabbisogno di memoria scala con il
# numero di target dichiarati, non è più fisso. Con tredici target servono
# alcuni GB; `PLENORA_FUZZ_RSS_MB` resta il tetto per singolo processo.
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
mapfile -t targets < <(cargo +"${toolchain}" fuzz list)

echo "=== build e seed corpus ==="
cargo +"${toolchain}" build --release -p plenora-fuzz --locked
./target/release/plenora-fuzz --export-corpus /work/fuzz/corpus
# I semi versionati coprono i target su formato contenitore, dove un input
# casuale non supererebbe nemmeno il controllo del magic.
for target in "${targets[@]}"; do
    mkdir -p "/work/fuzz/corpus/${target}"
    if [ -d "/work/fuzz/seeds/${target}" ]; then
        cp -r "/work/fuzz/seeds/${target}/." "/work/fuzz/corpus/${target}/"
    fi
done
cargo +"${toolchain}" --locked fuzz build

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
