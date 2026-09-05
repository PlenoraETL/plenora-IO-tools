#!/usr/bin/env python3
"""Quali sonde saltano, e con quale ragione, quando la suite gira dalla sdist.

# Perche' un elenco e non un numero

«27 saltate» e' un numero, e un numero non dice quali. Se domani una sonda
smettesse di girare per un difetto -- un `skipUnless` sbagliato, un import che
fallisce e viene inghiottito -- il conteggio resterebbe plausibile e nessuno
guarderebbe. Un elenco chiuso invece nomina ciascuna, e il gate lo confronta nei
due versi: una sonda che salta senza essere registrata e' una copertura persa in
silenzio; una registrata che non salta piu' e' una riga da togliere.

# Perche' esegue invece di leggere i decoratori

Perche' `skipUnless` decide a runtime, guardando se il repository ci sia. Cio'
che si vuole sapere e' che cosa salta **davvero** nell'ambiente in cui la sdist
verra' usata, e leggere il decoratore direbbe che cosa salterebbe se la
condizione fosse falsa -- che e' un'altra domanda.
"""

from __future__ import annotations

import argparse
import io
import json
import pathlib
import sys
import unittest


def main(argv: list[str] | None = None) -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument("--tests", required=True, type=pathlib.Path)
    argomenti.add_argument("--uscita", required=True, type=pathlib.Path)
    opzioni = argomenti.parse_args(argv)

    # La directory dei test entra nel percorso: gli helper che le sonde
    # importano -- `_repository` -- stanno li' accanto.
    sys.path.insert(0, str(opzioni.tests.resolve()))

    suite = unittest.defaultTestLoader.discover(
        start_dir=str(opzioni.tests), pattern="test_*.py"
    )
    risultato = unittest.TextTestRunner(stream=io.StringIO(), verbosity=0).run(suite)

    saltate = sorted(
        ({"id": test.id(), "ragione": ragione} for test, ragione in risultato.skipped),
        key=lambda voce: voce["id"],
    )

    documento = {
        "eseguite": risultato.testsRun,
        "saltate": saltate,
        "fallite": len(risultato.failures) + len(risultato.errors),
    }
    opzioni.uscita.write_text(
        json.dumps(documento, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    print(
        f"{risultato.testsRun} sonde, {len(saltate)} saltate, "
        f"{documento['fallite']} fallite -> {opzioni.uscita}"
    )
    return 1 if documento["fallite"] else 0


if __name__ == "__main__":
    raise SystemExit(main())
