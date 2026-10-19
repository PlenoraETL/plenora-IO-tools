#!/usr/bin/env python3
"""L'audit dei fork governati: legge, confronta, non tocca niente.

# Perche' un audit e non un altro gate

I tre gate dei fork verificano ciascuno il proprio: che l'albero coincida col
lock, che il registro sia coerente, che `Cargo.toml` usi la patch. Sono
fail-closed e girano nel checkpoint.

Cio' che nessuno guarda e' l'insieme: quanti fork ci sono, quanto e' grande il
delta di ciascuno, se il delta **dichiarato** coincida con quello reale rispetto
all'upstream, e se le tre pratiche siano ancora le stesse. Sono domande che si
pongono ogni tanto, non a ogni commit, e la risposta e' un documento da leggere
invece di un rosso da riparare.

# Read-only, e sul serio

Non scrive nel repository, non ricalcola lock, non tocca `vendor/`. L'unica
scrittura e' il rapporto, dove glielo si chiede. Un audit che correggesse cio'
che trova cambierebbe l'oggetto misurato mentre lo misura, e il rapporto
descriverebbe uno stato che non e' mai esistito.

# Il delta reale si misura dal `.crate`

Il pacchetto originale sta nella cache di cargo, con il proprio `crate_sha256`
nel lock. Confrontare l'albero vendorizzato con quello estratto dice **quali**
file sono cambiati davvero -- non quali il lock dichiara. Le due cose devono
coincidere, e se non coincidono e' il caso di saperlo prima che lo dica un gate.
"""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import pathlib
import sys
import tarfile
import tomllib
import urllib.request

try:
    from . import strict_json
except ImportError:
    import sys
    from pathlib import Path

    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import strict_json

ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import fork_comune  # noqa: E402
from fork_comune import fini_riga_divergenti, impronta, insieme_versionato  # noqa: E402

FORK = ("gdal", "dxf", "shapefile", "parquet")


def voci_del_delta(delta: object) -> list:
    """I file del delta funzionale, nelle due forme dei registri.

    I registri di gdal, dxf e shapefile lo scrivono come elenco di voci con
    `file`; quello di parquet come mappa da file a motivazione. L'audit leggeva
    solo la prima forma, e sul registro di parquet si fermava con
    `AttributeError` invece di riportare.
    """
    if isinstance(delta, dict):
        return sorted(delta)
    if isinstance(delta, list):
        return [voce.get("file") if isinstance(voce, dict) else voce for voce in delta]
    raise SystemExit("audit: `delta_funzionale` non e' ne' un elenco ne' una mappa")

def normalizza_eol(dati: bytes) -> bytes:
    """I fine riga non sono un delta.

    Il `.crate` porta i file come upstream li ha scritti; il nostro albero li
    porta normalizzati a LF da `.gitattributes`. Contare quella differenza come
    una modifica farebbe sembrare cambiati venti file che nessuno ha toccato --
    ed e' lo stesso equivoco che rendeva l'impronta dei fork dipendente dalla
    piattaforma.
    """
    return dati.replace(b"\r\n", b"\n")


def necessario_alla_libreria(nome: str) -> bool:
    """Questo file entrerebbe nella compilazione della libreria?

    E' la domanda che separa un'assenza innocua da una grave. Un fork che non
    vendorizza `examples/` o `.github/` non ha cambiato niente: cargo, quando
    compila una **dipendenza**, non guarda esempi, test o benchmark. Un fork a
    cui mancasse un file sotto `src/` avrebbe invece una libreria diversa da
    quella che il lock dice di aver forkato, e nessuna impronta lo direbbe --
    l'impronta descrive cio' che c'e', non cio' che manca.

    La regola e' la stessa per tutti e tre: non dipende da elenchi per fork, che
    andrebbero mantenuti e che si allineerebbero a cio' che si e' gia' fatto
    invece che a cio' che serve.
    """
    return (
        nome == "Cargo.toml"
        or nome.startswith("src/")
        or nome == "build.rs"
        or nome.startswith("build/")
        or nome.split("/")[0].upper().startswith(("LICENSE", "COPYING"))
    )


