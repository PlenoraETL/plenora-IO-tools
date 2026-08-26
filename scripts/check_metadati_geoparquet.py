#!/usr/bin/env python3
"""Ogni campo dei metadati GeoParquet ha una prova positiva e una negativa.

# Perche' esiste

L'invariante del lotto S10 e' «GeoParquet 1.1 e' validato per **intero**». Una
validazione parziale non si distingue da una completa guardando il codice: i
campi che nessuno controlla non lasciano traccia, e a mancare e' proprio cio'
che non c'e'. Prima di questo lotto il lettore ne consultava cinque su undici, e
di `encoding` -- il campo che dice se i byte sono WKB -- non sapeva nulla.

Qui l'insieme dei campi viene **dagli schemi ufficiali fissati**, e da nessun
altro posto.

La prima stesura lo estraeva dal modulo che doveva controllare -- da cio' che
`metadati.rs` interrogava con `.get("...")`. Era circolare: un campo che il
driver dimenticava non era una lacuna, diventava la definizione del perimetro,
e il gate certificava «coperto per intero» un insieme scelto dall'imputato. Il
controllo vecchio non e' rimasto accanto a quello nuovo: sarebbe stato una
seconda definizione concorrente, e due definizioni della stessa cosa divergono.

Il confronto va nelle due direzioni:

* un campo dello schema che nessuna prova esercita e' rosso -- ed e' il caso che
  la versione circolare non poteva vedere, perche' un campo non letto non
  entrava nemmeno nel suo elenco;
* una sonda che nomina qualcosa che negli schemi non c'e' e' rossa.

Per ogni campo servono **entrambi i versi**: una prova che accetta e una che
rifiuta. Solo la seconda direbbe che il validatore rifiuta tutto; solo la prima,
che accetta tutto.

# Come si riconosce il verso

Dal nome della sonda, e non da una tabella scritta a parte: una tabella
diverge, un nome no. La convenzione e' chiusa e verificata --

    <campo>_..._e_accettato | _sono_accettati | _e_accettata     positiva
    <campo>_..._e_non_conforme | _non_supportata | _non_supportati  negativa

Una sonda che non dichiara il proprio verso nel nome e' rossa: e' il modo in cui
una sonda smette di provare cio' che il suo nome promette senza che nessuno se
ne accorga.

# E poi le esegue

Un riferimento testuale non e' una prova. Le sonde vengono lanciate per nome con
`--exact`, e ognuna deve comparire una volta sola e passare.

# Uso

    python3 scripts/check_metadati_geoparquet.py
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

# Eseguire test nominati e pretendere che ognuno passi una volta sola e' gia'
# scritto e provato altrove: riscriverlo qui vorrebbe dire avere due
# implementazioni della stessa regola.
from check_prove_di_confine import esegui as esegui_i_test  # noqa: E402

# Gli schemi fissati, e la loro verifica, vivono nel gate che li custodisce:
# importarli evita di avere due letture dello stesso lock.
import check_schemi_geoparquet as schemi  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
MODULO = ROOT / "crates" / "driver-geoparquet" / "src" / "metadati.rs"
CRATE = "driver-geoparquet"
PERCORSO_DELLE_SONDE = "metadati::sonde::"

# Il documento non e' un campo del documento, e ha comunque bisogno delle due
# prove: `geo` che non e' JSON, o non e' un oggetto, e' il primo modo in cui un
# metadato puo' essere sbagliato -- ed era quello che veniva ignorato.
DOCUMENTO = "documento"

# I suffissi che dichiarano il verso di una sonda. L'elenco e' chiuso.
POSITIVI = ("_e_accettato", "_e_accettata", "_sono_accettati", "_sono_accettate")
NEGATIVI = (
    "_e_non_conforme",
    "_non_supportata",
    "_non_supportato",
    "_non_supportati",
    "_non_e_esprimibile",
)

# Un `#[test]` e la sua funzione, dentro `mod sonde`.
SONDA = re.compile(r"#\[test\]\s*\n\s*fn ([a-z_0-9]+)\(\)")



def sorgente() -> str:
    return MODULO.read_text(encoding="utf-8")


def campi_dello_schema() -> tuple[set[str], list[str]]:
    """I campi che la **specifica** definisce, dagli schemi fissati.

    Sono i tre obbligatori del documento e le proprieta' dell'oggetto-colonna,
    presi dall'unione delle due versioni: un campo che esiste solo in 1.1 --
    `covering` -- va comunque provato.

    Gli schemi arrivano dal gate che ne verifica lock, byte, sha256, `$id`,
    draft e `$ref`: se quelli non tornano, qui non si arriva.
    """
    documenti, errori = schemi.schemi_fissati()
    if errori:
        return set(), errori

    campi: set[str] = set()
    for chiave, documento in documenti.items():
        if not chiave.startswith("geoparquet-"):
            continue
        campi.update(documento.get("required", []))
        campi.update(schemi.colonna_dello_schema(documento)["properties"])
    return campi, []


def sonde(testo: str) -> list[str]:
    """Le sonde dichiarate nel modulo, in ordine."""
    principio = testo.find("mod sonde {")
    if principio == -1:
        return []
    return SONDA.findall(testo[principio:])


def verso(nome: str) -> str | None:
    """Positiva, negativa, o nessuna delle due."""
    if nome.endswith(POSITIVI):
        return "positiva"
    if nome.endswith(NEGATIVI):
        return "negativa"
    return None


def campo_di(nome: str, campi: set[str]) -> str | None:
    """Il campo che la sonda esercita, dal prefisso piu' lungo che combacia."""
    candidati = [c for c in campi if nome.startswith(f"{c}_")]
    return max(candidati, key=len) if candidati else None


