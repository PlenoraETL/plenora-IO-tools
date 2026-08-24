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

## Il codice dietro una feature, e quanto di esso questo gate verifica

Una crate compare nel report anche quando se ne misura **meta'**: senza
`--all-features` il percorso GDAL di `driver-filegdb` non viene compilato, e le
sue righe non sono scoperte ma assenti dal denominatore. La promessa qui e'
percio' piu' stretta di "la crate c'e'", e va detta per quello che e':

* **ogni** misuratore dichiarato in `MISURATORI` contiene almeno
  un'invocazione che misura, e **ogni** invocazione porta `--all-features`. La
  verifica era globale, e con tre misuratori bastavano gli altri due a tenerla
  verde mentre uno smetteva di misurare;
* **ogni** ancora compare nel report, non una qualsiasi. Un'ancora e' la prima
  funzione dentro un blocco `#[cfg(...)]` che nomina positivamente una feature
  — anche dentro `all(...)` o `any(...)`, e scendendo dentro i moduli, perche'
  `mod backend` e' il percorso GDAL per intero;
* i blocchi che **non** cominciano con una funzione — una macro, un blocco
  dentro un corpo, una dichiarazione di modulo in un altro file — non hanno una
  riga di cui llvm-cov garantisca il record: il gate li conta e li dichiara
  nella propria uscita invece di far finta di guardarli;
* i blocchi dentro un modulo `cfg(test)` restano fuori: la soglia sorveglia il
  codice di produzione, e un helper di prova che nessuno chiama puo' non avere
  alcun record anche a feature compilata.

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

# Il comando che **misura**, distinto da quello che riporta. La misura decide
# che cosa entra nel denominatore; il report decide che cosa se ne mostra.
INVOCAZIONE_MISURA = "cargo llvm-cov --workspace"

# Senza `--all-features` il denominatore perde il codice dietro una feature.
#
# Non e' un dettaglio di invocazione: `driver-filegdb` tiene l'intero percorso
# GDAL dietro `gdal-backend`, e una misura senza quella feature certifica la
# crate mentre omette il suo codice di produzione. Lo scope si chiama «library
# coverage», e una libreria misurata a meta' non e' la libreria.
FLAG_TUTTE_LE_FEATURE = "--all-features"

# I file dove la **misura** viene invocata. Il report ha i suoi sorvegliati;
# questi sono i posti in cui si decide che cosa viene compilato.
MISURATORI = (WORKFLOW, "scripts/s9-checkpoint.sh", "scripts/campagne_copertura.sh")


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

    errori.extend(_misura_con_tutte_le_feature(radice))

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


def _misura_con_tutte_le_feature(radice: Path) -> list[str]:
    """Ogni invocazione che **misura** compila con tutte le feature.

    Il gate guardava soltanto che ogni crate libreria comparisse nel report, e
    una crate compare anche quando se ne misura la meta': `driver-filegdb` ha il
    percorso GDAL dietro `gdal-backend`, e senza quella feature cinquecento
    righe di produzione restavano fuori dal denominatore -- non «scoperte», ma
    invisibili alla soglia.
    """
    errori: list[str] = []
    for relativo in MISURATORI:
        percorso = radice / relativo
        if not percorso.is_file():
            errori.append(
                f"{relativo}: misuratore dichiarato e assente. Finche' e' fra i "
                "misuratori, la sua misura fa parte di cio' che questo gate "
                "promette; se non misura piu', va tolto da `MISURATORI` con un "
                "gesto che si vede in diff."
            )
            continue
        invocazioni = [
            riga
            for riga in percorso.read_text(encoding="utf-8").splitlines()
            if INVOCAZIONE_MISURA in riga
        ]
        if not invocazioni:
            errori.append(
                f"{relativo}: nessuna invocazione di `{INVOCAZIONE_MISURA}`. La "
                "verifica era globale, e con tre misuratori bastavano gli altri "
                "due a tenerla verde: cancellare la misura da uno solo non "
                "faceva rumore."
            )
            continue
        for riga in invocazioni:
            if FLAG_TUTTE_LE_FEATURE not in riga:
                errori.append(
                    f"{relativo}: la misura della copertura non porta "
                    f"`{FLAG_TUTTE_LE_FEATURE}`. Il codice dietro una feature "
                    "non verrebbe compilato, e uscirebbe dal denominatore senza "
                    "comparire fra le righe scoperte: la soglia sorveglierebbe "
                    "meno di cio' che dichiara."
                )
    return errori


def righe_del_report(testo: str) -> dict[str, set[int]]:
    """`{file: righe strumentate}` dai record `DA:`.

    Serve a una domanda che i soli nomi dei file non possono chiudere: se il
    codice **dietro una feature** sia stato compilato. Un file compare nel
    report anche quando se ne misura solo meta'.
    """
    per_file: dict[str, set[int]] = {}
    corrente: set[int] | None = None
    for riga in testo.splitlines():
        if riga.startswith("SF:"):
            corrente = per_file.setdefault(riga[3:].strip().replace("\\", "/"), set())
        elif riga.startswith("DA:") and corrente is not None:
            numero, _, _ = riga[3:].partition(",")
            try:
                corrente.add(int(numero))
            except ValueError:
                continue
    return per_file


