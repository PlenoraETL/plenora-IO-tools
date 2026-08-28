#!/usr/bin/env bash
# La misura di profondita' di un fuzz target, e nient'altro.
#
# # Perche' e' separata dal gate
#
# `scripts/check_profondita_fuzz.py` gira in CI e nel checkpoint: deve costare
# millisecondi e non deve pretendere ne' nightly ne' cargo-fuzz. Questa misura
# costa minuti, richiede la toolchain di fuzzing e la strumentazione di
# copertura. Sono due cose diverse, e tenerle in un file solo significherebbe o
# non misurare mai o misurare a ogni push.
#
# Si lancia **a mano**, quando cambia qualcosa dentro il perimetro dichiarato nel
# registro del bersaglio. Il gate dice quando: ricalcola l'impronta del perimetro
# e diventa rosso appena la misura invecchia.
#
# # Perche' non e' un passo del checkpoint
#
# Scriverebbe un file **tracciato** durante la corsa, e `albero_invariato`
# diventerebbe rosso: un checkpoint che modifica l'albero che sta qualificando
# non qualifica niente.
#
# # Uso
#
#     bash scripts/fuzz-profondita.sh shp_reader
#     bash scripts/fuzz-profondita.sh filegdb_reader
set -uo pipefail

cd "$(dirname "$0")/.."

TARGET="${1:-}"
if [ -z "${TARGET}" ]; then
    echo "uso: $0 <bersaglio>" >&2
    echo "i bersagli sono quelli dichiarati in scripts/check_profondita_fuzz.py" >&2
    exit 2
fi
# Il bersaglio dev'essere noto **al gate**, non a questo script: due elenchi di
# bersagli divergerebbero, e la misura finirebbe in un artefatto che nessuno
# rilegge.
if ! python3 scripts/check_profondita_fuzz.py "${TARGET}" --help > /dev/null 2>&1; then
    echo "bersaglio «${TARGET}» sconosciuto al gate della profondita'" >&2
    exit 2
fi

USCITA="${FUZZ_PROFONDITA_OUT:-/tmp/fuzz-profondita-${TARGET}}"
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
# derivati da un generatore: chiunque rifaccia questa misura parte dagli stessi
# byte.
#
# E' anche il limite inferiore giusto: se i requisiti sono raggiunti dai soli
# semi, lo sono su qualunque macchina.
SEMI="fuzz/seeds/${TARGET}"
if [ ! -d "${SEMI}" ]; then
    echo "nessun seme per ${TARGET}: non c'e' profondita' da misurare" >&2
    exit 1
fi
quanti=$(find "${SEMI}" -type f | wc -l | tr -d ' ')
if [ "${quanti}" -eq 0 ]; then
    echo "nessun seme per ${TARGET}: non c'e' profondita' da misurare" >&2
    exit 1
fi

echo "=============================================================="
echo "profondita' del target ${TARGET}"
echo "revisione: $(git rev-parse HEAD)"
echo "albero:    $(git status --porcelain | wc -l) file non committati"
echo "toolchain: ${TOOLCHAIN}"
echo "semi:      ${SEMI} (${quanti} input)"
echo "=============================================================="

# I dati precedenti di **questo** target se ne vanno prima di misurare: una
# misura fallita che lasciasse in piedi la precedente produrrebbe una
# profondita' di un altro albero.
#
# Solo quelli di questo target: sotto `fuzz/coverage/` ci sono file **tracciati**
# di altre misure, finiti in git a suo tempo, e cancellarli qui renderebbe sporco
# l'albero di chiunque lanci questo script.
rm -rf "fuzz/coverage/${TARGET}"
if ! cargo fuzz coverage "${TARGET}" "${SEMI}" > "${USCITA}/coverage.log" 2>&1; then
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
#
# La scelta **non** e' piu' qui, e non e' piu' il primo che riesce. Questa parte
# era `find | while read | break`: l'ordine lo dava il filesystem e l'arresto al
# primo successo rendeva invisibile per costruzione l'esistenza di un secondo
# binario compatibile e diverso. Le quattro condizioni che la chiudono --
# enumerazione ordinata, verifica di tutti, rifiuto se i compatibili non sono
# byte-identici, scelta canonica solo fra copie identiche -- stanno in un modulo
# con le proprie sonde, perche' in shell nessuna delle quattro si potrebbe
# violare in una prova.
JSON="${USCITA}/${TARGET}.functions.json"
LCOV="${USCITA}/${TARGET}.lcov"
rm -f "${USCITA}/selezione.log"
binario="$(python3 scripts/seleziona_binario_strumentato.py "${TARGET}" \
    --radice target \
    --radice fuzz/target \
    --llvm-cov "${LLVM_COV}" \
    --instr-profile "${PROFDATA}" \
    --log "${USCITA}/selezione.log")" || {
    echo "selezione del binario fallita (vedi ${USCITA}/selezione.log)" >&2
    exit 1
}
echo "binario: ${binario}"

# L'export delle funzioni si rifa' sul binario **scelto**: la selezione decide e
# non produce, se no il file sul disco sarebbe quello di un candidato qualunque
# fra quelli provati.
if ! "${LLVM_COV}" export "${binario}" \
    --instr-profile="${PROFDATA}" \
    --format=text \
    --skip-expansions \
    > "${JSON}" 2>"${USCITA}/export.log"; then
    echo "export delle funzioni fallito (vedi ${USCITA}/export.log)" >&2
    exit 1
fi

# Due proiezioni della **stessa** misura, non due misure: le funzioni servono ai
# requisiti che nominano una funzione -- comprese quelle delle crate esterne, che
# stanno nel registry di cargo -- e le righe a quelli che nominano un ramo dentro
# il nostro sorgente.
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

python3 scripts/check_profondita_fuzz.py "${TARGET}" \
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
exec python3 scripts/check_profondita_fuzz.py "${TARGET}"
