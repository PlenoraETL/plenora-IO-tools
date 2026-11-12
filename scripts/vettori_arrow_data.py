#!/usr/bin/env python3
"""I vettori `arrow-data-v1` di plenora-contracts contro il binario di IO.

Arrow Geometry Semantics 1.0 (REJ-004) vuole che un componente che lo dichiara
tratti i vettori invalidi con la categoria che ciascuno porta e accetti quelli
validi. IO e' consumatore di Arrow in due operazioni, `read` da un IPC e
`write` da un IPC: ogni vettore diventa un IPC file e passa da entrambe, verso
IPC, cosi' che il verdetto dipenda dall'ingresso e non dal formato d'uscita.

IPC trasporta piu' geometrie: non sono ammesse esenzioni GEO-012.

    python scripts/vettori_arrow_data.py --contracts .plenora-contracts \\
        --cli target/release/plenora-io --referto vettori-arrow-data.json

Ogni rifiuto si legge anche con REJ-003: exit 3, `remote_effect: none`,
`retry.kind: never`, fase `validate` per le classi che decide lo schema (il
limite di supporto compreso) e `read` per le due classi di valore, che IO
incontra leggendo il proprio ingresso in entrambe le operazioni. La classe la
decide l'oracolo dei contratti (`schema_verdict`), non questo script.

Il referto elenca per vettore e per operazione l'esito osservato e quello
atteso; l'uscita e' 0 solo se ogni vettore ha il verdetto del contratto.
"""
from __future__ import annotations

import argparse
import copy
import json
import subprocess
import sys
import tempfile
from pathlib import Path

import pyarrow as pa
import pyarrow.ipc as ipc

TIPI = {"int64": pa.int64(), "binary": pa.binary(), "utf8": pa.utf8()}
OPERAZIONI = ("read", "write")


def tabella(vettore: dict) -> pa.Table:
    campi, colonne = [], []
    for campo in vettore["fields"]:
        tipo = TIPI[campo["type"]]
        md = {k.encode(): v.encode() for k, v in (campo.get("metadata") or {}).items()}
        campi.append(pa.field(campo["name"], tipo, campo.get("nullable", True), metadata=md or None))
        valori = [riga.get(campo["name"]) for riga in vettore["rows"]]
        if tipo == pa.binary():
            valori = [None if v is None else bytes.fromhex(v) for v in valori]
        colonne.append(pa.array(valori, type=tipo))
    md = {k.encode(): v.encode() for k, v in (vettore.get("schema_metadata") or {}).items()}
    return pa.Table.from_arrays(colonne, schema=pa.schema(campi, metadata=md or None))


def esegui(cli: Path, operazione: str, ingresso: Path, uscita: Path) -> dict:
    if operazione == "read":
        argv = [str(cli), "read", str(ingresso), "--output", str(uscita), "--format", "json"]
    else:
        argv = [str(cli), "write", str(ingresso), str(uscita), "--to", "ipc", "--format", "json"]
    esito = subprocess.run(argv, capture_output=True, text=True, timeout=120)
    try:
        busta = json.loads(esito.stdout)
    except json.JSONDecodeError:
        return {"exit": esito.returncode, "category": None, "busta": "illeggibile"}
    errore = busta.get("error") if isinstance(busta, dict) else None
    if not isinstance(errore, dict):
        return {"exit": esito.returncode, "category": None, "code": None}
    retry = errore.get("retry")
    return {"exit": esito.returncode, "category": errore.get("category"), "code": errore.get("code"),
            "phase": errore.get("phase"), "remote_effect": errore.get("remote_effect"),
            "retry": retry.get("kind") if isinstance(retry, dict) else None}


def rej_003(osservato: dict, di_valore: bool) -> bool:
    """Gli assi del rifiuto che REJ-003 fissa, per la classe del rifiuto."""
    return (osservato["exit"] == 3 and osservato.get("remote_effect") == "none"
            and osservato.get("retry") == "never"
            and osservato.get("phase") == ("read" if di_valore else "validate"))


