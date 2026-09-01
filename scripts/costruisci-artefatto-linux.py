#!/usr/bin/env python3
"""Costruisce l'artefatto Linux: albero installabile, archivio, manifesto, SBOM.

# Che cosa produce, e che cosa non produce

Produce un albero **autosufficiente** -- binario, librerie native, dati di GDAL
e di PROJ -- e l'archivio che lo contiene, con accanto i checksum, un manifesto
e un SBOM. Non firma niente e non pubblica niente: la firma e la pubblicazione
sono decisioni di rilascio, e questo script gira anche durante lo sviluppo.

Per questo il manifesto porta un **canale**. `prova` significa che l'artefatto
non e' una candidate: e' stato costruito per essere misurato, non installato da
qualcuno. Il gate di distribuzione lo legge, e un artefatto di prova non puo'
attraversare le verifiche che si fanno su una candidate.

# Perche' l'SBOM non elenca i pacchetti risolti

Il lock risolve cinquantotto pacchetti, ma l'artefatto ne spedisce una parte:
soltanto cio' che la chiusura `DT_NEEDED` di `bin/plenora-io` raggiunge, piu' i
dati che le librerie leggono. Un SBOM che elencasse i cinquantotto direbbe che
spediamo software che non spediamo -- e chi legge un SBOM lo legge per sapere
che cosa ha sul disco, non che cosa e' stato scaricato per costruirlo.

La stessa regola vale per le licenze: si spedisce la licenza di cio' che si
spedisce. La mappa file-a-pacchetto viene da `conda-meta/`, che conda scrive al
momento del link, e quindi non e' una ricostruzione: e' il registro di chi ha
messo quel file li'.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tarfile

RADICE = pathlib.Path(__file__).resolve().parent.parent
LOCK = RADICE / "scripts" / "linux-gdal-lock.json"
CHIUSURA = RADICE / "scripts" / "check-linux-gdal-runtime.py"


def esegui(comando: list[str], **kwargs) -> subprocess.CompletedProcess:
    stampabile = " ".join(str(c) for c in comando)
    print(f"  $ {stampabile}", flush=True)
    return subprocess.run(comando, check=True, **kwargs)


def sha256(percorso: pathlib.Path) -> str:
    digesto = hashlib.sha256()
    with percorso.open("rb") as f:
        for blocco in iter(lambda: f.read(1 << 20), b""):
            digesto.update(blocco)
    return digesto.hexdigest()


# --- la chiusura ----------------------------------------------------------
#
# Le funzioni che leggono un ELF stanno gia' in `check-linux-gdal-runtime.py`,
# che e' il programma che poi le verifica. Importarle di la' invece di
# riscriverle qui non e' pigrizia: due implementazioni della stessa lettura
# divergono, e divergerebbero proprio fra chi assembla e chi controlla -- cioe'
# fra le due parti che devono essere d'accordo.


def carica_lettore():
    import importlib.util

    spec = importlib.util.spec_from_file_location("chiusura", CHIUSURA)
    modulo = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(modulo)
    return modulo


def identificatori_spdx(espressione: str) -> list[str]:
    """Gli identificatori dentro un'espressione SPDX, in ordine.

    `GPL-3.0-only WITH GCC-exception-3.1` sono **due** testi, non uno: la
    seconda e' cio' che rende distribuibile un binario linkato alla prima, e
    consegnare solo la GPL sarebbe consegnare meta' della ragione per cui
    l'artefatto puo' esistere.

    Le parentesi e gli operatori si tolgono; cio' che resta sono i nomi. Non e'
    un parser SPDX completo, e non deve esserlo: se un giorno arrivasse
    un'espressione che questo non sa leggere, l'identificatore non si
    troverebbe nel lock e il costruttore si fermerebbe -- che e' l'esito
    giusto.
    """
    operatori = {"WITH", "AND", "OR"}
    parole = espressione.replace("(", " ").replace(")", " ").split()
    return [parola for parola in parole if parola.upper() not in operatori]


def procurati_testo(
    identificatore: str, fonte: dict, cache: pathlib.Path
) -> bytes:
    """Il testo fissato, verificato prima dell'uso.

    La verifica e' la stessa che si fa sui pacchetti, e per la stessa ragione:
    un testo che cambia sotto un checksum fissato deve far fallire il
    checksum, non entrare nell'artefatto perche' l'URL rispondeva.
    """
    cache.mkdir(parents=True, exist_ok=True)
    percorso = cache / f"{identificatore}.txt"
    if not percorso.is_file():
        esegui(["curl", "-sSL", "--fail", "-o", str(percorso), fonte["url"]])
    contenuto = percorso.read_bytes()
    if len(contenuto) != fonte["dimensione"]:
        raise SystemExit(
            f"{identificatore}: {len(contenuto)} byte, attesi {fonte['dimensione']}"
        )
    digesto = hashlib.sha256(contenuto).hexdigest()
    if digesto != fonte["sha256"]:
        raise SystemExit(
            f"{identificatore}: sha256 {digesto}, atteso {fonte['sha256']}. "
            "Il testo alla sorgente e' cambiato: va rifissato nel lock, non ignorato."
        )
    return contenuto


def testi_di_licenza(
    pacchetti: dict[str, dict],
    per_pacchetto: dict[str, list[str]],
    testi_esterni: dict,
    licenze: pathlib.Path,
    cache: pathlib.Path,
) -> tuple[int, list[dict]]:
    """Scrive in `licenze/` il testo della licenza di ogni componente.

    conda linka nel prefisso i soli file del pacchetto: `info/licenses` resta
    nella directory in cui il pacchetto e' stato estratto, e `conda-meta` ne
    porta il percorso. Copiarli di la' e' cio' che rende `LICENSES/` un
    contenuto invece di un elenco -- e un elenco di licenze non e' cio' che una
    licenza obbliga a distribuire.

    Alcuni pacchetti spediscono byte senza portare il proprio testo. Per quelli
    il testo si prende dall'autorita' dell'identificatore SPDX che dichiarano,
    fissato nel lock per URL, dimensione e sha256 come tutto il resto. Se non si
    riesce a procurarlo, si **ferma**: nominarlo in un elenco eviterebbe il
    silenzio senza consegnare la licenza, e questo costruttore ha gia' fatto
    quell'errore.

    Torna quanti hanno usato il proprio testo e l'elenco di quelli per cui e'
    stato preso quello canonico.
    """
    con_testo_proprio = 0
    con_testo_canonico: list[dict] = []
    for nome_pacchetto in sorted(pacchetti):
        identita = pacchetti[nome_pacchetto]
        quanti_file = len(per_pacchetto.get(nome_pacchetto, []))
        estratta = identita.get("directory_estratta") or ""
        origine = pathlib.Path(estratta) / "info" / "licenses" if estratta else None
        if origine is not None and origine.is_dir():
            # Il testo che il pacchetto porta con se' vince su quello canonico:
            # e' piu' vicino a cio' che ha effettivamente spedito.
            shutil.copytree(origine, licenze / nome_pacchetto, dirs_exist_ok=True)
            con_testo_proprio += 1
            continue

        if not identita["licenza"]:
            raise SystemExit(
                f"{nome_pacchetto}: nessun testo di licenza e nessuna licenza dichiarata, "
                f"e spedisce {quanti_file} file. Sotto quale licenza li si spedisca non e' "
                "una cosa che questo costruttore possa decidere."
            )
        destinazione = licenze / nome_pacchetto
        destinazione.mkdir(parents=True, exist_ok=True)
        identificatori = identificatori_spdx(identita["licenza"])
        for identificatore in identificatori:
            fonte = testi_esterni["identificatori"].get(identificatore)
            if fonte is None:
                raise SystemExit(
                    f"{nome_pacchetto} spedisce {quanti_file} file, dichiara "
                    f"«{identita['licenza']}» e non porta il proprio testo; "
                    f"«{identificatore}» non e' fra i testi fissati nel lock sotto "
                    "`testi_di_licenza_esterni`. Aggiungervelo, con URL, dimensione e "
                    "sha256: un artefatto non si spedisce senza la licenza di cio' che "
                    "contiene."
                )
            (destinazione / f"{identificatore}.txt").write_bytes(
                procurati_testo(identificatore, fonte, cache)
            )
        con_testo_canonico.append(
            {
                "pacchetto": nome_pacchetto,
                "licenza_dichiarata": identita["licenza"],
                "identificatori": identificatori,
                "file_spediti": quanti_file,
                "perche": (
                    "il pacchetto non porta `info/licenses`; il testo viene dall'autorita' "
                    "dell'identificatore SPDX che dichiara, fissato nel lock"
                ),
            }
        )
    return con_testo_proprio, con_testo_canonico


# --- la mappa file-a-pacchetto -------------------------------------------


def mappa_dei_pacchetti(prefisso: pathlib.Path) -> dict[str, dict]:
    """Da percorso relativo a pacchetto, come conda l'ha scritta al link."""
    mappa: dict[str, dict] = {}
    meta = prefisso / "conda-meta"
    if not meta.is_dir():
        raise SystemExit(
            f"{meta} non esiste: il prefisso non e' stato materializzato da conda, "
            "e senza il registro del link la provenienza dei file sarebbe una congettura"
        )
    for documento in sorted(meta.glob("*.json")):
        d = json.loads(documento.read_text(encoding="utf-8"))
        identita = {
            "nome": d["name"],
            "versione": d["version"],
            "build": d["build"],
            "canale": d.get("channel", ""),
            "licenza": d.get("license", ""),
            "licenza_famiglia": d.get("license_family", ""),
            # Dove conda ha estratto il pacchetto. I testi delle licenze stanno
            # li' sotto `info/licenses`, e non nel prefisso: conda vi linka i
            # soli file del pacchetto, e `info/` non e' fra quelli.
            "directory_estratta": d.get("extracted_package_dir", ""),
        }
        for f in d.get("files", []):
            mappa[f] = identita
    return mappa


