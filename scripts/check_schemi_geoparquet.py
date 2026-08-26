#!/usr/bin/env python3
"""Gli schemi ufficiali sono davvero l'autorita', e davvero quelli.

# Perche' esiste

Il lotto S10 dichiara che la conformita' `GeoParquet` e' decisa dagli schemi
ufficiali e non dalla nostra prosa. Un'affermazione del genere regge su tre
gambe, e questo gate le controlla tutte e tre:

1. **i file fissati sono quelli** -- byte, sha256, `$id`, draft e `$ref`
   confrontati con il lock. Uno schema modificato in casa sarebbe un'autorita'
   che dice cio' che vogliamo noi;
2. **il codice non se li e' riscritti** -- gli elenchi chiusi che il driver usa
   (versioni, codifiche, suffissi di tipo, spigoli del covering) vengono
   confrontati con quelli che si **estraggono dallo schema**. La prima stesura
   del gate faceva il contrario: derivava il perimetro dal modulo che doveva
   controllare, quindi una regola sbagliata diventava la definizione;
3. **il runtime li usa** -- la dipendenza di validazione e' compilata senza i
   resolver HTTP e filesystem, e la closure di `driver-geoparquet` non contiene
   nulla che possa aprire una connessione.

# Che cosa questo gate **non** fa

Non valida documenti. Le prove che validano -- il metadato scritto da un
Parquet vero, la forma fisica della colonna `bbox`, la via di compatibilita' --
sono test Rust, e questo gate le **esegue** nominandole: sono li' perche' li'
c'e' il validatore vero, e un riferimento testuale non e' una prova.

# Uso

    python3 scripts/check_schemi_geoparquet.py
"""

from __future__ import annotations

import hashlib
import json
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from check_prove_di_confine import esegui as esegui_i_test  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
LOCK = ROOT / "assurance" / "registries" / "geoparquet-schemi-lock.json"
CENSIMENTO = ROOT / "assurance" / "registries" / "closure-driver-geoparquet.json"
MODULO_SCHEMI = ROOT / "crates" / "driver-geoparquet" / "src" / "schema_ufficiale.rs"
MODULO_METADATI = ROOT / "crates" / "driver-geoparquet" / "src" / "metadati.rs"
CRATE = "driver-geoparquet"

DRAFT_ATTESO = "http://json-schema.org/draft-07/schema#"

# La closure di `driver-geoparquet` non deve contenere niente che sappia aprire
# una connessione o leggere un file per conto suo. Non basta guardare l'intero
# `Cargo.lock`: un'altra crate del workspace potrebbe legittimamente dipendere
# da `reqwest` senza che questo driver lo faccia.
VIETATE_NELLA_CLOSURE = ("reqwest", "rustls", "aws-lc-rs", "hyper", "tokio")
FEATURE_VIETATE = ("resolve-http", "resolve-file", "tls-aws-lc-rs", "tls-ring")

# Le prove che validano davvero, eseguite da qui.
PROVE_DI_CONFORMITA = (
    # L'output del writer, contro lo schema ufficiale e il PROJJSON referenziato.
    "tests::il_metadato_scritto_rispetta_lo_schema_ufficiale",
    # La forma fisica della colonna `bbox`, che lo schema JSON non copre.
    "tests::la_colonna_bbox_scritta_ha_la_forma_fisica_che_il_covering_designa",
    # Il registro dei `$ref` in memoria, e il PROJJSON che rifiuta `{"id": ...}`.
    "schema_ufficiale::sonde::il_crs_e_validato_contro_il_projjson_referenziato",
    "schema_ufficiale::sonde::gli_schemi_incorporati_si_compilano",
    "schema_ufficiale::sonde::lo_schema_rifiuta_cio_che_la_specifica_rifiuta",
    "schema_ufficiale::sonde::il_rifiuto_non_dice_niente_del_documento",
)

