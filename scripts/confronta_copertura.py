#!/usr/bin/env python3
"""Confronta due o piu' `lcov.info` **per (file, riga, coperta o no)**.

# Perche' non basta confrontare le percentuali

`copertura.variazione-fra-corse` e' un blocco perche' la copertura si e' mossa
su un sorgente Rust strumentato invariato. Guardando le percentuali non si
capisce niente: due decimali arrotondano, e a 31 192 righe strumentate il
confine di arrotondamento cade fra una riga e la successiva. Una percentuale
identica puo' nascondere righe diverse, e una percentuale diversa puo' essere
una riga sola.

La domanda a cui questo modulo risponde e' l'unica che porta a una causa:
**quali** righe cambiano di stato fra due campagne.

# Tre differenze, e non sono la stessa cosa

* `strumentate_solo_in` — la riga compare fra le righe strumentate di una
  campagna e non dell'altra. Il **denominatore** e' cambiato, ed e' un fatto
  piu' grave della copertura: significa che lo strumento ha visto un insieme
  diverso di righe;
* `coperte_solo_in` — la riga e' strumentata in entrambe e risulta eseguita in
  una sola. E' la variazione che si sta cercando;
* `conteggio_diverso` — la riga e' coperta in entrambe, con un numero di
  esecuzioni diverso. **Non e' una variazione di copertura**, ed e' riportata a
  parte perche' non muove la percentuale ma indica lo stesso fenomeno.

L'ultima e' segnalata e non fa fallire il confronto: una riga eseguita 3 volte
invece di 2 e' coperta in entrambi i casi, e chiamare quella una divergenza di
copertura sarebbe falso.

# Che cosa questo modulo non dice

Non dice la **causa**. Dice quali righe si muovono e in quali file, che e' il
punto da cui una causa si puo' cercare. Chiamare «rumore» cio' che non e' stato
spiegato e' precisamente l'ipotesi che il blocco esiste per non far passare come
fatto.
"""

from __future__ import annotations

import argparse
import pathlib
import sys
from collections import defaultdict

# `SF:` apre la sezione di un file sorgente, `DA:<riga>,<conteggio>` porta una
# riga strumentata con le volte in cui e' stata eseguita, `end_of_record` la
# chiude. Il resto del formato — funzioni, rami, riepiloghi — non serve qui: i
# riepiloghi sono derivati, e derivarli di nuovo darebbe una seconda verita'.
APERTURA = "SF:"
RIGA = "DA:"
CHIUSURA = "end_of_record"


class LcovMalformato(Exception):
    """Il file non e' un lcov leggibile: meglio fermarsi che contare male."""


def leggi(percorso: pathlib.Path) -> dict[tuple[str, int], int]:
    """`(file, riga) -> conteggio di esecuzioni`.

    Le chiavi sono le righe **strumentate**: quelle con conteggio zero ci sono,
    e sono la differenza fra «non coperta» e «non esistente».
    """
    misure: dict[tuple[str, int], int] = {}
    sorgente: str | None = None

    for numero, testo in enumerate(
        percorso.read_text(encoding="utf-8", errors="replace").splitlines(), 1
    ):
        if testo.startswith(APERTURA):
            sorgente = testo[len(APERTURA) :].strip()
            continue
        if testo.strip() == CHIUSURA:
            sorgente = None
            continue
        if not testo.startswith(RIGA):
            continue
        if sorgente is None:
            raise LcovMalformato(f"{percorso}:{numero}: `DA:` fuori da una sezione `SF:`")
        parti = testo[len(RIGA) :].split(",")
        if len(parti) < 2:
            raise LcovMalformato(f"{percorso}:{numero}: `DA:` senza conteggio")
        try:
            riga, conteggio = int(parti[0]), int(parti[1])
        except ValueError as errore:
            raise LcovMalformato(f"{percorso}:{numero}: {errore}") from errore
        chiave = (sorgente, riga)
        if chiave in misure and misure[chiave] != conteggio:
            raise LcovMalformato(
                f"{percorso}:{numero}: la riga {sorgente}:{riga} compare due "
                f"volte con conteggi diversi ({misure[chiave]} e {conteggio})"
            )
        misure[chiave] = conteggio
    if not misure:
        raise LcovMalformato(f"{percorso}: nessuna riga strumentata")
    return misure


