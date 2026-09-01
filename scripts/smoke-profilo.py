#!/usr/bin/env python3
"""Lo smoke sull'artefatto **installato**: che cosa sa fare, davvero.

# Perche' due esiti attesi e non uno

I due profili sono due prodotti, e ciascuno ha una promessa da mantenere.

`filegdb` promette che FileGDB **funziona**: si dimostra scrivendone uno e
rileggendolo, verificando schema, righe e geometria. Che `catalog` lo dichiari
non basta -- `catalog` dice cio' che il driver crede di poter fare, e fra la
dichiarazione e il fatto c'e' la GDAL spedita.

`base` promette che FileGDB **non c'e'**, e questa e' la promessa che si dimentica
di verificare. Un profilo base costruito per sbaglio con la feature attiva
sarebbe piu' grande di sessanta megabyte e nessuno se ne accorgerebbe dal nome:
porterebbe un runtime GDAL che il suo contratto non prevede, con una superficie
e una licenza che chi lo installa non ha accettato. Lo si dimostra chiedendo di
aprire un FileGDB e pretendendo che **rifiuti**, con il rifiuto giusto: non un
errore qualunque, ma quello che dice che il tier GDB non e' compilato.

Un rifiuto per la ragione sbagliata -- un percorso inesistente, un formato non
riconosciuto -- passerebbe un controllo che guardasse solo `is_err()`, e non
direbbe nulla su che cosa l'artefatto sa fare.

# Il rapporto

Lo smoke non stampa soltanto: scrive un documento con le misure. Un job verde
non e' un'evidenza verificabile, e cio' che il gate finale riconta sono i
numeri, non l'esito.
"""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import subprocess
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import distribuzione  # noqa: E402 -- dopo sys.path, che e' il punto

CSV_DI_PROVA = "codice,nome,geometry\nA-1,alfa,POINT(11.25 43.77)\nB-2,beta,POINT(12.49 41.90)\n"


def esegui(binario: pathlib.Path, argomenti: list[str], lavoro: pathlib.Path):
    return subprocess.run(
        [str(binario), *argomenti],
        capture_output=True,
        text=True,
        cwd=lavoro,
        env={**os.environ, "NO_COLOR": "1"},
    )


def busta(testo: str) -> dict:
    """Il documento JSON che la CLI scrive; vuoto se non ce n'e' uno.

    `stderr` porta **un solo** documento sul percorso d'errore: e' il contratto,
    e leggerlo come tale e' anche il modo di accorgersi se smettesse di valere.
    """
    testo = testo.strip()
    if not testo:
        return {}
    try:
        return json.loads(testo)
    except json.JSONDecodeError:
        return {"_non_json": testo[:400]}


def smoke_filegdb(binario: pathlib.Path, lavoro: pathlib.Path) -> tuple[list[str], dict]:
    """Scrive e rilegge un FileGDB con un CRS. La promessa del profilo pieno."""
    errori: list[str] = []
    misure: dict = {}
    (lavoro / "sorgente.csv").write_text(CSV_DI_PROVA, encoding="utf-8")

    scrittura = esegui(
        binario,
        ["convert", "sorgente.csv", "uscita.gdb",
         "--in-opt", "wkt_column=geometry", "--assume-crs", "EPSG:4326"],
        lavoro,
    )
    if scrittura.returncode != 0:
        return ([f"la scrittura del FileGDB e' fallita: {busta(scrittura.stderr)}"], misure)
    documento = busta(scrittura.stdout)
    misure["byte_scritti"] = documento.get("bytes_written")
    if not misure["byte_scritti"]:
        errori.append("la conversione dichiara zero byte scritti")

    rilettura = esegui(binario, ["inspect", "uscita.gdb"], lavoro)
    if rilettura.returncode != 0:
        return (errori + [f"la rilettura e' fallita: {busta(rilettura.stderr)}"], misure)
    riletto = json.dumps(busta(rilettura.stdout), ensure_ascii=False)

    # Non basta che il comando esca con zero: un FileGDB vuoto uscirebbe con
    # zero. Si pretendono schema, geometria e CRS -- il CRS in particolare e'
    # cio' che dimostra che PROJ ha trovato le proprie griglie.
    for atteso, che_cosa in (("nome", "un campo dello schema"),
                             ("geometry", "la colonna geometria"),
                             ("4326", "il CRS, cioe' che PROJ sia stato attraversato")):
        if atteso not in riletto:
            errori.append(f"nel FileGDB riletto manca {che_cosa} («{atteso}»)")
    misure["schema_riletto"] = not errori
    return errori, misure