def verifica(
    testo: str | None = None,
    campi: set[str] | None = None,
) -> tuple[list[str], dict[str, dict[str, list[str]]]]:
    """Il sorgente e il perimetro sono iniettabili perche' le sonde di questo
    gate ne costruiscono di finti: provare la regola su moduli inventati e'
    l'unico modo di provarla invece di provare il modulo di oggi."""
    if testo is None:
        testo = sorgente()
    errori: list[str] = []
    if campi is None:
        campi, errori = campi_dello_schema()
        if errori:
            return errori, {}
    attesi = campi | {DOCUMENTO}

    if not campi:
        return (
            [
                "nessun campo estratto dagli schemi ufficiali: senza perimetro "
                "questo gate misura il vuoto, e va rifatto invece che tolto."
            ],
            {},
        )

    copertura: dict[str, dict[str, list[str]]] = {
        campo: {"positiva": [], "negativa": []} for campo in sorted(attesi)
    }

    for nome in sonde(testo):
        direzione = verso(nome)
        if direzione is None:
            errori.append(
                f"la sonda «{nome}» non dichiara il proprio verso nel nome. "
                f"I suffissi ammessi sono {list(POSITIVI + NEGATIVI)}: un nome "
                "che non dice se accetta o rifiuta lascia smettere di provare "
                "senza che nessuno se ne accorga."
            )
            continue
        campo = campo_di(nome, attesi)
        if campo is None:
            errori.append(
                f"la sonda «{nome}» non comincia con nessuno dei campi che gli "
                f"schemi ufficiali definiscono {sorted(attesi)}: non si sa che "
                "cosa provi."
            )
            continue
        copertura[campo][direzione].append(nome)

    for campo, versi in copertura.items():
        for direzione in ("positiva", "negativa"):
            if not versi[direzione]:
                errori.append(
                    f"il campo «{campo}» non ha una prova {direzione}. Con la "
                    "sola negativa il gate direbbe che il validatore rifiuta "
                    "tutto; con la sola positiva, che accetta tutto."
                )
    return errori, copertura


def main() -> int:
    errori, copertura = verifica()
    if not errori:
        nomi = tuple(
            f"{PERCORSO_DELLE_SONDE}{nome}"
            for versi in copertura.values()
            for direzione in ("positiva", "negativa")
            for nome in versi[direzione]
        )
        errori.extend(esegui_i_test(CRATE, nomi, "la prova dei metadati GeoParquet"))

    for messaggio in errori:
        print(messaggio, file=sys.stderr)
    if errori:
        return 1

    quante = sum(len(v["positiva"]) + len(v["negativa"]) for v in copertura.values())
    print(
        f"metadati GeoParquet validati per intero: {len(copertura)} campi "
        f"estratti **dagli schemi ufficiali fissati**, ciascuno con almeno una prova "
        f"positiva e una negativa, per {quante} sonde eseguite e passate. "
        "Il perimetro di versione dichiarato nel catalogo e' verificato dal "
        "gate degli schemi, che lo confronta con l'autorita' e non con il codice."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
