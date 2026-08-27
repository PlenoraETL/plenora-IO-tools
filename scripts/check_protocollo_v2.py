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
) -> list[str]:
    """Gli errori trovati, in elenco. Vuoto significa verde.

    I tre argomenti esistono per le sonde: un gate verde sul repository sano
    dice che oggi e' verde, non che domani diventerebbe rosso, e ogni
    proprieta' affermata qui ha una sonda che la viola su un manifesto finto.
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
    print(
        f"protocollo v2 verificato: {len(MAPPATURA)} limiti dichiarati dal manifesto e "
        f"applicati dal codice, nessuna costante del budget taciuta; "
        f"{len(contratto()['sonde_che_lo_provano'])} sonde nominate dal contratto, "
        f"tutte presenti e nessuna in piu'; "
        f"{len(contratto()['sonde_della_redazione'])} sonde della redazione, tutte presenti."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
