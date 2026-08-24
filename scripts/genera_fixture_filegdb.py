#!/usr/bin/env python3
"""La fixture FileGDB del target `filegdb_reader`, e la prova che e' riproducibile.

# Perche' una fixture, e non un blob costruito dal fuzzer

Uno Shapefile si puo' scrivere dalla specifica: il formato e' pubblicato, e il
generatore dei semi di `shp_reader` lo fa in trecento righe. Un FileGDB no. E'
un formato proprietario, ricostruito per reverse engineering, fatto di una
**directory** di tabelle che si citano a vicenda per GUID. Scriverlo a mano
significherebbe riscrivere `OpenFileGDB`, e i semi sarebbero validi per
costruzione rispetto alla nostra idea del formato, non a quella di GDAL.

La fixture e' percio' un FileGDB **vero**, prodotto da GDAL stesso a partire da
un GeoJSON committato. Il fuzzer non la costruisce: la riceve intatta e ne
sostituisce **un file per volta**.

# In che senso e' riproducibile

`ogr2ogr` e' deterministico su questo input tranne che per i GUID che conia per
il dataset: quarantotto byte per tabella di metadati, tre identificatori da
sedici, scritti in `GDB_Items` e ripetuti in `GDB_ItemRelationships`. Tutto il
resto -- l'XML dei descrittori, le tabelle dei dati, l'indice spaziale -- e'
identico byte a byte fra due corse.

Dichiararlo non basterebbe. `--verifica` lo **dimostra**: rigenera la fixture
**due** volte e prende come «byte coniati» esattamente gli offset in cui le due
rigenerazioni differiscono fra loro. Poi confronta la fixture committata con la
prima rigenerazione e pretende che le differenze stiano dentro quell'insieme.

Un byte che cambia fra due rigenerazioni e' coniato; un byte che e' stabile fra
le rigenerazioni ma diverso da quello committato e' una divergenza vera -- una
versione di GDAL diversa, un input cambiato, una fixture modificata a mano -- e
diventa rosso. La tolleranza e' cosi' **derivata** da cio' che GDAL fa oggi, non
scritta da noi.

# La forma dell'archivio

La fixture e' una directory, e il target la vuole dentro il binario. Le venti
parti stanno percio' in un archivio con un indice, ordinato per nome:

```text
PLENORA-GDB-FIXTURE-1\\n
u32          numero di file
per file:    u16 lunghezza del nome, nome ASCII, u32 lunghezza, byte
```

L'ordinamento e' cio' che rende l'archivio deterministico a parita' di
contenuto, e l'indice e' cio' che permette al target di scegliere quale file
sostituire senza costruire un percorso dal payload.

# Uso

    python3 scripts/genera_fixture_filegdb.py --scrivi <directory.gdb>
    python3 scripts/genera_fixture_filegdb.py --verifica
    python3 scripts/genera_fixture_filegdb.py --confronta <gdb-1> <gdb-2>

`--scrivi` e `--confronta` vogliono un FileGDB gia' prodotto da GDAL: li produce
`scripts/genera-fixture-filegdb.sh`, che e' il solo posto in cui `ogr2ogr` viene
invocato.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import struct
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
ARCHIVIO = ROOT / "fuzz" / "fixtures" / "filegdb" / "citta.gdb.bundle"
# La **sorgente** da cui GDAL produce la fixture. Sta nel verbale perche' e' un
# ingresso della riproducibilita' quanto la versione di GDAL: cambiarla cambia
# ogni byte della fixture, e un verbale che le sopravvivesse direbbe
# «riproducibile» di un confronto fatto su un altro input.
SORGENTE = ROOT / "fuzz" / "fixtures" / "filegdb" / "citta.geojson"
# La prova della riproducibilita', registrata. Il confronto vuole GDAL e due
# rigenerazioni; il gate che lo rilegge no, e gira ovunque.
PROVA = ROOT / "assurance" / "fixture-filegdb.json"

INTESTAZIONE = b"PLENORA-GDB-FIXTURE-1\n"

# I nomi che una parte di FileGDB puo' avere. Non e' una convenzione nostra: e'
# cio' che `OpenFileGDB` scrive, e limitarlo qui impedisce che un archivio
# costruito altrove faccia scrivere al target un nome qualunque.
CARATTERI_AMMESSI = set("abcdefghijklmnopqrstuvwxyz0123456789._")


def nome_di_parte_ammesso(nome: str) -> bool:
    """La stessa regola che applica il target, e per la stessa ragione.

    Ogni parte che `OpenFileGDB` scrive comincia con una lettera minuscola --
    `gdb`, `timestamps`, `a00000001.gdbtable`. La condizione sulla **prima**
    lettera non e' estetica: senza, `".."` sarebbe fatto di soli caratteri
    ammessi e passerebbe, e ogni nome finisce in un `join`.

    La regola sta in due posti perche' i due posti servono a cose diverse: qui
    protegge chi **costruisce** l'archivio, in `driver-filegdb` protegge chi lo
    **legge**. Che siano la stessa regola lo prova una sonda per parte.
    """
    return (
        bool(nome)
        and nome[0].isascii()
        and nome[0].islower()
        and nome[0].isalpha()
        and not set(nome) - CARATTERI_AMMESSI
    )


def parti(directory: pathlib.Path) -> dict[str, bytes]:
    """Le parti del FileGDB, per nome. Solo file, nessuna ricorsione."""
    trovate: dict[str, bytes] = {}
    for percorso in sorted(directory.iterdir()):
        if not percorso.is_file() or percorso.is_symlink():
            continue
        nome = percorso.name
        if not nome_di_parte_ammesso(nome):
            raise ValueError(f"nome di parte non ammesso: {nome!r}")
        trovate[nome] = percorso.read_bytes()
    if not trovate:
        raise ValueError(f"{directory}: nessuna parte da archiviare")
    return trovate


def impacchetta(contenuti: dict[str, bytes]) -> bytes:
    corpo = [INTESTAZIONE, struct.pack("<I", len(contenuti))]
    for nome, byte in sorted(contenuti.items()):
        grezzo = nome.encode("ascii")
        corpo.append(struct.pack("<H", len(grezzo)))
        corpo.append(grezzo)
        corpo.append(struct.pack("<I", len(byte)))
        corpo.append(byte)
    return b"".join(corpo)


def spacchetta(archivio: bytes) -> dict[str, bytes]:
    if not archivio.startswith(INTESTAZIONE):
        raise ValueError("archivio senza intestazione")
    posizione = len(INTESTAZIONE)
    (quante,) = struct.unpack_from("<I", archivio, posizione)
    posizione += 4
    contenuti: dict[str, bytes] = {}
    for _ in range(quante):
        (lunghezza_nome,) = struct.unpack_from("<H", archivio, posizione)
        posizione += 2
        nome = archivio[posizione : posizione + lunghezza_nome].decode("ascii")
        if not nome_di_parte_ammesso(nome):
            raise ValueError(f"nome di parte non ammesso: {nome!r}")
        posizione += lunghezza_nome
        (lunghezza,) = struct.unpack_from("<I", archivio, posizione)
        posizione += 4
        contenuti[nome] = archivio[posizione : posizione + lunghezza]
        posizione += lunghezza
    if posizione != len(archivio):
        raise ValueError("archivio con byte di coda non dichiarati")
    return contenuti


def offset_coniati(uno: bytes, due: bytes) -> set[int]:
    """Gli offset in cui due rigenerazioni differiscono fra loro.

    Sono i byte che GDAL conia a ogni corsa. Derivarli dal confronto e' l'unico
    modo di non doverli scrivere a mano: una tolleranza scritta a mano
    resterebbe ferma il giorno in cui GDAL ne conia uno in piu'.
    """
    if len(uno) != len(due):
        # Una differenza di lunghezza non e' un byte coniato: e' un'altra
        # fixture. Nessun offset e' tollerato.
        return set()
    return {i for i in range(len(uno)) if uno[i] != due[i]}


def confronta(
    committata: dict[str, bytes],
    prima: dict[str, bytes],
    seconda: dict[str, bytes],
) -> list[str]:
    """La fixture committata e' quella che GDAL produce, salvo i byte coniati."""
    errori: list[str] = []

    if set(prima) != set(seconda):
        return ["due rigenerazioni con parti diverse: GDAL non e' stabile nemmeno nei nomi"]
    mancanti = sorted(set(prima) - set(committata))
    estranee = sorted(set(committata) - set(prima))
    if mancanti:
        errori.append(f"parti prodotte da GDAL e assenti dalla fixture: {mancanti}")
    if estranee:
        errori.append(f"parti nella fixture che GDAL non produce: {estranee}")
    if errori:
        return errori

    for nome in sorted(prima):
        coniati = offset_coniati(prima[nome], seconda[nome])
        atteso, ottenuto = prima[nome], committata[nome]
        if len(atteso) != len(ottenuto):
            errori.append(
                f"{nome}: la fixture ha {len(ottenuto)} byte, GDAL ne produce "
                f"{len(atteso)}. Una differenza di lunghezza non e' un byte coniato."
            )
            continue
        diversi = {i for i in range(len(atteso)) if atteso[i] != ottenuto[i]}
        stabili = sorted(diversi - coniati)
        if stabili:
            errori.append(
                f"{nome}: {len(stabili)} byte differiscono dalla fixture e sono "
                f"**stabili** fra due rigenerazioni (primo offset {stabili[0]}). "
                "Un byte stabile e diverso non e' coniato da GDAL: o la fixture "
                "e' stata modificata, o l'input e la versione di GDAL non sono "
                "piu' quelli con cui e' stata prodotta."
            )
    return errori


