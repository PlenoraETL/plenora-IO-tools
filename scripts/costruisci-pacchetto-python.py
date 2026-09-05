#!/usr/bin/env python3
"""Costruisce la wheel e la sdist dell'SDK Python, e i documenti che le legano.

# Perche' a mano, e non con `build`

Perche' i byte devono essere **riproducibili**. Un archivio costruito da
setuptools porta le date di modifica dei file e l'ordine in cui il filesystem
li ha restituiti: due costruzioni dallo stesso albero danno due file diversi, e
un checksum che cambia senza che cambi il contenuto non lega niente.

Qui l'ordine e' quello dei nomi e le date sono fisse. Due costruzioni dallo
stesso albero danno gli **stessi byte**, e questo rende il checksum una misura
invece di un numero.

Il secondo motivo e' che costruire non richiede piu' niente: nessun `build`,
nessuna rete, nessun ambiente isolato da preparare. Una wheel e' uno zip con
tre file di metadati, e la sdist un tar con un `PKG-INFO`. Scriverli qui e'
meno codice di quanto ne servirebbe a gestire le dipendenze di chi li scrive
per noi.

Chi installa **dalla sdist** usa invece setuptools, che il `pyproject.toml`
dichiara: quella strada e' la strada di tutti, e va provata com'e'.

# Che cosa questa wheel non contiene

Nessun binario `plenora-io`. E' un pacchetto `py3-none-any`, e resta puro per
una ragione che non e' di comodo: incorporare l'eseguibile vorrebbe dire una
wheel per piattaforma, il triplo degli artefatti da qualificare, e un binario
che si aggiorna solo cambiando il pacchetto Python. L'SDK trova un binario che
esiste gia' -- e se non c'e', lo dice invece di scaricarlo.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import io
import json
import pathlib
import re
import sys
import tarfile
import zipfile

RADICE = pathlib.Path(__file__).resolve().parent.parent
SDK = RADICE / "sdk" / "python"
SORGENTI = SDK / "src" / "plenora_io"

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import distribuzione  # noqa: E402 -- dopo sys.path, che e' il punto

#: Il nome del pacchetto sul wire dei metadati, e quello del modulo.
NOME = "plenora-io"
MODULO = "plenora_io"

#: La data che finisce in ogni voce degli archivi.
#:
#: Un valore fisso e non «adesso»: e' cio' che rende due costruzioni dallo
#: stesso albero identiche byte per byte. Il 1980 e' il minimo che il formato
#: ZIP sa rappresentare, ed e' la scelta convenzionale per dire «questa data non
#: significa niente» invece di far credere che significhi qualcosa.
DATA_FISSA = (1980, 1, 1, 0, 0, 0)


def versione() -> str:
    """La versione, dalla **sola** sorgente autorevole.

    Si legge dal sorgente e non importando il modulo: un costruttore che
    importasse cio' che sta impacchettando eseguirebbe il pacchetto per
    scoprire come si chiama.
    """
    testo = (SORGENTI / "__init__.py").read_text(encoding="utf-8")
    trovato = re.search(r'^__version__ = "([^"]+)"$', testo, re.M)
    if trovato is None:
        raise SystemExit(
            "`__version__` non si trova in `plenora_io/__init__.py`: e' la sola "
            "sorgente autorevole della versione, e senza non c'e' niente da "
            "impacchettare."
        )
    return trovato.group(1)


def requires_python() -> str:
    """`requires-python` dal `pyproject.toml`, senza interpretare il TOML.

    Il campo e' una riga sola e la si legge come tale: importare un parser TOML
    per un valore che il file dichiara in chiaro aggiungerebbe una dipendenza a
    un costruttore che non ne ha nessuna.
    """
    testo = (SDK / "pyproject.toml").read_text(encoding="utf-8")
    trovato = re.search(r'^requires-python = "([^"]+)"$', testo, re.M)
    if trovato is None:
        raise SystemExit("`requires-python` non si trova in pyproject.toml")
    return trovato.group(1)


def classificatori() -> list[str]:
    testo = (SDK / "pyproject.toml").read_text(encoding="utf-8")
    blocco = re.search(r"^classifiers = \[(.*?)^\]", testo, re.M | re.S)
    if blocco is None:
        return []
    return re.findall(r'^\s*"([^"]+)",', blocco.group(1), re.M)


def sorgenti_del_modulo() -> list[pathlib.Path]:
    """I file del pacchetto, in ordine di nome.

    `__pycache__` resta fuori: e' un artefatto dell'interprete che ha eseguito
    i sorgenti, non contenuto del pacchetto, e includerlo legherebbe la wheel
    alla versione di Python che l'ha costruita.
    """
    return sorted(
        percorso
        for percorso in SORGENTI.rglob("*")
        if percorso.is_file() and "__pycache__" not in percorso.parts
    )


def metadata(versione_pacchetto: str) -> str:
    """Il `METADATA` della wheel: quel che `pip show` mostrera'."""
    descrizione = (SDK / "README.md").read_text(encoding="utf-8")
    righe = [
        "Metadata-Version: 2.1",
        f"Name: {NOME}",
        f"Version: {versione_pacchetto}",
        "Summary: Wrapper Python puro sopra la CLI plenora-io e il suo protocollo v2",
        "Project-URL: Repository, https://github.com/PlenoraETL/plenora-IO-tools",
        f"Requires-Python: {requires_python()}",
        "Description-Content-Type: text/markdown",
    ]
    righe += [f"Classifier: {c}" for c in classificatori()]
    return "\n".join(righe) + "\n\n" + descrizione


