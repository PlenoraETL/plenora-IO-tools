#!/usr/bin/env python3
"""Fail if a workflow executes an action through a mutable reference.

# Che cosa questo gate verifica, e che cosa no

Verifica la **forma**: che ogni action sia fissata a un commit SHA completo
invece che a un tag, perche' un tag si sposta. Non verifica che quello SHA
**esista**: quaranta cifre esadecimali qualunque passano.

Non e' una svista, e' una scelta -- un gate che interroga la rete diventa
rosso quando la rete e' lenta, e un gate che diventa rosso per ragioni sue
smette di essere creduto. Ma la conseguenza va detta: un pin sbagliato a mano
supera questo gate e fallisce in CI, dove il messaggio parlera' di un'action
che non si scarica invece che di uno SHA inventato.

Per questo c'e' `--verifica-online`, che interroga l'API di GitHub e va
eseguito **quando si aggiunge o si cambia un pin**. Non gira nel checkpoint,
dove tutto dev'essere deterministico e offline.

Il difetto e' stato trovato scrivendo il workflow di distribuzione: uno SHA di
`actions/download-artifact` scritto a memoria e mai esistito superava il gate.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WORKFLOWS = ROOT / ".github" / "workflows"
USES = re.compile(r"^\s*(?:-\s*)?uses:\s*([^\s#]+)")
COMMIT = re.compile(r"^[0-9a-f]{40}$")
DIGEST = re.compile(r"^sha256:[0-9a-f]{64}$")
TOOL_INPUT = re.compile(r"^\s+tool:\s*(\S+)\s*(?:#.*)?$")


def validate_reference(reference: str) -> str | None:
    if reference.startswith("./"):
        return None
    if reference.startswith("docker://"):
        image = reference.removeprefix("docker://")
        _, separator, digest = image.rpartition("@")
        if separator and DIGEST.fullmatch(digest):
            return None
        return "immagine Docker senza digest sha256"

    action, separator, revision = reference.rpartition("@")
    if not separator or not action:
        return "riferimento action privo di revisione"
    if not COMMIT.fullmatch(revision):
        return "riferimento action non fissato a un commit SHA completo"
    return None


def required_tool_input(
    reference: str, workflow_lines: list[str], uses_index: int
) -> str | None:
    action = reference.rpartition("@")[0]
    if action != "taiki-e/install-action":
        return None

    uses_line = workflow_lines[uses_index]
    uses_indent = len(uses_line) - len(uses_line.lstrip())
    for line in workflow_lines[uses_index + 1 :]:
        stripped = line.lstrip()
        indent = len(line) - len(stripped)
        if stripped and indent <= uses_indent and stripped.startswith("-"):
            break
        match = TOOL_INPUT.match(line)
        if match is not None:
            return None if match.group(1) else "input tool vuoto"
    return "taiki-e/install-action fissata a SHA richiede with.tool esplicito"


def diagnosi_http(codice: int, repository: str) -> str | None:
    """Che cosa significa una risposta, e che cosa **non** significa.

    Le tre risposte sono tre cose diverse, e confonderle sarebbe il modo di
    trasformare un gate in un generatore di rumore. `404` e `422` dicono che lo
    SHA non c'e': e' il difetto che si cerca. `403` dice che non si e' potuto
    chiedere -- l'API non autenticata concede sessanta richieste l'ora, e questo
    repository ne ha piu' di trenta, quindi due esecuzioni ravvicinate la
    esauriscono. Non e' un pin sbagliato, ed e' importante che il messaggio non
    lo faccia credere.

    In entrambi i casi il gate diventa rosso, perche' fail-closed: «non ho
    potuto verificare» non e' «va bene». Ma chi legge sa quale delle due cose e'
    successa.
    """
    if codice in (404, 422):
        return f"lo SHA non esiste in {repository} (HTTP {codice})"
    if codice == 403:
        return (
            f"non verificabile ora: l'API di GitHub ha risposto 403 su {repository}. "
            "L'API non autenticata concede sessanta richieste l'ora e questo repository ne "
            "usa piu' di trenta per esecuzione. Non e' un pin sbagliato: e' una domanda a cui "
            "non si e' potuto rispondere, e resta rossa perche' non rispondere non e' un si'."
        )
    return f"interrogazione fallita: HTTP {codice}"


def esiste_online(reference: str) -> str | None:
    """Lo SHA esiste davvero nel repository dell'action?

    Sta qui e non nel percorso normale perche' interroga la rete: si esegue
    quando si aggiunge un pin, non a ogni checkpoint.
    """
    import json
    import urllib.error
    import urllib.request

    if reference.startswith(("./", "docker://")):
        return None
    action, _, revision = reference.rpartition("@")
    proprietario_repo = "/".join(action.split("/")[:2])
    url = f"https://api.github.com/repos/{proprietario_repo}/commits/{revision}"
    try:
        with urllib.request.urlopen(url, timeout=20) as risposta:
            json.load(risposta)
        return None
    except urllib.error.HTTPError as e:
        return diagnosi_http(e.code, proprietario_repo)
    except Exception as e:  # noqa: BLE001 -- rete: qualunque cosa e' «non lo so»
        return f"interrogazione non riuscita ({e}); riprovare, non ignorare"


def main() -> int:
    import argparse

    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument(
        "--verifica-online",
        action="store_true",
        help=(
            "interroga l'API di GitHub per accertare che ogni SHA esista. Da usare quando si "
            "aggiunge o si cambia un pin; non nel checkpoint, che dev'essere deterministico"
        ),
    )
    opzioni = argomenti.parse_args()
    workflows = sorted((*WORKFLOWS.glob("*.yml"), *WORKFLOWS.glob("*.yaml")))
    errors: list[str] = []
    references = 0

    for workflow in workflows:
        lines = workflow.read_text(encoding="utf-8").splitlines()
        for line_number, line in enumerate(lines, start=1):
            match = USES.match(line)
            if match is None:
                continue
            references += 1
            reference = match.group(1)
            error = validate_reference(reference)
            if error is not None:
                location = workflow.relative_to(ROOT)
                errors.append(f"{location}:{line_number}: {reference}: {error}")
                continue
            if opzioni.verifica_online:
                assente = esiste_online(reference)
                if assente is not None:
                    location = workflow.relative_to(ROOT)
                    errors.append(f"{location}:{line_number}: {reference}: {assente}")
            input_error = required_tool_input(reference, lines, line_number - 1)
            if input_error is not None:
                location = workflow.relative_to(ROOT)
                errors.append(
                    f"{location}:{line_number}: {reference}: {input_error}"
                )

    if not workflows:
        errors.append(".github/workflows: nessun workflow trovato")
    if references == 0:
        errors.append(".github/workflows: nessun riferimento uses trovato")

    if errors:
        print("GitHub Action pin gate failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1

    print(
        f"GitHub Action pin gate passed "
        f"({len(workflows)} workflow, {references} references)."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
