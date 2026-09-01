#!/usr/bin/env python3
"""Costruisce l'artefatto Windows: albero installabile, archivio, manifesto, SBOM.

# Che cosa cambia rispetto a Linux

Il caricatore di Windows cerca le DLL **accanto all'eseguibile**: non c'e' un
`$ORIGIN` da dichiarare, e le librerie stanno in `bin/` insieme al binario, non
in `lib/`. Non c'e' un `RPATH` da verificare, e non c'e' una soglia `GLIBC_*`:
la dipendenza dal runtime C si affronta spedendo cio' che serve invece di
misurare una versione.

Il contenitore e' uno ZIP: `tar.gz` non e' un formato che gli strumenti Windows
sappiano aprire senza aiuto, e chi installa non deve procurarsi uno strumento
per leggere un artefatto.

# Che cosa **non** fa ancora

Non firma. Il passo c'e' e sta nella sua posizione -- prima del manifesto, che
descrive i byte firmati -- ma senza certificato non appone nulla, e lo stato
resta `non_richiesta` sul canale di prova e `non_misurata` su una candidate.
Quest'ultimo e' rosso al gate, ed e' l'esito giusto.

# La prima corsa non qualifica

L'insieme delle DLL di sistema attese non esiste ancora, e non si scrive a
tavolino: si **misura** con `check-windows-runtime.py --discovery`, si rilegge,
si classifica ogni dipendenza a mano, e solo un commit successivo lo mette nel
lock insieme al digest del rilievo da cui viene. Fino ad allora il verificatore
si ferma, ed e' cio' che deve fare.
"""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import shutil
import subprocess
import sys
import zipfile

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import distribuzione  # noqa: E402 -- dopo sys.path, che e' il punto

RADICE = pathlib.Path(__file__).resolve().parent.parent
LOCK = RADICE / "scripts" / "windows-gdal-lock.json"
VERIFICATORE = RADICE / "scripts" / "check-windows-runtime.py"


def esegui(comando: list[str], **kwargs) -> subprocess.CompletedProcess:
    print("  $ " + " ".join(str(c) for c in comando), flush=True)
    return subprocess.run(comando, check=True, **kwargs)


def carica_verificatore():
    import importlib.util

    spec = importlib.util.spec_from_file_location("windows_runtime", VERIFICATORE)
    modulo = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(modulo)
    return modulo


def revisione_del_repository() -> str | None:
    """`None` quando non si riesce a leggerla, e non una stringa di comodo.

    Una provenance che dichiarasse una revisione inventata sarebbe peggio di una
    che ammette di non saperla: chi la legge deve poter distinguere una
    revisione assente da una sbagliata.
    """
    try:
        esito = subprocess.run(
            ["git", "rev-parse", "HEAD"], capture_output=True, text=True, cwd=RADICE
        )
    except (OSError, subprocess.SubprocessError):
        return None
    return esito.stdout.strip() if esito.returncode == 0 else None


