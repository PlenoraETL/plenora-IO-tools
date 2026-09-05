#!/usr/bin/env python3
"""Lo smoke del pacchetto Python **installato**, contro un artefatto vero.

# Che cosa dimostra, e perche' non basta lo smoke del pacchetto

`smoke-pacchetto-python.sh` prova che la wheel si installi e che l'SDK si
importi. E' necessario e non basta: quel che l'SDK esiste per fare e' parlare
con un binario, e finche' non gli si mette davanti un artefatto vero non si sa
se lo trovi, se ne legga il manifesto e se il profilo che dichiara sia quello.

Qui l'artefatto e' quello che il costruttore nativo ha appena prodotto, estratto
com'e' -- non il binario di `cargo`, che non porta un `MANIFEST.json` e quindi
non puo' dimostrare niente sul profilo.

# Perche' il binario non entra nella wheel

Perche' un pacchetto per piattaforma vorrebbe dire il triplo degli artefatti da
qualificare, e un prodotto che si aggiorna solo cambiando l'SDK. Questa sonda e'
il modo in cui quella scelta si paga: la separazione va **dimostrata**
funzionante, o e' solo un'omissione.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import subprocess
import sys
import tempfile

RADICE = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import distribuzione  # noqa: E402 -- dopo sys.path, che e' il punto

#: Il programma che gira **dentro** il venv, dove l'SDK e' installato.
#:
#: Un file a parte e non una stringa nel mezzo: qui si esegue un altro
#: interprete, e cio' che gli si passa e' un programma, non un argomento.
SONDA = r'''
import json
import pathlib
import sys

import plenora_io

albero = pathlib.Path(sys.argv[1])
profilo_atteso = sys.argv[2]
binario = albero / "bin" / "plenora-io"
if sys.platform == "win32":
    binario = binario.with_suffix(".exe")

misure = {}
errori = []

# 1. La scoperta esplicita, che e' la strada documentata.
cliente = plenora_io.Client(binary=binario)
misure["binario_trovato"] = str(cliente.binary)

# 2. Il manifesto dell'artefatto: e' la sola cosa che dice da dove il binario
#    viene, e senza di essa il profilo non e' conoscibile.
manifesto = cliente.manifest
if manifesto is None:
    errori.append("l'artefatto non porta un MANIFEST.json: il profilo non e' conoscibile")
    misure["manifesto_letto"] = False
else:
    misure["manifesto_letto"] = True
    misure["profilo_dichiarato"] = manifesto.profile
    misure["piattaforma"] = manifesto.platform
    misure["canale"] = manifesto.channel
    misure["release"] = manifesto.release
    if manifesto.profile != profilo_atteso:
        errori.append(
            f"il manifesto dichiara il profilo «{manifesto.profile}» e ci si "
            f"aspettava «{profilo_atteso}»"
        )

# 3. Il controllo del profilo, nei due versi. Che accetti quello giusto non
#    basta: una funzione che dicesse sempre di si' passerebbe la meta' della
#    prova, ed e' la meta' che non serve a niente.
try:
    cliente.require_profile(profilo_atteso)
    misure["profilo_accettato"] = True
except plenora_io.ProfileError as errore:
    misure["profilo_accettato"] = False
    errori.append(f"il profilo dichiarato e' stato rifiutato: {errore}")

altro = "filegdb" if profilo_atteso == "base" else "base"
try:
    cliente.require_profile(altro)
    misure["profilo_altrui_rifiutato"] = False
    errori.append(f"il profilo «{altro}» e' stato accettato da un artefatto «{profilo_atteso}»")
except plenora_io.ProfileError:
    misure["profilo_altrui_rifiutato"] = True

# 4. Il binario risponde, e la busta si decodifica.
versione = cliente.version()
misure["versione_del_prodotto"] = versione.version
catalogo = cliente.catalog()
misure["driver_nel_catalogo"] = len(catalogo.drivers)
misure["driver_disponibili"] = len(catalogo.available)

# 5. La promessa del profilo, letta dal catalogo invece che dal nome.
#    `base` promette che FileGDB **manchi**, e il catalogo lo dice.
filegdb = next((d for d in catalogo.drivers if d.id == "filegdb"), None)
if filegdb is None:
    errori.append("il catalogo non elenca `filegdb`: manca il driver, non la feature")
else:
    misure["filegdb_disponibile"] = filegdb.available
    if profilo_atteso == "base" and filegdb.available:
        errori.append("il profilo base dichiara FileGDB disponibile")
    if profilo_atteso == "filegdb" and not filegdb.available:
        errori.append("il profilo filegdb dichiara FileGDB non disponibile")

# 6. Il pacchetto non ha guadagnato dipendenze, e non porta binari.
from importlib import metadata

misure["dipendenze"] = len(metadata.requires("plenora-io") or [])
dentro = pathlib.Path(plenora_io.__file__).parent
misure["file_del_pacchetto"] = sum(1 for _ in dentro.rglob("*") if _.is_file())
binari = [p.name for p in dentro.rglob("*") if p.suffix in {".so", ".pyd", ".dll", ".exe"}]
misure["binari_nel_pacchetto"] = len(binari)
if binari:
    errori.append(f"il pacchetto porta binari: {binari}")

print(json.dumps({"misure": misure, "errori": errori}))
'''


def main(argv: list[str] | None = None) -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument(
        "--albero",
        required=True,
        type=pathlib.Path,
        help="l'albero nativo estratto contro cui l'SDK viene provato",
    )
    argomenti.add_argument(
        "--pacchetto",
        required=True,
        type=pathlib.Path,
        help="il pacchetto Python da installare: una wheel o una sdist",
    )
    argomenti.add_argument(
        "--formato",
        required=True,
        choices=("wheel", "sdist"),
        help="che cosa si sta provando; sono le coordinate del referto",
    )
    argomenti.add_argument(
        "--profilo-dell-albero",
        required=True,
        choices=("base", "filegdb"),
        dest="profilo_dell_albero",
        help="il profilo dell'artefatto nativo, che la sonda pretende dal manifesto",
    )
    argomenti.add_argument(
        "--piattaforma-dell-albero",
        required=True,
        dest="piattaforma_dell_albero",
        help="la piattaforma dell'artefatto nativo; entra fra le misure",
    )
    argomenti.add_argument("--canale", default="prova")
    argomenti.add_argument("--referto", type=pathlib.Path, default=None)
    opzioni = argomenti.parse_args(argv)

    with tempfile.TemporaryDirectory(prefix="plenora-smoke-sdk-") as lavoro:
        venv = pathlib.Path(lavoro) / "venv"
        subprocess.run([sys.executable, "-m", "venv", str(venv)], check=True)
        python = venv / ("Scripts" if sys.platform == "win32" else "bin") / "python"
        subprocess.run(
            # `--no-index` anche per la sdist: il pacchetto non ha dipendenze,
            # e cio' che si vuole provare e' che si installi **senza rete**.
            # Se la ricostruzione avesse bisogno di scaricare un backend,
            # questo passo fallirebbe, ed e' l'informazione che serve.
            [
                str(python),
                "-m",
                "pip",
                "install",
                "--quiet",
                "--no-index",
                str(opzioni.pacchetto),
            ],
            check=True,
        )

        programma = pathlib.Path(lavoro) / "sonda.py"
        programma.write_text(SONDA, encoding="utf-8")
        esito = subprocess.run(
            [
                str(python),
                str(programma),
                str(opzioni.albero),
                opzioni.profilo_dell_albero,
            ],
            capture_output=True,
            text=True,
            check=False,
            # Fuori dal repository: l'SDK dev'essere quello **installato**, e
            # non quello che Python troverebbe accanto ai sorgenti.
            cwd=lavoro,
        )

    if esito.returncode != 0:
        print(esito.stdout, file=sys.stderr)
        print(esito.stderr, file=sys.stderr)
        misure: dict = {}
        errori = [f"la sonda e' uscita con {esito.returncode}"]
    else:
        risultato = json.loads(esito.stdout.strip().splitlines()[-1])
        misure = risultato["misure"]
        errori = risultato["errori"]

    for chiave, valore in sorted(misure.items()):
        print(f"  {chiave}: {valore}")
    for errore in errori:
        print(f"  ERRORE: {errore}", file=sys.stderr)

    if opzioni.referto is not None:
        distribuzione.scrivi_referto(
            opzioni.referto,
            verifica="smoke-installato",
            # Le coordinate sono quelle dell'artefatto **Python**: `any` perche'
            # `py3-none-any` non ha piattaforme, e il formato al posto del
            # profilo. Erano quelle dell'albero nativo -- `linux-x86_64/wheel`
            # -- e il gate, che deriva gli attesi dalla matrice, cercava
            # `any/wheel` e non lo trovava.
            #
            # Contro quale artefatto nativo la prova sia stata fatta non si
            # perde: sta fra le misure, dove e' un dato invece che una
            # coordinata.
            piattaforma="any",
            profilo=opzioni.formato,
            canale=opzioni.canale,
            esito="ok" if not errori else "fallito",
            misure={
                **misure,
                "provato_contro": (
                    f"{opzioni.piattaforma_dell_albero}/{opzioni.profilo_dell_albero}"
                ),
            },
            errori=errori,
            note=(
                f"l'SDK installato dalla {opzioni.formato}, contro l'artefatto "
                f"«{opzioni.profilo_dell_albero}» di "
                f"{opzioni.piattaforma_dell_albero}. Il binario **non** sta nel "
                "pacchetto: questa e' la prova che la separazione funziona."
            ),
        )

    return 1 if errori else 0


if __name__ == "__main__":
    raise SystemExit(main())
