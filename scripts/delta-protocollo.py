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

#: I comandi che leggono il protocollo dagli argomenti. `catalog` non e' fra
#: loro, e non per una dimenticanza di questo strumento: il dispatch della CLI
#: tiene i quattro comandi che possono consegnare un documento legacy separati
#: da `catalog`, che emette sempre il v2. Metterlo nell'elenco produrrebbe una
#: riga «nessuna differenza» che sembrerebbe dire �le due buste coincidono� e
#: direbbe invece �non ci sono due buste�.
COMANDI = (
    ("inspect", ["inspect", "{shp}"]),
    ("layers", ["layers", "{shp}"]),
    ("read", ["read", "{shp}"]),
    # Il formato d'uscita lo decide l'estensione.
    # apposta: i suoi nomi di campo stanno in dieci caratteri ASCII, e una
    # conversione che li accorcia **produce diagnostica** invece di una busta
    # pulita. E' li' che i due protocolli differiscono davvero.
    ("convert", ["convert", "{shp}", "{uscita}"]),
)

#: Il comando fuori dalla scelta, e cio' che di lui va comunque misurato.
SENZA_SCELTA = ("catalog", ["catalog"])


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

    # `catalog` prima, perche' cio' che va detto di lui e' diverso: non che le
    # due buste coincidano, ma che di buste ce ne sia una sola.
    solo_v2, _ = esegui(opzioni.binario, list(SENZA_SCELTA[1]))
    col_flag, avviso_catalog = esegui(
        opzioni.binario, [*SENZA_SCELTA[1], "--legacy-protocol-v1-unsafe"]
    )
    delta["catalog"] = {
        "sceglie_il_protocollo": False,
        "contratto_senza_flag": solo_v2.get("contract"),
        "contratto_col_flag": col_flag.get("contract"),
        "il_flag_cambia_qualcosa": solo_v2 != col_flag,
        "avviso_su_stderr": avviso_catalog.strip() != "",
        "che_cosa_significa": (
            "`catalog` emette il v2 in ogni caso. Il flag legacy non lo "
            "riguarda, e la CLI non lo rifiuta: chi lo passa riceve una busta "
            "v2 senza che niente glielo dica."
        ),
    }
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

    catalog = delta["catalog"]
    print(
        f"catalog    sempre {catalog['contratto_senza_flag']}; col flag legacy "
        f"{catalog['contratto_col_flag']}, avviso: "
        f"{'sì' if catalog['avviso_su_stderr'] else 'nessuno'}"
    )
    print()
    print(f"{'comando':10} {'v1':7} {'v2':7} {'comuni':7}  campi che cambiano")
    for nome, voce in delta.items():
        if nome == "catalog":
            continue
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