#: Come si legge un'assenza, per chi legge il rapporto. Non cambia il verdetto
#: -- quello lo da' `necessario_alla_libreria` -- ma dice a colpo d'occhio che
#: cosa il perimetro ha lasciato fuori.
CATEGORIE_ESCLUSE = (
    ("esempi", lambda n: n.startswith("examples/")),
    ("test upstream", lambda n: n.startswith(("tests/", "benches/"))),
    ("fixture", lambda n: n.startswith("fixtures/")),
    ("integrazione continua", lambda n: n.startswith((".github/", ".cargo/", "script/"))),
    ("configurazione dell'editor", lambda n: n.startswith((".vscode/", ".devcontainer/")) or n in {".editorconfig", ".gitattributes", "clippy.toml"}),
    ("documentazione", lambda n: n.split("/")[0] in {"CHANGES.md", "CHANGELOG.md", "CODE_OF_CONDUCT.md"}),
    ("lockfile", lambda n: n == "Cargo.lock"),
    ("script di build upstream", lambda n: n.startswith("build-and-test")),
)


def categoria(nome: str) -> str:
    for etichetta, prova in CATEGORIE_ESCLUSE:
        if prova(nome):
            return etichetta
    return "altro"


#: I file che `cargo package` aggiunge al `.crate` e che non sono codice.
#:
#: Un fork che non li porta non ha **modificato** niente: ha vendorizzato meno.
#: Contarli fra i file «tolti» insieme a un sorgente sparito metterebbe sullo
#: stesso piano due fatti di gravita' opposta.
METADATI_DI_PACKAGING = frozenset(
    {".cargo_vcs_info.json", "Cargo.toml.orig", ".gitignore"}
)

#: Dove si prende il pacchetto originale quando la cache non ce l'ha.
#:
#: L'audit non si fida di cio' che scarica: verifica il `sha256` contro quello
#: che il lock registra, e se non coincide non lo usa. Il lock e' l'autorita' --
#: se qualcuno servisse un altro pacchetto sotto lo stesso nome, il confronto si
#: fermerebbe invece di descrivere un delta rispetto al pacchetto sbagliato.
CRATES_IO = "https://static.crates.io/crates/{nome}/{nome}-{versione}.crate"

#: Dove cargo tiene i `.crate` scaricati. Puo' non esserci -- un albero appena
#: clonato non ha una cache -- e allora il confronto con l'upstream si salta
#: **dicendolo**, invece di dare per buono il delta dichiarato.
CACHE = pathlib.Path("/usr/local/cargo/registry/cache")


