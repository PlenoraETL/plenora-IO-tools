#!/usr/bin/env python3
"""Il manifesto di adozione, con i digest misurati sui byte che si spediscono.

# Perche' un generatore e non un file scritto a mano

Il manifesto v4 porta, per ogni artefatto pubblico, una versione e un
`sha256:<64 hex>`. ADOPTION.md dice a che cosa serve: «the digest identifies the
exact binary, crate archive, wheel, package or deployed runtime image exercised
by the verification commands. A branch, mutable tag, rebuilt checkout or version
string without the digest is not sufficient adoption evidence».

Un digest battuto a tastiera e' un numero. Questo script lo **misura**, sui file
veri, dopo che sono stati costruiti: fra il congelamento del codice e il
manifesto non c'e' nessuna modifica da fare, solo una corsa.

# Che cosa viene da dove

* la parte redatta -- contratti, stato, deviazioni -- da
  `contracts/adozione-4.0.0.json`, che si rilegge in revisione;
* la **versione** dal `Cargo.toml` del workspace, che e' l'unica fonte: il
  registro della distribuzione lo dice per esteso, due copie divergono;
* i **digest** dai file che si passano sulla riga di comando.

# Dove va scritto, e perche' li'

In sviluppo, `contracts/adoption-manifest.json`, che non e' committato: porta i
digest degli artefatti e cambierebbe a ogni costruzione, e un digest stantio
somiglia a una garanzia piu' di quanto un digest assente somigli a una lacuna.

**In qualifica**, `assurance/evidence/adoption-manifest-<sha>.json`. La
destinazione non e' una preferenza: dopo il congelamento si puo' cambiare solo
cio' che l'assurance produce, e `assurance/evidence/` e' una delle quattro voci
dell'allowlist. Scrivere il manifesto li' vuol dire registrarlo **senza toccare
codice, lock, costruttori o verificatori** -- che e' esattamente la condizione
che rende confrontabili l'albero qualificato e quello da cui gli artefatti sono
usciti.

# Che cosa non fa

Non costruisce gli artefatti e non li verifica: li legge. Costruirli e'
`costruisci-artefatto-*.py`, verificarli e' il verificatore del profilo
pubblico, che riceve un percorso e interroga il processo. Questo script mette
insieme i due fatti e li scrive in una forma che un consumatore puo' leggere.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
import sys

RADICE = pathlib.Path(__file__).resolve().parent.parent
REDATTO = RADICE / "contracts" / "adozione-4.0.0.json"
USCITA = RADICE / "contracts" / "adoption-manifest.json"
SCHEMA = "adoption-manifest-v4.schema.json"

#: Le superfici che questo componente dichiara. `python_sdk` non c'e': lo SDK
#: esiste e viene spedito, ma il profilo non lo richiede e il manifesto lo
#: dichiara `not_applicable` -- elencarne un artefatto direbbe il contrario.
SUPERFICI = {"cli", "rust"}


def versione_del_workspace() -> str:
    testo = (RADICE / "Cargo.toml").read_text(encoding="utf-8")
    trovata = re.search(r'^version\s*=\s*"([^"]+)"', testo, re.M)
    if not trovata:
        raise SystemExit("Cargo.toml del workspace senza `version`")
    return trovata.group(1)


def digest(percorso: pathlib.Path) -> str:
    impronta = hashlib.sha256()
    with percorso.open("rb") as flusso:
        for pezzo in iter(lambda: flusso.read(1024 * 1024), b""):
            impronta.update(pezzo)
    return f"sha256:{impronta.hexdigest()}"


def artefatto(specifica: str, versione: str) -> dict:
    """`<superficie>:<percorso>:<comando di verifica>[;<altro>]`."""
    pezzi = specifica.split(":", 2)
    if len(pezzi) != 3:
        raise SystemExit(
            f"«{specifica}» non ha la forma <superficie>:<percorso>:<verifiche>"
        )
    superficie, percorso, verifiche = pezzi
    if superficie not in SUPERFICI:
        raise SystemExit(
            f"superficie «{superficie}» fuori da {sorted(SUPERFICI)}: il manifesto "
            "dichiara le superfici che esistono, e le altre stanno fra i contratti "
            "`not_applicable`"
        )
    file = pathlib.Path(percorso)
    if not file.is_file():
        raise SystemExit(f"«{percorso}» non e' un file: i digest si misurano, non si scrivono")
    return {
        "name": file.name,
        "surface": superficie,
        "version": versione,
        "digest": digest(file),
        "verification": [v for v in verifiche.split(";") if v],
    }


def main() -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument(
        "artefatto",
        nargs="+",
        help="<superficie>:<percorso>:<comando>[;<comando>]",
    )
    argomenti.add_argument("--uscita", default=str(USCITA))
    opzioni = argomenti.parse_args()

    redatto = json.loads(REDATTO.read_text(encoding="utf-8"))
    versione = versione_del_workspace()

    manifesto = {
        "schema_version": 4,
        "component": redatto["component"],
        "contracts_source": redatto["contracts_source"],
        "profile": redatto["profile"],
        "artifacts": [artefatto(uno, versione) for uno in opzioni.artefatto],
        "contracts": redatto["contracts"],
        "deviations": redatto["deviations"],
    }

    pathlib.Path(opzioni.uscita).write_bytes(
        (json.dumps(manifesto, ensure_ascii=False, indent=2) + "\n").encode("utf-8")
    )
    print(
        f"manifesto scritto in {opzioni.uscita}: {len(manifesto['artifacts'])} artefatti "
        f"alla {versione}, {len(manifesto['contracts'])} contratti, "
        f"{len(manifesto['deviations'])} deviazioni. Lo schema che lo valida e' "
        f"`{SCHEMA}` del checkout fissato."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
