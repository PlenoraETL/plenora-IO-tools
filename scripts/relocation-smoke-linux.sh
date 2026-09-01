#!/usr/bin/env bash
#
# Il relocation smoke: l'artefatto funziona dove non e' stato costruito?
#
# # La domanda
#
# I binari spediti portano cotti dentro i percorsi del prefisso di costruzione.
# Il ragionamento dice che non contano -- l'RPATH e' relativo a `$ORIGIN`, e le
# tre radici dei dati le imposta il binario da se'. E' un ragionamento, e i
# ragionamenti su cio' che non succede si sbagliano in silenzio: gia' una volta
# avevo concluso che quei percorsi fossero stringhe inerti, e una misura ha
# smontato la conclusione.
#
# Questo smoke lo mette alla prova dove si puo'.
#
# # Le condizioni
#
# 1. Si costruisce in A e si archivia.
# 2. **A viene cancellata.** Senza questo passo un percorso che contasse
#    davvero resterebbe risolvibile, e non lo vedremmo.
# 3. Si estrae in una B con percorso di lunghezza diversa da A -- perche' un
#    percorso rilocato male puo' funzionare per caso quando le lunghezze
#    coincidono.
# 4. Si esegue da una terza directory, che non e' ne' A ne' B.
# 5. Senza ambiente conda e senza `LD_LIBRARY_PATH`.
# 6. `GDAL_DATA`, `PROJ_DATA` e `GDAL_DRIVER_PATH` sono **preimpostate a
#    sentinelle inesistenti**: se il binario le lasciasse stare, fallirebbe.
# 7. Si scrive e si rilegge un FileGDB **con un CRS**, per attraversare anche
#    PROJ, verificando schema, righe e geometria.
# 8. Gli accessi ai file sono tracciati, e qualunque tentativo di toccare A fa
#    rosso.
# 9. Si verifica che ogni libreria fuori dall'allowlist ABI sia stata caricata
#    da B.
#
# # Che cosa dimostra, e che cosa no
#
# I percorsi **effettivamente attraversati**. I percorsi TLS, XML, terminfo e
# Kerberos che questo smoke non esercita restano governati dalla loro
# classificazione strutturale, e **non diventano «provati» perche' lo smoke
# locale e' verde**. Sono due garanzie diverse e vanno lette diverse: una e' una
# prova su un percorso, l'altra e' un'affermazione su un percorso non percorso.

set -euo pipefail

ARCHIVIO="${1:?uso: relocation-smoke-linux.sh <archivio.tar.gz> <directory-A> [radice-di-lavoro]}"
A="${2:?la directory di costruzione, che viene cancellata}"
LAVORO="${3:-/smoke}"
# Dove scrivere il referto nel formato comune; vuoto significa nessun referto.
REFERTO="${4:-}"

# Le verifiche si invocano con un percorso assoluto: piu' avanti si entra
# nella terza directory, e un percorso relativo li' non trova nulla.
VERIFICHE="$(readlink -f "$(dirname "$0")/verifiche_del_relocation_smoke.py")"
ARCHIVIO="$(readlink -f "${ARCHIVIO}")"
A="$(readlink -f "${A}")"

# B ha un percorso di lunghezza diversa da A: un nome deliberatamente lungo.
B="${LAVORO}/b-installazione-con-un-percorso-deliberatamente-piu-lungo"
TERZA="${LAVORO}/terza"
rm -rf "${B}" "${TERZA}"
mkdir -p "${B}" "${TERZA}"

