#!/usr/bin/env python3
"""Il pacchetto Python non va su un indice pubblico, e niente lo carica.

# Che cosa protegge

La distribuzione avviene per canale riservato a clienti autorizzati, e nessuna
licenza first-party e' dichiarata: chi riceve un artefatto non trova dentro
l'archivio termini che gli concedano qualcosa, e senza concessione esplicita
non c'e' permesso. Una pubblicazione pubblica sarebbe quindi una consegna senza
i termini che la governano.

Oggi quella promessa e' mantenuta **per assenza**: nessun workflow carica su un
indice, e il `pyproject.toml` porta il classificatore che i servizi rifiutano.

Una promessa mantenuta per assenza e' quella che si rompe piu' facilmente. Un
workflow aggiunto per comodita' -- «pubblichiamo su un indice interno, tanto e'
privato» -- o un `twine upload` in uno script di rilascio non farebbero rosso
da nessuna parte, e il primo a scoprirlo sarebbe chi trova il pacchetto dove
non doveva essere. Da li' non si torna indietro: un artefatto pubblicato e' un
artefatto che qualcuno ha gia' scaricato.

# Le tre domande

1. Il `pyproject.toml` dichiara `Private :: Do Not Upload`.
2. Nessun workflow invoca uno strumento di pubblicazione, e nessuno nomina un
   segreto d'indice.
3. La matrice dichiara il canale riservato, e non lo dichiara pubblico.

# Che cosa **non** afferma

Che il pacchetto sia segreto. Wheel e sdist sono Python puro: chi li riceve
legge i sorgenti, e la sdist porta anche i test. Cio' che questo gate tiene e'
il **canale**, non la riservatezza del codice: nessuna e' stata promessa, e
ottenerla richiederebbe un SDK compilato. Confondere le due cose farebbe
credere che qui si protegga qualcosa che nessuno protegge.
"""

from __future__ import annotations

import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
PYPROJECT = ROOT / "sdk" / "python" / "pyproject.toml"
WORKFLOWS = ROOT / ".github" / "workflows"
MATRICE = ROOT / "assurance" / "registries" / "distribuzione-matrice.json"

CLASSIFICATORE = "Private :: Do Not Upload"

#: Gli strumenti che caricano un pacchetto Python su un indice.
#:
#: Non e' un elenco di nomi cattivi: sono i comandi che **trasferiscono** byte a
#: un indice, ed e' quel trasferimento che qui non deve esistere. Un elenco per
#: nome invecchia -- domani ne esce un altro -- e per questo il gate guarda
#: anche i segreti: qualunque strumento nuovo avrebbe comunque bisogno di una
#: credenziale, e la credenziale ha una forma piu' stabile del comando.
PUBBLICATORI = (
    re.compile(r"\btwine\s+upload\b"),
    re.compile(r"gh-action-pypi-publish"),
    re.compile(r"\b(?:poetry|flit|uv|hatch)\s+publish\b"),
    re.compile(r"\bpython\s+-m\s+twine\b"),
)

#: I segreti che una pubblicazione pretenderebbe.
CREDENZIALI = re.compile(r"(PYPI_|TWINE_|POETRY_PYPI_TOKEN)", re.IGNORECASE)


def problemi() -> list[str]:
    trovati: list[str] = []

    testo = PYPROJECT.read_text(encoding="utf-8")
    if CLASSIFICATORE not in testo:
        trovati.append(
            f"`{PYPROJECT.relative_to(ROOT)}` non dichiara «{CLASSIFICATORE}»: "
            "e' l'affermazione che i servizi d'indice leggono per rifiutare il "
            "caricamento, ed e' la prima delle tre cose che tengono il canale "
            "chiuso."
        )
    # Una licenza dichiarata nei metadati sarebbe un'invenzione finche' il
    # titolare non fornisce testo e denominazione.
    if re.search(r"^license\s*=", testo, re.MULTILINE):
        trovati.append(
            "il `pyproject.toml` dichiara un campo `license`. Nessuna licenza "
            "first-party e' stata decisa, e finche' il titolare non fornisce i "
            "termini e la denominazione legale esatta qualunque valore li' e' "
            "inventato -- e' l'errore da cui si viene: la prima stesura diceva "
            "`Apache-2.0`."
        )

    for workflow in sorted(WORKFLOWS.glob("*.yml")) + sorted(WORKFLOWS.glob("*.yaml")):
        contenuto = workflow.read_text(encoding="utf-8")
        for schema in PUBBLICATORI:
            if schema.search(contenuto):
                trovati.append(
                    f"`{workflow.relative_to(ROOT)}` invoca uno strumento di "
                    f"pubblicazione ({schema.pattern}): il pacchetto si "
                    "consegna per canale riservato, non si carica su un indice."
                )
        credenziale = CREDENZIALI.search(contenuto)
        if credenziale is not None:
            trovati.append(
                f"`{workflow.relative_to(ROOT)}` nomina una credenziale "
                f"d'indice («{credenziale.group(0)}»). Un segreto esiste per "
                "essere usato, e questo non ha un uso legittimo qui."
            )

    matrice = json.loads(MATRICE.read_text(encoding="utf-8"))
    pubblicazione = matrice.get("pubblicazione", {})
    canale = pubblicazione.get("canale_del_pacchetto_python", {})
    if not canale.get("riservato"):
        trovati.append(
            "la matrice non dichiara riservato il canale del pacchetto Python. "
            "Il gate verifica cio' che il repository **fa**; la matrice dice "
            "che cosa promette, e una promessa non scritta non si puo' "
            "contraddire."
        )
    if canale.get("indice_pubblico") is not False:
        trovati.append(
            "la matrice non dichiara `indice_pubblico: false` per il pacchetto "
            "Python."
        )

    return trovati


def main() -> int:
    trovati = problemi()
    if trovati:
        for problema in trovati:
            print(problema, file=sys.stderr)
        return 1
    print(
        "canale privato verificato: il classificatore c'e', nessuno dei "
        f"{len(list(WORKFLOWS.glob('*.y*ml')))} workflow carica su un indice o "
        "nomina una credenziale d'indice, e la matrice dichiara il canale "
        "riservato."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