# La compatibilita' storica sta **fuori** dalla prova di conformita': non e' una
# via conforme, e mescolarla direbbe che lo e'. E' una prova sua.
PROVE_DI_COMPATIBILITA = (
    "tests::un_file_storico_senza_opt_in_e_rifiutato",
    "tests::un_file_storico_con_opt_in_si_apre_e_conserva_il_crs",
    "metadati::sonde::crs_storico_senza_opt_in_e_non_conforme",
    "metadati::sonde::crs_storico_con_opt_in_conserva_l_identificatore_ed_e_accettato",
    "metadati::sonde::crs_storico_di_forma_diversa_e_non_conforme",
    "metadati::sonde::crs_storico_con_altri_difetti_e_non_conforme",
    "metadati::sonde::crs_storico_in_un_documento_1_1_0_e_non_conforme",
)


def lock() -> dict:
    return json.loads(LOCK.read_text(encoding="utf-8"))


def schemi_fissati(registro: dict | None = None) -> tuple[dict[str, dict], list[str]]:
    """I quattro schemi, verificati contro il lock voce per voce.

    Il registro e' iniettabile perche' le sonde di questo gate ne costruiscono
    di storti: provare la regola su un lock inventato e' l'unico modo di provare
    la regola invece del lock di oggi.
    """
    errori: list[str] = []
    documenti: dict[str, dict] = {}
    if registro is None:
        registro = lock()
    if registro.get("schema_version") != 2:
        errori.append("il lock degli schemi non e' alla versione 2")
        return {}, errori

    for voce in registro["schemi"]:
        percorso = ROOT / voce["percorso"]
        if not percorso.is_file():
            errori.append(f"{voce['percorso']}: assente")
            continue
        grezzo = percorso.read_bytes()
        if len(grezzo) != voce["byte"]:
            errori.append(
                f"{voce['percorso']}: {len(grezzo)} byte, il lock ne dichiara "
                f"{voce['byte']}"
            )
        atteso = hashlib.sha256(grezzo).hexdigest()
        if atteso != voce["sha256"]:
            errori.append(
                f"{voce['percorso']}: sha256 «{atteso[:16]}», il lock dichiara "
                f"«{voce['sha256'][:16]}». Uno schema modificato in casa e' "
                "un'autorita' che dice cio' che vogliamo noi."
            )
        try:
            documento = json.loads(grezzo)
        except json.JSONDecodeError as errore:
            errori.append(f"{voce['percorso']}: non e' JSON ({errore})")
            continue
        canonico = json.dumps(documento, sort_keys=True, separators=(",", ":")).encode(
            "utf-8"
        )
        if hashlib.sha256(canonico).hexdigest() != voce["sha256_canonico"]:
            errori.append(f"{voce['percorso']}: impronta canonica diversa dal lock")
        if documento.get("$schema") != DRAFT_ATTESO:
            errori.append(
                f"{voce['percorso']}: dichiara draft «{documento.get('$schema')}», "
                f"atteso «{DRAFT_ATTESO}». Il validatore e' compilato per Draft 7."
            )
        if voce["draft"] != DRAFT_ATTESO:
            errori.append(f"{voce['percorso']}: il lock dichiara un altro draft")
        atteso_id = documento.get("$id")
        if voce["famiglia"] == "projjson" and atteso_id != voce["id"]:
            errori.append(
                f"{voce['percorso']}: `$id` «{atteso_id}» diverso da quello del "
                "lock. E' con quell'URI che il registro in memoria lo indicizza: "
                "se non combacia, il `$ref` non si risolve."
            )
        documenti[f"{voce['famiglia']}-{voce['versione']}"] = documento
    return documenti, errori


def ref_risolti(documenti: dict[str, dict]) -> list[str]:
    """Ogni `$ref` esterno di GeoParquet punta a un PROJJSON fissato."""
    errori: list[str] = []
    identificatori = {
        documento.get("$id")
        for chiave, documento in documenti.items()
        if chiave.startswith("projjson-")
    }
    for chiave, documento in documenti.items():
        if not chiave.startswith("geoparquet-"):
            continue
        for ref in riferimenti(documento):
            if ref.startswith("#"):
                continue
            if ref not in identificatori:
                errori.append(
                    f"{chiave}: il `$ref` «{ref}» non e' fra gli schemi fissati "
                    f"{sorted(i for i in identificatori if i)}. Il registro e' in "
                    "memoria e non scarica niente: un `$ref` che non c'e' fa "
                    "fallire la compilazione, non una richiesta di rete."
                )
    return errori