def wheel_metadata() -> str:
    return (
        "Wheel-Version: 1.0\n"
        "Generator: plenora-costruisci-pacchetto-python\n"
        "Root-Is-Purelib: true\n"
        "Tag: py3-none-any\n"
    )


def _digest_record(dati: bytes) -> str:
    grezzo = hashlib.sha256(dati).digest()
    return "sha256=" + base64.urlsafe_b64encode(grezzo).decode("ascii").rstrip("=")


def costruisci_wheel(uscita: pathlib.Path, versione_pacchetto: str) -> pathlib.Path:
    """La wheel `py3-none-any`, deterministica."""
    dist_info = f"{MODULO}-{versione_pacchetto}.dist-info"
    voci: list[tuple[str, bytes]] = []

    for percorso in sorgenti_del_modulo():
        nome = f"{MODULO}/{percorso.relative_to(SORGENTI).as_posix()}"
        voci.append((nome, percorso.read_bytes()))

    voci.append((f"{dist_info}/METADATA", metadata(versione_pacchetto).encode("utf-8")))
    voci.append((f"{dist_info}/WHEEL", wheel_metadata().encode("utf-8")))
    voci.sort()

    record = "".join(
        f"{nome},{_digest_record(dati)},{len(dati)}\n" for nome, dati in voci
    )
    # La riga di `RECORD` che descrive `RECORD` non porta ne' digest ne'
    # dimensione: non puo', perche' cambierebbe se stessa.
    record += f"{dist_info}/RECORD,,\n"
    voci.append((f"{dist_info}/RECORD", record.encode("utf-8")))

    percorso = uscita / f"{MODULO}-{versione_pacchetto}-py3-none-any.whl"
    with zipfile.ZipFile(percorso, "w", zipfile.ZIP_DEFLATED) as archivio:
        for nome, dati in voci:
            info = zipfile.ZipInfo(nome, date_time=DATA_FISSA)
            info.external_attr = 0o644 << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            archivio.writestr(info, dati)
    return percorso


def file_della_sdist() -> list[tuple[str, pathlib.Path]]:
    """Che cosa entra nella sdist, e con quale nome dentro l'archivio.

    Ci sono anche i **test**: una sdist e' la forma in cui si ricostruisce il
    pacchetto, e ricostruirlo senza poterlo provare vorrebbe dire consegnare a
    chi la usa meno di quanto serve per fidarsi.
    """
    dentro: list[tuple[str, pathlib.Path]] = [
        ("pyproject.toml", SDK / "pyproject.toml"),
        ("README.md", SDK / "README.md"),
    ]
    for percorso in sorgenti_del_modulo():
        dentro.append(
            (f"src/{MODULO}/{percorso.relative_to(SORGENTI).as_posix()}", percorso)
        )
    # Tutti i  di , non i soli : gli helper che le sonde
    # importano --  -- non hanno quel prefisso, e una sdist che
    # portasse le sonde senza cio' che importano consegnerebbe test che non
    # partono.
    for percorso in sorted((SDK / "tests").glob("*.py")):
        dentro.append((f"tests/{percorso.name}", percorso))
    return dentro


