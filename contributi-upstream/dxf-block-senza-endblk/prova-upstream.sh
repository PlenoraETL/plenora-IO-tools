#!/usr/bin/env bash
# Prova il riproduttore e la patch contro `dxf` upstream, non contro il fork.
#
#   1. il riproduttore contro `dxf 0.6.1` di crates.io, non modificato:
#      deve NON ritornare entro venti secondi;
#   2. la stessa versione con `patch.diff` applicata e `prova.rs` aggiunta:
#      deve ritornare, e i test del crate devono restare verdi.
#
# Da eseguire nel container di sviluppo, che ha rete e cargo:
#   docker run --rm -v <repo>:/work plenora-io-dev-198 \
#       bash /work/contributi-upstream/dxf-block-senza-endblk/prova-upstream.sh
set -u
QUI="$(cd "$(dirname "$0")" && pwd)"
export CARGO_TARGET_DIR=/tmp/contributo-dxf-target
RIPRODUTTORE='0\nSECTION\n2\nBLOCKS\n0\nBLOCK\n2\nsenza-endblk\n0\nENDSEC\n0\nEOF\n'

prepara_consumatore() {
  local dir="$1" dipendenza="$2"
  rm -rf "$dir"; mkdir -p "$dir/src"
  cat > "$dir/Cargo.toml" <<TOML
[package]
name = "prova_dxf"
version = "0.0.0"
edition = "2021"
[dependencies]
$dipendenza
[workspace]
TOML
  printf 'use std::io::Cursor;\nfn main() {\n    let ingresso: &[u8] = b"%b";\n    match dxf::Drawing::load(&mut Cursor::new(ingresso)) {\n        Ok(d) => println!("ok: {} blocchi", d.blocks().count()),\n        Err(e) => println!("errore: {e}"),\n    }\n}\n' "$RIPRODUTTORE" > "$dir/src/main.rs"
}

echo "########## 1. il crate pubblicato, non modificato"
prepara_consumatore /tmp/dxf-vergine 'dxf = "=0.6.1"'
(cd /tmp/dxf-vergine && cargo build -q 2>&1 | tail -2)
timeout 20 "$CARGO_TARGET_DIR/debug/prova_dxf"
if [ $? -eq 124 ]; then
  echo "   ESITO: non ritorna entro 20 secondi -- il difetto c'e'"
else
  echo "   ESITO: ritornato -- il difetto NON si riproduce, la segnalazione va rivista"
fi

echo
echo "########## 2. lo stesso crate con la patch"
SORG=/tmp/dxf-patchato
rm -rf "$SORG"; mkdir -p "$SORG"
curl -sSL "https://static.crates.io/crates/dxf/dxf-0.6.1.crate" -o /tmp/dxf.crate \
  && tar -xzf /tmp/dxf.crate -C "$SORG" --strip-components=1
if ! patch -d "$SORG" -p1 --dry-run < "$QUI/patch.diff" >/dev/null 2>&1; then
  echo "   la patch NON si applica: rivedere patch.diff"
  patch -d "$SORG" -p1 --dry-run < "$QUI/patch.diff" 2>&1 | head -5
  exit 1
fi
patch -d "$SORG" -p1 < "$QUI/patch.diff" >/dev/null
echo "   patch applicata"

# La prova, dentro il `mod tests` di src/block.rs.
python3 - "$SORG/src/block.rs" "$QUI/prova.rs" <<'PY'
import io, re, sys
sorgente, prova = sys.argv[1], sys.argv[2]
testo = io.open(sorgente, encoding="utf-8").read()
corpo = io.open(prova, encoding="utf-8").read()
# solo i due test, senza i commenti introduttivi del file
corpo = corpo[corpo.index("#[test]"):]
ancora = "mod tests {\n"
assert testo.count(ancora) == 1
testo = testo.replace(
    ancora,
    ancora + "    use crate::code_pair_iter::DirectCodePairIter;\n\n"
    + "\n".join("    " + r if r.strip() else r for r in corpo.splitlines()) + "\n",
)
io.open(sorgente, "w", encoding="utf-8").write(testo)
print("   prova innestata in src/block.rs")
PY

prepara_consumatore /tmp/dxf-consumatore "dxf = { path = \"$SORG\" }"
(cd /tmp/dxf-consumatore && cargo build -q 2>&1 | tail -3)
timeout 20 "$CARGO_TARGET_DIR/debug/prova_dxf"
if [ $? -eq 124 ]; then
  echo "   ESITO: NON ritorna -- la patch non corregge"
else
  echo "   ESITO: ritornato -- la patch corregge"
fi

echo
echo "########## 3. i test del crate, con la patch e la prova nuova"
(cd "$SORG" && cargo test -q 2>&1 | grep -E "^test result|^error|FAILED|panicked" | head -8)
