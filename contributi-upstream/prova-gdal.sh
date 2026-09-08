#!/usr/bin/env bash
# Prova la patch `gdal-api-mancanti` contro `georust/gdal` master.
#
# Il container di sviluppo ha GDAL 3.6.2, mentre upstream master dichiara di
# supportare la 3.8 e successive e spedisce binding pre-generati dalla 3_8 alla
# 3_13. La costruzione passa quindi dalla feature `bindgen`, che genera i
# binding al volo dagli header installati -- libclang c'e'.
#
#   docker run --rm -v <repo>:/work -v plenora-io-cargo:/usr/local/cargo/registry \
#       plenora-io-dev-198 bash /work/contributi-upstream/prova-gdal.sh
set -u
QUI="$(cd "$(dirname "$0")" && pwd)"
export CARGO_TARGET_DIR=/tmp/gdal-target
echo "ambiente: GDAL $(gdal-config --version 2>/dev/null || echo '?'), rustc $(rustc --version | cut -d' ' -f2)"

BASE=/tmp/gdal-master
rm -rf "$BASE"
git clone -q --depth 1 https://github.com/georust/gdal "$BASE" || { echo "clone fallito: serve rete"; exit 1; }

echo
echo "########## 1. la patch si applica su un albero pulito"
if patch -d "$BASE" -p1 --dry-run < "$QUI/gdal-api-mancanti/patch.diff" >/dev/null 2>&1; then
  patch -d "$BASE" -p1 < "$QUI/gdal-api-mancanti/patch.diff" >/dev/null
  echo "    applicata"
else
  echo "    NON si applica"
  patch -d "$BASE" -p1 --dry-run < "$QUI/gdal-api-mancanti/patch.diff" 2>&1 | head -5 | sed 's/^/       /'
  exit 1
fi

echo
echo "########## 2. compila"
(cd "$BASE" && cargo check -p gdal 2>&1 \
  | sed 's/\x1b\[[0-9;]*m//g' | grep -E "^(error|warning: unused)" | head -12 | sed 's/^/    /')
(cd "$BASE" && cargo check -p gdal >/dev/null 2>&1)
echo "    check_exit=$?"

echo
echo "########## 3. le due API rispondono"
mkdir -p "$BASE/examples"
cat > "$BASE/examples/prova_api.rs" <<'RUST'
//! Le due API proposte, esercitate.
fn main() {
    // I percorsi di PROJ: si leggono i default, si sostituiscono, si rileggono.
    let prima = gdal::spatial_ref::get_proj_search_paths();
    println!("percorsi di partenza: {prima:?}");
    gdal::spatial_ref::set_proj_search_paths(&["/tmp/proj-uno", "/tmp/proj-due"])
        .expect("i percorsi si impostano");
    let dopo = gdal::spatial_ref::get_proj_search_paths();
    println!("percorsi impostati:   {dopo:?}");
    assert_eq!(dopo, vec!["/tmp/proj-uno".to_string(), "/tmp/proj-due".to_string()]);
    println!("OK: set/get dei percorsi di PROJ");
}
RUST
(cd "$BASE" && cargo run -q --example prova_api 2>&1 | tail -4 | sed 's/^/    /')

echo
echo "########## 4. i test del crate"
(cd "$BASE" && cargo test -q -p gdal 2>&1 \
  | grep -E "^test result|^error|FAILED" | head -6 | sed 's/^/    /')
