#!/usr/bin/env python3
"""Fail if a direct dependency is not reproducibly declared, if the product
and the fuzzing project pin a shared dependency differently, or if the
migration census counts a set of dependencies and describes another."""

from __future__ import annotations

import json
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


CENSIMENTO = ROOT / "assurance" / "registries" / "censimento-verso-la-3.0.0.json"


def censimento_riconciliato(registro: dict | None = None) -> list[str]:
    """L'elenco della migrazione e quello dei fork descrivono lo stesso insieme.

    # Perche' esiste

    Il 2026-09-10, rimisurando le dipendenze dirette invece di rileggere il
    censimento, `shapefile` e' risultato indietro di tre minor -- e non era fra
    le voci di `dipendenze_ancora_da_migrare`. Il conteggio diceva «27 allineate
    su 28» e l'elenco ne descriveva sei: nessuno dei due era falso da solo, e
    insieme nascondevano una dipendenza.

    La causa e' strutturale, non una svista. I tre fork governati sono **sia**
    dipendenze dirette del workspace **sia** voci della sezione `fork`, e chi
    compilava l'elenco della migrazione ne ha portata una sola. Un elenco tenuto
    a mano accanto a un manifesto diverge, e la sola difesa e' che qualcosa li
    confronti.

    # Che cosa pretende

    Tre cose, tutte verificabili senza rete:

    * ogni crate nominato in `fork` compare anche fra le voci della migrazione:
      e' esattamente il buco che si e' aperto;
    * ogni voce della migrazione e' una dipendenza diretta vera, perche' un
      elenco puo' divergere anche nell'altro senso;
    * `allineate + indietro` uguaglia il numero di dipendenze dirette, cosi' il
      conteggio non puo' descrivere un insieme diverso da quello che c'e'.

    Non pretende invece che ogni dipendenza compaia fra le voci: quell'elenco
    nomina cio' che ha richiesto lavoro, ed elencarne ventotto lo renderebbe
    illeggibile senza dire niente di piu'.
    """
    if registro is None:
        with CENSIMENTO.open("rb") as stream:
            registro = json.load(stream)

    with (ROOT / "Cargo.toml").open("rb") as stream:
        radice = tomllib.load(stream)
    dirette = set(radice.get("workspace", {}).get("dependencies", {}))
    if not dirette:
        return ["Cargo.toml: nessuna [workspace.dependencies] da riconciliare"]

    sezione = registro.get("dipendenze_ancora_da_migrare") or {}
    voci = {v["crate"] for v in sezione.get("voci", []) if isinstance(v, dict)}
    fork = set(registro.get("fork") or {})

    errori: list[str] = []

    mancanti = sorted((fork & dirette) - voci)
    if mancanti:
        errori.append(
            f"fork che sono dipendenze dirette e non stanno fra le voci della "
            f"migrazione: {mancanti}. E' il modo in cui `shapefile` e' rimasto "
            "fuori dal conteggio pur essendo indietro di tre minor."
        )

    inventate = sorted(voci - dirette)
    if inventate:
        errori.append(
            f"voci della migrazione che non sono dipendenze dirette del "
            f"workspace: {inventate}"
        )

    somma = sezione.get("allineate", 0) + sezione.get("indietro", 0)
    if somma != len(dirette):
        errori.append(
            f"il conteggio della migrazione descrive {somma} dipendenze, il "
            f"workspace ne dichiara {len(dirette)}: un conteggio che non "
            "corrisponde all'insieme puo' essere vero e nascondere una voce."
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
    errors.extend(censimento_riconciliato())

    if errors:
        print("Dependency pin gate failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1
    with (ROOT / "Cargo.toml").open("rb") as stream:
        dirette = len(tomllib.load(stream).get("workspace", {}).get("dependencies", {}))
    print(
        f"Dependency pin gate passed ({len(manifests)} manifests, "
        f"{condivisi} pin condivisi con fuzz/ e coerenti, "
        f"{dirette} dipendenze dirette riconciliate col censimento)."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
