"""Gli avvisi di licenza di ogni crate linkato nel binario, crate per crate.

# Perche' non basta il testo per identificatore

Gli artefatti portavano un testo per identificatore SPDX
(`LICENSES/crate-rust/MIT.txt`, `Apache-2.0.txt`...): il testo **generico**.
MIT e BSD chiedono di includere l'avviso di copyright **di quel crate**, e
Apache-2.0 il suo `NOTICE` se ne ha uno. Un testo canonico non porta ne' l'uno
ne' l'altro.

Qui ogni crate di terzi linkato nel binario porta i propri file: quelli che il
suo pacchetto contiene (`LICENSE*`, `LICENCE*`, `COPYING*`, `NOTICE*`,
`COPYRIGHT*`, `UNLICENSE*` alla radice del pacchetto, piu' il `license-file` del
manifesto), copiati da `cargo metadata` -- cioe' dai sorgenti che cargo ha
verificato col checksum di `Cargo.lock` -- in
`LICENSES/crate-rust/<nome>-<versione>/`.

Per i crate il cui pacchetto non ne porta, il testo viene da
`assurance/registries/avvisi-crate.json`: preso dal repository upstream al
commit di `.cargo_vcs_info.json`, versionato in `assurance/licenze-crate/` e
verificato con lo sha256. Un crate senza alcun testo upstream e' elencato
nello stesso registro, con il perche', e porta un avviso generato dal suo
manifesto.

# Il gate

`python3 scripts/avvisi_crate.py --profilo base|filegdb` fallisce se un crate
linkato non ha un avviso -- ne' nel pacchetto, ne' nel registro -- se un testo
del registro non ha l'impronta registrata, o se una voce del registro non serve
piu' (un registro che accetta voci obsolete descriverebbe un altro grafo).
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import pathlib
import re
import sys

RADICE = pathlib.Path(__file__).resolve().parents[1]
REGISTRO = RADICE / "assurance" / "registries" / "avvisi-crate.json"
TESTI = RADICE / "assurance" / "licenze-crate"
NOMI = re.compile(r"^(LICEN[CS]E|COPYING|NOTICE|COPYRIGHT|UNLICENSE)", re.IGNORECASE)
AVVISO_GENERATO = "AVVISO-PLENORA.txt"


def _impronta(dati: bytes) -> str:
    return hashlib.sha256(dati.replace(b"\r\n", b"\n")).hexdigest()


def _dipendenze_rust():
    spec = importlib.util.spec_from_file_location(
        "dipendenze_rust", RADICE / "scripts" / "dipendenze-rust.py"
    )
    modulo = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(modulo)
    return modulo


def registro() -> dict:
    return json.loads(REGISTRO.read_text(encoding="utf-8"))


def file_del_pacchetto(cartella: pathlib.Path, license_file: str) -> list[pathlib.Path]:
    """I file di licenza che il pacchetto del crate porta, in ordine stabile."""
    trovati = {
        voce for voce in cartella.iterdir() if voce.is_file() and NOMI.match(voce.name)
    }
    if license_file:
        dichiarato = (cartella / license_file).resolve()
        if dichiarato.is_file():
            trovati.add(dichiarato)
    return sorted(trovati, key=lambda p: p.name)


def avvisi(
    crate: list[dict], destinazione: pathlib.Path | None = None
) -> tuple[list[dict], list[str], set[tuple[str, str]]]:
    """Gli avvisi di ogni crate di terzi, ed eventualmente la loro copia.

    Ogni elemento di `crate` e' quello di `dipendenze-rust.linkati`, con
    `cartella`. Ritorna, per crate, i file con il loro sha256 e la fonte, e gli
    errori. Con `destinazione` scrive i file in `<destinazione>/<nome>-<versione>/`.
    """
    dati = registro()
    dal_registro: dict[tuple[str, str], list[dict]] = {}
    for voce in dati["testi"]:
        dal_registro.setdefault((voce["crate"], voce["versione"]), []).append(voce)
    senza_testo = {(v["crate"], v["versione"]): v for v in dati["senza_testo_upstream"]}
    usate: set[tuple[str, str]] = set()
    risultato: list[dict] = []
    errori: list[str] = []

    for componente in crate:
        if componente["nostro"]:
            continue
        chiave = (componente["nome"], componente["versione"])
        etichetta = f"{componente['nome']}-{componente['versione']}"
        cartella = pathlib.Path(componente["cartella"])
        copiati: list[dict] = []
        sorgenti: list[tuple[str, bytes, str]] = []
        for percorso in file_del_pacchetto(cartella, componente.get("licenza_file", "")):
            sorgenti.append((percorso.name, percorso.read_bytes(), "pacchetto"))
        if not sorgenti and chiave in dal_registro:
            usate.add(chiave)
            for voce in dal_registro[chiave]:
                testo = TESTI / etichetta / voce["file"]
                if not testo.is_file():
                    errori.append(f"{etichetta}: il registro dichiara {voce['file']} e il testo non c'e'")
                    continue
                contenuto = testo.read_bytes()
                if _impronta(contenuto) != voce["sha256"]:
                    errori.append(f"{etichetta}: {voce['file']} non ha l'impronta registrata")
                    continue
                sorgenti.append((voce["file"], contenuto, voce["origine"]))
        elif sorgenti and chiave in dal_registro:
            errori.append(
                f"{etichetta}: il pacchetto porta i propri testi e il registro ne dichiara "
                "altri: la voce del registro e' obsoleta"
            )
            usate.add(chiave)
        if not sorgenti and chiave in senza_testo:
            usate.add(chiave)
            voce = senza_testo[chiave]
            generato = (
                f"{componente['nome']} {componente['versione']}\n"
                f"autori dichiarati nel manifesto: {voce['autori']}\n"
                f"licenza dichiarata nel manifesto: {voce['licenza']}\n\n"
                f"{voce['perche']}\n"
            ).encode("utf-8")
            sorgenti.append((AVVISO_GENERATO, generato, "generato dal manifesto del crate"))
        if not sorgenti:
            errori.append(
                f"{etichetta} ({componente['licenza']}): nessun avviso di licenza, ne' nel "
                "pacchetto ne' in assurance/registries/avvisi-crate.json"
            )
            continue
        for nome, contenuto, fonte in sorgenti:
            if destinazione is not None:
                uscita = destinazione / etichetta / nome
                uscita.parent.mkdir(parents=True, exist_ok=True)
                uscita.write_bytes(contenuto)
            copiati.append({"file": nome, "sha256": _impronta(contenuto), "fonte": fonte})
        risultato.append({"nome": componente["nome"], "versione": componente["versione"], "avvisi": copiati})
    return risultato, errori, usate


def pacchetti_con_avvisi(di_terzi: list[dict], destinazione: pathlib.Path) -> list[dict]:
    """Scrive gli avvisi in `destinazione` e rende le voci di `CRATE.json`.

    E' cio' che i due costruttori chiamano: ogni voce e' quella di `linkati`
    senza `cartella` (un percorso della macchina di chi costruisce) e con
    `avvisi`, i file e il loro sha256. Un crate senza avviso ferma la
    costruzione: un artefatto non si spedisce con un crate senza il suo.
    """
    per_crate, errori, _ = avvisi(di_terzi, destinazione)
    if errori:
        raise SystemExit(
            "crate linkati senza il proprio avviso di licenza:\n  " + "\n  ".join(errori)
        )
    per_chiave = {(v["nome"], v["versione"]): v["avvisi"] for v in per_crate}
    return [
        {
            **{k: v for k, v in c.items() if k != "cartella"},
            "avvisi": per_chiave[(c["nome"], c["versione"])],
        }
        for c in di_terzi
        if not c["nostro"]
    ]


def verifica(profili: list[str]) -> list[str]:
    """Il gate: ogni crate di ogni profilo coperto, e nessuna voce obsoleta."""
    dipendenze = _dipendenze_rust()
    errori: list[str] = []
    usate_in_tutti: set[tuple[str, str]] = set()
    for profilo in profili:
        crate = dipendenze.linkati(dipendenze.grafo(profilo))
        _, errori_del_profilo, usate = avvisi(crate)
        errori += [f"[{profilo}] {e}" for e in errori_del_profilo]
        usate_in_tutti |= usate
    dati = registro()
    dichiarate = {(v["crate"], v["versione"]) for v in dati["testi"]} | {
        (v["crate"], v["versione"]) for v in dati["senza_testo_upstream"]
    }
    for nome, versione in sorted(dichiarate - usate_in_tutti):
        errori.append(
            f"{nome}-{versione}: voce del registro che nessun crate linkato usa (obsoleta, o "
            "il pacchetto porta ora i propri testi)"
        )
    cartelle = {p.name for p in TESTI.iterdir() if p.is_dir()} if TESTI.is_dir() else set()
    attese = {f"{v['crate']}-{v['versione']}" for v in dati["testi"]}
    for estranea in sorted(cartelle - attese):
        errori.append(f"assurance/licenze-crate/{estranea}: testi che il registro non dichiara")
    return errori


def main() -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument(
        "--profilo", action="append", choices=["base", "filegdb"], default=None
    )
    opzioni = argomenti.parse_args()
    profili = opzioni.profilo or ["base", "filegdb"]
    errori = verifica(profili)
    if errori:
        print("avvisi dei crate: falliti")
        for errore in errori:
            print(f"  - {errore}")
        return 1
    print(f"avvisi dei crate verificati per {', '.join(profili)}: ogni crate linkato ha il proprio")
    return 0


if __name__ == "__main__":
    sys.exit(main())
