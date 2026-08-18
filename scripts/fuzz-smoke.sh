#!/bin/bash
# Smoke della campagna fuzz sotto AddressSanitizer.
#
# Non e' la campagna lunga (scripts/fuzz-campaign.sh): qui l'obiettivo e'
# dimostrare, a ogni revisione candidata, che ogni target compila strumentato e
# che i semi versionati piu' il corpus non producono crash. La durata per target
# e' volutamente breve; il valore sta nella copertura di TUTTI i target e nel
# fatto che il binario e' costruito con -Zsanitizer=address.
#
# Uso: scripts/fuzz-smoke.sh [--include-quarantined] [--seconds N] [target ...]
#
# Senza target li esegue **tutti**, che e' il comportamento su cui la CI conta.
# Con un sottoinsieme esegue solo quelli, con la stessa interfaccia posizionale
# di scripts/fuzz-replay.sh: le due si usano insieme, e ricordarsi due
# convenzioni diverse e' il modo di lanciare la cosa sbagliata.
#
# La durata era il primo argomento posizionale; ora e' `--seconds` (oppure
# PLENORA_FUZZ_SECONDS). Nessun chiamante nel repository la passava
# posizionalmente — la CI invoca lo script senza argomenti — quindi il posto
# resta libero per i target, che e' cio' che serve.
set -euo pipefail

cd "$(dirname "$0")/.."

include_quarantined=0
seconds_flag=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --include-quarantined)
            include_quarantined=1
            shift
            ;;
        --seconds)
            if [ "$#" -lt 2 ]; then
                echo "--seconds richiede un valore" >&2
                exit 2
            fi
            seconds_flag="$2"
            shift 2
            ;;
        --)
            shift
            break
            ;;
        -*)
            echo "opzione sconosciuta: $1" >&2
            echo "uso: $0 [--include-quarantined] [--seconds N] [target ...]" >&2
            exit 2
            ;;
        *)
            break
            ;;
    esac
done

# La toolchain e' scelta **qui**, non ereditata dall'ambiente. -Zsanitizer
# richiede nightly, e senza una scelta esplicita lo script userebbe cio' che
# capita: rust-toolchain.toml lo porterebbe su stable 1.92.0, dove la build
# strumentata fallisce con "only accepted on nightly"; un RUSTUP_TOOLCHAIN
# impostato altrove lo porterebbe su un nightly qualsiasi, e due esecuzioni
# della stessa revisione produrrebbero binari diversi. Il pin e' lo stesso di
# scripts/toolchain-pins.env, verificato da check_toolchain_pins.py.
toolchain="${PLENORA_FUZZ_TOOLCHAIN:-nightly-2026-07-21}"
if ! rustup toolchain list | grep -q "^${toolchain}"; then
    echo "toolchain ${toolchain} non installata: e' quella che serve a -Zsanitizer=address" >&2
    echo "installala con: rustup toolchain install ${toolchain} --profile minimal" >&2
    exit 1
fi
echo "toolchain fuzz: ${toolchain}"

duration="${seconds_flag:-${PLENORA_FUZZ_SECONDS:-60}}"
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
mapfile -t dichiarati < <(cargo +"${toolchain}" fuzz list)
if [ "${#dichiarati[@]}" -eq 0 ]; then
    echo "nessun target fuzz dichiarato in fuzz/Cargo.toml" >&2
    exit 1
fi