def main() -> int:
    a = argparse.ArgumentParser(description=__doc__)
    a.add_argument("--prefisso", required=True, type=pathlib.Path,
                   help="il runtime GDAL materializzato dal lock")
    a.add_argument("--uscita", required=True, type=pathlib.Path,
                   help="dove costruire l'albero e l'archivio")
    a.add_argument("--versione", required=True)
    a.add_argument("--canale", default="prova", choices=["prova", "candidate"])
    a.add_argument("--profilo", default="filegdb", choices=["base", "filegdb"])
    a.add_argument("--salta-build", action="store_true",
                   help="riusa il binario gia' compilato, per iterare sull'assemblaggio")
    arg = a.parse_args()

    lock = json.loads(LOCK.read_text(encoding="utf-8"))
    prefisso = arg.prefisso.resolve()
    uscita = arg.uscita.resolve()
    nome = f"plenora-io-{arg.versione}-linux-x86_64-{arg.profilo}"
    albero = uscita / nome

    if albero.exists():
        shutil.rmtree(albero)
    for sotto in ("bin", "lib", "share", "LICENSES"):
        (albero / sotto).mkdir(parents=True, exist_ok=True)

    # --- 1. il binario ----------------------------------------------------
    #
    # L'RPATH e' `$ORIGIN/../lib` e non un percorso assoluto: e' cio' che rende
    # l'albero spostabile. `--disable-new-dtags` chiede un RPATH vero invece di
    # un RUNPATH perche' il RUNPATH non e' transitivo -- ma qui non servirebbe,
    # dato che ogni libreria del runtime porta gia' il proprio. Lo si chiede lo
    # stesso: la transitivita' e' una proprieta' che non vogliamo dipenda da
    # come conda ha costruito i suoi .so.
    target = RADICE / "target" / "artefatto"
    binario = target / "release" / "plenora-io"
    if not arg.salta_build:
        ambiente = dict(os.environ)
        ambiente["PKG_CONFIG_PATH"] = str(prefisso / "lib" / "pkgconfig")
        ambiente["LD_LIBRARY_PATH"] = str(prefisso / "lib")
        ambiente["CARGO_TARGET_DIR"] = str(target)
        ambiente["RUSTFLAGS"] = (
            "-C link-arg=-Wl,-rpath,$ORIGIN/../lib "
            "-C link-arg=-Wl,--disable-new-dtags"
        )
        comando = [
            "cargo", "build", "--release", "--locked",
            "-p", "plenora-io-cli",
        ]
        if arg.profilo == "filegdb":
            comando += ["--features", "gdal-backend"]
        print("1. compilazione", flush=True)
        esegui(comando, cwd=RADICE, env=ambiente)
    if not binario.is_file():
        raise SystemExit(f"{binario} non esiste")
    shutil.copy2(binario, albero / "bin" / "plenora-io")

    # --- 2. la chiusura, a partire dal binario vero -----------------------
    #
    # La radice e' `bin/plenora-io` e non `libgdal.so`: la domanda e' che cosa
    # serve **all'artefatto**, e non che cosa serve a GDAL. Sono due chiusure
    # diverse, e la seconda non contiene la prima.
    print("2. chiusura DT_NEEDED dal binario", flush=True)
    lettore = carica_lettore()
    interne, esterne = lettore.chiusura(albero / "bin" / "plenora-io", prefisso)
    print(f"   interne {len(interne)}, esterne {len(esterne)}", flush=True)

    attese = set(lock["contratto_di_verifica"]["dipendenze_esterne_attese"])
    if esterne != attese:
        raise SystemExit(
            "le dipendenze esterne non sono quelle attese dal lock.\n"
            f"  in piu':  {sorted(esterne - attese)}\n"
            f"  mancanti: {sorted(attese - esterne)}\n"
            "Ogni variazione e' un cambio di cio' che l'artefatto porta con se', "
            "e vuole un lock nuovo."
        )

    # --- 3. i file spediti ------------------------------------------------
    # Si copia sotto il **SONAME**, non sotto il nome del file risolto.
    #
    # Nel prefisso `lib/libgdal.so.35` e' un symlink a `libgdal.so.35.3.9.3`, e
    # `libgdal.so.35` e' il nome che il loader cerchera': e' quello scritto nel
    # `DT_NEEDED` di chi la usa. Copiare il file risolto e basta produce un
    # albero che contiene la libreria e non la trova -- un difetto che si vede
    # solo eseguendo, e che qui si e' visto perche' il controllo parte dai nomi
    # richiesti e non dai file presenti.
    #
    # Quando piu' SONAME risolvono allo stesso file si copia una volta sola e si
    # collega: duplicare i byte funzionerebbe, ma direbbe che sono due librerie.
    # Un `DT_NEEDED` assoluto non e' un nome: e' un percorso, e sopravvive allo
    # spostamento soltanto se quella directory esiste ancora. Qui se ne tiene il
    # solo nome del file, e chi lo dichiara viene riscritto piu' sotto, quando
    # l'albero e' completo. Il collasso puo' far coincidere due chiavi -- e le
    # fa coincidere: `/A/runtime/lib/libsqlite3.so` e `libsqlite3.so` sono la
    # stessa libreria chiesta in due modi -- quindi si deduplica prima.
    per_nome: dict[str, pathlib.Path] = {}
    for richiesto, origine in sorted(interne.items()):
        base = richiesto.rsplit("/", 1)[-1]
        precedente = per_nome.setdefault(base, origine)
        if precedente != origine:
            raise SystemExit(
                f"«{base}» e' chiesto in due modi che risolvono a file diversi: "
                f"{precedente} e {origine}. Sceglierne uno sarebbe una decisione "
                "presa in silenzio su quale ABI spedire."
            )

    spediti: list[pathlib.Path] = []
    gia_copiati: dict[pathlib.Path, str] = {}
    for soname, origine in sorted(per_nome.items()):
        destinazione = albero / "lib" / soname
        primo = gia_copiati.get(origine)
        if primo is None:
            shutil.copy2(origine, destinazione, follow_symlinks=True)
            gia_copiati[origine] = soname
        else:
            destinazione.symlink_to(primo)
        spediti.append(destinazione)

    # --- 3b. i DT_NEEDED assoluti ----------------------------------------
    #
    # `libgdal.so.35` di conda-forge dichiara `libsqlite3.so` con un percorso
    # **assoluto**. Non e' cotto nel pacchetto: e' il placeholder del prefisso
    # di costruzione, che la rilocazione di conda ha sostituito con il prefisso
    # di materializzazione. Il risultato e' una libreria che cerca una sorella
    # in una directory precisa, e che smette di caricarsi appena quella
    # directory non esiste piu'. Un artefatto cosi' funziona finche' resta dove
    # e' nato, che e' esattamente cio' che un artefatto non deve fare.
    #
    # Si riscrivono con il solo nome, cosi' che li risolva l'RPATH come tutte le
    # altre. Modificare un binario di terze parti va detto, e il manifesto lo
    # dice: `normalizzazioni` elenca che cosa e' stato cambiato e perche'.
    normalizzazioni: list[dict] = []
    for elf in sorted((albero / "lib").glob("*")):
        if elf.is_symlink() or not elf.is_file():
            continue
        try:
            richiesti = lettore.dt_needed(elf)
        except Exception:  # noqa: BLE001 -- non e' un ELF, e non e' un errore
            continue
        for richiesto in richiesti:
            if not richiesto.startswith("/"):
                continue
            base = richiesto.rsplit("/", 1)[-1]
            if not (albero / "lib" / base).exists():
                raise SystemExit(
                    f"{elf.name} dichiara «{richiesto}», e «{base}» non e' fra i file "
                    "spediti: normalizzarlo lo farebbe fallire al caricamento invece che "
                    "all'assemblaggio."
                )
            esegui(["patchelf", "--replace-needed", richiesto, base, str(elf)])
            normalizzazioni.append(
                {"file": f"lib/{elf.name}", "da": richiesto, "a": base}
            )
    if normalizzazioni:
        print(f"3b. {len(normalizzazioni)} DT_NEEDED assoluti normalizzati", flush=True)

    if arg.profilo == "filegdb":
        for origine, sotto in (
            (prefisso / "share" / "gdal", "share/gdal"),
            (prefisso / "share" / "proj", "share/proj"),
            (prefisso / "lib" / "gdalplugins", "lib/gdalplugins"),
        ):
            destinazione = albero / sotto
            if origine.is_dir():
                shutil.copytree(origine, destinazione, dirs_exist_ok=True)
            else:
                # `gdalplugins` puo' non esistere: i driver che ci servono sono
                # compilati dentro il core. La directory si crea comunque, ed e'
                # una decisione: `GDAL_DRIVER_PATH` puntata a una directory
                # vuota **nostra** e' cio' che tiene fuori dal processo un
                # plugin di sistema. Senza, il default cotto dentro resterebbe.
                destinazione.mkdir(parents=True, exist_ok=True)
            spediti.extend(p for p in destinazione.rglob("*") if p.is_file())

    # --- 4. licenze e SBOM, legati a cio' che si spedisce ------------------
    print("4. licenze e SBOM sui file spediti", flush=True)
    mappa = mappa_dei_pacchetti(prefisso)
    testi_esterni = lock["testi_di_licenza_esterni"]
    cache_licenze = uscita / ".testi-di-licenza"
    pacchetti: dict[str, dict] = {}
    per_pacchetto: dict[str, list[str]] = {}
    non_attribuiti: list[str] = []
    for f in spediti:
        relativo = str(f.relative_to(albero))
        # `lib/foo.so` nell'albero viene da `lib/foo.so` nel prefisso; per i
        # dati il percorso coincide gia'.
        identita = mappa.get(relativo)
        if identita is None:
            non_attribuiti.append(relativo)
            continue
        pacchetti.setdefault(identita["nome"], identita)
        per_pacchetto.setdefault(identita["nome"], []).append(relativo)

    if non_attribuiti:
        raise SystemExit(
            "file spediti senza un pacchetto che li rivendichi:\n  "
            + "\n  ".join(non_attribuiti[:20])
            + "\nUn file senza provenienza non ha licenza, e un SBOM che lo omette "
            "e' incompleto proprio dove serve."
        )

    licenze = albero / "LICENSES"
    con_testo_proprio, con_testo_canonico = testi_di_licenza(
        pacchetti, per_pacchetto, testi_esterni, licenze, cache_licenze
    )
    print(
        f"   licenze: {con_testo_proprio} pacchetti con il proprio testo, "
        f"{len(con_testo_canonico)} con il testo canonico dell'identificatore dichiarato",
        flush=True,
    )

    (licenze / "PROVENIENZA.json").write_text(
        json.dumps(
            {
                "nota": (
                    "la licenza di ciascun pacchetto che ha messo un file in questo artefatto. "
                    "La fonte e' `conda-meta/` del prefisso, cioe' il registro che conda scrive "
                    "al momento del link: non e' una ricostruzione a posteriori. I testi stanno "
                    "nelle directory accanto, una per pacchetto."
                ),
                "pacchetti": [pacchetti[k] for k in sorted(pacchetti)],
                "con_testo_canonico": con_testo_canonico,
                "con_testo_canonico_nota": (
                    "questi pacchetti non portano `info/licenses` nel proprio archivio, e "
                    "spediscono comunque byte. Il testo accanto viene dall'autorita' "
                    "dell'identificatore SPDX che dichiarano, fissata nel lock per URL, "
                    "dimensione e sha256. Non e' il testo che il progetto ha spedito con i "
                    "propri sorgenti: e' il testo della licenza che dichiara, ed e' quanto di "
                    "piu' vicino si possa consegnare senza scaricare il tarball di GCC per "
                    "estrarne un file."
                ),
            },
            ensure_ascii=False,
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )

    componenti = []
    for nome_pacchetto in sorted(pacchetti):
        p = pacchetti[nome_pacchetto]
        componenti.append(
            {
                "SPDXID": f"SPDXRef-Package-{nome_pacchetto}",
                "name": nome_pacchetto,
                "versionInfo": p["versione"],
                "downloadLocation": p["canale"] or "NOASSERTION",
                "licenseConcluded": "NOASSERTION",
                "licenseDeclared": p["licenza"] or "NOASSERTION",
                "filesAnalyzed": False,
                "comment": f"build {p['build']}",
            }
        )
    sbom = {
        "spdxVersion": "SPDX-2.3",
        "dataLicense": "CC0-1.0",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": nome,
        "documentNamespace": f"https://plenora.invalid/{nome}",
        "creationInfo": {"creators": ["Tool: costruisci-artefatto-linux.py"]},
        "comment": (
            "elenca i pacchetti che hanno messo almeno un file in questo artefatto, "
            "non i pacchetti risolti dal lock: il lock ne risolve di piu', e cio' che "
            "non viene spedito non sta su nessun disco."
        ),
        "packages": componenti,
    }
    (albero / "SBOM.spdx.json").write_text(
        json.dumps(sbom, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )

    # --- 5. il manifesto --------------------------------------------------
    manifesto = {
        "nome": nome,
        "versione": arg.versione,
        "piattaforma": "linux-x86_64",
        "profilo": arg.profilo,
        "canale": arg.canale,
        "non_release": arg.canale != "candidate",
        "canale_nota": (
            "«prova» significa che questo artefatto e' stato costruito per essere "
            "misurato, non installato. Non e' firmato, non e' pubblicato, e il gate "
            "di distribuzione lo rifiuta ovunque si pretenda una candidate."
        ),
        "gdal": lock["gdal_version"],
        "lock": sha256(LOCK),
        # Il prefisso in cui il runtime e' stato materializzato, cioe' cio' che
        # i binari nominano dentro di se'. Sta qui perche' il controllo non
        # debba farselo dire a mano: passarne uno sbagliato non trova nessun
        # percorso assoluto, e senza la guardia che rende rosso lo zero
        # sembrerebbe un artefatto pulito. E' gia' successo.
        "prefisso_di_costruzione": str(prefisso),
        "licenze": {
            "con_testo_proprio": con_testo_proprio,
            "con_testo_canonico": len(con_testo_canonico),
            "senza_testo": 0,
            "senza_testo_nota": (
                "e' sempre zero, e non per fortuna: un pacchetto che spedisce byte senza un "
                "testo ferma il costruttore. Prima erano tre, nominati in un elenco -- il che "
                "evitava il silenzio ma non consegnava la licenza."
            ),
        },
        "normalizzazioni": normalizzazioni,
        "normalizzazioni_nota": (
            "i `DT_NEEDED` riscritti da `patchelf`. Erano percorsi assoluti al prefisso di "
            "materializzazione -- il placeholder di conda sostituito al link -- e un artefatto "
            "che li conservasse smetterebbe di caricarsi appena quel prefisso non esiste. Sono "
            "elencati perche' modificare un binario di terze parti va detto: il file spedito non "
            "e' byte per byte quello del pacchetto, e chi verifica un checksum a monte deve "
            "sapere perche' non corrisponde."
        ),
        "file": sorted(str(p.relative_to(albero)) for p in spediti),
    }
    (albero / "MANIFEST.json").write_text(
        json.dumps(manifesto, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )

    # --- 6. archivio e checksum ------------------------------------------
    print("6. archivio e checksum", flush=True)
    archivio = uscita / f"{nome}.tar.gz"
    if archivio.exists():
        archivio.unlink()
    with tarfile.open(archivio, "w:gz") as t:
        t.add(albero, arcname=nome)
    (uscita / f"{nome}.tar.gz.sha256").write_text(
        f"{sha256(archivio)}  {archivio.name}\n", encoding="utf-8"
    )
    print(f"   {archivio}  ({archivio.stat().st_size} byte)", flush=True)
    print(f"   sha256 {sha256(archivio)}", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
