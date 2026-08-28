#!/usr/bin/env python3
"""Un fuzz target raggiunge davvero il formato che dice di esercitare.

# Il difetto che questo gate esiste per chiudere

`fuzz.reader-shapefile` era bloccante perche' `shp_wkb` converte WKB e forme
ESRI in memoria e veniva contato come copertura del formato. Rimpiazzarlo con un
target che apre un file non basta a chiudere il blocco: una build che compila e
un replay senza crash **non** dimostrano che i semi arrivino al parser. Un
input rifiutato all'apertura non fa crashare niente, e da fuori e'
indistinguibile da uno letto per intero.

Serve percio' una misura, e serve che qualcuno la legga. Questo gate legge
l'artefatto prodotto da `scripts/fuzz-profondita.sh <bersaglio>` e pretende che
ogni requisito dichiarato nel registro del bersaglio sia stato **raggiunto**.

# Un motore, due bersagli

Shapefile e FileGDB pongono la stessa domanda -- «il target arriva dove dice di
arrivare?» -- e la risposta si costruisce allo stesso modo. Cio' che cambia sono
i **dati**: quale registro leggere, quale nucleo di requisiti il gate pretende
per conto proprio, quali percorsi formano il perimetro.

Due gate gemelli sarebbero divergiti al primo cambiamento, e la divergenza si
sarebbe vista solo nel formato che nessuno stava guardando. `BERSAGLI` tiene le
differenze in un posto solo; il resto del file non sa quale formato sta
verificando.

# Perche' l'artefatto non e' creduto sulla parola

Una misura committata invecchia: il codice cambia, il target smette di
raggiungere cio' che raggiungeva, e il JSON continua a dire di si'. Qui la
misura porta l'**impronta del perimetro** -- i percorsi che possono cambiare
cio' che il target attraversa -- e il gate la ricalcola dal working tree. Se una
sola riga del driver, del target, dei semi o di `Cargo.lock` cambia, l'impronta
diverge e il gate diventa rosso finche' la misura non viene rifatta.

Non e' legata allo SHA: il commit che *pubblica* una misura ha per forza uno SHA
diverso da quello su cui e' girata, e legarla al commit la renderebbe scaduta
per costruzione.

# Perche' la misura dice «raggiunto» e non «quante volte»

Fino allo schema 1 ogni requisito portava il `conteggio` delle esecuzioni, e il
gate ne pretendeva il **segno**: `conteggio > 0`. Il numero pero' non e'
riproducibile. Su quattro corse di `scripts/fuzz-profondita.sh shp_reader`, due
delle quali su albero bit-identico, otto requisiti su trentacinque alternavano
fra due stati: nessuno compariva o spariva, `famiglia`, `riga` e `simboli` non
si muovevano, e a variare era il solo conteggio -- che `docs/RELEASE.md` gia'
dichiarava variabile e fuori da ogni soglia.

Nessun verdetto ne dipendeva. Il danno era un altro: il rumore entrava in un
file **versionato** a ogni rimisura, indistinguibile da un fatto, e un artefatto
che cambia senza che sia cambiato nulla insegna a non leggerlo.

Lo schema 2 registra percio' `"raggiunto": true`, che il generatore deriva da
`conteggio > 0` -- la stessa affermazione, senza la parte instabile. Cancellare
`conteggio` e basta non sarebbe bastato: il suo segno *era* la prova di
raggiungimento, e una misura senza ne' l'uno ne' l'altro non direbbe piu'
niente. Il gate pretende un booleano **vero** e rifiuta il campo assente, il
`false`, un numero al suo posto e un artefatto che porti ancora `conteggio`:
quella e' la forma vecchia, e riletta con la regola nuova sarebbe verde per
assenza di domanda.

Non si e' indagato per rendere deterministici i conteggi. Comprerebbe una
proprieta' che nessuno usa.

# Che cosa questo gate non dice

Che un ramo sia **raggiunto** non dice che il suo contratto sia verificato. La
separazione fra «non raggiunto» e «raggiunto» e' qui; la verifica semantica sta
nelle sonde dei driver, che chiamano lo stesso entry point del target sugli
stessi input e guardano righe drenate e messaggi di rifiuto.

Per FileGDB c'e' un secondo limite, ed e' scritto dove non si puo' non vederlo:
`assurance/registries/asan-filegdb.json`. Il percorso attraversa GDAL, ma
`libgdal.so` non e' strumentata: questa misura conta cio' che la copertura vede,
e la copertura dentro GDAL non c'e'.

# Uso

    python3 scripts/check_profondita_fuzz.py <bersaglio>
    python3 scripts/check_profondita_fuzz.py <bersaglio> --registra <export.json> --lcov <file.lcov>

Il secondo modo lo invoca `scripts/fuzz-profondita.sh` dopo la misura: e' il
solo posto in cui l'artefatto viene scritto.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent
REGISTRI = ROOT / "assurance" / "registries"

#: La versione dello schema dell'artefatto di profondita'.
#:
#: E' `2` da quando il `conteggio` delle esecuzioni ha lasciato il posto a
#: `raggiunto`. Il numero e' preteso e non ignorato: una misura di schema 1 non
#: e' una misura incompleta, e' una misura che risponde a un'altra domanda, e
#: leggerla con la regola nuova la farebbe passare senza portare l'affermazione
#: che la regola nuova chiede.
SCHEMA_ARTEFATTO = 2


@dataclass(frozen=True)
class Bersaglio:
    """Cio' che cambia da un formato all'altro, e nient'altro.

    `famiglia_del_nucleo` e `perimetro_obbligatorio` sono scritti **qui** e non
    solo nel registro. Senza, il modo piu' semplice di rendere verde il gate
    sarebbe svuotare il registro: zero requisiti, zero requisiti mancati. E' la
    stessa famiglia del `"test": []` del contratto, e si chiude allo stesso modo
    -- un insieme che il registro deve contenere e che il gate conosce senza
    chiederglielo.
    """

    nome: str
    registro: Path
    #: `identita' -> famiglia`. La famiglia serve per una ragione sua: un
    #: requisito di riga individua un ramo preciso del nostro sorgente, uno di
    #: funzione dice che un simbolo e' stato eseguito. Sono due affermazioni
    #: diverse, e senza il vincolo la seconda poteva essere riscritta come la
    #: prima -- a nome di un simbolo che esiste comunque -- lasciando il gate
    #: verde su un ramo mai percorso.
    famiglia_del_nucleo: dict[str, str]
    #: I percorsi che il perimetro deve contenere comunque: il codice che la
    #: misura attraversa e gli input che ce la portano. Un perimetro ridotto a
    #: `Cargo.lock` renderebbe la misura eterna.
    perimetro_obbligatorio: frozenset[str]

    @property
    def nucleo(self) -> frozenset[str]:
        """Derivato, non riscritto: due elenchi allineati a mano divergono."""
        return frozenset(self.famiglia_del_nucleo)


BERSAGLI: dict[str, Bersaglio] = {
    "shp_reader": Bersaglio(
        nome="shp_reader",
        registro=REGISTRI / "profondita-fuzz-shapefile.json",
        famiglia_del_nucleo={
            "driver.apertura": "funzioni",
            "driver.schema": "funzioni",
            "driver.dbf-layout": "funzioni",
            "dbf.intestazione": "funzioni",
            "dbf.descrittori-di-campo": "funzioni",
            "dbf.valori": "funzioni",
            "shp.intestazione": "funzioni",
            "shp.intestazione-di-record": "funzioni",
            "shp.geometria-punto": "funzioni",
            "shp.geometria-polilinea": "funzioni",
            "shp.geometria-multipunto": "funzioni",
            "shx.indice": "funzioni",
            "shp.conteggio-delle-forme": "funzioni",
            "drenaggio.batch": "funzioni",
            "prevalidazione.conteggi-del-multipunto": "righe",
            "rifiuto.conteggi-all-apertura": "righe",
            "rifiuto.cardinalita-nel-drenaggio": "righe",
        },
        perimetro_obbligatorio=frozenset(
            {
                "Cargo.lock",
                "crates/driver-shp/Cargo.toml",
                "crates/driver-shp/src",
                # Il workspace di fuzzing e' **detached**: le versioni con cui il
                # target viene costruito stanno nel suo lockfile, non in quello
                # del workspace principale. Senza, cambiare una dipendenza del
                # target non farebbe scadere la misura.
                "fuzz/Cargo.lock",
                "fuzz/Cargo.toml",
                "fuzz/fuzz_targets",
                "fuzz/seeds/shp_reader",
            }
        ),
    ),
    "filegdb_reader": Bersaglio(
        nome="filegdb_reader",
        registro=REGISTRI / "profondita-fuzz-filegdb.json",
        famiglia_del_nucleo={
            "driver.apertura": "funzioni",
            "driver.backend": "funzioni",
            "driver.contratto-geometrico": "funzioni",
            "driver.tipi-arrow": "funzioni",
            "drenaggio.reader": "funzioni",
            "drenaggio.batch": "funzioni",
            "materializzazione.parti": "righe",
            "materializzazione.fixture-intatta": "righe",
        },
        perimetro_obbligatorio=frozenset(
            {
                "Cargo.lock",
                # Non solo i sorgenti: il manifesto decide **quali** feature e
                # quali dipendenze entrano nel target, e `gdal-backend` e' la
                # feature senza la quale questo percorso non esiste.
                "crates/driver-filegdb/Cargo.toml",
                "crates/driver-filegdb/src",
                # Il wrapper Rust di GDAL fa parte di cio' che la misura
                # attraversa: e' il fork governato, ed e' l'unica parte del
                # percorso GDAL che la copertura vede. `build.rs` ci sta dentro
                # perche' e' li' che si decide **contro quale** libreria si
                # collega, e una misura che gli sopravvivesse descriverebbe un
                # binario diverso.
                "vendor/gdal/Cargo.toml",
                "vendor/gdal/build.rs",
                "vendor/gdal/src",
                # Il workspace di fuzzing e' detached: le versioni con cui il
                # target viene costruito stanno nel suo lockfile.
                "fuzz/Cargo.lock",
                "fuzz/Cargo.toml",
                "fuzz/fuzz_targets",
                "fuzz/fixtures/filegdb",
                "fuzz/seeds/filegdb_reader",
            }
        ),
    ),
    "wkt_parse": Bersaglio(
        nome="wkt_parse",
        registro=REGISTRI / "profondita-fuzz-wkt.json",
        famiglia_del_nucleo={
            "analisi.ingresso": "funzioni",
            "analisi.geometria": "funzioni",
            "analisi.coordinata": "funzioni",
            "analisi.poligono": "funzioni",
            "analisi.multipunto": "funzioni",
            "analisi.collezione": "funzioni",
            "analisi.suffisso-attaccato": "funzioni",
            "tetto.superato": "funzioni",
            "rifiuto.testo-residuo": "righe",
        },
        perimetro_obbligatorio=frozenset(
            {
                "Cargo.lock",
                "crates/driver-common/Cargo.toml",
                "crates/driver-common/src",
                "fuzz/Cargo.lock",
                "fuzz/Cargo.toml",
                "fuzz/fuzz_targets",
                "fuzz/seeds/wkt_parse",
            }
        ),
    ),
    "geojson_reader": Bersaglio(
        nome="geojson_reader",
        registro=REGISTRI / "profondita-fuzz-geojson.json",
        famiglia_del_nucleo={
            "analisi.geometria": "funzioni",
            "analisi.albero": "funzioni",
            "analisi.figlie": "funzioni",
            "budget.addebito": "funzioni",
            "budget.profondita": "funzioni",
            "errore.canale-laterale": "funzioni",
            "addebito.posizione": "righe",
            "tetto.annidamento": "righe",
            "addebito.membri": "righe",
        },
        perimetro_obbligatorio=frozenset(
            {
                "Cargo.lock",
                "crates/driver-geojson/Cargo.toml",
                "crates/driver-geojson/src",
                "fuzz/Cargo.lock",
                "fuzz/Cargo.toml",
                "fuzz/fuzz_targets",
                "fuzz/seeds/geojson_reader",
            }
        ),
    ),
}


class RegistroMalformato(Exception):
    """Il registro non e' leggibile: nessuna verifica ha senso senza."""