# Un sottoinsieme richiesto viene **verificato** contro il manifest prima di
# costruire qualunque cosa. Un nome sbagliato deve fermarsi subito e dire quali
# sono i nomi buoni: senza il controllo, `cargo fuzz run` fallirebbe comunque,
# ma dopo una build strumentata da minuti e con un messaggio che non elenca le
# alternative.
if [ "$#" -gt 0 ]; then
    targets=("$@")
    ignoti=()
    for richiesto in "${targets[@]}"; do
        trovato=0
        for dichiarato in "${dichiarati[@]}"; do
            if [ "${richiesto}" = "${dichiarato}" ]; then
                trovato=1
                break
            fi
        done
        if [ "${trovato}" -eq 0 ]; then
            ignoti+=("${richiesto}")
        fi
    done
    if [ "${#ignoti[@]}" -ne 0 ]; then
        echo "target non dichiarati in fuzz/Cargo.toml: ${ignoti[*]}" >&2
        echo "dichiarati: ${dichiarati[*]}" >&2
        exit 2
    fi
    echo "sottoinsieme richiesto: ${#targets[@]} di ${#dichiarati[@]} target"
else
    targets=("${dichiarati[@]}")
fi

# I target con un finding aperto sono dichiarati in fuzz/quarantine.txt: si
# compilano sempre, si eseguono solo su richiesta esplicita. Il debito resta
# visibile a ogni esecuzione.
quarantined=()
if [ -f fuzz/quarantine.txt ]; then
    mapfile -t quarantined < <(
        grep -vE '^\s*(#|$)' fuzz/quarantine.txt | awk '{print $1}'
    )
fi

is_quarantined() {
    local candidate="$1" entry
    for entry in ${quarantined[@]+"${quarantined[@]}"}; do
        if [ "${entry}" = "${candidate}" ]; then
            return 0
        fi
    done
    return 1
}

# `cargo fuzz build` senza nome costruisce **tutti** i target, anche quando ne
# e' stato richiesto un sottoinsieme, e va bene cosi': "ogni target compila
# strumentato" e' meta' del valore di questo smoke, e la build incrementale la
# rende quasi gratis. La riga lo dice, invece di annunciare il numero del
# sottoinsieme mentre ne costruisce tredici.
echo "=== build strumentata (tutti i ${#dichiarati[@]} target dichiarati) ==="
cargo +"${toolchain}" fuzz build "${options[@]}"

if [ "${#quarantined[@]}" -ne 0 ]; then
    echo
    echo "=== ATTENZIONE: ${#quarantined[@]} target in quarantena (finding aperti) ==="
    grep -vE '^\s*(#|$)' fuzz/quarantine.txt
    if [ "${include_quarantined}" -eq 1 ]; then
        echo "--include-quarantined: verranno eseguiti comunque."
    else
        echo "Compilati sotto AddressSanitizer ma NON eseguiti in questo smoke."
    fi
    echo
fi

# I semi versionati sono l'ingresso minimo perche' un target su formato
# contenitore superi il controllo del magic e raggiunga il parser vero.
for target in "${targets[@]}"; do
    mkdir -p "fuzz/corpus/${target}" "fuzz/artifacts/${target}"
    if [ -d "fuzz/seeds/${target}" ]; then
        cp -rf "fuzz/seeds/${target}/." "fuzz/corpus/${target}/"
    fi
done

failed=()
skipped=0
for target in "${targets[@]}"; do
    if [ "${include_quarantined}" -eq 0 ] && is_quarantined "${target}"; then
        echo "=== ${target}: saltato (quarantena) ==="
        skipped=$((skipped + 1))
        continue
    fi
    echo "=== ${target}: ${duration}s ==="
    if ! cargo +"${toolchain}" fuzz run "${options[@]}" "${target}" -- \
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

eseguiti=$(( ${#targets[@]} - skipped ))
if [ "${#targets[@]}" -eq "${#dichiarati[@]}" ]; then
    echo "smoke fuzz completato senza finding su ${eseguiti} target eseguiti (${skipped} in quarantena, comunque compilati)"
else
    # Il sottoinsieme e' detto nell'esito, non solo nell'invocazione: una riga
    # che dice «completato senza finding» senza dire su quanti target invita a
    # leggerla come se fossero tutti.
    echo "smoke fuzz completato senza finding su ${eseguiti} dei ${#dichiarati[@]} target dichiarati (sottoinsieme richiesto: ${targets[*]}; ${skipped} in quarantena, comunque compilati)"
fi
