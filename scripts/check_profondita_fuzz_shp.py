#!/usr/bin/env python3
"""Il target `shp_reader` raggiunge davvero il parsing `.shp`/`.dbf`.

# Il difetto che questo gate esiste per chiudere

`fuzz.reader-shapefile` era bloccante perche' `shp_wkb` converte WKB e forme
ESRI in memoria e veniva contato come copertura del formato. Rimpiazzarlo con un
target che apre un file non basta a chiudere il blocco: una build che compila e
un replay senza crash **non** dimostrano che i semi arrivino al parser. Un
bundle rifiutato all'apertura non fa crashare niente, e da fuori e'
indistinguibile da uno letto per intero.

Serve percio' una misura, e serve che qualcuno la legga. Questo gate legge
l'artefatto prodotto da `scripts/fuzz-profondita-shp.sh` e pretende che ogni
requisito dichiarato in
`assurance/registries/profondita-fuzz-shapefile.json` sia stato **raggiunto**.

# Perche' l'artefatto non e' creduto sulla parola

Una misura committata invecchia: il codice cambia, il target smette di
raggiungere cio' che raggiungeva, e il JSON continua a dire di si'. Qui la
misura porta l'**impronta del perimetro** -- i percorsi che possono cambiare
cio' che il target attraversa -- e il gate la ricalcola dal working tree. Se una
sola riga di `driver-shp`, del target, dei semi o di `Cargo.lock` cambia,
l'impronta diverge e il gate diventa rosso finche' la misura non viene rifatta.

Non e' legata allo SHA: il commit che *pubblica* una misura ha per forza uno SHA
diverso da quello su cui e' girata, e legarla al commit la renderebbe scaduta
per costruzione.

# Che cosa questo gate non dice

Che un ramo sia **raggiunto** non dice che il suo contratto sia verificato. La
separazione fra «non raggiunto» e «raggiunto» e' qui; la verifica semantica sta
nelle sonde di `driver-shp`, che chiamano lo stesso entry point del target sui
semi versionati e guardano righe drenate e messaggi di rifiuto.

# Uso

    python3 scripts/check_profondita_fuzz_shp.py
    python3 scripts/check_profondita_fuzz_shp.py --registra <export.json> --lcov <file.lcov>

Il secondo modo lo invoca `scripts/fuzz-profondita-shp.sh` dopo la misura: e' il
solo posto in cui l'artefatto viene scritto.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent
REGISTRO = ROOT / "assurance" / "registries" / "profondita-fuzz-shapefile.json"

# Il nucleo minimo dei requisiti, scritto **qui** e non solo nel registro, con
# la famiglia di ciascuno.
#
# Senza l'insieme, il modo piu' semplice di rendere verde questo gate sarebbe
# svuotare il registro: zero requisiti, zero requisiti mancati. E' la stessa
# famiglia del `"test": []` del contratto, e si chiude allo stesso modo -- un
# insieme che il registro deve contenere e che il gate conosce senza
# chiederglielo.
#
# La **famiglia** serve per una ragione diversa, e altrettanto concreta. Un
# requisito di riga individua un ramo preciso del nostro sorgente; uno di
# funzione dice che un simbolo e' stato eseguito. `shp.geometria-multipunto`
# prova che il decoder esterno decodifica un multipunto;
# `prevalidazione.conteggi-del-multipunto` prova che la difesa che lo precede e'
# stata percorsa. Sono due affermazioni diverse, e senza la famiglia la seconda
# poteva essere riscritta come la prima -- a nome di un simbolo che esiste
# comunque -- lasciando il gate verde su un ramo mai eseguito.
FAMIGLIA_DEL_NUCLEO = {
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
}

# Derivato, non riscritto: due elenchi da tenere allineati a mano divergono.
NUCLEO_OBBLIGATORIO = frozenset(FAMIGLIA_DEL_NUCLEO)

# I percorsi che il perimetro deve contenere comunque: sono il codice che la
# misura attraversa e gli input che ce la portano. Un perimetro ridotto a
# `Cargo.lock` renderebbe la misura eterna.
PERIMETRO_OBBLIGATORIO = frozenset(
    {
        "Cargo.lock",
        "crates/driver-shp/src",
        "fuzz/Cargo.toml",
        "fuzz/fuzz_targets",
        "fuzz/seeds/shp_reader",
    }
)


class RegistroMalformato(Exception):
    """Il registro non e' leggibile: nessuna verifica ha senso senza."""