def riferimenti(nodo) -> list[str]:
    """Tutti i `$ref` di un documento, a qualunque profondita'."""
    trovati: list[str] = []
    if isinstance(nodo, dict):
        for chiave, valore in nodo.items():
            if chiave == "$ref" and isinstance(valore, str):
                trovati.append(valore)
            else:
                trovati.extend(riferimenti(valore))
    elif isinstance(nodo, list):
        for valore in nodo:
            trovati.extend(riferimenti(valore))
    return trovati


def colonna_dello_schema(documento: dict) -> dict:
    return documento["properties"]["columns"]["patternProperties"][".+"]


def elenchi_del_codice() -> dict[str, list[str]]:
    """Gli elenchi chiusi che il driver usa, letti dai sorgenti."""
    schemi = MODULO_SCHEMI.read_text(encoding="utf-8")
    metadati = MODULO_METADATI.read_text(encoding="utf-8")
    citazione = re.compile(r'"([^"]*)"')

    def elenco(testo: str, nome: str) -> list[str]:
        trovato = re.search(rf"{nome}[^=]*=\s*\[([^\]]*)\]", testo, re.S)
        return citazione.findall(trovato.group(1)) if trovato else []

    return {
        "versioni": elenco(schemi, "VERSIONI_SUPPORTATE"),
        "codifiche_native": elenco(metadati, "CODIFICHE_NATIVE"),
        "nomi_di_tipo": elenco(metadati, "NOMI_DI_TIPO"),
        "suffissi": elenco(metadati, "SUFFISSI"),
        "spigoli": elenco(metadati, "SPIGOLI"),
    }


def elenchi_dallo_schema(documenti: dict[str, dict]) -> dict[str, list[str]]:
    """Gli stessi elenchi, **estratti dagli schemi ufficiali**."""
    uno_uno = documenti["geoparquet-1.1.0"]
    colonna = colonna_dello_schema(uno_uno)

    versioni = sorted(
        documenti[chiave]["properties"]["version"]["const"]
        for chiave in documenti
        if chiave.startswith("geoparquet-")
    )

    # `^(WKB|point|linestring|...)$` -> le alternative, meno WKB.
    codifiche = re.match(
        r"\^\(([^)]*)\)\$", colonna["properties"]["encoding"]["pattern"]
    )
    native = [v for v in (codifiche.group(1).split("|") if codifiche else []) if v != "WKB"]

    # `^(GeometryCollection|(Multi)?(Point|LineString|Polygon))( Z)?$`
    tipi = colonna["properties"]["geometry_types"]["items"]["pattern"]
    nomi: list[str] = []
    if "GeometryCollection" in tipi:
        nomi.append("GeometryCollection")
    basi = re.search(r"\(Multi\)\?\(([^)]*)\)", tipi)
    for base in basi.group(1).split("|") if basi else []:
        nomi.extend([base, f"Multi{base}"])
    suffissi = re.findall(r"\(\s(\w)\)\?", tipi)

    spigoli = sorted(colonna["properties"]["covering"]["properties"]["bbox"]["required"])

    return {
        "versioni": versioni,
        "codifiche_native": sorted(native),
        "nomi_di_tipo": sorted(nomi),
        "suffissi": sorted(f" {s}" for s in suffissi) + [""],
        "spigoli": spigoli,
    }


def elenchi_coincidono(documenti: dict[str, dict]) -> list[str]:
    """Il codice non si e' riscritto la specifica."""
    dal_codice = elenchi_del_codice()
    dallo_schema = elenchi_dallo_schema(documenti)
    errori: list[str] = []
    for nome, atteso in dallo_schema.items():
        osservato = sorted(dal_codice.get(nome, []))
        if osservato != sorted(atteso):
            errori.append(
                f"l'elenco «{nome}» del codice e' {osservato}, lo schema "
                f"ufficiale ne ricava {sorted(atteso)}. L'autorita' e' lo "
                "schema: un elenco che diverge e' il codice che si riscrive la "
                "specifica."
            )
    return errori


# La versione massima che il catalogo pubblica, dal descrittore del driver.
DICHIARATA = re.compile(
    r"// `spec_version_supported`:(?:[^\n]*\n\s*//[^\n]*)*\n\s*Some\(\s*\"([^\"]+)\"\s*\)"
)
DESCRITTORE = ROOT / "crates" / "driver-geoparquet" / "src" / "lib.rs"


