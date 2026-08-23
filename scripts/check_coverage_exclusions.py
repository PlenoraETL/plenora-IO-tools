#!/usr/bin/env python3
"""Verifica che la coverage misuri lo scope dichiarato (INFRA-0.1).

Lo scope e' **library coverage**: la soglia dell'80% vale sulle librerie del
workspace, non su cio' che il workspace usa come attrezzaggio. Le crate non
libreria — `plenora-bench`, `plenora-fuzz`, `plenora-io-cli` — restano fuori
dal denominatore.

Fino a INFRA-0.1 l'esclusione era scritta come
`(plenora-bench|plenora-fuzz|plenora-io-cli)/src/main\\.rs$`, cioe' nominava un
file per crate invece della crate. `plenora-bench/src/bin/spool_ab.rs` non e'
un `main.rs` e rientrava nella misura: 146 righe a 0% nel denominatore di una
percentuale dichiarata "delle librerie". Il modo di sbagliare non e' finito con
quel file: qualunque binario aggiunto domani sotto `src/bin/`, o qualunque
modulo accanto al `main.rs`, rientrerebbe allo stesso modo e in silenzio,
perche' una percentuale che passa la soglia non fa rumore.

## Come misura

Il gate non confronta la regex con una costante scritta qui accanto: la
**deriva dal workspace**. Una crate e' libreria se dichiara una libreria —
`src/lib.rs` o una sezione `[lib]` nel manifest — e non lo e' altrimenti. La
regex canonica e' quindi una funzione dell'albero, e una crate binaria nuova
la cambia da sola: chi la aggiunge senza aggiornare il workflow trova il gate
rosso, invece di una percentuale che si sposta senza che nessuno lo decida.

Sopra questa derivazione ci sono tre verifiche, e servono tutte e tre:

* **presenza** — ogni invocazione di `cargo llvm-cov report` nel workflow
  dichiara l'esclusione. Senza questa, cancellare la riga farebbe passare il
  gate; e la dichiara **ogni** invocazione, perche' un report esportato senza
  filtro e una soglia applicata con filtro sono due misure diverse che si
  presentano con lo stesso nome.
* **esclusivita'** — nessun altro valore di `--ignore-filename-regex` compare
  nei file sorvegliati. Senza questa, indebolire la regex passando accanto a
  quella giusta non verrebbe visto.
* **osservazione del report** — sul LCOV prodotto davvero (`--lcov`) nessun
  file delle crate non libreria compare, e ogni crate libreria compare. Le
  prime due verifiche leggono l'intenzione scritta nel workflow; questa legge
  il risultato. Sono cose diverse: una regex puo' essere quella giusta e non
  fare cio' che si crede.

La seconda meta' dell'ultima verifica — "ogni crate libreria compare" —
sorveglia l'errore speculare, che e' il piu' comodo da commettere: una regex
troppo larga alza la percentuale togliendo dal denominatore proprio il codice
che la soglia doveva sorvegliare.

La soglia e' verificata come valore, non come presenza: se la copertura scende
sotto l'80% si aggiungono test, non si sposta la soglia. Spostarla e' una
decisione che va presa, e questo gate la rende visibile invece che comoda.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# Le funzioni prendono la radice come parametro invece di leggere ROOT: le
# sonde negative in test_check_coverage_exclusions.py costruiscono un albero
# finto e ci verificano che ogni indebolimento venga davvero intercettato.
# Provarlo mutando i file veri lascerebbe il repository sporco se un test si
# interrompe.
CRATES = "crates"

# Il workflow e' l'unico posto che *deve* dichiarare l'esclusione; gli altri
# file sono sorvegliati perche' non ne dichiarino una diversa.
#
# `campagne_copertura.sh` misura la copertura piu' volte sulla stessa
# revisione, e deve misurare **lo stesso perimetro**: un'esclusione diversa
# darebbe numeri che non si confrontano con le evidenze del checkpoint, e la
# differenza sembrerebbe della corsa invece che del perimetro.
WORKFLOW = ".github/workflows/ci.yml"
SORVEGLIATI = (WORKFLOW, "Dockerfile.dev", "scripts/campagne_copertura.sh")

# Soglia di riga dichiarata per lo scope "library coverage". E' un valore, non
# un minimo: il gate rifiuta anche chi la alza, perche' una soglia che si
# muove insieme alla misura non e' una soglia.
SOGLIA_LINEE = 80

FLAG_ESCLUSIONE = re.compile(r"--ignore-filename-regex[ =]+(['\"])(?P<valore>.+?)\1")
FLAG_SOGLIA = re.compile(r"--fail-under-lines[ =]+(?P<valore>\d+)")
INVOCAZIONE_REPORT = "cargo llvm-cov report"


def crate_non_libreria(radice: Path) -> tuple[str, ...]:
    """Le crate del workspace che non dichiarano una libreria, in ordine."""
    nomi = []
    for manifest in sorted((radice / CRATES).glob("*/Cargo.toml")):
        cartella = manifest.parent
        if (cartella / "src" / "lib.rs").is_file():
            continue
        if re.search(r"^\[lib\]", manifest.read_text(encoding="utf-8"), re.MULTILINE):
            continue
        nomi.append(cartella.name)
    return tuple(nomi)


def crate_libreria(radice: Path) -> tuple[str, ...]:
    non_librerie = set(crate_non_libreria(radice))
    return tuple(
        manifest.parent.name
        for manifest in sorted((radice / CRATES).glob("*/Cargo.toml"))
        if manifest.parent.name not in non_librerie
    )


def regex_canonica(radice: Path) -> str:
    """L'esclusione che lo scope dichiarato implica, derivata dal workspace.

    Esclude *tutti* i sorgenti delle crate non libreria, non un file per crate:
    e' la differenza fra escludere una crate ed escludere il suo `main.rs`.
    """
    nomi = crate_non_libreria(radice)
    return r"(^|/)(" + "|".join(nomi) + r")/src/.*\.rs$"


def _blocchi_passo(testo: str) -> list[str]:
    """Spezza il workflow nei suoi passi, per attribuire i flag a chi li usa."""
    return re.split(r"\n(?=\s*- )", testo)


def verifica_workflow(radice: Path) -> list[str]:
    attesa = regex_canonica(radice)
    errori: list[str] = []

    if not crate_non_libreria(radice):
        errori.append(
            "nessuna crate non libreria nel workspace: la regex di esclusione "
            "sarebbe vuota e escluderebbe tutto. Se le crate binarie sono "
            "davvero sparite, questo gate va rimosso per scelta."
        )
        return errori

    for relativo in SORVEGLIATI:
        percorso = radice / relativo
        if not percorso.is_file():
            continue
        for corrispondenza in FLAG_ESCLUSIONE.finditer(percorso.read_text(encoding="utf-8")):
            trovata = corrispondenza.group("valore")
            if trovata != attesa:
                errori.append(
                    f"{relativo}: esclusione `{trovata}`, ma lo scope "
                    f"\"library coverage\" implica `{attesa}`. Una regex piu' "
                    "stretta lascia nel denominatore codice di attrezzaggio; "
                    "una piu' larga ne toglie proprio il codice sorvegliato."
                )

    percorso_workflow = radice / WORKFLOW
    if not percorso_workflow.is_file():
        errori.append(f"{WORKFLOW}: manca, ma e' dove la coverage e' misurata.")
        return errori

    testo = percorso_workflow.read_text(encoding="utf-8")
    invocazioni = [b for b in _blocchi_passo(testo) if INVOCAZIONE_REPORT in b]
    if not invocazioni:
        errori.append(
            f"{WORKFLOW}: nessuna invocazione di `{INVOCAZIONE_REPORT}`. "
            "Senza un report la soglia non e' applicata da nessuno."
        )
        return errori

    con_soglia = 0
    con_lcov = 0
    for blocco in invocazioni:
        etichetta = _etichetta(blocco)
        if not FLAG_ESCLUSIONE.search(blocco):
            errori.append(
                f"{WORKFLOW}: il passo `{etichetta}` invoca "
                f"`{INVOCAZIONE_REPORT}` senza `--ignore-filename-regex`. "
                "Ogni report deve dichiarare l'esclusione: un report esportato "
                "senza filtro e una soglia applicata con filtro sono due misure "
                "diverse che si presentano con lo stesso nome."
            )
        if "--lcov" in blocco:
            con_lcov += 1
        for corrispondenza in FLAG_SOGLIA.finditer(blocco):
            con_soglia += 1
            valore = int(corrispondenza.group("valore"))
            if valore != SOGLIA_LINEE:
                errori.append(
                    f"{WORKFLOW}: il passo `{etichetta}` dichiara una soglia di "
                    f"{valore}%, ma quella dello scope e' {SOGLIA_LINEE}%. Se la "
                    "copertura scende sotto soglia si aggiungono test: la soglia "
                    "non segue la misura."
                )

    if con_soglia == 0:
        errori.append(
            f"{WORKFLOW}: nessun passo dichiara `--fail-under-lines`. "
            "Senza, la coverage viene misurata e non applicata."
        )
    if con_lcov == 0:
        errori.append(
            f"{WORKFLOW}: nessun passo esporta il report LCOV. E' il report su "
            "cui questo gate osserva le esclusioni davvero applicate."
        )

    return errori


def _etichetta(blocco: str) -> str:
    corrispondenza = re.search(r"-\s*name:\s*(.+)", blocco)
    return corrispondenza.group(1).strip() if corrispondenza else "senza nome"


def file_del_report(testo: str) -> list[str]:
    """I percorsi sorgente che partecipano al report, normalizzati."""
    return [
        riga[len("SF:"):].strip().replace("\\", "/")
        for riga in testo.splitlines()
        if riga.startswith("SF:")
    ]


def verifica_report(radice: Path, percorso: Path) -> list[str]:
    """Osserva il LCOV prodotto: l'intenzione scritta nel workflow e' un'altra cosa."""
    errori: list[str] = []
    if not percorso.is_file():
        return [f"{percorso}: report LCOV assente, non c'e' niente da verificare."]

    file_misurati = file_del_report(percorso.read_text(encoding="utf-8"))
    if not file_misurati:
        return [
            f"{percorso}: nessun record `SF:`. Un report vuoto supera qualunque "
            "soglia, quindi vale come fallimento e non come assenza di dati."
        ]

    for nome in crate_non_libreria(radice):
        intrusi = sorted(f for f in file_misurati if f"/{nome}/src/" in f"/{f}")
        if intrusi:
            quanti = (
                "un file" if len(intrusi) == 1 else f"{len(intrusi)} file"
            )
            errori.append(
                f"{percorso}: {quanti} di `{nome}` nel denominatore, ma la "
                "crate non e' una libreria e lo scope dichiarato e' "
                "\"library coverage\": " + ", ".join(intrusi)
            )

    for nome in crate_libreria(radice):
        if not any(f"/{nome}/src/" in f"/{f}" for f in file_misurati):
            errori.append(
                f"{percorso}: la crate libreria `{nome}` non compare nel "
                "report. Escludere una libreria alza la percentuale togliendo "
                "dal denominatore proprio il codice che la soglia sorveglia."
            )

    return errori


def verifica(radice: Path, lcov: Path | None = None) -> list[str]:
    """Restituisce l'elenco delle divergenze; vuoto se lo scope e' rispettato."""
    errori = verifica_workflow(radice)
    if lcov is not None:
        errori.extend(verifica_report(radice, lcov))
    return errori


def main() -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument(
        "--lcov",
        type=Path,
        help="report LCOV da osservare; senza, si verifica solo il workflow",
    )
    opzioni = argomenti.parse_args()

    errori = verifica(ROOT, opzioni.lcov)
    if errori:
        for messaggio in errori:
            print(messaggio, file=sys.stderr)
        return 1

    non_librerie = crate_non_libreria(ROOT)
    print(
        "scope coverage rispettato: soglia "
        f"{SOGLIA_LINEE}% sulle {len(crate_libreria(ROOT))} crate libreria, "
        f"escluse {', '.join(non_librerie)}"
        + (f"; report osservato: {opzioni.lcov}" if opzioni.lcov else "")
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
