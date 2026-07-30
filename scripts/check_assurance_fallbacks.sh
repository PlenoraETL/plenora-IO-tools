#!/usr/bin/env bash
set -eu

# Registro conservativo dell'intero workspace: include anche i moduli
# #[cfg(test)] e i target non distribuibili. Ogni nuova occorrenza richiede una
# revisione H-01 e l'aggiornamento esplicito del registro.
expected='
driver-csv 3
driver-dxf 14
driver-filegdb 3
driver-geojson 1
driver-geoparquet 4
driver-gpkg 4
driver-ipc 2
driver-kml 2
driver-shp 2
driver-xls 1
plenora-io-model 1
plenora-io-core 2
plenora-io-cli 19
plenora-bench 22
plenora-fuzz 6
'

actual_total=0
while read -r crate expected_count; do
    if [ -z "${crate}" ]; then
        continue
    fi
    if command -v rg >/dev/null 2>&1; then
        actual_count=$(
            rg -o 'unwrap_or(_else|_default)?\(' "crates/${crate}" -g '*.rs' |
                wc -l |
                tr -d ' '
        )
    else
        actual_count=$(
            grep -R -E -o --include='*.rs' 'unwrap_or(_else|_default)?\(' "crates/${crate}" |
                wc -l |
                tr -d ' '
        )
    fi
    if [ "${actual_count}" != "${expected_count}" ]; then
        echo "${crate}: fallback registrati=${expected_count}, trovati=${actual_count}" >&2
        exit 1
    fi
    actual_total=$((actual_total + actual_count))
done <<EOF
${expected}
EOF

if [ "${actual_total}" -ne 86 ]; then
    echo "totale fallback del workspace inatteso: ${actual_total}" >&2
    exit 1
fi

echo "fallback assurance verificati: ${actual_total}"
