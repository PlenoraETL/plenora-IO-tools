#!/usr/bin/env python3
"""Fail if a direct dependency is not reproducibly declared, or if the
product and the fuzzing project pin a shared dependency differently."""

from __future__ import annotations

import sys
import tomllib
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
DEPENDENCY_TABLES = {
    "dependencies",
    "dev-dependencies",
    "build-dependencies",
}


def dependency_tables(value: Any, path: tuple[str, ...] = ()):
    if not isinstance(value, dict):
        return
    for key, child in value.items():
        child_path = (*path, key)
        if key in DEPENDENCY_TABLES and isinstance(child, dict):
            yield child_path, child
        else:
            yield from dependency_tables(child, child_path)


def validate_dependency(
    manifest: Path, table: tuple[str, ...], name: str, specification: Any
) -> list[str]:
    location = f"{manifest.relative_to(ROOT)} [{'.'.join(table)}] {name}"
    if isinstance(specification, str):
        return [] if specification.startswith("=") else [f"{location}: pin non esatto"]
    if not isinstance(specification, dict):
        return [f"{location}: dichiarazione non riconosciuta"]

    version = specification.get("version")
    if version is not None and (
        not isinstance(version, str) or not version.startswith("=")
    ):
        return [f"{location}: pin di versione non esatto"]

    if "git" in specification:
        if "rev" not in specification or "branch" in specification or "tag" in specification:
            return [f"{location}: dipendenza git senza solo rev immutabile"]
        return []

    if version is None and not (
        specification.get("workspace") is True or "path" in specification
    ):
        return [f"{location}: manca versione, workspace o path"]
    return []


def versione_dichiarata(specification: Any) -> str | None:
    """La versione fissata da una dichiarazione, se ne fissa una."""
    if isinstance(specification, str):
        return specification
    if isinstance(specification, dict):
        version = specification.get("version")
        if isinstance(version, str):
            return version
    return None


def pin_condivisi(radice: dict, fuzz: dict) -> list[str]:
    """I pin dichiarati da entrambi i manifesti devono coincidere.

    Perche' esiste
    --------------
    `fuzz/Cargo.toml` e' un workspace *detached*: `workspace = true` non puo'
    ereditare niente, perche' cercherebbe una `[workspace.dependencies]` nella
    propria sezione, che e' vuota. Le poche dipendenze condivise col prodotto il
    progetto di fuzzing le dichiara percio' per conto suo, e le due
    dichiarazioni possono separarsi senza che niente lo dica.

    Il 2026-09-09 si sono separate: alzato `geo-types` a `=0.7.20` nel
    workspace, `fuzz/` e' rimasto a `=0.7.19`. Ciascun manifesto restava coerente
    col proprio lockfile -- quindi `cargo metadata --locked` passava su entrambi
    -- e a fermarsi e' stata la build strumentata, molto piu' tardi e con un
    messaggio che parlava d'altro. Questo confronto dice la stessa cosa subito.

    Che cosa confronta
    ------------------
    Le versioni, e solo quando entrambi i lati ne dichiarano una. Non le feature
    ne' i percorsi: una divergenza li' e' un fatto diverso, e mescolarla qui
    renderebbe il messaggio piu' vago invece che piu' utile.
    """
    condivisi = radice.get("workspace", {}).get("dependencies", {})
    if not condivisi:
        return ["Cargo.toml: nessuna [workspace.dependencies] da confrontare"]

    errori: list[str] = []
    for table, dependencies in dependency_tables(fuzz):
        for name, specification in sorted(dependencies.items()):
            if name not in condivisi:
                continue
            nostra = versione_dichiarata(condivisi[name])
            loro = versione_dichiarata(specification)
            if nostra is None or loro is None:
                continue
            if nostra != loro:
                errori.append(
                    f"{name}: pin condiviso divergente - "
                    f"Cargo.toml [workspace.dependencies] lo fissa a "
                    f"'{nostra}', fuzz/Cargo.toml [{'.'.join(table)}] a "
                    f"'{loro}'. Va alzato in entrambi i manifesti: il progetto "
                    "di fuzzing e' detached e non eredita."
                )
    return errori


def pin_condivisi_del_repository() -> tuple[list[str], int]:
    """Le divergenze fra i due manifesti sul disco, e quanti pin ha confrontato."""
    with (ROOT / "Cargo.toml").open("rb") as stream:
        radice = tomllib.load(stream)
    with (ROOT / "fuzz" / "Cargo.toml").open("rb") as stream:
        fuzz = tomllib.load(stream)
    condivisi = set(radice.get("workspace", {}).get("dependencies", {}))
    quanti = sum(
        1
        for _, dependencies in dependency_tables(fuzz)
        for name in dependencies
        if name in condivisi
    )
    return pin_condivisi(radice, fuzz), quanti


def main() -> int:
    manifests = [ROOT / "Cargo.toml"]
    manifests.extend(sorted((ROOT / "crates").glob("*/Cargo.toml")))
    manifests.append(ROOT / "fuzz" / "Cargo.toml")
    errors: list[str] = []

    for manifest in manifests:
        with manifest.open("rb") as stream:
            document = tomllib.load(stream)
        for table, dependencies in dependency_tables(document):
            for name, specification in dependencies.items():
                errors.extend(
                    validate_dependency(manifest, table, name, specification)
                )

    divergenze, condivisi = pin_condivisi_del_repository()
    errors.extend(divergenze)

    if errors:
        print("Dependency pin gate failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1
    print(
        f"Dependency pin gate passed ({len(manifests)} manifests, "
        f"{condivisi} pin condivisi con fuzz/ e coerenti)."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
