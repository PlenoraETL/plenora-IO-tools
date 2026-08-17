#!/usr/bin/env python3
"""Impedisce che i pin di toolchain divergano fra i loro consumatori (INFRA-0).

Quattro pin — stable, nightly, `cargo-fuzz`, `cargo-llvm-cov` — sono replicati
come letterali in sei posti: `rust-toolchain.toml`, `Dockerfile.dev`,
`fuzz/Dockerfile`, il workflow CI e i due script di fuzzing. La duplicazione e'
voluta: un Dockerfile non legge un file fuori dal contesto di build senza
`--build-arg`, e una CI che facesse `source` di un env file renderebbe i pin
invisibili a chi legge il workflow.

Il rischio della duplicazione e' che un aggiornamento ne tocchi cinque su sei.
Il caso peggiore non e' un errore di build, che si vedrebbe subito: e' che il
fuzzing giri con un nightly diverso da quello dell'immagine di sviluppo, o la
copertura con un `cargo-llvm-cov` diverso da quello della CI. La percentuale
che il gate confronta con la soglia dipende da come lo strumento aggrega le
regioni; una divergenza li' produce due misure entrambe plausibili e diverse,
ed e' esattamente il genere di differenza che si scopre tardi.

## Come misura

Per ogni pin il gate fa **due** verifiche, e servono entrambe:

* **presenza** — ogni consumatore obbligatorio deve dichiarare il pin. Senza
  questa, cancellare la riga farebbe passare il gate.
* **esclusivita'** — nessun valore *della stessa famiglia* diverso da quello
  canonico puo' comparire nei file sorvegliati. Senza questa, aggiungere un
  secondo nightly accanto a quello giusto passerebbe.

La seconda e' il motivo per cui la famiglia e' descritta da un pattern
(`nightly-AAAA-MM-GG`, `--version X.Y.Z` accanto al nome dello strumento) e non
da una ricerca del valore atteso: cercare cio' che ci si aspetta trova sempre
quello che c'e' di giusto e mai quello che c'e' di sbagliato.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# Le funzioni prendono la radice come parametro invece di leggere ROOT: le
# sonde negative in test_check_toolchain_pins.py costruiscono un albero finto e
# ci verificano che una divergenza venga davvero intercettata. Provarlo mutando
# i file veri lascerebbe il repository sporco se il test si interrompe.
PINS_RELATIVO = "scripts/toolchain-pins.env"

# File sorvegliati per l'esclusivita'. Un pin di quella famiglia che compaia
# qui dentro con un valore diverso da quello canonico e' una divergenza.
SORVEGLIATI = (
    "rust-toolchain.toml",
    "Dockerfile.dev",
    "fuzz/Dockerfile",
    ".github/workflows/ci.yml",
    ".github/workflows/release.yml",
    "scripts/fuzz-smoke.sh",
    "scripts/fuzz-campaign.sh",
)


def leggi_pin(radice: Path) -> dict[str, str]:
    valori: dict[str, str] = {}
    for riga in (radice / PINS_RELATIVO).read_text(encoding="utf-8").splitlines():
        nuda = riga.strip()
        if not nuda or nuda.startswith("#") or "=" not in nuda:
            continue
        chiave, _, valore = nuda.partition("=")
        valori[chiave.strip()] = valore.strip()
    return valori


class Famiglia:
    """Un pin, con il pattern che ne riconosce **qualsiasi** valore."""

    def __init__(
        self,
        chiave: str,
        pattern: str,
        obbligatori: tuple[str, ...],
        nota: str,
    ) -> None:
        self.chiave = chiave
        self.pattern = re.compile(pattern)
        self.obbligatori = obbligatori
        self.nota = nota


FAMIGLIE = (
    Famiglia(
        "PLENORA_RUST_NIGHTLY",
        r"nightly-\d{4}-\d{2}-\d{2}",
        (
            "Dockerfile.dev",
            "fuzz/Dockerfile",
            ".github/workflows/ci.yml",
            "scripts/fuzz-smoke.sh",
            "scripts/fuzz-campaign.sh",
        ),
        "due nightly diversi costruiscono binari strumentati diversi a parita' "
        "di revisione, e il fuzzing smette di essere riproducibile",
    ),
    Famiglia(
        "PLENORA_CARGO_FUZZ_VERSION",
        r"cargo-fuzz(?:[^\n]*?)--version[= ]\"?\$?\{?[A-Z_]*\}?\"?\s*(\d+\.\d+\.\d+)"
        r"|cargo-fuzz@(\d+\.\d+\.\d+)"
        r"|CARGO_FUZZ_VERSION=(\d+\.\d+\.\d+)"
        r"|cargo install cargo-fuzz --locked --version (\d+\.\d+\.\d+)",
        ("Dockerfile.dev", "fuzz/Dockerfile", ".github/workflows/ci.yml"),
        "cargo-fuzz decide i flag passati a libFuzzer",
    ),
    Famiglia(
        "PLENORA_CARGO_LLVM_COV_VERSION",
        r"cargo-llvm-cov@(\d+\.\d+\.\d+)"
        r"|CARGO_LLVM_COV_VERSION=(\d+\.\d+\.\d+)"
        r"|cargo install cargo-llvm-cov[^\n]*?--version \"?\$?\{?[A-Z_]*\}?\"?\s*(\d+\.\d+\.\d+)",
        ("Dockerfile.dev", ".github/workflows/ci.yml"),
        "la percentuale confrontata con la soglia dipende da come lo strumento "
        "aggrega le regioni, quindi la versione fa parte della misura",
    ),
)

# Lo stable e' trattato a parte: compare in due forme, il numero pieno e il tag
# dell'immagine Docker, che porta solo maggiore e minore.
STABLE_OBBLIGATORI = (
    "rust-toolchain.toml",
    "Dockerfile.dev",
    "fuzz/Dockerfile",
    ".github/workflows/ci.yml",
)


def valori_trovati(testo: str, famiglia: Famiglia) -> set[str]:
    trovati: set[str] = set()
    for corrispondenza in famiglia.pattern.finditer(testo):
        gruppi = [g for g in corrispondenza.groups() if g] or [corrispondenza.group(0)]
        trovati.update(gruppi)
    return trovati


def controlla_stable(radice: Path, atteso: str, errori: list[str]) -> None:
    maggiore_minore = ".".join(atteso.split(".")[:2])
    pieno = re.compile(r"(?<![\d.])\d+\.\d+\.\d+(?![\d.])")
    tag = re.compile(r"rust:(\d+\.\d+)(?:\.\d+)?-")

    for relativo in SORVEGLIATI:
        percorso = radice / relativo
        if not percorso.is_file():
            continue
        testo = percorso.read_text(encoding="utf-8")

        for trovato in set(tag.findall(testo)):
            if trovato != maggiore_minore:
                errori.append(
                    f"{relativo}: immagine `rust:{trovato}-...`, ma lo stable "
                    f"pinnato e' {atteso}. Il container costruirebbe con un "
                    "compilatore diverso da quello di rust-toolchain.toml."
                )

        # Il numero pieno va cercato solo dove e' davvero una toolchain: nei
        # workflow accanto a `toolchain:`, nel toml accanto a `channel`.
        for riga in testo.splitlines():
            if "toolchain:" not in riga and "channel" not in riga:
                continue
            if "nightly" in riga:
                continue
            for trovato in pieno.findall(riga):
                if trovato != atteso:
                    errori.append(
                        f"{relativo}: toolchain {trovato} dichiarata, ma lo "
                        f"stable pinnato e' {atteso}."
                    )

    for relativo in STABLE_OBBLIGATORI:
        percorso = radice / relativo
        if not percorso.is_file():
            errori.append(f"{relativo}: manca, ma deve dichiarare lo stable.")
            continue
        testo = percorso.read_text(encoding="utf-8")
        if maggiore_minore not in testo:
            errori.append(
                f"{relativo}: non dichiara lo stable {atteso}. Ogni "
                "consumatore deve dichiararlo: senza, cancellare la riga "
                "farebbe passare il gate."
            )


def verifica(radice: Path) -> list[str]:
    """Restituisce l'elenco delle divergenze; vuoto se i pin sono allineati."""
    if not (radice / PINS_RELATIVO).is_file():
        return [f"manca la fonte unica dei pin: {PINS_RELATIVO}"]

    pin = leggi_pin(radice)
    errori: list[str] = []

    for famiglia in FAMIGLIE:
        atteso = pin.get(famiglia.chiave)
        if atteso is None:
            errori.append(f"{famiglia.chiave} non e' definito nei pin.")
            continue

        for relativo in SORVEGLIATI:
            percorso = radice / relativo
            if not percorso.is_file():
                continue
            testo = percorso.read_text(encoding="utf-8")
            for trovato in valori_trovati(testo, famiglia):
                if trovato != atteso:
                    errori.append(
                        f"{relativo}: {famiglia.chiave} vale `{trovato}`, ma il "
                        f"pin canonico e' `{atteso}`. Divergenza: {famiglia.nota}."
                    )

        for relativo in famiglia.obbligatori:
            percorso = radice / relativo
            testo = percorso.read_text(encoding="utf-8") if percorso.is_file() else ""
            if atteso not in testo:
                errori.append(
                    f"{relativo}: non dichiara {famiglia.chiave} = `{atteso}`. "
                    "Ogni consumatore obbligatorio deve dichiararlo, altrimenti "
                    "cancellare la riga farebbe passare il gate."
                )

    stable = pin.get("PLENORA_RUST_STABLE")
    if stable is None:
        errori.append("PLENORA_RUST_STABLE non e' definito nei pin.")
    else:
        controlla_stable(radice, stable, errori)

    return errori


def main() -> int:
    errori = verifica(ROOT)
    if errori:
        for messaggio in errori:
            print(messaggio, file=sys.stderr)
        return 1

    pin = leggi_pin(ROOT)
    print(
        "pin di toolchain allineati: "
        + ", ".join(f"{chiave.removeprefix('PLENORA_')}={valore}" for chiave, valore in pin.items())
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