# --- il registro ------------------------------------------------------------


def leggi_registro(bersaglio: Bersaglio) -> dict[str, Any]:
    percorso = bersaglio.registro
    try:
        documento = json.loads(percorso.read_text(encoding="utf-8"))
    except FileNotFoundError as errore:
        raise RegistroMalformato(f"{percorso}: registro assente") from errore
    except json.JSONDecodeError as errore:
        raise RegistroMalformato(f"{percorso}: non e' JSON leggibile ({errore})") from errore
    if not isinstance(documento, dict):
        raise RegistroMalformato(f"{percorso}: la radice non e' un oggetto")
    return documento


def requisiti(
    bersaglio: Bersaglio, registro: dict[str, Any]
) -> tuple[list[dict[str, Any]], list[str]]:
    """`(requisiti, errori)`: le due famiglie unite, con la loro forma verificata."""
    errori: list[str] = []
    uniti: list[dict[str, Any]] = []

    for famiglia, campi in (("funzioni", ("segmenti",)), ("righe", ("file", "ancora"))):
        voci = registro.get(famiglia)
        if not isinstance(voci, list) or not voci:
            errori.append(
                f"`{famiglia}`: assente o vuota. Un elenco vuoto non fa fallire "
                "niente, ed e' il modo in cui questo gate diventerebbe verde per "
                "assenza di domanda."
            )
            continue
        for voce in voci:
            if not isinstance(voce, dict):
                errori.append(f"`{famiglia}`: voce che non e' un oggetto")
                continue
            identita = voce.get("id")
            if not isinstance(identita, str) or not identita:
                errori.append(f"`{famiglia}`: voce senza identita'")
                continue
            mancanti = [campo for campo in (*campi, "perche") if not voce.get(campo)]
            if mancanti:
                errori.append(f"{identita}: campi mancanti o vuoti {mancanti}")
                continue
            if famiglia == "funzioni":
                segmenti = voce["segmenti"]
                if not isinstance(segmenti, list) or not all(
                    isinstance(s, str) and s for s in segmenti
                ):
                    errori.append(f"{identita}: `segmenti` non e' un elenco di nomi")
                    continue
            uniti.append({**voce, "famiglia": famiglia})

    identita = [voce["id"] for voce in uniti]
    ripetute = sorted({nome for nome in identita if identita.count(nome) > 1})
    if ripetute:
        errori.append(
            f"identita' ripetute {ripetute}: due requisiti omonimi ne fanno uno, "
            "e il conteggio non se ne accorge"
        )

    senza = sorted(bersaglio.nucleo - set(identita))
    if senza:
        errori.append(
            f"requisiti del nucleo assenti dal registro: {senza}. Sono la "
            "definizione di «il reader e' esercitato»: toglierne uno non riduce "
            "l'ambizione, cambia cio' che la frase significa."
        )

    per_identita = {voce["id"]: voce["famiglia"] for voce in uniti}
    spostati = sorted(
        f"{identita} e' dichiarato fra le «{per_identita[identita]}», atteso fra "
        f"le «{famiglia}»"
        for identita, famiglia in bersaglio.famiglia_del_nucleo.items()
        if identita in per_identita and per_identita[identita] != famiglia
    )
    errori.extend(
        f"{messaggio}. Un ramo del sorgente e un simbolo eseguito non sono la "
        "stessa prova, e scambiarli lascerebbe verde un ramo mai percorso."
        for messaggio in spostati
    )

    dichiarato = registro.get("nucleo")
    if not isinstance(dichiarato, list) or set(dichiarato) != bersaglio.nucleo:
        errori.append(
            "`nucleo` del registro diverso da quello che il gate pretende. Il "
            "registro lo scrive per chi legge, il gate lo conosce per non "
            "dipendere da chi lo scrive: quando divergono, uno dei due mente."
        )

    if registro.get("target") != bersaglio.nome:
        errori.append(
            f"il registro dichiara il target «{registro.get('target')}», il gate "
            f"lo ha aperto come «{bersaglio.nome}». Un registro letto per il "
            "bersaglio sbagliato verificherebbe un formato a nome di un altro."
        )

    return uniti, errori


