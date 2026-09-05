#!/bin/bash
# Lo smoke del pacchetto Python: in ambienti **puliti**, uno per strada.
#
# # Perche' un venv per ciascuna
#
# Perche' un `import` che riesce nella directory del repository non dimostra
# niente: `src/` e' li', e Python lo trova. Cio' che si vuole sapere e' se il
# pacchetto **installato** funzioni, e per saperlo bisogna stare dove il
# sorgente non c'e'.
#
# Le due strade sono diverse e vanno provate entrambe. La wheel si installa
# copiando; la sdist si **ricostruisce** con setuptools, che legge il
# `pyproject.toml` e decide da se' che cosa impacchettare -- ed e' li' che un
# `package-data` dimenticato si vede, non nella wheel che l'ha scritto a mano.
#
# # L'isolamento: quando si puo' evitare, e quando no
#
# Costruire dalla sdist richiede il backend che `build-system.requires`
# dichiara. Se l'ambiente ce l'ha gia', lo smoke usa `--no-build-isolation` e
# dimostra qualcosa in piu': che la costruzione non ha bisogno di rete. Se non
# ce l'ha, lascia che pip lo procuri -- e' la strada standard, ed e' quella che
# chi riceve la sdist percorrera'.
#
# La differenza non e' teorica: da Python 3.12 `setuptools` non e' piu'
# preinstallato, e uno smoke che desse per scontato di trovarlo passava su 3.11
# e falliva sulle altre due. L'abbiamo scoperto dalla matrice della CI, che
# esiste per questo.
#
# Cio' che il pacchetto promette di non scaricare e' un'altra cosa: il
# **binario**, a runtime. Quella promessa la verifica lo smoke installato, che
# guarda dentro il pacchetto e non trova eseguibili.
set -euo pipefail

RADICE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="${1:?serve la directory con la wheel e la sdist}"
LAVORO="$(mktemp -d)"
trap 'rm -rf "$LAVORO"' EXIT

# Assoluti, e risolti **prima** di cambiare directory: lo smoke si sposta
# fuori dal repository -- e' il suo mestiere -- e un percorso relativo li' non
# indica piu' niente.
DIST="$(cd "$DIST" && pwd)"
WHEEL="$(ls "$DIST"/*.whl)"
SDIST="$(ls "$DIST"/*.tar.gz)"
ATTESA="$(python3 -c "
import re, pathlib
testo = pathlib.Path('$RADICE/sdk/python/src/plenora_io/__init__.py').read_text(encoding='utf-8')
print(re.search(r'^__version__ = \"([^\"]+)\"\$', testo, re.M).group(1))
")"

echo "=== versione attesa: $ATTESA"

# --- 1. la wheel, installata in un ambiente vuoto --------------------------
echo "=== 1. installazione della wheel"
python3 -m venv "$LAVORO/da-wheel"
"$LAVORO/da-wheel/bin/pip" install --quiet --no-index "$WHEEL"

cd "$LAVORO"  # fuori dal repository: `src/` non e' raggiungibile da qui
"$LAVORO/da-wheel/bin/python" - <<'FINE'
import plenora_io
import pathlib

print("   import:", plenora_io.__version__, "protocollo", plenora_io.PROTOCOL_VERSION)

# `py.typed` deve essere **installato**, non solo esistere nel sorgente: senza,
# un type checker ignora le annotazioni e chi installa perde i tipi.
marcatore = pathlib.Path(plenora_io.__file__).parent / "py.typed"
assert marcatore.is_file(), "py.typed non e' stato installato"
print("   py.typed:", marcatore)

# Nessun binario dentro il pacchetto: la wheel e' pura, e resta pura.
dentro = list(pathlib.Path(plenora_io.__file__).parent.rglob("*"))
eseguibili = [p for p in dentro if p.suffix in {".so", ".pyd", ".dll", ".exe"}]
assert not eseguibili, f"la wheel porta binari: {eseguibili}"
assert not any(p.name == "plenora-io" for p in dentro), "la wheel porta la CLI"
print(f"   {len(dentro)} file, nessun binario")

# La superficie pubblica c'e' tutta.
mancanti = [n for n in plenora_io.__all__ if not hasattr(plenora_io, n)]
assert not mancanti, f"`__all__` promette cio' che non c'e': {mancanti}"
print(f"   __all__: {len(plenora_io.__all__)} nomi, tutti presenti")
FINE

