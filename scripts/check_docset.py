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

ROOT = Path(__file__).resolve().parent.parent
STATO = ROOT / "assurance" / "current-state.json"

CANONICI = [
    "README.md",
    "docs/PRODUCT.md",
    "docs/ENGINEERING.md",
    "docs/RELEASE.md",
]

# Letti da una piattaforma o da Cargo per convenzione di percorso. Non sono
# documentazione: sono configurazione che si dà il caso sia scritta in Markdown.
OPERATIVI = {
    "vendor/dxf/README.md": "Cargo.toml del fork dichiara `readme`; contenuto di terzi",
    "vendor/gdal/README.md": "Cargo.toml del fork dichiara `readme`; contenuto di terzi",
    ".github/pull_request_template.md": "GitHub lo legge per convenzione di percorso",
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
    nomi = {Path(r).name for r in spariti}
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


def tracciati(estensione: str) -> list[str]:
    uscita = subprocess.run(
        ["git", "ls-files", estensione],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    )
    return [r for r in uscita.stdout.splitlines() if r]


def allowlist() -> list[str]:
    errori: list[str] = []
    presenti = set(tracciati("*.md"))

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
    for percorso in tracciati("scripts/*.py") + tracciati("scripts/*.sh"):
        if percorso in VALIDATORI:
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


def _numeri(stato: dict) -> dict[str, str]:
    misura = stato["ultima_misura"]
    copertura = misura["copertura"]
    return {
        "baseline documentale": stato["revisioni"]["baseline_documentale"]["sha"],
        "ultima qualificata": stato["revisioni"]["ultima_qualificata"]["sha"],
        "passi del checkpoint": str(misura["checkpoint"]["passi_eseguiti"]),
        "input di replay": f"{misura['fuzz']['replay_input']:n}".replace(",", " "),
        "target di replay": str(misura["fuzz"]["replay_target"]),
        "target di smoke": str(misura["fuzz"]["smoke_target_eseguiti"]),
        "copertura LCOV": f"{copertura['lcov_percentuale']:.2f}".replace(".", ","),
        "copertura cargo": f"{copertura['cargo_lines_percentuale']:.2f}".replace(".", ","),
        "gruppi N1 aperti": str(stato["aperto"]["assurance_n1"]["gruppi_aperti"]),
    }


def stato_coerente() -> list[str]:
    """`docs/RELEASE.md` riporta i numeri di `assurance/current-state.json`."""
    if not STATO.exists():
        return [f"{STATO}: fonte strutturata dello stato assente"]
    stato = json.loads(STATO.read_text(encoding="utf-8"))
    testo = (ROOT / "docs" / "RELEASE.md").read_text(encoding="utf-8")
    # I separatori di migliaia non cambiano il numero: «35 562» e 35562
    # sono lo stesso valore, e un gate che li distinguesse costringerebbe a
    # scrivere male il documento per farlo passare.
    compatto = testo.replace(" ", "").replace(" ", "").replace(" ", "")

    errori = [
        f"docs/RELEASE.md: «{nome}» vale {valore} nella fonte strutturata, e non "
        "compare nel documento. Due verita' divergono, e divergono in silenzio."
        for nome, valore in _numeri(stato).items()
        if valore not in testo and valore.replace(" ", "") not in compatto
    ]
    if stato["release_authorized"] is not False:
        errori.append("assurance/current-state.json: release_authorized non e' false")
    if "release_authorized: false" not in testo:
        errori.append("docs/RELEASE.md: non dichiara release_authorized: false")
    return errori


def cronaca_residua() -> list[str]:
    """Nessun riferimento a cio' che e' stato eliminato."""
    errori: list[str] = []
    schemi = [re.compile(s) for s in _nomi_eliminati()]
    for modello in PERIMETRO:
        for percorso in tracciati(modello) + tracciati(f"*/{modello}"):
            if percorso.startswith("vendor/") or percorso in VALIDATORI:
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
    ("nessuna cronaca residua", cronaca_residua),
)


def main() -> int:
    argparse.ArgumentParser(description=__doc__).parse_args()
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
        f"{len(OPERATIVI)} file operativi, nessun altro Markdown tracciato."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