def versione_dichiarata() -> str | None:
    """La versione che il descrittore pubblica nel catalogo."""
    trovato = DICHIARATA.search(DESCRITTORE.read_text(encoding="utf-8"))
    return trovato.group(1) if trovato else None


def perimetro_dichiarato(documenti: dict[str, dict]) -> list[str]:
    """Il catalogo dichiara la versione massima che **gli schemi** definiscono.

    `spec_version_supported` e' un'affermazione pubblica: un consumatore la
    legge per sapere se una 2.0 sarebbe accettata. Il confronto e' con le
    versioni ricavate dagli schemi fissati -- non con un elenco del codice --
    perche' un perimetro dichiarato diverso da quello applicato e' peggio di
    nessun perimetro: chi legge il catalogo decide su di esso.
    """
    versioni = elenchi_dallo_schema(documenti)["versioni"]
    dichiarata = versione_dichiarata()
    if dichiarata is None:
        return [
            "il descrittore di `driver-geoparquet` non dichiara "
            "`spec_version_supported`: il perimetro che il codice applica "
            "resterebbe invisibile a chi legge il catalogo"
        ]
    massima = max(versioni)
    if dichiarata != massima:
        return [
            f"il catalogo dichiara «{dichiarata}» e gli schemi ufficiali "
            f"definiscono {versioni}, la cui massima e' «{massima}»."
        ]
    return []


def supply_chain(manifesto: str | None = None) -> list[str]:
    """La dipendenza di validazione e' fissata e senza resolver."""
    errori: list[str] = []
    if manifesto is None:
        manifesto = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    riga = re.search(r"^jsonschema\s*=\s*(.+)$", manifesto, re.M)
    if not riga:
        return ["`jsonschema` non e' dichiarata nel workspace"]
    dichiarazione = riga.group(1)
    if "default-features = false" not in dichiarazione:
        errori.append(
            "`jsonschema` senza `default-features = false`: le feature "
            "predefinite riaccendono i resolver HTTP e filesystem, e uno schema "
            "che si scarica quando serve non e' fissato."
        )
    if not re.search(r'"=\d+\.\d+\.\d+"', dichiarazione):
        errori.append("`jsonschema` senza versione esatta")
    for feature in FEATURE_VIETATE:
        if feature in dichiarazione:
            errori.append(f"`jsonschema` riaccende la feature «{feature}»")
    return errori


def closure_del_driver() -> list[str]:
    """Le crate raggiungibili da `driver-geoparquet`, con `cargo tree`."""
    import subprocess

    try:
        esito = subprocess.run(
            ["cargo", "tree", "-p", CRATE, "--edges", "normal", "--prefix", "none"],
            cwd=str(ROOT),
            capture_output=True,
            text=True,
            check=False,
        )
    except OSError as errore:
        return [f"`cargo tree` non e' invocabile ({errore})"]
    if esito.returncode != 0:
        return [f"`cargo tree` esce con {esito.returncode}"]

    nomi = {riga.split()[0] for riga in esito.stdout.splitlines() if riga.strip()}
    errori = [
        f"la closure di {CRATE} contiene «{vietata}»: questo driver non deve "
        "poter aprire una connessione ne' leggere uno schema da fuori"
        for vietata in VIETATE_NELLA_CLOSURE
        if vietata in nomi
    ]
    return errori


