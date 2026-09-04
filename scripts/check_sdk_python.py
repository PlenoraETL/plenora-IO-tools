#!/usr/bin/env python3
"""I modelli dell'SDK Python sono quelli che il protocollo v2 dichiara.

# Che cosa protegge

L'SDK espone dataclass tipizzate per le buste della CLI. Sono una **seconda
scrittura** dello stesso contratto: i campi stanno in `release/cli-protocol-v2.json`
e stanno di nuovo in `sdk/python/src/plenora_io/models.py`, e due scritture
della stessa cosa divergono.

Le due divergenze non sono simmetriche.

* un campo che il protocollo dichiara e il modello non ha e' un pezzo di busta
  che l'SDK **butta via in silenzio**. Chi lo usa non sa che esiste, e per
  saperlo deve leggere il JSON grezzo -- cioe' fare a mano il lavoro per cui ha
  installato l'SDK;
* un campo che il modello ha e il protocollo non dichiara e' un campo
  **inventato**: `from_json` lo pretende, e il primo binario che non lo manda
  fa fallire l'SDK su una busta perfettamente valida.

# Che cosa non pretende

Che l'SDK modelli **tutto**. `write_capabilities` e `format_options` restano
dizionari grezzi, e non e' una dimenticanza: servono a `convert`, che il primo
ciclo dell'SDK non copre, e modellarli adesso li lascerebbe scritti e non
esercitati -- la promessa non onorata che il gate delle buste esiste per
impedire. Il gate lo sa perche' e' **dichiarato qui**, con la sua ragione: una
sottostruttura che sparisse da questo elenco tornerebbe a essere pretesa.

# Perche' legge il sorgente e non importa il modulo

Importare l'SDK vorrebbe dire eseguirlo, e un gate che esegue cio' che verifica
gli concede di prepararsi. Qui si legge l'AST: le tuple `OBBLIGATORI` sono
letterali, e un letterale si legge senza chiedere il permesso a nessuno.
"""

from __future__ import annotations

import ast
import json
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
CONTRATTO = ROOT / "release" / "cli-protocol-v2.json"
MODELLI = ROOT / "sdk" / "python" / "src" / "plenora_io" / "models.py"