# --- il registro ------------------------------------------------------------


def leggi_registro(percorso: Path = REGISTRO) -> dict[str, Any]:
    try:
        documento = json.loads(percorso.read_text(encoding="utf-8"))
    except FileNotFoundError as errore:
        raise RegistroMalformato(f"{percorso}: registro assente") from errore
    except json.JSONDecodeError as errore:
        raise RegistroMalformato(f"{percorso}: non e' JSON leggibile ({errore})") from errore
    if not isinstance(documento, dict):
        raise RegistroMalformato(f"{percorso}: la radice non e' un oggetto")
    return documento


def requisiti(registro: dict[str, Any]) -> tuple[list[dict[str, Any]], list[str]]:
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

    senza = sorted(NUCLEO_OBBLIGATORIO - set(identita))
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
        for identita, famiglia in FAMIGLIA_DEL_NUCLEO.items()
        if identita in per_identita and per_identita[identita] != famiglia
    )
    errori.extend(
        f"{messaggio}. Un ramo del sorgente e un simbolo eseguito non sono la "
        "stessa prova, e scambiarli lascerebbe verde un ramo mai percorso."
        for messaggio in spostati
    )

    dichiarato = registro.get("nucleo")
    if not isinstance(dichiarato, list) or set(dichiarato) != NUCLEO_OBBLIGATORIO:
        errori.append(
            "`nucleo` del registro diverso da quello che il gate pretende. Il "
            "registro lo scrive per chi legge, il gate lo conosce per non "
            "dipendere da chi lo scrive: quando divergono, uno dei due mente."
        )

    return uniti, errori


# --- il perimetro -----------------------------------------------------------


def percorsi_del_perimetro(registro: dict[str, Any]) -> tuple[list[str], list[str]]:
    perimetro = registro.get("perimetro")
    if not isinstance(perimetro, dict):
        return [], ["`perimetro` assente: senza, la misura non scadrebbe mai"]
    percorsi = perimetro.get("percorsi")
    if not isinstance(percorsi, list) or not all(
        isinstance(p, str) and p for p in percorsi
    ):
        return [], ["`perimetro.percorsi` non e' un elenco di percorsi"]

    errori: list[str] = []
    senza = sorted(PERIMETRO_OBBLIGATORIO - set(percorsi))
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


def riga_dell_ancora(relativo: str, ancora: str, strumentate: dict[int, int]) -> tuple[int | None, list[str]]:
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
    registro: dict[str, Any], export: dict[str, Any], lcov: str
) -> tuple[list[dict[str, Any]], list[str]]:
    """Che cosa la misura dice di ciascun requisito. Nessun giudizio, qui."""
    voci, errori = requisiti(registro)
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
            migliore = max(trovati.values(), default=None)
            osservazioni.append(
                {
                    "id": voce["id"],
                    "famiglia": "funzione",
                    "simboli": len(trovati),
                    "conteggio": migliore,
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
                "conteggio": strumentate.get(numero, 0),
            }
        )
    return osservazioni, errori


# --- la verifica ------------------------------------------------------------


def verifica(registro: dict[str, Any], misura: dict[str, Any]) -> list[str]:
    voci, errori = requisiti(registro)
    percorsi, problemi = percorsi_del_perimetro(registro)
    errori.extend(problemi)
    if errori:
        return errori

    if misura.get("target") != registro.get("target"):
        errori.append(
            f"la misura riguarda il target «{misura.get('target')}», il registro "
            f"«{registro.get('target')}»"
        )

    corpus = misura.get("corpus")
    if not isinstance(corpus, dict) or not isinstance(corpus.get("input"), int):
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
            "la misura: va rifatta con scripts/fuzz-profondita-shp.sh."
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

    for identita in sorted(dichiarati & set(per_id)):
        voce = per_id[identita]
        conteggio = voce.get("conteggio")
        if not isinstance(conteggio, int) or isinstance(conteggio, bool):
            errori.append(f"{identita}: conteggio «{conteggio}» non e' un intero")
        elif conteggio <= 0:
            errori.append(
                f"{identita}: non raggiunto dal replay del corpus (conteggio "
                f"{conteggio}). Il target compila e non crasha, e non arriva qui."
            )
        if voce.get("famiglia") == "funzione" and not voce.get("simboli"):
            errori.append(
                f"{identita}: nessun simbolo corrisponde ai segmenti dichiarati. "
                "O la funzione e' stata rinominata, o non e' stata compilata nel "
                "target."
            )
    return errori


