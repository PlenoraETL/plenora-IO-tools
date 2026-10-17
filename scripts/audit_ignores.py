#!/usr/bin/env python3
"""Stampa i flag `--ignore`, o audita entrambi i lockfile con `--esegui`.

Gli advisory restano sotto --deny warnings; un secondo passaggio JSON con
yanked abilitato ammette soltanto zip@=8.6.0 se dichiarato nel registro.

Gli ID vivevano in due posti: un elenco nel workflow di CI e le ragioni in un
documento. Due elenchi divergono, e quello nel workflow non porta la ragione
dell'eccezione — quindi chi lo legge non sa se sia ancora giustificata.

Qui l'elenco e' uno solo, `assurance/registries/dependency-exceptions.json`, e
la CI lo legge da qui. Aggiungere un'eccezione significa scriverne motivo,
esposizione, condizione di chiusura e trigger di riesame: e' quello il punto,
non il flag.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
REGISTRO = ROOT / "assurance" / "registries" / "dependency-exceptions.json"

CAMPI = {"id", "crate", "accettata_dal", "motivo", "esposizione", "chiusura", "trigger_di_riesame"}


def flag(registro: dict) -> list[str]:
    """`['--ignore', 'RUSTSEC-…', …]`, o solleva se una voce e' incompleta."""
    fuori: list[str] = []
    for voce in registro.get("accettate", []):
        mancanti = CAMPI - set(voce)
        if mancanti:
            raise ValueError(
                f"{voce.get('id', '<senza id>')}: eccezione incompleta, mancano "
                f"{sorted(mancanti)}. Un'eccezione senza condizione di chiusura "
                "non e' temporanea, e' permanente senza dirlo."
            )
        fuori.extend(["--ignore", voce["id"]])
    return fuori


def verifica_yanked(report: dict, registro: dict) -> list[str]:
    """Controlla il report JSON 0.22.2, senza trasformare yanked in ignore RustSec."""
    ammessi = {v["crate"] for v in registro.get("yanked_accettati", [])}
    if ammessi - {"zip@=8.6.0"}:
        raise ValueError("eccezioni yanked: ammesso solo zip@=8.6.0")
    warnings = report["warnings"]
    if not isinstance(warnings, dict):
        raise ValueError("report audit: warnings non valido")
    yanked = warnings.get("yanked", [])
    if not isinstance(yanked, list):
        raise ValueError("report audit: yanked non valido")
    errori = []
    for voce in yanked:
        p = voce["package"]
        spec = f"{p['name']}@={p['version']}"
        if spec not in ammessi:
            errori.append(f"crate yanked non autorizzato: {spec}")
    return errori


def esegui(registro: dict) -> int:
    ignorati = flag(registro)
    # Valida l'allowlist anche prima di eseguire strumenti esterni.
    verifica_yanked({"warnings": {}}, registro)
    fallito = False
    for lock in ("Cargo.lock", "fuzz/Cargo.lock"):
        base = ["cargo", "audit", "--file", lock, *ignorati]
        # Mantiene anche i warning sullo strumento stesso e ogni advisory.
        advisory = subprocess.run(
            [*base, "--deny", "warnings", "--no-yanked"], cwd=ROOT, check=False
        )
        fallito |= advisory.returncode != 0
        corsa = subprocess.run(
            [*base, "--json"], cwd=ROOT, capture_output=True, text=True, check=False
        )
        print(corsa.stderr, end="", file=sys.stderr)
        if corsa.returncode != 0:
            print(corsa.stdout, file=sys.stderr)
        # Non convertire mai un errore operativo/vulnerabilita' in successo.
        # 0.22.2 puo' segnalare errori dell'indice solo su stderr.
        fallito |= corsa.returncode != 0 or "error:" in corsa.stderr
        try:
            errori = verifica_yanked(json.loads(corsa.stdout), registro)
            for errore in errori:
                print(f"{lock}: {errore}", file=sys.stderr)
            fallito |= bool(errori)
        except (ValueError, KeyError, TypeError) as errore:
            print(f"{lock}: report audit non valido: {errore}", file=sys.stderr)
            fallito = True
    return int(fallito)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--esegui", action="store_true", help="audita entrambi i lockfile")
    opzioni = parser.parse_args()
    try:
        registro = json.loads(REGISTRO.read_text(encoding="utf-8"))
        if opzioni.esegui:
            return esegui(registro)
        print(" ".join(flag(registro)))
    except (ValueError, OSError, KeyError, TypeError) as errore:
        print(errore, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
