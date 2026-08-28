#!/usr/bin/env python3
"""I numeri del protocollo v2 sono gli stessi nel contratto e nel codice.

# Che cosa protegge

`release/cli-protocol-v2.json` dichiara i limiti della diagnostica: quante
categorie stanno in una sezione, quanti byte pesa un identificatore, quanti
byte ha una sezione e quanti la busta intera. Sono nove numeri, e sono la
promessa su cui un consumatore dimensiona i propri buffer.

Nel codice gli stessi nove numeri sono costanti, in **due** crate diversi.
Nessuno li confrontava. Un tetto alzato in `busta.rs` e non nel manifesto
lascerebbe il contratto a promettere il numero vecchio, e il modo in cui i due
divergono non si vede: restano entrambi coerenti con se stessi, le sonde
restano verdi, e il primo ad accorgersene e' un consumatore con un buffer
troppo corto.

E' il difetto chiuso in `7eb1060` sui metadati GeoParquet -- il documento
diceva del file cose che nessuno confrontava col file -- applicato al
contratto invece che ai metadati.

# Le tre clausole che erano prosa

Gli undici numeri erano confrontati con il codice; tre clausole di
**comportamento** no, ed erano prosa. La ratifica di `wire.loss-report` ha
trovato quattro clausole di questo stesso manifesto che descrivevano un codice
cambiato sotto di loro: l'invariante era `verified` perche' i numeri si
confrontano e la prosa non la guarda nessuno.

Un gate generale prosa-contro-comportamento non e' realistico; tre cose pero'
si strutturano, e allora si confrontano come i numeri:

  * l'**ordine canonico** di ragioni ed esempi, dai campi che le due `chiave()`
    compongono;
  * l'**identita'** su cui le respinte deduplicano, dal tipo dell'elemento dei
    due `BTreeSet` che le conservano;
  * le due **fonti** di `omesse_per_byte`, dai siti che incrementano il
    contatore.

In tutti e tre i casi il comportamento si **ricava dal codice**: il gate non
confronta due copie del manifesto, che divergerebbero insieme. Cio' che sa da
se' e' *dove* guardare -- il tipo, il campo, il nome della funzione -- e se
quel posto sparisce diventa rosso.

# Perche' la verifica e' nei due versi

Il gate pretende che ogni numero dichiarato abbia una costante **e** che ogni
costante del budget sia dichiarata. Un verso solo lascerebbe passare la meta'
piu' probabile dell'errore: un tetto che nasce nel codice e resta muto nel
manifesto non e' un'omissione rara, e' il modo normale in cui un limite
compare.

Lo stesso vale per le sonde. Il contratto le nomina, e il gate pretende che
esistano tutte e che non ne esista **nessun'altra** non nominata: una sonda che
il contratto non cita si puo' cancellare senza che il checkpoint se ne accorga,
e una che il contratto cita e non c'e' e' una prova promessa e mai eseguita.

# Che cosa non guarda

Il valore delle costanti si legge dal testo della loro definizione, non
dall'esecuzione: il gate afferma che il sorgente dichiara quel numero, non che
il compilatore ne produca un altro. E' la stessa fiducia che si da' a
`cargo fmt --check`, ed e' dichiarata qui invece di essere sottintesa.

Non guarda **se** i limiti siano quelli giusti. Se contratto e codice dicessero
entrambi dodici byte, questo gate sarebbe verde: che dodici KiB bastino al caso
peggiore lo prova una sonda, che qui viene pretesa e non eseguita. Le due cose
sono separate apposta -- questo confronta due dichiarazioni, quella misura un
documento -- e confonderle darebbe a ciascuna il credito dell'altra.
"""

from __future__ import annotations

import ast
import json
import re
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
CONTRATTO = ROOT / "release" / "cli-protocol-v2.json"
BUSTA = ROOT / "crates" / "plenora-io-cli" / "src" / "busta.rs"
LOSS = ROOT / "crates" / "plenora-io-core" / "src" / "loss.rs"
REGISTRO_CATEGORIE = ROOT / "assurance" / "registries" / "categorie-di-perdita.json"
DRIVER = ROOT / "crates" / "plenora-io-core" / "src" / "driver.rs"

#: Il numero dichiarato nel contratto, e la costante che lo produce.
#:
#: I due crate non sono un dettaglio di organizzazione: i tetti della busta
#: stanno nella CLI, quelli su ragioni ed esempi nel core, perche' li' vivono
#: le strutture che li portano. Un gate che ne guardasse uno solo direbbe di
#: aver verificato nove numeri avendone visti sette.
MAPPATURA: dict[str, str] = {
    "categorie_per_sezione": "MAX_CATEGORIE",
    "byte_per_identificatore_di_categoria": "MAX_BYTE_ID_CATEGORIA",
    "ragioni_per_sezione": "MAX_FIDELITY_REASONS",
    "esempi_per_sezione": "MAX_LOSS_EXAMPLES",
    "byte_per_dettaglio_curato": "MAX_BYTE_DETTAGLIO",
    "sezioni_con_budget_proprio": "SEZIONI",
    "byte_per_sezione": "BYTE_PER_SEZIONE",
    "byte_per_la_struttura_aggregata": "BYTE_DELLA_STRUTTURA",
    "byte_totali": "MAX_BYTE_BUSTA",
    "ragioni_trattenute": "MAX_RAGIONI_TRATTENUTE",
    "esempi_trattenuti": "MAX_ESEMPI_TRATTENUTI",
}

#: Le costanti `usize` pubbliche di questi due file **sono** il budget: non ce
#: ne sono altre, ed e' cio' che rende il verso inverso una verifica e non una
#: approssimazione. L'ordine conta: `MAX_BYTE_BUSTA` e' scritta in funzione
#: delle precedenti, e si risolve solo se sono gia' note.
SORGENTI = (BUSTA, LOSS)

#: I campi che dicono **di che cosa** e' il manifesto. Non sono numeri e non
#: hanno una controparte nel codice: qui si pretende solo che dicano quello che
#: devono, perche' un manifesto che si dichiarasse `protocol_version: 1`
#: verrebbe letto come il contratto congelato, che promette altro.
IDENTITA = {
    "manifest_version": 1,
    "component": "plenora-IO-tools",
    "protocol_version": 2,
    "compatibility_scope": "cli_json_only",
}

CONST_USIZE = re.compile(r"^pub const ([A-Z][A-Z0-9_]*): usize = ([^;]+);", re.M)
SONDA = re.compile(r"#\[test\]\s*\n\s*fn ([a-z_][a-z0-9_]*)\s*\(")

# --- le tre clausole di comportamento --------------------------------------
#
# Gli undici numeri erano gia' confrontati col codice; queste tre cose no, ed
# erano **prosa**. La ratifica di `wire.loss-report` ha trovato quattro clausole
# di questo stesso manifesto che descrivevano un codice cambiato sotto di loro:
# l'invariante era `verified` perche' i numeri si confrontano e la prosa non la
# guarda nessuno.
#
# Un gate generale prosa-contro-comportamento non e' realistico. Queste tre si
# strutturano, e allora si confrontano come i numeri: l'ordine canonico di
# ragioni ed esempi, l'identita' su cui le respinte deduplicano, e le due fonti
# di `omesse_per_byte`.
#
# Il gate le **ricava dal codice**, e non confronta due copie del manifesto.
# Quello che sta scritto qui sotto e' *dove* guardare -- il tipo, il campo, il
# nome della funzione -- non che cosa aspettarsi di trovarci: se il posto sparisce
# il gate diventa rosso, e se cambia cio' che c'e' dentro diventa rosso il
# confronto col manifesto.