def registra(uno: pathlib.Path, due: pathlib.Path, versione_gdal: str) -> int:
    """Scrive che cosa il confronto fra due rigenerazioni ha trovato.

    Senza questo artefatto, l'invariante «la fixture e' riproducibile» avrebbe
    per prova un gate che rilegge l'archivio -- cioe' proverebbe che l'archivio
    e' ben formato, che e' un'altra affermazione. La riproducibilita' si
    dimostra rigenerando, e rigenerare vuole GDAL: cio' che resta al gate e'
    **rileggere il verbale**, legato ai byte della fixture che descrive.
    """
    committata = spacchetta(ARCHIVIO.read_bytes())
    errori = confronta(committata, parti(uno), parti(due))
    if errori:
        for messaggio in errori:
            print(messaggio, file=sys.stderr)
        return 1

    prima, seconda = parti(uno), parti(due)
    coniati = {
        nome: sorted(offset_coniati(prima[nome], seconda[nome]))
        for nome in sorted(prima)
    }
    documento = {
        "schema_version": 1,
        "descrizione": (
            "La prova che la fixture FileGDB e' riproducibile. Prodotta da "
            "scripts/genera-fixture-filegdb.sh rigenerandola **due** volte; "
            "letta da `genera_fixture_filegdb.py --verifica`, che la lega ai "
            "byte della fixture."
        ),
        "come_e_stata_ottenuta": (
            "due rigenerazioni indipendenti con la stessa versione di GDAL. Gli "
            "offset elencati qui sotto sono quelli in cui le due differiscono "
            "**fra loro**: sono i byte che GDAL conia a ogni corsa, e sono la "
            "sola tolleranza ammessa nel confronto con la fixture committata. Un "
            "byte stabile fra le rigenerazioni e diverso da quello committato non "
            "e' coniato, ed e' rosso."
        ),
        "versione_gdal": versione_gdal,
        "impronta_della_sorgente": hashlib.sha256(SORGENTE.read_bytes()).hexdigest(),
        "impronta_della_fixture": hashlib.sha256(ARCHIVIO.read_bytes()).hexdigest(),
        "parti": len(committata),
        "byte_coniati_per_parte": {
            nome: len(offset) for nome, offset in coniati.items() if offset
        },
        "byte_coniati_totali": sum(len(o) for o in coniati.values()),
        "offset_coniati": {nome: offset for nome, offset in coniati.items() if offset},
    }
    PROVA.parent.mkdir(parents=True, exist_ok=True)
    PROVA.write_text(
        json.dumps(documento, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
        newline="\n",
    )
    print(
        f"riproducibilita' registrata in {PROVA.relative_to(ROOT).as_posix()}: "
        f"{documento['byte_coniati_totali']} byte coniati su "
        f"{len(documento['byte_coniati_per_parte'])} parti, con {versione_gdal}"
    )
    return 0


def scrivi(directory: pathlib.Path) -> int:
    contenuti = parti(directory)
    archivio = impacchetta(contenuti)
    ARCHIVIO.parent.mkdir(parents=True, exist_ok=True)
    ARCHIVIO.write_bytes(archivio)
    print(
        f"fixture archiviata in {ARCHIVIO.relative_to(ROOT).as_posix()}: "
        f"{len(contenuti)} parti, {len(archivio)} byte, "
        f"sha256 {hashlib.sha256(archivio).hexdigest()[:16]}"
    )
    return 0


def verifica_forma() -> int:
    """Il controllo che costa millisecondi: l'archivio si rilegge intero.

    Non dice che la fixture sia quella che GDAL produce -- per quello serve
    GDAL, e serve `--confronta`. Dice che l'archivio non e' troncato, che i nomi
    sono nomi di parte e che il target trovera' cio' che si aspetta.
    """
    if not ARCHIVIO.exists():
        print(f"{ARCHIVIO}: fixture assente", file=sys.stderr)
        return 1
    try:
        contenuti = spacchetta(ARCHIVIO.read_bytes())
    except (ValueError, struct.error, UnicodeDecodeError) as errore:
        print(f"{ARCHIVIO}: archivio illeggibile ({errore})", file=sys.stderr)
        return 1

    errori: list[str] = []
    for nome in sorted(contenuti):
        if not nome_di_parte_ammesso(nome):
            errori.append(f"nome di parte non ammesso: {nome!r}")
    # Un FileGDB senza il file `gdb` non e' un FileGDB, e senza almeno una
    # tabella non ha niente da leggere: sono le due condizioni che rendono la
    # fixture una base di partenza invece di una directory qualunque.
    if "gdb" not in contenuti:
        errori.append("la fixture non contiene il file `gdb`: non e' un FileGDB")
    if not any(nome.endswith(".gdbtable") for nome in contenuti):
        errori.append("la fixture non contiene nessuna tabella `.gdbtable`")
    if not contenuti:
        errori.append(
            "archivio senza parti: il target sceglierebbe una parte **modulo "
            "zero**, cioe' dividerebbe per zero"
        )

    errori.extend(_prova_legata_alla_fixture(contenuti))

    for messaggio in errori:
        print(messaggio, file=sys.stderr)
    if errori:
        return 1
    prova = json.loads(PROVA.read_text(encoding="utf-8"))
    print(
        f"fixture FileGDB verificata: {len(contenuti)} parti, "
        f"{sum(len(v) for v in contenuti.values())} byte di contenuto; "
        f"riproducibilita' provata con {prova['versione_gdal']}, "
        f"{prova['byte_coniati_totali']} byte coniati da GDAL."
    )
    return 0


def _relativo(percorso: pathlib.Path) -> str:
    """Il percorso relativo alla radice quando ci sta dentro.

    Le sonde spostano `PROVA` in una directory temporanea per provare i casi
    rossi: un `relative_to` incondizionato le farebbe fallire nel **messaggio**,
    cioe' fuori da cio' che stanno verificando.
    """
    try:
        return percorso.relative_to(ROOT).as_posix()
    except ValueError:
        return percorso.as_posix()


def _prova_legata_alla_fixture(contenuti: dict[str, bytes]) -> list[str]:
    """Il verbale della riproducibilita' descrive **questa** fixture.

    Un verbale che sopravvivesse a una fixture rigenerata direbbe «riproducibile»
    di byte che nessuno ha confrontato. L'impronta lo lega; le parti e i byte
    coniati devono essere quelli che il verbale dichiara.
    """
    if not PROVA.exists():
        return [
            f"{_relativo(PROVA)}: prova di riproducibilita' "
            "assente. La fixture puo' essere ben formata e non essere quella che "
            "GDAL produce: si genera con `bash scripts/genera-fixture-filegdb.sh`."
        ]
    try:
        prova = json.loads(PROVA.read_text(encoding="utf-8"))
    except json.JSONDecodeError as errore:
        return [f"{PROVA.name}: non e' JSON leggibile ({errore})"]

    errori: list[str] = []
    attesa = hashlib.sha256(ARCHIVIO.read_bytes()).hexdigest()
    if prova.get("impronta_della_fixture") != attesa:
        return [
            f"la prova di riproducibilita' descrive una fixture con impronta "
            f"«{prova.get('impronta_della_fixture')}», quella committata ha "
            f"«{attesa}». Il verbale e' di un'altra fixture: va rifatto con "
            "`bash scripts/genera-fixture-filegdb.sh`."
        ]

    # La sorgente e' un ingresso quanto la versione di GDAL: cambiarla cambia
    # ogni byte della fixture. Senza questo legame, un verbale poteva descrivere
    # un confronto fatto su un altro GeoJSON.
    if not SORGENTE.is_file():
        errori.append(
            f"{_relativo(SORGENTE)}: la sorgente della fixture non c'e' piu'. "
            "Senza, la fixture non e' riproducibile da nessuno."
        )
    else:
        sorgente = hashlib.sha256(SORGENTE.read_bytes()).hexdigest()
        if prova.get("impronta_della_sorgente") != sorgente:
            errori.append(
                f"la prova e' stata ottenuta da una sorgente con impronta "
                f"«{prova.get('impronta_della_sorgente')}», quella committata ha "
                f"«{sorgente}». Il GeoJSON di partenza e' cambiato: la fixture "
                "che GDAL produrrebbe oggi e' un'altra."
            )
    if prova.get("parti") != len(contenuti):
        errori.append(
            f"la prova dichiara {prova.get('parti')} parti, la fixture ne ha "
            f"{len(contenuti)}"
        )
    if not prova.get("versione_gdal"):
        errori.append(
            "la prova non dice con **quale** GDAL e' stata ottenuta: due versioni "
            "scrivono tabelle di metadati diverse, e la differenza non sarebbe un "
            "byte coniato"
        )
    coniati = prova.get("byte_coniati_totali")
    if not isinstance(coniati, int) or isinstance(coniati, bool) or coniati <= 0:
        errori.append(
            f"`byte_coniati_totali` vale «{coniati}». Zero byte coniati vorrebbe "
            "dire che due rigenerazioni sono identiche byte a byte: sarebbe una "
            "buona notizia, e renderebbe la tolleranza del confronto vuota -- va "
            "verificato invece che dato per scontato."
        )
    errori.extend(_offset_riconciliati(prova, contenuti))
    return errori


def _intero(valore: object) -> bool:
    """Un intero vero.

    `True` e' un `int` per Python, e vale 1: senza questo controllo un offset
    `true` era un offset, e un conteggio per parte `true` tornava con un elenco
    di un elemento. Non e' un caso teorico -- e' la forma che prende un JSON
    scritto da uno strumento diverso -- e gli altri gate di questo repository lo
    escludono da tempo.
    """
    return isinstance(valore, int) and not isinstance(valore, bool) and valore >= 0


def _offset_riconciliati(prova: dict, contenuti: dict[str, bytes]) -> list[str]:
    """I tre modi in cui il verbale conta i byte coniati devono coincidere.

    `offset_coniati` e' l'elenco, `byte_coniati_per_parte` il conteggio,
    `byte_coniati_totali` la somma. Erano tre campi che nessuno confrontava, e un
    verbale che ne portasse due coerenti e uno inventato passava: bastava
    lasciare l'elenco vuoto per non farlo guardare da nessuno.
    """
    offset = prova.get("offset_coniati")
    per_parte = prova.get("byte_coniati_per_parte")
    if not isinstance(offset, dict) or not isinstance(per_parte, dict):
        return [
            "`offset_coniati` o `byte_coniati_per_parte` assenti o non sono "
            "mappe: senza l'elenco, i conteggi non hanno niente da cui derivare "
            "e la tolleranza del confronto resta indimostrata."
        ]

    errori: list[str] = []
    if not offset:
        errori.append(
            "`offset_coniati` e' vuoto: nessun byte coniato elencato, mentre il "
            "totale ne dichiara. E' la forma in cui un verbale sembra completo e "
            "non dice niente."
        )
    if set(offset) != set(per_parte):
        errori.append(
            f"`offset_coniati` elenca {sorted(offset)}, "
            f"`byte_coniati_per_parte` conta {sorted(per_parte)}: due parti "
            "diverse per lo stesso confronto."
        )
        return errori

    estranee = sorted(set(offset) - set(contenuti))
    if estranee:
        errori.append(
            f"il verbale conta byte coniati per parti che la fixture non ha: "
            f"{estranee}"
        )

    somma = 0
    for nome, elenco in sorted(offset.items()):
        if not isinstance(elenco, list) or not all(_intero(o) for o in elenco):
            errori.append(f"`offset_coniati[{nome}]` non e' un elenco di offset")
            continue
        if len(elenco) != len(set(elenco)):
            errori.append(f"`offset_coniati[{nome}]` ripete lo stesso offset")
        if elenco != sorted(elenco):
            errori.append(f"`offset_coniati[{nome}]` non e' ordinato")
        lunghezza = len(contenuti.get(nome, b""))
        fuori = [o for o in elenco if o < 0 or o >= lunghezza]
        if fuori:
            errori.append(
                f"`offset_coniati[{nome}]` cita offset fuori dalla parte "
                f"({lunghezza} byte): {fuori[:5]}"
            )
        conteggio = per_parte.get(nome)
        if not _intero(conteggio) or conteggio != len(elenco):
            errori.append(
                f"`byte_coniati_per_parte[{nome}]` vale «{conteggio}», "
                f"l'elenco ne porta {len(elenco)}"
            )
        somma += len(elenco)

    if prova.get("byte_coniati_totali") != somma:
        errori.append(
            f"`byte_coniati_totali` vale «{prova.get('byte_coniati_totali')}», "
            f"gli offset elencati sono {somma}"
        )
    return errori


def main(argv: list[str] | None = None) -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    modo = argomenti.add_mutually_exclusive_group()
    modo.add_argument("--scrivi", type=pathlib.Path, metavar="GDB")
    modo.add_argument("--verifica", action="store_true")
    modo.add_argument("--confronta", nargs=2, type=pathlib.Path, metavar=("GDB1", "GDB2"))
    modo.add_argument("--registra", nargs=2, type=pathlib.Path, metavar=("GDB1", "GDB2"))
    argomenti.add_argument("--gdal", default="", help="la versione di GDAL usata")
    opzioni = argomenti.parse_args(argv)

    if opzioni.scrivi:
        return scrivi(opzioni.scrivi)
    if opzioni.registra:
        if not opzioni.gdal:
            print("--registra richiede --gdal <versione>", file=sys.stderr)
            return 2
        return registra(opzioni.registra[0], opzioni.registra[1], opzioni.gdal)
    if opzioni.confronta:
        if not ARCHIVIO.exists():
            print(f"{ARCHIVIO}: fixture assente", file=sys.stderr)
            return 1
        errori = confronta(
            spacchetta(ARCHIVIO.read_bytes()),
            parti(opzioni.confronta[0]),
            parti(opzioni.confronta[1]),
        )
        for messaggio in errori:
            print(messaggio, file=sys.stderr)
        if errori:
            return 1
        print(
            "fixture riproducibile: le differenze dalla rigenerazione stanno "
            "tutte fra i byte che GDAL conia a ogni corsa."
        )
        return 0
    return verifica_forma()


if __name__ == "__main__":
    sys.exit(main())
