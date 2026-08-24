#!/usr/bin/env bash
# Misura il confine di AddressSanitizer nel target `filegdb_reader`.
#
# # Due proprieta' distinte, due misure
#
# **Strumentazione**: `libgdal.so` e' compilata con `-fsanitize=address`? Si
# misura sulla libreria, contando i simboli del runtime che porta. Zero vuol
# dire non strumentata; centinaia vorrebbero dire il contrario.
#
# **Feedback di copertura**: il fuzzer vede che cosa succede dentro GDAL? Si
# misura sui contatori che libFuzzer carica all'avvio.
#
# Sono cose diverse, e la prima stesura di questo script le confondeva: inferiva
# la strumentazione dai contatori, che non la riguardano. Un binario puo' avere
# copertura senza sanitizer e sanitizer senza copertura.
#
# # Perche' si misura invece di dichiararlo
#
# «GDAL non e' strumentata» e' una frase che invecchia: basta che qualcuno la
# costruisca da sorgente, o che il link cambi, e la frase resta scritta mentre
# il fatto cambia. Qui i numeri vengono dal binario e dalla libreria, e il gate
# rimisura la libreria locale a ogni esecuzione invece di credere a questo file.
#
# # Uso
#
#     bash scripts/asan-filegdb.sh
set -uo pipefail

cd "$(dirname "$0")/.."

TARGET=filegdb_reader
USCITA="${ASAN_FILEGDB_OUT:-/tmp/asan-filegdb}"
mkdir -p "${USCITA}"

TOOLCHAIN="${PLENORA_FUZZ_TOOLCHAIN:-nightly-2026-07-21}"
export RUSTUP_TOOLCHAIN="${TOOLCHAIN}"

# Il binario dev'essere quello **strumentato**, non quello della copertura: sono
# due build diverse, e solo la prima porta il runtime del sanitizer.
if ! cargo fuzz build "${TARGET}" > "${USCITA}/build.log" 2>&1; then
    echo "build strumentata fallita -- ${USCITA}/build.log" >&2
    exit 1
fi
BINARIO="fuzz/target/x86_64-unknown-linux-gnu/release/${TARGET}"
if [ ! -x "${BINARIO}" ]; then
    echo "binario strumentato assente: ${BINARIO}" >&2
    exit 1
fi

echo "=============================================================="
echo "confine AddressSanitizer del target ${TARGET}"
echo "revisione: $(git rev-parse HEAD)"
echo "binario:   ${BINARIO}"
echo "=============================================================="

# 1. Quale GDAL, e da dove. `ldd` risolve il soname come lo risolvera' il
#    processo: e' cio' che verra' caricato, non cio' che il manifest dichiara.
RIGA_GDAL="$(ldd "${BINARIO}" 2>/dev/null | grep -i 'libgdal' | head -1)"
SONAME="$(echo "${RIGA_GDAL}" | awk '{print $1}')"
PERCORSO="$(echo "${RIGA_GDAL}" | awk '{print $3}')"
if [ -z "${SONAME}" ] || [ ! -f "${PERCORSO}" ]; then
    echo "il binario non collega una libgdal risolvibile: non sta esercitando FileGDB" >&2
    exit 1
fi

# 2. **La** misura della strumentazione: i simboli del runtime dentro la
#    libreria. Non i contatori, che sono un'altra proprieta'; non un'estensione
#    di file, che non dice come e' stata compilata.
SIMBOLI_IN_GDAL="$(nm -D "${PERCORSO}" 2>/dev/null | grep -c '__asan')"
SIMBOLI_IN_GDAL_STATICI="$(nm "${PERCORSO}" 2>/dev/null | grep -c '__asan')"
if [ "${SIMBOLI_IN_GDAL_STATICI}" -gt "${SIMBOLI_IN_GDAL}" ]; then
    SIMBOLI_IN_GDAL="${SIMBOLI_IN_GDAL_STATICI}"
fi

