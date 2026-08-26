#!/usr/bin/env python3
"""Ogni campo dei metadati GeoParquet ha una prova positiva e una negativa.

# Perche' esiste

L'invariante del lotto S10 e' «GeoParquet 1.1 e' validato per **intero**». Una
validazione parziale non si distingue da una completa guardando il codice: i
campi che nessuno controlla non lasciano traccia, e a mancare e' proprio cio'
che non c'e'. Prima di questo lotto il lettore ne consultava cinque su undici, e
di `encoding` -- il campo che dice se i byte sono WKB -- non sapeva nulla.

Qui l'insieme dei campi non e' trascritto: e' **estratto dal modulo che li
legge**. Il gate confronta cio' che `metadati.rs` interroga con l'elenco delle
prove, nelle due direzioni:

* un campo che il modulo legge e nessuna prova esercita e' rosso -- e' il caso
  di un campo aggiunto senza sonde;
* un campo dichiarato qui e non letto dal modulo e' rosso -- e' il caso di una
  sonda rimasta a provare qualcosa che nessuno guarda piu'.

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

ROOT = Path(__file__).resolve().parents[1]
MODULO = ROOT / "crates" / "driver-geoparquet" / "src" / "metadati.rs"
DESCRITTORE = ROOT / "crates" / "driver-geoparquet" / "src" / "lib.rs"
CRATE = "driver-geoparquet"
PERCORSO_DELLE_SONDE = "metadati::sonde::"

# I campi che il modulo interroga, estratti da come li interroga.
LETTURA = re.compile(r'\.get\("([a-z_]+)"\)')
OBBLIGATORIA = re.compile(r'stringa_obbligatoria\(\s*\w+\s*,\s*"([a-z_]+)"')

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


# Le versioni che il validatore accetta, e quella che il catalogo dichiara.
VERSIONI = re.compile(r"VERSIONI_SUPPORTATE: \[&str; \d+\] = \[([^\]]*)\]")
DICHIARATA = re.compile(
    r"// `spec_version_supported`:(?:[^\n]*\n\s*//[^\n]*)*\n\s*Some\(\s*\"([^\"]+)\"\s*\)"
)
CITAZIONE = re.compile(r"\"([^\"]+)\"")


def versioni_accettate(testo: str) -> list[str]:
    """Le versioni che il validatore accetta, dal modulo che le applica."""
    trovato = VERSIONI.search(testo)
    return CITAZIONE.findall(trovato.group(1)) if trovato else []


def versione_dichiarata() -> str | None:
    """La versione che il descrittore pubblica nel catalogo."""
    trovato = DICHIARATA.search(DESCRITTORE.read_text(encoding="utf-8"))
    return trovato.group(1) if trovato else None


def perimetro_dichiarato(testo: str) -> list[str]:
    """Il catalogo dichiara la versione massima che il validatore applica.

    `spec_version_supported` e' un'affermazione pubblica: un consumatore la
    legge per sapere se una 2.0 sarebbe accettata. Confrontarla con l'elenco che
    il codice applica e' l'unico modo perche' sia una verita' invece che una
    riga di JSON -- ed e' la stessa disciplina della capability
    `hostile_input_hardened`.
    """
    accettate = versioni_accettate(testo)
    if not accettate:
        return [
            "non si legge `VERSIONI_SUPPORTATE` dal modulo: senza, il perimetro "
            "dichiarato nel catalogo non ha niente con cui essere confrontato"
        ]
    dichiarata = versione_dichiarata()
    if dichiarata is None:
        return [
            "il descrittore di `driver-geoparquet` non dichiara "
            "`spec_version_supported`: il perimetro che il codice applica "
            "resterebbe invisibile a chi legge il catalogo"
        ]
    massima = max(accettate)
    if dichiarata != massima:
        return [
            f"il catalogo dichiara «{dichiarata}» e il validatore accetta "
            f"{accettate}, la cui massima e' «{massima}». Un perimetro "
            "dichiarato diverso da quello applicato e' peggio di nessun "
            "perimetro: chi legge il catalogo decide su di esso."
        ]
    return []


def sorgente() -> str:
    return MODULO.read_text(encoding="utf-8")


def campi_letti(testo: str) -> set[str]:
    """I campi che il **codice di produzione** interroga davvero.

    L'estrazione si ferma a `mod sonde`, e non e' pedanteria: le sonde
    costruiscono documenti e li interrogano, quindi contarle vorrebbe dire
    lasciare che una sonda si inventi un campo -- e poi lo copra da sola. Cio'
    che il gate misura deve venire da cio' che il driver legge in produzione.

    `covering.bbox` si interroga con lo stesso nome del `bbox` di colonna: qui
    e' un campo solo, e le prove del covering stanno sotto `covering`.
    """
    produzione = testo.split("mod sonde {", 1)[0]
    return set(LETTURA.findall(produzione)) | set(OBBLIGATORIA.findall(produzione))


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
) -> tuple[list[str], dict[str, dict[str, list[str]]]]:
    """Il sorgente e' iniettabile perche' le sonde di questo gate ne
    costruiscono di finti: provare la regola su moduli inventati e' l'unico modo
    di provarla invece di provare il modulo di oggi."""
    if testo is None:
        testo = sorgente()
    letti = campi_letti(testo)
    attesi = letti | {DOCUMENTO}
    errori: list[str] = []

    if not letti:
        return (
            [
                f"{MODULO.name}: nessun campo interrogato. L'estrazione guarda "
                "`.get(\"...\")` e `stringa_obbligatoria`: se il modulo cambia "
                "forma, questo gate misura il vuoto e va rifatto, non tolto."
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
                f"la sonda «{nome}» non comincia con nessuno dei campi che il "
                f"modulo legge {sorted(attesi)}: non si sa che cosa provi."
            )
            continue
        copertura[campo][direzione].append(nome)

    errori.extend(perimetro_dichiarato(testo))

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
        f"estratti dal modulo che li legge, ciascuno con almeno una prova "
        f"positiva e una negativa, per {quante} sonde eseguite e passate. "
        f"Le versioni lette sono {versioni_accettate(sorgente())}, e il "
        f"catalogo dichiara «{versione_dichiarata()}»: oltre, il rifiuto e' di "
        "funzionalita' non supportata, non di formato."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
