#!/usr/bin/env python3
"""Censimento dei `WkbLimits::default()` residui (Lotto 0 / S5, permanente).

S5 ha portato le quote configurate fino all'inferenza di CSV, GeoJSON e XLSX:
fino ad allora quelle passate usavano il default del contratto, e
`--max-wkb-cell-bytes` non le raggiungeva. Chi stringeva il flag otteneva un
rifiuto piu' tardi, o non lo otteneva affatto.

Restano occorrenze legittime, e vanno tenute distinte da quelle che sarebbero
un ritorno del difetto. Questo gate le classifica e fissa il conteggio per
categoria: **non** vieta il simbolo — sarebbe sbagliato, alcune di quelle
occorrenze sono corrette — ma impedisce che ne compaia una nuova senza che
qualcuno la classifichi.

## Le categorie

* **test** — un modulo `#[cfg(test)]` che decodifica un WKB prodotto dal test
  stesso. Il tetto non governa nulla: il dato e' noto e piccolo. Sarebbe
  rumore imporre quote configurate a un `decode_wkb` di verifica.
* **attrezzaggio** — `plenora-bench` e `plenora-fuzz`. Non sono codice
  spedito; il fuzz harness ha le proprie quote strette, e il benchmark misura
  il percorso, non le quote.
* **produzione** — tutto il resto. Ogni occorrenza qui deve avere una ragione
  scritta accanto, e il gate la elenca perche' sia visibile in review.

Come il registro dei fallback, la misura e' sintattica.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

OCCORRENZA = re.compile(r"WkbLimits::default\(\)")

# Conteggio atteso per categoria. `produzione` puo' solo scendere: e' il
# residuo del difetto che S5 ha corretto.
ATTESI = {"test": 45, "attrezzaggio": 4, "produzione": 3}

# Occorrenze di produzione **legittime**: il default e' la scelta giusta, non
# un residuo. Chiave: `percorso:riga`.
LEGITTIME: dict[str, str] = {
    "crates/driver-gpkg/src/lib.rs:1673": (
        "`__fuzz_gpkg_geometry`, entry point `#[doc(hidden)]` per libFuzzer. "
        "L'input del fuzzer e' gia' bounded a 1 MiB dall'harness, quindi il "
        "tetto di 64 MiB non governa nulla; e non ci sono opzioni da cui "
        "prendere una quota, perche' il target non apre un dataset"
    ),
    "crates/driver-shp/src/lib.rs:2460": (
        "`__fuzz_wkb_roundtrip`, stessa natura del precedente"
    ),
}

# Occorrenze di produzione che sono **residui dichiarati**: il default arriva
# dove una quota configurata dovrebbe arrivare. Non fanno fallire il gate —
# sono note — ma restano visibili a ogni corsa, con cio' che le chiuderebbe.
RESIDUI: dict[str, str] = {
    "crates/plenora-io-core/src/driver/reader_adapters.rs:633": (
        "`collect_read_violations` valida le geometrie del batch con il tetto "
        "predefinito perche' non riceve le opzioni: la firma prende contratto, "
        "batch e offset. Un `--max-wkb-cell-bytes` piu' stretto del default "
        "non viene quindi applicato qui, benche' lo sia in inferenza e nella "
        "materializzazione. **Fuori dal perimetro di S5**, che copre "
        "l'inferenza dei tre driver tabellari; chiuderlo richiede di portare i "
        "limiti dell'operazione dentro la validazione del contratto di lettura"
    ),
}

ATTREZZAGGIO = ("plenora-bench", "plenora-fuzz", "fuzz")


def sorgenti(root: Path) -> list[Path]:
    trovati: list[Path] = []
    for radice in (root / "crates", root / "fuzz"):
        if not radice.is_dir():
            continue
        for sorgente in sorted(radice.rglob("*.rs")):
            if "target" in sorgente.relative_to(radice).parts:
                continue
            trovati.append(sorgente)
    return trovati


def righe_di_test(testo: str) -> set[int]:
    """Numeri di riga (1-based) dentro un modulo `#[cfg(test)]`."""
    dentro: set[int] = set()
    i = 0
    while True:
        j = testo.find("#[cfg(test)]", i)
        if j == -1:
            break
        apertura = testo.find("{", j)
        if apertura == -1:
            break
        if "mod " not in testo[j:apertura]:
            i = j + 1
            continue
        profondita = 0
        k = apertura
        while k < len(testo):
            if testo[k] == "{":
                profondita += 1
            elif testo[k] == "}":
                profondita -= 1
                if profondita == 0:
                    break
            k += 1
        prima = testo.count("\n", 0, j) + 1
        ultima = testo.count("\n", 0, k) + 1
        dentro.update(range(prima, ultima + 1))
        i = k
    return dentro


def main() -> int:
    conteggi = {nome: 0 for nome in ATTESI}
    produzione: list[tuple[str, int]] = []

    for sorgente in sorgenti(ROOT):
        testo = sorgente.read_text(encoding="utf-8")
        if not OCCORRENZA.search(testo):
            continue
        percorso = sorgente.relative_to(ROOT).as_posix()
        crate = sorgente.relative_to(ROOT).parts[1 if percorso.startswith("crates/") else 0]
        in_test = righe_di_test(testo)
        for trovata in OCCORRENZA.finditer(testo):
            riga = testo.count("\n", 0, trovata.start()) + 1
            if crate in ATTREZZAGGIO:
                conteggi["attrezzaggio"] += 1
            elif riga in in_test:
                conteggi["test"] += 1
            else:
                conteggi["produzione"] += 1
                produzione.append((percorso, riga))

    errori: list[str] = []
    for nome, atteso in sorted(ATTESI.items()):
        if conteggi[nome] != atteso:
            errori.append(
                f"{nome}: {conteggi[nome]} occorrenze, attese {atteso}. "
                "Il conteggio va aggiornato nello stesso commit che lo cambia, "
                "cosi' una nuova occorrenza non passa senza essere classificata."
            )

    for percorso, riga in produzione:
        chiave = f"{percorso}:{riga}"
        if chiave not in LEGITTIME and chiave not in RESIDUI:
            errori.append(
                f"{chiave}: `WkbLimits::default()` su un percorso di produzione "
                "non censito. S5 ha portato le quote configurate fino "
                "all'inferenza: un default qui le riporterebbe indietro. "
                "Classificarlo in LEGITTIME, con la ragione, o in RESIDUI, con "
                "cio' che lo chiuderebbe."
            )

    if errori:
        for messaggio in errori:
            print(messaggio, file=sys.stderr)
        return 1

    print(
        "WkbLimits::default() censiti: "
        f"{len(LEGITTIME)} legittimi in produzione, "
        f"{len(RESIDUI)} residui dichiarati, "
        f"{conteggi['test']} nei test, "
        f"{conteggi['attrezzaggio']} nell'attrezzaggio"
    )
    for chiave, motivo in sorted(RESIDUI.items()):
        print(f"  residuo aperto — {chiave}: {motivo}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