# --- il perimetro -----------------------------------------------------------


def percorsi_del_perimetro(
    bersaglio: Bersaglio, registro: dict[str, Any]
) -> tuple[list[str], list[str]]:
    perimetro = registro.get("perimetro")
    if not isinstance(perimetro, dict):
        return [], ["`perimetro` assente: senza, la misura non scadrebbe mai"]
    percorsi = perimetro.get("percorsi")
    if not isinstance(percorsi, list) or not all(
        isinstance(p, str) and p for p in percorsi
    ):
        return [], ["`perimetro.percorsi` non e' un elenco di percorsi"]

    errori: list[str] = []
    senza = sorted(bersaglio.perimetro_obbligatorio - set(percorsi))
    if senza:
        errori.append(
            f"perimetro senza {senza}: sono il codice che la misura attraversa e "
            "gli input che ce la portano. Un perimetro che non li contiene "
            "renderebbe la misura valida anche dopo averli riscritti."
        )
    return sorted(percorsi), errori


def impronta_del_perimetro(percorsi: list[str]) -> tuple[str, list[str]]:
    """SHA-256 dei file sotto i percorsi dichiarati.

    L'elenco passa da git e comprende i file **non ancora tracciati** che git
    non ignora. Le due esclusioni contano entrambe: un artefatto di build
    cambierebbe l'impronta senza cambiare cio' che viene compilato, e un seme
    nuovo non ancora aggiunto all'indice **cambia** cio' che il target legge --
    contarlo solo dopo `git add` farebbe scadere la misura al momento del
    commit, cioe' sempre.
    """
    esito = subprocess.run(
        [
            "git",
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
            *percorsi,
        ],
        cwd=ROOT,
        capture_output=True,
        check=False,
    )
    if esito.returncode != 0:
        return "", [f"`git ls-files` fallito: {esito.stderr.decode('utf-8', 'replace')}"]

    relativi = sorted(
        {pezzo.decode("utf-8") for pezzo in esito.stdout.split(b"\x00") if pezzo}
    )
    if not relativi:
        return "", [
            "il perimetro non seleziona nessun file: un'impronta di niente "
            "coincide sempre con se stessa"
        ]

    accumulatore = hashlib.sha256()
    assenti: list[str] = []
    for relativo in relativi:
        percorso = ROOT / relativo
        if not percorso.is_file():
            # Un file nell'indice e non sul disco: l'albero e' a meta' di
            # qualcosa, e un'impronta calcolata adesso non descrive ne' il
            # prima ne' il dopo.
            assenti.append(relativo)
            continue
        accumulatore.update(relativo.encode("utf-8"))
        accumulatore.update(b"\x00")
        accumulatore.update(
            hashlib.sha256(percorso.read_bytes()).hexdigest().encode("ascii")
        )
        accumulatore.update(b"\n")
    if assenti:
        return "", [
            f"file del perimetro nell'indice e non sul disco: {assenti}. "
            "L'impronta di un albero a meta' non descrive nessuna misura."
        ]
    return accumulatore.hexdigest(), []


