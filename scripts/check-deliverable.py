#!/usr/bin/env python3
"""Verifica i byte scaricati dal gate, non quelli rimasti sul runner produttore."""

from __future__ import annotations

import argparse
import json
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import distribuzione  # noqa: E402


RADICE = pathlib.Path(__file__).resolve().parent.parent
MATRICE = RADICE / "assurance" / "registries" / "distribuzione-matrice.json"
LOCK = {
    "linux-x86_64": RADICE / "scripts" / "linux-gdal-lock.json",
    "windows-x86_64": RADICE / "scripts" / "windows-gdal-lock.json",
}


def _sidecar(percorso: pathlib.Path, archivio: pathlib.Path) -> str:
    righe = percorso.read_text(encoding="utf-8").splitlines()
    if len(righe) != 1:
        raise ValueError("deve contenere una sola riga")
    atteso_suffisso = f"  {archivio.name}"
    if not righe[0].endswith(atteso_suffisso):
        raise ValueError(f"non nomina esattamente {archivio.name}")
    digesto = righe[0][: -len(atteso_suffisso)]
    if len(digesto) != 64 or any(c not in "0123456789abcdef" for c in digesto):
        raise ValueError("il digest non e' uno SHA-256 minuscolo")
    return digesto


def verifica(
    directory: pathlib.Path,
    versione: str,
    canale: str,
    revisione: str,
) -> list[str]:
    matrice = json.loads(MATRICE.read_text(encoding="utf-8"))
    piattaforme = [p["id"] for p in matrice["piattaforme"]]
    profili = [p["id"] for p in matrice["profili"]]
    errori: list[str] = []
    attesi: set[str] = set()

    for piattaforma in piattaforme:
        for profilo in profili:
            nome = distribuzione.nome_archivio(versione, piattaforma, profilo)
            estensione = distribuzione.contenitore(piattaforma)
            archivio = directory / f"{nome}.{estensione}"
            checksum = directory / f"{archivio.name}.sha256"
            provenance = directory / f"{archivio.name}.provenance.json"
            attesi.update((archivio.name, checksum.name, provenance.name))
            prefisso = f"{piattaforma}/{profilo}"

            mancanti = [p.name for p in (archivio, checksum, provenance) if not p.is_file()]
            if mancanti:
                errori.append(f"{prefisso}: file mancanti: {mancanti}")
                continue

            reale = distribuzione.sha256(archivio)
            try:
                dichiarato = _sidecar(checksum, archivio)
            except (OSError, UnicodeError, ValueError) as exc:
                errori.append(f"{prefisso}: sidecar non valido: {exc}")
                dichiarato = None
            if dichiarato is not None and dichiarato != reale:
                errori.append(
                    f"{prefisso}: checksum {dichiarato} diverso dai byte scaricati {reale}"
                )

            try:
                prova = json.loads(provenance.read_text(encoding="utf-8"))
            except (OSError, UnicodeError, json.JSONDecodeError) as exc:
                errori.append(f"{prefisso}: provenance non leggibile: {exc}")
                continue
            pretese = {
                "artefatto": archivio.name,
                "sha256": reale,
                "dimensione": archivio.stat().st_size,
                "piattaforma": piattaforma,
                "profilo": profilo,
                "canale": canale,
                "non_release": canale != "candidate",
                "revisione": revisione,
                "lock": distribuzione.sha256(LOCK[piattaforma]),
            }
            for campo, atteso in pretese.items():
                if prova.get(campo) != atteso:
                    errori.append(
                        f"{prefisso}: provenance.{campo} vale {prova.get(campo)!r}, "
                        f"atteso {atteso!r}"
                    )

    presenti = {
        str(p.relative_to(directory)).replace("\\", "/")
        for p in directory.rglob("*")
        if p.is_file()
    }
    extra = sorted(presenti - attesi)
    if extra:
        errori.append(f"file non dichiarati fra i deliverable: {extra}")
    return errori


