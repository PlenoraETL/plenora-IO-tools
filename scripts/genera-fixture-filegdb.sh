#!/usr/bin/env bash
# La fixture FileGDB del target `filegdb_reader`: la produce, e ne dimostra la
# riproducibilita'.
#
# # Perche' e' uno script a se'
#
# E' il **solo** posto in cui `ogr2ogr` viene invocato, e vuole GDAL installato.
# Il gate che la rilegge -- `genera_fixture_filegdb.py --verifica` -- costa
# millisecondi e gira ovunque; questa parte costa secondi e gira dove c'e' la
# toolchain. Sono due cose diverse, e tenerle in un file solo significherebbe o
# non verificare mai la fixture o pretendere GDAL a ogni push.
#
# # Che cosa dimostra
#
# `ogr2ogr` e' deterministico su questo input tranne che per i GUID che conia
# per il dataset. Lo script non lo dichiara: genera la fixture **due** volte e
# lascia che sia il confronto fra le due a dire quali byte sono coniati, poi
# pretende che la fixture committata differisca solo li'.
#
# # Uso
#
#     bash scripts/genera-fixture-filegdb.sh            # verifica
#     bash scripts/genera-fixture-filegdb.sh --scrivi   # rigenera e riscrive
set -uo pipefail

cd "$(dirname "$0")/.."

SORGENTE="fuzz/fixtures/filegdb/citta.geojson"
LAVORO="${FILEGDB_FIXTURE_OUT:-/tmp/fixture-filegdb}"
NOME_LAYER=citta
CRS=EPSG:4326

if ! command -v ogr2ogr > /dev/null 2>&1; then
    echo "ogr2ogr assente: la fixture si produce dove c'e' GDAL" >&2
    exit 2
fi
if [ ! -f "${SORGENTE}" ]; then
    echo "sorgente della fixture assente: ${SORGENTE}" >&2
    exit 1
fi

# La versione di GDAL fa parte della riproducibilita': due versioni diverse
# scrivono tabelle di metadati diverse, e la differenza non sarebbe un byte
# coniato.
VERSIONE="$(ogr2ogr --version)"
ATTESA="${FILEGDB_GDAL_ATTESA:-GDAL 3.6.2}"
case "${VERSIONE}" in
    "${ATTESA}"*) ;;
    *)
        echo "GDAL e' «${VERSIONE}», la fixture e' stata prodotta con «${ATTESA}»." >&2
        echo "Le differenze che ne seguono non sarebbero byte coniati." >&2
        exit 1
        ;;
esac

rm -rf "${LAVORO}"
mkdir -p "${LAVORO}"

# Due corse indipendenti: la seconda esiste **solo** per far emergere i byte
# coniati. Senza, la tolleranza andrebbe scritta a mano, e resterebbe ferma il
# giorno in cui GDAL ne coniasse uno in piu'.
for corsa in 1 2; do
    if ! ogr2ogr -f OpenFileGDB "${LAVORO}/corsa${corsa}.gdb" "${SORGENTE}" \
        -nln "${NOME_LAYER}" -a_srs "${CRS}" > "${LAVORO}/ogr2ogr-${corsa}.log" 2>&1; then
        echo "ogr2ogr fallito alla corsa ${corsa} -- ${LAVORO}/ogr2ogr-${corsa}.log" >&2
        exit 1
    fi
done

echo "GDAL:   ${VERSIONE}"
echo "lavoro: ${LAVORO}"

if [ "${1:-}" = "--scrivi" ]; then
    python3 scripts/genera_fixture_filegdb.py --scrivi "${LAVORO}/corsa1.gdb" || exit 1
fi

python3 scripts/genera_fixture_filegdb.py --confronta \
    "${LAVORO}/corsa1.gdb" "${LAVORO}/corsa2.gdb" || exit 1

# Il confronto e' appena avvenuto: se ne registra il verbale, cosi' il gate
# leggero' -- che non ha GDAL e non puo' rigenerare niente -- ha qualcosa da
# rileggere invece di dover credere alla prosa.
python3 scripts/genera_fixture_filegdb.py --registra \
    "${LAVORO}/corsa1.gdb" "${LAVORO}/corsa2.gdb" --gdal "${VERSIONE}" || exit 1

# La forma dell'archivio la rilegge il gate leggero, che e' quello che gira in
# CI: farlo girare anche qui significa accorgersi subito se cio' che abbiamo
# appena scritto non e' cio' che il target sapra' rileggere.
exec python3 scripts/genera_fixture_filegdb.py --verifica
