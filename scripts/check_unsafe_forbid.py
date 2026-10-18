#!/usr/bin/env python3
"""`unsafe_code = "forbid"` vale per ogni crate del workspace, PyO3 compreso.

# Perche' un gate, se il compilatore lo impone gia'

Il compilatore impone il lint **dove e' dichiarato**. Basta che un crate perda
`[lints] workspace = true`, o che il workspace passi da `forbid` a `deny`, e un
`#[allow(unsafe_code)]` locale torna possibile senza che niente diventi rosso.
Il crate PyO3 (`crates/plenora-io-py`) e' quello in cui la tentazione e'
concreta: le macro `#[pymodule]` e `#[pyfunction]` generano `unsafe` per
l'interfaccia C di CPython. Quel codice e' espansione di una macro di un altro
crate, che il lint `unsafe_code` non esamina (rustc non riporta questo lint
dentro le macro esterne); e' codice di `pyo3`, non nostro, ed e' registrato
come eccezione della dipendenza in `assurance/registries/dependency-exceptions.json`
(`unsafe_nelle_dipendenze`), come fanno plenora-database-tools e
plenora-data-tools per i loro crate PyO3.

Il gate verifica le tre cose che tengono in piedi quella frase:

* il workspace dichiara `unsafe_code = "forbid"`, non `deny` ne' `warn`;
* ogni crate membro eredita i lint del workspace;
* nessun sorgente dei crate scrive `unsafe` o `allow(unsafe_code)`;

salvo i crate con una deroga dichiarata nello stesso registro
(`deroghe_al_forbid`: oggi il solo `plenora-bench`, harness di misura non
spedito), che devono restare non pubblicabili;

e che ogni crate con una dipendenza `pyo3` abbia la sua voce nel registro.
"""

from __future__ import annotations

import json
import pathlib
import re
import sys
import tomllib

RADICE = pathlib.Path(__file__).resolve().parent.parent
REGISTRO = RADICE / "assurance" / "registries" / "dependency-exceptions.json"

# Un `unsafe` scritto: blocco, funzione, impl, trait, extern. Nei commenti e
# nelle stringhe la parola puo' comparire, e il gate guarda solo il codice.
UNSAFE = re.compile(r"\bunsafe\s*(\{|fn\b|impl\b|trait\b|extern\b)")
ALLOW = re.compile(r"allow\s*\(\s*unsafe_code\s*\)")


STRINGA = re.compile(r'"(?:\\.|[^"\\])*"')


def _senza_commenti(riga: str) -> str:
    """Il codice della riga: senza il commento `//` e senza le stringhe.

    Basta per questo uso: un `unsafe` vero non sta mai in una stringa, e un
    commento o un messaggio che nomina la parola non e' codice.
    """
    dentro = False
    for indice, carattere in enumerate(riga):
        if carattere == '"' and (indice == 0 or riga[indice - 1] != "\\"):
            dentro = not dentro
        elif not dentro and riga.startswith("//", indice):
            riga = riga[:indice]
            break
    return STRINGA.sub('""', riga)


def problemi(radice: pathlib.Path) -> list[str]:
    trovati: list[str] = []
    workspace = tomllib.loads((radice / "Cargo.toml").read_text(encoding="utf-8"))
    livello = workspace.get("workspace", {}).get("lints", {}).get("rust", {}).get("unsafe_code")
    if livello != "forbid":
        trovati.append(
            f"Cargo.toml: `unsafe_code` del workspace e' {livello!r}, non \"forbid\""
        )

    registro = json.loads((radice / REGISTRO.relative_to(RADICE)).read_text(encoding="utf-8"))
    registrati = {voce.get("crate_del_workspace") for voce in registro.get("unsafe_nelle_dipendenze", [])}
    derogati = {voce.get("crate_del_workspace") for voce in registro.get("deroghe_al_forbid", [])}

    for manifesto in sorted((radice / "crates").glob("*/Cargo.toml")):
        crate = manifesto.parent
        nome = crate.name
        dati = tomllib.loads(manifesto.read_text(encoding="utf-8"))
        if nome in derogati:
            # Una deroga dichiarata vale solo per un crate che non si spedisce.
            if dati.get("package", {}).get("publish") is not False:
                trovati.append(f"{nome}: derogato al `forbid` ma pubblicabile")
            continue
        if dati.get("lints", {}).get("workspace") is not True:
            trovati.append(f"{nome}: non eredita i lint del workspace (`[lints] workspace = true`)")
        if "pyo3" in dati.get("dependencies", {}) and nome not in registrati:
            trovati.append(
                f"{nome}: dipende da `pyo3` e non ha la voce in "
                "`unsafe_nelle_dipendenze` del registro delle eccezioni"
            )
        for sorgente in sorted(crate.rglob("*.rs")):
            for numero, riga in enumerate(sorgente.read_text(encoding="utf-8").splitlines(), 1):
                codice = _senza_commenti(riga)
                if UNSAFE.search(codice) or ALLOW.search(codice):
                    relativo = sorgente.relative_to(radice).as_posix()
                    trovati.append(f"{relativo}:{numero}: `unsafe` scritto nel crate")
    return trovati


def main() -> int:
    trovati = problemi(RADICE)
    for problema in trovati:
        print(problema, file=sys.stderr)
    if trovati:
        return 1
    print("unsafe_code = \"forbid\" su ogni crate del workspace; l'unsafe di pyo3 e' registrato")
    return 0


if __name__ == "__main__":
    sys.exit(main())