# Quale libreria, in due modi indipendenti: il build-id la identifica, il digest
# la fissa. Servono al gate per dire se sta guardando la stessa.
BUILD_ID="$(readelf -n "${PERCORSO}" 2>/dev/null | sed -n 's/.*Build ID: \([0-9a-f]*\).*/\1/p' | head -1)"
DIGEST="$(sha256sum "${PERCORSO}" | awk '{print $1}')"

# Dentro l'albero di build? **Derivato** dal percorso, non scritto a mano: se un
# giorno GDAL venisse costruita qui dentro, la risposta cambierebbe da sola.
RADICE="$(pwd -P)"
case "$(readlink -f "${PERCORSO}")" in
    "${RADICE}"/*) DENTRO_ALBERO=true ;;
    *) DENTRO_ALBERO=false ;;
esac

printf "" > "${USCITA}/vuoto"

# 3. Il runtime del sanitizer e' nel **nostro** binario? Lo si chiede al
#    runtime, non alla tabella dei simboli: rustc lo lega staticamente e i suoi
#    simboli restano locali, quindi `nm -D` non ne trova nemmeno uno. E'
#    l'errore che la prima stesura ha commesso, misurando `false` su un binario
#    strumentato.
#
#    L'uscita si cattura prima di cercarci dentro: con `help=1` il runtime
#    stampa e poi esce con stato non zero, e sotto `pipefail` sarebbe quello
#    stato a decidere l'`if`.
AIUTO="$(ASAN_OPTIONS=help=1 "${BINARIO}" "${USCITA}/vuoto" 2>&1 || true)"
if printf '%s' "${AIUTO}" | grep -q "Available flags for AddressSanitizer"; then
    ASAN_NEL_BINARIO=true
else
    ASAN_NEL_BINARIO=false
fi
SIMBOLI_NEL_BINARIO="$(nm "${BINARIO}" 2>/dev/null | grep -c '__asan')"

# 4. Quanti moduli portano contatori, e quanti contatori. E' la contabilita' di
#    libFuzzer, non la nostra stima. Un modulo solo vuol dire che nessuna
#    libreria condivisa ne porta: il fuzzer e' cieco oltre il confine.
BANNER="$("${BINARIO}" "${USCITA}/vuoto" 2>&1 | grep 'inline 8-bit counters' | head -1)"
MODULI="$(echo "${BANNER}" | sed -n 's/.*Loaded \([0-9]*\) modules.*/\1/p')"
CONTATORI="$(echo "${BANNER}" | sed -n 's/.*(\([0-9]*\) inline 8-bit counters).*/\1/p')"
if [ -z "${MODULI}" ] || [ -z "${CONTATORI}" ]; then
    echo "libFuzzer non ha stampato la contabilita' dei contatori" >&2
    echo "riga letta: ${BANNER}" >&2
    exit 1
fi

# 5. Quanti sorgenti **di GDAL** compaiono nei dati di copertura. Non «quanti
#    file C/C++»: un conteggio generico direbbe zero anche il giorno in cui
#    GDAL fosse strumentata e un'altra libreria no. I sorgenti di GDAL si
#    riconoscono dal percorso.
rm -rf "fuzz/coverage/${TARGET}"
if ! cargo fuzz coverage "${TARGET}" "fuzz/seeds/${TARGET}" > "${USCITA}/coverage.log" 2>&1; then
    echo "cargo fuzz coverage fallito -- ${USCITA}/coverage.log" >&2
    exit 1
fi
LLVM_COV="$(rustc --print target-libdir)/../bin/llvm-cov"
PROFDATA="fuzz/coverage/${TARGET}/coverage.profdata"
LCOV="${USCITA}/${TARGET}.lcov"
binario_copertura=""
while IFS= read -r candidato; do
    if "${LLVM_COV}" export "${candidato}" --instr-profile="${PROFDATA}" \
        --format=lcov > "${LCOV}" 2>/dev/null; then
        binario_copertura="${candidato}"
        break
    fi