def verdetto(vettore: dict, osservato: dict, oracolo) -> str:
    accettato = osservato["exit"] == 0
    if vettore["expect"] == "valid":
        return "conforme" if accettato else "rifiuto-indebito"
    if accettato:
        return "accettazione-indebita"
    if osservato["category"] != vettore["category"]:
        return "categoria-sbagliata"
    di_valore = oracolo.schema_verdict(vettore) is None
    return "conforme" if rej_003(osservato, di_valore) else "assi-del-rifiuto"


def expected_ipc(vettore):
    """Trasformazioni IO dichiarate, calcolate dall'input, mai dall'osservato."""
    v = copy.deepcopy(vettore)
    v.setdefault("schema_metadata", {})["plenora.contract.version"] = "1"
    occupied = {int(f["metadata"]["plenora.field_id"]) for f in v["fields"] if "plenora.field_id" in f.get("metadata", {})}
    for f in v["fields"]:
        md = f.setdefault("metadata", {})
        if "plenora.field_id" not in md:
            i = 0
            while i in occupied: i += 1
            md["plenora.field_id"] = str(i); occupied.add(i)
    return tabella(v)


def main() -> int:
    argomenti = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    argomenti.add_argument("--contracts", type=Path, required=True)
    argomenti.add_argument("--cli", type=Path, required=True)
    argomenti.add_argument("--referto", type=Path, required=True)
    opzioni = argomenti.parse_args()

    sys.path.insert(0, str(opzioni.contracts / "tools"))
    import arrow_data as oracolo

    cartella = opzioni.contracts / "vectors" / "arrow-data-v1"
    vettori = sorted(cartella.glob("*.json"))
    if not vettori:
        print("nessun vettore arrow-data-v1 nel checkout dei contratti", file=sys.stderr)
        return 2
    righe, conteggi = [], {}
    with tempfile.TemporaryDirectory(prefix="vettori-arrow-data-") as lavoro:
        lavoro = Path(lavoro)
        for percorso in vettori:
            vettore = json.loads(percorso.read_text(encoding="utf-8"))
            ingresso = lavoro / f"{percorso.stem}.arrow"
            t = tabella(vettore)
            with pa.OSFile(str(ingresso), "wb") as f, ipc.new_file(f, t.schema) as w:
                w.write_table(t)
            for operazione in OPERAZIONI:
                osservato = esegui(opzioni.cli, operazione, ingresso, lavoro / f"{percorso.stem}-{operazione}.arrow")
                v = verdetto(vettore, osservato, oracolo)
                if v == "conforme" and vettore["expect"] == "valid":
                    with ipc.open_file(pa.BufferReader((lavoro / f"{percorso.stem}-{operazione}.arrow").read_bytes())) as reader:
                        actual = reader.read_all()
                    if not actual.equals(expected_ipc(vettore), check_metadata=True):
                        v = "output-diverso"
                conteggi[v] = conteggi.get(v, 0) + 1
                righe.append({"vettore": percorso.stem, "operazione": operazione,
                              "atteso": vettore["expect"], "categoria_attesa": vettore.get("category"),
                              "osservato": osservato, "verdetto": v})
    opzioni.referto.write_text(json.dumps({"schema_version": 1, "vettori": len(vettori),
                                           "conteggi": conteggi, "esiti": righe},
                                          ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    totale = len(righe)
    print(f"vettori arrow-data-v1: {len(vettori)} su {len(OPERAZIONI)} operazioni, {totale} esiti: "
          + ", ".join(f"{k} {v}" for k, v in sorted(conteggi.items())))
    # Nessuna esenzione IPC multigeometria.
    return 0 if set(conteggi) <= {"conforme"} else 1


if __name__ == "__main__":
    sys.exit(main())
