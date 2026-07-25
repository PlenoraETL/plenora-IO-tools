#!/usr/bin/env bash
# Oracolo di interop (Fase 2B): gate bidirezionale contro GDAL.
#  - i file scritti dai NOSTRI driver devono essere letti correttamente da GDAL
#  - i file scritti da GDAL devono essere letti correttamente dai NOSTRI driver
# Da eseguire nell'immagine plenora-gdb-dev (GDAL 3.6+) con /work montato.
# Il binario CLI dev'essere gia' compilato in /work/target/release/plenora-io.
set -uo pipefail

BIN=/work/target/release/plenora-io
WORK=$(mktemp -d)
cd "$WORK"

fail=0
pass=0
check() { # descr, testo, atteso
  if printf '%s' "$2" | grep -qF "$3"; then
    printf '  OK   %s\n' "$1"; pass=$((pass+1))
  else
    printf '  FAIL %s  (atteso: %s)\n' "$1" "$3"; fail=$((fail+1))
  fi
}

cat > in.geojson <<'EOF'
{"type":"FeatureCollection","features":[
{"type":"Feature","geometry":{"type":"Point","coordinates":[12.5,45.9]},"properties":{"id":1,"nome":"Roma","pop":2800000}},
{"type":"Feature","geometry":{"type":"Point","coordinates":[9.19,45.46]},"properties":{"id":2,"nome":"Milano","pop":1400000}},
{"type":"Feature","geometry":{"type":"Point","coordinates":[7.68,45.07]},"properties":{"id":3,"nome":"Torino","pop":870000}}
]}
EOF

echo "=== GDAL $(ogrinfo --version) ==="
echo "== BIN: $($BIN --version) =="

echo "--- direzione A: NOSTRO writer -> GDAL reader ---"

# gpkg
if $BIN convert in.geojson ours.gpkg >/dev/null 2>err.txt; then
  I=$(ogrinfo -al -so ours.gpkg 2>&1)
  check "gpkg: GDAL conta 3 feature" "$I" "Feature Count: 3"
  F=$(ogrinfo -al ours.gpkg 2>&1)
  check "gpkg: attributo testo (Roma)"   "$F" "Roma"
  check "gpkg: attributo intero (2800000)" "$F" "2800000"
  check "gpkg: geometria (POINT 12.5 45.9)" "$F" "POINT (12.5 45.9)"
else
  echo "  FAIL convert->gpkg: $(cat err.txt)"; fail=$((fail+1))
fi

# csv (WKT); GDAL individua la geometria via GEOM_POSSIBLE_NAMES
if $BIN convert in.geojson ours.csv >/dev/null 2>err.txt; then
  C=$(ogrinfo -al -oo GEOM_POSSIBLE_NAMES=geometry -oo KEEP_GEOM_COLUMNS=NO ours.csv 2>&1)
  check "csv: GDAL legge attributo (Torino)" "$C" "Torino"
  check "csv: GDAL conta 3 feature" "$C" "Feature Count: 3"
else
  echo "  FAIL convert->csv: $(cat err.txt)"; fail=$((fail+1))
fi

# shapefile (write multi-file .shp/.shx/.dbf/.prj)
if $BIN convert in.geojson ours.shp >/dev/null 2>err.txt; then
  S=$(ogrinfo -al ours.shp 2>&1)
  check "shp: GDAL conta 3 feature" "$S" "Feature Count: 3"
  check "shp: attributo (Torino)" "$S" "Torino"
  check "shp: geometria (POINT 7.68 45.07)" "$S" "POINT (7.68 45.07)"
  test -f ours.prj && check "shp: .prj scritto (WGS 84)" "$(cat ours.prj)" "WGS 84"
else
  echo "  FAIL convert->shp: $(cat err.txt)"; fail=$((fail+1))
fi

# kml (Placemark + ExtendedData)
if $BIN convert in.geojson ours.kml >/dev/null 2>err.txt; then
  K=$(ogrinfo -al ours.kml 2>&1)
  check "kml: GDAL conta 3 feature" "$K" "Feature Count: 3"
  check "kml: geometria (POINT)" "$K" "POINT"
else
  echo "  FAIL convert->kml: $(cat err.txt)"; fail=$((fail+1))
fi

# dxf (geometria come entita', attributi in LossReport)
if $BIN convert in.geojson ours.dxf >/dev/null 2>err.txt; then
  D=$(ogrinfo -al ours.dxf 2>&1)
  check "dxf: GDAL conta 3 feature" "$D" "Feature Count: 3"
else
  echo "  FAIL convert->dxf: $(cat err.txt)"; fail=$((fail+1))
fi

# xlsx (tabellare + geometria WKT)
if $BIN convert in.geojson ours.xlsx >/dev/null 2>err.txt; then
  X=$(ogrinfo -al -oo GEOM_POSSIBLE_NAMES=geometry ours.xlsx 2>&1)
  check "xlsx: GDAL legge attributo (Roma)" "$X" "Roma"
else
  echo "  FAIL convert->xlsx: $(cat err.txt)"; fail=$((fail+1))
fi