# --- i due modi -------------------------------------------------------------


def registra(export_json: Path, lcov_file: Path, uscita: Path, input_totali: int) -> int:
    registro = leggi_registro()
    percorsi, problemi = percorsi_del_perimetro(registro)
    if problemi:
        for messaggio in problemi:
            print(messaggio, file=sys.stderr)
        return 1

    export = json.loads(export_json.read_text(encoding="utf-8"))
    lcov = lcov_file.read_text(encoding="utf-8")
    osservazioni, errori = osserva(registro, export, lcov)
    if errori:
        for messaggio in errori:
            print(messaggio, file=sys.stderr)
        return 1

    impronta, problemi = impronta_del_perimetro(percorsi)
    if problemi:
        for messaggio in problemi:
            print(messaggio, file=sys.stderr)
        return 1

    revisione = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, text=True, check=False
    ).stdout.strip()

    documento = {
        "schema_version": 1,
        "descrizione": (
            "Che cosa il replay deterministico del corpus di `shp_reader` ha "
            "raggiunto. Prodotto da scripts/fuzz-profondita-shp.sh; letto da "
            "scripts/check_profondita_fuzz_shp.py, che ne ricalcola l'impronta "
            "del perimetro e rifiuta una misura invecchiata."
        ),
        "target": registro["target"],
        "corpus": {"input": input_totali},
        "impronta_perimetro": impronta,
        "revisione_di_misura": revisione,
        "nota_sulla_revisione": (
            "informativa: il commit che pubblica questa misura ne ha per forza "
            "un'altra. A legare la misura all'albero e' l'impronta del "
            "perimetro, non questo campo."
        ),
        "requisiti": sorted(osservazioni, key=lambda voce: voce["id"]),
    }
    uscita.parent.mkdir(parents=True, exist_ok=True)
    uscita.write_text(
        json.dumps(documento, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
        newline="\n",
    )
    raggiunti = sum(1 for voce in osservazioni if (voce.get("conteggio") or 0) > 0)
    print(
        f"profondita' registrata in {uscita.relative_to(ROOT).as_posix()}: "
        f"{raggiunti}/{len(osservazioni)} requisiti raggiunti su "
        f"{input_totali} input"
    )
    return 0


def main(argv: list[str] | None = None) -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument("--registra", type=Path, help="l'export JSON di llvm-cov")
    argomenti.add_argument("--lcov", type=Path, help="il report lcov della stessa misura")
    argomenti.add_argument("--input", type=int, default=0, help="quanti input ha rigiocato")
    opzioni = argomenti.parse_args(argv)

    try:
        registro = leggi_registro()
    except RegistroMalformato as errore:
        print(str(errore), file=sys.stderr)
        return 1

    artefatto = ROOT / registro.get("artefatto", "")

    if opzioni.registra:
        if not opzioni.lcov:
            print("--registra richiede anche --lcov", file=sys.stderr)
            return 2
        return registra(opzioni.registra, opzioni.lcov, artefatto, opzioni.input)

    if not artefatto.exists():
        print(
            f"{registro.get('artefatto')}: misura di profondita' assente. Si "
            "produce con scripts/fuzz-profondita-shp.sh.",
            file=sys.stderr,
        )
        return 1
    try:
        misura = json.loads(artefatto.read_text(encoding="utf-8"))
    except json.JSONDecodeError as errore:
        print(f"{registro.get('artefatto')}: non e' JSON leggibile ({errore})", file=sys.stderr)
        return 1

    errori = verifica(registro, misura)
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
