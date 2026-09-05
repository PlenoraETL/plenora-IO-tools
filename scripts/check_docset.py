#!/usr/bin/env python3
"""Il docset e' minimo, corrente e non e' un database.

# Che cosa questo gate impedisce

Una documentazione che cresce per accumulo smette di essere letta: quando i
documenti sono centoventidue, nessuno sa quale valga ancora. Il rimedio non e'
scriverne di migliori, e' impedire che il numero cresca.

Sette Markdown, e nessun altro. Quattro sono il docset canonico; tre sono file
operativi che una piattaforma o Cargo leggono per convenzione di percorso, e
restano per quello, non come documentazione.

# Le proprieta' verificate

1. l'allowlist e' **esatta**: nessun `.md` tracciato fuori da essa, e nessuno
   dei sette mancante;
2. i due README vendorizzati sono **byte-identici** alla baseline: sono
   contenuto di terzi e input del packaging Cargo, non nostri da riscrivere;
3. nessun gate legge un Markdown come input macchina;
4. ogni collegamento relativo risolve;
5. i tre documenti di `docs/` sono raggiungibili da `README.md`;
6. `docs/RELEASE.md` e' coerente con `assurance/current-state.json`;
7. nessun riferimento residuo alla cronaca eliminata.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

# Il renderer del blocco di stato. Vive separato perche' non legge il
# docset: legge JSON e restituisce testo.
import stato_release  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
STATO = ROOT / "assurance" / "current-state.json"

CANONICI = [
    "README.md",
    "docs/PRODUCT.md",
    "docs/ENGINEERING.md",
    "docs/RELEASE.md",
    # Il quinto ha un destinatario che gli altri quattro non hanno: chi
    # **riceve** il prodotto. PRODUCT dice che cosa promette, ENGINEERING come
    # e' fatto, RELEASE dove siamo -- tutte domande di chi ci lavora. «Che cosa
    # scarico, come verifico che sia arrivato intero, che cosa riscrivo nel mio
    # codice» e' un'altra cosa, e infilarla in uno dei quattro l'avrebbe resa
    # una sezione che chi la cerca non trova.
    "docs/INSTALL.md",
]

# Letti da una piattaforma o da Cargo per convenzione di percorso. Non sono
# documentazione: sono configurazione che si dà il caso sia scritta in Markdown.
OPERATIVI = {
    "vendor/dxf/README.md": "Cargo.toml del fork dichiara `readme`; contenuto di terzi",
    "vendor/gdal/README.md": "Cargo.toml del fork dichiara `readme`; contenuto di terzi",
    # Il terzo fork ridistribuisce anche un CHANGELOG e la propria licenza in
    # Markdown. Non entrano in `vendor_intatti`, e non per dimenticanza:
    # l'integrita' dell'albero `vendor/shapefile` e' verificata per intero --
    # venticinque file, un digest solo -- da `scripts/check_shapefile_fork.py`
    # contro il lock del fork, che e' una difesa piu' forte del confronto di un
    # file con una baseline.
    "vendor/shapefile/README.md": "Cargo.toml del fork dichiara `readme`; contenuto di terzi",
    "vendor/shapefile/CHANGELOG.md": "cronaca upstream ridistribuita; contenuto di terzi",
    "vendor/shapefile/LICENSE.md": "licenza MIT upstream ridistribuita; contenuto di terzi",
    ".github/pull_request_template.md": "GitHub lo legge per convenzione di percorso",
    "sdk/python/README.md": (
        "`pyproject.toml` lo dichiara `readme`, e finisce nei metadati del "
        "pacchetto: e' la stessa convenzione di percorso dei fork Cargo. Non "
        "e' documentazione del prodotto -- quella sta in `docs/` -- ma la "
        "pagina che chi installa l'SDK vede dal proprio gestore di pacchetti."
    ),
}

AMMESSI = set(CANONICI) | set(OPERATIVI)

# I soli file che possono **leggere** un documento.
#
# Il criterio non e' «questi due file», e' che cosa fanno con cio' che leggono:
# lo **validano** — collegamenti, numeri, raggiungibilita' — invece di
# estrarne stato operativo. Un gate che leggesse un numero da un documento per
# usarlo altrove sarebbe la dipendenza che questa regola vieta, e starebbe
# fuori dall'eccezione anche se comparisse qui.
#
# Sono due perche' una sonda che verifica il validatore deve leggere cio' che
# il validatore legge: escluderla costringerebbe a non sondarlo.
VALIDATORI = {
    "scripts/check_docset.py",
    "scripts/test_check_docset.py",
}

# Chi **trascrive** un documento invece di estrarne stato.
#
# E' un'eccezione di natura diversa da quella dei validatori, e sta in un
# insieme suo per non allargare quello: un validatore legge per **verificare**,
# un trascrittore copia il testo dove il testo deve andare. Mescolarli avrebbe
# reso `VALIDATORI` un elenco di «script che possono leggere i documenti», che
# e' esattamente la regola che qui non si vuole.
#
# Il costruttore del pacchetto Python mette `sdk/python/README.md` nel campo
# `Description` dei metadati. Nella convenzione dei pacchetti Python la
# descrizione lunga **e'** il README, e `pyproject.toml` lo dichiara con
# `readme = "README.md"`: un costruttore che lo riscrivesse a mano produrrebbe
# due testi destinati a divergere, e chi installa leggerebbe quello sbagliato.
TRASCRITTORI = {
    "scripts/costruisci-pacchetto-python.py",
}

# La baseline dei README vendorizzati: sono ridistribuiti, non nostri.
BASELINE_VENDOR = "2fe9b54"

# I nomi della cronaca eliminata, **derivati** dalla baseline invece che
# scritti a mano.
#
# Un elenco compilato a mano e' incompleto per costruzione: la prima stesura
# dimenticava `ROADMAP-1.1.0.md`, `Prestazioni.md` e `Architetture.md`, e quei
# riferimenti restavano vivi nel codice senza che nulla li vedesse. Qui i nomi
# vengono da cio' che la baseline conteneva e il docset non contiene piu': se
# un documento sparisce, il suo nome entra nell'elenco da solo.
BASELINE_DOCSET = "2fe9b54"


def _nomi_eliminati() -> list[str]:
    uscita = subprocess.run(
        ["git", "ls-files", "--with-tree", BASELINE_DOCSET, "*.md"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    )
    spariti = [r for r in uscita.stdout.splitlines() if r and r not in AMMESSI]
    # Un nome base e' al bando solo se **nessun** documento ammesso lo porta
    # ancora. `README.md` era sparito da una sottodirectory ed e' vivo alla
    # radice, in `vendor/dxf/` e in `sdk/python/`: metterlo al bando rendeva
    # rosso ogni `readme = "README.md"` fuori da `vendor/`, che e' il modo in
    # cui un manifesto Python o Cargo dichiara il proprio.
    #
    # E' lo stesso riguardo che le cartelle avevano gia' -- contano solo se non
    # esistono piu' -- e che ai nomi non era stato dato: l'elenco confronta
    # nomi base con percorsi ammessi, e i due non si incontrano mai.
    vivi = {Path(r).name for r in AMMESSI}
    nomi = {Path(r).name for r in spariti} - vivi
    # Le directory contano solo se **non esistono piu'**: `docs/` e
    # `vendor/dxf/` contenevano documenti eliminati ma sono vive, e vietarne il
    # nome renderebbe rosso ogni riferimento legittimo. La prima stesura lo
    # faceva.
    cartelle = {
        str(Path(r).parent) + "/"
        for r in spariti
        if Path(r).parent != Path(".") and not (ROOT / Path(r).parent).exists()
    }
    return sorted(re.escape(n) for n in nomi | cartelle)


# Dove cercare i riferimenti residui: codice, script, CI e il docset stesso.
# I JSON sono nel perimetro: un manifesto che cita un documento eliminato lo
# cita quanto un commento, e nessuno lo rileggera' per accorgersene.
#
# Accenderlo ha richiesto prima di risolvere i manifesti storici orfani sotto
# `release/`: quattordici file che nessuno leggeva piu', piu' la candidate
# `1.0.1`. Il loro fatto vivo — una candidate pendente legata a uno SHA vecchio
# e non autorizzata — e' nel registro del contratto corrente; la provenienza
# resta in git.
PERIMETRO = ("*.rs", "*.py", "*.sh", "*.toml", "*.yml", "*.yaml", "*.md", "*.json")


def _elenco(argomenti: list[str]) -> list[str]:
    uscita = subprocess.run(
        ["git", "ls-files", *argomenti],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    )
    return [r for r in uscita.stdout.splitlines() if r]


def nel_perimetro(estensione: str) -> list[str]:
    """I file tracciati **e** gli untracked non ignorati.

    Il gate misurava i soli tracciati, e quella scelta apriva una finestra
    esattamente dove serviva chiuderla: un nuovo lettore di Markdown, o un
    nuovo documento, e' untracked fino al primo `git add`. Chi lo scrive lancia
    il livello 1 prima di committare, e sarebbe stato verde proprio nel momento
    in cui l'errore era presente sul disco.

    Gli ignorati restano fuori: `target/`, gli artefatti di build e le copie di
    lavoro non sono materiale del repository, e includerli renderebbe il gate
    dipendente da cio' che c'e' sulla macchina di chi lo lancia.
    """
    visti = _elenco([estensione]) + _elenco(
        ["--others", "--exclude-standard", estensione]
    )
    fuori: list[str] = []
    for percorso in visti:
        if percorso not in fuori:
            fuori.append(percorso)
    return sorted(fuori)


def tracciati(estensione: str) -> list[str]:
    """I soli tracciati. Serve dove la domanda e' «che cosa e' committato»."""
    return _elenco([estensione])


def allowlist() -> list[str]:
    errori: list[str] = []
    presenti = set(nel_perimetro("*.md"))

    for extra in sorted(presenti - AMMESSI):
        errori.append(
            f"{extra}: Markdown tracciato fuori dall'allowlist. Il docset e' "
            "minimo per scelta: un documento in piu' e' un documento che "
            "nessuno rileggera'."
        )
    for mancante in sorted(AMMESSI - presenti):
        errori.append(f"{mancante}: documento dell'allowlist assente")
    return errori


def vendor_intatti() -> list[str]:
    """I README vendorizzati sono byte-identici alla baseline."""
    errori: list[str] = []
    for relativo in ("vendor/dxf/README.md", "vendor/gdal/README.md"):
        atteso = subprocess.run(
            ["git", "show", f"{BASELINE_VENDOR}:{relativo}"],
            cwd=ROOT,
            capture_output=True,
            check=False,
        )
        if atteso.returncode != 0:
            errori.append(f"{relativo}: baseline {BASELINE_VENDOR} non leggibile")
            continue
        corrente = (ROOT / relativo).read_bytes()
        if corrente != atteso.stdout:
            errori.append(
                f"{relativo}: differisce dalla baseline {BASELINE_VENDOR}. E' "
                "contenuto di terzi e input del packaging Cargo: riscriverlo "
                "altera materiale ridistribuito."
            )
    return errori


def nessun_markdown_come_database() -> list[str]:
    """Nessun gate legge un Markdown come input macchina."""
    errori: list[str] = []
    lettura = re.compile(r"""read_text|read_bytes|open\(|load\(""")
    for percorso in nel_perimetro("scripts/*.py") + nel_perimetro("scripts/*.sh"):
        if percorso in VALIDATORI | TRASCRITTORI:
            continue
        testo = (ROOT / percorso).read_text(encoding="utf-8", errors="replace")
        for numero, riga in enumerate(testo.splitlines(), 1):
            nuda = riga.split("#", 1)[0]
            if ".md" not in nuda or not lettura.search(nuda):
                continue
            if any(operativo in nuda for operativo in OPERATIVI):
                continue
            errori.append(
                f"{percorso}:{numero}: legge un Markdown come input macchina. "
                "Un gate che dipende dalla prosa dipende da qualcosa che "
                "nessuno puo' validare e che si riscrive senza accorgersene."
            )
    return errori


def collegamenti() -> list[str]:
    """Ogni collegamento relativo del docset risolve."""
    errori: list[str] = []
    ancora = re.compile(r"\[[^\]]*\]\(([^)#]+)(#[^)]*)?\)")
    for relativo in CANONICI:
        sorgente = ROOT / relativo
        for bersaglio, _ in ancora.findall(sorgente.read_text(encoding="utf-8")):
            if bersaglio.startswith(("http://", "https://", "mailto:")):
                continue
            risolto = (sorgente.parent / bersaglio).resolve()
            if not risolto.exists():
                errori.append(f"{relativo}: collegamento rotto verso «{bersaglio}»")
    return errori


def raggiungibili() -> list[str]:
    """I documenti di `docs/` sono raggiungibili da `README.md`."""
    testo = (ROOT / "README.md").read_text(encoding="utf-8")
    return [
        f"{relativo}: non e' raggiungibile da README.md"
        for relativo in CANONICI
        if relativo != "README.md" and relativo not in testo
    ]


def stato_coerente() -> list[str]:
    """Il blocco di stato di `docs/RELEASE.md` e' quello reso dalla fonte.

    Prima il gate cercava ogni numero della fonte **come sottostringa** nel
    documento. Coglieva un numero cambiato in un solo posto, e nient'altro: un
    campo dichiarato dalla fonte e mai nominato passava, perche' il gate non
    sapeva quali campi pretendere, e un numero giusto sotto l'etichetta
    sbagliata passava, perche' cercava la cifra e non la coppia.

    Ora il documento riceve il blocco invece di riportarlo, e il confronto e'
    carattere per carattere fra i due marcatori.

    Il valore di `release_authorized` non viene fissato qui. E' una decisione
    scritta, e la condizione che la usa vive dove le condizioni di
    autorizzazione sono congiunte: `check_release_contract.py --release`. Un
    perno in questo gate diventerebbe rosso il giorno in cui l'autorizzazione
    fosse legittima, e verrebbe tolto in fretta.
    """
    if not STATO.exists():
        return [f"{STATO}: fonte strutturata dello stato assente"]
    atteso, errori = stato_release.blocco(
        json.loads(STATO.read_text(encoding="utf-8")),
        json.loads(stato_release.REGISTRO.read_text(encoding="utf-8")),
    )
    if errori:
        return errori

    testo = (ROOT / "docs" / "RELEASE.md").read_text(encoding="utf-8")
    for marcatore in (stato_release.APERTURA, stato_release.CHIUSURA):
        if testo.count(marcatore) != 1:
            return [
                f"docs/RELEASE.md: il marcatore «{marcatore}» compare "
                f"{testo.count(marcatore)} volte, e ne serve esattamente una."
            ]
    if testo.index(stato_release.APERTURA) > testo.index(stato_release.CHIUSURA):
        return ["docs/RELEASE.md: i marcatori del blocco generato sono invertiti"]

    principio = testo.index(stato_release.APERTURA)
    conclusione = testo.index(stato_release.CHIUSURA) + len(stato_release.CHIUSURA)
    corrente = testo[principio:conclusione]
    if corrente != atteso:
        return [
            "docs/RELEASE.md: il blocco generato non coincide con la fonte "
            "strutturata. Si rigenera con `python3 scripts/check_docset.py "
            "--riscrivi-stato`; scriverlo a mano crea la seconda verita' che "
            "quel blocco esiste per impedire."
        ]
    return []


def runbook_operativo() -> list[str]:
    """Il runbook copre l'intero ciclo, non soltanto la costruzione.

    Il criterio di uscita nomina installazione, aggiornamento, rollback e
    recovery. Queste parole da sole non bastano: la procedura deve anche
    verificare prima di attivare e vietare l'estrazione sopra una versione
    esistente.
    """
    percorso = ROOT / "docs" / "RELEASE.md"
    testo = percorso.read_text(encoding="utf-8")
    apertura = "#### Installazione, aggiornamento, rollback e recovery"
    chiusura = "### 6. Decisione finale di rilascio"
    if apertura not in testo or chiusura not in testo:
        return ["docs/RELEASE.md: sezione operativa del runbook assente"]
    inizio = testo.index(apertura)
    fine = testo.index(chiusura, inizio)
    runbook = testo[inizio:fine].lower()
    errori: list[str] = []
    richiesti = (
        "**installazione.**",
        "**attivazione e aggiornamento.**",
        "**rollback.**",
        "**recovery dei dati.**",
        "check-digest-manifesto.py",
        "affiancate",
        "non estrarre mai sopra",
    )
    for richiesto in richiesti:
        if richiesto not in runbook:
            errori.append(f"docs/RELEASE.md: runbook senza «{richiesto}»")
    if "verificare il checksum" in runbook and "rinominare la directory" in runbook:
        if runbook.index("verificare il checksum") > runbook.index("rinominare la directory"):
            errori.append("docs/RELEASE.md: il runbook attiva prima di verificare")
    return errori


def riscrivi_stato() -> int:
    """Propaga la fonte strutturata nel documento.

    La scrittura sta qui e non in `stato_release` per non allargare la
    allowlist chiusa dei validatori: questo file e la sua sonda sono gia' i due
    ammessi a leggere il docset, e un terzo modulo che apre `RELEASE.md`
    andrebbe aggiunto a quella lista con l'unica ragione di avere una funzione
    in piu'.

    L'autorita' resta la fonte. Che il gate sappia anche riscrivere non e' un
    modo di rendersi verde: rende verde il documento **facendolo coincidere con
    il JSON**, che e' esattamente cio' che il controllo pretende.
    """
    atteso, errori = stato_release.blocco(
        json.loads(STATO.read_text(encoding="utf-8")),
        json.loads(stato_release.REGISTRO.read_text(encoding="utf-8")),
    )
    if errori:
        for messaggio in errori:
            print(messaggio, file=sys.stderr)
        return 1

    percorso = ROOT / "docs" / "RELEASE.md"
    testo = percorso.read_text(encoding="utf-8")
    if testo.count(stato_release.APERTURA) != 1 or testo.count(stato_release.CHIUSURA) != 1:
        print("docs/RELEASE.md: marcatori del blocco generato assenti o ripetuti", file=sys.stderr)
        return 1
    principio = testo.index(stato_release.APERTURA)
    conclusione = testo.index(stato_release.CHIUSURA) + len(stato_release.CHIUSURA)
    percorso.write_text(testo[:principio] + atteso + testo[conclusione:], encoding="utf-8", newline="\n")
    print("docs/RELEASE.md: blocco di stato rigenerato dalla fonte strutturata")
    return 0


def cronaca_residua() -> list[str]:
    """Nessun riferimento a cio' che e' stato eliminato."""
    errori: list[str] = []
    schemi = [re.compile(s) for s in _nomi_eliminati()]
    for modello in PERIMETRO:
        for percorso in nel_perimetro(modello) + nel_perimetro(f"*/{modello}"):
            if percorso.startswith("vendor/") or percorso in VALIDATORI | TRASCRITTORI:
                continue
            testo = (ROOT / percorso).read_text(encoding="utf-8", errors="replace")
            for schema in schemi:
                trovato = schema.search(testo)
                if trovato:
                    errori.append(
                        f"{percorso}: riferisce «{trovato.group(0)}», che non "
                        "esiste piu'. Un collegamento al nulla promette un "
                        "approfondimento che non c'e'."
                    )
    return sorted(set(errori))


CONTROLLI = (
    ("allowlist", allowlist),
    ("README vendorizzati intatti", vendor_intatti),
    ("nessun Markdown come database", nessun_markdown_come_database),
    ("collegamenti relativi", collegamenti),
    ("raggiungibilita' da README", raggiungibili),
    ("RELEASE.md coerente con lo stato", stato_coerente),
    ("runbook operativo completo", runbook_operativo),
    ("nessuna cronaca residua", cronaca_residua),
)


def main(argv: list[str] | None = None) -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument(
        "--riscrivi-stato",
        action="store_true",
        help="propaga assurance/current-state.json nel blocco generato di docs/RELEASE.md",
    )
    opzioni = argomenti.parse_args(argv)
    if opzioni.riscrivi_stato:
        return riscrivi_stato()

    errori: list[str] = []
    for nome, controllo in CONTROLLI:
        trovati = controllo()
        if trovati:
            print(f"--- {nome}", file=sys.stderr)
            for messaggio in trovati:
                print(f"    {messaggio}", file=sys.stderr)
        errori.extend(trovati)

    if errori:
        return 1
    print(
        f"docset verificato: {len(CANONICI)} documenti canonici, "
        f"{len(OPERATIVI)} file operativi, nessun altro Markdown nel perimetro "
        "(tracciati piu' untracked non ignorati); blocco di stato coerente con "
        "la fonte strutturata."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