def confronta(
    prima: dict[tuple[str, int], int], seconda: dict[tuple[str, int], int]
) -> dict[str, list[tuple[str, int]]]:
    """Le tre famiglie di differenza, ciascuna in ordine stabile."""
    solo_prima = sorted(set(prima) - set(seconda))
    solo_seconda = sorted(set(seconda) - set(prima))
    comuni = set(prima) & set(seconda)

    coperte_solo_prima = sorted(
        chiave for chiave in comuni if prima[chiave] > 0 and seconda[chiave] == 0
    )
    coperte_solo_seconda = sorted(
        chiave for chiave in comuni if seconda[chiave] > 0 and prima[chiave] == 0
    )
    conteggio_diverso = sorted(
        chiave
        for chiave in comuni
        if prima[chiave] != seconda[chiave] and prima[chiave] > 0 and seconda[chiave] > 0
    )
    return {
        "strumentate_solo_nella_prima": solo_prima,
        "strumentate_solo_nella_seconda": solo_seconda,
        "coperte_solo_nella_prima": coperte_solo_prima,
        "coperte_solo_nella_seconda": coperte_solo_seconda,
        "conteggio_diverso": conteggio_diverso,
    }


# Le famiglie che rendono due campagne **diverse in copertura**. Il conteggio di
# esecuzioni non ne fa parte: una riga eseguita tre volte invece di due e'
# coperta in entrambi i casi.
DIVERGENTI = (
    "strumentate_solo_nella_prima",
    "strumentate_solo_nella_seconda",
    "coperte_solo_nella_prima",
    "coperte_solo_nella_seconda",
)


def per_file(chiavi: list[tuple[str, int]]) -> list[tuple[str, list[int]]]:
    """Le righe raggruppate per sorgente: la causa si cerca per file."""
    gruppi: dict[str, list[int]] = defaultdict(list)
    for sorgente, riga in chiavi:
        gruppi[sorgente].append(riga)
    return sorted((s, sorted(r)) for s, r in gruppi.items())


def sommario(misure: dict[tuple[str, int], int]) -> tuple[int, int]:
    """`(coperte, strumentate)`, ricontate qui e non lette da un riepilogo."""
    return sum(1 for conteggio in misure.values() if conteggio > 0), len(misure)


def stampa(
    nomi: tuple[str, str],
    misure: tuple[dict[tuple[str, int], int], dict[tuple[str, int], int]],
    differenze: dict[str, list[tuple[str, int]]],
    quante: int,
) -> None:
    for nome, misura in zip(nomi, misure):
        coperte, strumentate = sommario(misura)
        print(f"{nome}: {coperte}/{strumentate} righe coperte")

    if not any(differenze[famiglia] for famiglia in DIVERGENTI):
        print()
        print("nessuna riga cambia stato fra le due campagne.")
        if differenze["conteggio_diverso"]:
            print(
                f"  {len(differenze['conteggio_diverso'])} righe sono eseguite un "
                "numero diverso di volte, e restano coperte in entrambe: non e' "
                "una variazione di copertura, ma e' lo stesso fenomeno visto piu' "
                "da vicino."
            )
        return

    for famiglia, chiavi in differenze.items():
        if not chiavi:
            continue
        print()
        print(f"{famiglia}: {len(chiavi)} righe")
        gruppi = per_file(chiavi)
        for sorgente, righe in gruppi[:quante]:
            mostrate = ", ".join(str(r) for r in righe[:quante])
            resto = "" if len(righe) <= quante else f", … (+{len(righe) - quante})"
            print(f"  {sorgente}: {mostrate}{resto}")
        if len(gruppi) > quante:
            print(f"  … e altri {len(gruppi) - quante} file")


def main(argv: list[str] | None = None) -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument("lcov", nargs="+", type=pathlib.Path)
    argomenti.add_argument(
        "--quante",
        type=int,
        default=10,
        help="quanti file e quante righe per file mostrare (predefinito 10)",
    )
    opzioni = argomenti.parse_args(argv)

    if len(opzioni.lcov) < 2:
        print("servono almeno due campagne da confrontare.", file=sys.stderr)
        return 2

    try:
        misure = [leggi(percorso) for percorso in opzioni.lcov]
    except (LcovMalformato, OSError) as errore:
        print(str(errore), file=sys.stderr)
        return 2

    # Ogni campagna si confronta con la **prima**: confrontare a coppie
    # consecutive nasconderebbe una campagna che torna al punto di partenza.
    divergono = False
    for indice, altra in enumerate(misure[1:], 1):
        if indice > 1:
            print()
            print("-" * 70)
        differenze = confronta(misure[0], altra)
        stampa(
            (opzioni.lcov[0].name, opzioni.lcov[indice].name),
            (misure[0], altra),
            differenze,
            opzioni.quante,
        )
        if any(differenze[famiglia] for famiglia in DIVERGENTI):
            divergono = True

    print()
    if divergono:
        print("le campagne NON coincidono riga per riga.")
        print("  Che cosa lo produca non e' detto qui: questo confronto dice")
        print("  quali righe si muovono, che e' il punto da cui una causa si")
        print("  puo' cercare. Chiamarlo «rumore» senza averla trovata")
        print("  trasformerebbe un'ipotesi in un fatto.")
        return 1
    print("le campagne coincidono riga per riga.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
