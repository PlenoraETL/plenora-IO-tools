#!/usr/bin/env python3
"""Le categorie di perdita vengono da un vocabolario chiuso, dichiarato fuori dal codice.

# Che cosa protegge

Le chiavi di `LossReport::counts` finiscono nella busta JSON della CLI, che e'
un'interfaccia esterna con un ICD congelato (`release/cli-protocol-v1.json`).
Una chiave costruita a runtime e' una chiave che nessuno puo' enumerare
leggendo il codice; se la costruisce con un nome preso dal file, allora **la
cardinalita' e i byte della busta li decide chi fornisce il file**.

Il censimento del lotto `wire.loss-report` ha trovato esattamente una via del
genere, in `driver-dxf`, e nessuno se n'era accorto perche' niente la cercava.
Questo gate esiste perche' non ne nasca una seconda.

# Perche' l'autorita' sta in un registro

`assurance/registries/categorie-di-perdita.json` e' l'elenco, e sta fuori dai
sorgenti che lo producono. Un gate che ricavasse il vocabolario dai siti che
deve controllare direbbe soltanto che il codice e' uguale a se stesso: una
categoria dimenticata non sarebbe una lacuna, sarebbe la definizione del
perimetro. La verifica e' percio' nei **due versi** -- ogni categoria prodotta
sta nel registro, e ogni voce del registro ha almeno un produttore -- cosi' che
ne' aggiungerne una nel codice ne' lasciarne una morta nel registro passi.

# Che cosa non guarda

Il valore delle costanti si legge dal testo della loro definizione, non
dall'esecuzione: il gate afferma che il sorgente dichiara quella stringa, non
che il compilatore ne produca un'altra. E' la stessa fiducia che si da' a
`cargo fmt --check`, ed e' dichiarata qui invece di essere sottintesa.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
REGISTRO = ROOT / "assurance" / "registries" / "categorie-di-perdita.json"
CRATES = ROOT / "crates"

#: Il tetto in byte UTF-8 su un identificatore di categoria, deciso col lotto
#: `wire.loss-report`. Sta qui e nel registro, e il gate pretende che i due
#: coincidano: due verita' divergono, e divergono in silenzio.
LIMITE_ID_BYTE = 128

#: `LossReport::record(&str, u64)` ne prende **due**. `ShpRowDiagnostics::record`
#: ne prende quattro ed e' un omonimo che con la busta non c'entra: distinguerli
#: per nome del ricevente sarebbe una convenzione, l'arieta' e' un fatto.
ARIETA_DI_RECORD = 2

MODULO_DI_PROVA = re.compile(r"^(pub )?mod (tests|sonde)")

#: Le sole sequenze di escape che compaiono in queste stringhe.
#:
#: `bytes.decode("unicode_escape")` sarebbe la scorciatoia, e sbaglia: tratta i
#: byte UTF-8 come latin-1, cosi' «entità» diventa «entitÃ ». Le categorie DXF
#: sono in italiano e hanno gli accenti, quindi la scorciatoia le avrebbe rese
#: tutte diverse da quelle del registro -- e il gate lo ha mostrato subito.
ESCAPE = ((r"\\", "\\"), (r"\"", '"'), (r"\n", "\n"), (r"\t", "\t"))


def testo_rust(grezzo: str) -> str:
    """Il contenuto di un literal Rust, con gli escape sciolti."""
    risultato, i = [], 0
    while i < len(grezzo):
        for sequenza, valore in ESCAPE:
            if grezzo.startswith(sequenza, i):
                risultato.append(valore)
                i += len(sequenza)
                break
        else:
            risultato.append(grezzo[i])
            i += 1
    return "".join(risultato)

#: `const NOME: &str = "valore";`, anche spezzata su piu' righe.
COSTANTE = re.compile(
    r"const\s+([A-Z_][A-Z0-9_]*)\s*:\s*&(?:'static\s+)?str\s*=\s*\"((?:[^\"\\]|\\.)*)\"\s*;"
)

#: Un metodo `const fn <nome>(self) -> &'static str` il cui corpo e' un `match`
#: su costanti: e' la forma con cui `driver-gpkg` chiude le sue quattro
#: categorie, e il gate la risolve invece di fidarsi.
METODO_CHIUSO = re.compile(
    r"const fn (\w+)\([^)]*\)\s*->\s*(?:Option<)?&'static str>?\s*\{(.*?)\n    \}",
    re.DOTALL,
)


def registro(testo: str | None = None) -> dict:
    """Il registro, iniettabile perche' le sonde ne costruiscano di storti."""
    grezzo = REGISTRO.read_text(encoding="utf-8") if testo is None else testo
    return json.loads(grezzo)