def costruisci_sdist(uscita: pathlib.Path, versione_pacchetto: str) -> pathlib.Path:
    """La sdist `.tar.gz`, deterministica."""
    radice = f"{MODULO}-{versione_pacchetto}"
    percorso = uscita / f"{radice}.tar.gz"

    voci: list[tuple[str, bytes]] = [
        (f"{radice}/PKG-INFO", metadata(versione_pacchetto).encode("utf-8"))
    ]
    for nome, sorgente in file_della_sdist():
        voci.append((f"{radice}/{nome}", sorgente.read_bytes()))
    voci.sort()

    # `mtime=0` nel gzip **e** nelle voci del tar: l'intestazione gzip porta una
    # data propria, e lasciarla al valore predefinito renderebbe l'archivio
    # diverso a ogni costruzione anche con un tar identico dentro.
    import gzip

    grezzo = io.BytesIO()
    with tarfile.open(fileobj=grezzo, mode="w") as archivio:
        for nome, dati in voci:
            info = tarfile.TarInfo(nome)
            info.size = len(dati)
            info.mtime = 0
            info.mode = 0o644
            info.uid = info.gid = 0
            info.uname = info.gname = ""
            archivio.addfile(info, io.BytesIO(dati))

    with open(percorso, "wb") as file:
        with gzip.GzipFile(fileobj=file, mode="wb", mtime=0) as compresso:
            compresso.write(grezzo.getvalue())
    return percorso



# --- i documenti che accompagnano i due artefatti ---------------------------
#
# Sono gli stessi che i costruttori nativi scrivono, con i contenuti che a un
# pacchetto Python puro competono. Un artefatto distribuito senza di loro non e'
# qualificabile: il checksum dice che i byte sono quelli, il manifesto che cosa
# sono, la provenance da dove vengono e l'SBOM che cosa contengono di terzi.


def sbom(versione_pacchetto: str, artefatti: list[pathlib.Path]) -> dict:
    """L'SBOM del pacchetto: **zero** componenti di terzi, e lo dice.

    `distribuzione.py` avverte che un SBOM vuoto e' peggio di nessun SBOM,
    perche' afferma che l'artefatto non contiene niente di terzi -- e non e'
    vero di nessun artefatto che spedisca un runtime.

    Qui e' vero, ed e' la ragione per cui il documento non si limita a un elenco
    vuoto: dichiara **perche'** sia vuoto, e da quale fatto verificabile la
    dichiarazione segua. `dependencies = []` nel `pyproject.toml` e' quel fatto,
    e lo smoke lo ricontrolla sul pacchetto installato.
    """
    return {
        "schema_sbom": 1,
        "artefatto": f"{NOME} {versione_pacchetto}",
        "componenti": [],
        "componenti_di_terzi": 0,
        "perche_vuoto": (
            "il pacchetto non ha dipendenze e non spedisce byte di terzi: usa "
            "`subprocess`, `json` e `pathlib`, che stanno nella libreria "
            "standard. Non e' un elenco che nessuno ha compilato -- e' un "
            "elenco che non ha voci, e il fatto da cui segue e' "
            "`dependencies = []` nel `pyproject.toml`, che lo smoke ricontrolla "
            "sul pacchetto **installato**."
        ),
        "che_cosa_non_contiene": (
            "il binario `plenora-io`. La wheel e' `py3-none-any` e resta pura: "
            "il prodotto si installa a parte, e l'SDK lo trova o dice dove ha "
            "cercato."
        ),
        "file": [
            {"nome": a.name, "sha256": distribuzione.sha256(a), "byte": a.stat().st_size}
            for a in artefatti
        ],
    }