done < <(find target fuzz/target -type f -name "${TARGET}" ! -name '*.d' 2>/dev/null)
if [ -z "${binario_copertura}" ]; then
    echo "nessun binario combacia con il profdata" >&2
    exit 1
fi
SORGENTI="$(grep '^SF:' "${LCOV}" | sed 's/^SF://')"
# Un file `.rs` sotto `vendor/gdal` e' il **wrapper**, che e' nostro ed e'
# strumentato: escluderlo e' cio' che distingue le due cose.
GDAL_STRUMENTATI="$(printf '%s\n' "${SORGENTI}" | grep -i 'gdal' | grep -vE '\.rs$' | wc -l | tr -d ' ')"
CPP_STRUMENTATI="$(printf '%s\n' "${SORGENTI}" | grep -cE '\.(c|cc|cpp|cxx|h|hpp)$' || true)"
WRAPPER_RUST="$(printf '%s\n' "${SORGENTI}" | grep -c 'vendor/gdal.*\.rs$' || true)"
rm -rf "fuzz/coverage/${TARGET}"

cat > "${USCITA}/misura.json" <<JSON
{
  "libreria_collegata": {
    "soname": "${SONAME}",
    "percorso_risolto": "${PERCORSO}",
    "build_id": "${BUILD_ID}",
    "sha256": "${DIGEST}",
    "come_e_stata_misurata": "ldd sul binario strumentato risolve il soname come lo risolvera' il processo; build-id da readelf -n e digest da sha256sum sulla libreria risolta"
  },
  "simboli_asan_nella_libreria": ${SIMBOLI_IN_GDAL},
  "simboli_asan_nel_binario": ${SIMBOLI_NEL_BINARIO},
  "runtime_asan_nel_binario": ${ASAN_NEL_BINARIO},
  "libreria_gdal_dentro_l_albero_di_build": ${DENTRO_ALBERO},
  "moduli_con_contatori": ${MODULI},
  "contatori_di_copertura": ${CONTATORI},
  "file_sorgente_gdal_strumentati": ${GDAL_STRUMENTATI},
  "file_sorgente_c_cpp_strumentati": ${CPP_STRUMENTATI},
  "file_del_wrapper_rust_strumentati": ${WRAPPER_RUST},
  "come_sono_stati_contati": "la strumentazione della libreria contando i simboli __asan che porta, con nm sulla tabella dinamica e su quella statica -- e' la proprieta' che dice se e' stata compilata con -fsanitize=address, e non si inferisce dai contatori; i moduli e i contatori dalla riga che libFuzzer stampa all'avvio, che e' una proprieta' distinta e riguarda il feedback del fuzzer; i sorgenti di GDAL riconoscendoli dal percorso ed escludendo i .rs, che sono il wrapper e sono nostri; la presenza del runtime nel nostro binario chiedendola al runtime con ASAN_OPTIONS=help=1, perche' i suoi simboli sono locali e nella tabella dinamica non ce n'e' nessuno; se la libreria stia dentro l'albero di build confrontando il percorso reale con la radice del repository, invece di scriverlo a mano"
}
JSON

echo "soname:        ${SONAME}"
echo "risolto in:    ${PERCORSO}"
echo "build-id:      ${BUILD_ID}"
echo "simboli __asan nella libreria: ${SIMBOLI_IN_GDAL}"
echo "simboli __asan nel binario:    ${SIMBOLI_NEL_BINARIO}"
echo "runtime nel binario:           ${ASAN_NEL_BINARIO}"
echo "dentro l'albero di build:      ${DENTRO_ALBERO}"
echo "moduli con contatori:          ${MODULI}"
echo "contatori:                     ${CONTATORI}"
echo "sorgenti di GDAL strumentati:  ${GDAL_STRUMENTATI}"
echo "wrapper Rust strumentato:      ${WRAPPER_RUST} file"

python3 scripts/check_asan_filegdb.py --registra "${USCITA}/misura.json" || exit 1
exec python3 scripts/check_asan_filegdb.py