def confine_delle_prove(righe: list[str]) -> int:
    """La riga da cui comincia il modulo di prova.

    Non il primo `#[cfg(test)]`: quell'attributo compare anche su singoli
    elementi molto prima del modulo, e prenderlo per confine fa sparire dal
    censimento meta' dei siti di produzione -- e' successo, ed e' il motivo per
    cui questa funzione ha una prova sua.
    """
    for i, riga in enumerate(righe):
        if (
            riga.startswith("#[cfg(test)]")
            and i + 1 < len(righe)
            and MODULO_DI_PROVA.match(righe[i + 1])
        ):
            return i
        if MODULO_DI_PROVA.match(riga):
            return i
    return len(righe)


def sorgenti() -> list[tuple[str, str]]:
    return [
        (f.relative_to(ROOT).as_posix(), f.read_text(encoding="utf-8"))
        for f in sorted(CRATES.rglob("*.rs"))
    ]


def argomenti(testo: str, apertura: int) -> list[str] | None:
    """Gli argomenti della chiamata che apre a `apertura`, o `None` se non chiude."""
    livello = 0
    pezzi: list[str] = []
    inizio = apertura + 1
    for i in range(apertura, len(testo)):
        c = testo[i]
        if c in "([{":
            livello += 1
        elif c in ")]}":
            livello -= 1
            if livello == 0:
                pezzi.append(testo[inizio:i])
                puliti = [" ".join(p.split()) for p in pezzi]
                # Una virgola finale non introduce un argomento. Contarla faceva
                # sembrare di arieta' tre ogni chiamata scritta su piu' righe --
                # cioe' quasi tutte -- e quelle sparivano dal censimento.
                if puliti and not puliti[-1]:
                    puliti.pop()
                return puliti
        elif c == "," and livello == 1:
            pezzi.append(testo[inizio:i])
            inizio = i + 1
    return None


def ricevente(testo: str, punto: int) -> str:
    inizio = punto
    while inizio > 0 and (testo[inizio - 1].isalnum() or testo[inizio - 1] in "._"):
        inizio -= 1
    return testo[inizio:punto]


def siti_di_record(sorgenti_del_workspace: list[tuple[str, str]]) -> list[dict]:
    """Ogni chiamata a `record` con due argomenti, fuori dai moduli di prova."""
    trovati: list[dict] = []
    for percorso, testo in sorgenti_del_workspace:
        righe = testo.splitlines()
        confine = confine_delle_prove(righe)
        inizi, offset = [], 0
        for riga in righe:
            inizi.append(offset)
            offset += len(riga) + 1
        for m in re.finditer(r"\.record\(", testo):
            args = argomenti(testo, m.end() - 1)
            if args is None or len(args) != ARIETA_DI_RECORD:
                continue
            numero = max(i for i, inizio in enumerate(inizi) if inizio <= m.start())
            if numero >= confine:
                continue
            trovati.append(
                {
                    "file": percorso,
                    "riga": numero + 1,
                    "ricevente": ricevente(testo, m.start()),
                    "categoria": args[0],
                }
            )
    return trovati