#: `clausola -> tipo Rust la cui chiave() compone l'ordine canonico`.
#:
#: Un meccanismo **solo**, e non due. `LossExample` derivava `Ord`, quindi il suo
#: ordine era quello di dichiarazione dei campi: un fatto vero ma di natura
#: diversa, che avrebbe obbligato questo gate a due letture e a dichiarare il
#: caso particolare. Una `chiave()` esplicita anche per gli esempi costa una
#: funzione e toglie il caso particolare: l'ordine si legge dove e' scritto.
ORDINE_CANONICO: dict[str, str] = {
    "ragioni": "FidelityReason",
    "esempi": "LossExample",
}

#: I tratti che i due tipi devono **delegare** a `chiave()`, e la forma
#: **esatta** con cui devono farlo: `(metodo, corpo ammesso)`, dove `{p}` e' il
#: nome del parametro, che i due tipi scrivono in modo diverso.
#:
#: La forma esatta, e non la presenza di `chiave()` da qualche parte nel blocco.
#: La prima stesura cercava la sottostringa, e passavano tre cose che l'ordine
#: canonico lo cambiano:
#:
#:   * `altro.chiave().cmp(&self.chiave())` -- l'ordine **inverso**, che taglia
#:     dalla parte opposta della sezione;
#:   * `self.chiave().cmp(&altro.chiave()).then(...)` -- un criterio in piu'
#:     dopo la chiave, cioe' un ordine che il manifesto non descrive;
#:   * una menzione di `chiave()` in un **commento**, con il corpo che confronta
#:     tutt'altro.
#:
#: Tutte e tre lasciavano il gate verde mentre l'affermazione che pretende di
#: verificare -- «l'ordine e' quello che `chiave()` compone» -- era falsa.
#:
#: `PartialOrd` sta qui accanto agli altri due e non e' un di piu': `<` e `>`
#: passano da li', e un `partial_cmp` che non fosse `Some(self.cmp(..))`
#: darebbe agli operatori una relazione diversa da quella con cui le collezioni
#: tagliano.
FORME_DELEGATE: dict[str, tuple[str, str]] = {
    "Ord": ("cmp", "self.chiave().cmp(&{p}.chiave())"),
    "PartialOrd": ("partial_cmp", "Some(self.cmp({p}))"),
    "PartialEq": ("eq", "self.chiave() == {p}.chiave()"),
}
DERIVE_VIETATI = frozenset({"Ord", "PartialOrd", "PartialEq", "Eq"})

#: `clausola -> (struttura, campo)` del `BTreeSet` che conserva le respinte.
INSIEMI_DELLE_RESPINTE: dict[str, tuple[str, str]] = {
    "ragioni": ("FidelityAssessment", "respinte"),
    "esempi": ("LossReport", "respinti"),
}

#: Come si legge un elemento di quei due insiemi. Un tipo non mappato e' un
#: errore e non un caso da ignorare: significa che l'identita' su cui le
#: respinte deduplicano e' cambiata, ed e' precisamente cio' che il manifesto
#: dichiara.
TIPI_DELLA_RESPINTA: dict[str, str] = {
    "FidelityReasonCode": "code",
    "Posizione": "posizione",
}

#: Le due fonti di `omesse_per_byte`, e il vocabolario con cui il manifesto le
#: nomina. Sono contate insieme perche' la decisione che inducono e' la stessa
#: -- la voce non c'e', e non e' una questione di cardinalita' -- ma restano
#: due, e il manifesto deve dirlo.
FONTE_DELLA_VOCE = "limite_della_voce"
FONTE_DELLA_SEZIONE = "budget_della_sezione"

#: I tetti in byte **per voce**: una partizione che ci si confronta e' il filtro
#: alla porta, cioe' la prima delle due fonti.
COSTANTI_DI_VOCE = ("MAX_BYTE_ID_CATEGORIA", "MAX_BYTE_DETTAGLIO")

#: La funzione che consuma il budget della sezione. E' l'altra fonte, e il
#: secondo valore che restituisce e' il numero di voci lasciate fuori.
FUNZIONE_DEL_BUDGET = "entro_il_budget"

#: Il contatore da cui i siti si riconoscono.
CONTATORE = "troncamento.omesse_per_byte"

IMPL = re.compile(
    r"^impl(?:<[^>]*>)?\s+(?:(?P<tratto>[\w:]+)\s+for\s+)?(?P<tipo>\w+)", re.M
)
FN_CHIAVE = re.compile(
    r"^(?P<rientro>[ ]*)fn chiave\(&self\)[^\n]*\{\n(?P<corpo>.*?)^(?P=rientro)\}",
    re.M | re.S,
)
CAMPO_DI_SELF = re.compile(r"self\.(\w+)")
DERIVE_DELLA_STRUTTURA = re.compile(
    r"#\[derive\(([^)]*)\)\]\s*(?:#\[[^\]]*\]\s*)*pub struct (\w+)"
)
ACCESSORE_DELLA_LUNGHEZZA = re.compile(
    r"pub fn (\w+)\(&self\) -> u64 \{\s*self\.(\w+)\.len\(\) as u64\s*\}"
)
LEGAME_A_DUE = re.compile(r"let \(\s*\w+\s*,\s*(\w+)\s*\)[^=]*=\s*(.*?);", re.S)
PAROLA = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")

#: Un'assegnazione, e non un confronto ne' un'assegnazione composta: serve a
#: riconoscere `(troncamento.omesse_per_byte,) = (nuova,)`, dove il contatore e'
#: scritto pur non essendo seguito da `=`.
ASSEGNAZIONE = re.compile(r"(?<![=!<>+\-*/%&|^])=(?!=)")

#: **Ogni** uso del contatore, non le sole assegnazioni con `=`. Lo spazio in
#: mezzo e' ammesso perche' `rustfmt` spezza la catena su piu' righe:
#: `troncamento\n    .omesse_per_byte\n    .saturating_add(..)`.
USO_DEL_CONTATORE = re.compile(r"troncamento\s*\.\s*omesse_per_byte")