echo "== 1-2. l'archivio e' fuori da A, e A viene cancellata"
case "${ARCHIVIO}" in
  "${A}"/*)
    echo "ROSSO: l'archivio sta dentro A, che sto per cancellare." >&2
    exit 1
    ;;
esac
rm -rf "${A}"
if [ -e "${A}" ]; then
  echo "ROSSO: A esiste ancora." >&2
  exit 1
fi
echo "   A cancellata: ${A}"

echo "== 3. estrazione in B"
tar -xzf "${ARCHIVIO}" -C "${B}"
RADICE="$(find "${B}" -mindepth 1 -maxdepth 1 -type d | head -1)"
BINARIO="${RADICE}/bin/plenora-io"
[ -x "${BINARIO}" ] || { echo "ROSSO: ${BINARIO} non e' eseguibile" >&2; exit 1; }
echo "   ${RADICE}"
echo "   lunghezza A=${#A}  B=${#RADICE}"
if [ "${#A}" -eq "${#RADICE}" ]; then
  echo "ROSSO: A e B hanno la stessa lunghezza; un percorso rilocato male passerebbe per caso." >&2
  exit 1
fi

echo "== 4-6. esecuzione dalla terza directory, senza ambiente, con sentinelle"
cd "${TERZA}"
SENTINELLA="/percorso/che/non/esiste/mai"

# `env -i` toglie tutto: nessun `LD_LIBRARY_PATH`, nessun `CONDA_PREFIX`,
# nessun residuo del prefisso di costruzione. Restano `PATH` e `HOME` perche'
# senza non si esegue nulla, e le tre sentinelle, che sono il punto.
ambiente() {
  env -i \
    PATH=/usr/bin:/bin \
    HOME="${TERZA}" \
    GDAL_DATA="${SENTINELLA}/gdal" \
    PROJ_DATA="${SENTINELLA}/proj" \
    GDAL_DRIVER_PATH="${SENTINELLA}/plugins" \
    "$@"
}

echo "== 7-8. scrittura e rilettura di un FileGDB con CRS, sotto strace"
cat > "${TERZA}/sorgente.csv" <<'CSV'
codice,nome,geometry
A-1,alfa,POINT(11.25 43.77)
B-2,beta,POINT(12.49 41.90)
CSV

TRACCIA="${LAVORO}/traccia.txt"
: > "${TRACCIA}"

# La traccia va in un file separato, non mescolata all'uscita del comando: e'
# la condizione 8, e serve a poterla interrogare per **quali** file sono stati
# toccati -- non soltanto per sapere che il comando non e' fallito.
ambiente strace -f -e trace=file -o "${TRACCIA}" \
  "${BINARIO}" convert "${TERZA}/sorgente.csv" "${TERZA}/uscita.gdb" \
    --in-opt wkt_column=geometry \
    --assume-crs EPSG:4326 \
  > "${TERZA}/convert.json" 2> "${TERZA}/convert.err" || {
    echo "ROSSO: la conversione e' fallita" >&2
    cat "${TERZA}/convert.err" >&2
    exit 1
  }
echo "   scritto: $(head -c 200 "${TERZA}/convert.json")"

ambiente "${BINARIO}" inspect "${TERZA}/uscita.gdb" \
  > "${TERZA}/inspect.json" 2> "${TERZA}/inspect.err" || {
    echo "ROSSO: la rilettura e' fallita" >&2
    cat "${TERZA}/inspect.err" >&2
    exit 1
  }

python3 "${VERIFICHE}" rilettura "${TERZA}/inspect.json"

# La controprova della condizione 6.
#
# Che il comando riesca con le sentinelle attive prova che qualcosa le
# scavalca, ma non prova che siano **letali**: se GDAL trovasse i propri dati
# da solo, il verde di sopra sarebbe un verde che non ha misurato niente.
#
# Qui si toglie all'artefatto il proprio layout -- `share/gdal` rinominata -- in
# una copia. `radici.rs` non riconosce piu' un artefatto, non imposta nulla, e
# restano le sentinelle. Il comando **deve** fallire; se riuscisse, saprei che
# la condizione 6 non misura cio' che dice.
echo "== 6-bis. controprova: senza il layout, le sentinelle devono essere letali"
CIECA="${LAVORO}/cieca"
rm -rf "${CIECA}"
cp -r "${RADICE}" "${CIECA}"
mv "${CIECA}/share/gdal" "${CIECA}/share/gdal-nascosta"
if ambiente "${CIECA}/bin/plenora-io" convert \
     "${TERZA}/sorgente.csv" "${TERZA}/controprova.gdb" \
     --in-opt wkt_column=geometry --assume-crs EPSG:4326 \
     > "${TERZA}/controprova.json" 2> "${TERZA}/controprova.err"; then
  echo "ROSSO: senza share/gdal e con le sentinelle attive la conversione e' riuscita." >&2
  echo "La condizione 6 non sta misurando cio' che dice: qualcosa trova i dati altrove." >&2
  exit 1
fi
echo "   senza il layout il comando fallisce: le sentinelle sono davvero letali"

echo "== 8. nessun accesso ad A"
if grep -F -- "${A}" "${TRACCIA}" > "${LAVORO}/accessi-ad-A.txt"; then
  echo "ROSSO: il binario ha tentato di accedere alla directory di costruzione:" >&2
  head -20 "${LAVORO}/accessi-ad-A.txt" >&2
  exit 1
fi
echo "   nessuna riga della traccia nomina ${A}"

echo "== 9. ogni libreria non di sistema viene da B"
ambiente "${BINARIO}" catalog > /dev/null 2>&1 || true
MAPPA="${LAVORO}/mappa.txt"
ambiente env LD_DEBUG=libs LD_DEBUG_OUTPUT="${LAVORO}/lddebug" \
  "${BINARIO}" catalog > /dev/null 2>&1 || true
cat "${LAVORO}"/lddebug.* 2>/dev/null | grep -E "calling init|generating link map" > "${MAPPA}" || true

python3 "${VERIFICHE}" librerie "${MAPPA}" "${RADICE}" "${REFERTO:-}"

echo
echo "RELOCATION SMOKE VERDE"
echo "Dimostra i percorsi effettivamente attraversati: dati di GDAL, griglie di"
echo "PROJ, driver FileGDB, e la risoluzione delle librerie. I percorsi TLS,"
echo "XML, terminfo e Kerberos non sono stati esercitati e restano governati"
echo "dalla loro classificazione strutturale: questo verde non li promuove."