def costanti(sorgenti_del_workspace: list[tuple[str, str]]) -> dict[str, str]:
    valori: dict[str, str] = {}
    for _, testo in sorgenti_del_workspace:
        for nome, valore in COSTANTE.findall(testo):
            valori[nome] = testo_rust(valore)
    return valori


def metodi_chiusi(sorgenti_del_workspace: list[tuple[str, str]]) -> dict[str, list[str]]:
    """`nome del metodo -> nomi delle costanti che restituisce`."""
    mappa: dict[str, list[str]] = {}
    for _, testo in sorgenti_del_workspace:
        for nome, corpo in METODO_CHIUSO.findall(testo):
            riferimenti = re.findall(
                r"(?:=>|Some\()\s*([A-Z_][A-Z0-9_]*)\s*[,)]", corpo
            )
            if riferimenti:
                mappa.setdefault(nome, []).extend(riferimenti)
    return mappa


def legature_locali(testo: str) -> dict[str, str]:
    """`let <nome> = <espressione>;` su una riga sola."""
    legature = {
        nome: espressione.strip()
        for nome, espressione in re.findall(r"let (\w+) = ([^;\n]+);", testo)
    }
    # `let Some(x) = <espressione> else { ... };` lega quanto un `let` semplice:
    # la categoria di `record_crs_representation_loss` arriva di qui.
    legature.update(
        {
            nome: espressione.strip()
            for nome, espressione in re.findall(
                r"let Some\((\w+)\) = ([^;\n]+?) else", testo
            )
        }
    )
    return legature


def risolvi(
    espressione: str,
    valori: dict[str, str],
    metodi: dict[str, list[str]],
    locali: dict[str, str],
) -> list[str] | None:
    """Le categorie che quell'espressione puo' produrre, o `None` se dinamica."""
    e = espressione.strip().lstrip("&")
    letterale = re.fullmatch(r"\"((?:[^\"\\]|\\.)*)\"", e)
    if letterale:
        return [testo_rust(letterale.group(1))]
    if re.fullmatch(r"[A-Z_][A-Z0-9_]*", e) and e in valori:
        return [valori[e]]
    # Anche con argomenti: `representation.categoria(state)` sceglie fra sei
    # costanti, e pretendere le parentesi vuote la faceva sembrare dinamica.
    chiamata = re.fullmatch(r"\w+\.(\w+)\([^)]*\)", e)
    if chiamata and chiamata.group(1) in metodi:
        return [valori[c] for c in metodi[chiamata.group(1)] if c in valori]
    if e in locali:
        return risolvi(locali[e], valori, metodi, {})
    return None