#: Le scritture composte. Il censimento cercava `= `, quindi
#: `troncamento.omesse_per_byte += nuova_fonte;` non veniva censito affatto e il
#: gate continuava a dichiarare esattamente due fonti: una terza sarebbe entrata
#: nel codice senza che nulla lo dicesse. Le piu' lunghe vanno provate per
#: prime, se no `<<=` si legge come `<`.
SCRITTURE_COMPOSTE = ("<<=", ">>=", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=")

#: Le forme **complete** in cui il contatore compare senza essere scritto.
#:
#: Forme, e non singoli segni: un segno ammette tutto cio' che comincia con
#: quel segno, ed e' cosi' che il `.` ammetteva
#:
#:     troncamento.omesse_per_byte.clone_from(&nuova_fonte);
#:
#: -- una scrittura per **auto-borrow**, in cui `&mut` non compare affatto. Una
#: chiamata di metodo si ammette percio' per nome intero, non perche' comincia
#: con un punto: `.saturating_add(` e' la sola catena che il contatore
#: attraversa, e qualunque altro metodo e' rosso finche' qualcuno non lo scrive
#: qui.
#:
#: Le forme si confrontano su cio' che segue il contatore **con gli spazi
#: normalizzati**, quindi non portano spazi in testa. Le piu' lunghe per prime,
#: se no `>=` si legge come `>`.
#:
#: `,` e `)` **non** sono qui, e prima lo erano. Dentro una chiamata quei due
#: segni non dicono niente su chi li riceve: una macro prende i token e puo'
#: farne cio' che vuole, campo compreso --
#:
#:     scrivi!(troncamento.omesse_per_byte, nuova_fonte);
#:
#: -- che e' un'assegnazione scritta con una virgola. Un'occorrenza dentro una
#: chiamata si ammette percio' per **chiamata** e non per segno: vedi
#: `CHIAMATE_DI_SOLA_LETTURA`. Fuori da ogni chiamata restano queste forme, che
#: consumano il contatore come valore e non possono scriverlo.
LETTURE_AMMESSE = (
    ".saturating_add(",
    ">= ",
    "<= ",
    "== ",
    "!= ",
    "> ",
    "< ",
    ";",
)

#: Le chiamate che possono ricevere il contatore senza scriverlo.
#:
#: L'elenco e' chiuso, e vale per **qualunque** occorrenza dentro una chiamata:
#: `assert_eq!` e `assert_ne!` prendono i due lati per riferimento condiviso,
#: `assert!` valuta un'espressione. Una chiamata che non e' qui e' rossa anche
#: se innocua -- distinguere una macro che legge da una che scrive vuole
#: l'espansione, non i token, e la sola alternativa onesta e' fallire chiusi.
CHIAMATE_DI_SOLA_LETTURA = ("assert_eq!", "assert_ne!", "assert!")

#: Un contatore preso per riferimento mutabile e' una scrittura che avviene
#: altrove, e che questo modulo non puo' seguire.
PRESA_MUTABILE = ("&mut", "&")


def _corpo_della_struttura(testo: str, tipo: str) -> str | None:
    """Il corpo di `pub struct <tipo> { ... }`, o `None` se non c'e' piu'."""
    trovato = re.search(
        r"^pub struct " + re.escape(tipo) + r"\b[^\n]*\{\n(.*?)^\}",
        testo,
        re.M | re.S,
    )
    return trovato.group(1) if trovato else None


def _tipo_che_possiede(testo: str, posizione: int) -> tuple[str | None, str | None]:
    """`(tratto, tipo)` del blocco `impl` che contiene quella posizione.

    Si guarda all'indietro invece di bilanciare le graffe: in un sorgente Rust
    le graffe stanno anche dentro le stringhe -- `format!("{format}: ...")` --
    e un contatore ingenuo si perderebbe alla prima.
    """
    ultimo = None
    for trovato in IMPL.finditer(testo):
        if trovato.start() > posizione:
            break
        ultimo = trovato
    if ultimo is None:
        return None, None
    return ultimo.group("tratto"), ultimo.group("tipo")


def _corpo_dell_impl(testo: str, tratto: str, tipo: str) -> str | None:
    """Il testo di `impl <tratto> for <tipo>`, fino al blocco `impl` successivo."""
    for trovato in IMPL.finditer(testo):
        if trovato.group("tratto") != tratto or trovato.group("tipo") != tipo:
            continue
        successivo = IMPL.search(testo, trovato.end())
        return testo[trovato.start() : successivo.start() if successivo else len(testo)]
    return None


def _senza_commenti(testo: str) -> str:
    """Il testo con i commenti sostituiti da spazi, e le righe al loro posto.

    **Sostituiti**, non tolti: le espressioni regolari di questo modulo si
    ancorano al rientro e alle righe, e accorciare il testo sposterebbe cio'
    che segue.

    Va fatto **prima di cercare**, non prima di confrontare. La prima stesura
    toglieva i soli `//` e lo faceva sul corpo gia' estratto, quindi

        impl Ord for LossExample {
            /*
            fn cmp(&self, altro: &Self) -> Ordering {
                self.chiave().cmp(&altro.chiave())
            }
            */
            fn cmp(&self, altro: &Self) -> Ordering {
                self.category.cmp(&altro.category)
            }
        }

    lasciava il gate verde: l'espressione regolare trovava il **primo** `fn cmp`,
    che sta nel commento ed e' canonico, e il corpo vero non veniva mai
    guardato. Vale per ogni ricerca di questo modulo, non per la sola delega: un
    `chiave()`, un `BTreeSet` o un sito del contatore commentati mentirebbero
    allo stesso modo.

    Le stringhe si saltano, perche' un `//` dentro una stringa non apre un
    commento; i blocchi si annidano, come in Rust.
    """
    fuori: list[str] = []
    i, quanti = 0, len(testo)
    profondita = 0
    in_stringa = False
    while i < quanti:
        carattere = testo[i]
        if profondita:
            if testo.startswith("/*", i):
                profondita += 1
                fuori.append("  ")
                i += 2
                continue
            if testo.startswith("*/", i):
                profondita -= 1
                fuori.append("  ")
                i += 2
                continue
            fuori.append("\n" if carattere == "\n" else " ")
            i += 1
            continue
        if in_stringa:
            fuori.append(carattere)
            if carattere == "\\" and i + 1 < quanti:
                fuori.append(testo[i + 1])
                i += 2
                continue
            if carattere == '"':
                in_stringa = False
            i += 1
            continue
        if carattere == '"':
            in_stringa = True
            fuori.append(carattere)
            i += 1
            continue
        if testo.startswith("//", i):
            while i < quanti and testo[i] != "\n":
                fuori.append(" ")
                i += 1
            continue
        if testo.startswith("/*", i):
            profondita = 1
            fuori.append("  ")
            i += 2
            continue
        fuori.append(carattere)
        i += 1
    return "".join(fuori)


def _senza_stringhe(testo: str) -> str:
    """Il testo con il **contenuto** dei letterali di stringa fatto di spazi.

    La lunghezza si conserva, cosi' una posizione trovata qui vale anche sul
    testo vero.

    Serve dove un carattere decide una classificazione: un `=` dentro
    `"{caratteri} caratteri = {} byte"` non e' un'assegnazione, e un nome di
    variabile dentro una stringa non e' un uso di quella variabile. Cercarli sul
    testo grezzo faceva chiamare scrittura una sonda che legge il contatore, e
    avrebbe potuto far chiamare «fonte nota» una fonte nuova che nomina l'altra
    in un messaggio d'errore.
    """
    fuori: list[str] = []
    dentro = False
    i, quanti = 0, len(testo)
    while i < quanti:
        carattere = testo[i]
        if dentro:
            if carattere == "\\" and i + 1 < quanti:
                fuori.append("  ")
                i += 2
                continue
            if carattere == '"':
                dentro = False
                fuori.append(carattere)
            else:
                fuori.append(carattere if carattere == chr(10) else " ")
            i += 1
            continue
        fuori.append(carattere)
        if carattere == '"':
            dentro = True
        i += 1
    return "".join(fuori)


def _sorgente_leggibile(testo: str) -> str:
    """Il sorgente senza commenti e senza il **contenuto** delle stringhe.

    E' cio' su cui questo modulo cerca, e nient'altro. I due passaggi chiudono
    la stessa classe di difetto -- del testo che *sembra* codice e non lo e' --
    e vanno fatti **prima di cercare**, non prima di confrontare:

        impl Ord for LossExample {
            const AIUTO: &str = "
            fn cmp(&self, altro: &Self) -> Ordering {
                self.chiave().cmp(&altro.chiave())
            }
        ";
            fn cmp(&self, altro: &Self) -> Ordering {
                self.category.cmp(&altro.category)
            }
        }

    L'espressione regolare trovava il `fn cmp` **dentro la stringa**, che e'
    canonico, e il corpo vero non veniva mai guardato. Lo stesso vale per un
    `chiave()`, un `BTreeSet` o un sito del contatore scritti dentro una
    stringa: fabbricherebbero un fatto che il codice non ha, o -- nell'altro
    verso -- un rosso su una sonda che nomina il contatore in un messaggio.

    L'ordine conta: prima i commenti, poi le stringhe. `_senza_commenti` salta
    le stringhe, quindi un `//` dentro una stringa non apre un commento; e con
    i commenti gia' tolti, una virgoletta dentro un commento non apre una
    stringa.
    """
    return _senza_stringhe(_senza_commenti(testo))


def _chiamata_che_racchiude(testo: str, posizione: int) -> str | None:
    """Il nome della chiamata la cui parentesi aperta racchiude la posizione.

    `None` se li' non c'e' nessuna chiamata: l'occorrenza sta al primo livello
    dell'istruzione. Si guarda all'indietro bilanciando le parentesi, e ci si
    ferma alla fine dell'istruzione precedente o all'inizio di un blocco.
    """
    profondita = 0
    indice = posizione - 1
    while indice >= 0:
        carattere = testo[indice]
        if carattere == ")":
            profondita += 1
        elif carattere == "(":
            if profondita == 0:
                fine = indice - 1
                while fine >= 0 and testo[fine].isspace():
                    fine -= 1
                inizio = fine
                while inizio >= 0 and (testo[inizio].isalnum() or testo[inizio] in "_!"):
                    inizio -= 1
                nome = testo[inizio + 1 : fine + 1]
                return nome or None
            profondita -= 1
        elif carattere in ";{}" and profondita == 0:
            return None
        indice -= 1
    return None


def _normalizzato(testo: str) -> str:
    """Gli spazi non contano; tutto il resto si'."""
    return " ".join(testo.split())


def _delega_esatta(testo: str, tratto: str, tipo: str) -> str | None:
    """`None` se `impl <tratto> for <tipo>` delega a `chiave()` **nella forma**.

    Non «nomina `chiave()`»: e' *quel* corpo e nessun altro. Vedi
    `FORME_DELEGATE` per i tre modi in cui la ricerca di una sottostringa
    lasciava passare un ordine diverso da quello dichiarato.
    """
    metodo, atteso = FORME_DELEGATE[tratto]
    corpo = _corpo_dell_impl(testo, tratto, tipo)
    if corpo is None:
        return (
            f"`impl {tratto} for {tipo}` non esiste. Senza, l'ordine con cui la "
            f"sezione taglia non passa da `{tipo}::chiave()`, e i campi che quella "
            "funzione compone non sono l'ordine canonico: sono un'opinione."
        )
    trovato = re.search(
        r"^(?P<rientro>[ ]*)fn "
        + re.escape(metodo)
        + r"\(&self, (?P<parametro>\w+): &Self\)[^\n]*\{\n"
        r"(?P<corpo>.*?)^(?P=rientro)\}",
        corpo,
        re.M | re.S,
    )
    if trovato is None:
        return (
            f"`impl {tratto} for {tipo}`: `fn {metodo}(&self, ..: &Self)` non si "
            "trova nella forma attesa."
        )
    osservato = _normalizzato(trovato.group("corpo"))
    voluto = atteso.format(p=trovato.group("parametro"))
    if osservato != voluto:
        return (
            f"`impl {tratto} for {tipo}`: il corpo di `{metodo}` e' «{osservato}», "
            f"atteso esattamente «{voluto}». Un ordine invertito, un criterio in "
            "piu' dopo la chiave o una menzione in un commento passerebbero per "
            "delega senza esserlo, e l'ordine canonico dichiarato dal manifesto "
            "sarebbe un altro."
        )
    return None


def ordine_canonico_dal_codice(testo: str) -> tuple[dict[str, list[str]], list[str]]:
    """`{clausola: campi}` dalle `chiave()` dei due tipi, con le loro guardie.

    Non basta leggere la funzione: bisogna anche che sia lei a decidere. Un
    `derive(Ord)` rimesso sulla struttura riporterebbe l'ordine nell'ordine di
    dichiarazione dei campi, e `chiave()` diventerebbe una funzione vera che
    con il punto in cui la sezione taglia non ha piu' rapporto.
    """
    testo = _sorgente_leggibile(testo)
    errori: list[str] = []
    chiavi: dict[str, list[str]] = {}
    per_tipo: dict[str, list[list[str]]] = {}

    for trovato in FN_CHIAVE.finditer(testo):
        tratto, tipo = _tipo_che_possiede(testo, trovato.start())
        if tratto is not None or tipo is None:
            continue
        per_tipo.setdefault(tipo, []).append(
            CAMPO_DI_SELF.findall(trovato.group("corpo"))
        )

    derive = {tipo: campi for campi, tipo in DERIVE_DELLA_STRUTTURA.findall(testo)}

    for clausola, tipo in ORDINE_CANONICO.items():
        composizioni = per_tipo.get(tipo, [])
        if not composizioni:
            errori.append(
                f"`{tipo}::chiave()` non esiste piu': l'ordine canonico di "
                f"`{clausola}` non si legge da nessuna parte, e il manifesto lo "
                "dichiarerebbe da solo."
            )
            continue
        if len(composizioni) > 1:
            errori.append(
                f"`{tipo}` ha {len(composizioni)} funzioni `chiave()`: quale "
                "componga l'ordine canonico e' ambiguo, e un'ambiguita' qui e' un "
                "ordine indefinito."
            )
            continue
        campi = composizioni[0]
        if not campi:
            errori.append(f"`{tipo}::chiave()` non compone alcun campo di `self`")
            continue
        ripetuti = sorted({c for c in campi if campi.count(c) > 1})
        if ripetuti:
            errori.append(
                f"`{tipo}::chiave()` compone {ripetuti} piu' di una volta: un campo "
                "ripetuto non aggiunge un criterio all'ordine."
            )
            continue

        if tipo not in derive:
            errori.append(f"`pub struct {tipo}` non si trova piu'")
            continue
        derivati = {d.strip() for d in derive[tipo].split(",") if d.strip()}
        vietati = sorted(derivati & DERIVE_VIETATI)
        if vietati:
            errori.append(
                f"`{tipo}` deriva {vietati}. Con il derive l'ordine torna a essere "
                "quello di **dichiarazione dei campi**, cioe' un posto che il "
                f"manifesto non nomina, e `{tipo}::chiave()` smette di decidere il "
                "punto in cui la sezione taglia."
            )
            continue
        difetti = [
            messaggio
            for tratto in FORME_DELEGATE
            if (messaggio := _delega_esatta(testo, tratto, tipo)) is not None
        ]
        if difetti:
            errori.extend(difetti)
            continue
        chiavi[clausola] = campi

    return chiavi, errori


def identita_delle_respinte_dal_codice(
    testo: str,
) -> tuple[dict[str, list[str]], list[str]]:
    """`{clausola: campi}` dal tipo dell'elemento dei due `BTreeSet`.

    E' li' che l'identita' vive: una voce respinta non si conserva, e cio' su cui
    il conteggio deduplica e' esattamente cio' che l'insieme contiene.
    """
    testo = _sorgente_leggibile(testo)
    errori: list[str] = []
    identita: dict[str, list[str]] = {}

    for clausola, (struttura, campo) in INSIEMI_DELLE_RESPINTE.items():
        corpo = _corpo_della_struttura(testo, struttura)
        if corpo is None:
            errori.append(f"`pub struct {struttura}` non si trova piu'")
            continue
        trovato = re.search(
            r"^\s*" + re.escape(campo) + r": BTreeSet<(.+)>,$", corpo, re.M
        )
        if trovato is None:
            errori.append(
                f"`{struttura}.{campo}` non e' piu' un `BTreeSet`: l'identita' su "
                f"cui le respinte di `{clausola}` deduplicano non si legge piu'."
            )
            continue
        elemento = trovato.group(1).strip()
        if elemento.startswith("(") and elemento.endswith(")"):
            pezzi = [p.strip() for p in elemento[1:-1].split(",") if p.strip()]
        else:
            pezzi = [elemento]
        campi: list[str] = []
        for pezzo in pezzi:
            if pezzo not in TIPI_DELLA_RESPINTA:
                errori.append(
                    f"`{struttura}.{campo}` conserva un `{pezzo}`, che questo gate "
                    "non sa nominare. L'identita' delle respinte e' cambiata, ed e' "
                    "precisamente cio' che il manifesto dichiara."
                )
                campi = []
                break
            campi.append(TIPI_DELLA_RESPINTA[pezzo])
        if not campi:
            continue
        if len(set(campi)) != len(campi):
            errori.append(
                f"`{struttura}.{campo}` conserva due volte lo stesso campo: una "
                "chiave con un componente ripetuto non e' piu' stretta, e' solo "
                "illeggibile."
            )
            continue
        identita[clausola] = campi

    return identita, errori


def fonti_di_omesse_per_byte_dal_codice(
    busta: str, loss: str
) -> tuple[list[str], list[str]]:
    """Le fonti del contatore, dai **siti che lo incrementano**.

    Ogni sito deve ricadere in una delle due fonti, e in una sola. Un sito che
    non si classifica non e' un caso da ignorare: e' una terza fonte comparsa
    senza che il manifesto la dichiari, ed e' il modo normale in cui una clausola
    invecchia.
    """
    busta = _sorgente_leggibile(busta)
    loss = _sorgente_leggibile(loss)
    errori: list[str] = []

    insiemi = {campo for _, campo in INSIEMI_DELLE_RESPINTE.values()}
    accessori = {
        nome
        for nome, campo in ACCESSORE_DELLA_LUNGHEZZA.findall(loss)
        if campo in insiemi
    }
    if len(accessori) != len(insiemi):
        errori.append(
            f"gli accessori che contano le respinte sono {sorted(accessori)}, e gli "
            f"insiemi che le conservano sono {sorted(insiemi)}. Senza quel legame il "
            "filtro alla porta non e' piu' riconoscibile fra le fonti del contatore."
        )

    del_budget: set[str] = set()
    della_voce: set[str] = set()
    for nome, destra in LEGAME_A_DUE.findall(busta):
        if FUNZIONE_DEL_BUDGET in destra:
            del_budget.add(nome)
        elif ".partition(" in destra and any(c in destra for c in COSTANTI_DI_VOCE):
            della_voce.add(nome)
    if not del_budget:
        errori.append(
            f"nessun legame a `{FUNZIONE_DEL_BUDGET}`: il budget della sezione non "
            "si riconosce piu' fra le fonti del contatore."
        )
    if not della_voce:
        errori.append(
            "nessuna partizione sui tetti in byte della singola voce: il filtro alla "
            "porta non si riconosce piu' fra le fonti del contatore."
        )

    fonti: set[str] = set()
    siti, problemi = _siti_del_contatore(busta)
    errori.extend(problemi)
    if not siti:
        errori.append(
            f"nessun sito incrementa `{CONTATORE}`: il contatore non ha piu' fonti, e "
            "la clausola descriverebbe un comportamento che non esiste."
        )
    for indice, destra in enumerate(siti, 1):
        nomi = set(PAROLA.findall(destra))
        trovate = set()
        if nomi & del_budget:
            trovate.add(FONTE_DELLA_SEZIONE)
        if (nomi & della_voce) or (nomi & accessori):
            trovate.add(FONTE_DELLA_VOCE)
        if not trovate:
            errori.append(
                f"il sito {indice} di `{CONTATORE}` non ricade in nessuna delle due "
                f"fonti dichiarate: «{' '.join(destra.split())}». Una terza fonte "
                "comparsa senza che il manifesto la dichiari e' il modo normale in "
                "cui una clausola invecchia."
            )
        elif len(trovate) > 1:
            errori.append(
                f"il sito {indice} di `{CONTATORE}` ricade in entrambe le fonti: "
                f"«{' '.join(destra.split())}». Un sito ambiguo non dice da dove "
                "venga la voce omessa."
            )
        fonti |= trovate

    return sorted(fonti), errori


def _siti_del_contatore(busta: str) -> tuple[list[str], list[str]]:
    """Le parti destre di ogni **scrittura** del contatore, e cio' che non lo e'.

    Il censimento cercava `troncamento.omesse_per_byte = ...;` e nient'altro.
    Una fonte nuova scritta `troncamento.omesse_per_byte += nuova_fonte;` non
    veniva censita: il gate continuava a dichiarare esattamente due fonti, e la
    terza entrava nel codice senza che nulla lo dicesse. E' lo stesso falso
    verde che questa clausola esiste per chiudere, dal lato del gate.

    Qui si guarda **ogni** occorrenza del contatore e la si classifica:
    assegnazione, assegnazione composta, o lettura fra quelle ammesse. Tutto il
    resto e' un errore -- una presa per riferimento mutabile, o una forma che
    questo modulo non conosce -- perche' distinguere lettura da scrittura senza
    un parser non si puo' fare per esaustione, e la sola alternativa onesta a un
    censimento incompleto e' fallire chiusi.
    """
    destre: list[str] = []
    errori: list[str] = []
    for uso in USO_DEL_CONTATORE.finditer(busta):
        prima = busta[: uso.start()].rstrip()
        if any(prima.endswith(segno) for segno in PRESA_MUTABILE):
            errori.append(
                f"`{CONTATORE}` e' preso per riferimento: «{_estratto(busta, uso)}». "
                "La scrittura avviene altrove, e questo censimento non la puo' "
                "seguire: la fonte che ne uscirebbe non sarebbe dichiarata da "
                "nessuno."
            )
            continue

        coda = busta[uso.end() :]
        dopo = " ".join(coda.split())
        composta = next((s for s in SCRITTURE_COMPOSTE if dopo.startswith(s)), None)
        if composta is not None:
            resto = dopo[len(composta) :]
        elif dopo.startswith("=") and not dopo.startswith("=="):
            resto = dopo[1:]
        else:
            # Il contatore puo' essere scritto **senza** comparire a sinistra di
            # un `=` che lo segue immediatamente: `(troncamento.omesse_per_byte,)
            # = (nuova_fonte,);` e' un'assegnazione destrutturante, e la virgola
            # da sola la faceva passare per una lettura. Se nell'istruzione
            # compare un'assegnazione **dopo** l'occorrenza, allora il contatore
            # sta a sinistra di quella, ed e' scritto.
            istruzione = dopo[: dopo.find(";")] if ";" in dopo else dopo
            destrutturante = ASSEGNAZIONE.search(istruzione)
            if destrutturante is not None:
                resto = istruzione[destrutturante.end() :]
            else:
                # Dentro una chiamata l'ammissione e' della **chiamata**: i
                # segni che seguono il contatore non dicono niente su chi lo
                # riceve, e una macro puo' assegnarlo pur essendone separata da
                # una virgola.
                chiamata = _chiamata_che_racchiude(busta, uso.start())
                if chiamata is not None:
                    if chiamata in CHIAMATE_DI_SOLA_LETTURA:
                        continue
                    errori.append(
                        f"`{CONTATORE}` e' passato a `{chiamata}`, che non e' fra le "
                        f"chiamate di sola lettura {list(CHIAMATE_DI_SOLA_LETTURA)}: "
                        f"«{_estratto(busta, uso)}». Una macro riceve i token e puo' "
                        "assegnare cio' che le si passa, quindi da fuori una lettura "
                        "e una scrittura si somigliano."
                    )
                    continue
                if any(dopo.startswith(lettura) for lettura in LETTURE_AMMESSE):
                    continue
                errori.append(
                    f"uso di `{CONTATORE}` che questo gate non sa classificare: "
                    f"«{_estratto(busta, uso)}». Non e' ne' una scrittura nota ne' "
                    "una lettura fra quelle ammesse, e un uso non classificato "
                    "potrebbe essere una fonte che nessuno dichiara."
                )
                continue

        fine = resto.find(";")
        destre.append(resto[:fine] if fine >= 0 else resto)
    return destre, errori


def _estratto(testo: str, uso: re.Match[str]) -> str:
    """L'istruzione attorno a un uso, su una riga, per il messaggio d'errore."""
    inizio = max(testo.rfind(";", 0, uso.start()), testo.rfind("\n", 0, uso.start()))
    fine = testo.find(";", uso.end())
    pezzo = testo[inizio + 1 : fine if fine >= 0 else uso.end() + 40]
    return " ".join(pezzo.split())[:160]


def comportamento_dal_codice() -> dict[str, Any]:
    """Le tre clausole di comportamento, dai due sorgenti che le decidono."""
    try:
        testo_loss = LOSS.read_text(encoding="utf-8")
        testo_busta = BUSTA.read_text(encoding="utf-8")
    except OSError as errore:
        return {"errori": [f"sorgenti del comportamento illeggibili: {errore}"]}

    ordine, errori_ordine = ordine_canonico_dal_codice(testo_loss)
    identita, errori_identita = identita_delle_respinte_dal_codice(testo_loss)
    fonti, errori_fonti = fonti_di_omesse_per_byte_dal_codice(testo_busta, testo_loss)
    return {
        "ordine_canonico": ordine,
        "identita_delle_respinte": identita,
        "fonti_di_omesse_per_byte": fonti,
        "errori": errori_ordine + errori_identita + errori_fonti,
    }


def _elenco_di_campi(valore: Any) -> list[str] | None:
    """Un elenco di nomi non vuoti e senza ripetizioni, o `None`.

    Le ripetizioni non sono una svista da tollerare: `["code", "code"]` e
    `["code"]` descrivono la stessa identita' e si leggono come due, e in un
    ordine canonico un campo ripetuto non aggiunge un criterio.
    """
    if not isinstance(valore, list) or not valore:
        return None
    if not all(isinstance(nome, str) and nome for nome in valore):
        return None
    if len(set(valore)) != len(valore):
        return None
    return valore


def _confronta_fonti(dichiarato: Any, derivate: list[str]) -> list[str]:
    """Le fonti dichiarate contro quelle dei siti che incrementano il contatore.

    Un **insieme** e non una sequenza: le due fonti sono contate insieme e non
    hanno un ordine, quindi si confrontano ordinate. Cio' che conta e' che siano
    quelle, e che non ce ne sia una terza che nessuno dichiara.
    """
    if not derivate:
        return []
    if not isinstance(dichiarato, dict):
        return [
            "cli-protocol-v2: `troncamento.omesse_per_byte` assente o non un oggetto. "
            "Le due fonti restavano prosa, e una prosa non la confronta nessuno."
        ]
    errori: list[str] = []
    fonti = _elenco_di_campi(dichiarato.get("fonti"))
    if fonti is None:
        errori.append(
            "cli-protocol-v2: `troncamento.omesse_per_byte.fonti` non e' un elenco di "
            "nomi non vuoti e distinti."
        )
    elif sorted(fonti) != derivate:
        errori.append(
            f"cli-protocol-v2: `troncamento.omesse_per_byte.fonti` dichiara "
            f"{sorted(fonti)}, i siti che incrementano il contatore ne producono "
            f"{derivate}. Il manifesto descrive un comportamento che il codice non ha."
        )
    estranee = sorted(set(dichiarato) - {"fonti"} - PROSA_AMMESSA)
    if estranee:
        errori.append(
            f"cli-protocol-v2: `troncamento.omesse_per_byte` dichiara {estranee}, che "
            "il gate non ricava da nessuna parte. Una clausola che nessuno confronta e' "
            "prosa con la forma di un campo."
        )
    return errori


#: Le due chiavi di prosa ammesse accanto ai campi confrontati: dove il gate
#: guarda, e perche' la clausola esiste. Tutto il resto dentro quegli oggetti
#: sarebbe un campo che nessuno confronta, cioe' il difetto di partenza con una
#: forma nuova.
PROSA_AMMESSA = frozenset({"come_si_ricava", "nota"})

def _confronta_clausola(
    percorso: str, dichiarato: Any, derivato: dict[str, list[str]]
) -> list[str]:
    """Il campo del manifesto contro cio' che il codice compone.

    Se il codice non si e' lasciato leggere il confronto non si fa: il motivo e'
    gia' nell'elenco degli errori, e aggiungere «il manifesto dichiara cose che
    il gate non ricava» darebbe due diagnosi alla stessa causa.
    """
    if not derivato:
        return []
    if not isinstance(dichiarato, dict):
        return [
            f"cli-protocol-v2: `{percorso}` assente o non un oggetto. Una clausola di "
            "comportamento che resta prosa non la confronta nessuno, ed e' la "
            "famiglia di difetti che la ratifica del v2 ha trovato."
        ]
    errori: list[str] = []
    for clausola, campi in sorted(derivato.items()):
        valore = _elenco_di_campi(dichiarato.get(clausola))
        if valore is None:
            errori.append(
                f"cli-protocol-v2: `{percorso}.{clausola}` non e' un elenco di nomi "
                "non vuoti e distinti."
            )
            continue
        if valore != campi:
            errori.append(
                f"cli-protocol-v2: `{percorso}.{clausola}` dichiara {valore}, il "
                f"codice compone {campi}. Il manifesto descrive un comportamento che "
                "il codice non ha."
            )
    estranee = sorted(set(dichiarato) - set(derivato) - PROSA_AMMESSA)
    if estranee:
        errori.append(
            f"cli-protocol-v2: `{percorso}` dichiara {estranee}, che il gate non "
            "ricava da nessuna parte. Una clausola che nessuno confronta e' prosa con "
            "la forma di un campo."
        )
    return errori




def valore(espressione: str, note: dict[str, int]) -> int:
    """Il valore di una definizione, sui soli operatori che vi compaiono.

    `BYTE_PER_SEZIONE` e' `12 * 1024` e `MAX_BYTE_BUSTA` e'
    `SEZIONI * BYTE_PER_SEZIONE + BYTE_DELLA_STRUTTURA`: leggerne il testo non
    basta. `eval` sarebbe la scorciatoia, e comprerebbe tre righe al prezzo di
    eseguire qualunque cosa il sorgente contenga. Qui si valuta l'albero, e
    soltanto i nodi che servono davvero: un intero, un nome gia' noto, una
    somma, un prodotto. Tutto il resto e' un errore, non un caso da gestire.
    """

    def risolvi(nodo: ast.AST) -> int:
        if isinstance(nodo, ast.Constant) and isinstance(nodo.value, int):
            return nodo.value
        if isinstance(nodo, ast.Name):
            if nodo.id not in note:
                raise ValueError(f"costante non ancora nota: {nodo.id}")
            return note[nodo.id]
        if isinstance(nodo, ast.BinOp) and isinstance(nodo.op, (ast.Mult, ast.Add)):
            sinistra, destra = risolvi(nodo.left), risolvi(nodo.right)
            return sinistra * destra if isinstance(nodo.op, ast.Mult) else sinistra + destra
        raise ValueError(f"espressione non ammessa: {espressione.strip()}")

    return risolvi(ast.parse(espressione.strip(), mode="eval").body)


def contratto() -> dict:
    """Il manifesto del v2, cosi' come sta su disco."""
    return json.loads(CONTRATTO.read_text(encoding="utf-8"))


def registro_categorie() -> dict:
    """Il registro del vocabolario, che dichiara il proprio tetto in byte."""
    return json.loads(REGISTRO_CATEGORIE.read_text(encoding="utf-8"))


def costanti_dai_testi(sorgenti: list[tuple[str, str]]) -> dict[str, int]:
    """Le costanti del budget, nell'ordine in cui i sorgenti le definiscono.

    Una costante definita **due volte** e' un errore, non l'ultima che vince.
    La prima stesura faceva `note[nome] = ...`, cioe' sovrascriveva in silenzio:
    con `MAX_BYTE_DETTAGLIO` dichiarata sia in `busta.rs` sia in `loss.rs` il
    gate avrebbe confrontato il manifesto con **una sola** delle due e sarebbe
    stato verde mentre il codice ne applicava un'altra. E' precisamente il
    difetto che questo lotto toglie dal codice, e sarebbe rimasto nel gate che
    lo verifica.

    L'ordine conta e non e' un dettaglio: `MAX_BYTE_BUSTA` e' scritta in
    funzione delle precedenti e si risolve solo se sono gia' note.
    """
    note: dict[str, int] = {}
    duplicate: dict[str, list[str]] = {}
    provenienza: dict[str, str] = {}
    for nome_sorgente, testo in sorgenti:
        for nome, espressione in CONST_USIZE.findall(testo):
            if nome in note:
                duplicate.setdefault(nome, [provenienza[nome]]).append(nome_sorgente)
                continue
            note[nome] = valore(espressione, note)
            provenienza[nome] = nome_sorgente
    if duplicate:
        elenco = "; ".join(f"`{n}` in {sorted(set(d))}" for n, d in sorted(duplicate.items()))
        raise ValueError(
            f"costanti del budget definite piu' di una volta: {elenco}. "
            "Due definizioni non sono una ridondanza: il compilatore ne usa una per "
            "contesto e questo gate ne confronterebbe un'altra."
        )
    return note


def costanti() -> dict[str, int]:
    """Le costanti del budget, lette dai sorgenti che le dichiarano."""
    return costanti_dai_testi(
        [(str(s.relative_to(ROOT)), s.read_text(encoding="utf-8")) for s in SORGENTI]
    )


def sonde() -> set[str]:
    """Le sonde di `busta.rs`, prese dall'attributo e non dal nome.

    Cercare `fn qualcosa` prenderebbe anche gli aiutanti del modulo di prova --
    `rapporto_con` non e' una sonda -- e il contratto dovrebbe nominare
    funzioni che non provano niente.
    """
    return set(SONDA.findall(BUSTA.read_text(encoding="utf-8")))


def verifica(
    manifesto: dict | None = None,
    note: dict[str, int] | None = None,
    esistenti: set[str] | None = None,
    registro: dict | None = None,
    comportamento: dict[str, Any] | None = None,
) -> list[str]:
    """Gli errori trovati, in elenco. Vuoto significa verde.

    Gli argomenti esistono per le sonde: un gate verde sul repository sano dice
    che oggi e' verde, non che domani diventerebbe rosso, e ogni proprieta'
    affermata qui ha una sonda che la viola su un manifesto finto.
    """
    manifesto = contratto() if manifesto is None else manifesto
    errori: list[str] = []

    for campo, atteso in IDENTITA.items():
        if manifesto.get(campo) != atteso:
            errori.append(
                f"cli-protocol-v2: `{campo}` e' {manifesto.get(campo)!r} e non {atteso!r}."
            )
    if manifesto.get("status") == "frozen_for_1_0":
        errori.append(
            "cli-protocol-v2: `status` congelato come il v1. Il v2 e' in qualifica, e "
            "dichiararlo congelato prometterebbe una stabilita' che nessuno ha ratificato."
        )

    limiti = manifesto.get("limiti_della_diagnostica")
    if not isinstance(limiti, dict):
        errori.append("cli-protocol-v2: `limiti_della_diagnostica` assente o non un oggetto.")
        return errori

    try:
        note = costanti() if note is None else note
    except (OSError, ValueError, SyntaxError) as errore:
        errori.append(f"costanti del budget non determinate: {errore}")
        return errori

    for chiave, costante in MAPPATURA.items():
        if chiave not in limiti:
            errori.append(f"cli-protocol-v2: `{chiave}` non e' dichiarato nel manifesto.")
            continue
        if costante not in note:
            errori.append(
                f"`{costante}` non e' fra le costanti del budget, ma il manifesto "
                f"dichiara `{chiave}`: il contratto promette un tetto che nessuno applica."
            )
            continue
        if limiti[chiave] != note[costante]:
            errori.append(
                f"cli-protocol-v2: `{chiave}` dichiara {limiti[chiave]!r}, "
                f"`{costante}` vale {note[costante]}. Il contratto promette un numero "
                "che il codice non applica."
            )

    # Il payload dichiarato e' **derivato** dai limiti, non un numero scritto a
    # mano accanto a loro: se fosse indipendente sarebbe una quarta copia da
    # tenere allineata, e il modo in cui si disallinea non si vedrebbe. Qui si
    # ricalcola e si confronta.
    derivati = {
        "payload_stringhe_v2_trattenute_ragioni": (
            ("ragioni_trattenute", "byte_per_dettaglio_curato"),
            lambda a, b: a * b,
        ),
        "payload_stringhe_v2_trattenute_esempi": (
            (
                "esempi_trattenuti",
                "byte_per_identificatore_di_categoria",
                "byte_per_dettaglio_curato",
            ),
            lambda a, b, c: a * (b + c),
        ),
    }
    for chiave, (fattori, calcolo) in derivati.items():
        if any(f not in limiti for f in fattori):
            continue
        if chiave not in limiti:
            errori.append(
                f"cli-protocol-v2: `{chiave}` non e' dichiarato. Un payload che nessuno "
                "dichiara non e' una promessa."
            )
            continue
        atteso = calcolo(*(limiti[f] for f in fattori))
        if limiti[chiave] != atteso:
            errori.append(
                f"cli-protocol-v2: `{chiave}` dichiara {limiti[chiave]!r}, e dai limiti "
                f"si ricava {atteso}. Il payload si **deriva** dai tetti: dichiararne uno "
                "diverso sarebbe una quarta copia da tenere allineata a mano."
            )

    non_dichiarate = sorted(set(note) - set(MAPPATURA.values()))
    if non_dichiarate:
        errori.append(
            f"costanti del budget che il manifesto non dichiara: {non_dichiarate}. "
            "Un tetto che vive solo nel codice non e' una promessa: chi legge la busta "
            "non ha modo di conoscerlo, e lo scopre quando lo colpisce."
        )

    # Il registro delle categorie dichiara lo stesso tetto, e `check_categorie
    # _di_perdita.py` ci confronta la propria costante. Legandolo qui alla
    # costante Rust, tutte e tre le copie risalgono a **una** autorita': quel
    # gate resta pinnato per transitivita' senza dover leggere Rust anche lui.
    try:
        registro = registro_categorie() if registro is None else registro
    except (OSError, ValueError) as errore:
        errori.append(f"registro delle categorie illeggibile: {errore}")
        registro = None
    if registro is not None:
        dichiarato = registro.get("limite_di_lunghezza_byte")
        atteso = note.get("MAX_BYTE_ID_CATEGORIA")
        if dichiarato != atteso:
            errori.append(
                f"il registro delle categorie dichiara un tetto di {dichiarato!r} byte e "
                f"`MAX_BYTE_ID_CATEGORIA` ne vale {atteso}. Lo stesso identificatore sarebbe "
                "limitato in un posto e non nell'altro."
            )

    # Le tre clausole di comportamento. Il comportamento si **ricava** dai due
    # sorgenti che lo decidono; qui si confronta soltanto, come per i numeri.
    comportamento = comportamento_dal_codice() if comportamento is None else comportamento
    errori.extend(comportamento.get("errori", []))
    determinismo = manifesto.get("determinismo")
    troncamento = manifesto.get("troncamento")
    errori.extend(
        _confronta_clausola(
            "determinismo.ordine_canonico",
            determinismo.get("ordine_canonico") if isinstance(determinismo, dict) else None,
            comportamento.get("ordine_canonico") or {},
        )
    )
    errori.extend(
        _confronta_clausola(
            "troncamento.identita_delle_respinte",
            troncamento.get("identita_delle_respinte")
            if isinstance(troncamento, dict)
            else None,
            comportamento.get("identita_delle_respinte") or {},
        )
    )
    errori.extend(
        _confronta_fonti(
            troncamento.get("omesse_per_byte") if isinstance(troncamento, dict) else None,
            comportamento.get("fonti_di_omesse_per_byte") or [],
        )
    )

    dichiarate = manifesto.get("sonde_che_lo_provano")
    if not isinstance(dichiarate, list) or not all(isinstance(s, str) for s in dichiarate):
        errori.append("cli-protocol-v2: `sonde_che_lo_provano` assente o non un elenco di nomi.")
        return errori
    if len(dichiarate) != len(set(dichiarate)):
        errori.append("cli-protocol-v2: `sonde_che_lo_provano` nomina due volte la stessa sonda.")

    try:
        esistenti = sonde() if esistenti is None else esistenti
    except OSError as errore:
        errori.append(f"sonde illeggibili: {errore}")
        return errori

    promesse = sorted(set(dichiarate) - esistenti)
    if promesse:
        errori.append(
            f"sonde dichiarate dal contratto e inesistenti: {promesse}. Una prova "
            "promessa e mai eseguita vale meno di una non promessa: la prima si legge "
            "come verificata."
        )
    # Le sonde della redazione stanno in `driver.rs`, dove i quattro siti
    # redatti vivono, e li' l'esaustivita' non si puo' pretendere: quel file ha
    # decine di sonde che col protocollo non c'entrano. Qui il verso e' **uno
    # solo** -- ogni nome dichiarato deve esistere -- ed e' dichiarato tale,
    # perche' un gate che promettesse i due versi su un perimetro che non
    # delimita direbbe piu' di quanto guarda.
    della_redazione = manifesto.get("sonde_della_redazione")
    if not isinstance(della_redazione, list) or not all(
        isinstance(s, str) for s in della_redazione
    ):
        errori.append(
            "cli-protocol-v2: `sonde_della_redazione` assente o non un elenco di nomi."
        )
    else:
        try:
            nel_driver = set(SONDA.findall(DRIVER.read_text(encoding="utf-8")))
        except OSError as errore:
            errori.append(f"sonde della redazione illeggibili: {errore}")
            nel_driver = None
        if nel_driver is not None:
            assenti = sorted(set(della_redazione) - nel_driver)
            if assenti:
                errori.append(
                    f"sonde della redazione dichiarate e inesistenti: {assenti}. "
                    "Sono le prove che i nomi presi dal file restano nel v1 e spariscono "
                    "dal v2: promesse e assenti, la redazione si leggerebbe come verificata."
                )

    mute = sorted(esistenti - set(dichiarate))
    if mute:
        errori.append(
            f"sonde di `busta.rs` che il contratto non nomina: {mute}. Una sonda che "
            "nessuno nomina si puo' cancellare senza che il checkpoint se ne accorga."
        )
    return errori


def main() -> int:
    errori = verifica()
    if errori:
        for errore in errori:
            print(errore, file=sys.stderr)
        print(
            "\nIl manifesto del protocollo v2 e il codice devono dire lo stesso numero. "
            "Due verita' divergono, e divergono in silenzio.",
            file=sys.stderr,
        )
        return 1
    manifesto = contratto()
    print(
        f"protocollo v2 verificato: {len(MAPPATURA)} limiti dichiarati dal manifesto e "
        f"applicati dal codice, nessuna costante del budget taciuta; "
        f"tre clausole di comportamento ricavate dal codice e confrontate -- ordine "
        f"canonico di ragioni ed esempi, identita' delle respinte, fonti di "
        f"`omesse_per_byte`; "
        f"{len(manifesto['sonde_che_lo_provano'])} sonde nominate dal contratto, "
        f"tutte presenti e nessuna in piu'; "
        f"{len(manifesto['sonde_della_redazione'])} sonde della redazione, tutte presenti."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
