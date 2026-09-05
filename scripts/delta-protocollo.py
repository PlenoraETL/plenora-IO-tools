#!/usr/bin/env python3
"""Che cosa cambia, sul filo, fra il protocollo v1 e il v2.

# Perche' misurarlo invece di scriverlo

Una guida alla migrazione e' un elenco di differenze, e un elenco scritto a mano
invecchia in silenzio: il giorno in cui una busta cambia, la guida continua a
descrivere quella di prima con la stessa aria di certezza. Questo strumento
esegue il **binario** nei due protocolli sulla stessa fixture e confronta le
strutture che escono. Cio' che dice e' misurato.

# Perche' non si puo' dedurre dai due manifesti

Il manifesto del v1 e' congelato e descrive soltanto il primo livello delle
buste: `required_top_level`, e sotto tace. Era il difetto che il v2 ha chiuso
descrivendo anche le strutture annidate. Confrontare i due documenti direbbe
quindi che il v2 ha centinaia di campi in piu' -- che e' vero del **documento**
e falso del filo.

# Che cosa conta come differenza

I percorsi, non i valori. Due buste con lo stesso insieme di percorsi sono la
stessa forma anche se i numeri differiscono; una busta a cui manca un percorso
richiede una modifica in chi la legge. I valori cambiano col file d'ingresso, la
forma no -- ed e' la forma che si migra.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
CANONICHE = ROOT / "crates" / "plenora-io-cli" / "tests" / "fixtures" / "canoniche"

#: I cinque comandi, tutti e cinque.
#:
#: `catalog` c'e' perche' il contratto v1 dichiara sei buste e la sua e' una di
#: quelle. Per un periodo il dispatch lo teneva fuori dalla scelta del
#: protocollo, e questo strumento -- nato dopo -- aveva registrato quell'assenza
#: come se fosse il disegno: una misura che descrive un difetto e lo chiama
#: intenzione e' peggio di una misura mancante.
#:
#: Gli argomenti sono scelti per **far uscire** la diagnostica invece di una
#: busta pulita: e' li' che i due protocolli differiscono, e un confronto su un
#: file senza perdita non troverebbe niente da migrare.
COMANDI = (
    ("catalog", ["catalog"]),
    ("inspect", ["inspect", "{shp}"]),
    ("layers", ["layers", "{shp}"]),
    ("read", ["read", "{shp}"]),
    # Il formato d'uscita lo decide l'estensione.
    ("convert", ["convert", "{shp}", "{uscita}"]),
)


def forma(documento, prefisso: str = "") -> set[str]:
    """L'insieme dei percorsi di un documento JSON.

    Le liste collassano in un elemento solo (`[]`): due buste che differiscono
    per **quante** ragioni riportano non differiscono per forma, e contare le
    ripetizioni farebbe sembrare una differenza cio' che dipende dal file.
    """
    percorsi = {prefisso} if prefisso else set()
    if isinstance(documento, dict):
        for chiave, valore in documento.items():
            percorsi |= forma(valore, f"{prefisso}.{chiave}")
    elif isinstance(documento, list):
        for voce in documento:
            percorsi |= forma(voce, f"{prefisso}[]")
    return percorsi


def esegui(binario: pathlib.Path, argomenti: list[str]) -> tuple[dict, str]:
    esito = subprocess.run(
        [str(binario), *argomenti], capture_output=True, text=True, timeout=120
    )
    try:
        return json.loads(esito.stdout), esito.stderr
    except json.JSONDecodeError:
        return {}, esito.stderr


def main(argv: list[str] | None = None) -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument("--binario", required=True, type=pathlib.Path)
    argomenti.add_argument("--fixture", type=pathlib.Path, default=None)
    argomenti.add_argument("--lavoro", type=pathlib.Path, required=True)
    argomenti.add_argument("--rapporto", type=pathlib.Path, default=None)
    opzioni = argomenti.parse_args(argv)
    opzioni.lavoro.mkdir(parents=True, exist_ok=True)

    shp = opzioni.fixture or (CANONICHE / "canonico.geojson")
    if not shp.is_file():
        print(f"la fixture {shp} non c'e'", file=sys.stderr)
        return 1

    delta: dict[str, dict] = {}

    for nome, modello in COMANDI:
        # Un'uscita per esecuzione. Riusare lo stesso percorso faceva fallire
        # la seconda -- il prodotto rifiuta di sovrascrivere -- e il confronto
        # leggeva un errore al posto della busta legacy: una differenza
        # inventata dallo strumento invece che misurata sul prodotto.
        def argomenti_per(suffisso: str) -> list[str]:
            return [
                pezzo.format(
                    shp=shp, uscita=opzioni.lavoro / f"{nome}-{suffisso}.csv"
                )
                for pezzo in modello
            ]

        # Il v1 si sceglie con un'opzione il cui nome dice che cosa si sceglie.
        v2, _ = esegui(opzioni.binario, argomenti_per("v2"))
        v1, avviso = esegui(
            opzioni.binario, [*argomenti_per("v1"), "--legacy-protocol-v1-unsafe"]
        )
        fa, fb = forma(v1), forma(v2)
        delta[nome] = {
            "contratto_v1": v1.get("contract"),
            "contratto_v2": v2.get("contract"),
            "solo_nel_v1": sorted(fa - fb),
            "solo_nel_v2": sorted(fb - fa),
            "in_comune": len(fa & fb),
            "avviso_su_stderr": avviso.strip() != "",
        }

    print(f"{'comando':10} {'v1':7} {'v2':7} {'comuni':7}  campi che cambiano")
    for nome, voce in delta.items():
        print(
            f"{nome:10} {len(voce['solo_nel_v1']):<7} {len(voce['solo_nel_v2']):<7} "
            f"{voce['in_comune']:<7}  {voce['contratto_v1']} -> {voce['contratto_v2']}"
        )
        for percorso in voce["solo_nel_v1"]:
            print(f"             - {percorso}   (sparisce nel v2)")
        for percorso in voce["solo_nel_v2"]:
            print(f"             + {percorso}   (nuovo nel v2)")

    if opzioni.rapporto is not None:
        opzioni.rapporto.write_text(
            json.dumps(delta, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
        )
        print(f"\nrapporto in {opzioni.rapporto}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