def licenze(versione_pacchetto: str) -> dict:
    """Le licenze dei componenti spediti: nessuno, e la propria che manca ancora.

    Il primo numero e' zero perche' non si spedisce niente di terzi, e zero e'
    una misura come le altre.

    Il secondo fatto e' che una licenza first-party **non e' dichiarata**, e per
    questa distribuzione non serve: il pacchetto va a clienti autorizzati per un
    canale riservato, dove i termini stanno nel contratto con loro. Non e' un
    blocco e non e' una verifica: e' un perimetro, e il contratto di release lo
    registra come tale.

    Il dato viene da `distribuzione.LICENZA_FIRST_PARTY`, che e' l'unica fonte:
    scritto qui e nel referto nativo, sarebbe divergiuto il giorno in cui lo
    stato cambia e qualcuno aggiorna un posto solo.
    """
    return {
        "schema_licenze": 1,
        "artefatto": f"{NOME} {versione_pacchetto}",
        "componenti_con_testo": 0,
        "perche_zero": (
            "il pacchetto non spedisce byte di terzi, quindi non c'e' nessuna "
            "licenza altrui da consegnare."
        ),
        "licenza_first_party": distribuzione.licenza_first_party(),
        "conseguenza": (
            "il pacchetto non si pubblica su un indice: la distribuzione "
            "avviene per canale riservato a clienti autorizzati. Il "
            "classificatore `Private :: Do Not Upload` lo dichiara nei "
            "metadati, i servizi che lo leggono rifiutano il caricamento, e "
            "`scripts/check_canale_privato.py` verifica che nessun workflow "
            "provi a farlo comunque."
        ),
    }


def provenance(versione_pacchetto: str, artefatti: list[pathlib.Path]) -> dict:
    """Da quale revisione vengono questi byte, e da quali sorgenti.

    `lock_sha256` non e' un lockfile Cargo: il pacchetto non ne ha uno, perche'
    non ha dipendenze. E' il digest dell'**albero dei sorgenti impacchettati**,
    che risponde alla stessa domanda -- «con quale contenuto e' stato costruito»
    -- con la sola cosa che qui la determina.
    """
    accumulatore = hashlib.sha256()
    for percorso_sorgente in sorgenti_del_modulo():
        accumulatore.update(percorso_sorgente.relative_to(SORGENTI).as_posix().encode())
        accumulatore.update(b"\0")
        accumulatore.update(hashlib.sha256(percorso_sorgente.read_bytes()).hexdigest().encode())
        accumulatore.update(b"\n")
    return {
        "schema_provenance": 1,
        "artefatto": f"{NOME} {versione_pacchetto}",
        "revisione": distribuzione.revisione_del_repository(),
        "lock_sha256": accumulatore.hexdigest(),
        "che_cosa_e_il_lock": (
            "il digest dell'albero dei sorgenti impacchettati, non un lockfile: "
            "il pacchetto non ha dipendenze, e la domanda a cui il campo risponde "
            "-- con quale contenuto e' stato costruito -- qui la determina solo "
            "il sorgente."
        ),
        "file": [
            {"nome": a.name, "archivio_sha256": distribuzione.sha256(a)}
            for a in artefatti
        ],
    }


def manifesto(versione_pacchetto: str, canale: str, artefatti: list[pathlib.Path]) -> dict:
    """Il manifesto dei due artefatti, nella forma dei costruttori nativi.

    Non passa da `distribuzione.manifesto`: quella funzione pretende i quattordici
    campi comuni ai due costruttori **nativi**, fra cui `runtime_nativo` e
    `prefisso_di_costruzione`, che qui non significano niente. Ripeterli vuoti
    sarebbe un documento che finge di descrivere un artefatto che non e'.
    """
    return {
        "schema_manifesto_python": 1,
        "nome": NOME,
        "versione": versione_pacchetto,
        "classe": "python-puro",
        "piattaforma": "any",
        "canale": canale,
        "canale_nota": distribuzione.nota_del_canale(canale),
        "non_release": canale != "candidate",
        "revisione": distribuzione.revisione_del_repository(),
        "requires_python": requires_python(),
        "tag_wheel": "py3-none-any",
        "runtime_nativo": {
            "presente": False,
            "perche": (
                "e' un pacchetto Python puro: nessun modulo compilato, nessuna "
                "libreria, e nessun binario `plenora-io`. Il campo esiste perche' "
                "la domanda va posta a ogni artefatto, e la risposta va misurata "
                "invece che dedotta dal formato."
            ),
        },
        "file": [
            {
                "nome": a.name,
                "sha256": distribuzione.sha256(a),
                "byte": a.stat().st_size,
            }
            for a in artefatti
        ],
    }