def rilievi_di_licenza(nome: str, registro: dict, vendor: pathlib.Path) -> list[str]:
    """Le licenze che il fork ridistribuisce, verificate sui byte. Senza rete.

    Ogni file di `licenze` nel registro deve esserci, essere **versionato** (un
    file presente solo sul disco di chi lancia non arriva a chi riceve il
    sorgente) e avere lo sha256 registrato, a fine riga normalizzati: il testo
    di una licenza e' la condizione per ridistribuire, e un testo riscritto e'
    un'altra licenza. Anche `file_licenza` e `file_notice` devono comparire fra
    le `licenze`: dichiararli senza impronta sarebbe un'affermazione che nessuno
    verifica, ed e' esattamente come il fork di gdal ha ridistribuito codice MIT
    senza il suo testo fino alla 4.1.1.
    """
    rilievi: list[str] = []
    licenze = registro.get("licenze")
    if not isinstance(licenze, list) or not licenze:
        return [f"{nome}: il registro non elenca `licenze` con la loro impronta"]
    versionati = {
        percorso.relative_to(vendor).as_posix() for percorso in insieme_versionato(vendor)
    }
    elencati = set()
    for voce in licenze:
        file = voce.get("file") if isinstance(voce, dict) else None
        atteso = voce.get("sha256") if isinstance(voce, dict) else None
        if not isinstance(file, str) or not isinstance(atteso, str):
            rilievi.append(f"{nome}: una voce di `licenze` non ha `file` e `sha256`")
            continue
        elencati.add(file)
        percorso = vendor / file
        if not percorso.is_file():
            rilievi.append(
                f"{nome}: il registro dichiara «{file}» e il file non c'e': si "
                "ridistribuisce codice altrui senza la sua licenza"
            )
            continue
        if file not in versionati:
            rilievi.append(f"{nome}: «{file}» c'e' sul disco ma non e' versionato")
        ottenuto = hashlib.sha256(normalizza_eol(percorso.read_bytes())).hexdigest()
        if ottenuto != atteso:
            rilievi.append(
                f"{nome}: il testo di «{file}» non e' quello registrato (sha256 "
                f"{ottenuto[:12]}, il registro vuole {atteso[:12]})"
            )
    for chiave in ("file_licenza", "file_notice"):
        dichiarato = registro.get(chiave)
        if dichiarato and dichiarato not in elencati:
            rilievi.append(
                f"{nome}: `{chiave}` dichiara «{dichiarato}» e `licenze` non ne "
                "registra l'impronta"
            )
    return rilievi


def crate_originale(
    nome: str, versione: str, atteso: str, scarico: pathlib.Path | None
) -> tuple[pathlib.Path | None, str]:
    """Il `.crate` upstream: dalla cache se c'e', altrimenti da crates.io.

    Torna anche **da dove** viene, perche' un audit che confrontasse con un
    pacchetto di provenienza ignota direbbe meno di quanto sembra.
    """
    if CACHE.is_dir():
        for indice in CACHE.iterdir():
            candidato = indice / f"{nome}-{versione}.crate"
            if candidato.is_file():
                return candidato, "cache di cargo"

    if scarico is None:
        return None, "non cercato in rete"

    destinazione = scarico / f"{nome}-{versione}.crate"
    if not destinazione.is_file():
        try:
            with urllib.request.urlopen(
                CRATES_IO.format(nome=nome, versione=versione), timeout=60
            ) as risposta:
                destinazione.write_bytes(risposta.read())
        except Exception as errore:  # noqa: BLE001 -- la ragione va riportata
            return None, f"crates.io non raggiungibile: {errore}"

    ottenuto = hashlib.sha256(destinazione.read_bytes()).hexdigest()
    if ottenuto != atteso:
        # Non e' il pacchetto che il lock registra: confrontarci il fork
        # descriverebbe un delta rispetto a un originale sbagliato.
        return None, f"scaricato con sha256 {ottenuto[:12]}, il lock ne vuole un altro"
    return destinazione, "crates.io, verificato contro il lock"


def delta_reale(vendor: pathlib.Path, crate: pathlib.Path) -> dict[str, list[str]]:
    """Quali file differiscono dall'upstream, e quali sono stati aggiunti o tolti."""
    originali: dict[str, bytes] = {}
    with tarfile.open(crate, "r:gz") as archivio:
        for voce in archivio.getmembers():
            if not voce.isfile():
                continue
            # Il `.crate` ha una radice `<nome>-<versione>/`: si toglie, cosi' i
            # nomi coincidono con quelli dell'albero vendorizzato.
            relativo = voce.name.split("/", 1)[1] if "/" in voce.name else voce.name
            estratto = archivio.extractfile(voce)
            if estratto is not None:
                originali[relativo] = estratto.read()

    nostri = {
        percorso.relative_to(vendor).as_posix(): percorso.read_bytes()
        for percorso in insieme_versionato(vendor)
    }

    assenti = sorted(set(originali) - set(nostri))
    fuori_perimetro = [
        n
        for n in assenti
        if n not in METADATI_DI_PACKAGING and not necessario_alla_libreria(n)
    ]
    per_categoria: dict[str, int] = {}
    for nome in fuori_perimetro:
        per_categoria[categoria(nome)] = per_categoria.get(categoria(nome), 0) + 1
    return {
        "metadati_non_vendorizzati": [n for n in assenti if n in METADATI_DI_PACKAGING],
        "fuori_perimetro": len(fuori_perimetro),
        "fuori_perimetro_per_categoria": dict(sorted(per_categoria.items())),
        "modificati": sorted(
            nome
            for nome, dati in nostri.items()
            if nome in originali
            and normalizza_eol(originali[nome]) != normalizza_eol(dati)
        ),
        "aggiunti": sorted(set(nostri) - set(originali)),
        # Cio' che resta: un file che **sarebbe stato compilato** e non c'e'.
        # Quello e' un rilievo, e per questo l'elenco e' nominale invece che
        # contato.
        "mancanti_alla_libreria": [
            n
            for n in assenti
            if n not in METADATI_DI_PACKAGING and necessario_alla_libreria(n)
        ],
    }


