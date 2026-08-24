#!/usr/bin/env bash
# Misura il confine di AddressSanitizer nel target `filegdb_reader`.
#
# # Perche' si misura invece di dichiararlo
#
# «GDAL non e' strumentata» e' una frase che invecchia: basta che qualcuno
# costruisca GDAL da sorgente dentro l'immagine, o che il link diventi statico,
# e la frase resta scritta mentre il fatto cambia. Qui i numeri vengono dal
# binario: quale `libgdal` e' collegata e da dove, quanti moduli portano
# contatori di copertura, quanti contatori ci sono, quanti file sorgente di GDAL
# compaiono nei dati di copertura.
#
# Il gate `scripts/check_asan_filegdb.py` li rilegge e pretende che raccontino
# il confine che la sua prosa descrive.
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
if [ -z "${SONAME}" ]; then
    echo "il binario non collega libgdal: non sta esercitando FileGDB" >&2
    exit 1
fi

printf "" > "${USCITA}/vuoto"

# 2. Il runtime del sanitizer e' collegato? Lo si chiede **al runtime**, non
#    alla tabella dei simboli: con `-Zsanitizer=address` rustc lo lega
#    staticamente e i suoi simboli restano locali, quindi `nm -D` non ne trova
#    nemmeno uno -- ed e' esattamente l'errore che la prima stesura di questo
#    script ha commesso, misurando `false` su un binario strumentato.
#
#    `ASAN_OPTIONS=help=1` fa stampare al runtime l'elenco delle proprie
#    opzioni: se risponde, c'e'. E' una prova di comportamento, non di forma.
#    L'uscita si cattura **prima** di cercarci dentro. Con `help=1` il runtime
#    stampa e poi esce con stato non zero, e sotto `pipefail` sarebbe quello
#    stato a decidere l'`if` -- non il `grep` che ha trovato la riga. E' il
#    secondo modo in cui questa misura ha detto `false` su un binario
#    strumentato.
AIUTO="$(ASAN_OPTIONS=help=1 "${BINARIO}" "${USCITA}/vuoto" 2>&1 || true)"
if printf '%s' "${AIUTO}" | grep -q "Available flags for AddressSanitizer"; then
    ASAN=true
else
    ASAN=false
fi
# Corroborazione, non prova: quanti simboli del runtime stanno nel binario.
SIMBOLI_ASAN="$(nm "${BINARIO}" 2>/dev/null | grep -ci asan)"

# 3. Quanti moduli portano contatori, e quanti contatori. Il numero lo stampa
#    libFuzzer all'avvio: e' la sua contabilita', non la nostra stima.
BANNER="$("${BINARIO}" "${USCITA}/vuoto" 2>&1 | grep 'inline 8-bit counters' | head -1)"
MODULI="$(echo "${BANNER}" | sed -n 's/.*Loaded \([0-9]*\) modules.*/\1/p')"
CONTATORI="$(echo "${BANNER}" | sed -n 's/.*(\([0-9]*\) inline 8-bit counters).*/\1/p')"
if [ -z "${MODULI}" ] || [ -z "${CONTATORI}" ]; then
    echo "libFuzzer non ha stampato la contabilita' dei contatori" >&2
    echo "riga letta: ${BANNER}" >&2
    exit 1
fi

# 4. Quanti file sorgente di GDAL compaiono nei dati di copertura. Zero e' il
#    fatto centrale: se GDAL fosse strumentata, i suoi `.cpp` sarebbero li'.
#    Si misura sulla build di copertura, che e' l'unica che produce un profdata.
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
# I sorgenti di GDAL hanno estensioni C/C++ e stanno sotto i suoi alberi. Un
# file `.rs` sotto `vendor/gdal` e' il **wrapper**, che e' nostro ed e'
# strumentato: contarlo come GDAL confonderebbe le due cose.
GDAL_STRUMENTATI="$(grep '^SF:' "${LCOV}" | sed 's/^SF://' \
    | grep -E '\.(c|cc|cpp|cxx|h|hpp)$' | wc -l | tr -d ' ')"
rm -rf "fuzz/coverage/${TARGET}"

cat > "${USCITA}/misura.json" <<JSON
{
  "libreria_collegata": {
    "soname": "${SONAME}",
    "percorso_risolto": "${PERCORSO}",
    "come_e_stata_misurata": "ldd sul binario strumentato: risolve il soname come lo risolvera' il processo"
  },
  "libreria_gdal_dentro_l_albero_di_build": false,
  "runtime_asan_collegato": ${ASAN},
  "simboli_del_runtime_asan": ${SIMBOLI_ASAN},
  "moduli_con_contatori": ${MODULI},
  "contatori_di_copertura": ${CONTATORI},
  "file_sorgente_gdal_strumentati": ${GDAL_STRUMENTATI},
  "come_sono_stati_contati": "i moduli e i contatori dalla riga che libFuzzer stampa all'avvio; i file di GDAL contando i sorgenti C/C++ presenti nell'export lcov della build di copertura. Un file .rs sotto vendor/gdal e' il wrapper, che e' nostro ed e' strumentato; la presenza del runtime chiedendola al runtime con ASAN_OPTIONS=help=1, perche' i suoi simboli sono locali e nella tabella dinamica non ce n'e' nessuno."
}
JSON

echo "soname:     ${SONAME}"
echo "risolto in: ${PERCORSO}"
echo "asan:       ${ASAN} (${SIMBOLI_ASAN} simboli del runtime)"
echo "moduli:     ${MODULI}"
echo "contatori:  ${CONTATORI}"
echo "file C/C++ strumentati: ${GDAL_STRUMENTATI}"

python3 scripts/check_asan_filegdb.py --registra "${USCITA}/misura.json" || exit 1
exec python3 scripts/check_asan_filegdb.py
