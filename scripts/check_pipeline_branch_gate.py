#!/usr/bin/env python3
"""Il ramo Pipeline non e' utilizzabile prima dell'handoff reale (S4.b.3).

`ReadOptions::from_read_parts` esiste dal commit S4.b ed e' coperto da test,
ma finche' il **percorso comune** — l'adapter di lettura e lo `StagedSpool` —
prenota memoria con la `ResourceLease` del modello legacy, un driver che
costruisse opzioni sul ramo `Pipeline` otterrebbe un oggetto formalmente
corretto e un comportamento a meta': i contatori dal modello nuovo, la memoria
dei batch da quello vecchio, e la finestra non contabilizzata che
`InternalMemoryLease::shrink_to` chiude resterebbe aperta proprio sul percorso
che dovrebbe averla chiusa.

"Costruibile" non e' "utilizzabile". Questo gate tiene separate le due cose:
finche' l'handoff non e' cablato e dimostrato da un test end-to-end, nessun
crate fuori da `plenora-io-core` puo' costruire opzioni `Pipeline`.

Il gate si disattiva da solo quando le condizioni sono soddisfatte, e va
rimosso insieme al ponte in S4.e.

## Cosa conta come "soddisfatto"

Le condizioni descrivono **responsabilita' distinte**, non la presenza degli
stessi simboli in due file. Chiedere `shrink_to` in entrambi spingerebbe verso
una seconda chiamata inutile nello spool — o, peggio, verso un commento messo
li' per accontentare il gate:

* `reader_adapters.rs` **acquisisce** la lease, misura l'ingombro, chiama
  davvero `.shrink_to(...)` e la **trasferisce per move** allo spool;
* `spool.rs` **riceve e custodisce** la lease, e non ne acquisisce una
  seconda: e' la seconda acquisizione che riaprirebbe la doppia
  contabilizzazione;
* entrambi liberi da `ResourceLease`;
* il test end-to-end dimostra assenza di finestra e assenza di bridge.

## Perche' il sorgente viene spogliato

Le regex girano su un testo da cui commenti e stringhe sono stati rimossi. Un
`/* #[test] fn handoff_reale... */`, o una stringa che nomini
`InternalMemoryLease`, non descrivono codice che gira: la prima versione del
gate si sarebbe sbloccata su entrambi, e i commenti scritti per spiegare cosa
mancava erano proprio di quella forma.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

CORE = ROOT / "crates" / "plenora-io-core" / "src"

ADAPTER = "driver/reader_adapters.rs"
SPOOL = "driver/spool.rs"

TEST_HANDOFF = "handoff_reale_della_memoria_senza_bridge_legacy"

DEFINIZIONE_TEST = re.compile(
    r"#\[test\]\s*(?:#\[[^\]]*\]\s*)*(?:async\s+)?fn\s+" + re.escape(TEST_HANDOFF) + r"\s*\(",
    re.MULTILINE,
)

COSTRUTTORI_PIPELINE = re.compile(r"::from_(?:read|write)_parts\b")

# L'adapter deve chiamare `shrink_to` su qualcosa, non nominarlo.
CHIAMATA_SHRINK = re.compile(r"\.shrink_to\s*\(")
# ...e cedere la lease per move a `push`. La forma esatta dell'argomento non
# e' imposta: si richiede che `push` riceva una lease, non un nome preciso.
ACQUISIZIONE_LEASE = re.compile(r"\.lease_memory_internal\s*\(")


def spoglia(sorgente: str) -> str:
    """Rimuove commenti e stringhe, sostituendoli con spazi.

    Preserva la lunghezza e le andate a capo cosi' che le regex multilinea
    continuino a comportarsi come sul testo originale. Non e' un parser di
    Rust: non gestisce le raw string con cancelletti multipli oltre il primo
    livello, che in questo repository non compaiono nei file esaminati.
    """
    fuori = []
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
            # I block comment di Rust annidano.
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
        elif c == 'r' and sorgente[i + 1 : i + 2] in ('"', "#"):
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


def senza_moduli_di_test(codice: str) -> str:
    """Rimuove i blocchi `#[cfg(test)] mod ... { ... }`.

    Le regole di responsabilita' descrivono il **percorso di produzione**. Un
    helper di test che imita l'adapter acquisisce legittimamente una lease:
    contarlo come violazione spingerebbe a scrivere test che non passano piu'
    dall'API reale, che e' il difetto opposto a quello che il gate previene.
    """
    fuori = []
    i = 0
    n = len(codice)
    marcatore = "#[cfg(test)]"
    while i < n:
        j = codice.find(marcatore, i)
        if j == -1:
            fuori.append(codice[i:])
            break
        apertura = codice.find("{", j)
        if apertura == -1:
            fuori.append(codice[i:])
            break
        # Solo i moduli: un `#[cfg(test)]` su una funzione o una `impl` non
        # introduce un blocco di test da ignorare in blocco.
        if "mod " not in codice[j:apertura]:
            fuori.append(codice[i : j + len(marcatore)])
            i = j + len(marcatore)
            continue
        profondita = 0
        k = apertura
        while k < n:
            if codice[k] == "{":
                profondita += 1
            elif codice[k] == "}":
                profondita -= 1
                if profondita == 0:
                    k += 1
                    break
            k += 1
        fuori.append(codice[i:j])
        fuori.append("".join("\n" if ch == "\n" else " " for ch in codice[j:k]))
        i = k
    return "".join(fuori)


def codice(relativo: str, con_test: bool = False) -> str | None:
    sorgente = CORE / relativo
    if not sorgente.is_file():
        return None
    spogliato = spoglia(sorgente.read_text(encoding="utf-8"))
    return spogliato if con_test else senza_moduli_di_test(spogliato)


def handoff_completato() -> tuple[bool, list[str]]:
    """Le responsabilita' che rendono il ramo Pipeline realmente utilizzabile."""
    mancanti: list[str] = []

    adapter = codice(ADAPTER)
    spool = codice(SPOOL)

    for nome, testo in ((ADAPTER, adapter), (SPOOL, spool)):
        if testo is None:
            mancanti.append(f"{nome}: assente")
            continue
        if "ResourceLease" in testo:
            mancanti.append(f"{nome}: prenota ancora con `ResourceLease` (modello legacy)")
        if "InternalMemoryLease" not in testo:
            mancanti.append(f"{nome}: non maneggia `InternalMemoryLease`")

    # L'adapter e' il titolare: acquisisce, riduce, cede.
    if adapter is not None:
        if not ACQUISIZIONE_LEASE.search(adapter):
            mancanti.append(
                f"{ADAPTER}: non acquisisce la lease con `lease_memory_internal`, "
                "quindi non e' lui a possedere la memoria del batch"
            )
        if not CHIAMATA_SHRINK.search(adapter):
            mancanti.append(
                f"{ADAPTER}: non chiama `.shrink_to(...)`. Senza la riduzione a "
                "grandezza nota il trasferimento riaprirebbe la finestra che "
                "deve chiudere"
            )

    # Lo spool custodisce: non deve acquisire una seconda lease.
    if spool is not None:
        if ACQUISIZIONE_LEASE.search(spool):
            mancanti.append(
                f"{SPOOL}: acquisisce una propria lease con `lease_memory_internal`. "
                "Deve ricevere quella gia' ridotta dall'adapter: una seconda "
                "acquisizione contabilizza due volte lo stesso batch"
            )
        if CHIAMATA_SHRINK.search(spool):
            mancanti.append(
                f"{SPOOL}: chiama `.shrink_to(...)`. La riduzione appartiene a chi "
                "ha misurato il batch, cioe' l'adapter: qui sarebbe una seconda "
                "riduzione su una prenotazione gia' esatta"
            )

    sorgenti_core = list(CORE.glob("**/*.rs"))
    testo_core = "\n".join(spoglia(s.read_text(encoding="utf-8")) for s in sorgenti_core)
    if not DEFINIZIONE_TEST.search(testo_core):
        mancanti.append(
            f"plenora-io-core: manca la **definizione** `#[test] fn {TEST_HANDOFF}`. "
            "Citarne il nome in un commento o in una stringa non e' una "
            "dimostrazione."
        )

    return (not mancanti), mancanti


def costruttori_fuori_dal_core() -> list[str]:
    trovati: list[str] = []
    for crate, sorgente in sorgenti_rust(ROOT):
        if crate == "plenora-io-core":
            continue
        if COSTRUTTORI_PIPELINE.search(spoglia(sorgente.read_text(encoding="utf-8"))):
            trovati.append(sorgente.relative_to(ROOT).as_posix())
    return trovati


def main() -> int:
    completato, mancanti = handoff_completato()
    consumatori = costruttori_fuori_dal_core()

    if completato:
        print(
            "handoff reale completato: l'adapter acquisisce, riduce e cede la "
            "lease; lo spool la custodisce senza riacquisirla. Il ramo Pipeline "
            "e' dichiarabile utilizzabile: rimuovere questo gate con il ponte (S4.e)"
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
        return 1

    print(
        "handoff reale non ancora cablato: nessun crate fuori da "
        "plenora-io-core costruisce opzioni Pipeline, come atteso prima di S4.d"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
