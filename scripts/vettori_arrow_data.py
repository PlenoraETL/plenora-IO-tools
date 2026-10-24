#!/usr/bin/env python3
"""I vettori `arrow-data-v1` di plenora-contracts contro il binario di IO.

Arrow Geometry Semantics 1.0 (REJ-004) vuole che un componente che lo dichiara
tratti i vettori invalidi con la categoria che ciascuno porta e accetti quelli
validi. IO e' consumatore di Arrow in due operazioni, `read` da un IPC e
`write` da un IPC: ogni vettore diventa un IPC file e passa da entrambe, verso
IPC, cosi' che il verdetto dipenda dall'ingresso e non dal formato d'uscita.

Un vettore valido rifiutato con `unsupported` e' il verdetto di **supporto** di
GEO-000 (un componente puo' rifiutare un ingresso conforme che i suoi
bersagli non rappresentano): si conta a parte, perche' e' ammesso solo se il
manifesto lo dichiara, e non si confonde ne' con un'accettazione ne' con un
errore.

    python scripts/vettori_arrow_data.py --contracts .plenora-contracts \\
        --cli target/release/plenora-io --referto vettori-arrow-data.json

Il referto elenca per vettore e per operazione l'esito osservato e quello
atteso; l'uscita e' 0 solo se ogni vettore ha il verdetto del contratto.
"""
from __future__ import annotations

import argparse
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
    return {"exit": esito.returncode,
            "category": errore.get("category") if isinstance(errore, dict) else None,
            "code": errore.get("code") if isinstance(errore, dict) else None}


def verdetto(vettore: dict, osservato: dict) -> str:
    accettato = osservato["exit"] == 0
    if vettore["expect"] == "valid":
        if accettato:
            return "conforme"
        if osservato["category"] == "unsupported":
            return "rifiuto-di-supporto"
        return "rifiuto-indebito"
    if accettato:
        return "accettazione-indebita"
    return "conforme" if osservato["category"] == vettore["category"] else "categoria-sbagliata"


def main() -> int:
    argomenti = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    argomenti.add_argument("--contracts", type=Path, required=True)
    argomenti.add_argument("--cli", type=Path, required=True)
    argomenti.add_argument("--referto", type=Path, required=True)
    opzioni = argomenti.parse_args()

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
                v = verdetto(vettore, osservato)
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
    return 0 if conteggi.get("conforme", 0) == totale else 1


if __name__ == "__main__":
    sys.exit(main())