def verifica(
    dati: dict | None = None, fonti: list[tuple[str, str]] | None = None
) -> list[str]:
    """Gli errori trovati, elenco vuoto se il vocabolario regge.

    Registro e sorgenti sono iniettabili perche' le sonde possano costruirne di
    storti: un gate che si sa provare solo sul repository sano dice che oggi e'
    verde, non che domani diventerebbe rosso.
    """
    errori: list[str] = []
    reg = registro() if dati is None else dati

    if type(reg.get("schema_version")) is not int or reg.get("schema_version") != 1:
        errori.append("`schema_version` non e' l'intero 1: il registro non e' quello atteso.")
        return errori
    if reg.get("limite_di_lunghezza_byte") != LIMITE_ID_BYTE:
        errori.append(
            f"il registro dichiara un limite di {reg.get('limite_di_lunghezza_byte')} byte, "
            f"il gate ne applica {LIMITE_ID_BYTE}: due verita' divergono in silenzio."
        )

    voci = reg.get("categorie")
    if not isinstance(voci, list) or not voci:
        errori.append("`categorie` non e' un elenco non vuoto.")
        return errori

    dichiarate: list[str] = []
    for voce in voci:
        if not isinstance(voce, dict) or set(voce) != {"id", "superficie", "forma"}:
            errori.append(f"voce con chiavi diverse da id/superficie/forma: {voce}")
            continue
        ident = voce["id"]
        if not isinstance(ident, str) or not ident:
            errori.append(f"identificatore non e' una stringa non vuota: {ident!r}")
            continue
        byte = len(ident.encode("utf-8"))
        if byte > LIMITE_ID_BYTE:
            errori.append(
                f"«{ident[:40]}…» misura {byte} byte UTF-8, oltre il tetto di {LIMITE_ID_BYTE}."
            )
        dichiarate.append(ident)
    if len(set(dichiarate)) != len(dichiarate):
        ripetuti = sorted({i for i in dichiarate if dichiarate.count(i) > 1})
        errori.append(f"identificatori ripetuti nel registro: {ripetuti}")

    fonti = sorgenti() if fonti is None else fonti
    valori = costanti(fonti)
    metodi = metodi_chiusi(fonti)
    per_file = dict(fonti)

    prodotte: set[str] = set()
    dinamici: list[dict] = []
    for sito in siti_di_record(fonti):
        # La primitiva e la propagazione di `merge` vivono dentro il tipo: li'
        # `record` non produce una categoria, la riceve.
        # Solo `self`, e cioe' i metodi del tipo stesso: `merge` propaga
        # categorie altrui e la primitiva le riceve. Una funzione libera che
        # prende `&mut LossReport` e' invece un produttore come ogni altro --
        # escluderla per il nome del parametro sarebbe una convenzione, e
        # `declare_crs_inconsistency` sarebbe sparita dal censimento.
        if sito["file"].endswith("plenora-io-core/src/loss.rs") and sito["ricevente"] == "self":
            continue
        risolte = risolvi(
            sito["categoria"], valori, metodi, legature_locali(per_file[sito["file"]])
        )
        if risolte is None:
            dinamici.append(sito)
        else:
            prodotte.update(risolte)

    mancanti = sorted(prodotte - set(dichiarate))
    if mancanti:
        errori.append(
            f"categorie prodotte dal codice e assenti dal registro: {mancanti}. "
            "Una categoria che il registro non nomina non e' dichiarata a nessuno."
        )
    morte = sorted(set(dichiarate) - prodotte)
    if morte:
        errori.append(
            f"voci del registro che nessun sito produce: {morte}. Un vocabolario "
            "che nomina cio' che non esiste non e' un vocabolario, e' un elenco di desideri."
        )

    ammesse = reg.get("vie_dinamiche_ammesse")
    if not isinstance(ammesse, list):
        errori.append("`vie_dinamiche_ammesse` non e' un elenco.")
        return errori
    attese = {(v.get("file"), v.get("espressione")) for v in ammesse if isinstance(v, dict)}
    trovate = {(s["file"], s["categoria"]) for s in dinamici}
    if trovate - attese:
        errori.append(
            f"vie dinamiche non dichiarate: {sorted(trovate - attese)}. Una categoria "
            "costruita a runtime non si puo' enumerare leggendo il codice, e se prende "
            "un nome dal file la cardinalita' della busta la decide chi fornisce il file."
        )
    if attese - trovate:
        errori.append(
            f"vie dinamiche dichiarate e non trovate: {sorted(attese - trovate)}. Se sono "
            "state chiuse, vanno tolte dal registro: un'eccezione che sopravvive a se "
            "stessa autorizza in silenzio la prossima."
        )
    return errori


def main() -> int:
    errori = verifica()
    if errori:
        for e in errori:
            print(e, file=sys.stderr)
        print(
            "\nIl vocabolario delle categorie di perdita e' l'autorita' del registro, "
            "non del codice.",
            file=sys.stderr,
        )
        return 1
    reg = registro()
    print(
        f"categorie di perdita verificate: {len(reg['categorie'])} identificatori dichiarati "
        f"nel registro e prodotti dal codice, nessuno morto e nessuno in piu'; "
        f"{len(reg['vie_dinamiche_ammesse'])} via dinamica ammessa e dichiarata, "
        f"nessun'altra. Tetto di {LIMITE_ID_BYTE} byte UTF-8 per identificatore."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
