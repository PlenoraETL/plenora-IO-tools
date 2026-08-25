#!/usr/bin/env python3
"""La capability `hostile_input_hardened` dice il vero, o e' un booleano.

# Che cosa afferma la capability

Che ogni testo che il driver interpreta come geometria passi da un'analisi che
applica i tetti del bordo -- byte, componenti, profondita' -- **mentre**
consuma, e non dopo aver costruito l'albero. E' la garanzia del lotto S12, ed e'
dichiarata nel catalogo perche' un consumatore possa verificarla senza leggere
il nostro codice.

`false` non dice «insicuro»: dice **non dichiarato**. Un driver che legge un
formato binario ha altre difese, e riassumerle in un booleano solo lo renderebbe
inutile.

# Perche' un gate

Una capability e' un'affermazione che il driver fa su se stesso. Senza un
confronto con il codice, il modo piu' semplice di ottenerla e' scriverla: un
`true` costa un carattere, e nessun test esistente lo smentirebbe -- il
descrittore compila comunque, il catalogo si serializza comunque.

Qui la dichiarazione viene confrontata con cio' che il driver **attraversa**:
i due entry point che il lotto S12 ha reso progressivi. Chi dichiara `true`
senza chiamarli e' rosso; chi li chiama senza dichiararlo pure -- la seconda
direzione conta quanto la prima, perche' una garanzia che c'e' e non e'
dichiarata e' una garanzia che nessuno usa.

# Uso

    python3 scripts/check_capability_input_ostile.py
"""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CRATES = ROOT / "crates"

# Gli entry point che applicano i tetti **durante** il parse. Sono due, e sono
# i soli: aggiungerne uno qui senza averlo reso progressivo sarebbe il modo di
# far passare un driver che non lo e'.
INGRESSI_PROGRESSIVI = (
    # WKT: `driver_common::wkt_progressivo` dietro il confine pubblico.
    "parse_wkt_bounded",
    # GeoJSON: la deserializzazione che addebita mentre serde consegna.
    "geometria_progressiva::analizza",
)

# Il commento con cui un driver apre la propria dichiarazione. Il valore sta
# nella prima riga non commentata che segue: leggerlo riga per riga invece che
# con un'espressione regolare evita il backtracking, ed e' anche piu' facile da
# leggere di un pattern che deve saltare un numero qualunque di commenti.
APERTURA = "`hostile_input_hardened`:"


def crate_dei_driver(radice: Path) -> list[str]:
    """I driver, che sono le crate che **costruiscono un descrittore**.

    Non quelle il cui nome comincia per `driver-`: `driver-common` e' codice
    condiviso e non dichiara niente al catalogo. Derivare l'elenco dal
    descrittore invece che dal nome fa entrare da solo un driver nuovo, e non
    fa entrare una libreria che si chiama come loro.
    """
    trovati = []
    for percorso in sorted((radice / "crates").glob("*/src/lib.rs")):
        if "FormatDescriptor::const_new(" in percorso.read_text(encoding="utf-8"):
            trovati.append(percorso.parent.parent.name)
    return trovati


def _sorgenti(radice: Path, crate: str) -> str:
    """Tutto il codice della crate, meno i suoi test.

    I test chiamano gli entry point per provarli, ed e' il loro mestiere:
    contarli come uso di produzione direbbe che un driver e' irrigidito perche'
    una sonda lo esercita.
    """
    pezzi = []
    for percorso in sorted((radice / "crates" / crate / "src").rglob("*.rs")):
        testo = percorso.read_text(encoding="utf-8")
        principio = testo.find("mod tests {")
        if principio == -1:
            principio = testo.find("mod sonde {")
        pezzi.append(testo if principio == -1 else testo[:principio])
    return "\n".join(pezzi)


def dichiarato(radice: Path, crate: str) -> bool | None:
    """Il valore che il driver scrive nel proprio descrittore."""
    testo = (radice / "crates" / crate / "src" / "lib.rs").read_text(encoding="utf-8")
    righe = testo.splitlines()
    for indice, riga in enumerate(righe):
        if APERTURA not in riga:
            continue
        for seguente in righe[indice + 1 :]:
            nuda = seguente.strip()
            if nuda.startswith("//") or not nuda:
                continue
            if nuda in ("true,", "false,"):
                return nuda == "true,"
            break
    return None


def osservato(radice: Path, crate: str) -> bool:
    """Il driver attraversa davvero un ingresso progressivo?"""
    sorgenti = _sorgenti(radice, crate)
    return any(ingresso in sorgenti for ingresso in INGRESSI_PROGRESSIVI)


def verifica(radice: Path) -> list[str]:
    errori: list[str] = []
    for crate in crate_dei_driver(radice):
        detto = dichiarato(radice, crate)
        visto = osservato(radice, crate)
        if detto is None:
            errori.append(
                f"{crate}: la capability `hostile_input_hardened` non e' "
                "dichiarata con la sua ragione. Il descrittore la porta comunque "
                "-- e' un campo obbligatorio -- ma senza il commento che la "
                "motiva nessuno sa perche' vale quel che vale."
            )
            continue
        if detto and not visto:
            errori.append(
                f"{crate}: dichiara `hostile_input_hardened = true` e non chiama "
                f"nessuno di {list(INGRESSI_PROGRESSIVI)}. Un `true` costa un "
                "carattere; la garanzia costa un parser."
            )
        if visto and not detto:
            errori.append(
                f"{crate}: attraversa un ingresso progressivo e dichiara "
                "`hostile_input_hardened = false`. Una garanzia che c'e' e non e' "
                "dichiarata e' una garanzia che nessun consumatore puo' usare."
            )
    return errori


def main() -> int:
    errori = verifica(ROOT)
    for messaggio in errori:
        print(messaggio, file=sys.stderr)
    if errori:
        return 1
    quanti = crate_dei_driver(ROOT)
    irrigiditi = [c for c in quanti if dichiarato(ROOT, c)]
    print(
        f"capability `hostile_input_hardened` verificata su {len(quanti)} driver: "
        f"{len(irrigiditi)} la dichiarano ({', '.join(irrigiditi)}), e ognuno "
        "attraversa davvero un'analisi che applica i tetti durante il parse"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
