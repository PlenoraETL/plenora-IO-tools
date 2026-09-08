#!/usr/bin/env bash
# Prova la patch `arrow-dizionario-senza-dati` contro `apache/arrow-rs` main.
#
#   1. il riproduttore contro `main` non modificato: deve **panicare**;
#   2. con la patch: deve restituire un errore;
#   3. i test di `arrow-ipc` devono restare verdi.
#
#   docker run --rm -v <repo>:/work -v plenora-io-cargo:/usr/local/cargo/registry \
#       plenora-io-dev-198 bash /work/contributi-upstream/prova-arrow.sh
set -u
QUI="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$QUI/.." && pwd)"
export CARGO_TARGET_DIR=/tmp/contributo-arrow-target
SEME="$REPO/fuzz/seeds/ipc_reader/dizionario-senza-dati.arrow"

prepara() {
  local dir="$1"
  rm -rf "$dir"
  git clone -q --depth 1 https://github.com/apache/arrow-rs "$dir" 2>/dev/null || return 1
  mkdir -p "$dir/arrow-ipc/examples"
  cp "$SEME" "$dir/arrow-ipc/examples/dizionario-senza-dati.arrow"
  cat > "$dir/arrow-ipc/examples/riproduttore.rs" <<'RUST'
//! Un messaggio di dizionario senza il proprio `data`.
//!
//! `DictionaryBatch.data` e' facoltativo nella grammatica flatbuffer e
//! obbligatorio nel formato: `get_dictionary_values` lo apre con `unwrap`.
//! Senza la patch questo programma **panica**; con la patch stampa un errore.
use std::io::Cursor;

fn main() {
    let byte: &[u8] = include_bytes!("dizionario-senza-dati.arrow");
    println!("ingresso: {} byte", byte.len());
    match arrow_ipc::reader::FileReader::try_new(Cursor::new(byte), None) {
        Ok(_) => println!("RITORNATO ok"),
        Err(e) => println!("RITORNATO errore: {e}"),
    }
}
RUST
}

esegui() {
  local dir="$1" etichetta="$2"
  echo "--- $etichetta"
  (cd "$dir/arrow-ipc" && cargo run -q --example riproduttore 2>&1 | tail -4 | sed 's/^/    /')
}

echo "########## 1. main non modificato"
prepara /tmp/arrow-vergine || { echo "clone fallito: serve rete"; exit 1; }
esegui /tmp/arrow-vergine "main, senza patch"

echo
echo "########## 2. con la patch"
prepara /tmp/arrow-patchato
if patch -d /tmp/arrow-patchato -p1 --dry-run < "$QUI/arrow-dizionario-senza-dati/patch.diff" >/dev/null 2>&1; then
  patch -d /tmp/arrow-patchato -p1 < "$QUI/arrow-dizionario-senza-dati/patch.diff" >/dev/null
  echo "    patch applicata"
else
  echo "    la patch NON si applica"
  patch -d /tmp/arrow-patchato -p1 --dry-run < "$QUI/arrow-dizionario-senza-dati/patch.diff" 2>&1 | head -4 | sed 's/^/       /'
  exit 1
fi
esegui /tmp/arrow-patchato "main, con patch"

echo
echo "########## 3. i test di arrow-ipc, con la patch"
(cd /tmp/arrow-patchato && cargo test -q -p arrow-ipc 2>&1 | grep -E "^test result|^error|FAILED|panicked" | head -6 | sed 's/^/    /')
