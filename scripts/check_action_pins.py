#!/usr/bin/env python3
"""Fail if a workflow executes an action through a mutable reference."""

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


def main() -> int:
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