def censimento_della_closure(nomi: set[str] | None = None) -> list[str]:
    """Il censimento nomina **esattamente** la closure, con le licenze.

    Un censimento che si limitasse a un numero -- «sono entrate 63 crate» --
    direbbe quanto e non che cosa: la volta dopo il numero tornerebbe con crate
    diverse e nessuno se ne accorgerebbe. Qui il gate ricalcola l'insieme dal
    `Cargo.lock` e pretende che coincida voce per voce, cosi' una dipendenza
    nuova entra solo passando di qui, con la sua licenza sotto gli occhi.
    """
    if not CENSIMENTO.is_file():
        return [f"{CENSIMENTO.name}: censimento della closure assente"]
    registro = json.loads(CENSIMENTO.read_text(encoding="utf-8"))
    dichiarate = {voce["crate"] for voce in registro["crate"]}

    if nomi is None:
        osservate = closure_dal_lock()
        if osservate is None:
            return ["la closure non si ricava dal `Cargo.lock`"]
    else:
        osservate = nomi

    errori: list[str] = []
    entrate = sorted(osservate - dichiarate)
    uscite = sorted(dichiarate - osservate)
    if entrate:
        errori.append(
            f"crate nella closure di {CRATE} e non nel censimento: {entrate}. "
            "Una dipendenza che entra senza essere censita entra senza che "
            "nessuno ne abbia guardato la licenza."
        )
    if uscite:
        errori.append(
            f"crate censite e non piu' nella closure: {uscite}. Un censimento "
            "che nomina cio' che non c'e' piu' e' un elenco che nessuno rilegge."
        )
    if registro.get("crate_totali") != len(registro["crate"]):
        errori.append("il conteggio del censimento non coincide con il suo elenco")
    senza = [v["crate"] for v in registro["crate"] if not v.get("licenza")]
    if senza:
        errori.append(f"crate censite senza licenza: {senza}")
    return errori


def closure_dal_lock() -> set[str] | None:
    """La closure di `driver-geoparquet`, per attraversamento del lock.

    Dal lock e non da `cargo tree`: cosi' il censimento si verifica anche dove
    `cargo` non c'e', e la risposta non dipende da quali feature sono attive nel
    momento in cui si guarda.
    """
    testo = (ROOT / "Cargo.lock").read_text(encoding="utf-8")
    pacchi: dict[str, list[str]] = {}
    for blocco in testo.split("[[package]]")[1:]:
        nome = re.search(r'^name = "([^"]+)"', blocco, re.M)
        if not nome:
            continue
        dipendenze = re.search(r"^dependencies = \[(.*?)\]", blocco, re.M | re.S)
        elenco = (
            [
                d.strip().strip('",').split()[0]
                for d in dipendenze.group(1).splitlines()
                if d.strip()
            ]
            if dipendenze
            else []
        )
        pacchi[nome.group(1)] = [d for d in elenco if d]
    if CRATE not in pacchi:
        return None
    visti: set[str] = set()
    coda = [CRATE]
    while coda:
        nome = coda.pop()
        if nome in visti or nome not in pacchi:
            continue
        visti.add(nome)
        coda.extend(pacchi[nome])
    return visti


def verifica() -> tuple[list[str], dict[str, dict]]:
    documenti, errori = schemi_fissati()
    if errori:
        return errori, documenti
    errori.extend(ref_risolti(documenti))
    errori.extend(elenchi_coincidono(documenti))
    errori.extend(perimetro_dichiarato(documenti))
    errori.extend(supply_chain())
    errori.extend(closure_del_driver())
    errori.extend(censimento_della_closure())
    return errori, documenti


def main() -> int:
    errori, documenti = verifica()
    if not errori:
        errori.extend(
            esegui_i_test(CRATE, PROVE_DI_CONFORMITA, "la prova di conformita'")
        )
        errori.extend(
            esegui_i_test(CRATE, PROVE_DI_COMPATIBILITA, "la prova di compatibilita'")
        )

    for messaggio in errori:
        print(messaggio, file=sys.stderr)
    if errori:
        return 1

    print(
        f"schemi GeoParquet verificati: {len(documenti)} file fissati -- byte, "
        "sha256, impronta canonica, `$id`, Draft 7 e `$ref` -- e gli elenchi "
        "chiusi del codice ricavati **dagli schemi**, non dal codice. La closure "
        f"di {CRATE} non contiene {list(VIETATE_NELLA_CLOSURE)}, e `jsonschema` "
        f"resta senza i resolver; il censimento nomina le sue "
        f"{len(json.loads(CENSIMENTO.read_text(encoding='utf-8'))['crate'])} crate "
        "con le loro licenze. Il catalogo dichiara "
        f"«{versione_dichiarata()}», la massima che gli schemi definiscono. "
        f"{len(PROVE_DI_CONFORMITA)} prove di conformita' eseguite -- fra cui il "
        "metadato di un Parquet realmente scritto e la forma fisica della "
        f"colonna `bbox` -- e {len(PROVE_DI_COMPATIBILITA)} prove della sola via "
        "storica, tenute separate perche' quella via conforme non e'."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
