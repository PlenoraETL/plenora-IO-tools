#!/usr/bin/env python3
"""Le buste che il manifesto v2 descrive sono quelle che il binario emette.

# Che cosa protegge

`release/cli-protocol-v2.json` e' il contratto su cui un consumatore scrive il
proprio parser. Fino a questo gate ne descriveva il **primo livello**: per
`read`, nove chiavi; per `convert`, quattordici. Sotto quelle chiavi c'e' tutto
il resto -- `layer.geometry.crs_resolution.definition_format`, gli indici
opzionali di una ragione, la forma di un esempio di perdita -- e nessuna riga
del contratto lo diceva.

Un consumatore che scrive il proprio tipo su quel documento lo deriva percio'
dall'osservazione, cioe' dagli esempi che ha visto passare. Il contratto non
gliene risponde, e il giorno in cui un campo cambia forma nessuno dei due se ne
accorge: il manifesto non lo nominava, e le sonde della CLI verificano che i
campi dichiarati **ci siano**, non che non ce ne siano altri.

# Perche' esegue il binario

Le sonde di `plenora-io-cli` costruiscono le buste in-process. E' utile e non
e' la stessa cosa: verificano che una funzione produca un documento, non che il
comando che un utente digita lo consegni. Fra le due c'e' il dispatch, la scelta
del protocollo, la scrittura su stdout invece che su stderr e il codice
d'uscita.

Questo gate esegue il **binario compilato** su fixture versionate e legge cio'
che esce dai due flussi. E' la sola forma in cui l'affermazione «la CLI emette
questo» si puo' verificare invece di dedurla.

# Il confronto e' nei due versi, e sono due difetti diversi

* un percorso **osservato e non dichiarato** e' un campo che il prodotto emette
  e il contratto non promette. E' il verso che protegge chi legge: consuma un
  campo che nessuno si e' impegnato a mantenere;
* un percorso **dichiarato e non osservato** e' una promessa che nessuna
  esecuzione onora. Puo' voler dire che il campo e' sparito dal codice, o che
  la matrice non lo raggiunge piu': in entrambi i casi il contratto afferma
  qualcosa che nessuno sta verificando.

Il secondo verso e' quello che rende la matrice onesta. Senza, si potrebbe
dichiarare qualunque cosa e non esercitarla mai.

# `sempre` e' esatto, non un minimo

Un percorso e' dichiarato `sempre: true` quando compare in **ogni** busta di
quel contratto che la matrice produce, `false` quando in almeno una manca. Il
gate pretende che la dichiarazione coincida con l'osservazione, nei due sensi.

Dichiarare `false` un campo che c'e' sempre sarebbe un'affermazione piu' debole
del vero -- non falsa -- e proprio per questo lascerebbe passare in silenzio il
giorno in cui il campo diventasse davvero condizionale. La coerenza esatta
costringe invece la matrice a contenere il caso che rende un campo opzionale, o
a non dichiararlo tale.

# Che cosa non guarda

I **valori**. Che `status` sia una stringa lo dice questo gate; che valga `ok` o
`error` lo dicono le sonde della CLI e il vocabolario chiuso dei contratti. Qui
si confronta la forma, e allargarlo ai valori gli darebbe il credito di
verifiche che non fa.

Non guarda nemmeno se la matrice **basti**. Che ogni busta possibile sia
rappresentata non e' dimostrabile da qui: si dimostra che cio' che la matrice
raggiunge e cio' che il contratto dichiara sono la stessa cosa. Il resto lo
tiene il verso inverso, che rende visibile ogni dichiarazione non esercitata.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
CONTRATTO = ROOT / "release" / "cli-protocol-v2.json"
FIXTURE = ROOT / "crates" / "plenora-io-cli" / "tests" / "fixtures"
CANONICHE = FIXTURE / "canoniche"
OSTILI = FIXTURE / "ostili"

#: I nomi JSON dei tipi, dai tipi Python in cui `json` li decodifica.
#:
#: `bool` prima di `int`: in Python un booleano **e'** un intero, e l'ordine di
#: un dizionario per tipo non lo distinguerebbe.
def _tipo(valore: Any) -> str:
    if valore is None:
        return "null"
    if isinstance(valore, bool):
        return "boolean"
    if isinstance(valore, int):
        return "integer"
    if isinstance(valore, float):
        return "number"
    if isinstance(valore, str):
        return "string"
    if isinstance(valore, list):
        return "array"
    return "object"


#: Gli oggetti le cui **chiavi sono dati**, non contratto.
#:
#: `row_diagnostics.counts` associa una causa al proprio conteggio, e le cause
#: le decide il driver che ha respinto la riga: `kml.invalid_placemark` oggi,
#: un'altra domani, e nessuna delle due e' una promessa del wire. Esplorarle
#: come si esplora un oggetto normale metterebbe un dato dentro il contratto --
#: il manifesto dichiarerebbe che esiste la chiave `kml.invalid_placemark` --
#: e il gate diventerebbe rosso al primo file che ne porta un'altra.
#:
#: Collassano percio' su `{}`, come gli elementi di un array su `[]`: il
#: contratto dice che c'e' una mappa e di che tipo sono i suoi valori, e i nomi
#: delle chiavi li governa il vocabolario delle cause, che sta altrove.
MAPPE: frozenset[str] = frozenset({".error.row_diagnostics.counts"})


def forma(valore: Any, prefisso: str = "", out: dict[str, set[str]] | None = None):
    """I percorsi di un documento, con i tipi osservati a ciascuno.

    Gli elementi di un array collassano su un unico percorso `[]`: il contratto
    parla del **tipo** dell'elemento, non della sua posizione, e distinguere
    `layers[0]` da `layers[1]` produrrebbe una struttura che dipende da quante
    righe aveva la fixture. Le chiavi degli oggetti elencati in `MAPPE`
    collassano su `{}` per la stessa ragione.
    """
    if out is None:
        out = {}
    if isinstance(valore, dict):
        for chiave, dentro in valore.items():
            percorso = f"{prefisso}{{}}" if prefisso in MAPPE else f"{prefisso}.{chiave}"
            out.setdefault(percorso, set()).add(_tipo(dentro))
            forma(dentro, percorso, out)
    elif isinstance(valore, list):
        for dentro in valore:
            percorso = f"{prefisso}[]"
            out.setdefault(percorso, set()).add(_tipo(dentro))
            forma(dentro, percorso, out)
    return out


#: I casi che la matrice esegue, e che cosa ciascuno esiste per raggiungere.
#:
#: Non e' un elenco di comandi: e' l'insieme che rende **esatta** la colonna
#: `sempre`. Ogni caso qui sta perche' porta una busta che gli altri non
#: portano -- un campo condizionale presente o assente, un flusso invece
#: dell'altro -- e toglierne uno sposta una dichiarazione da `false` a `true`,
#: che il gate vede.
#:
#: `busta` dice quale documento il caso deve produrre, ed e' verificato. Senza,
#: una fixture cancellata farebbe uscire una busta d'**errore** al posto di
#: quella attesa, e il gate resterebbe verde: la busta d'errore e' dichiarata
#: come le altre, quindi tutti i suoi percorsi tornerebbero. Il caso sparirebbe
#: dalla matrice in silenzio, e con lui la copertura che porta.
MATRICE: tuple[dict[str, Any], ...] = (
    {
        "nome": "catalogo",
        "busta": "plenora-io-catalog-v2",
        "argomenti": ["catalog"],
        "perche": "l'unica busta del comando: non prende argomenti.",
    },
    {
        "nome": "inspect-geojson",
        "busta": "plenora-io-inspect-v2",
        "argomenti": ["inspect", "{canoniche}/canonico.geojson"],
        "perche": "un descrittore con CRS fisso e un layer con geometria.",
    },
    {
        "nome": "inspect-gpkg",
        "busta": "plenora-io-inspect-v2",
        "argomenti": ["inspect", "{canoniche}/canonico.gpkg"],
        "perche": (
            "un descrittore con CRS incorporato e opzioni di formato diverse: "
            "il descrittore e' lo stesso tipo del catalogo, e un solo driver "
            "non ne esercita i rami."
        ),
    },
    {
        "nome": "inspect-shp",
        "busta": "plenora-io-inspect-v2",
        "argomenti": [
            "inspect",
            "{canoniche}/canonico_punti.shp",
            "--assume-crs",
            "EPSG:3003",
        ],
        "perche": (
            "l'unico descrittore con limiti di nome interi: il DBF li ha, e "
            "senza questo caso `field_names.max_bytes` uscirebbe solo `null`. "
            "`--assume-crs` e' necessario e non incidentale: il `.prj` di "
            "questa fixture porta una definizione senza autorita', e il driver "
            "rifiuta chiuso invece di indovinare un codice EPSG. E' una "
            "decisione di prodotto, non un difetto da aggirare qui."
        ),
    },
    {
        "nome": "inspect-parquet",
        "busta": "plenora-io-inspect-v2",
        "argomenti": ["inspect", "{canoniche}/canonico.parquet"],
        "perche": (
            "l'unico descrittore che dichiara una versione di specifica: senza, "
            "`spec_version_supported` uscirebbe solo `null`."
        ),
    },
    {
        "nome": "layers-gpkg",
        "busta": "plenora-io-layers-v2",
        "argomenti": ["layers", "{canoniche}/canonico.gpkg"],
        "perche": "il riassunto per layer, che non porta i campi.",
    },
    {
        "nome": "layers-geojson",
        "busta": "plenora-io-layers-v2",
        "argomenti": ["layers", "{canoniche}/canonico.geojson"],
        "perche": "lo stesso comando su un formato a layer unico.",
    },
    {
        "nome": "read-geojson",
        "busta": "plenora-io-read-v2",
        "argomenti": ["read", "{canoniche}/canonico.geojson"],
        "perche": "una lettura intera: `truncated` falso.",
    },
    {
        "nome": "read-troncato",
        "busta": "plenora-io-read-v2",
        "argomenti": ["read", "{canoniche}/canonico.geojson", "--limit", "1"],
        "perche": (
            "`truncated` vero. E' l'unico caso che lo porta: senza, il campo "
            "sarebbe dichiarato su un solo valore osservato."
        ),
    },
    {
        "nome": "read-parquet",
        "busta": "plenora-io-read-v2",
        "argomenti": ["read", "{canoniche}/canonico_pieno.parquet"],
        "perche": "uno schema piu' ricco, e un CRS risolto per identificatore.",
    },
    {
        "nome": "convert-geojson-csv",
        "busta": "plenora-io-convert-v2",
        "argomenti": [
            "convert",
            "{canoniche}/canonico.geojson",
            "{uscita}/da-geojson.csv",
        ],
        "perche": (
            "una conversione pubblicata con perdita in scrittura: e' il caso "
            "che porta `write_loss.counts` ed `esempi` non vuoti, e le ragioni "
            "con `field_index` e `layer_index`."
        ),
    },
    {
        "nome": "convert-geojson-geojson",
        "busta": "plenora-io-convert-v2",
        "argomenti": [
            "convert",
            "{canoniche}/canonico.geojson",
            "{uscita}/da-geojson.geojson",
        ],
        "perche": (
            "la stessa conversione senza perdita di CRS: `write_loss.counts` "
            "resta vuoto, ed e' cio' che rende osservabile che i suoi percorsi "
            "interni non ci sono sempre."
        ),
    },
    {
        "nome": "errore-semplice",
        "busta": "plenora-io-error-v1",
        "argomenti": ["read", "{uscita}/non-esiste.geojson"],
        "perche": "la busta d'errore nella sua forma minima: sei chiavi.",
    },
    {
        "nome": "errore-con-diagnostica-di-riga",
        "busta": "plenora-io-error-v1",
        "argomenti": ["read", "{ostili}/lettura.kml"],
        "perche": (
            "la busta d'errore con `row_diagnostics`, la settima chiave. Il "
            "manifesto affermava sei chiavi «e nessuna in piu'»: questo caso "
            "e' la ragione per cui l'affermazione non reggeva."
        ),
    },
    {
        "nome": "errore-d-uso",
        "busta": "plenora-io-error-v1",
        "argomenti": ["--opzione-che-non-esiste"],
        "perche": "la via d'uso, che passa da `usage_err` e non da `map_err`.",
    },
    {
        "nome": "versione",
        "busta": "senza-contratto:--version",
        "argomenti": ["--version"],
        "perche": (
            "la sesta busta su stdout, e la sola senza `contract` ne' "
            "`protocol_version`. Non la censiva nessuno."
        ),
    },
)


def binario() -> str:
    """Il binario da esercitare: quello indicato, o quello appena costruito.

    `cargo build` e' eseguito qui invece che pretendere un artefatto gia'
    pronto, perche' un gate che verifica «cio' che il binario emette» non puo'
    dipendere da chi lo ha costruito e quando.
    """
    indicato = os.environ.get("PLENORA_IO_BIN")
    if indicato:
        return indicato

    esito = subprocess.run(
        [
            "cargo",
            "build",
            "-p",
            "plenora-io-cli",
            "--message-format=json-render-diagnostics",
        ],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if esito.returncode != 0:
        raise SystemExit(f"il binario non si costruisce:\n{esito.stderr}")
    for riga in esito.stdout.splitlines():
        try:
            messaggio = json.loads(riga)
        except json.JSONDecodeError:
            continue
        if (
            messaggio.get("reason") == "compiler-artifact"
            and messaggio.get("target", {}).get("name") == "plenora-io"
            and messaggio.get("executable")
        ):
            return messaggio["executable"]
    raise SystemExit("cargo non ha dichiarato l'eseguibile di plenora-io")


def esegui(percorso_binario: str, uscita: Path) -> list[dict[str, Any]]:
    """La matrice, eseguita. Ogni caso torna con la busta che ha prodotto.

    La busta e' cercata su **stdout e poi stderr**, nell'ordine in cui il
    protocollo le colloca: il successo va su stdout, l'errore su stderr. Un
    caso che non produce JSON su nessuno dei due e' un difetto e viene detto,
    non saltato.
    """
    sostituzioni = {
        "canoniche": CANONICHE.as_posix(),
        "ostili": OSTILI.as_posix(),
        "uscita": uscita.as_posix(),
    }
    osservazioni = []
    for caso in MATRICE:
        argomenti = [a.format(**sostituzioni) for a in caso["argomenti"]]
        esito = subprocess.run(
            [percorso_binario, *argomenti],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
        documento = None
        flusso = None
        for nome, testo in (("stdout", esito.stdout), ("stderr", esito.stderr)):
            if not testo.strip():
                continue
            try:
                documento = json.loads(testo)
                flusso = nome
                break
            except json.JSONDecodeError:
                continue
        osservazioni.append(
            {
                "caso": caso["nome"],
                "attesa": caso["busta"],
                "argomenti": argomenti,
                "exit": esito.returncode,
                "flusso": flusso,
                "documento": documento,
            }
        )
    return osservazioni


def raggruppa(osservazioni: list[dict[str, Any]]) -> tuple[dict[str, dict], list[str]]:
    """Le osservazioni per busta, con i percorsi visti e quelli visti sempre.

    La chiave e' il `contract` della busta, che e' l'identita' che il documento
    dichiara di se'. `--version` non ne ha uno -- ed e' il difetto che questo
    censimento ha trovato -- quindi la sua busta si raccoglie sotto un nome
    riservato, che il manifesto usa per nominarla.
    """
    per_busta: dict[str, dict] = {}
    problemi: list[str] = []
    for osservazione in osservazioni:
        documento = osservazione["documento"]
        if documento is None:
            problemi.append(
                f"il caso «{osservazione['caso']}» non ha prodotto JSON su "
                f"nessuno dei due flussi (exit {osservazione['exit']})"
            )
            continue
        nome = documento.get("contract") or "senza-contratto:--version"
        if nome != osservazione["attesa"]:
            problemi.append(
                f"il caso «{osservazione['caso']}» doveva produrre "
                f"«{osservazione['attesa']}» e ha prodotto «{nome}»"
                + (
                    f": {documento.get('error', {}).get('message', '')}"
                    if nome == "plenora-io-error-v1"
                    else ""
                )
            )
        stato = per_busta.setdefault(
            nome, {"osservati": {}, "in_tutte": None, "casi": [], "flussi": set()}
        )
        stato["casi"].append(osservazione["caso"])
        stato["flussi"].add(osservazione["flusso"])
        percorsi = forma(documento)
        for percorso, tipi in percorsi.items():
            stato["osservati"].setdefault(percorso, set()).update(tipi)
        visti = set(percorsi)
        stato["in_tutte"] = visti if stato["in_tutte"] is None else stato["in_tutte"] & visti
    return per_busta, problemi


def confronta(nome: str, dichiarata: dict[str, dict], stato: dict) -> list[str]:
    """I due versi, e la colonna `sempre`."""
    problemi: list[str] = []
    osservati = stato["osservati"]
    in_tutte = stato["in_tutte"] or set()

    for percorso in sorted(set(osservati) - set(dichiarata)):
        problemi.append(
            f"{nome}: il binario emette «{percorso}» e il manifesto non lo "
            f"dichiara (tipi osservati: {sorted(osservati[percorso])})"
        )
    for percorso in sorted(set(dichiarata) - set(osservati)):
        problemi.append(
            f"{nome}: il manifesto dichiara «{percorso}» e nessun caso della "
            "matrice lo produce"
        )
    for percorso in sorted(set(osservati) & set(dichiarata)):
        visti = osservati[percorso]
        detti = dichiarata[percorso]["tipi"]
        if visti - detti:
            problemi.append(
                f"{nome}: «{percorso}» esce come {sorted(visti - detti)}, che "
                f"il manifesto non dichiara (dice {sorted(detti)})"
            )
        if detti - visti:
            problemi.append(
                f"{nome}: «{percorso}» e' dichiarato {sorted(detti - visti)} e "
                "nessun caso lo produce con quel tipo"
            )
        sempre_davvero = percorso in in_tutte
        if dichiarata[percorso]["sempre"] != sempre_davvero:
            problemi.append(
                f"{nome}: «{percorso}» e' dichiarato "
                f"sempre={dichiarata[percorso]['sempre']} e la matrice lo "
                f"osserva sempre={sempre_davvero}"
            )
    return problemi


def censimento(per_busta: dict[str, dict]) -> dict[str, Any]:
    """La struttura osservata, nella forma che il manifesto dichiara.

    Serve a **scrivere** il manifesto la prima volta e a rivederlo quando il
    wire cambia per una decisione. Non e' il gate: un censimento che si
    riscrivesse da solo confronterebbe il binario con se stesso.
    """
    fuori: dict[str, Any] = {}
    for nome, stato in sorted(per_busta.items()):
        in_tutte = stato["in_tutte"] or set()
        fuori[nome] = {
            percorso: {
                "tipi": sorted(tipi),
                "sempre": percorso in in_tutte,
            }
            for percorso, tipi in sorted(stato["osservati"].items())
        }
    return fuori


def main(argv: list[str] | None = None) -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument(
        "--censisci",
        action="store_true",
        help="stampa la struttura osservata invece di confrontarla",
    )
    opzioni = argomenti.parse_args(argv)

    percorso_binario = binario()
    with tempfile.TemporaryDirectory(prefix="plenora-buste-") as temporanea:
        osservazioni = esegui(percorso_binario, Path(temporanea))
    per_busta, problemi = raggruppa(osservazioni)

    if opzioni.censisci:
        json.dump(censimento(per_busta), sys.stdout, indent=2, ensure_ascii=False)
        sys.stdout.write("\n")
        return 0

    manifesto = json.loads(CONTRATTO.read_text(encoding="utf-8"))
    dichiarate = {
        voce["contract"]: voce
        for voce in manifesto.get("envelopes", {}).values()
        if "contract" in voce
    }
    # La busta d'errore non sta in `envelopes`: il v2 la riusa dal v1 senza
    # modifiche e la descrive in una sezione propria. Il gate la prende di li'
    # invece di pretendere che il manifesto la ripeta dove non le compete.
    errore = manifesto.get("busta_degli_errori")
    if errore and "contract" in errore:
        dichiarate[errore["contract"]] = errore
    for nome_riservato, voce in manifesto.get("buste_senza_contratto", {}).items():
        dichiarate[f"senza-contratto:{nome_riservato}"] = voce

    for nome in sorted(set(per_busta) - set(dichiarate)):
        problemi.append(
            f"la matrice produce la busta «{nome}» e il manifesto non la "
            "dichiara affatto"
        )
    for nome in sorted(set(dichiarate) - set(per_busta)):
        if "struttura" in dichiarate[nome]:
            problemi.append(
                f"il manifesto dichiara la struttura di «{nome}» e nessun caso "
                "della matrice la produce"
            )

    for nome in sorted(set(per_busta) & set(dichiarate)):
        voce = dichiarate[nome]
        if "struttura" not in voce:
            problemi.append(
                f"la busta «{nome}» e' nominata dal manifesto senza una "
                "`struttura`: il primo livello da solo non descrive cio' che "
                "un consumatore deve leggere"
            )
            continue
        dichiarata = {
            percorso: {"tipi": set(v["tipi"]), "sempre": v["sempre"]}
            for percorso, v in voce["struttura"].items()
        }
        problemi.extend(confronta(nome, dichiarata, per_busta[nome]))

    if problemi:
        for problema in problemi:
            print(problema, file=sys.stderr)
        print(
            f"\n{len(problemi)} divergenze fra il manifesto v2 e il binario.",
            file=sys.stderr,
        )
        return 1

    percorsi = sum(len(s["osservati"]) for s in per_busta.values())
    print(
        f"buste v2 verificate: {len(MATRICE)} casi, {len(per_busta)} buste, "
        f"{percorsi} percorsi confrontati col manifesto."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