def main() -> int:
    a = argparse.ArgumentParser(description=__doc__)
    a.add_argument("--prefisso", required=True, type=pathlib.Path,
                   help="il runtime GDAL materializzato da install-windows-gdal.ps1")
    a.add_argument("--uscita", required=True, type=pathlib.Path)
    a.add_argument("--versione", required=True)
    a.add_argument("--canale", default="prova", choices=["prova", "candidate"])
    a.add_argument("--profilo", default="filegdb", choices=["base", "filegdb"])
    a.add_argument("--referti", type=pathlib.Path, default=None)
    a.add_argument("--salta-build", action="store_true")
    arg = a.parse_args()

    lock = json.loads(LOCK.read_text(encoding="utf-8"))
    prefisso = arg.prefisso.resolve()
    uscita = arg.uscita.resolve()
    nome = f"plenora-io-{arg.versione}-windows-x86_64-{arg.profilo}"
    albero = uscita / nome

    if albero.exists():
        shutil.rmtree(albero)
    # Nessuna `lib/`: il caricatore di Windows guarda accanto all'eseguibile, e
    # mettere le DLL altrove vorrebbe dire dirgli dove guardare -- cioe' un
    # `PATH` o una `SetDllDirectory`, che e' esattamente il genere di dipendenza
    # dall'ambiente che un artefatto rilocabile non deve avere.
    for sotto in ("bin", "share", "LICENSES"):
        (albero / sotto).mkdir(parents=True, exist_ok=True)

    # =====================================================================
    # 1. IL PAYLOAD
    # =====================================================================
    libreria = prefisso / "Library"
    target = RADICE / "target" / f"artefatto-windows-{arg.profilo}"
    binario = target / "release" / "plenora-io.exe"

    if not arg.salta_build:
        ambiente = dict(os.environ)
        ambiente["GDAL_HOME"] = str(libreria)
        ambiente["GDAL_VERSION"] = lock["gdal_version"]
        ambiente["CARGO_TARGET_DIR"] = str(target)
        comando = ["cargo", "build", "--release", "--locked", "-p", "plenora-io-cli"]
        if arg.profilo == "filegdb":
            comando += ["--features", "gdal-backend"]
        print("1a. compilazione", flush=True)
        esegui(comando, cwd=RADICE, env=ambiente)
    if not binario.is_file():
        raise SystemExit(f"{binario} non esiste")
    shutil.copy2(binario, albero / "bin" / "plenora-io.exe")

    verificatore = carica_verificatore()

    # Il binario dice da se' quale profilo e'. Verificarlo costa una lettura e
    # chiude la classe di difetti in cui il nome dell'archivio e il suo
    # contenuto divergono -- che e' la peggiore, perche' il nome e' cio' che chi
    # installa legge.
    normali, ritardati = verificatore.importazioni(albero / "bin" / "plenora-io.exe")
    linka_gdal = any(n.startswith("gdal") for n in normali | ritardati)
    if linka_gdal != (arg.profilo == "filegdb"):
        raise SystemExit(
            f"il binario {'importa' if linka_gdal else 'non importa'} GDAL, e il profilo "
            f"richiesto e' «{arg.profilo}»."
        )

    print("1b. chiusura degli import dal binario", flush=True)
    if arg.profilo == "filegdb":
        # Le DLL candidate stanno in `Library/bin` del prefisso; si copiano in
        # `bin/` e si richiude la chiusura finche' non si aggiungono piu' nomi.
        # E' un punto fisso e non una passata sola: una DLL trascinata da
        # un'altra comparirebbe solo al secondo giro.
        da_cercare = [albero / "bin" / "plenora-io.exe"]
        copiate: set[str] = set()
        while da_cercare:
            normali, ritardati = verificatore.importazioni(da_cercare.pop())
            for richiesta in sorted(normali | ritardati):
                if richiesta in copiate or verificatore.e_api_set(richiesta):
                    continue
                candidata = libreria / "bin" / richiesta
                if not candidata.exists():
                    continue
                destinazione = albero / "bin" / richiesta
                shutil.copy2(candidata, destinazione)
                copiate.add(richiesta)
                da_cercare.append(destinazione)
        print(f"   DLL spedite: {len(copiate)}", flush=True)

        for origine, sotto in (
            (libreria / "share" / "gdal", "share/gdal"),
            (libreria / "share" / "proj", "share/proj"),
        ):
            if origine.is_dir():
                shutil.copytree(origine, albero / sotto, dirs_exist_ok=True)

    spediti = [p for p in albero.rglob("*") if p.is_file()]

    # 1e. licenze -- lo stesso principio di Linux: si spedisce il testo di cio'
    # che si spedisce. Su Windows la mappa file-a-pacchetto viene dallo stesso
    # `conda-meta`, perche' la catena e' la stessa.
    print("1e. licenze", flush=True)
    meta = prefisso / "conda-meta"
    licenze_scritte = 0
    if meta.is_dir():
        nomi_spediti = {p.name.lower() for p in spediti}
        for documento in sorted(meta.glob("*.json")):
            d = json.loads(documento.read_text(encoding="utf-8"))
            contribuisce = any(
                pathlib.PurePath(f).name.lower() in nomi_spediti for f in d.get("files", [])
            )
            if not contribuisce:
                continue
            estratta = d.get("extracted_package_dir") or ""
            origine = pathlib.Path(estratta) / "info" / "licenses" if estratta else None
            if origine is not None and origine.is_dir():
                shutil.copytree(origine, albero / "LICENSES" / d["name"], dirs_exist_ok=True)
                licenze_scritte += 1
    print(f"   pacchetti con il proprio testo: {licenze_scritte}", flush=True)

    # =====================================================================
    # 2. LA FIRMA -- prima del manifesto, che descrive i byte firmati
    # =====================================================================
    firma = distribuzione.stato_della_firma("windows-x86_64", arg.canale)
    print(f"2. firma: {firma['stato']}", flush=True)
    if firma["stato"] in ("assente", "non_misurata"):
        raise SystemExit(
            f"il canale «{arg.canale}» pretende una firma {firma['meccanismo']}, e lo stato e' "
            f"«{firma['stato']}». Senza certificato non si costruisce una candidate: un "
            "artefatto candidate non firmato e' un artefatto che chi lo riceve non puo' "
            "verificare."
        )

    # =====================================================================
    # 3. IL MANIFESTO, dai byte firmati
    # =====================================================================
    print("3. manifesto", flush=True)
    manifesto = {
        "nome": nome,
        "versione": arg.versione,
        "piattaforma": "windows-x86_64",
        "profilo": arg.profilo,
        "canale": arg.canale,
        "non_release": arg.canale != "candidate",
        "gdal": lock["gdal_version"],
        "lock": distribuzione.sha256(LOCK),
        "prefisso_di_costruzione": str(libreria),
        "firma": firma,
        "layout": (
            "le DLL stanno in `bin/` accanto all'eseguibile, perche' e' li' che il caricatore "
            "di Windows guarda. Non c'e' una `lib/` e non c'e' un RPATH: metterle altrove "
            "vorrebbe dire dire al caricatore dove guardare, cioe' dipendere dall'ambiente."
        ),
        "file": sorted(str(p.relative_to(albero)) for p in spediti),
    }
    (albero / "MANIFEST.json").write_text(
        json.dumps(manifesto, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )

    # =====================================================================
    # 4. L'ARCHIVIO
    # =====================================================================
    contenitore = distribuzione.contenitore("windows-x86_64")
    print(f"4. archivio ({contenitore})", flush=True)
    archivio = uscita / f"{nome}.{contenitore}"
    if archivio.exists():
        archivio.unlink()
    with zipfile.ZipFile(archivio, "w", zipfile.ZIP_DEFLATED) as z:
        for percorso in sorted(albero.rglob("*")):
            if percorso.is_file():
                z.write(percorso, f"{nome}/{percorso.relative_to(albero)}")

    # 5. notarizzazione: non esiste su Windows. Il passo resta perche' l'ordine
    # e' uno per tutte e tre le piattaforme.
    print("5. notarizzazione: non applicabile su Windows", flush=True)

    # =====================================================================
    # 6. I CHECKSUM, sui byte finali
    # =====================================================================
    print("6. checksum", flush=True)
    digesto = distribuzione.sha256(archivio)
    (uscita / f"{archivio.name}.sha256").write_text(
        f"{digesto}  {archivio.name}\n", encoding="utf-8"
    )
    print(f"   {archivio}  ({archivio.stat().st_size} byte)", flush=True)
    print(f"   sha256 {digesto}", flush=True)

    print("7. smoke: lo esegue scripts/smoke-profilo.py sull'artefatto", flush=True)

    # =====================================================================
    # 8. LA PROVENANCE, legata a quel checksum
    # =====================================================================
    print("8. provenance", flush=True)
    revisione = revisione_del_repository()
    provenance = {
        "schema": 1,
        "artefatto": archivio.name,
        "sha256": digesto,
        "dimensione": archivio.stat().st_size,
        "piattaforma": "windows-x86_64",
        "profilo": arg.profilo,
        "canale": arg.canale,
        "non_release": arg.canale != "candidate",
        "revisione": revisione,
        "lock": distribuzione.sha256(LOCK),
        "prefisso_di_costruzione": str(libreria),
        "firma": firma,
        "ordine_delle_operazioni": firma["ordine_delle_operazioni"],
    }
    (uscita / f"{archivio.name}.provenance.json").write_text(
        json.dumps(provenance, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )

    if arg.referti:
        distribuzione.scrivi_referto(
            arg.referti / f"windows-{arg.profilo}-provenance.json",
            verifica="provenance",
            piattaforma="windows-x86_64",
            profilo=arg.profilo,
            canale=arg.canale,
            esito="verde",
            misure={
                "archivio_sha256": digesto,
                "revisione": revisione,
                "lock_sha256": provenance["lock"],
                "dimensione": provenance["dimensione"],
            },
            errori=[],
        )
    print(f"   {archivio.name}.provenance.json", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