# --- la misura --------------------------------------------------------------

# Un simbolo v0 porta i disambiguatori di crate (`Cs<hash>_`), che cambiano a
# ogni build: cercarli renderebbe il registro valido per una compilazione sola.
# I nomi, invece, sono codificati con la loro lunghezza davanti -- `10driver_shp`
# -- e quella forma e' stabile. Il registro dichiara i segmenti; il pattern lo
# costruisce questo modulo.
def pattern_dei_segmenti(segmenti: list[str]) -> re.Pattern[str]:
    return re.compile(".*".join(re.escape(f"{len(s)}{s}") for s in segmenti))


def funzioni_coperte(export: dict[str, Any]) -> dict[str, int]:
    """`{simbolo: conteggio}` dal `llvm-cov export --format=text`."""
    dati = export.get("data")
    if not isinstance(dati, list) or not dati:
        raise RegistroMalformato("export senza `data`: non e' una misura")
    funzioni = dati[0].get("functions")
    if not isinstance(funzioni, list) or not funzioni:
        raise RegistroMalformato("export senza `functions`: non e' una misura")
    conteggi: dict[str, int] = {}
    for voce in funzioni:
        nome = voce.get("name")
        conteggio = voce.get("count", 0)
        if isinstance(nome, str):
            conteggi[nome] = max(conteggi.get(nome, 0), int(conteggio))
    return conteggi


