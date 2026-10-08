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
# capita: rust-toolchain.toml lo porterebbe su stable 1.98.1, dove la build
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

# Lo stack di un esaurimento di memoria si legge solo simbolizzato: senza
# `llvm-symbolizer` AddressSanitizer stampa indirizzi, e il classificatore non
# ha una firma da confrontare -- ogni esaurimento diventa illeggibile, cioe'
# rosso, anche quello gia' registrato. Si cerca quindi un simbolizzatore prima
# di correre, e si dice quale. Se non c'e' lo si dice, e il rosso resta: e' la
# direzione sicura.
if [ -z "${ASAN_SYMBOLIZER_PATH:-}" ]; then
    simbolizzatore="$(command -v llvm-symbolizer || true)"
    if [ -z "${simbolizzatore}" ]; then
        simbolizzatore="$(ls /usr/bin/llvm-symbolizer-* 2>/dev/null | sort -V | tail -n 1 || true)"
    fi
    if [ -n "${simbolizzatore}" ]; then
        export ASAN_SYMBOLIZER_PATH="${simbolizzatore}"
    fi
fi
if [ -n "${ASAN_SYMBOLIZER_PATH:-}" ]; then
    echo "simbolizzatore: ${ASAN_SYMBOLIZER_PATH}"
else
    echo "ATTENZIONE: nessun llvm-symbolizer: un esaurimento di memoria sara' illeggibile, cioe' rosso" >&2
fi
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
# sottoinsieme mentre li costruisce tutti.
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

# --- la classificazione del crash ------------------------------------------
#
# Prima un crash faceva fallire lo smoke e basta. Un finding gia' tracciato a
# monte lo faceva fallire ogni volta, e l'unica via era la quarantena -- che e'
# per **bersaglio**, cioe' smetteva di esplorarlo del tutto.
#
# Ora l'uscita di ogni corsa si conserva e si classifica. Tre esiti, tre
# insiemi, e nessuno dei tre si confonde con gli altri:
#
#   * nessun crash    -- il bersaglio ha finito il proprio tempo;
#   * crash compatibile con un finding noto -- la firma corrisponde a una voce
#                        di `assurance/registries/finding-noti-fuzz.json`.
#                        **Compatibile, non identico**: due difetti diversi
#                        possono dare lo stesso errore nello stesso modulo, e la
#                        firma non li separa. Non fa fallire lo smoke, ma il
#                        bersaglio **si e\' fermato li\'**: libFuzzer non riparte
#                        dopo un crash. Non entra fra quelli che hanno finito, e
#                        l\'input viene conservato con il suo referto perche\' la
#                        classificazione si possa riesaminare;
#   * finding nuovo   -- rosso, come prima.
#
# Un crash che il classificatore non riesce a leggere e' rosso anch'esso: una
# corsa fallita senza panico riconoscibile e' un guasto, non un finding noto.
failed=()
noti=()
# `bersaglio=id` per ogni bersaglio fermato a un finding noto: l'arresto si
# legge con la voce che l'ha riconosciuto, nel riepilogo e nel verbale.
voci=()
skipped=0
uscite=$(mktemp -d)
trap 'rm -rf "${uscite}"' EXIT

for target in "${targets[@]}"; do
    if [ "${include_quarantined}" -eq 0 ] && is_quarantined "${target}"; then
        echo "=== ${target}: saltato (quarantena) ==="
        skipped=$((skipped + 1))
        continue
    fi
    echo "=== ${target}: ${duration}s ==="
    uscita="${uscite}/${target}.txt"
    if cargo +"${toolchain}" fuzz run "${options[@]}" "${target}" -- \
        "-max_total_time=${duration}" \
        "-rss_limit_mb=${rss_limit_mb}" \
        "-max_len=${max_len}" \
        "-timeout=15" \
        "-print_final_stats=1" \
        "-artifact_prefix=fuzz/artifacts/${target}/" 2>&1 | tee "${uscita}"; then
        continue
    fi
    # `pipefail` non e' attivo qui: l'esito della corsa e' quello di `cargo`,
    # non di `tee`, e si rilegge da PIPESTATUS.
    if [ "${PIPESTATUS[0]}" -eq 0 ]; then
        continue
    fi
    voce_nota="${uscite}/${target}.voce"
    python3 "$(dirname "$0")/classifica_finding_fuzz.py" "${target}" \
        --uscita "${uscita}" \
        --conserva "assurance/evidence/finding-fuzz" \
        --voce-nota "${voce_nota}"
    case "$?" in
        3)
            noti+=("${target}")
            voci+=("${target}=$(cat "${voce_nota}")")
            ;;
        *) failed+=("${target}") ;;
    esac
