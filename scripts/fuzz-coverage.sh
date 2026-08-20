#!/usr/bin/env bash
# Copertura raggiunta dal **replay deterministico** di un fuzz target.
#
# # Perche' esiste
#
# La misura di copertura del checkpoint gira sui test unitari. Un ramo d'errore
# che solo un file ostile raggiunge risulta scoperto anche quando il corpus del
# fuzzer lo attraversa a ogni corsa.
#
# Senza questa misura la scelta e' fra due errori: dichiarare coperto cio' che
# non si e' misurato, o scrivere test per rami che il fuzzing gia' esercita. La
# diagnostica differenziale del 2026-08-21 ha lasciato 45 gruppi di funzioni in
# questa incertezza.
#
# # Che cosa NON dice
#
# Che un ramo sia **raggiunto** dal fuzzing non dice che il suo contratto sia
# verificato. Un fuzz target controlla che non si panichi; non controlla che
# `category`, `phase`, `code` e `retry` siano quelli giusti. Il risultato di
# questo script separa «non raggiunto» da «raggiunto», e la seconda categoria
# resta da verificare semanticamente -- ma raggruppata, non riga per riga.
#
# # Proprieta' della misura
#
# * **deterministica**: `cargo fuzz coverage` rigioca il corpus, non muta;
# * **separata**: i dati stanno in `fuzz/coverage/`, non in
#   `target/llvm-cov-target/` -- non si fondono con la copertura dei test;
# * **pulita**: i dati precedenti sono rimossi prima, per la stessa ragione per
#   cui il checkpoint fa `cargo llvm-cov clean` (vedi INFRA-5: una misura
#   fallita che lascia in piedi la precedente produce verdi di un altro albero);
# * **dichiarata**: per ogni target stampa il corpus usato e quanti input ha
#   attraversato, perche' una percentuale senza il suo denominatore non si
#   rilegge.
#
# # Uso
#
#     bash scripts/fuzz-coverage.sh [target ...]
#
# Senza argomenti misura i tre target dei driver in classificazione.

set -u

cd "$(dirname "$0")/.."

USCITA="${FUZZ_COVERAGE_OUT:-/tmp/fuzz-coverage}"
mkdir -p "${USCITA}"

# La toolchain e' scelta qui, non ereditata: la strumentazione richiede nightly,
# ed e' la stessa scelta esplicita che fanno fuzz-smoke.sh e fuzz-replay.sh.
# Lo stesso pin di fuzz-replay.sh e fuzz-smoke.sh: due misure sulla stessa
# revisione con nightly diversi non sono confrontabili, e la strumentazione
# di copertura e' proprio la parte che cambia fra release del compilatore.
TOOLCHAIN="${PLENORA_FUZZ_TOOLCHAIN:-nightly-2026-07-21}"
export RUSTUP_TOOLCHAIN="${TOOLCHAIN}"

ESCLUSIONI='(^|/)(plenora-bench|plenora-fuzz|plenora-io-cli)/src/.*\.rs$|\.cargo/registry|/rustc/|^/usr/'

# `llvm-cov` viene dalla **stessa** toolchain che ha costruito il target: quella
# di stable leggerebbe un formato di profdata che potrebbe non essere il suo.
LLVM_COV="$(rustc "+${TOOLCHAIN}" --print target-libdir)/../bin/llvm-cov"
if [ ! -x "${LLVM_COV}" ]; then
    echo "llvm-cov assente in ${TOOLCHAIN}: manca il componente llvm-tools." >&2
    exit 2
fi

if [ "$#" -gt 0 ]; then
    target=("$@")
else
    target=(shp_wkb xlsx_reader gpkg_reader)
fi

echo "=============================================================="
echo "copertura dal replay deterministico"
echo "revisione: $(git rev-parse HEAD)"
echo "albero:    $(git status --porcelain | wc -l) file non committati"
echo "toolchain: ${TOOLCHAIN}"
echo "uscita:    ${USCITA}"
echo "=============================================================="

# I dati precedenti se ne vanno prima di misurare.
rm -rf fuzz/coverage
esito_globale=0

for nome in "${target[@]}"; do
    echo
    echo "--- ${nome} ---------------------------------------------"

    sorgenti=()
    quanti=0
    for cartella in "fuzz/seeds/${nome}" "fuzz/corpus/${nome}" "fuzz/artifacts/${nome}"; do
        if [ -d "${cartella}" ]; then
            n=$(find "${cartella}" -type f | wc -l | tr -d ' ')
            if [ "${n}" -gt 0 ]; then
                sorgenti+=("${cartella}")
                quanti=$((quanti + n))
                echo "  corpus: ${cartella} (${n} input)"
            fi
        fi
    done
    if [ "${#sorgenti[@]}" -eq 0 ]; then
        echo "  nessun input: target saltato" >&2
        esito_globale=1
        continue
    fi
    echo "  input totali: ${quanti}"

    log="${USCITA}/${nome}.log"
    if ! cargo fuzz coverage "${nome}" "${sorgenti[@]}" > "${log}" 2>&1; then
        echo "  ROSSO: cargo fuzz coverage e' fallito — ${log}" >&2
        esito_globale=1
        continue
    fi

    profdata="fuzz/coverage/${nome}/coverage.profdata"
    if [ ! -f "${profdata}" ]; then
        echo "  ROSSO: profdata assente dopo la misura" >&2
        esito_globale=1
        continue
    fi

    # Il binario strumentato **non** sta sotto `fuzz/target`: `cargo fuzz
    # coverage` costruisce in `target/<triple>/coverage/<triple>/release/`. Il
    # percorso e' un dettaglio di cargo-fuzz e puo' cambiare, quindi non lo si
    # indovina: si prendono i candidati e si tiene quello i cui dati di
    # copertura **combaciano davvero** con il profdata.
    #
    # E' la differenza fra «ho trovato un file con il nome giusto» e «ho trovato
    # il binario che ha prodotto questa misura»: il primo, nella corsa del
    # 2026-08-21, era la build con AddressSanitizer, che di copertura non ne ha.
    lcov="${USCITA}/${nome}.lcov"
    binario=""
    while IFS= read -r candidato; do
        if "${LLVM_COV}" export "${candidato}" \
            --instr-profile="${profdata}" \
            --format=lcov \
            --ignore-filename-regex="${ESCLUSIONI}" \
            > "${lcov}" 2>"${USCITA}/${nome}.export.log"; then
            binario="${candidato}"
            break
        fi
    done < <(find target fuzz/target -type f -name "${nome}" ! -name '*.d' 2>/dev/null)

    if [ -z "${binario}" ]; then
        echo "  ROSSO: nessun binario combacia con il profdata" >&2
        echo "         (ultimo tentativo in ${USCITA}/${nome}.export.log)" >&2
        esito_globale=1
        continue
    fi
    echo "  binario: ${binario}"
    if [ ! -s "${lcov}" ]; then
        echo "  ROSSO: report lcov vuoto" >&2
        esito_globale=1
        continue
    fi
    echo "  report: ${lcov} ($(grep -c '^SF:' "${lcov}") file)"
done

echo
if [ "${esito_globale}" -ne 0 ]; then
    echo "misura incompleta: vedi i ROSSO sopra." >&2
    exit 1
fi
echo "copertura da replay prodotta in ${USCITA}"