def righe_coperte(lcov: str) -> dict[str, dict[int, int]]:
    """`{file: {riga: conteggio}}` dai record `DA:` di un lcov."""
    per_file: dict[str, dict[int, int]] = {}
    corrente: dict[int, int] | None = None
    for riga in lcov.splitlines():
        if riga.startswith("SF:"):
            corrente = per_file.setdefault(riga[3:].strip(), {})
        elif riga.startswith("DA:") and corrente is not None:
            numero, _, conteggio = riga[3:].partition(",")
            try:
                corrente[int(numero)] = max(
                    corrente.get(int(numero), 0), int(conteggio.split(",")[0])
                )
            except ValueError:
                continue
    return per_file


def _righe_del_file(coperte: dict[str, dict[int, int]], relativo: str) -> dict[int, int]:
    """Il file dell'lcov si nomina in assoluto dentro il container: si cerca
    per suffisso, e una corrispondenza ambigua e' un errore, non una scelta."""
    candidati = [
        righe
        for percorso, righe in coperte.items()
        if percorso.replace("\\", "/").endswith("/" + relativo)
        or percorso.replace("\\", "/") == relativo
    ]
    if len(candidati) != 1:
        return {}
    return candidati[0]


def riga_dell_ancora(
    relativo: str, ancora: str, strumentate: dict[int, int]
) -> tuple[int | None, list[str]]:
    """La riga **strumentata** che contiene l'ancora, se e' una sola.

    Le righe di un `mod tests` non compaiono qui: il binario di fuzzing non le
    compila, quindi non hanno dati di copertura. E' cio' che permette di usare
    come ancora un messaggio che il codice e la sua sonda condividono senza
    doverli distinguere a mano.
    """
    sorgente = (ROOT / relativo).read_text(encoding="utf-8").splitlines()
    trovate = [
        numero
        for numero, testo in enumerate(sorgente, 1)
        if ancora in testo and numero in strumentate
    ]
    if not trovate:
        presenti = any(ancora in testo for testo in sorgente)
        return None, [
            f"{relativo}: l'ancora «{ancora}» "
            + (
                "esiste ma su nessuna riga strumentata: o non e' compilata nel "
                "target, o la misura riguarda un altro albero"
                if presenti
                else "non esiste piu' nel sorgente"
            )
        ]
    if len(trovate) > 1:
        return None, [
            f"{relativo}: l'ancora «{ancora}» compare su {len(trovate)} righe "
            f"strumentate {trovate}. Un'ancora ambigua non individua un ramo: "
            "sceglierne una che compaia una volta sola."
        ]
    return trovate[0], []