def audita(nome: str, scarico: pathlib.Path | None) -> dict:
    lock = strict_json.loads(
        (ROOT / "scripts" / f"{nome}-fork-lock.json").read_text(encoding="utf-8")
    )
    registro = strict_json.loads(
        (ROOT / "assurance" / "registries" / f"vendor-{nome}-fork.json").read_text(
            encoding="utf-8"
        )
    )
    vendor = ROOT / lock["vendor_path"]
    conteggio, digest = impronta(vendor)

    voce: dict = {
        "package": lock["package"],
        "version": lock["version"],
        "file": conteggio,
        "tree_sha256": digest,
        "impronta_coincide": digest == lock["tree_sha256"],
        "fini_riga_divergenti": fini_riga_divergenti(vendor),
        "delta_dichiarato": {
            "funzionale": sorted(lock["functional_delta_files"]),
            "packaging": sorted(lock.get("packaging_delta_files", [])),
        },
        "licenza_upstream": registro.get("licenza_upstream"),
        "voci_del_registro": voci_del_delta(registro.get("delta_funzionale", [])),
    }

    # La licenza: quella che redistribuiamo, non quella che diciamo.
    #
    # Vendorizzare significa **ridistribuire** codice di qualcun altro, e la
    # condizione per farlo e' portarne la licenza intatta. Un registro che
    # dichiarasse MIT accanto a un albero senza il file, o con un file
    # modificato, sarebbe un'affermazione sulla nostra buona fede invece che sui
    # byte -- ed e' esattamente il genere di dichiarazione che un audit esiste
    # per non accettare.
    manifesto = tomllib.loads((vendor / "Cargo.toml").read_text(encoding="utf-8"))
    file_licenza = registro.get("file_licenza")
    voce["licenza"] = {
        "dichiarata_nel_registro": registro.get("licenza_upstream"),
        "dichiarata_nel_manifesto": manifesto.get("package", {}).get("license"),
        "file": file_licenza,
        "file_presente": bool(file_licenza) and (vendor / file_licenza).is_file(),
    }

    crate, provenienza = crate_originale(
        lock["package"], lock["version"], lock["crate_sha256"], scarico
    )
    if crate is None:
        voce["confronto_con_upstream"] = {
            "eseguito": False,
            "perche": (
                f"il pacchetto originale non si e' potuto ottenere ({provenienza}): "
                "senza, il delta reale non si puo' misurare. Il confronto e' "
                "saltato, non dato per buono."
            ),
        }
        return voce

    verificato = hashlib.sha256(crate.read_bytes()).hexdigest()
    reale = delta_reale(vendor, crate)
    voce["confronto_con_upstream"] = {
        "eseguito": True,
        "crate": crate.name,
        "provenienza": provenienza,
        "crate_sha256": verificato,
        "crate_sha256_coincide": verificato == lock["crate_sha256"],
        **reale,
    }
    if voce["licenza"]["file_presente"]:
        # Il testo deve essere quello di upstream: e' la condizione, non una
        # formalita'. Una licenza riscritta e' una licenza diversa.
        with tarfile.open(crate, "r:gz") as archivio:
            radice = f"{lock['package']}-{lock['version']}"
            try:
                originale = archivio.extractfile(f"{radice}/{file_licenza}")
                testo = originale.read() if originale is not None else None
            except KeyError:
                testo = None
        if testo is None:
            # Il pacchetto pubblicato non porta il file: il testo viene da un
            # altro punto di upstream (il registro dice quale), e lo verifica
            # l'impronta di `licenze`, non questo confronto.
            voce["licenza"]["identica_a_upstream"] = None
            voce["licenza"]["assente_dal_crate"] = True
        else:
            voce["licenza"]["identica_a_upstream"] = normalizza_eol(
                testo
            ) == normalizza_eol((vendor / file_licenza).read_bytes())

    dichiarati = set(voce["delta_dichiarato"]["funzionale"]) | set(
        voce["delta_dichiarato"]["packaging"]
    )
    voce["confronto_con_upstream"]["non_dichiarati"] = sorted(
        set(reale["modificati"]) - dichiarati
    )
    # Un file aggiunto (la licenza di gdal, che il crate non porta) e' un
    # delta anche se non e' «modificato»: non c'e' un originale con cui
    # coincidere.
    voce["confronto_con_upstream"]["dichiarati_e_identici"] = sorted(
        dichiarati - set(reale["modificati"]) - set(reale["aggiunti"])
    )
    return voce


