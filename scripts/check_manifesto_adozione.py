#!/usr/bin/env python3
"""Il manifesto di adozione dice il vero, e lo dice nella forma fissata.

# Che cosa verifica

Quattro cose, e nessuna e' una ripetizione delle altre.

1. **La forma.** Il documento valida contro
   `adoption-manifest-v4.schema.json` del checkout fissato -- non contro una
   copia locale, che si allineerebbe da sola il giorno in cui il contratto
   cambia.
2. **Il pin.** `contracts_source.revision` e' la stessa revisione che
   `contracts/adoption-source.json` dichiara. Due pin diversi nello stesso
   repository sono due adozioni diverse che si presentano come una.
3. **I digest.** Ogni artefatto elencato esiste e il suo `sha256` e' quello
   misurato ora. Un digest che nessuno ricalcola e' una garanzia apparente:
   chi legge il manifesto suppone che qualcuno l'abbia controllata.
4. **La copertura dei contratti.** Ogni contratto che il profilo dichiara
   applicabile compare nel manifesto. Un contratto applicabile e taciuto e' la
   forma piu' comoda di conformita' parziale: non si vede.

# Che cosa **non** verifica, e perche' va detto

Che i contratti dichiarati `conforming` lo siano davvero. Quello lo dicono i
comandi in `verification`, che girano altrove -- il verificatore del profilo
pubblico, le prove al confine, i gate. Questo gate verifica che il **documento**
sia onesto nella sua forma, non che il prodotto sia conforme: sono due domande,
e un gate che pretendesse di rispondere a entrambe risponderebbe male alla
seconda.

Verifica pero' una cosa che riguarda la sostanza: che nessun contratto sia
dichiarato `not_applicable` mentre una deviazione lo nomina. `not_applicable`
dice «non si applica», una deviazione dice «si applica e non lo soddisfo»:
insieme sono una contraddizione, e un lettore ne trarrebbe conclusioni opposte.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
import sys

RADICE = pathlib.Path(__file__).resolve().parent.parent
MANIFESTO = RADICE / "contracts" / "adoption-manifest.json"
SORGENTE_DEL_PIN = RADICE / "contracts" / "adoption-source.json"

#: I contratti che il profilo dichiara applicabili, letti dal profilo stesso
#: del checkout fissato: l'elenco non si ricopia qui.
PROFILO = "profiles/io-tools.md"


def contratti_del_profilo(contracts: pathlib.Path) -> set[str]:
    """Gli identificatori dei contratti che il profilo elenca.

    Il profilo li nomina per **documento** (`../specs/errors/ERRORS-1.0.md`), e
    l'identificatore sta dentro quel documento alla riga «Contract identifier».
    Si segue il rimando invece di tenere una tabella: una tabella diventa
    vecchia senza che nulla lo dica.
    """
    testo = (contracts / PROFILO).read_text(encoding="utf-8")
    dentro = testo.split("## Applicable contracts", 1)[1].split("##", 1)[0]
    identificatori = set()
    for rimando in re.findall(r"\]\((\.\./specs/[^)]+\.md)\)", dentro):
        documento = (contracts / "profiles" / rimando).resolve()
        if not documento.is_file():
            continue
        trovato = re.search(
            r"^Contract identifier: `([^`]+)`", documento.read_text(encoding="utf-8"), re.M
        )
        if trovato:
            identificatori.add(trovato.group(1))
    return identificatori


def valida_forma(documento: dict, schema: dict, percorso: str = "") -> list[str]:
    """Un validatore minimo per i costrutti che questo schema usa davvero.

    Non e' un validatore JSON Schema generale, e non deve esserlo: `jsonschema`
    non e' fra le dipendenze e aggiungerne una per un gate sarebbe un costo
    permanente per un uso solo. I costrutti coperti sono quelli che
    `adoption-manifest-v4.schema.json` impiega -- `type`, `required`,
    `properties`, `additionalProperties`, `items`, `enum`, `const`, `pattern`,
    `minItems`, `uniqueItems`, `minLength`, `maxLength`, `$ref` interni e
    `allOf`/`anyOf`/`if`/`then`/`else` -- e ogni costrutto **non** riconosciuto
    diventa un errore invece di essere ignorato: un validatore che salta cio'
    che non capisce dice verde su cio' che non ha guardato.
    """
    return _valida(documento, schema, schema, percorso)


_CONOSCIUTI = {
    "$schema", "$id", "$defs", "$comment", "title", "description",
    "type", "required", "properties", "additionalProperties", "items",
    "enum", "const", "pattern", "minItems", "uniqueItems", "minLength",
    "maxLength", "minimum", "allOf", "anyOf", "if", "then", "else", "not", "$ref",
}

_TIPI = {
    "object": dict,
    "array": list,
    "string": str,
    "integer": int,
    "boolean": bool,
}


def _risolvi(schema: dict, radice: dict) -> dict:
    while "$ref" in schema:
        rimando = schema["$ref"]
        if not rimando.startswith("#/"):
            raise ValueError(f"$ref esterno non supportato: {rimando}")
        nodo = radice
        for pezzo in rimando[2:].split("/"):
            nodo = nodo[pezzo]
        schema = nodo
    return schema


def _valida(valore, schema: dict, radice: dict, dove: str) -> list[str]:
    schema = _risolvi(schema, radice)
    ignoti = set(schema) - _CONOSCIUTI
    if ignoti:
        return [f"{dove or '/'}: lo schema usa costrutti che il gate non conosce: {sorted(ignoti)}"]

    errori: list[str] = []
    if "type" in schema:
        atteso = _TIPI[schema["type"]]
        if atteso is int and isinstance(valore, bool):
            errori.append(f"{dove}: atteso {schema['type']}, trovato boolean")
        elif not isinstance(valore, atteso):
            errori.append(f"{dove}: atteso {schema['type']}, trovato {type(valore).__name__}")
            return errori
    if "const" in schema and valore != schema["const"]:
        errori.append(f"{dove}: atteso {schema['const']!r}, trovato {valore!r}")
    if "enum" in schema and valore not in schema["enum"]:
        errori.append(f"{dove}: {valore!r} fuori da {schema['enum']}")
    if isinstance(valore, str):
        if "pattern" in schema and not re.search(schema["pattern"], valore):
            errori.append(f"{dove}: «{valore}» non combacia con {schema['pattern']}")
        if "minLength" in schema and len(valore) < schema["minLength"]:
            errori.append(f"{dove}: piu' corto di {schema['minLength']}")
        if "maxLength" in schema and len(valore) > schema["maxLength"]:
            errori.append(f"{dove}: piu' lungo di {schema['maxLength']}")
    if isinstance(valore, list):
        if "minItems" in schema and len(valore) < schema["minItems"]:
            errori.append(f"{dove}: meno di {schema['minItems']} elementi")
        if schema.get("uniqueItems") and len(
            {json.dumps(v, sort_keys=True) for v in valore}
        ) != len(valore):
            errori.append(f"{dove}: elementi ripetuti")
        if "items" in schema:
            for indice, uno in enumerate(valore):
                errori.extend(_valida(uno, schema["items"], radice, f"{dove}[{indice}]"))
    if isinstance(valore, dict):
        for chiave in schema.get("required", []):
            if chiave not in valore:
                errori.append(f"{dove}: manca «{chiave}»")
        proprieta = schema.get("properties", {})
        if schema.get("additionalProperties") is False:
            estranee = sorted(set(valore) - set(proprieta))
            if estranee:
                errori.append(f"{dove}: chiavi non previste {estranee}")
        for chiave, sotto in proprieta.items():
            if chiave in valore:
                errori.extend(_valida(valore[chiave], sotto, radice, f"{dove}.{chiave}"))
    for sotto in schema.get("allOf", []):
        errori.extend(_valida(valore, sotto, radice, dove))
    if "anyOf" in schema:
        if all(_valida(valore, s, radice, dove) for s in schema["anyOf"]):
            errori.append(f"{dove}: nessuna delle alternative di anyOf e' soddisfatta")
    if "if" in schema:
        ramo = "then" if not _valida(valore, schema["if"], radice, dove) else "else"
        if ramo in schema:
            errori.extend(_valida(valore, schema[ramo], radice, dove))
    if "not" in schema and not _valida(valore, schema["not"], radice, dove):
        # Il messaggio nomina cio' che il `not` vieta: «soddisfa un not» non
        # dice a chi legge quale chiave togliere, ed e' l'unico costrutto in
        # cui l'errore sta in cio' che c'e' invece che in cio' che manca.
        vietate = _risolvi(schema["not"], radice).get("required")
        dettaglio = f": {sorted(vietate)} non va qui" if vietate else ""
        errori.append(f"{dove}: soddisfa un `not`{dettaglio}")
    return errori


def digest(percorso: pathlib.Path) -> str:
    impronta = hashlib.sha256()
    with percorso.open("rb") as flusso:
        for pezzo in iter(lambda: flusso.read(1024 * 1024), b""):
            impronta.update(pezzo)
    return f"sha256:{impronta.hexdigest()}"


def verifica(
    manifesto: dict, contracts: pathlib.Path, artefatti: dict[str, pathlib.Path]
) -> list[str]:
    errori: list[str] = []

    schema_percorso = contracts / "schemas" / "adoption-manifest-v4.schema.json"
    if not schema_percorso.is_file():
        return [f"lo schema «{schema_percorso.name}» non e' nel checkout fissato"]
    errori.extend(valida_forma(manifesto, json.loads(schema_percorso.read_text("utf-8"))))

    pin = json.loads(SORGENTE_DEL_PIN.read_text(encoding="utf-8"))
    atteso = (pin.get("contracts_source") or {}).get("revision")
    dichiarato = manifesto.get("contracts_source", {}).get("revision")
    if atteso and dichiarato != atteso:
        errori.append(
            f"il manifesto fissa «{dichiarato}» e `adoption-source.json` «{atteso}»: "
            "due pin nello stesso repository sono due adozioni che si presentano come una"
        )

    dichiarati = {c["id"]: c for c in manifesto.get("contracts", [])}
    applicabili = contratti_del_profilo(contracts)
    taciuti = sorted(applicabili - set(dichiarati))
    if taciuti:
        errori.append(
            f"contratti che il profilo dichiara applicabili e il manifesto tace: "
            f"{taciuti}. Un contratto applicabile e non nominato e' conformita' "
            "parziale che non si vede."
        )

    for deviazione in manifesto.get("deviations", []):
        for identificatore, voce in dichiarati.items():
            if identificatore in deviazione["rule"] and voce["status"] == "not_applicable":
                errori.append(
                    f"«{identificatore}» e' dichiarato `not_applicable` e una deviazione "
                    "lo nomina: «non si applica» e «si applica e non lo soddisfo» non "
                    "possono valere insieme."
                )

    for voce in manifesto.get("artifacts", []):
        percorso = artefatti.get(voce["name"])
        if percorso is None:
            errori.append(
                f"«{voce['name']}»: nessun file passato al gate. I digest si "
                "ricalcolano sui byte, e un digest che nessuno ricalcola e' una "
                "garanzia apparente."
            )
            continue
        misurato = digest(percorso)
        if misurato != voce["digest"]:
            errori.append(
                f"«{voce['name']}»: il manifesto dice {voce['digest']}, i byte dicono "
                f"{misurato}"
            )

    return errori


def verifica_redatto(redatto: dict, contracts: pathlib.Path) -> list[str]:
    """Le regole che non hanno bisogno degli artefatti.

    La parte redatta -- contratti, stato, deviazioni, pin -- si rilegge a ogni
    commit; i digest esistono solo dopo che gli artefatti sono stati costruiti.
    Pretendere i secondi per verificare i primi vorrebbe dire non verificare i
    primi quasi mai, che e' il contrario di cio' che serve: le deviazioni sono
    la parte che si sbaglia scrivendo, non misurando.
    """
    finto = {
        "schema_version": 4,
        "component": redatto["component"],
        "contracts_source": redatto["contracts_source"],
        "profile": redatto["profile"],
        # Un artefatto segnaposto **non** viene inventato: la forma degli
        # artefatti la verifica la qualifica, sui file veri. Qui si guarda cio'
        # che e' scritto a mano.
        "artifacts": [],
        "contracts": redatto["contracts"],
        "deviations": redatto["deviations"],
    }
    schema_percorso = contracts / "schemas" / "adoption-manifest-v4.schema.json"
    schema = json.loads(schema_percorso.read_text("utf-8"))
    # `artifacts` ha `minItems: 1`, e qui e' vuoto per costruzione: l'errore
    # corrispondente e' atteso e si toglie, gli altri no.
    errori = [
        e
        for e in valida_forma(finto, schema)
        if "artifacts" not in e.split(":")[0]
    ]
    # `verifica` rivalida anche la forma, e con `artifacts` vuoto ripeterebbe
    # lo stesso errore atteso: si filtra li' come sopra.
    errori.extend(
        e
        for e in verifica(finto, contracts, {})
        if "nessun file passato" not in e and "artifacts" not in e.split(":")[0]
    )
    return errori


def main() -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument("--contracts", required=True)
    argomenti.add_argument("--manifesto", default=str(MANIFESTO))
    argomenti.add_argument(
        "--redatto",
        action="store_true",
        help="verifica la sola parte redatta, senza gli artefatti: gira a ogni commit",
    )
    argomenti.add_argument(
        "--artefatto",
        action="append",
        default=[],
        help="percorso di un artefatto elencato nel manifesto, per ricalcolarne il digest",
    )
    opzioni = argomenti.parse_args()

    if opzioni.redatto:
        redatto = json.loads(
            (RADICE / "contracts" / "adozione-4.0.0.json").read_text(encoding="utf-8")
        )
        errori = verifica_redatto(redatto, pathlib.Path(opzioni.contracts))
        if errori:
            for uno in errori:
                print(uno, file=sys.stderr)
            return 1
        conformi = sum(1 for c in redatto["contracts"] if c["status"] == "conforming")
        print(
            f"parte redatta del manifesto verificata: {conformi} contratti conformi, "
            f"{len(redatto['contracts']) - conformi} non applicabili, "
            f"{len(redatto['deviations'])} deviazioni, pin allineato con "
            "`adoption-source.json`. I digest degli artefatti li misura la qualifica."
        )
        return 0

    percorso = pathlib.Path(opzioni.manifesto)
    if not percorso.is_file():
        print(
            f"«{opzioni.manifesto}» non esiste. Si produce con "
            "`scripts/costruisci-manifesto-adozione.py`, che misura i digest sugli "
            "artefatti costruiti.",
            file=sys.stderr,
        )
        return 1

    artefatti = {pathlib.Path(a).name: pathlib.Path(a) for a in opzioni.artefatto}
    errori = verifica(
        json.loads(percorso.read_text(encoding="utf-8")),
        pathlib.Path(opzioni.contracts),
        artefatti,
    )
    if errori:
        for uno in errori:
            print(uno, file=sys.stderr)
        return 1

    manifesto = json.loads(percorso.read_text(encoding="utf-8"))
    conformi = sum(1 for c in manifesto["contracts"] if c["status"] == "conforming")
    print(
        f"manifesto di adozione verificato: {len(manifesto['artifacts'])} artefatti con "
        f"digest ricalcolati, {conformi} contratti dichiarati conformi e "
        f"{len(manifesto['contracts']) - conformi} non applicabili, "
        f"{len(manifesto['deviations'])} deviazioni. Le deviazioni **non** contano come "
        "conformita' per le regole che nominano."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
