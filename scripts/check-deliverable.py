#!/usr/bin/env python3
"""Verifica i byte scaricati dal gate, non quelli rimasti sul runner produttore."""

from __future__ import annotations

import argparse
import json
import pathlib
import sys
import tarfile
import zipfile

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


def revisione_del_manifesto(
    archivio: pathlib.Path, nome: str
) -> tuple[object, str | None]:
    """La revisione dichiarata dal `MANIFEST.json` **dentro** l'archivio.

    Restituisce `(revisione, guasto)`: il guasto e' una stringa quando il
    manifesto non si legge, e in quel caso la revisione non va confrontata --
    un archivio che non si apre non deve leggersi come «va bene».

    Il manifesto viaggia dentro l'archivio, la provenance accanto. Se le due
    nominano revisioni diverse, chi legge l'albero installato e chi verifica il
    sidecar concludono cose diverse sulla stessa installazione, e nessuno dei
    due sbaglia a leggere. Finche' nessuno le confrontava, la divergenza non era
    osservabile da nessuna parte.
    """
    percorso = f"{nome}/MANIFEST.json"
    try:
        if archivio.suffix == ".zip":
            with zipfile.ZipFile(archivio) as z:
                crudo = z.read(percorso)
        else:
            with tarfile.open(archivio) as tar:
                estratto = tar.extractfile(percorso)
                if estratto is None:
                    return None, f"{percorso} non e' un file dentro l'archivio"
                crudo = estratto.read()
    except (KeyError, OSError, tarfile.TarError, zipfile.BadZipFile) as errore:
        return None, f"MANIFEST.json non leggibile dentro l'archivio: {errore}"
    try:
        manifesto = json.loads(crudo.decode("utf-8"))
    except (UnicodeError, json.JSONDecodeError) as errore:
        return None, f"MANIFEST.json non e' JSON leggibile: {errore}"
    if "revisione" not in manifesto:
        return None, "il MANIFEST.json dell'archivio non porta `revisione`"
    return manifesto["revisione"], None


