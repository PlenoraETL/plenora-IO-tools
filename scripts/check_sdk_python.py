#!/usr/bin/env python3
"""I modelli e gli errori dell'SDK sono quelli che il protocollo v2 dichiara.

# Che cosa protegge

L'SDK espone dataclass tipizzate per le buste della CLI, e una gerarchia di
eccezioni per le categorie d'errore. Sono una **seconda scrittura** di cose che
il contratto gia' dice, e due scritture della stessa cosa divergono.

Le divergenze non sono simmetriche.

* un campo che il protocollo dichiara e il modello non ha e' un pezzo di busta
  che l'SDK **butta via in silenzio**. Chi lo usa non sa che esiste, e per
  saperlo deve leggere il JSON grezzo -- cioe' fare a mano il lavoro per cui ha
  installato l'SDK;
* un campo che il modello ha e il protocollo non dichiara e' un campo
  **inventato**: `from_json` lo pretende, e il primo binario che non lo manda
  fa fallire l'SDK su una busta perfettamente valida.

Lo stesso vale per le categorie: una categoria del contratto senza la propria
classe costringe chi la incontra a leggere `envelope.category` a mano, e una
classe senza categoria e' un `except` che non scattera' mai.

# Che cosa non pretende

Che l'SDK modelli **tutto**. `write_capabilities` e `format_options` restano
dizionari grezzi, e non e' una dimenticanza: servono a `convert`, che l'SDK non
copre ancora, e modellarli adesso li lascerebbe scritti e non esercitati -- la
promessa non onorata che il gate delle buste esiste per impedire. Il gate lo sa
perche' e' **dichiarato qui**, con la sua ragione: una sottostruttura che
sparisse da questo elenco tornerebbe a essere pretesa.

# Perche' legge il sorgente e non importa i moduli

Importare l'SDK vorrebbe dire eseguirlo, e un gate che esegue cio' che verifica
gli concede di prepararsi. Qui si legge l'AST: le tuple `OBBLIGATORI` e la mappa
delle categorie sono letterali, e un letterale si legge senza chiedere il
permesso a nessuno.
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
SDK = ROOT / "sdk" / "python" / "src" / "plenora_io"
MODELLI = SDK / "models.py"
TETTI = SDK / "limits.py"
ERRORI = SDK / "errors.py"
CATEGORIE_RUST = ROOT / "crates" / "plenora-io-model" / "src" / "error.rs"
CLI = ROOT / "crates" / "plenora-io-cli" / "src" / "main.rs"

#: Le opzioni della CLI che **non** sono tetti: hanno un parametro proprio nei
#: metodi del client, o sono comandi. Dichiarate qui perche' un elenco che si
#: allunga da solo sarebbe il modo in cui «non e' un tetto» diventa «me ne sono
#: dimenticato».
NON_SONO_TETTI = frozenset(
    {
        "--assume-crs",
        "--layer",
        "--limit",
        "--in-opt",
        "--out-opt",
        "--opt",
        "--durable",
        "--version",
    }
)

#: Dove ciascun modello va confrontato: la busta e il prefisso nella sua
#: struttura. Il livello e' quello **immediato**: la profondita' la governano i
#: modelli annidati, che compaiono qui con il proprio prefisso.
POSTI: dict[str, tuple[str, str]] = {
    "Catalog": ("catalog", ""),
    "Driver": ("catalog", ".drivers[]"),
    "FormatDescriptor": ("inspect", ".format"),
    "Inspect": ("inspect", ""),
    "Layer": ("inspect", ".layers[]"),
    "Field": ("inspect", ".layers[].fields[]"),
    "Geometry": ("inspect", ".layers[].geometry"),
    "CrsResolution": ("inspect", ".layers[].geometry.crs_resolution"),
    "Layers": ("layers", ""),
    "LayerSummary": ("layers", ".layers[]"),
    "Validation": ("read", ""),
    "Fidelity": ("layers", ".fidelity"),
    "Omissions": ("layers", ".fidelity.omesse"),
    # Le ragioni si confrontano con quelle di `convert`, non con quelle di
    # `layers`: sono le sole che la matrice del gate delle buste raggiunge con
    # gli indici opzionali, e un confronto altrove direbbe che `field_index`
    # non esiste. Il tipo e' lo stesso -- e' il caso che cambia.
    "FidelityReason": ("convert", ".conversion_fidelity.reasons[]"),
}

#: I modelli che dichiarano campi **propri** oltre a quelli ereditati.
#: `Driver` e' un `FormatDescriptor` piu' i due campi che il catalogo aggiunge,
#: e il gate somma le due tuple prima di confrontare.
EREDITA: dict[str, str] = {"Driver": "FormatDescriptor"}

#: Le sottostrutture che l'SDK lascia grezze, e perche'.
GREZZE: dict[str, str] = {
    ".drivers[].write_capabilities": (
        "undici sottostrutture e un vocabolario chiuso per ciascuna, che "
        "servono a `convert`. Modellarle in un ciclo che non copre `convert` "
        "le lascerebbe scritte e non esercitate."
    ),
    ".drivers[].format_options": (
        "il valore di un'opzione e' una somma -- testo, carattere, enum, "
        "intervallo di interi -- e la sua forma dipende dall'opzione. La "
        "modellazione ha senso quando qualcuno dovra' **passarle** per nome, "
        "cioe' con i comandi che le prendono tutte."
    ),
}


def _tuple_di_stringhe(nodo: ast.AST) -> list[str] | None:
    if not isinstance(nodo, (ast.Tuple, ast.List)):
        return None
    fuori: list[str] = []
    for elemento in nodo.elts:
        if not isinstance(elemento, ast.Constant) or not isinstance(elemento.value, str):
            return None
        fuori.append(elemento.value)
    return fuori


def tuple_dichiarate(percorso: Path, nome: str) -> dict[str, list[str]]:
    """`<Classe>.<nome>` per ogni classe che ne dichiara una.

    Fallisce chiuso su una tupla che non sia di stringhe letterali: se un domani
    quell'elenco venisse costruito a runtime, questo gate leggerebbe una lista
    vuota e direbbe verde su un confronto che non ha fatto.
    """
    albero = ast.parse(percorso.read_text(encoding="utf-8"), filename=str(percorso))
    fuori: dict[str, list[str]] = {}
    for nodo in ast.walk(albero):
        if not isinstance(nodo, ast.ClassDef):
            continue
        for corpo in nodo.body:
            if not isinstance(corpo, ast.Assign):
                continue
            if nome not in [t.id for t in corpo.targets if isinstance(t, ast.Name)]:
                continue
            campi = _tuple_di_stringhe(corpo.value)
            if campi is None:
                raise SystemExit(
                    f"{nodo.name}.{nome} non e' una tupla di stringhe letterali: "
                    "il gate la leggerebbe vuota e direbbe verde su un "
                    "confronto che non ha fatto."
                )
            fuori[nodo.name] = campi
    return fuori


def campi_del_protocollo(struttura: dict[str, Any], prefisso: str) -> dict[str, bool]:
    """I campi immediatamente sotto `prefisso`, con la loro obbligatorieta'."""
    fuori: dict[str, bool] = {}
    for percorso, voce in struttura.items():
        if not percorso.startswith(prefisso + "."):
            continue
        resto = percorso[len(prefisso) + 1 :]
        if "." in resto or "[]" in resto or "{}" in resto:
            continue
        fuori[resto] = bool(voce["sempre"])
    return fuori


