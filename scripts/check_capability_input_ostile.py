#!/usr/bin/env python3
"""La capability `hostile_input_hardened` dice il vero, o e' un booleano.

# Che cosa afferma la capability

Che ogni testo che il driver interpreta come geometria passi da un'analisi che
applica i tetti del bordo -- byte, componenti, profondita' -- **mentre**
consuma, e non dopo aver costruito l'albero. E' la garanzia del lotto S12, ed e'
dichiarata nel catalogo perche' un consumatore possa verificarla senza leggere
il nostro codice.

`false` non dice «insicuro»: dice **non dichiarato**. Un driver che legge un
formato binario ha altre difese, e riassumerle in un booleano solo lo renderebbe
inutile.

# Perche' un gate

Una capability e' un'affermazione che il driver fa su se stesso. Senza un
confronto con il codice, il modo piu' semplice di ottenerla e' scriverla: un
`true` costa un carattere, e nessun test esistente lo smentirebbe -- il
descrittore compila comunque, il catalogo si serializza comunque.

Qui la dichiarazione viene confrontata con il codice in due modi, e servono
tutti e due.

## Uno: la chiamata

Il driver deve **chiamare** uno dei due entry point che il lotto S12 ha reso
progressivi. Chi dichiara `true` senza chiamarli e' rosso; chi li chiama senza
dichiararlo pure -- la seconda direzione conta quanto la prima, perche' una
garanzia che c'e' e non e' dichiarata e' una garanzia che nessuno usa.

Questo controllo legge il **testo**, e va detto cio' che percio' non puo'
vedere: una chiamata dentro `#[cfg(any())]`, dietro una feature spenta, o nel
corpo di una macro che nessuno invoca, e' testo che assomiglia a una chiamata e
non e' codice che gira. Distinguerli vorrebbe dire avere un compilatore, e un
gate che legge sorgenti non ce l'ha.

## Due: la prova eseguita

Percio' ogni driver che dichiara `true` deve avere anche una prova **eseguita
attraverso il proprio entry point pubblico**, con una quota stretta: si apre un
dataset vero con un tetto piu' piccolo del default, si da' in pasto una
geometria che lo supera, e si pretende il rifiuto tipizzato -- piu' l'asserzione
che con il default quella stessa geometria passerebbe, se no il test non
distinguerebbe la quota dal caso.

E' questa la parte che regge il peso. Una chiamata esclusa dalla compilazione
non fa passare nessun test; una sostituita con il parser non progressivo fa
fallire il rifiuto o il codice. Il controllo sul testo resta perche' e'
istantaneo e coglie la sparizione pura, ma da solo non basterebbe, e questo file
non finge il contrario.

# Uso

    python3 scripts/check_capability_input_ostile.py
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

# Lo spoglio di commenti e stringhe e' gia' scritto, provato e usato da un altro
# gate: riscriverlo qui vorrebbe dire avere due implementazioni della stessa
# regola, e la seconda sbaglierebbe da sola il giorno in cui la prima cambia.
from check_wkb_limits_defaults import spoglia  # noqa: E402

# Eseguire test nominati e pretendere che ognuno passi una volta sola e' gia'
# scritto e provato: `--exact` da solo non basta, perche' un filtro che non
# trova niente lascia `cargo test` a zero test e a exit 0. Riscriverlo qui
# vorrebbe dire avere due implementazioni della stessa regola.
from check_prove_di_confine import esegui as esegui_i_test  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
CRATES = ROOT / "crates"

# Gli entry point che applicano i tetti **durante** il parse. Sono due, e sono
# i soli: aggiungerne uno qui senza averlo reso progressivo sarebbe il modo di
# far passare un driver che non lo e'.
INGRESSI_PROGRESSIVI = (
    # WKT: `driver_common::wkt_progressivo` dietro il confine pubblico.
    "parse_wkt_bounded",
    # GeoJSON: la deserializzazione che addebita mentre serde consegna.
    "geometria_progressiva::analizza",
)

# Un attraversamento e' una **chiamata**, non il nome scritto da qualche parte.
#
# La prima stesura cercava il nome come sottostringa, e tre cose diverse
# passavano per la stessa: una chiamata vera, un commento che la nomina -- e
# questo file, come ogni altro qui dentro, spiega le proprie ragioni nominando
# i simboli -- e un `use` che importa il simbolo senza usarlo. La controprova
# e' stata costruita e passava: `hostile_input_hardened = true` con il solo
# commento.
#
# Il sorgente viene percio' spogliato di commenti e stringhe, e cio' che resta
# deve essere il nome seguito da una parentesi aperta: un `use` non ne ha.
def _invocazione(nome: str) -> re.Pattern[str]:
    return re.compile(rf"(?<![A-Za-z0-9_]){re.escape(nome)}\s*\(")


# Un `use` non e' un attraversamento: importa un simbolo, non lo chiama.
IMPORTAZIONE = re.compile(r"^\s*(pub\s+)?use\s[^;]*;", re.MULTILINE)

# La prova eseguita di ciascun driver che dichiara `true`: l'identita' esatta
# del test che apre un dataset vero attraverso l'entry point pubblico e pretende
# il rifiuto tipizzato.
#
# La quota stretta e' quella sui **componenti**, e non e' un dettaglio.
#
# La prima stesura nominava i tre test sul cap in byte, e non provavano questo
# lotto: il cap in byte esisteva prima del parser progressivo e scatta **prima**
# di deserializzare, quindi restavano verdi anche rimettendo il parser vecchio.
# Verificato neutralizzando il budget dei componenti nei due parser: i tre test
# sul cap in byte restano verdi, questi tre diventano rossi. E' la differenza
# fra provare S5 e provare S12.
#
# I tre input stanno comodamente sotto il cap in byte, passano con il tetto sui
# componenti al valore esatto e sono rifiutati con `LimitExceeded` a un
# componente di meno.
#
# L'elenco e' una **mappa chiusa**: un driver che dichiara `true` e non compare
# qui e' rosso, e una voce che nomina un driver che non dichiara `true` pure. E'
# la stessa disciplina di `check_prove_di_confine.py`, per la stessa ragione: un
# riferimento testuale non e' una prova, un'esecuzione si'.
PROVE_DELLA_CAPABILITY: dict[str, tuple[str, ...]] = {
    "driver-csv": ("tests::la_cella_wkt_e_rifiutata_per_componenti_sotto_il_cap_in_byte",),
    "driver-geojson": (
        "tests::la_geometria_e_rifiutata_per_componenti_sotto_il_cap_in_byte",
    ),
    "driver-xls": ("tests::la_cella_wkt_e_rifiutata_per_componenti_sotto_il_cap_in_byte",),
}


def mappa_valida(prove: dict[str, tuple[str, ...]]) -> list[str]:
    """Ogni voce nomina almeno una prova, e nomina prove distinte.

    `crate in prove` era soddisfatto da `"driver-csv": ()`: la voce c'era, il
    gate la contava, e `cargo test -- --exact` senza nomi non filtra niente --
    esegue tutto ed esce 0. Un `true` si sarebbe tenuto cancellando la prova.
    """
    errori: list[str] = []
    for crate, nomi in sorted(prove.items()):
        if not isinstance(nomi, tuple) or not nomi:
            errori.append(
                f"{crate}: la voce delle prove e' vuota. Una voce vuota conta "
                "come prova presente e non esegue niente, che e' il modo di "
                "tenersi un `true` cancellando cio' che lo giustifica."
            )
            continue
        for nome in nomi:
            if not isinstance(nome, str) or not nome:
                errori.append(f"{crate}: identita' di prova non valida: «{nome}»")
        ripetute = sorted({n for n in nomi if nomi.count(n) > 1})
        if ripetute:
            errori.append(
                f"{crate}: le prove {ripetute} sono dichiarate piu' di una volta. "
                "Il harness elenca ogni test una volta sola, quindi il conteggio "
                "annuncerebbe piu' prove di quante ne girano."
            )
    return errori

# Il commento con cui un driver apre la propria dichiarazione. Il valore sta
# nella prima riga non commentata che segue: leggerlo riga per riga invece che
# con un'espressione regolare evita il backtracking, ed e' anche piu' facile da
# leggere di un pattern che deve saltare un numero qualunque di commenti.
APERTURA = "`hostile_input_hardened`:"


def crate_dei_driver(radice: Path) -> list[str]:
    """I driver, che sono le crate che **costruiscono un descrittore**.

    Non quelle il cui nome comincia per `driver-`: `driver-common` e' codice
    condiviso e non dichiara niente al catalogo. Derivare l'elenco dal
    descrittore invece che dal nome fa entrare da solo un driver nuovo, e non
    fa entrare una libreria che si chiama come loro.
    """
    trovati = []
    for percorso in sorted((radice / "crates").glob("*/src/lib.rs")):
        if "FormatDescriptor::const_new(" in percorso.read_text(encoding="utf-8"):
            trovati.append(percorso.parent.parent.name)
    return trovati


def _sorgenti(radice: Path, crate: str) -> str:
    """Tutto il codice della crate, meno i suoi test.

    I test chiamano gli entry point per provarli, ed e' il loro mestiere:
    contarli come uso di produzione direbbe che un driver e' irrigidito perche'
    una sonda lo esercita.
    """
    pezzi = []
    for percorso in sorted((radice / "crates" / crate / "src").rglob("*.rs")):
        testo = percorso.read_text(encoding="utf-8")
        principio = testo.find("mod tests {")
        if principio == -1:
            principio = testo.find("mod sonde {")
        pezzi.append(testo if principio == -1 else testo[:principio])
    return "\n".join(pezzi)


def dichiarato(radice: Path, crate: str) -> bool | None:
    """Il valore che il driver scrive nel proprio descrittore."""
    testo = (radice / "crates" / crate / "src" / "lib.rs").read_text(encoding="utf-8")
    righe = testo.splitlines()
    for indice, riga in enumerate(righe):
        if APERTURA not in riga:
            continue
        for seguente in righe[indice + 1 :]:
            nuda = seguente.strip()
            if nuda.startswith("//") or not nuda:
                continue
            if nuda in ("true,", "false,"):
                return nuda == "true,"
            break
    return None


def osservato(radice: Path, crate: str) -> bool:
    """Il driver **chiama** davvero un ingresso progressivo?

    Non «lo nomina»: lo chiama. Commenti e stringhe spariscono prima di
    guardare, le importazioni pure, e cio' che resta deve essere il nome
    seguito da una parentesi aperta.
    """
    sorgenti = IMPORTAZIONE.sub(" ", spoglia(_sorgenti(radice, crate)))
    return any(
        _invocazione(ingresso).search(sorgenti) for ingresso in INGRESSI_PROGRESSIVI
    )


def verifica(
    radice: Path, prove: dict[str, tuple[str, ...]] | None = None
) -> list[str]:
    """La dichiarazione, la chiamata e la prova eseguita si corrispondono.

    `prove` e' iniettabile perche' le sonde di questo gate costruiscono alberi
    finti, dove i test veri non esistono: iniettarlo e' l'unico modo di provare
    la regola invece della mappa.
    """
    if prove is None:
        prove = PROVE_DELLA_CAPABILITY
    errori = mappa_valida(prove)
    driver = crate_dei_driver(radice)
    for crate in sorted(set(prove) - set(driver)):
        errori.append(
            f"{crate}: e' nominato fra le prove della capability e non e' un "
            "driver di questo albero. Una prova che nomina un driver che non "
            "c'e' non prova niente, e nasconde che il driver vero non ne ha una."
        )
    for crate in driver:
        detto = dichiarato(radice, crate)
        visto = osservato(radice, crate)
        # `crate in prove` non basta: una voce vuota c'e' e non prova niente.
        provato = bool(prove.get(crate))
        if detto is not None and detto and not provato:
            errori.append(
                f"{crate}: dichiara `hostile_input_hardened = true` e non ha una "
                "prova **eseguita** attraverso il proprio entry point pubblico. "
                "Il controllo sul testo non distingue una chiamata che gira da "
                "una dentro `#[cfg(any())]`, dietro una feature spenta o in una "
                "macro mai invocata: a distinguerle e' un test che apre un "
                "dataset vero con una quota stretta e pretende il rifiuto."
            )
        if detto is not None and not detto and provato:
            errori.append(
                f"{crate}: ha una prova della capability e dichiara "
                "`hostile_input_hardened = false`. La mappa delle prove e i "
                "descrittori devono dire la stessa cosa."
            )
        if detto is None:
            errori.append(
                f"{crate}: la capability `hostile_input_hardened` non e' "
                "dichiarata con la sua ragione. Il descrittore la porta comunque "
                "-- e' un campo obbligatorio -- ma senza il commento che la "
                "motiva nessuno sa perche' vale quel che vale."
            )
            continue
        if detto and not visto:
            errori.append(
                f"{crate}: dichiara `hostile_input_hardened = true` e non "
                f"**chiama** nessuno di {list(INGRESSI_PROGRESSIVI)}. Nominarli "
                "in un commento o importarli non basta: un `true` costa un "
                "carattere, la garanzia costa un parser."
            )
        if visto and not detto:
            errori.append(
                f"{crate}: attraversa un ingresso progressivo e dichiara "
                "`hostile_input_hardened = false`. Una garanzia che c'e' e non e' "
                "dichiarata e' una garanzia che nessun consumatore puo' usare."
            )
    return errori


def main() -> int:
    errori = verifica(ROOT)
    if not errori:
        for crate, nomi in sorted(PROVE_DELLA_CAPABILITY.items()):
            errori.extend(esegui_i_test(crate, nomi, "la prova della capability"))
    for messaggio in errori:
        print(messaggio, file=sys.stderr)
    if errori:
        return 1
    quanti = crate_dei_driver(ROOT)
    irrigiditi = [c for c in quanti if dichiarato(ROOT, c)]
    quante = sum(len(n) for n in PROVE_DELLA_CAPABILITY.values())
    print(
        f"capability `hostile_input_hardened` verificata su {len(quanti)} driver: "
        f"{len(irrigiditi)} la dichiarano ({', '.join(irrigiditi)}), ognuno "
        "chiama un'analisi che applica i tetti durante il parse, e ognuno lo "
        f"**dimostra**: {quante} prove eseguite attraverso l'entry point "
        "pubblico con il tetto sui **componenti** stretto e l'input sotto il cap "
        "in byte, ciascuna passata. Il cap in byte non proverebbe questo lotto: "
        "esisteva prima del parser progressivo."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