def risoluzione() -> dict:
    """Come il workspace risolve i tre fork, e che nessuna `[patch]` ricompaia.

    Il gate di ogni fork guarda il proprio nome. Nessuno guarda l'**insieme**:
    che i tre siano dipendenze dirette verso `vendor/`, che nessuna dipendenza
    per percorso esca da `vendor/` con un nome di fork, e che un quarto nome
    non si sia aggiunto senza che qualcuno se ne accorgesse. Sono domande sul
    sistema, non sul singolo fork, e per questo stanno qui.

    Fino alla 4.1.1 la stessa funzione verificava le `[patch.crates-io]` del
    workspace e del fuzz. Una patch pero' vale solo dal workspace radice, e chi
    dipendeva da noi riceveva i crate upstream: ora i fork sono dipendenze
    dirette con nome proprio, e una patch e' un rilievo.
    """
    radice = ROOT / "Cargo.toml"
    dati = tomllib.loads(radice.read_text(encoding="utf-8"))
    dirette: dict[str, dict] = {}
    for nome, valore in dati.get("workspace", {}).get("dependencies", {}).items():
        if not isinstance(valore, dict) or "path" not in valore:
            continue
        assoluto = (radice.parent / valore["path"]).resolve()
        dirette[nome] = {
            "package": valore.get("package"),
            "path": valore["path"],
            "dentro_vendor": assoluto.is_relative_to(ROOT / "vendor"),
            "esiste": assoluto.is_dir(),
        }
    return {"dipendenze_dirette": dirette, "patch": fork_comune.patch_presenti()}


