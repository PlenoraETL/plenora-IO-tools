#!/usr/bin/env python3
"""Il ramo Pipeline non e' utilizzabile prima dell'handoff reale (S4.b.3).

`ReadOptions::from_read_parts` esiste dal commit S4.b ed e' coperto da test,
ma il **percorso comune** — l'adapter di lettura e lo `StagedSpool` — prenota
ancora memoria con la `ResourceLease` del modello legacy. Finche' e' cosi', un
driver che costruisse opzioni sul ramo `Pipeline` otterrebbe un oggetto
formalmente corretto e un comportamento a meta': i contatori di riga
verrebbero dal modello nuovo, la memoria dei batch da quello vecchio, e la
finestra non contabilizzata che `InternalMemoryLease::shrink_to` chiude
resterebbe aperta proprio sul percorso che dovrebbe averla chiusa.

"Costruibile" non e' "utilizzabile". Questo gate tiene separate le due cose:
finche' l'handoff non e' cablato sul percorso reale e dimostrato da un test
end-to-end, nessun crate fuori da `plenora-io-core` puo' costruire opzioni
`Pipeline`.

Il gate si disattiva da solo: quando le tre condizioni dell'handoff sono
soddisfatte, smette di vincolare e va rimosso insieme al ponte in S4.e.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

CORE = ROOT / "crates" / "plenora-io-core" / "src"

# Il test che dimostra l'handoff sul percorso reale: costruisce le opzioni dal
# modello unificato, apre e legge davvero attraverso adapter e spool, e lo fa
# senza passare dal ponte legacy.
TEST_HANDOFF = "handoff_reale_della_memoria_senza_bridge_legacy"

COSTRUTTORI_PIPELINE = re.compile(r"::from_(?:read|write)_parts\b")


def handoff_completato() -> tuple[bool, list[str]]:
    """Le tre condizioni che rendono il ramo Pipeline realmente utilizzabile."""
    mancanti: list[str] = []

    percorso_comune = [
        CORE / "driver" / "spool.rs",
        CORE / "driver" / "reader_adapters.rs",
    ]
    for sorgente in percorso_comune:
        if not sorgente.is_file():
            mancanti.append(f"{sorgente.name}: assente")
            continue
        if "ResourceLease" in sorgente.read_text(encoding="utf-8"):
            mancanti.append(
                f"{sorgente.name}: prenota ancora con `ResourceLease` (modello legacy)"
            )

    sorgenti_core = list(CORE.glob("**/*.rs"))
    testo_core = "\n".join(s.read_text(encoding="utf-8") for s in sorgenti_core)
    if "InternalMemoryLease" not in testo_core:
        mancanti.append(
            "plenora-io-core: nessun uso di `InternalMemoryLease`, quindi "
            "`shrink_to` + move non e' cablato sul percorso reale"
        )
    if TEST_HANDOFF not in testo_core:
        mancanti.append(f"plenora-io-core: manca il test `{TEST_HANDOFF}`")

    return (not mancanti), mancanti


def costruttori_fuori_dal_core() -> list[str]:
    trovati: list[str] = []
    for sorgente in sorted((ROOT / "crates").glob("*/src/**/*.rs")):
        crate = sorgente.relative_to(ROOT / "crates").parts[0]
        if crate == "plenora-io-core":
            continue
        if COSTRUTTORI_PIPELINE.search(sorgente.read_text(encoding="utf-8")):
            trovati.append(sorgente.relative_to(ROOT).as_posix())
    return trovati


def main() -> int:
    completato, mancanti = handoff_completato()
    consumatori = costruttori_fuori_dal_core()

    if completato:
        print(
            "handoff reale completato: il ramo Pipeline e' dichiarabile "
            "utilizzabile, rimuovere questo gate con il ponte (S4.e)"
        )
        return 0

    if consumatori:
        print(
            "Il ramo Pipeline e' costruito fuori da plenora-io-core, ma "
            "l'handoff reale della memoria non e' completo:",
            file=sys.stderr,
        )
        for voce in mancanti:
            print(f"  - {voce}", file=sys.stderr)
        print("Costruttori trovati:", file=sys.stderr)
        for voce in consumatori:
            print(f"  - {voce}", file=sys.stderr)
        print(
            "\nMigrare il percorso comune a `InternalMemoryLease` con "
            f"`shrink_to` + move e aggiungere `{TEST_HANDOFF}` prima di "
            "spostare qualunque driver sul ramo Pipeline.",
            file=sys.stderr,
        )
        return 1

    print(
        "handoff reale non ancora cablato: nessun crate fuori da "
        "plenora-io-core costruisce opzioni Pipeline, come atteso prima di S4.d"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