def confronta(
    nome: str,
    obbligatori: list[str],
    opzionali: list[str],
    attesi: dict[str, bool],
    dove: str,
) -> list[str]:
    problemi: list[str] = []
    dichiarati = obbligatori + opzionali
    if len(dichiarati) != len(set(dichiarati)):
        ripetuti = sorted({c for c in dichiarati if dichiarati.count(c) > 1})
        problemi.append(f"{nome}: gli elenchi ripetono {ripetuti}")

    sempre = {campo for campo, s in attesi.items() if s}
    a_volte = {campo for campo, s in attesi.items() if not s}

    for campo in sorted(sempre - set(obbligatori)):
        problemi.append(
            f"{nome}: il protocollo dichiara «{dove}.{campo}» sempre presente e "
            "il modello non lo pretende: l'SDK lo butta via in silenzio, e chi "
            "lo usa deve leggere il JSON grezzo per saperlo."
        )
    for campo in sorted(a_volte - set(opzionali)):
        if campo in obbligatori:
            problemi.append(
                f"{nome}: «{campo}» e' dichiarato opzionale dal protocollo e "
                "obbligatorio dal modello. Un campo che a volte non c'e' non si "
                "puo' pretendere."
            )
        else:
            problemi.append(
                f"{nome}: il protocollo dichiara «{dove}.{campo}» come "
                "opzionale e il modello non lo espone affatto."
            )
    for campo in sorted(set(dichiarati) - set(attesi)):
        problemi.append(
            f"{nome}: il modello nomina «{campo}» e il protocollo non lo "
            "dichiara. Il primo binario che non lo manda fa fallire l'SDK su "
            "una busta valida."
        )
    return problemi


