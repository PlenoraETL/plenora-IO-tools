#!/usr/bin/env python3
"""La politica cargo-deny: coerente col registro delle eccezioni, ed eseguita.

# Che cosa verifica

1. **Le eccezioni sono quelle del registro.** Gli advisory ignorati in
   `deny.toml` sono esattamente gli `id` di
   `assurance/registries/dependency-exceptions.json`, lo stesso registro da cui
   `scripts/audit_ignores.py` ricava i flag di `cargo audit`. Un'eccezione
   scritta solo in `deny.toml` non avrebbe motivo, esposizione ne' condizione
   di chiusura; una scritta solo nel registro sarebbe un'eccezione che uno dei
   due strumenti non applica.
2. **La versione dello strumento e' quella fissata.** `cargo deny --version`
   deve rendere `PLENORA_CARGO_DENY_VERSION` di `scripts/toolchain-pins.env`:
   una versione mobile cambia le regole fra due corse dello stesso commit.
3. **Con `--esegui`**, `cargo deny --locked check` sui due grafi: il workspace
   e quello staccato di `fuzz/`, ciascuno col proprio `Cargo.lock`.

# Perche' `advisories` puo' diventare rosso senza un commit

Dipende dal database RustSec, che cambia ogni giorno: e' il motivo per cui il
workflow gira anche a calendario. Licenze, ban e sorgenti dipendono invece
solo dal lockfile e da `deny.toml`.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import re
import subprocess
import sys
import tomllib

ROOT = pathlib.Path(__file__).resolve().parent.parent
POLITICA = ROOT / "deny.toml"
REGISTRO = ROOT / "assurance" / "registries" / "dependency-exceptions.json"
PIN = ROOT / "scripts" / "toolchain-pins.env"
CHIAVE_DEL_PIN = "PLENORA_CARGO_DENY_VERSION"

#: I due grafi che il repository dichiara: il workspace e `fuzz/`.
GRAFI = (None, "fuzz/Cargo.toml")


def ignorati_della_politica(politica: dict) -> set[str]:
    voci = politica.get("advisories", {}).get("ignore", [])
    return {voce["id"] if isinstance(voce, dict) else voce for voce in voci}


def accettati_del_registro(registro: dict) -> set[str]:
    return {voce["id"] for voce in registro.get("accettate", [])}


def coerenza(politica: dict, registro: dict) -> list[str]:
    errori: list[str] = []
    if politica.get("advisories", {}).get("yanked") != "deny":
        errori.append("deny.toml: `advisories.yanked` deve essere `deny`")
    nella_politica = ignorati_della_politica(politica)
    nel_registro = accettati_del_registro(registro)
    for solo in sorted(nella_politica - nel_registro):
        errori.append(
            f"«{solo}» e' ignorato in deny.toml e non e' nel registro delle "
            "eccezioni: un'eccezione senza motivo ne' condizione di chiusura"
        )
    for solo in sorted(nel_registro - nella_politica):
        errori.append(
            f"«{solo}» e' nel registro delle eccezioni e deny.toml non lo "
            "ignora: i due strumenti applicano politiche diverse"
        )
    sorgenti = politica.get("sources", {})
    if sorgenti.get("unknown-registry") != "deny" or sorgenti.get("unknown-git") != "deny":
        errori.append("deny.toml: registri e repository git sconosciuti vanno rifiutati")
    return errori


def versione_fissata(testo_del_pin: str) -> str | None:
    for riga in testo_del_pin.splitlines():
        nuda = riga.strip()
        if nuda.startswith(f"{CHIAVE_DEL_PIN}="):
            return nuda.partition("=")[2].strip()
    return None


def versione_installata(uscita: str) -> str | None:
    trovata = re.match(r"cargo-deny (\d+\.\d+\.\d+)\s*$", uscita.strip())
    return trovata.group(1) if trovata else None


def esegui(atteso: str) -> list[str]:
    corsa = subprocess.run(
        ["cargo", "deny", "--version"], capture_output=True, text=True, check=False
    )
    installata = versione_installata(corsa.stdout) if corsa.returncode == 0 else None
    if installata != atteso:
        return [
            f"cargo-deny installato: «{installata}», fissato: «{atteso}». Una "
            "versione diversa applica regole diverse allo stesso commit."
        ]
    errori: list[str] = []
    for manifesto in GRAFI:
        comando = ["cargo", "deny", "--locked"]
        if manifesto:
            comando += ["--manifest-path", manifesto]
        comando += ["check", "--hide-inclusion-graph"]
        if subprocess.run(comando, cwd=ROOT, check=False).returncode != 0:
            errori.append(f"cargo deny rosso sul grafo «{manifesto or 'Cargo.toml'}»")
    return errori


def main() -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument(
        "--esegui",
        action="store_true",
        help="esegue anche cargo deny sui due grafi, con la versione fissata",
    )
    opzioni = argomenti.parse_args()

    politica = tomllib.loads(POLITICA.read_text(encoding="utf-8"))
    registro = json.loads(REGISTRO.read_text(encoding="utf-8"))
    errori = coerenza(politica, registro)
    atteso = versione_fissata(PIN.read_text(encoding="utf-8"))
    if atteso is None:
        errori.append(f"{CHIAVE_DEL_PIN} non e' in scripts/toolchain-pins.env")
    if not errori and opzioni.esegui and atteso is not None:
        errori.extend(esegui(atteso))
    for errore in errori:
        print(errore, file=sys.stderr)
    if errori:
        return 1
    coda = "; cargo deny verde sul workspace e su fuzz/" if opzioni.esegui else ""
    print(
        f"politica cargo-deny coerente col registro "
        f"({len(accettati_del_registro(registro))} eccezioni), cargo-deny {atteso}{coda}."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
