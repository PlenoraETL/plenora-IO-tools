#!/usr/bin/env bash
# Prova le due patch shapefile contro `master`, non contro il nostro fork.
#
# Per ciascuna: il riproduttore deve essere accettato dal crate non
# modificato -- che e' il difetto -- e rifiutato con la patch. Poi i test del
# crate devono restare verdi.
#
#   docker run --rm -v <repo>:/work plenora-io-dev-198 \
#       bash /work/contributi-upstream/prova-shapefile.sh
set -u
QUI="$(cd "$(dirname "$0")" && pwd)"
export CARGO_TARGET_DIR=/tmp/contributo-shp-target
U="https://github.com/tmontaigu/shapefile-rs"

prepara() {
  local dir="$1"
  rm -rf "$dir"
  git clone -q --depth 1 "$U" "$dir" 2>/dev/null || return 1
  # Il riproduttore, come esempio: costruisce i due file ostili in memoria e
  # dice se il reader li accetta.
  mkdir -p "$dir/examples"
  cp "$QUI/riproduttore-shapefile.rs" "$dir/examples/riproduttore.rs"
}

esegui() {
  local dir="$1" etichetta="$2"
  echo "--- $etichetta"
  (cd "$dir" && cargo run -q --example riproduttore 2>&1 | sed 's/^/    /')
}

echo "########## il crate non modificato"
prepara /tmp/shp-vergine || { echo "clone fallito: serve rete"; exit 1; }
esegui /tmp/shp-vergine "master, senza patch"

echo
echo "########## con le due patch"
prepara /tmp/shp-patchato
for p in shapefile-lunghezza-negativa shapefile-record-nullo; do
  if patch -d /tmp/shp-patchato -p1 --dry-run < "$QUI/$p/patch.diff" >/dev/null 2>&1; then
    patch -d /tmp/shp-patchato -p1 < "$QUI/$p/patch.diff" >/dev/null
    echo "    patch applicata: $p"
  else
    echo "    la patch NON si applica: $p"
    patch -d /tmp/shp-patchato -p1 --dry-run < "$QUI/$p/patch.diff" 2>&1 | head -4 | sed 's/^/       /'
    exit 1
  fi
done
esegui /tmp/shp-patchato "master, con patch"

echo
echo "########## i test del crate, con le patch"
(cd /tmp/shp-patchato && cargo test -q 2>&1 | grep -E "^test result|^error|FAILED|panicked" | head -8 | sed 's/^/    /')