ATTRIBUTO_CFG = re.compile(r"^\s*#\[cfg\((?P<contenuto>.+)\)\]\s*$")
FEATURE = re.compile(r'feature\s*=\s*"([^"]+)"')
FIRMA = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:default\s+|const\s+|async\s+|unsafe\s+|extern\s+\"[^\"]*\"\s+)*fn\s+\w+"
)
MODULO = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*\{")
IGNORABILE = re.compile(r'^\s*(?://|/\*|\*|#!?\[|$)')
PREDICATO_TEST = re.compile(r"(^|[(,\s])test($|[),\s])")


def _senza_negazioni(contenuto: str) -> str:
    """Il predicato `cfg` senza i suoi rami `not(...)`.

    `#[cfg(not(feature = "x"))]` nomina una feature e vale quando **non** c'e':
    ancorarci sopra pretenderebbe strumentata proprio la riga che
    `--all-features` fa sparire. I rami negativi si tolgono a mano perche' le
    parentesi si annidano e una regex non le conta.
    """
    fuori: list[str] = []
    indice = 0
    while indice < len(contenuto):
        if contenuto.startswith("not(", indice):
            profondita = 0
            for scorri in range(indice + 3, len(contenuto)):
                if contenuto[scorri] == "(":
                    profondita += 1
                elif contenuto[scorri] == ")":
                    profondita -= 1
                    if profondita == 0:
                        indice = scorri + 1
                        break
            else:
                indice = len(contenuto)
            continue
        fuori.append(contenuto[indice])
        indice += 1
    return "".join(fuori)


def _moduli_di_test(righe: list[str]) -> list[range]:
    """Le righe dei moduli dietro `cfg(test)`, che non sono il denominatore.

    L'affermazione di questo gate riguarda il **codice di produzione**: e' quello
    che la soglia sorveglia. Il codice di prova sta dietro `cfg(test)`, e la sua
    presenza nel report non e' garantita nemmeno quando la feature e' compilata
    -- un helper che nessuno chiama puo' non avere alcun record. Preteserlo
    renderebbe rosso un report corretto, ed e' successo alla prima stesura di
    questa verifica, su `opzioni_scrittura` in `driver-filegdb`.
    """
    intervalli: list[range] = []
    for indice, riga in enumerate(righe):
        trovato = ATTRIBUTO_CFG.match(riga)
        if not trovato:
            continue
        if not PREDICATO_TEST.search(_senza_negazioni(trovato.group("contenuto"))):
            continue
        for avanti in range(indice + 1, len(righe)):
            if IGNORABILE.match(righe[avanti]):
                continue
            if not MODULO.match(righe[avanti]):
                break
            profondita = 0
            for scorri in range(avanti, len(righe)):
                profondita += righe[scorri].count("{") - righe[scorri].count("}")
                if profondita == 0 and scorri > avanti:
                    intervalli.append(range(indice, scorri + 1))
                    break
            break
    return intervalli


def _riga_ancorabile(righe: list[str], dopo: int) -> int | None:
    """La prima riga **strumentabile** dopo un attributo, o `None`.

    E' la firma di una funzione: llvm-cov emette un record `DA:` per la riga in
    cui una funzione compilata comincia, e per nient'altro con altrettanta
    certezza -- l'intestazione di un `impl`, una `struct`, un `use` non
    producono codice, e pretenderle nel report renderebbe rosso un report
    corretto. Dentro un `mod` si scende, perche' il modulo e' solo
    l'involucro; davanti a qualunque altro elemento ci si ferma e il blocco
    resta **non ancorato**, cioe' dichiarato invece che finto coperto.
    """
    indice = dopo
    while indice < len(righe):
        riga = righe[indice]
        if IGNORABILE.match(riga):
            indice += 1
            continue
        if FIRMA.match(riga):
            return indice
        if MODULO.match(riga):
            # Dentro un modulo **tutto** e' governato dalla stessa feature, e la
            # prima funzione va cercata fino alla graffa che lo chiude: fermarsi
            # al primo `use` lasciava non ancorato `mod backend`, che e' il
            # percorso GDAL per intero. Oltre la chiusura non si guarda: li' il
            # codice non e' piu' dietro quella feature.
            profondita = 0
            for scorri in range(indice, len(righe)):
                profondita += righe[scorri].count("{") - righe[scorri].count("}")
                if profondita == 0 and scorri > indice:
                    fine = scorri
                    break
            else:
                fine = len(righe)
            for dentro in range(indice + 1, fine):
                if FIRMA.match(righe[dentro]):
                    return dentro
            return None
        return None
    return None


def ancore_feature_gated(
    radice: Path,
) -> tuple[dict[str, list[tuple[str, int, str]]], list[tuple[str, int, str]]]:
    """`(ancore per crate, blocchi non ancorati)`.

    Un'ancora e' `(file, riga, feature)`: la firma della prima funzione dentro
    un blocco `#[cfg(...)]` che nomina positivamente una feature -- anche
    dentro `all(...)` o `any(...)`, che la prima stesura non vedeva. Se la
    feature non e' stata compilata, quella riga non ha dati di copertura, e la
    sua assenza dal report dice esattamente cio' che serve sapere.

    I blocchi che non cominciano con una funzione -- un `impl`, una `struct`,
    uno statement dentro un corpo -- non hanno una riga di cui si possa
    pretendere il record: tornano nel secondo elenco, che il gate **conta e
    dichiara**. Contarli e' la differenza fra una verifica parziale e una
    verifica parziale che si sa dov'e' parziale.

    Restano fuori i blocchi dentro un modulo `cfg(test)`: l'affermazione
    riguarda il codice di produzione, e un helper di prova che nessuno chiama
    puo' non avere alcun record anche quando la feature e' compilata.
    """
    per_crate: dict[str, list[tuple[str, int, str]]] = {}
    non_ancorati: list[tuple[str, int, str]] = []
    for nome in crate_libreria(radice):
        sorgenti = sorted((radice / CRATES / nome / "src").rglob("*.rs"))
        for percorso in sorgenti:
            righe = percorso.read_text(encoding="utf-8").splitlines()
            relativo = percorso.relative_to(radice).as_posix()
            di_prova = _moduli_di_test(righe)
            for indice, riga in enumerate(righe):
                trovato = ATTRIBUTO_CFG.match(riga)
                if not trovato:
                    continue
                if any(indice in intervallo for intervallo in di_prova):
                    continue
                feature = FEATURE.search(_senza_negazioni(trovato.group("contenuto")))
                if not feature:
                    continue
                ancora = _riga_ancorabile(righe, indice + 1)
                if ancora is None:
                    non_ancorati.append((relativo, indice + 1, feature.group(1)))
                    continue
                per_crate.setdefault(nome, []).append(
                    (relativo, ancora + 1, feature.group(1))
                )
    return per_crate, non_ancorati


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

    errori.extend(_feature_nel_denominatore(radice, percorso))
    return errori


def _feature_nel_denominatore(radice: Path, percorso: Path) -> list[str]:
    """Il codice dietro una feature e' **compilato**, non solo dichiarato.

    Che la crate compaia nel report non basta: `driver-filegdb` comparirebbe
    anche misurando il solo stub. Si pretende strumentata **ogni** ancora, non
    una qualsiasi: la prima stesura si accontentava che l'elenco delle
    raggiunte non fosse vuoto, e una crate con venti blocchi dietro due feature
    restava verde con una feature sola compilata.
    """
    per_file = righe_del_report(percorso.read_text(encoding="utf-8"))

    def strumentate(relativo: str) -> set[int]:
        for misurato, righe in per_file.items():
            if misurato.endswith("/" + relativo) or misurato == relativo:
                return righe
        return set()

    errori: list[str] = []
    ancore, _ = ancore_feature_gated(radice)
    for nome, elenco in sorted(ancore.items()):
        mancanti = [
            (relativo, riga, feature)
            for relativo, riga, feature in elenco
            if riga not in strumentate(relativo)
        ]
        if not mancanti:
            continue
        feature_toccate = sorted({f for _, _, f in mancanti})
        dettaglio = ", ".join(f"{r}:{n}" for r, n, _ in mancanti[:5])
        if len(mancanti) > 5:
            dettaglio += f", e altre {len(mancanti) - 5}"
        errori.append(
            f"{percorso}: `{nome}` ha {len(mancanti)} blocchi su {len(elenco)} "
            f"dietro una feature la cui prima funzione non compare nel report "
            f"({dettaglio}). Le feature coinvolte sono {feature_toccate}: la "
            "crate e' nel denominatore, quel suo codice no, e la misura e' "
            "stata compilata senza `--all-features`."
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
    if opzioni.lcov:
        # Quanto copre questa verifica, detto dalla verifica stessa. Un gate che
        # tace su cio' che non guarda si fa leggere come se guardasse tutto.
        ancore, non_ancorati = ancore_feature_gated(ROOT)
        quante = sum(len(v) for v in ancore.values())
        print(
            f"  codice dietro una feature: {quante} blocchi ancorati e trovati "
            f"nel report, tutti; {len(non_ancorati)} non ancorabili -- non "
            "cominciano con una funzione (una macro, un blocco dentro un corpo, "
            "una dichiarazione di modulo), e per quelle righe llvm-cov non "
            "garantisce un record di cui si possa pretendere la presenza."
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
