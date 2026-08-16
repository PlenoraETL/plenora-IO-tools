#!/usr/bin/env python3
"""Il modello budget legacy non deve rientrare (Lotto 0 / S4.e, permanente).

Il compilatore impedisce di **usare** i tipi rimossi, ma non impedisce a un
commit futuro di reintrodurli insieme ai propri usi: un `resource.rs`
ripristinato da un revert, o un `Default` riaggiunto alle opzioni "per comodita'
dei test", compilerebbero senza che nulla protesti. Questo gate e' la rete
contro quel caso, e a differenza dell'inventario che ha sostituito non ha
tetti: qualunque occorrenza e' un errore.

Non e' l'inventario con un nome diverso. L'inventario misurava una migrazione
in corso e i suoi numeri scendevano a ogni sottopasso; questo dichiara uno
stato raggiunto e non ammette gradazioni. E' anche la forma che INV-1 chiedeva
fin dalla ratifica — "gate CI che vieta i nomi post-M4" — e che fino a S4.e non
era scrivibile, perche' quei nomi erano ancora in uso.

Come il registro dei fallback, e' una misura **sintattica**: un ritorno
scritto in una forma che le regex non riconoscono non viene visto. Serve a
impedire il rientro distratto, non a dimostrare l'impossibilita'.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# I file che non devono esistere.
FILE_VIETATI = ("crates/plenora-io-model/src/resource.rs",)

VIETATI: list[tuple[re.Pattern[str], str]] = [
    (
        # Sia `mod resource;` sia il modulo inline `mod resource { ... }`: il
        # secondo reintrodurrebbe il modello senza bisogno di ricreare il
        # file, ed e' la forma che la prima versione del gate non vedeva.
        re.compile(
            r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+resource\s*[;{]",
            re.MULTILINE,
        ),
        "il modulo `resource` e' stato eliminato in S4.e insieme al modello "
        "legacy dei contatori",
    ),
    (
        re.compile(r"\bResourceBudget\b"),
        "`ResourceBudget` e' sostituito da `OperationBudget`: i contatori "
        "cumulativi vivono nell'operazione, non in un budget parallelo",
    ),
    (
        re.compile(r"\bResourceLease\b"),
        "`ResourceLease` e' sostituito da `CountedLease` per i contatori e da "
        "`InternalMemoryLease`/`SpillLease` per le occupazioni: solo queste "
        "ultime sanno ridursi senza restituire quota, che e' cio' su cui "
        "poggia l'handoff senza finestra",
    ),
    (
        re.compile(r"\bResourceLimits\b"),
        "`ResourceLimits` e' confluito in `PipelineLimits`, dove la "
        "divergenza dal vecchio `Limits` — finding L0.2 — non e' piu' "
        "rappresentabile",
    ),
    (
        re.compile(r"\bResourceKind\b"),
        "`ResourceKind` e' sostituito da `OperationCounter` per i contatori e "
        "dai metodi del context per memoria, spill e concorrenza",
    ),
    (
        # Il tipo esatto, non `PipelineLimits` ne' `WkbLimits`: **la sola**
        # lookbehind basta, perche' esclude qualunque cosa preceda il nome
        # dentro un identificatore composto.
        #
        # La prima versione aggiungeva anche `(?!\s*::\s*wkb)`. Era un buco:
        # non escludeva nulla di legittimo — `WkbLimits` e' gia' coperto dalla
        # lookbehind — e lasciava passare un `Limits` reintrodotto con quello
        # specifico accesso, che e' proprio la forma piu' probabile in un
        # revert del vecchio `effective_wkb()`.
        re.compile(r"(?<![A-Za-z0-9_])Limits\b"),
        "il tipo `Limits` e' sostituito da `PipelineLimits`. `WkbLimits` "
        "resta: e' un tipo del contratto geometrico, non del budget",
    ),
    (
        re.compile(r"\bBudgetPayload\b"),
        "`BudgetPayload` era il ponte transitorio fra i due modelli: senza un "
        "secondo modello non ha nulla da mediare",
    ),
    (
        re.compile(r"\bfrom_legacy\b"),
        "i costruttori `from_legacy` costruivano opzioni sul modello vecchio",
    ),
    (
        re.compile(r"\blegacy_budget\b|\blegacy_limits\b"),
        "gli accessori `legacy_*` esponevano il ramo vecchio del payload",
    ),
    (
        re.compile(r"\bresource_budget\b"),
        "`resource_budget` era l'accessore al budget legacy dalle opzioni",
    ),
    (
        re.compile(
            r"impl\s+Default\s+for\s+(?:Read|Write)Options\b"
            r"|#\[derive\([^)]*\bDefault\b[^)]*\)\]\s*(?:///[^\n]*\n\s*)*"
            r"pub\s+struct\s+(?:Read|Write)Options\b"
        ),
        "`ReadOptions`/`WriteOptions` non hanno `Default`: portano un "
        "`OperationBudget`, che nasce da una costruzione che puo' fallire, e "
        "un default dovrebbe scegliere fra il panico e quote che nessun "
        "chiamante ha chiesto",
    ),
]


def sorgenti_rust(root: Path) -> list[Path]:
    """Ogni `.rs` di ogni crate piu' `fuzz/`, non solo quelli sotto `src/`.

    Un test d'integrazione, un benchmark, un `examples/` o un `build.rs` sono
    posti dove si scrive codice di servizio con meno attenzione, ed e' li' che
    un tipo rimosso rientrerebbe piu' facilmente.
    """
    trovati: list[Path] = []
    for radice in (root / "crates", root / "fuzz"):
        if not radice.is_dir():
            continue
        for sorgente in sorted(radice.rglob("*.rs")):
            if "target" in sorgente.relative_to(radice).parts:
                continue
            trovati.append(sorgente)
    return trovati


def spoglia(sorgente: str) -> str:
    """Rimuove commenti e stringhe, sostituendoli con spazi.

    Un commento che spiega **perche'** un tipo e' stato rimosso ne nomina il
    nome, ed e' esattamente cio' che questo file fa in ogni sua riga. Senza lo
    spoglio, un gate del genere vieterebbe di documentare la propria ragione.
    """
    fuori: list[str] = []
    i = 0
    n = len(sorgente)
    while i < n:
        c = sorgente[i]
        due = sorgente[i : i + 2]
        if due == "//":
            j = sorgente.find("\n", i)
            j = n if j == -1 else j
            fuori.append(" " * (j - i))
            i = j
        elif due == "/*":
            profondita = 1
            j = i + 2
            while j < n and profondita:
                if sorgente[j : j + 2] == "/*":
                    profondita += 1
                    j += 2
                elif sorgente[j : j + 2] == "*/":
                    profondita -= 1
                    j += 2
                else:
                    j += 1
            fuori.append("".join(" " if ch != "\n" else "\n" for ch in sorgente[i:j]))
            i = j
        elif c == "r" and sorgente[i + 1 : i + 2] in ('"', "#"):
            m = re.match(r'r(#*)"', sorgente[i:])
            if not m:
                fuori.append(c)
                i += 1
                continue
            chiusura = '"' + m.group(1)
            j = sorgente.find(chiusura, i + m.end())
            j = n if j == -1 else j + len(chiusura)
            fuori.append("".join(" " if ch != "\n" else "\n" for ch in sorgente[i:j]))
            i = j
        elif c == '"':
            j = i + 1
            while j < n:
                if sorgente[j] == "\\":
                    j += 2
                    continue
                if sorgente[j] == '"':
                    j += 1
                    break
                j += 1
            fuori.append("".join(" " if ch != "\n" else "\n" for ch in sorgente[i:j]))
            i = j
        else:
            fuori.append(c)
            i += 1
    return "".join(fuori)


def main() -> int:
    errori: list[str] = []

    for relativo in FILE_VIETATI:
        if (ROOT / relativo).exists():
            errori.append(
                f"{relativo}: il file e' stato eliminato in S4.e e non deve "
                "tornare."
            )

    for sorgente in sorgenti_rust(ROOT):
        testo = spoglia(sorgente.read_text(encoding="utf-8"))
        percorso = sorgente.relative_to(ROOT).as_posix()
        for modello, motivo in VIETATI:
            for trovato in modello.finditer(testo):
                riga = testo.count("\n", 0, trovato.start()) + 1
                nome = trovato.group(0).strip().splitlines()[0]
                errori.append(f"{percorso}:{riga}: `{nome}` — {motivo}.")

    if errori:
        print(
            "Il modello budget legacy e' rientrato. Rimosso in S4.e, non ha "
            "un percorso di ritorno supportato:",
            file=sys.stderr,
        )
        for messaggio in errori:
            print(f"  {messaggio}", file=sys.stderr)
        return 1

    print("nessun residuo del modello budget legacy")
    return 0


if __name__ == "__main__":
    sys.exit(main())
