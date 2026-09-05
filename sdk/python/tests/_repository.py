"""Dove sta il repository, quando c'e'.

# Perche' serve

Le sonde di questo pacchetto sono di due specie. La maggior parte prova l'SDK e
basta: costruisce documenti, li da' ai modelli, guarda che cosa ne esce. Alcune
hanno bisogno di cose che stanno nel **repository** -- il contratto
`release/cli-protocol-v2.json`, le fixture della CLI, il binario -- e quelle
cose non entrano nella sdist: il contratto e' del prodotto, le fixture pesano, e
il binario non ci va per scelta.

La prima stesura risaliva per posizione, con `parents[3]`. Funziona finche' i
test stanno dove sono stati scritti, e dalla sdist estratta indica una directory
qualunque: quindici sonde fallivano dicendo che un file non c'era, invece di
dire che non erano applicabili.

# Perche' cercare un marcatore e non contare i livelli

Perche' la profondita' e' un'ipotesi sulla posizione, e il marcatore e' un
fatto sul contenuto. Chi estrae la sdist in una directory annidata come vuole
ottiene la stessa risposta -- nessun repository -- invece di trovarne uno
sbagliato.
"""

from __future__ import annotations

import unittest
from pathlib import Path

#: Il file che dice «questo e' il repository e non un'altra cosa».
#:
#: Il contratto e non `.git`: un checkout esportato senza `.git` resta il
#: repository, e cio' che alle sonde serve e' il contratto.
MARCATORE = Path("release") / "cli-protocol-v2.json"


def radice() -> Path | None:
    """La radice del repository, o `None` se le sonde girano altrove."""
    for candidata in Path(__file__).resolve().parents:
        if (candidata / MARCATORE).is_file():
            return candidata
    return None


RADICE = radice()
CONTRATTO = RADICE / MARCATORE if RADICE else None
CANONICHE = (
    RADICE / "crates" / "plenora-io-cli" / "tests" / "fixtures" / "canoniche"
    if RADICE
    else None
)

#: Da mettere su una sonda che senza il repository non ha niente da guardare.
#:
#: Uno `skip` e non un fallimento: quella sonda non e' rossa, e' **inapplicabile**,
#: e le due cose vanno distinte o chi installa la sdist legge quindici rossi che
#: non dicono niente sul pacchetto che ha in mano.
serve_il_repository = unittest.skipUnless(
    RADICE is not None,
    "serve il repository: il contratto e le fixture non stanno nella sdist",
)

serve_le_fixture = unittest.skipUnless(
    CANONICHE is not None and CANONICHE.is_dir(),
    "servono le fixture canoniche della CLI, che non stanno nella sdist",
)