def categorie_del_contratto() -> list[str]:
    """Le varianti di `ErrorCategory`, in snake_case come arrivano sul wire."""
    sorgente = CATEGORIE_RUST.read_text(encoding="utf-8")
    corpo = re.search(r"pub enum ErrorCategory\s*\{(.*?)\n\}", sorgente, re.S)
    if corpo is None:
        raise SystemExit("`ErrorCategory` non si trova: il gate non sa che cosa confrontare.")
    fuori = []
    for riga in corpo.group(1).splitlines():
        riga = riga.strip()
        if not riga or riga.startswith("//") or riga.startswith("#"):
            continue
        variante = riga.split("(")[0].split("{")[0].strip().rstrip(",")
        if not variante or not variante[0].isupper():
            continue
        fuori.append(re.sub(r"(?<!^)(?=[A-Z])", "_", variante).lower())
    return fuori


def categorie_dell_sdk() -> dict[str, str]:
    """La mappa `CATEGORIE` dell'SDK: categoria -> nome della classe."""
    albero = ast.parse(ERRORI.read_text(encoding="utf-8"), filename=str(ERRORI))
    for nodo in ast.walk(albero):
        if not isinstance(nodo, (ast.Assign, ast.AnnAssign)):
            continue
        bersagli = (
            [nodo.target] if isinstance(nodo, ast.AnnAssign) else list(nodo.targets)
        )
        if "CATEGORIE" not in [t.id for t in bersagli if isinstance(t, ast.Name)]:
            continue
        if not isinstance(nodo.value, ast.Dict):
            raise SystemExit("`CATEGORIE` non e' un dizionario letterale.")
        fuori: dict[str, str] = {}
        for chiave, valore in zip(nodo.value.keys, nodo.value.values):
            if not isinstance(chiave, ast.Constant) or not isinstance(valore, ast.Name):
                raise SystemExit(
                    "`CATEGORIE` deve associare stringhe letterali a nomi di "
                    "classe: il gate la legge senza eseguirla."
                )
            fuori[chiave.value] = valore.id
        return fuori
    raise SystemExit("`CATEGORIE` non si trova in errors.py.")



def opzioni_della_cli() -> set[str]:
    """Le opzioni che `OPZIONI_AMMESSE` elenca, dalla CLI stessa."""
    sorgente = CLI.read_text(encoding="utf-8")
    riga = re.search(r'const OPZIONI_AMMESSE: &str = "(.*?)";', sorgente, re.S)
    if riga is None:
        raise SystemExit(
            "`OPZIONI_AMMESSE` non si trova nella CLI: il gate non sa che cosa "
            "confrontare, e dirsi verde su un confronto non fatto e' peggio che "
            "essere rosso."
        )
    return {pezzo.strip() for pezzo in riga.group(1).split(",") if pezzo.strip()}


def tetti_dell_sdk() -> set[str]:
    """I nomi delle opzioni che `Limits` sa produrre, dai suoi campi.

    Si leggono dall'AST, come tutto il resto: i campi di una dataclass sono
    annotazioni, e un'annotazione si legge senza eseguire il modulo.
    """
    albero = ast.parse(TETTI.read_text(encoding="utf-8"), filename=str(TETTI))
    for nodo in ast.walk(albero):
        if not isinstance(nodo, ast.ClassDef) or nodo.name != "Limits":
            continue
        durata = None
        campi = []
        for corpo in nodo.body:
            if isinstance(corpo, ast.AnnAssign) and isinstance(corpo.target, ast.Name):
                campi.append(corpo.target.id)
            elif isinstance(corpo, ast.Assign):
                nomi = [t.id for t in corpo.targets if isinstance(t, ast.Name)]
                if "DURATA" in nomi and isinstance(corpo.value, ast.Constant):
                    durata = corpo.value.value
        if durata is None:
            raise SystemExit("`Limits.DURATA` non e' una costante letterale.")
        return {
            "--deadline-ms" if campo == durata else f"--{campo.replace('_', '-')}"
            for campo in campi
        }
    raise SystemExit("`Limits` non si trova in limits.py.")


