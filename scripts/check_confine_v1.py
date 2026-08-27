#!/usr/bin/env python3
"""L'identita' del v1 esce da **un solo** posto, e da nessun altro.

# Che cosa protegge

`FidelityReason` porta due testi: quello **curato**, che il v2 pubblica, e
`dettaglio_v1`, che e' la frase congelata del protocollo v1 con dentro i nomi
presi dal file. Il secondo e' privato e `#[serde(skip)]`, e si legge da un solo
accessore, `detail_v1()`.

Un accessore pubblico pero' e' pubblico: qualunque modulo puo' chiamarlo, e la
prima chiamata fuori posto rimetterebbe sul filo del v2 esattamente i nomi che
il v2 toglie. La visibilita' di Rust non sa dire «questo modulo e nessun
altro» -- `pub(in ...)` legherebbe il tipo a un percorso di un altro crate, e
`pub(crate)` non basta perche' l'adattatore v2 vive nello stesso crate di
quello v1.

Quello che il type system non esprime, lo verifica un gate.

# E il derive che non deve tornare

`FidelityAssessment` non deriva `Serialize`, e non e' una dimenticanza: il
derive pubblicherebbe `prime_v1` -- le sessantaquattro frasi congelate -- e la
meccanica del trattenimento. Toglierlo pero' non impedisce a nessuno di
rimetterlo, e un `#[derive(...)]` in piu' non si nota rileggendo un diff. Qui
la sua assenza e' pretesa.

# Che cosa non guarda

Il codice di prova. Le sonde della redazione **devono** chiamare `detail_v1()`:
e' cio' che verificano. Il gate si ferma al primo modulo di prova di ogni file,
come fa `check_categorie_di_perdita.py`, e lo dichiara invece di sottintenderlo.

Guarda il testo del sorgente spogliato di commenti e stringhe, non l'albero
sintattico: afferma che nessun altro file **nomina** quella chiamata, non che
nessuno la esegua per altre vie. E' la stessa fiducia che si da' agli altri
gate di censimento, ed e' scritta qui invece di essere sottintesa.
"""

from __future__ import annotations

import pathlib
import re
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

from check_errori_redatti import spoglia  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
CRATES = ROOT / "crates"

#: L'unico file che puo' leggere l'identita' legacy, e il modulo che deve
#: contenerne la chiamata.
ADATTATORE = "plenora-io-cli/src/busta.rs"
MODULO = "mod legacy_v1"

#: Il modulo che **possiede** il campo.
#:
#: Non e' un'eccezione concessa, e' il posto dove la privatezza lo confina gia'
#: da se': `dettaglio_v1` e' privato, quindi dentro `loss.rs` il campo si legge
#: comunque, con o senza accessore. Li' la chiamata compare due volte -- la
#: definizione, e la chiave con cui `prime_v1` deduplica come faceva il v1 --
#: e nessuna delle due pubblica niente: confrontare non e' emettere. Il gate
#: esiste per gli **altri** moduli, che il campo non lo vedono e l'accessore
#: si'.
PROPRIETARIO = "plenora-io-core/src/loss.rs"

CHIAMATA = re.compile(r"\bdetail_v1\s*\(")
MODULO_DI_PROVA = re.compile(r"^\s*(pub )?mod (tests|sonde)\b", re.M)
DERIVE_DELLA_VALUTAZIONE = re.compile(
    r"((?:#\[[^\]]*\]\s*)*)pub struct FidelityAssessment\b"
)


def codice_di_produzione(sorgente: pathlib.Path) -> str:
    """Il testo del file, spogliato e troncato al primo modulo di prova."""
    testo = spoglia(sorgente.read_text(encoding="utf-8"))
    prova = MODULO_DI_PROVA.search(testo)
    return testo[: prova.start()] if prova else testo


def chiamanti() -> dict[str, int]:
    """`percorso relativo -> quante chiamate`, nel solo codice di produzione."""
    trovati: dict[str, int] = {}
    for sorgente in sorted(CRATES.rglob("*.rs")):
        quante = len(CHIAMATA.findall(codice_di_produzione(sorgente)))
        if quante:
            trovati[sorgente.relative_to(CRATES).as_posix()] = quante
    return trovati


def verifica(trovati: dict[str, int] | None = None) -> list[str]:
    errori: list[str] = []
    trovati = chiamanti() if trovati is None else trovati

    estranei = sorted(set(trovati) - {ADATTATORE, PROPRIETARIO})
    if estranei:
        errori.append(
            f"`detail_v1()` e' chiamata fuori dall'adattatore v1: {estranei}. "
            "Quella funzione restituisce i nomi presi dal file, e il v2 esiste per "
            "toglierli: un secondo chiamante li rimette sul filo."
        )
    if ADATTATORE not in trovati:
        errori.append(
            f"`detail_v1()` non e' chiamata da {ADATTATORE}. O l'adattatore v1 ha "
            "smesso di pubblicare l'identita' congelata -- e allora il v1 non e' piu' "
            "congelato -- o e' stato spostato senza aggiornare questo gate."
        )

    sorgente = CRATES / ADATTATORE
    if sorgente.is_file():
        testo = codice_di_produzione(sorgente)
        inizio = testo.find(MODULO)
        if inizio < 0:
            errori.append(
                f"{ADATTATORE}: `{MODULO}` non c'e'. L'adattatore v1 sta in un modulo "
                "suo perche' la condivisione era il difetto: una funzione sola per i "
                "due protocolli farebbe uscire dal v2 cio' che il v2 toglie."
            )
        else:
            for occorrenza in CHIAMATA.finditer(testo):
                if occorrenza.start() < inizio:
                    errori.append(
                        f"{ADATTATORE}: `detail_v1()` e' chiamata **prima** di "
                        f"`{MODULO}`, cioe' fuori dall'adattatore legacy. Nel file "
                        "giusto ma nel posto sbagliato non e' confinamento."
                    )
                    break

    derive = _derive_della_valutazione()
    if derive is None:
        errori.append(
            "`pub struct FidelityAssessment` non si trova: il gate non puo' dire "
            "se derivi `Serialize`."
        )
    elif "Serialize" in derive:
        errori.append(
            "`FidelityAssessment` deriva di nuovo `Serialize`. Il derive pubblicherebbe "
            "`prime_v1` -- le frasi congelate con i nomi del file -- e la meccanica del "
            "trattenimento. Le due forme sul filo le costruisce l'adattatore, a mano."
        )
    return errori


def _derive_della_valutazione() -> str | None:
    percorso = CRATES / "plenora-io-core/src/loss.rs"
    if not percorso.is_file():
        return None
    trovato = DERIVE_DELLA_VALUTAZIONE.search(percorso.read_text(encoding="utf-8"))
    return trovato.group(1) if trovato else None


def main() -> int:
    errori = verifica()
    if errori:
        for errore in errori:
            print(errore, file=sys.stderr)
        print(
            "\nL'identita' del protocollo congelato ha un solo lettore, e la "
            "visibilita' di Rust non sa esprimerlo.",
            file=sys.stderr,
        )
        return 1
    print(
        f"confine v1 verificato: fuori da {PROPRIETARIO}, che possiede il campo, "
        f"`detail_v1()` e' chiamata solo da {ADATTATORE} e dentro `{MODULO}`; "
        "`FidelityAssessment` non deriva `Serialize`."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