def verifica_contro_la_candidate(
    directory: pathlib.Path,
    artefatti: list[dict],
    revisione_candidate: str,
) -> list[str]:
    """I byte **pubblicati** sono quelli congelati, non una ricostruzione.

    `verifica` qui sopra confronta ogni archivio con i **propri** sidecar: un
    insieme ricostruito da capo e' internamente coerente e passa, perche' ogni
    checksum descrive fedelmente l'archivio che gli sta accanto. La domanda a
    cui non risponde e' se quegli archivi siano gli **stessi** su cui e' girata
    la qualifica.

    Non e' una distinzione teorica. Due costruzioni della stessa revisione
    possono differire di un byte -- un timestamp dentro l'archivio basta -- e
    allora cio' che si e' misurato e cio' che si consegna sono due insiemi
    diversi, «equivalenti» nel senso che nessuno ha verificato. Il digest
    congelato al momento della candidate e' l'unico riferimento esterno che
    rende la differenza visibile.

    Si esegue **dopo** la pubblicazione, sui byte riscaricati dal canale di
    release: prima non c'e' niente da confrontare.
    """
    if not artefatti:
        return [
            "nessun artefatto congelato con cui confrontare: un elenco vuoto "
            "non produce nessun confronto, e nessun confronto non e' un "
            "confronto riuscito."
        ]

    errori: list[str] = []
    for congelato in artefatti:
        nome = congelato.get("nome")
        if not isinstance(nome, str) or not nome:
            errori.append(f"artefatto congelato senza nome: {congelato!r}")
            continue
        if congelato.get("revisione") != revisione_candidate:
            errori.append(
                f"{nome}: congelato sulla revisione "
                f"«{str(congelato.get('revisione'))[:12]}», la candidate e' "
                f"«{revisione_candidate[:12]}»"
            )
        percorso = directory / nome
        if not percorso.is_file():
            errori.append(f"{nome}: assente fra i byte pubblicati")
            continue
        reale = distribuzione.sha256(percorso)
        if reale != congelato.get("sha256"):
            errori.append(
                f"{nome}: i byte pubblicati hanno digest {reale}, il congelato "
                f"e' {congelato.get('sha256')}. Un archivio ricostruito non e' "
                "l'archivio qualificato, per quanto equivalente."
            )
        dimensione = percorso.stat().st_size
        if dimensione != congelato.get("dimensione"):
            errori.append(
                f"{nome}: dimensione pubblicata {dimensione}, congelata "
                f"{congelato.get('dimensione')}"
            )
    return errori


def main() -> int:
    a = argparse.ArgumentParser(description=__doc__)
    a.add_argument("--directory", required=True, type=pathlib.Path)
    a.add_argument("--versione", required=True)
    a.add_argument("--canale", required=True, choices=("prova", "candidate"))
    a.add_argument("--revisione", required=True)
    # Il confronto con i digest congelati e' un passo **successivo alla
    # pubblicazione**, e ha percio' una sua opzione invece di stare sempre
    # acceso: nel canale `prova` non esiste una candidate congelata con cui
    # confrontarsi, e pretenderla renderebbe rosso cio' che non ha nulla da
    # verificare.
    a.add_argument(
        "--contro-la-candidate",
        type=pathlib.Path,
        metavar="STATO",
        help="assurance/current-state.json: verifica che questi byte siano "
        "quelli congelati dalla candidate, non una ricostruzione equivalente",
    )
    arg = a.parse_args()
    errori = verifica(arg.directory, arg.versione, arg.canale, arg.revisione)
    congelati = 0
    if arg.contro_la_candidate is not None:
        stato = json.loads(arg.contro_la_candidate.read_text(encoding="utf-8"))
        candidate = stato.get("aperto", {}).get("candidate_release", {})
        artefatti = candidate.get("artefatti") or []
        congelati = len(artefatti)
        errori.extend(
            verifica_contro_la_candidate(
                arg.directory, artefatti, candidate.get("revisione_candidate")
            )
        )
    if errori:
        for ciascuno in errori:
            print(f"ERRORE: {ciascuno}", file=sys.stderr)
        return 1
    print("deliverable verificati: 4 archivi, 4 checksum, 4 provenance")
    if arg.contro_la_candidate is not None:
        print(
            f"byte pubblicati identici ai {congelati} artefatti congelati "
            "dalla candidate"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