def osserva(
    bersaglio: Bersaglio, registro: dict[str, Any], export: dict[str, Any], lcov: str
) -> tuple[list[dict[str, Any]], list[str]]:
    """Che cosa la misura dice di ciascun requisito. Nessun giudizio, qui."""
    voci, errori = requisiti(bersaglio, registro)
    if errori:
        return [], errori

    conteggi = funzioni_coperte(export)
    per_file = righe_coperte(lcov)

    osservazioni: list[dict[str, Any]] = []
    for voce in voci:
        if voce["famiglia"] == "funzioni":
            pattern = pattern_dei_segmenti(voce["segmenti"])
            trovati = {
                simbolo: conteggio
                for simbolo, conteggio in conteggi.items()
                if pattern.search(simbolo)
            }
            migliore = max(trovati.values(), default=0)
            # Da qui in poi il conteggio non esce piu': la misura registra
            # `raggiunto`, che e' il suo segno. Vedi il docstring del modulo --
            # il numero alternava fra due stati a parita' di albero, e nessun
            # verdetto ne dipendeva.
            osservazioni.append(
                {
                    "id": voce["id"],
                    "famiglia": "funzione",
                    "simboli": len(trovati),
                    "raggiunto": migliore > 0,
                }
            )
            continue

        strumentate = _righe_del_file(per_file, voce["file"])
        numero, problemi = riga_dell_ancora(voce["file"], voce["ancora"], strumentate)
        if problemi:
            errori.extend(f"{voce['id']}: {m}" for m in problemi)
            continue
        osservazioni.append(
            {
                "id": voce["id"],
                "famiglia": "riga",
                "riga": numero,
                "raggiunto": strumentate.get(numero, 0) > 0,
            }
        )
    return osservazioni, errori


# --- la verifica ------------------------------------------------------------


