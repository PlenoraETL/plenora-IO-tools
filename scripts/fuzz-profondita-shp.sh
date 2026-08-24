#!/usr/bin/env bash
# La misura di profondita' del target `shp_reader`, e nient'altro.
#
# # Perche' e' separata dal gate
#
# `scripts/check_profondita_fuzz_shp.py` gira in CI e nel checkpoint: deve
# costare millisecondi e non deve pretendere ne' nightly ne' cargo-fuzz. Questa
# misura costa minuti, richiede la toolchain di fuzzing e la strumentazione di
# copertura. Sono due cose diverse, e tenerle in un file solo significherebbe o
# non misurare mai o misurare a ogni push.
#
# Si lancia **a mano**, quando cambia qualcosa dentro il perimetro dichiarato in
# `assurance/registries/profondita-fuzz-shapefile.json`. Il gate dice quando:
# ricalcola l'impronta del perimetro e diventa rosso appena la misura invecchia.
#
# # Perche' non e' un passo del checkpoint
#
# Scriverebbe un file **tracciato** durante la corsa, e `albero_invariato`
# diventerebbe rosso: un checkpoint che modifica l'albero che sta qualificando
# non qualifica niente.
#
# # Uso
#
#     bash scripts/fuzz-profondita-shp.sh
set -uo pipefail

cd "$(dirname "$0")/.."

TARGET=shp_reader
USCITA="${FUZZ_PROFONDITA_OUT:-/tmp/fuzz-profondita-shp}"
mkdir -p "${USCITA}"

# Lo stesso pin di fuzz-replay.sh, fuzz-smoke.sh e fuzz-coverage.sh: la
# strumentazione di copertura e' proprio la parte che cambia fra release del
# compilatore, e due misure con nightly diversi non sono confrontabili.
TOOLCHAIN="${PLENORA_FUZZ_TOOLCHAIN:-nightly-2026-07-21}"
export RUSTUP_TOOLCHAIN="${TOOLCHAIN}"

LLVM_COV="$(rustc "+${TOOLCHAIN}" --print target-libdir)/../bin/llvm-cov"
if [ ! -x "${LLVM_COV}" ]; then
    echo "llvm-cov assente in ${TOOLCHAIN}: manca il componente llvm-tools." >&2
    exit 2
fi

# Si misura sui **soli semi versionati**, non su corpus e artefatti.
#
# Corpus e artefatti sono locali alla macchina: crescono a ogni campagna e non
# sono in git. Includerli darebbe una profondita' che nessun altro puo'
# riprodurre, e il numero registrato nell'artefatto direbbe piu' di quanto un
# controllo indipendente possa confermare. I semi invece sono versionati e
# derivati da `scripts/genera_semi_shp.py`: chiunque rifaccia questa misura
# parte dagli stessi byte.
#
# E' anche il limite inferiore giusto: se i requisiti sono raggiunti dai soli
# semi, lo sono su qualunque macchina.
sorgenti=("fuzz/seeds/${TARGET}")
if [ ! -d "${sorgenti[0]}" ]; then
    echo "nessun seme per ${TARGET}: non c'e' profondita' da misurare" >&2
    exit 1
fi
quanti=$(find "${sorgenti[0]}" -type f | wc -l | tr -d ' ')
if [ "${quanti}" -eq 0 ]; then
    echo "nessun seme per ${TARGET}: non c'e' profondita' da misurare" >&2
    exit 1
fi
echo "  semi: ${sorgenti[0]} (${quanti} input)"

echo "=============================================================="
echo "profondita' del target ${TARGET}"
echo "revisione: $(git rev-parse HEAD)"
echo "albero:    $(git status --porcelain | wc -l) file non committati"
echo "toolchain: ${TOOLCHAIN}"
echo "input:     ${quanti}"
echo "=============================================================="

# I dati precedenti di **questo** target se ne vanno prima di misurare: una
# misura fallita che lasciasse in piedi la precedente produrrebbe una
# profondita' di un altro albero, ed e' esattamente il difetto che l'impronta
# del perimetro esiste per vedere -- ma dopo, e per caso.
#
# Solo quelli di questo target: sotto `fuzz/coverage/` ci sono file **tracciati**
# di altre misure, finiti in git a suo tempo, e cancellarli qui renderebbe sporco
# l'albero di chiunque lanci questo script.
rm -rf "fuzz/coverage/${TARGET}"
if ! cargo fuzz coverage "${TARGET}" "${sorgenti[@]}" > "${USCITA}/coverage.log" 2>&1; then
    echo "cargo fuzz coverage fallito -- ${USCITA}/coverage.log" >&2
    exit 1
fi

PROFDATA="fuzz/coverage/${TARGET}/coverage.profdata"
if [ ! -f "${PROFDATA}" ]; then
    echo "profdata assente dopo la misura" >&2
    exit 1
fi

# Il binario strumentato non sta dove il nome suggerisce, e il percorso e' un
# dettaglio di cargo-fuzz: si prendono i candidati e si tiene quello i cui dati
# combaciano **davvero** con il profdata. Nella corsa del 2026-08-21 il primo
# candidato con il nome giusto era la build con AddressSanitizer, che di
# copertura non ne ha.
JSON="${USCITA}/${TARGET}.functions.json"
LCOV="${USCITA}/${TARGET}.lcov"
binario=""
while IFS= read -r candidato; do
    if "${LLVM_COV}" export "${candidato}" \
        --instr-profile="${PROFDATA}" \
        --format=text \
        --skip-expansions \
        > "${JSON}" 2>"${USCITA}/export.log"; then
        binario="${candidato}"
        break
    fi
done < <(find target fuzz/target -type f -name "${TARGET}" ! -name '*.d' 2>/dev/null)

if [ -z "${binario}" ]; then
    echo "nessun binario combacia con il profdata (vedi ${USCITA}/export.log)" >&2
    exit 1
fi
echo "binario: ${binario}"

# Due proiezioni della **stessa** misura, non due misure: le funzioni servono ai
# requisiti che nominano una funzione -- comprese quelle di `shapefile` e
# `dbase`, che stanno nel registry di cargo -- e le righe a quelli che nominano
# un ramo dentro il nostro sorgente.
if ! "${LLVM_COV}" export "${binario}" \
    --instr-profile="${PROFDATA}" \
    --format=lcov \
    > "${LCOV}" 2>>"${USCITA}/export.log"; then
    echo "export lcov fallito (vedi ${USCITA}/export.log)" >&2
    exit 1
fi
if [ ! -s "${JSON}" ] || [ ! -s "${LCOV}" ]; then
    echo "una delle due proiezioni e' vuota: la misura non e' leggibile" >&2
    exit 1
fi

python3 scripts/check_profondita_fuzz_shp.py \
    --registra "${JSON}" \
    --lcov "${LCOV}" \
    --input "${quanti}" || exit 1

# I dati di profiling di questo target se ne vanno: sono output di build, e
# `fuzz/coverage/` non e' ignorato da git. Lasciarli li' renderebbe sporco
# l'albero, e un checkpoint di livello 2 pretende un albero pulito -- quindi la
# misura impedirebbe la corsa che deve qualificarla.
rm -rf "fuzz/coverage/${TARGET}"

# La registrazione scrive; il gate rilegge. Sono due programmi nello stesso
# file, e farli girare in fila e' il solo modo di accorgersi subito se il primo
# ha scritto qualcosa che il secondo rifiuta.
exec python3 scripts/check_profondita_fuzz_shp.py