# filegdb (tier GDB via GDAL) — solo se la CLI è compilata con gdal-backend
if $BIN convert in.geojson ours.gdb >/dev/null 2>err.txt; then
  G=$(ogrinfo -al ours.gdb 2>&1)
  check "filegdb: GDAL conta 3 feature" "$G" "Feature Count: 3"
  check "filegdb: attributo (Roma)" "$G" "Roma"
  # il nostro reader streaming rilegge il gdb
  RG=$($BIN read ours.gdb 2>&1)
  check "filegdb: nostro read streaming conta 3 righe" "$RG" "\"rows_read\":3"
else
  echo "  SKIP filegdb (CLI senza gdal-backend)"
fi

# geoparquet, solo se GDAL ha il driver Parquet
if ogrinfo --formats 2>/dev/null | grep -qi parquet; then
  if $BIN convert in.geojson ours.parquet >/dev/null 2>err.txt; then
    P=$(ogrinfo -al -so ours.parquet 2>&1)
    check "geoparquet: GDAL conta 3 feature" "$P" "Feature Count: 3"
    check "geoparquet: attributo (Milano)" "$(ogrinfo -al ours.parquet 2>&1)" "Milano"
  else
    echo "  FAIL convert->parquet: $(cat err.txt)"; fail=$((fail+1))
  fi
else
  echo "  SKIP geoparquet (GDAL senza driver Parquet)"
fi

echo "--- direzione B: GDAL writer -> NOSTRO reader ---"

# GDAL scrive gpkg da geojson; il nostro reader deve contarci 3 righe
ogr2ogr gdal.gpkg in.geojson >/dev/null 2>&1
R=$($BIN read gdal.gpkg 2>&1)
check "nostro read del gpkg-di-GDAL: 3 righe" "$R" "\"rows_read\":3"

# GDAL scrive geojson; il nostro reader lo scorre
ogr2ogr -f GeoJSON gdal.geojson in.geojson >/dev/null 2>&1
RG=$($BIN read gdal.geojson 2>&1)
check "nostro read del geojson-di-GDAL: 3 righe" "$RG" "\"rows_read\":3"

# GDAL scrive shp; il nostro reader lo scorre (usa il .prj di GDAL)
ogr2ogr gdal.shp in.geojson >/dev/null 2>&1
RS=$($BIN read gdal.shp --assume-crs EPSG:4326 2>&1)
check "nostro read dello shp-di-GDAL: 3 righe" "$RS" "\"rows_read\":3"

# round-trip completo attraverso GDAL: gpkg nostro -> geojson via ogr2ogr
ogr2ogr rt.geojson ours.gpkg >/dev/null 2>&1
check "ogr2ogr round-trip (Milano preservato)" "$(cat rt.geojson 2>/dev/null)" "Milano"

echo "--- multi-layer: GDAL crea 2 layer -> noi convertiamo -> GDAL riconta ---"
cat > l2.geojson <<'EOF'
{"type":"FeatureCollection","features":[{"type":"Feature","geometry":{"type":"Point","coordinates":[1.0,1.0]},"properties":{"k":"x"}}]}
EOF
ogr2ogr -f GPKG multi.gpkg in.geojson -nln alpha >/dev/null 2>&1
ogr2ogr -f GPKG -update -append multi.gpkg l2.geojson -nln beta >/dev/null 2>&1
L=$($BIN layers multi.gpkg 2>&1)
check "layers: la nostra CLI elenca 'alpha'" "$L" "alpha"
check "layers: la nostra CLI elenca 'beta'" "$L" "beta"
if $BIN convert multi.gpkg outmulti.gpkg >/dev/null 2>err.txt; then
  M=$(ogrinfo outmulti.gpkg 2>&1)
  check "convert multi-layer: GDAL vede 'alpha' nel risultato" "$M" "alpha"
  check "convert multi-layer: GDAL vede 'beta' nel risultato" "$M" "beta"
else
  echo "  FAIL convert multi-layer: $(cat err.txt)"; fail=$((fail+1))
fi
# multi-layer -> single-layer deve essere rifiutato (fail-closed)
if $BIN convert multi.gpkg nope.geojson >/dev/null 2>err.txt; then
  echo "  FAIL: convert multi->geojson doveva fallire"; fail=$((fail+1))
else
  check "convert multi->single rifiutato (fail-closed)" "$(cat err.txt)" "SINGLE_LAYER_SINK"
fi

echo "--- CRS non-WGS84: GDAL crea EPSG:3857 -> noi leggiamo/convertiamo -> GDAL riconosce ---"
ogr2ogr -t_srs EPSG:3857 src3857.gpkg in.geojson >/dev/null 2>&1
I3=$($BIN layers src3857.gpkg 2>&1)
check "layers: la nostra CLI legge EPSG:3857 dal gpkg" "$I3" "EPSG:3857"
if $BIN convert src3857.gpkg out3857.gpkg >/dev/null 2>err.txt; then
  O3=$(ogrinfo -al -so out3857.gpkg 2>&1)
  check "convert: GDAL riconosce 3857 nel gpkg prodotto" "$O3" "3857"
else
  echo "  FAIL convert 3857: $(cat err.txt)"; fail=$((fail+1))
fi

echo "==============================="
echo "PASS: $pass   FAIL: $fail"
if [ "$fail" -eq 0 ]; then echo "ORACOLO GDAL: PASS"; exit 0; else echo "ORACOLO GDAL: FAIL"; exit 1; fi
