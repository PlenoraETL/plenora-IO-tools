#!/bin/bash
# Campagna riproducibile: tutti i target libFuzzer coverage-guided più il fuzzer
# strutturato stabile. Pensato per l'immagine definita da fuzz/Dockerfile.
# La lista dei target è derivata dal manifest, non riscritta qui.
#
# I target girano tutti in parallelo, quindi il fabbisogno di memoria scala con
# il numero di target dichiarati. Il tetto RSS per processo non è più fisso: è
# derivato dalla memoria disponibile, così che il totale resti sotto di essa a
# qualunque numero di target. `PLENORA_FUZZ_RSS_MB` lo forza, se serve.
set -euo pipefail

cd /work

duration="${1:-3600}"
toolchain="${PLENORA_FUZZ_TOOLCHAIN:-nightly-2026-07-21}"
rss_limit_mb="${PLENORA_FUZZ_RSS_MB:-}"
max_len="${PLENORA_FUZZ_MAX_LEN:-65536}"
run_id="$(date -u +%Y%m%dT%H%M%SZ)"
log_dir="/work/fuzz/logs/${run_id}"
artifact_dir="/work/fuzz/artifacts/${run_id}"
finding_dir="/work/fuzz-findings/${run_id}"
mkdir -p "${log_dir}" "${artifact_dir}" "${finding_dir}"
mapfile -t targets < <(cargo +"${toolchain}" fuzz list)

# Il tetto RSS è per processo, ma i target girano tutti insieme: con un valore
# fisso il fabbisogno totale cresce con il numero di target e può superare la
# memoria della macchina. Sotto pressione il kernel non uccide il fuzzer, lo
# rallenta — e `-timeout` misura tempo di parete, non tempo di CPU. Un input che
# gira in decine di millisecondi finisce archiviato come `timeout-<sha>`, e un
# finding così non si riproduce: è esattamente il caso di xlsx_reader, 28 ms
# in isolamento contro i 15 s dichiarati dalla campagna.
#
# Deriviamo quindi il tetto dalla memoria disponibile. `PLENORA_FUZZ_RSS_MB`
# resta come scelta esplicita, per chi vuole forzare un valore.
processi=$(( ${#targets[@]} + 1 ))   # + il fuzzer strutturato
if [ -z "${rss_limit_mb}" ]; then
    disponibili_mb="$(awk '/^MemAvailable:/ { print int($2 / 1024) }' /proc/meminfo)"
    # Metà del disponibile, divisa fra i processi: l'altra metà copre page
    # cache e shadow map di AddressSanitizer, che non compaiono nell'RSS su cui
    # libFuzzer decide.
    rss_limit_mb=$(( disponibili_mb / 2 / processi ))
    # Sotto i 512 MiB un target strumentato con AddressSanitizer si ferma per
    # RSS prima di esplorare qualsiasi cosa, quindi il minimo vince sulla
    # divisione. A quel punto il totale torna a superare la memoria: è il
    # segnale che i target sono troppi per la macchina, non un valore da usare.
    if [ "${rss_limit_mb}" -lt 512 ]; then
        rss_limit_mb=512
        echo "=== attenzione: ${processi} processi non stanno in ${disponibili_mb} MiB disponibili ==="
    fi
fi
# Stampato perché un finding futuro sia interpretabile a posteriori: senza
# questo valore nel log non si distingue un OOM del target da uno indotto dalla
# campagna.
echo "=== tetto RSS: ${rss_limit_mb} MiB per processo x ${processi} processi ==="

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