def verifica_python(
    directory: pathlib.Path, canale: str, revisione: str
) -> tuple[list[str], set[str]]:
    """I due artefatti Python: `(errori, nomi attesi)`.

    # Perche' non passano dal ciclo delle piattaforme

    Perche' non ne hanno una, e nemmeno un profilo: `py3-none-any` vuol dire
    che ne servono zero. E soprattutto perche' **la loro versione e' un'altra**:
    il nome degli archivi nativi lo compone la versione passata alla corsa, il
    pacchetto Python porta la propria -- quella di `plenora_io.__version__` --
    e derivarne il nome dalla prima darebbe un file che non esiste.

    Il nome lo dice il manifesto del pacchetto, che viaggia accanto agli
    archivi. E' la stessa disciplina del lato nativo, dove il manifesto sta
    dentro l'archivio: si legge cio' che il costruttore ha dichiarato, e lo si
    ricalcola sui byte arrivati.

    # I sidecar, e il documento che ne fa le veci

    Ogni archivio ha il proprio `.sha256`. La provenance invece e' **una** per
    entrambi -- `provenance.json` con un elenco `file` -- perche' i due sono la
    stessa costruzione in due formati: separarle avrebbe creato due documenti
    che dicono la stessa revisione e lo stesso lock, destinati a divergere per
    una modifica fatta a meta'.
    """
    errori: list[str] = []
    attesi: set[str] = set()

    manifesto_percorso = directory / "MANIFEST.json"
    provenance_percorso = directory / "provenance.json"
    for documento in (manifesto_percorso, provenance_percorso):
        if not documento.is_file():
            errori.append(f"python: {documento.name} assente fra i deliverable")
    if errori:
        return errori, attesi

    attesi.update({"MANIFEST.json", "provenance.json", "sbom.json", "licenze.json"})
    try:
        manifesto = json.loads(manifesto_percorso.read_text(encoding="utf-8"))
        provenance = json.loads(provenance_percorso.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        return [f"python: documento non leggibile: {exc}"], attesi

    for campo, atteso in (
        ("revisione", revisione),
        ("canale", canale),
        ("non_release", canale != "candidate"),
        ("classe", "python-puro"),
        ("piattaforma", "any"),
    ):
        if manifesto.get(campo) != atteso:
            errori.append(
                f"python: MANIFEST.{campo} vale {manifesto.get(campo)!r}, "
                f"atteso {atteso!r}"
            )
    if provenance.get("revisione") != revisione:
        errori.append(
            f"python: la provenance dichiara la revisione "
            f"{provenance.get('revisione')!r}, la corsa e' su {revisione!r}"
        )

    dichiarati = manifesto.get("file") or []
    if len(dichiarati) != 2:
        errori.append(
            f"python: il manifesto dichiara {len(dichiarati)} file, e i formati "
            "distribuiti sono due -- wheel e sdist"
        )
    per_nome = {
        voce.get("nome"): voce.get("archivio_sha256")
        for voce in (provenance.get("file") or [])
    }

    for voce in dichiarati:
        nome = voce.get("nome")
        if not nome:
            errori.append("python: una voce del manifesto non ha nome")
            continue
        archivio = directory / nome
        checksum = directory / f"{nome}.sha256"
        attesi.update((nome, checksum.name))
        mancanti = [q.name for q in (archivio, checksum) if not q.is_file()]
        if mancanti:
            errori.append(f"python/{nome}: file mancanti: {mancanti}")
            continue

        reale = distribuzione.sha256(archivio)
        if voce.get("sha256") != reale:
            errori.append(
                f"python/{nome}: il manifesto dichiara {voce.get('sha256')}, "
                f"i byte scaricati danno {reale}"
            )
        if voce.get("byte") != archivio.stat().st_size:
            errori.append(
                f"python/{nome}: il manifesto dichiara {voce.get('byte')} byte, "
                f"il file ne ha {archivio.stat().st_size}"
            )
        try:
            dichiarato = _sidecar(checksum, archivio)
        except (OSError, UnicodeError, ValueError) as exc:
            errori.append(f"python/{nome}: sidecar non valido: {exc}")
        else:
            if dichiarato != reale:
                errori.append(
                    f"python/{nome}: il sidecar dice {dichiarato}, i byte "
                    f"scaricati danno {reale}"
                )
        # La provenance e il manifesto descrivono gli stessi byte, e a dirlo
        # deve essere il **calcolo**: due documenti coerenti fra loro e sbagliati
        # allo stesso modo passerebbero un confronto reciproco.
        if per_nome.get(nome) != reale:
            errori.append(
                f"python/{nome}: la provenance dice {per_nome.get(nome)}, i byte "
                f"scaricati danno {reale}"
            )

    return errori, attesi


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
            nel_manifesto, guasto = revisione_del_manifesto(archivio, nome)
            if guasto is not None:
                errori.append(f"{prefisso}: {archivio.name}: {guasto}")
            elif nel_manifesto != prova.get("revisione"):
                errori.append(
                    f"{prefisso}: {archivio.name} porta la revisione "
                    f"{nel_manifesto!r} nel manifesto e "
                    f"{prova.get('revisione')!r} nella provenance. Il manifesto "
                    "viaggia dentro l'archivio e la provenance accanto: se "
                    "divergono, chi legge l'albero installato e chi verifica il "
                    "sidecar concludono cose diverse sulla stessa installazione."
                )
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

    errori_python, attesi_python = verifica_python(directory, canale, revisione)
    errori.extend(errori_python)
    attesi.update(attesi_python)

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


def main(argv: list[str] | None = None) -> int:
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
    arg = a.parse_args(argv)
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
    # Contati, non scritti. Il numero era fisso -- «4 archivi» -- ed e' rimasto
    # quattro quando gli artefatti sono diventati sei: il riepilogo diceva meno
    # di cio' che la verifica aveva fatto, e chi lo legge ne avrebbe dedotto che
    # i due pacchetti Python nessuno li avesse guardati.
    archivi = sorted(
        percorso.name
        for percorso in arg.directory.iterdir()
        if percorso.is_file()
        and (arg.directory / f"{percorso.name}.sha256").is_file()
    )
    print(
        f"deliverable verificati: {len(archivi)} archivi con altrettanti "
        f"checksum, di cui {sum(1 for n in archivi if n.startswith('plenora_io-'))} "
        "Python"
    )
    if arg.contro_la_candidate is not None:
        print(
            f"byte pubblicati identici ai {congelati} artefatti congelati "
            "dalla candidate"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
