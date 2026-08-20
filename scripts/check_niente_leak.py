#!/usr/bin/env python3
"""Nessuna promozione **non autorizzata** di testo runtime a `&'static str`.

# Il limite che questo gate esiste per non far dimenticare

S9 impone che il testo dei messaggi pubblici sia scelto a compile time, e lo fa
con il tipo: `PublicMessage::Curated` prende un `&'static str`.

**`&'static str` garantisce la durata, non la provenienza.** Un chiamante
deliberato puo' trasformare testo runtime in `'static` con `Box::leak`, e
infilarlo in un messaggio curato senza che il compilatore obietti. La garanzia
realistica di S9 e' percio' piu' stretta di quanto il tipo lasci intendere:

> impedire la propagazione **accidentale** di testo runtime nel workspace, non
> rendere crittograficamente inconiabile un messaggio dinamico da codice
> ostile.

I crate sono interni e `publish = false`: l'avversario di questo invariante e'
la distrazione, non un aggressore. Questo gate copre la distrazione.

# La proprieta' non e' «zero occorrenze»

Nel repository **una occorrenza esiste deliberatamente**: il doctest che
dimostra il limite, e che deve restare eseguibile — una garanzia descritta piu'
forte di com'e' e' peggio di una dichiarata con il suo limite, e una
dimostrazione marcata `ignore` sarebbe un'affermazione non verificata.

La proprieta' verificata e' quindi:

> zero occorrenze non autorizzate; **una sola** dimostrazione eseguibile e
> identificata.

L'identita' e' il marcatore `DIMOSTRAZIONE-LIMITE-STATIC` dentro il blocco, non
un numero di riga: un'identita' per riga si stacca al primo commit che sposta
il file. Il gate diventa rosso in **tutti e tre** i modi in cui l'attestazione
puo' rompersi: un'occorrenza altrove, piu' di una attestata, oppure
l'attestazione che sopravvive alla propria occorrenza.

# Vale anche per i test, e per i doctest

La prima stesura dei test sul tetto del messaggio otteneva gli statici lunghi
con `Box::leak`. Funzionava, e diceva la cosa sbagliata: un test che si
costruisce lo statico a runtime dimostra proprio cio' che S9 non promette. Un
divieto limitato alla produzione non l'avrebbe intercettato.

I doctest sono nel perimetro per la stessa ragione: escluderli in blocco
sarebbe una deroga **piu' ampia** di una allowlist, e un `Box::leak` in un
qualunque altro esempio della documentazione — cioe' nella prima cosa che un
consumatore copia — resterebbe invisibile.

Che cosa sia un doctest lo decide `check_errori_redatti.doctest_che_devono_compilare`,
riusata qui: due definizioni diverse divergerebbero, e divergerebbero in
silenzio.

Per gli statici lunghi la via legittima e' `concat!` su letterali, che produce
un letterale: la provenienza e' letterale **per costruzione**.
"""

from __future__ import annotations

import pathlib
import re
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

from check_errori_redatti import (  # noqa: E402
    doctest_che_devono_compilare,
    funzione_che_racchiude,
    intervalli_di_funzione,
    sorgenti,
    spoglia,
)

ROOT = pathlib.Path(__file__).resolve().parent.parent

# `Box::leak`, `String::leak`, `Vec::leak` e la forma a metodo `.leak()`.
PROMOZIONE = re.compile(r"\b(?:Box|String|Vec)::leak\s*\(|\.leak\s*\(\s*\)")

# L'unica occorrenza autorizzata, identificata dal marcatore e dal file.
ATTESTAZIONE = "DIMOSTRAZIONE-LIMITE-STATIC"
SORGENTE_ATTESTATA = "crates/plenora-io-model/src/error.rs"


def violazioni(radice: pathlib.Path) -> list[str]:
    """`[]` se e solo se: zero promozioni non autorizzate **e** esattamente una
    attestata."""
    fuori: list[str] = []
    attestate = 0

    for sorgente in sorgenti(radice):
        grezzo = sorgente.read_text(encoding="utf-8")
        percorso = sorgente.relative_to(radice).as_posix()

        # --- codice: nessuna autorizzazione possibile ----------------------
        testo = spoglia(grezzo)
        if PROMOZIONE.search(testo):
            intervalli = intervalli_di_funzione(testo)
            for m in PROMOZIONE.finditer(testo):
                funzione = funzione_che_racchiude(intervalli, m.start())
                fuori.append(
                    f"{percorso}::{funzione}: `{m.group(0).strip()}` promuove "
                    "testo runtime a `'static`. `&'static str` garantisce la "
                    "durata, non la provenienza: per uno statico lungo si usa "
                    "`concat!` su letterali, che e' letterale per costruzione."
                )

        # --- doctest: una sola occorrenza, attestata ------------------------
        for riga, codice in doctest_che_devono_compilare(grezzo):
            # Il marcatore si cerca nel testo **grezzo**, perche' e' un
            # commento e `spoglia` lo cancellerebbe; la promozione nel testo
            # spogliato, perche' nominarla in un commento non e' usarla.
            if not PROMOZIONE.search(spoglia(codice)):
                continue
            if ATTESTAZIONE in codice and percorso == SORGENTE_ATTESTATA:
                attestate += 1
                continue
            fuori.append(
                f"{percorso}: il doctest alla riga {riga} promuove testo "
                "runtime a `'static`. I doctest sono la prima cosa che un "
                f"consumatore copia. L'unica occorrenza autorizzata e' quella "
                f"marcata `{ATTESTAZIONE}` in {SORGENTE_ATTESTATA}."
            )

    if attestate == 0:
        fuori.append(
            f"{SORGENTE_ATTESTATA}: l'occorrenza attestata `{ATTESTAZIONE}` non "
            "esiste piu'. Se la dimostrazione del limite e' stata tolta, va "
            "tolta anche l'attestazione, nello stesso commit: un'autorizzazione "
            "che sopravvive al proprio codice autorizza qualcosa che nessuno "
            "rilegge."
        )
    elif attestate > 1:
        fuori.append(
            f"{SORGENTE_ATTESTATA}: {attestate} occorrenze attestate, una sola "
            "ammessa. La dimostrazione del limite e' una; due sarebbero una "
            "deroga che cresce."
        )

    return fuori


def main() -> int:
    errori = violazioni(ROOT)
    for messaggio in errori:
        print(messaggio, file=sys.stderr)
    if errori:
        return 1
    print(
        "promozioni a 'static: zero non autorizzate, "
        f"una attestata ({ATTESTAZIONE})"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