def main() -> int:
    manifesto = json.loads(CONTRATTO.read_text(encoding="utf-8"))
    obbligatori = tuple_dichiarate(MODELLI, "OBBLIGATORI")
    opzionali = tuple_dichiarate(MODELLI, "OPZIONALI")
    propri = tuple_dichiarate(MODELLI, "PROPRI")
    problemi: list[str] = []

    for nome, (busta, prefisso) in POSTI.items():
        if nome not in obbligatori and nome not in propri:
            problemi.append(f"il modello «{nome}» non dichiara `OBBLIGATORI`.")
            continue
        campi = list(obbligatori.get(nome, []))
        base = EREDITA.get(nome)
        if base is not None:
            campi = list(obbligatori.get(base, [])) + list(propri.get(nome, []))
        struttura = manifesto["envelopes"][busta]["struttura"]
        problemi.extend(
            confronta(
                nome,
                campi,
                list(opzionali.get(nome, [])),
                campi_del_protocollo(struttura, prefisso),
                prefisso or busta,
            )
        )

    # La busta di bootstrap ha uno schema **chiuso**, e il modello lo rifiuta
    # anche per eccesso: il confronto qui e' con `schema_esatto`.
    sorgente = MODELLI.read_text(encoding="utf-8")
    for percorso in manifesto["busta_di_bootstrap"]["schema_esatto"]:
        campo = percorso.lstrip(".")
        if f'"{campo}"' not in sorgente:
            problemi.append(
                f"la busta di bootstrap dichiara «{campo}» e `models.py` non lo "
                "nomina."
            )

    catalogo = manifesto["envelopes"]["catalog"]["struttura"]
    for percorso in GREZZE:
        if percorso not in catalogo:
            problemi.append(
                f"«{percorso}» e' dichiarato grezzo e il protocollo non lo "
                "contiene piu': l'esenzione non ha piu' un oggetto."
            )

    # --- i tetti, contro le opzioni che la CLI ammette ---------------------
    #
    # Un tetto che l'SDK offre e la CLI non conosce e' un comando che fallira'
    # sull'uso; uno che la CLI accetta e l'SDK non offre e' un tetto
    # raggiungibile solo scrivendo la riga a mano, cioe' facendo a mano il
    # lavoro per cui l'SDK esiste.
    ammesse = opzioni_della_cli()
    offerte = tetti_dell_sdk()
    for opzione in sorted(offerte - ammesse):
        problemi.append(
            f"l'SDK offre il tetto «{opzione}», che la CLI non ammette: chi lo "
            "passa ottiene un errore d'uso."
        )
    for opzione in sorted(ammesse - offerte - NON_SONO_TETTI):
        problemi.append(
            f"la CLI ammette «{opzione}» e l'SDK non lo offre: e' raggiungibile "
            "solo scrivendo la riga a mano. Se non e' un tetto, va dichiarato "
            "in `NON_SONO_TETTI`."
        )

    # --- le categorie d'errore --------------------------------------------
    attese = set(categorie_del_contratto())
    coperte = categorie_dell_sdk()
    for categoria in sorted(attese - set(coperte)):
        problemi.append(
            f"la categoria «{categoria}» non ha una classe: chi la incontra "
            "deve leggere `envelope.category` a mano, che e' esattamente cio' "
            "che questa gerarchia esiste per evitare."
        )
    for categoria in sorted(set(coperte) - attese):
        problemi.append(
            f"l'SDK mappa la categoria «{categoria}», che il contratto non "
            f"dichiara: `except {coperte[categoria]}` non scattera' mai."
        )

    if problemi:
        for problema in problemi:
            print(problema, file=sys.stderr)
        print(
            f"\n{len(problemi)} divergenze fra l'SDK e il protocollo v2.",
            file=sys.stderr,
        )
        return 1

    campi = sum(len(v) for v in obbligatori.values()) + sum(
        len(v) for v in opzionali.values()
    )
    print(
        f"SDK verificato: {len(POSTI)} modelli e {campi} campi confrontati col "
        f"protocollo v2, {len(coperte)} categorie d'errore coperte una a una, "
        f"{len(GREZZE)} sottostrutture dichiarate grezze, "
        f"{len(offerte)} tetti che la CLI ammette."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