def verifica(
    bersaglio: Bersaglio, registro: dict[str, Any], misura: dict[str, Any]
) -> list[str]:
    voci, errori = requisiti(bersaglio, registro)
    percorsi, problemi = percorsi_del_perimetro(bersaglio, registro)
    errori.extend(problemi)
    if errori:
        return errori

    # La versione dello schema si legge prima di tutto il resto, e si pretende
    # esatta invece che «almeno». Una misura di schema 1 porta `conteggio` e non
    # `raggiunto`: riletta qui direbbe soltanto che i requisiti ci sono tutti,
    # senza portare l'affermazione che li dichiara raggiunti.
    if misura.get("schema_version") != SCHEMA_ARTEFATTO:
        errori.append(
            f"la misura dichiara lo schema «{misura.get('schema_version')}», il "
            f"gate legge lo schema {SCHEMA_ARTEFATTO}. Lo schema 1 registrava il "
            "`conteggio` delle esecuzioni, che a parita' di albero non e' "
            "riproducibile; il 2 registra `raggiunto`. Va rifatta con "
            f"`scripts/fuzz-profondita.sh {bersaglio.nome}`."
        )

    if misura.get("target") != registro.get("target"):
        errori.append(
            f"la misura riguarda il target «{misura.get('target')}», il registro "
            f"«{registro.get('target')}»"
        )

    corpus = misura.get("corpus")
    # `bool` e' sottotipo di `int` in Python, e `True` passerebbe per il numero
    # uno: un `corpus.input: true` diceva «un input» a chi legge il codice e
    # «vero» a chi legge il JSON.
    if (
        not isinstance(corpus, dict)
        or not isinstance(corpus.get("input"), int)
        or isinstance(corpus.get("input"), bool)
    ):
        errori.append("la misura non dice su quanti input e' girata")
    elif corpus["input"] <= 0:
        errori.append(
            "la misura e' girata su zero input: una copertura senza denominatore "
            "non si rilegge, e senza input non c'e' niente da raggiungere"
        )

    attesa, problemi = impronta_del_perimetro(percorsi)
    errori.extend(problemi)
    if attesa and misura.get("impronta_perimetro") != attesa:
        errori.append(
            f"impronta del perimetro diversa: la misura dice "
            f"«{misura.get('impronta_perimetro')}», il working tree "
            f"«{attesa}». Il codice che il target attraversa e' cambiato dopo "
            f"la misura: va rifatta con `scripts/fuzz-profondita.sh {bersaglio.nome}`."
        )

    osservazioni = misura.get("requisiti")
    if not isinstance(osservazioni, list) or not osservazioni:
        return errori + ["la misura non porta osservazioni: non c'e' niente da leggere"]

    per_id: dict[str, Any] = {}
    for voce in osservazioni:
        if not isinstance(voce, dict) or not isinstance(voce.get("id"), str):
            errori.append("osservazione senza identita'")
            continue
        if voce["id"] in per_id:
            errori.append(f"{voce['id']}: osservazione ripetuta nella misura")
        per_id[voce["id"]] = voce

    dichiarati = {voce["id"] for voce in voci}
    mancanti = sorted(dichiarati - set(per_id))
    if mancanti:
        errori.append(
            f"requisiti dichiarati e non misurati: {mancanti}. La misura e' di "
            "un registro piu' piccolo di questo."
        )
    estranei = sorted(set(per_id) - dichiarati)
    if estranei:
        errori.append(
            f"la misura osserva requisiti che il registro non dichiara: "
            f"{estranei}. Le due fonti descrivono cose diverse."
        )

    # La famiglia dichiarata dal registro, per confrontarla con quella che la
    # misura riporta. Sono due parole diverse per la stessa cosa -- il registro
    # dice «funzioni» e «righe», l'osservazione «funzione» e «riga» -- e la
    # traduzione sta qui invece che in nessuno dei due.
    famiglia_dichiarata = {
        voce["id"]: {"funzioni": "funzione", "righe": "riga"}[voce["famiglia"]]
        for voce in voci
    }

    for identita in sorted(dichiarati & set(per_id)):
        voce = per_id[identita]
        attesa = famiglia_dichiarata[identita]
        if voce.get("famiglia") != attesa:
            # Una funzione osservata come riga, o viceversa, e' una misura che
            # risponde a una domanda diversa da quella posta. Senza questo
            # confronto la sola cosa che restava era il conteggio, e un conteggio
            # positivo si ottiene da qualunque riga eseguita.
            errori.append(
                f"{identita}: la misura lo osserva come «{voce.get('famiglia')}», "
                f"il registro lo dichiara «{attesa}». Un ramo del sorgente e un "
                "simbolo eseguito non rispondono alla stessa domanda."
            )
        # `raggiunto` deve essere un booleano **vero**, e i quattro modi di non
        # esserlo sono errori distinti perche' portano a diagnosi distinte: il
        # campo assente e' una misura che non afferma niente, `false` e' un ramo
        # mai percorso, un numero e' la forma vecchia travestita, e `conteggio`
        # ancora presente e' la forma vecchia intera.
        if "conteggio" in voce:
            errori.append(
                f"{identita}: l'osservazione porta ancora `conteggio`. E' la "
                "forma dello schema 1, dove il numero era instabile a parita' "
                "di albero: la misura va rifatta, non ritoccata."
            )
        raggiunto = voce.get("raggiunto")
        if not isinstance(raggiunto, bool):
            errori.append(
                f"{identita}: `raggiunto` vale «{raggiunto}» e non e' un "
                "booleano. Un numero o un campo assente al suo posto non "
                "affermano che il requisito sia stato raggiunto."
            )
        elif not raggiunto:
            errori.append(
                f"{identita}: non raggiunto dal replay del corpus. Il target "
                "compila e non crasha, e non arriva qui."
            )
        simboli = voce.get("simboli")
        if voce.get("famiglia") == "funzione" and isinstance(simboli, bool):
            errori.append(f"{identita}: `simboli` vale «{simboli}», non e' un conteggio")
        elif voce.get("famiglia") == "funzione" and not simboli:
            errori.append(
                f"{identita}: nessun simbolo corrisponde ai segmenti dichiarati. "
                "O la funzione e' stata rinominata, o non e' stata compilata nel "
                "target."
            )
    return errori


# --- i due modi -------------------------------------------------------------