#: Le sottostrutture che l'SDK lascia grezze in questo ciclo, e perche'.
#:
#: Il gate non pretende che siano modellate, e pretende che siano **queste**: un
#: elenco che si allunga da solo sarebbe il modo in cui «non modellato» diventa
#: «dimenticato».
GREZZE: dict[str, str] = {
    ".drivers[].write_capabilities": (
        "undici sottostrutture e un vocabolario chiuso per ciascuna, che "
        "servono a `convert`. Modellarle in un ciclo che non copre `convert` "
        "le lascerebbe scritte e non esercitate."
    ),
    ".drivers[].format_options": (
        "il valore di un'opzione e' una somma -- testo, carattere, enum, "
        "intervallo di interi -- e la sua forma dipende dall'opzione. La "
        "modellazione ha senso quando qualcuno dovra' **passarle**, cioe' con "
        "i comandi che prendono `--in-opt`."
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


def obbligatori_dichiarati() -> dict[str, list[str]]:
    """`<Classe>.OBBLIGATORI` per ogni dataclass che ne dichiara una.

    Fallisce chiuso su una tupla che non sia di stringhe letterali: se un
    domani quell'elenco venisse costruito a runtime, questo gate leggerebbe una
    lista vuota e direbbe verde su un confronto che non ha fatto.
    """
    albero = ast.parse(MODELLI.read_text(encoding="utf-8"), filename=str(MODELLI))
    fuori: dict[str, list[str]] = {}
    for nodo in ast.walk(albero):
        if not isinstance(nodo, ast.ClassDef):
            continue
        for corpo in nodo.body:
            if not isinstance(corpo, ast.Assign):
                continue
            nomi = [t.id for t in corpo.targets if isinstance(t, ast.Name)]
            if "OBBLIGATORI" not in nomi:
                continue
            campi = _tuple_di_stringhe(corpo.value)
            if campi is None:
                raise SystemExit(
                    f"{nodo.name}.OBBLIGATORI non e' una tupla di stringhe "
                    "letterali: il gate la leggerebbe vuota e direbbe verde su "
                    "un confronto che non ha fatto."
                )
            fuori[nodo.name] = campi
    return fuori


def campi_del_protocollo(struttura: dict[str, Any], prefisso: str) -> dict[str, bool]:
    """I campi immediatamente sotto `prefisso`, con la loro obbligatorieta'.

    Solo il livello immediato: la profondita' la governano i modelli, e
    pretendere qui l'albero intero significherebbe pretendere che l'SDK modelli
    tutto -- che e' proprio cio' che `GREZZE` dichiara di non fare.
    """
    fuori: dict[str, bool] = {}
    for percorso, voce in struttura.items():
        if not percorso.startswith(prefisso + "."):
            continue
        resto = percorso[len(prefisso) + 1 :]
        if "." in resto or "[]" in resto or "{}" in resto:
            continue
        fuori[resto] = bool(voce["sempre"])
    return fuori


def confronta(nome: str, dichiarati: list[str], attesi: dict[str, bool], prefisso: str):
    problemi: list[str] = []
    obbligatori = {campo for campo, sempre in attesi.items() if sempre}
    opzionali = {campo for campo, sempre in attesi.items() if not sempre}
    dichiarati_insieme = set(dichiarati)

    if len(dichiarati) != len(dichiarati_insieme):
        ripetuti = sorted({c for c in dichiarati if dichiarati.count(c) > 1})
        problemi.append(f"{nome}.OBBLIGATORI ripete {ripetuti}")

    for campo in sorted(obbligatori - dichiarati_insieme):
        problemi.append(
            f"{nome}: il protocollo dichiara «{prefisso}.{campo}» sempre "
            "presente e il modello non lo espone: l'SDK lo butta via in "
            "silenzio, e chi lo usa deve leggere il JSON grezzo per saperlo."
        )
    for campo in sorted(dichiarati_insieme - set(attesi)):
        problemi.append(
            f"{nome}: il modello pretende «{campo}» e il protocollo non lo "
            "dichiara. Il primo binario che non lo manda fa fallire l'SDK su "
            "una busta valida."
        )
    for campo in sorted(dichiarati_insieme & opzionali):
        problemi.append(
            f"{nome}: «{campo}» e' dichiarato opzionale dal protocollo e "
            "obbligatorio dal modello. Un campo che a volte non c'e' non si "
            "puo' pretendere."
        )
    return problemi


def main() -> int:
    manifesto = json.loads(CONTRATTO.read_text(encoding="utf-8"))
    modelli = obbligatori_dichiarati()
    problemi: list[str] = []

    catalogo = manifesto["envelopes"]["catalog"]["struttura"]
    bootstrap = manifesto["busta_di_bootstrap"]

    attesi = {"Catalog": ("", catalogo), "Driver": (".drivers[]", catalogo)}
    for nome, (prefisso, struttura) in attesi.items():
        if nome not in modelli:
            problemi.append(f"il modello «{nome}» non dichiara `OBBLIGATORI`.")
            continue
        problemi.extend(
            confronta(
                nome,
                modelli[nome],
                campi_del_protocollo(struttura, prefisso),
                prefisso or "catalog",
            )
        )

    # La busta di bootstrap non ha un `OBBLIGATORI`: il suo schema e' chiuso, e
    # il modello lo scrive in linea perche' rifiuta anche i campi in **piu'**.
    # Il confronto qui e' con `schema_esatto`, che e' l'elenco chiuso del
    # manifesto: due elenchi chiusi che devono coincidere.
    da_schema = sorted(p.lstrip(".") for p in bootstrap["schema_esatto"])
    sorgente = MODELLI.read_text(encoding="utf-8")
    for campo in da_schema:
        if f'"{campo}"' not in sorgente:
            problemi.append(
                f"la busta di bootstrap dichiara «{campo}» e `models.py` non lo "
                "nomina."
            )

    # Le sottostrutture lasciate grezze devono esistere: una che sparisse dal
    # protocollo resterebbe qui a giustificare un'omissione che non c'e' piu'.
    for percorso in GREZZE:
        if percorso not in catalogo:
            problemi.append(
                f"«{percorso}» e' dichiarato grezzo e il protocollo non lo "
                "contiene piu': l'esenzione non ha piu' un oggetto."
            )

    if problemi:
        for problema in problemi:
            print(problema, file=sys.stderr)
        print(
            f"\n{len(problemi)} divergenze fra i modelli dell'SDK e il "
            "protocollo v2.",
            file=sys.stderr,
        )
        return 1

    campi = sum(len(v) for v in modelli.values())
    print(
        f"modelli dell'SDK verificati: {len(modelli)} dataclass, {campi} campi "
        f"confrontati col protocollo v2, {len(GREZZE)} sottostrutture "
        "dichiarate grezze."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