done

# Il verbale della corsa, e perche' esiste.
#
# Lo smoke esce 0 anche quando un bersaglio si e' fermato a un crash noto: e'
# voluto, perche' lo sviluppo prosegua sugli altri. Quello 0 pero' non deve
# diventare «campagna completata» piu' in la' nella catena -- la CI e il passo
# `fuzz_smoke` del checkpoint leggono l'esito, non la riga stampata.
#
# Il verbale separa i due stati e li rende leggibili da un gate. La
# qualificazione finale lo rilegge con
# `classifica_finding_fuzz.py --verifica-campagna`, che e' rossa se qualcuno si
# e' fermato.
# «Finito» vuol dire **una** cosa: il bersaglio ha consumato il proprio
# tempo senza fermarsi. Chi si e' fermato a un crash noto non ci sta, e
# nemmeno chi e' fallito su un finding nuovo -- quello si e' fermato pure
# lui, e contarlo fra i completi renderebbe il verbale piu' generoso della
# corsa che descrive.
finiti=()
for target in "${targets[@]}"; do
    fermo=0
    for gia in ${noti[@]+"${noti[@]}"} ${failed[@]+"${failed[@]}"}; do
        [ "${gia}" = "${target}" ] && fermo=1
    done
    if [ "${include_quarantined}" -eq 0 ] && is_quarantined "${target}"; then
        fermo=1
    fi
    [ "${fermo}" -eq 0 ] && finiti+=("${target}")
done

# Il verbale si scrive **prima** dell'uscita rossa: una corsa che ha
# trovato un finding nuovo e' comunque una corsa avvenuta, e cancellarne
# la traccia lascerebbe la qualificazione a rileggere il verbale di quella
# precedente.
python3 "$(dirname "$0")/classifica_finding_fuzz.py" \
    --scrivi-verbale "${duration}" \
    --finiti ${finiti[@]+"${finiti[@]}"} \
    --fermati ${noti[@]+"${noti[@]}"} \
    --voci ${voci[@]+"${voci[@]}"} \
    --falliti ${failed[@]+"${failed[@]}"} \
    --dichiarati ${dichiarati[@]+"${dichiarati[@]}"}

if [ "${#failed[@]}" -ne 0 ]; then
    echo "target con finding: ${failed[*]}" >&2
    exit 1
fi

eseguiti=$(( ${#targets[@]} - skipped ))
if [ "${#noti[@]}" -ne 0 ]; then
    # La riga non puo' dire «completato»: i bersagli fermati a un finding noto
    # non hanno esplorato il tempo che restava, e una riga che li contasse fra
    # i completi direbbe di una campagna piu' di quanto sia successo.
    echo "smoke fuzz: ${#finiti[@]} target hanno finito il proprio tempo, ${#noti[@]} si sono fermati a un crash COMPATIBILE con un finding noto (bersaglio=voce: ${voci[*]}), ${skipped} in quarantena, comunque compilati. Chi si e' fermato NON ha esplorato il tempo restante, e il verbale in assurance/evidence/fuzz-smoke-ultima.json lo dice al gate della qualificazione."
    exit 0
fi

if [ "${#targets[@]}" -eq "${#dichiarati[@]}" ]; then
    echo "smoke fuzz completato senza finding su ${eseguiti} target eseguiti (${skipped} in quarantena, comunque compilati)"
else
    # Il sottoinsieme e' detto nell'esito, non solo nell'invocazione: una riga
    # che dice «completato senza finding» senza dire su quanti target invita a
    # leggerla come se fossero tutti.
    echo "smoke fuzz completato senza finding su ${eseguiti} dei ${#dichiarati[@]} target dichiarati (sottoinsieme richiesto: ${targets[*]}; ${skipped} in quarantena, comunque compilati)"
fi
