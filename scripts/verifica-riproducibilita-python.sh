#!/bin/bash
# Due costruzioni dallo stesso albero danno gli stessi byte.
#
# # Perche' e' un passo a se'
#
# Perche' la riproducibilita' non si vede costruendo una volta. Il costruttore
# fissa le date e ordina le voci, e quelle scelte si possono perdere in una
# riga: un `sorted` tolto, un `mtime` lasciato al valore predefinito. Il difetto
# non fa rosso da nessun'altra parte -- l'artefatto resta valido -- e si scopre
# il giorno in cui due checksum dello stesso contenuto non coincidono.
#
# E' anche la condizione che rende sensata la regola della pubblicazione:
# riusare i byte qualificati invece di ricostruirli ha senso solo se
# ricostruirli darebbe gli stessi byte. Se non li desse, la regola sarebbe
# l'unica difesa contro una differenza che nessuno saprebbe spiegare.
set -euo pipefail

RADICE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
UNO="$(mktemp -d)"
DUE="$(mktemp -d)"
trap 'rm -rf "$UNO" "$DUE"' EXIT

python3 "$RADICE/scripts/costruisci-pacchetto-python.py" --uscita "$UNO" > /dev/null
python3 "$RADICE/scripts/costruisci-pacchetto-python.py" --uscita "$DUE" > /dev/null

diverse=0
for percorso in "$UNO"/*.whl "$UNO"/*.tar.gz; do
  nome="$(basename "$percorso")"
  a="$(sha256sum "$percorso" | cut -d' ' -f1)"
  b="$(sha256sum "$DUE/$nome" | cut -d' ' -f1)"
  if [ "$a" = "$b" ]; then
    echo "  $nome: identico ($a)"
  else
    echo "  $nome: DIVERSO ($a vs $b)" >&2
    diverse=1
  fi
done

if [ "$diverse" -ne 0 ]; then
  echo "due costruzioni dallo stesso albero hanno dato byte diversi." >&2
  exit 1
fi
echo "pacchetto Python riproducibile: due costruzioni, gli stessi byte"
