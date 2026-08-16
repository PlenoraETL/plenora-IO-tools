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

Il gate si disattiva da solo: quando le condizioni dell'handoff sono
soddisfatte, smette di vincolare e va rimosso insieme al ponte in S4.e.

## Cosa conta come "soddisfatto" (S4.d, parte 0)

La prima versione si accontentava della **presenza del nome del test in
qualunque punto del crate**: un commento che lo citasse — come quelli scritti
per spiegare cosa mancasse — sarebbe bastato a sbloccare il gate. Allo stesso
modo chiedeva `InternalMemoryLease` "da qualche parte nel core", che un
`use` inutilizzato avrebbe soddisfatto.

Ora le condizioni sono ancorate ai file che devono davvero cambiare, e il
test e' cercato come **definizione** `#[test] fn <nome>`, non come stringa.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

CORE = ROOT / "crates" / "plenora-io-core" / "src"

# Il percorso comune: i due file che oggi prenotano con la lease legacy e che
# S4.d deve portare sul modello unificato.
PERCORSO_COMUNE = ("driver/spool.rs", "driver/reader_adapters.rs")

# Il test che dimostra l'handoff sul percorso reale: costruisce le opzioni dal
# modello unificato, apre e legge davvero attraverso adapter e spool, e lo fa
# senza passare dal ponte legacy.
TEST_HANDOFF = "handoff_reale_della_memoria_senza_bridge_legacy"

# Definizione vera, non menzione: `#[test]` seguito dalla firma. Fra i due
# possono stare altri attributi (`#[ignore]`, `#[should_panic]`), non
# commenti di codice arbitrario.
DEFINIZIONE_TEST = re.compile(
    r"#\[test\]\s*(?:#\[[^\]]*\]\s*)*(?:async\s+)?fn\s+" + re.escape(TEST_HANDOFF) + r"\s*\(",
    re.MULTILINE,
)

COSTRUTTORI_PIPELINE = re.compile(r"::from_(?:read|write)_parts\b")


def sorgenti_rust(root: Path) -> list[tuple[str, Path]]:
    """Ogni `.rs` di ogni crate, non solo quelli sotto `src/`."""
    trovati: list[tuple[str, Path]] = []
    for crate_dir in sorted((root / "crates").iterdir()):
        if not crate_dir.is_dir():
            continue
        for sorgente in sorted(crate_dir.rglob("*.rs")):
            if "target" in sorgente.relative_to(crate_dir).parts:
                continue
            trovati.append((crate_dir.name, sorgente))
    fuzz = root / "fuzz"
    if fuzz.is_dir():
        for sorgente in sorted(fuzz.rglob("*.rs")):
            if "target" in sorgente.relative_to(fuzz).parts:
                continue
            trovati.append(("fuzz", sorgente))
    return trovati


def handoff_completato() -> tuple[bool, list[str]]:
    """Le condizioni che rendono il ramo Pipeline realmente utilizzabile."""
    mancanti: list[str] = []

    for relativo in PERCORSO_COMUNE:
        sorgente = CORE / relativo
        if not sorgente.is_file():
            mancanti.append(f"{relativo}: assente")
            continue
        testo = sorgente.read_text(encoding="utf-8")
        if "ResourceLease" in testo:
            mancanti.append(
                f"{relativo}: prenota ancora con `ResourceLease` (modello legacy)"
            )
        # Ancorato al file che deve cambiare, non al crate: un `use` altrove
        # non dimostra che questo percorso usi la lease nuova.
        if "InternalMemoryLease" not in testo:
            mancanti.append(f"{relativo}: non usa `InternalMemoryLease`")
        if "shrink_to" not in testo:
            mancanti.append(
                f"{relativo}: non riduce la prenotazione con `shrink_to`, quindi "
                "l'handoff senza finestra scoperta non e' cablato qui"
            )

    sorgenti_core = list(CORE.glob("**/*.rs"))
    testo_core = "\n".join(s.read_text(encoding="utf-8") for s in sorgenti_core)
    if not DEFINIZIONE_TEST.search(testo_core):
        mancanti.append(
            f"plenora-io-core: manca la **definizione** `#[test] fn {TEST_HANDOFF}`. "
            "Citarne il nome in un commento non e' una dimostrazione."
        )

    return (not mancanti), mancanti


def costruttori_fuori_dal_core() -> list[str]:
    trovati: list[str] = []
    for crate, sorgente in sorgenti_rust(ROOT):
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