installata="$("$LAVORO/da-wheel/bin/python" -c 'import plenora_io; print(plenora_io.__version__)')"
[ "$installata" = "$ATTESA" ] || { echo "versione installata $installata != $ATTESA"; exit 1; }

# --- 2. i metadati che pip ha registrato -----------------------------------
echo "=== 2. metadati del pacchetto installato"
"$LAVORO/da-wheel/bin/python" - <<FINE
from importlib import metadata

d = metadata.metadata("plenora-io")
print("   Name:", d["Name"], "| Version:", d["Version"])
print("   Requires-Python:", d["Requires-Python"])
assert d["Version"] == "$ATTESA", "i metadati dichiarano un'altra versione"
assert "Private :: Do Not Upload" in d.get_all("Classifier"), (
    "manca il classificatore che impedisce la pubblicazione su un indice"
)
assert not metadata.requires("plenora-io"), "il pacchetto ha guadagnato dipendenze"
FINE

# --- 3. la sdist: si ricostruisce e si installa ----------------------------
echo "=== 3. ricostruzione e installazione dalla sdist"
# `--system-site-packages` per un motivo solo: rendere visibile il backend di
# build, se l'ambiente ce l'ha. Il pacchetto in prova si installa comunque
# **dentro** il venv, e la sonda qui sotto verifica che venga da li'.
python3 -m venv --system-site-packages "$LAVORO/da-sdist"

if "$LAVORO/da-sdist/bin/python" -c "import setuptools, wheel" 2>/dev/null; then
  echo "   backend gia' presente: costruzione senza rete"
  "$LAVORO/da-sdist/bin/pip" install --quiet --no-index --no-build-isolation "$SDIST"
else
  echo "   backend assente: pip lo procura, come fara' chi riceve la sdist"
  "$LAVORO/da-sdist/bin/pip" install --quiet "$SDIST"
fi
ricostruita="$("$LAVORO/da-sdist/bin/python" -c 'import plenora_io; print(plenora_io.__version__)')"
dove="$("$LAVORO/da-sdist/bin/python" -c 'import plenora_io, pathlib; print(pathlib.Path(plenora_io.__file__).parent)')"
case "$dove" in
  "$LAVORO/da-sdist"/*) ;;
  *) echo "il pacchetto viene da $dove, non dal venv"; exit 1 ;;
esac
[ "$ricostruita" = "$ATTESA" ] || { echo "dalla sdist esce $ricostruita"; exit 1; }
echo "   import dalla sdist: $ricostruita"

"$LAVORO/da-sdist/bin/python" - <<'FINE'
import pathlib
import plenora_io

# La stessa pretesa della wheel, sull'altra strada: setuptools decide da se'
# che cosa impacchettare, e un `package-data` dimenticato si vede **qui**.
marcatore = pathlib.Path(plenora_io.__file__).parent / "py.typed"
assert marcatore.is_file(), "py.typed non e' arrivato attraverso la sdist"
print("   py.typed dalla sdist: c'e'")
FINE

# --- 4. la suite, contro il pacchetto installato ---------------------------
#
# I test si prendono dalla **sdist**, non dal repository: cosi' si prova anche
# che la sdist li porti, che e' cio' che permette a chi la riceve di verificare
# quel che ha installato.
echo "=== 4. suite dell'SDK, contro il pacchetto installato"
tar xzf "$SDIST" -C "$LAVORO"
ESTRATTA="$LAVORO/$(basename "$SDIST" .tar.gz)"
cd "$ESTRATTA"
"$LAVORO/da-wheel/bin/python" -m unittest discover -s tests -p "test_*.py" 2>&1 | tail -4

# --- 5. e quali sonde non ha potuto esercitare -----------------------------
#
# Il conteggio delle saltate e' un numero, e un numero non dice quali. L'elenco
# si misura qui -- e' l'unico posto dove la condizione e' quella vera -- e il
# gate lo confronta con il registro, nei due versi.
echo "=== 5. l'elenco delle sonde saltate e' quello registrato"
"$LAVORO/da-wheel/bin/python" "$RADICE/scripts/elenca-sonde-saltate.py"   --tests tests --uscita "$LAVORO/saltate.json"
python3 "$RADICE/scripts/check_sonde_saltate.py" --misurato "$LAVORO/saltate.json"

echo "=== smoke del pacchetto Python: superato"