def main(argv: list[str] | None = None) -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument("--rapporto", type=pathlib.Path, default=None)
    argomenti.add_argument(
        "--scarico",
        type=pathlib.Path,
        default=None,
        help=(
            "dove tenere i `.crate` presi da crates.io quando la cache non li "
            "ha. Fuori dal repository: l'audit non ci scrive dentro."
        ),
    )
    opzioni = argomenti.parse_args(argv)
    if opzioni.scarico is not None:
        opzioni.scarico.mkdir(parents=True, exist_ok=True)

    rapporto = {"fork": {nome: audita(nome, opzioni.scarico) for nome in FORK}}
    rilievi: list[str] = []
    licenze: list[str] = []
    for nome in FORK:
        lock = strict_json.loads(
            (ROOT / "scripts" / f"{nome}-fork-lock.json").read_text(encoding="utf-8")
        )
        registro = strict_json.loads(
            (ROOT / "assurance" / "registries" / f"vendor-{nome}-fork.json").read_text(
                encoding="utf-8"
            )
        )
        licenze += rilievi_di_licenza(nome, registro, ROOT / lock["vendor_path"])

    for nome, voce in rapporto["fork"].items():
        if not voce["impronta_coincide"]:
            rilievi.append(f"{nome}: l'albero non coincide col lock")
        if voce["fini_riga_divergenti"]:
            rilievi.append(
                f"{nome}: {len(voce['fini_riga_divergenti'])} file coi fine riga "
                "diversi da quelli che git registrerebbe"
            )
        licenza = voce["licenza"]
        if licenza["dichiarata_nel_registro"] != licenza["dichiarata_nel_manifesto"]:
            licenze.append(
                f"{nome}: il registro dice «{licenza['dichiarata_nel_registro']}» e "
                f"il manifesto «{licenza['dichiarata_nel_manifesto']}»"
            )

        confronto = voce["confronto_con_upstream"]
        if not confronto["eseguito"]:
            continue
        if licenza.get("identica_a_upstream") is False:
            licenze.append(
                f"{nome}: il testo di «{licenza['file']}» non e' quello di "
                "upstream"
            )
        if not confronto["crate_sha256_coincide"]:
            rilievi.append(f"{nome}: il `.crate` in cache non e' quello del lock")
        if confronto["non_dichiarati"]:
            rilievi.append(
                f"{nome}: modificati rispetto a upstream e non dichiarati: "
                f"{confronto['non_dichiarati']}"
            )
        if confronto["dichiarati_e_identici"]:
            rilievi.append(
                f"{nome}: dichiarati nel delta e identici a upstream: "
                f"{confronto['dichiarati_e_identici']}"
            )
        aggiunti = [
            file
            for file in confronto["aggiunti"]
            if file not in voce["delta_dichiarato"]["packaging"]
        ]
        if aggiunti:
            rilievi.append(f"{nome}: file aggiunti rispetto a upstream: {aggiunti}")
        if confronto["mancanti_alla_libreria"]:
            rilievi.append(
                f"{nome}: mancano file che entrerebbero nella compilazione: "
                f"{confronto['mancanti_alla_libreria']}"
            )

    misurati = {
        nome: set(voce["confronto_con_upstream"].get("metadati_non_vendorizzati", []))
        for nome, voce in rapporto["fork"].items()
        if voce["confronto_con_upstream"]["eseguito"]
    }
    if len(misurati) > 1 and len({frozenset(v) for v in misurati.values()}) > 1:
        rapporto["coerenza_dei_metadati"] = {
            "coerente": False,
            "per_fork": {nome: sorted(v) for nome, v in misurati.items()},
            "che_cosa_significa": (
                "i fork non vendorizzano lo stesso insieme di metadati di "
                "packaging. Non e' una differenza di codice -- nessuno di quei "
                "file ne contiene -- ma dice che le tre vendorizzazioni sono "
                "state fatte con procedure leggermente diverse, e vale la pena "
                "saperlo prima che la differenza si allarghi."
            ),
        }
    else:
        rapporto["coerenza_dei_metadati"] = {"coerente": True, "per_fork": {
            nome: sorted(v) for nome, v in misurati.items()
        }}

    rapporto["risoluzione"] = risoluzione()
    for patch in rapporto["risoluzione"]["patch"]:
        rilievi.append(
            f"{patch}: una patch vale solo dal workspace radice, e chi dipende "
            "da noi non la eredita"
        )
    dirette = rapporto["risoluzione"]["dipendenze_dirette"]
    for atteso in FORK:
        voce = dirette.get(atteso)
        if voce is None or voce["package"] != f"plenora-fork-{atteso}":
            rilievi.append(
                f"Cargo.toml: «{atteso}» non e' una dipendenza diretta del fork "
                f"con nome proprio, e chi dipende da noi riceverebbe la versione "
                "di crates.io"
            )
    for nome, voce in dirette.items():
        if not voce["dentro_vendor"]:
            continue
        if nome not in FORK:
            rilievi.append(
                f"Cargo.toml: «{nome}» dipende da {voce['path']!r}, che non e' fra "
                "i fork governati: un fork senza lock, senza registro e senza gate"
            )
        if not voce["esiste"]:
            rilievi.append(
                f"Cargo.toml: «{nome}» dipende da {voce['path']!r}, che non esiste"
            )

    rilievi = licenze + rilievi
    rapporto["rilievi"] = rilievi
    rapporto["rilievi_di_licenza"] = licenze

    for nome, voce in rapporto["fork"].items():
        confronto = voce["confronto_con_upstream"]
        print(f"=== {voce['package']} {voce['version']} ({voce['file']} file)")
        print(f"    impronta col lock: {'coincide' if voce['impronta_coincide'] else 'DIVERSA'}")
        licenza = voce["licenza"]
        stato = "presente" if licenza["file_presente"] else "ASSENTE"
        if licenza.get("identica_a_upstream") is True:
            stato = "presente e identica a upstream"
        elif licenza.get("assente_dal_crate"):
            stato = "presente; il crate non la porta, vale l'impronta registrata"
        elif licenza.get("identica_a_upstream") is False:
            stato = "presente ma DIVERSA da upstream"
        print(
            f"    licenza:           {licenza['dichiarata_nel_registro']} "
            f"({licenza['file']}: {stato})"
        )
        print(
            f"    delta dichiarato:  {len(voce['delta_dichiarato']['funzionale'])} "
            f"funzionali, {len(voce['delta_dichiarato']['packaging'])} di packaging"
        )
        if confronto["eseguito"]:
            print(
                f"    delta reale:       {len(confronto['modificati'])} modificati, "
                f"{len(confronto['aggiunti'])} aggiunti, "
                f"{len(confronto['mancanti_alla_libreria'])} mancanti alla libreria"
                f"  [{confronto['provenienza']}]"
            )
            fuori = confronto["fuori_perimetro_per_categoria"]
            if fuori:
                dettaglio = ", ".join(f"{q} {c}" for c, q in fuori.items())
                print(
                    f"    fuori perimetro:   {confronto['fuori_perimetro']} file "
                    f"({dettaglio}) -- nessuno compilato come dipendenza"
                )
        else:
            print(f"    delta reale:       non misurato ({confronto['perche'][:60]}...)")

    print()
    dirette = rapporto["risoluzione"]["dipendenze_dirette"]
    for nome in sorted(dirette):
        voce = dirette[nome]
        print(f"    {nome}: dipendenza diretta {voce['package']} da {voce['path']}")
    patch = rapporto["risoluzione"]["patch"]
    print(f"    [patch]: {', '.join(patch) if patch else 'nessuna'}")

    print()
    if rilievi:
        print(f"{len(rilievi)} rilievi:")
        for rilievo in rilievi:
            print(f"  - {rilievo}")
    else:
        print("nessun rilievo: i tre alberi sono quelli che i lock dichiarano, e")
        print("il delta reale rispetto a upstream e' quello registrato.")

    if opzioni.rapporto is not None:
        opzioni.rapporto.write_text(
            json.dumps(rapporto, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
        )
        print(f"\nrapporto in {opzioni.rapporto}")

    # L'audit **riporta**, e blocca su una cosa sola: le licenze. Gli altri
    # rilievi hanno il loro gate (i `check_*_fork.py`), e duplicarne il
    # verdetto qui darebbe due voci alla stessa verifica. Le licenze no: nessun
    # gate le verificava, e il fork di gdal ha ridistribuito codice MIT senza il
    # suo testo dalla prima vendorizzazione fino alla 4.1.1, con il registro che
    # dichiarava il contrario. Un rilievo di licenza esce con 1, e la CI lo
    # esegue a ogni push.
    if licenze:
        print(f"\n{len(licenze)} rilievi di licenza: l'audit fallisce.")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
