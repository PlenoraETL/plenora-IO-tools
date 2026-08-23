#!/usr/bin/env bash
# Campagne di **sola copertura** sulla stessa revisione, per il blocco
# `copertura.variazione-fra-corse`.
#
#   bash scripts/campagne_copertura.sh                    due campagne
#   S9_CAMPAGNE=3 bash scripts/campagne_copertura.sh      tre
#   S9_SINGOLO_THREAD=1 bash scripts/campagne_copertura.sh  con --test-threads=1
#
# # Che cosa questo script non e'
#
# Non e' un checkpoint e non produce un verdetto. Non ha soglie, non tocca la
# soglia dell'80% e non chiude niente: **misura piu' volte la stessa cosa** e
# lascia il confronto a `confronta_copertura.py`, che guarda le righe e non le
# percentuali.
#
# # Perche' ogni campagna riparte da zero
#
# Il difetto che il checkpoint ha gia' incontrato una volta e' un report
# stantio letto come misura fresca. Qui sarebbe peggio: due campagne che
# condividono un profilo darebbero per costruzione lo stesso risultato, e la
# riproducibilita' apparirebbe dimostrata senza essere stata misurata.
#
# Ogni campagna quindi:
#
#   1. `cargo llvm-cov clean --workspace` — via i profili precedenti;
#   2. cancella il proprio `.info`, cosi' un report vecchio non puo'
#      sopravvivere a un export fallito;
#   3. esegue la suite strumentata;
#   4. esporta lcov e riepilogo.
#
# Se un passo fallisce la campagna si ferma li' e lo script esce: una campagna
# a meta' non e' un campione, e confrontarla con una intera direbbe che le
# righe mancanti «non sono coperte».
#
# # Gli artefatti sono persistenti, e fuori dal repository
#
# `S9_CAMPAGNE_DIR` sceglie dove. Il valore predefinito sta sotto `/tmp`, ma
# chi lancia dentro un container **deve** montarlo da fuori: gli artefatti sono
# l'unico modo di rileggere una campagna, e una directory che il container si
# porta via ha gia' fatto scartare una corsa intera.

set -u

cd "$(dirname "$0")/.."

CAMPAGNE="${S9_CAMPAGNE:-2}"
DIRECTORY="${S9_CAMPAGNE_DIR:-/tmp/campagne-copertura}"
SINGOLO_THREAD="${S9_SINGOLO_THREAD:-0}"

mkdir -p "${DIRECTORY}"

# Lo stesso perimetro del checkpoint: la soglia vale sulle librerie, non
# sull'attrezzaggio. Misurare un insieme diverso darebbe numeri che non si
# possono confrontare con le evidenze gia' registrate.
#
# La regex compare **per esteso** in ogni invocazione, e non attraverso una
# variabile di shell: questo file e' fra i sorvegliati di
# `check_coverage_exclusions.py`, che legge il valore scritto accanto al flag.
# Passandolo per variabile, quel gate leggerebbe il nome della variabile invece
# della regex — cioe' non sorveglierebbe niente, ed e' proprio la deriva che
# esiste per impedire. Il prezzo e' un letterale ripetuto due volte; il gate lo
# confronta entrambe le volte con quello canonico.

# Lo stesso ambiente del checkpoint. `cross_filesystem_publish_is_rejected…`
# **esce subito** se la variabile non c'e', quindi senza di essa la campagna
# coprirebbe meno righe — e la differenza sarebbe dell'ambiente, non della
# corsa. Una campagna che misura un ambiente diverso non si confronta con
# un'evidenza del checkpoint.
export PLENORA_CROSS_FS_TEST_ROOT="${PLENORA_CROSS_FS_TEST_ROOT:-/dev/shm}"