def main(argv: list[str] | None = None) -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument("--uscita", required=True, type=pathlib.Path)
    argomenti.add_argument("--canale", default="prova", choices=sorted(distribuzione.NOTA_DEL_CANALE))
    argomenti.add_argument("--referti", type=pathlib.Path, default=None)
    opzioni = argomenti.parse_args(argv)

    opzioni.uscita.mkdir(parents=True, exist_ok=True)
    versione_pacchetto = versione()

    wheel = costruisci_wheel(opzioni.uscita, versione_pacchetto)
    sdist = costruisci_sdist(opzioni.uscita, versione_pacchetto)

    artefatti = [wheel, sdist]
    for artefatto in artefatti:
        digest = hashlib.sha256(artefatto.read_bytes()).hexdigest()
        artefatto.with_suffix(artefatto.suffix + ".sha256").write_text(
            f"{digest}  {artefatto.name}\n", encoding="utf-8"
        )
        print(f"{artefatto.name}: {artefatto.stat().st_size} byte, sha256={digest}")

    documenti = {
        "MANIFEST.json": manifesto(versione_pacchetto, opzioni.canale, artefatti),
        "sbom.json": sbom(versione_pacchetto, artefatti),
        "licenze.json": licenze(versione_pacchetto),
        "provenance.json": provenance(versione_pacchetto, artefatti),
    }
    for nome, documento in documenti.items():
        (opzioni.uscita / nome).write_text(
            json.dumps(documento, ensure_ascii=False, indent=2) + "\n",
            encoding="utf-8",
        )
    print(f"documenti: {', '.join(documenti)}")

    # I referti che il gate finale riconta. `licenze-artefatto` e `provenance`
    # si scrivono qui perche' i loro numeri li conosce chi costruisce; lo smoke
    # scrive il proprio dove l'artefatto viene **installato**, che e' l'unico
    # posto dove quella misura si puo' prendere.
    if opzioni.referti is not None:
        for artefatto, variante in ((wheel, "wheel"), (sdist, "sdist")):
            distribuzione.scrivi_referto(
                opzioni.referti / f"python-{variante}-licenze.json",
                verifica="licenze-artefatto",
                piattaforma="any",
                profilo=variante,
                canale=opzioni.canale,
                esito="verde",
                misure={
                    "componenti_con_testo": 0,
                    "licenza_first_party": distribuzione.licenza_first_party(),
                },
                errori=[],
                note=(
                    "zero e' una misura: il pacchetto non spedisce byte di terzi. "
                    "Che il repository non dichiari una licenza propria sta in "
                    "`licenze.json`, ed e' un'altra cosa."
                ),
            )
            distribuzione.scrivi_referto(
                opzioni.referti / f"python-{variante}-provenance.json",
                verifica="provenance",
                piattaforma="any",
                profilo=variante,
                canale=opzioni.canale,
                esito="verde",
                misure={
                    "archivio_sha256": distribuzione.sha256(artefatto),
                    "revisione": distribuzione.revisione_del_repository(),
                    "lock_sha256": provenance(versione_pacchetto, artefatti)["lock_sha256"],
                },
                errori=[],
            )
            distribuzione.scrivi_referto(
                opzioni.referti / f"python-{variante}-sbom.json",
                verifica="sbom",
                piattaforma="any",
                profilo=variante,
                canale=opzioni.canale,
                esito="verde",
                misure={"componenti_di_terzi": 0},
                errori=[],
                note="il pacchetto non ha dipendenze: l'elenco non ha voci.",
            )
        print(f"referti in {opzioni.referti}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