def smoke_base(binario: pathlib.Path, lavoro: pathlib.Path) -> tuple[list[str], dict]:
    """Pretende che FileGDB sia assente, e per la ragione giusta."""
    errori: list[str] = []
    misure: dict = {}

    finto = lavoro / "qualsiasi.gdb"
    finto.mkdir(exist_ok=True)
    esito = esegui(binario, ["inspect", str(finto)], lavoro)
    if esito.returncode == 0:
        return (
            ["il profilo base ha aperto un FileGDB: porta un runtime GDAL che il suo "
             "contratto non prevede, con una superficie e una licenza che chi lo installa "
             "non ha accettato"],
            {"filegdb_assente": False},
        )

    documento = busta(esito.stderr)
    errore = documento.get("error", {})
    messaggio = str(errore.get("message", ""))
    categoria = errore.get("category")
    misure["categoria_del_rifiuto"] = categoria
    misure["codice_del_rifiuto"] = errore.get("code")

    # Il rifiuto giusto, non un rifiuto qualunque. Un percorso inesistente o un
    # formato non riconosciuto passerebbero un controllo che guardasse solo il
    # codice d'uscita, e non direbbero nulla su cio' che l'artefatto sa fare.
    if categoria != "unsupported":
        errori.append(
            f"FileGDB e' rifiutato con categoria «{categoria}» invece di «unsupported»: "
            "il rifiuto non dimostra che il tier GDB manchi, solo che qualcosa e' andato storto"
        )
    if "gdal-backend" not in messaggio:
        errori.append(
            f"il messaggio del rifiuto non nomina il tier mancante: «{messaggio[:120]}»"
        )
    misure["filegdb_assente"] = not errori

    # E la controparte: cio' che il profilo base **deve** saper fare.
    (lavoro / "sorgente.csv").write_text(CSV_DI_PROVA, encoding="utf-8")
    # La destinazione e' GeoParquet e non GeoJSON: GeoJSON impone un CRS suo, e
    # un rifiuto per quello direbbe qualcosa sul formato invece che sul profilo.
    conversione = esegui(
        binario,
        ["convert", "sorgente.csv", "uscita.parquet", "--in-opt", "wkt_column=geometry",
         "--assume-crs", "EPSG:4326"],
        lavoro,
    )
    misure["converte_senza_gdal"] = conversione.returncode == 0
    if conversione.returncode != 0:
        errori.append(
            "il profilo base non converte CSV in GeoParquet: dimostrare che FileGDB manca "
            f"non basta se manca anche il resto. {busta(conversione.stderr)}"
        )
    return errori, misure


def main() -> int:
    a = argparse.ArgumentParser(description=__doc__)
    a.add_argument("--albero", required=True, type=pathlib.Path)
    a.add_argument("--lavoro", required=True, type=pathlib.Path)
    a.add_argument("--referto", type=pathlib.Path, default=None)
    a.add_argument(
        "--smoke-prima-della-firma",
        action="store_true",
        help=(
            "dichiara che questo smoke ha girato prima della firma. Il gate finale lo "
            "rifiuta su una candidate: un binario firmato e' un altro file"
        ),
    )
    arg = a.parse_args()

    albero = arg.albero.resolve()
    manifesto_percorso = albero / "MANIFEST.json"
    if not manifesto_percorso.is_file():
        sys.exit(f"{manifesto_percorso} assente: senza manifesto non si sa che cosa promette")
    manifesto = json.loads(manifesto_percorso.read_text(encoding="utf-8"))
    profilo = manifesto["profilo"]

    binario = albero / "bin" / ("plenora-io.exe" if os.name == "nt" else "plenora-io")
    if not binario.is_file():
        sys.exit(f"{binario} assente")

    lavoro = arg.lavoro.resolve()
    lavoro.mkdir(parents=True, exist_ok=True)

    print(f"smoke del profilo «{profilo}» su {binario}")
    if profilo == "filegdb":
        errori, misure = smoke_filegdb(binario, lavoro)
    elif profilo == "base":
        errori, misure = smoke_base(binario, lavoro)
    else:
        sys.exit(f"profilo sconosciuto: {profilo}")

    # La firma entra fra le misure: e' il referto che il gate finale interroga
    # su di essa, perche' e' l'unico che esiste per entrambi i profili ed e'
    # l'ultimo passo nell'ordine deciso -- assembla, firma, checksum, smoke.
    misure["firma"] = manifesto.get("firma", {"stato": "non_dichiarata"})
    if arg.smoke_prima_della_firma:
        misure["firma"] = {**misure["firma"], "smoke_prima_della_firma": True}

    for chiave, valore in sorted(misure.items()):
        print(f"  {chiave}: {valore}")

    if arg.referto:
        distribuzione.scrivi_referto(
            arg.referto,
            verifica="smoke-profilo",
            piattaforma=manifesto["piattaforma"],
            profilo=profilo,
            canale=manifesto["canale"],
            esito="verde" if not errori else "rosso",
            misure=misure,
            errori=errori,
            note=(
                "il profilo `base` e' verde quando FileGDB **manca**, e con il rifiuto "
                "giusto; il profilo `filegdb` quando un FileGDB e' scritto e riletto. Sono "
                "due promesse opposte, e un solo esito non le distinguerebbe."
            ),
        )

    if errori:
        print("\n--- ROSSO ---")
        for errore in errori:
            print(f"  {errore}")
        return 1
    promessa = (
        "FileGDB e' scritto e riletto" if profilo == "filegdb"
        else "FileGDB e' assente, e il rifiuto lo dice"
    )
    print(f"lo smoke conferma la promessa del profilo: {promessa}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