ARGOMENTI_DI_TEST=()
if [ "${SINGOLO_THREAD}" = "1" ]; then
    # `--test-threads=1` separa la concorrenza dal resto: se la variazione
    # sparisce, la causa e' nell'esecuzione parallela; se resta, non lo e'.
    ARGOMENTI_DI_TEST=(-- --test-threads=1)
fi

echo "=============================================================="
echo "campagne di copertura"
echo "revisione:      $(git rev-parse HEAD)"
echo "albero:         $(git status --porcelain | wc -l) file non committati"
echo "campagne:       ${CAMPAGNE}"
echo "thread singolo: ${SINGOLO_THREAD}"
echo "artefatti:      ${DIRECTORY}"
echo "cross-fs root:  ${PLENORA_CROSS_FS_TEST_ROOT}"
echo "=============================================================="

for numero in $(seq 1 "${CAMPAGNE}"); do
    LCOV="${DIRECTORY}/campagna-${numero}.info"
    RIEPILOGO="${DIRECTORY}/campagna-${numero}.riepilogo"
    REGISTRO="${DIRECTORY}/campagna-${numero}.log"

    echo
    echo "--- campagna ${numero}/${CAMPAGNE} ---------------------------------"

    rm -f "${LCOV}" "${RIEPILOGO}"

    if ! cargo llvm-cov clean --workspace > "${REGISTRO}" 2>&1; then
        echo "  pulizia dei profili FALLITA — ${REGISTRO}" >&2
        exit 1
    fi
    echo "  profili azzerati"

    if ! cargo llvm-cov --workspace --all-targets --locked --no-report \
        "${ARGOMENTI_DI_TEST[@]}" >> "${REGISTRO}" 2>&1; then
        echo "  suite strumentata FALLITA — ${REGISTRO}" >&2
        exit 1
    fi
    echo "  suite eseguita"

    if ! cargo llvm-cov report --lcov --output-path "${LCOV}" \
        --ignore-filename-regex '(^|/)(plenora-bench|plenora-fuzz|plenora-io-cli)/src/.*\.rs$' >> "${REGISTRO}" 2>&1; then
        echo "  export lcov FALLITO — ${REGISTRO}" >&2
        exit 1
    fi
    if [ ! -s "${LCOV}" ]; then
        echo "  export lcov vuoto: un report assente non e' una misura." >&2
        exit 1
    fi

    if ! cargo llvm-cov report --summary-only \
        --ignore-filename-regex '(^|/)(plenora-bench|plenora-fuzz|plenora-io-cli)/src/.*\.rs$' > "${RIEPILOGO}" 2>&1; then
        echo "  riepilogo FALLITO — ${RIEPILOGO}" >&2
        exit 1
    fi

    # Il conteggio si ricava dai record `DA:`, non dal riepilogo: quel numero
    # e' lo strumento che riferisce su se stesso, e qui serve un valore
    # ricontato fuori.
    python3 - "${LCOV}" <<'PYTHON'
import sys

strumentate = coperte = 0
for riga in open(sys.argv[1], encoding="utf-8", errors="replace"):
    if riga.startswith("DA:"):
        strumentate += 1
        if int(riga[3:].split(",")[1]) > 0:
            coperte += 1
print(f"  righe: {coperte}/{strumentate} ({coperte / strumentate * 100:.4f}%)")
PYTHON
done

echo
echo "=============================================================="
echo "confronto per (file, riga, coperta o no)"
echo "=============================================================="
python3 scripts/confronta_copertura.py "${DIRECTORY}"/campagna-*.info
esito=$?

echo
if [ "${esito}" -eq 0 ]; then
    echo "Le campagne coincidono. Non e' ancora una dimostrazione di"
    echo "riproducibilita': lo diventa con un numero di campagne dichiarato e"
    echo "con le condizioni della corsa registrate in un'evidenza."
else
    echo "Le campagne divergono. Il passo successivo e' guardare **quali**"
    echo "righe si muovono, non ripetere la misura sperando che coincida."
fi
exit "${esito}"