def registra(
    bersaglio: Bersaglio,
    export_json: Path,
    lcov_file: Path,
    uscita: Path,
    input_totali: int,
) -> int:
    registro = leggi_registro(bersaglio)
    percorsi, problemi = percorsi_del_perimetro(bersaglio, registro)
    if problemi:
        for messaggio in problemi:
            print(messaggio, file=sys.stderr)
        return 1

    export = json.loads(export_json.read_text(encoding="utf-8"))
    lcov = lcov_file.read_text(encoding="utf-8")
    osservazioni, errori = osserva(bersaglio, registro, export, lcov)
    if errori:
        for messaggio in errori:
            print(messaggio, file=sys.stderr)
        return 1

    impronta, problemi = impronta_del_perimetro(percorsi)
    if problemi:
        for messaggio in problemi:
            print(messaggio, file=sys.stderr)
        return 1

    documento = {
        "schema_version": SCHEMA_ARTEFATTO,
        "descrizione": (
            f"Che cosa il replay deterministico dei semi di `{bersaglio.nome}` ha "
            f"raggiunto. Prodotto da `scripts/fuzz-profondita.sh "
            f"{bersaglio.nome}`; letto da scripts/check_profondita_fuzz.py, che "
            "ne ricalcola l'impronta del perimetro e rifiuta una misura "
            "invecchiata."
        ),
        "target": registro["target"],
        "corpus": {"input": input_totali},
        "impronta_perimetro": impronta,
        "nota_sulla_revisione": (
            "la misura non nomina una revisione, e non e' una dimenticanza. Il "
            "campo `revisione_di_misura` c'e' stato, dichiarato informativo, e "
            "diceva lo SHA di HEAD al momento della corsa: uno SHA locale "
            "riscritto prima di entrare in storia lo rendeva irrisolvibile, e un "
            "identificatore che non risolve piu' e' peggio di nessun "
            "identificatore. A legare la misura all'albero e' l'impronta del "
            "perimetro, che invecchia quando il codice attraversato cambia."
        ),
        "nota_sul_raggiungimento": (
            "ogni requisito dice `raggiunto`, non quante volte. Lo schema 1 "
            "registrava il `conteggio` delle esecuzioni e il gate ne pretendeva "
            "il segno: il numero pero' alternava fra due stati a parita' di "
            "albero -- otto requisiti su trentacinque, su quattro corse di cui "
            "due bit-identiche -- e ogni rimisura scriveva quel rumore in un file "
            "versionato, indistinguibile da un fatto. `raggiunto` e' la stessa "
            "affermazione senza la parte instabile: lo deriva questo generatore "
            "da `conteggio > 0`, e il gate lo pretende booleano e vero."
        ),
        "requisiti": sorted(osservazioni, key=lambda voce: voce["id"]),
    }
    uscita.parent.mkdir(parents=True, exist_ok=True)
    uscita.write_text(
        json.dumps(documento, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
        newline="\n",
    )
    raggiunti = sum(1 for voce in osservazioni if voce["raggiunto"])
    print(
        f"profondita' registrata in {uscita.relative_to(ROOT).as_posix()}: "
        f"{raggiunti}/{len(osservazioni)} requisiti raggiunti su "
        f"{input_totali} input"
    )
    return 0


def main(argv: list[str] | None = None) -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument("bersaglio", choices=sorted(BERSAGLI), help="quale fuzz target")
    argomenti.add_argument("--registra", type=Path, help="l'export JSON di llvm-cov")
    argomenti.add_argument("--lcov", type=Path, help="il report lcov della stessa misura")
    argomenti.add_argument("--input", type=int, default=0, help="quanti input ha rigiocato")
    opzioni = argomenti.parse_args(argv)
    bersaglio = BERSAGLI[opzioni.bersaglio]

    try:
        registro = leggi_registro(bersaglio)
    except RegistroMalformato as errore:
        print(str(errore), file=sys.stderr)
        return 1

    artefatto = ROOT / registro.get("artefatto", "")

    if opzioni.registra:
        if not opzioni.lcov:
            print("--registra richiede anche --lcov", file=sys.stderr)
            return 2
        return registra(bersaglio, opzioni.registra, opzioni.lcov, artefatto, opzioni.input)

    if not artefatto.exists():
        print(
            f"{registro.get('artefatto')}: misura di profondita' assente. Si "
            f"produce con `scripts/fuzz-profondita.sh {bersaglio.nome}`.",
            file=sys.stderr,
        )
        return 1
    try:
        misura = json.loads(artefatto.read_text(encoding="utf-8"))
    except json.JSONDecodeError as errore:
        print(f"{registro.get('artefatto')}: non e' JSON leggibile ({errore})", file=sys.stderr)
        return 1

    errori = verifica(bersaglio, registro, misura)
    for messaggio in errori:
        print(messaggio, file=sys.stderr)
    if errori:
        return 1

    quanti = len(misura.get("requisiti", []))
    print(
        f"profondita' del target {registro['target']}: {quanti} requisiti "
        f"raggiunti, misura valida per il perimetro corrente."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
