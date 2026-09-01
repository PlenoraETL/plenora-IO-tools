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


def carica_costruttore_linux():
    """Le funzioni sulle licenze, prese da chi le ha gia'.

    `identificatori_spdx` e `procurati_testo` non hanno niente di specifico di
    una piattaforma: leggono un'espressione SPDX e verificano un testo fissato.
    Riscriverle qui produrrebbe due implementazioni della stessa regola, e le
    licenze sono la cosa peggiore su cui lasciare divergere due copie.
    """
    import importlib.util

    percorso = RADICE / "scripts" / "costruisci-artefatto-linux.py"
    spec = importlib.util.spec_from_file_location("costruttore_linux", percorso)
    modulo = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(modulo)
    return modulo


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
    linux = carica_costruttore_linux()
    identificatori_spdx = linux.identificatori_spdx
    procurati_testo = linux.procurati_testo
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
        # Esplicito, invece di lasciarlo dedurre da `GDAL_HOME\lib`: e' li' che
        # `gdal-sys` cerca `gdal_i.lib`, e dirlo costa una riga.
        ambiente["GDAL_LIB_DIR"] = str(libreria / "lib")
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

    # La chiusura si fa per **entrambi** i profili, e non solo per `filegdb`.
    #
    # Era un'ipotesi ereditata da Linux: li' il profilo base non spedisce
    # librerie, perche' `libgcc_s` e `libm` sono garantite dal sistema. Su
    # Windows non e' cosi': la prima corsa di scoperta ha mostrato che anche il
    # profilo base importa `vcruntime140.dll`, che **non** e' un componente del
    # sistema operativo -- e' il runtime C ridistribuibile di Visual Studio.
    #
    # Il runner ce l'ha perche' ci gira Visual Studio; un server pulito
    # potrebbe non averla, e l'artefatto non partirebbe con un errore che parla
    # di una DLL mancante invece che di cio' che manca davvero. Il contratto lo
    # diceva gia' -- «il runtime C si affronta spedendo cio' che serve» -- e il
    # codice non lo faceva.
    #
    # Si copia da `Library/bin` e si richiude finche' non si aggiungono piu'
    # nomi: e' un punto fisso e non una passata sola, perche' una DLL trascinata
    # da un'altra comparirebbe solo al secondo giro.
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

    if arg.profilo == "filegdb":
        for origine, sotto in (
            (libreria / "share" / "gdal", "share/gdal"),
            (libreria / "share" / "proj", "share/proj"),
        ):
            if origine.is_dir():
                shutil.copytree(origine, albero / sotto, dirs_exist_ok=True)

    # --- 1e. licenze, SBOM e provenienza ---------------------------------
    #
    # Lo stesso principio di Linux: si spedisce il testo di cio' che si
    # spedisce. La mappa file-a-pacchetto viene dallo stesso `conda-meta`,
    # perche' la catena e' la stessa.
    #
    # I file spediti si contano **dopo** le licenze e non prima: contandoli
    # prima, il manifesto elencava l'albero senza `LICENSES/`, e diceva quindi
    # di spedire meno di quanto spedisse. Un manifesto che sbaglia per difetto
    # e' peggio di uno che sbaglia per eccesso: chi lo verifica trova file che
    # nessuno ha dichiarato.
    print("1e. licenze, SBOM e provenienza", flush=True)
    meta = prefisso / "conda-meta"
    if not meta.is_dir():
        raise SystemExit(
            f"{meta} non esiste: il prefisso non e' stato materializzato da conda, e senza il "
            "registro del link la provenienza dei file sarebbe una congettura"
        )

    nomi_spediti = {p.name.lower() for p in albero.rglob("*") if p.is_file()}
    pacchetti: dict[str, dict] = {}
    con_testo_proprio = 0
    con_testo_canonico: list[dict] = []
    testi_esterni = lock.get("testi_di_licenza_esterni", {"identificatori": {}})
    cache_licenze = uscita / ".testi-di-licenza"

    for documento in sorted(meta.glob("*.json")):
        d = json.loads(documento.read_text(encoding="utf-8"))
        contribuiti = [
            f for f in d.get("files", []) if pathlib.PurePath(f).name.lower() in nomi_spediti
        ]
        if not contribuiti:
            continue
        pacchetti[d["name"]] = {
            "nome": d["name"],
            "versione": d["version"],
            "build": d["build"],
            "canale": d.get("channel", ""),
            "licenza": d.get("license", ""),
            "licenza_famiglia": d.get("license_family", ""),
            "directory_estratta": d.get("extracted_package_dir", ""),
            "file_spediti": len(contribuiti),
        }

    for nome_pacchetto in sorted(pacchetti):
        identita = pacchetti[nome_pacchetto]
        estratta = identita["directory_estratta"]
        origine = pathlib.Path(estratta) / "info" / "licenses" if estratta else None
        if origine is not None and origine.is_dir():
            shutil.copytree(origine, albero / "LICENSES" / nome_pacchetto, dirs_exist_ok=True)
            con_testo_proprio += 1
            continue
        if not identita["licenza"]:
            raise SystemExit(
                f"{nome_pacchetto}: nessun testo di licenza e nessuna licenza dichiarata, e "
                f"spedisce {identita['file_spediti']} file."
            )
        destinazione = albero / "LICENSES" / nome_pacchetto
        destinazione.mkdir(parents=True, exist_ok=True)
        identificatori = identificatori_spdx(identita["licenza"])
        for identificatore in identificatori:
            fonte = testi_esterni["identificatori"].get(identificatore)
            if fonte is None:
                raise SystemExit(
                    f"{nome_pacchetto} spedisce {identita['file_spediti']} file, dichiara "
                    f"«{identita['licenza']}» e non porta il proprio testo; «{identificatore}» "
                    "non e' fra i testi fissati nel lock."
                )
            (destinazione / f"{identificatore}.txt").write_bytes(
                procurati_testo(identificatore, fonte, cache_licenze)
            )
        con_testo_canonico.append(
            {
                "pacchetto": nome_pacchetto,
                "licenza_dichiarata": identita["licenza"],
                "identificatori": identificatori,
                "file_spediti": identita["file_spediti"],
            }
        )
    print(
        f"   licenze: {con_testo_proprio} con il proprio testo, "
        f"{len(con_testo_canonico)} con quello canonico",
        flush=True,
    )

    (albero / "LICENSES" / "PROVENIENZA.json").write_text(
        json.dumps(
            {
                "nota": (
                    "la licenza di ciascun pacchetto che ha messo un file in questo artefatto. "
                    "La fonte e' `conda-meta/` del prefisso, cioe' il registro che conda scrive "
                    "al momento del link: non e' una ricostruzione a posteriori."
                ),
                "pacchetti": [pacchetti[k] for k in sorted(pacchetti)],
                "con_testo_canonico": con_testo_canonico,
            },
            ensure_ascii=False,
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )

    (albero / "SBOM.spdx.json").write_text(
        json.dumps(
            {
                "spdxVersion": "SPDX-2.3",
                "dataLicense": "CC0-1.0",
                "SPDXID": "SPDXRef-DOCUMENT",
                "name": nome,
                "documentNamespace": f"https://plenora.invalid/{nome}",
                "creationInfo": {"creators": ["Tool: costruisci-artefatto-windows.py"]},
                "comment": (
                    "elenca i pacchetti che hanno messo almeno un file in questo artefatto, non "
                    "i pacchetti risolti dal lock: il lock ne risolve di piu', e cio' che non "
                    "viene spedito non sta su nessun disco."
                ),
                "packages": [
                    {
                        "SPDXID": f"SPDXRef-Package-{k}",
                        "name": k,
                        "versionInfo": pacchetti[k]["versione"],
                        "downloadLocation": pacchetti[k]["canale"] or "NOASSERTION",
                        "licenseConcluded": "NOASSERTION",
                        "licenseDeclared": pacchetti[k]["licenza"] or "NOASSERTION",
                        "filesAnalyzed": False,
                        "comment": f"build {pacchetti[k]['build']}",
                    }
                    for k in sorted(pacchetti)
                ],
            },
            ensure_ascii=False,
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )

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
        "revisione": revisione_del_repository(),
        "firma": firma,
        "licenze": {
            "con_testo_proprio": con_testo_proprio,
            "con_testo_canonico": len(con_testo_canonico),
            "senza_testo": 0,
        },
        "layout": (
            "le DLL stanno in `bin/` accanto all'eseguibile, perche' e' li' che il caricatore "
            "di Windows guarda. Non c'e' una `lib/` e non c'e' un RPATH: metterle altrove "
            "vorrebbe dire dire al caricatore dove guardare, cioe' dipendere dall'ambiente."
        ),
        # I file **dopo** le licenze, e ciascuno con il proprio digest: un
        # elenco di nomi dice che cosa c'era, un elenco di digest dice che cosa
        # c'e'. Chi verifica un artefatto estratto puo' rifare il conto senza
        # fidarsi del nome.
        "file": [
            {
                "percorso": str(percorso.relative_to(albero)),
                "sha256": distribuzione.sha256(percorso),
                "byte": percorso.stat().st_size,
            }
            for percorso in sorted(
                p for p in albero.rglob("*") if p.is_file() and p.name != "MANIFEST.json"
            )
        ],
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
