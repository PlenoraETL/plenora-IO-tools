#!/usr/bin/env python3
"""La copertura della matrice cross-format si **deriva** dal registro.

# Il difetto che chiude

Dieci driver R/W sembrano cento conversioni, e non lo sono: profilo, classe di
CRS dell'origine, classe di fedelta' del bersaglio e multi-layer rompono
l'equivalenza. Provare cento coppie con la stessa asserzione costerebbe cento
volte e proverebbe una cosa sola; provarne quattordici scelte male ne
proverebbe meno di quattro.

La scelta va quindi **verificata**, non dichiarata. Il registro nomina le
conversioni una per una; questo gate ne deriva la copertura e diventa rosso se
una classe resta scoperta -- anche una classe che nasce domani, perche' le
classi vengono dai descrittori e non da un elenco scritto qui.

# Perche' la copertura non e' un numero nel registro

Un «14/14» scritto accanto alle conversioni sarebbe un numero da tenere
allineato a mano, e il modo in cui smette di esserlo non si vede: si toglie una
conversione, il conteggio resta, e la riga continua a dire che la copertura e'
completa. Qui non c'e' niente da allineare.

# Che cosa questo gate **non** puo' verificare

Che i campi dei driver nel registro descrivano davvero i `FormatDescriptor`.
Il catalogo emesso dalla CLI non porta `fidelity_class`, `crs_handling` ne'
`multi_layer`, quindi da Python quei valori non sono derivabili da nessuna
fonte viva. A confrontarli e' il test Rust
`conversioni::il_registro_descrive_i_driver_come_i_descrittori`, che ha i
descrittori in mano. I due controlli guardano due cose diverse e nessuno dei
due basta da solo.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import sys

RADICE = pathlib.Path(__file__).resolve().parent.parent
REGISTRO = RADICE / "assurance" / "registries" / "conversioni-cross-format.json"
SUITE = RADICE / "crates" / "plenora-io-cli" / "tests" / "conversioni.rs"

# I campi che ogni driver del registro deve portare. Sono i nomi dei campi del
# `FormatDescriptor`, non nomi nuovi: un secondo vocabolario per le stesse
# proprieta' obbligherebbe a tradurre, e una traduzione e' un posto in cui
# sbagliare.
CAMPI_DEL_DRIVER = frozenset(
    {"direction", "crs_handling", "fidelity_class", "multi_layer", "runtime"}
)

# I valori ammessi, dagli enum di `descriptor.rs`. Un valore fuori da questi
# non e' un driver descritto diversamente: e' un refuso, e un refuso in un
# registro che governa la copertura la rende falsa in silenzio.
DIREZIONI = frozenset({"Read", "Write", "Bidirectional"})
CRS = frozenset({"Embedded", "FixedWgs", "None"})
FEDELTA = frozenset({"Lossless", "Conditional", "Approximating"})
RUNTIME = frozenset({"PureRust", "Gdal"})

PROFILI = frozenset({"base", "filegdb"})

# Le forme di rifiuto, e sono tre perche' tre sono le autorita' che rifiutano:
# la capability del bersaglio, il CRS, e la riga. Un vocabolario con un solo
# «rifiuto» direbbe che il caso non e' passato senza dire chi l'ha fermato, e
# tre casi con cause diverse si somiglierebbero nel registro.
ESITI = frozenset({"successo", "rifiuto_capability", "rifiuto_crs", "rifiuto_riga"})
RIFIUTI = frozenset(e for e in ESITI if e != "successo")

CAMPI_DELLA_CONVERSIONE = frozenset(
    {
        "id",
        "sorgente",
        "destinazione",
        "fixture",
        "opzioni",
        "profilo",
        "esito_atteso",
        "test",
        "prova",
    }
)

#: Il registro delle fixture, per verificare che ogni caso ne nomini una vera.
FIXTURE = RADICE / "assurance" / "registries" / "fixture-canoniche.json"


def _driver_ben_formati(registro: dict) -> list[str]:
    errori: list[str] = []
    driver = registro.get("driver")
    if not isinstance(driver, dict) or not driver:
        return ["`driver` assente o vuoto: senza i descrittori non c'e' copertura da derivare"]

    for nome, d in sorted(driver.items()):
        if not isinstance(d, dict):
            errori.append(f"driver «{nome}»: non e' un oggetto")
            continue
        mancanti = CAMPI_DEL_DRIVER - set(d)
        if mancanti:
            errori.append(f"driver «{nome}»: campi mancanti {sorted(mancanti)}")
        extra = set(d) - CAMPI_DEL_DRIVER
        if extra:
            errori.append(
                f"driver «{nome}»: campi non previsti {sorted(extra)}. I nomi sono "
                "quelli del `FormatDescriptor`, e uno in piu' qui e' un campo che "
                "nessun descrittore confronta."
            )
        for campo, ammessi in (
            ("direction", DIREZIONI),
            ("crs_handling", CRS),
            ("fidelity_class", FEDELTA),
            ("runtime", RUNTIME),
        ):
            if campo in d and d[campo] not in ammessi:
                errori.append(
                    f"driver «{nome}»: `{campo}` vale «{d[campo]}», fuori da {sorted(ammessi)}"
                )
        if "multi_layer" in d and not isinstance(d["multi_layer"], bool):
            errori.append(f"driver «{nome}»: `multi_layer` non e' un booleano")
    return errori


def _conversioni_ben_formate(registro: dict) -> list[str]:
    errori: list[str] = []
    conversioni = registro.get("conversioni")
    if not isinstance(conversioni, list) or not conversioni:
        return [
            "`conversioni` assente o vuoto. Un elenco vuoto supera ogni verifica "
            "di copertura senza provare niente, ed e' il modo piu' comodo di "
            "rendere verde questo gate."
        ]
    driver = registro.get("driver") or {}
    visti: set[str] = set()
    for c in conversioni:
        if not isinstance(c, dict) or not isinstance(c.get("id"), str):
            errori.append(f"conversione senza id leggibile: {c!r}")
            continue
        identita = c["id"]
        if identita in visti:
            errori.append(f"conversione «{identita}»: dichiarata due volte")
        visti.add(identita)
        mancanti = CAMPI_DELLA_CONVERSIONE - set(c)
        if mancanti:
            errori.append(f"conversione «{identita}»: campi mancanti {sorted(mancanti)}")
            continue
        for estremo in ("sorgente", "destinazione"):
            if c[estremo] not in driver:
                errori.append(
                    f"conversione «{identita}»: `{estremo}` nomina «{c[estremo]}», "
                    "che non e' fra i driver del registro"
                )
        if c["profilo"] not in PROFILI:
            errori.append(f"conversione «{identita}»: profilo «{c['profilo']}» non ammesso")
        if c["esito_atteso"] not in ESITI:
            errori.append(f"conversione «{identita}»: esito «{c['esito_atteso']}» non ammesso")
        if not isinstance(c.get("opzioni"), list) or any(
            not isinstance(o, str) for o in c.get("opzioni", [])
        ):
            errori.append(
                f"conversione «{identita}»: `opzioni` non e' un elenco di argomenti"
            )
        if not isinstance(c.get("prova"), str) or not c["prova"].strip():
            errori.append(
                f"conversione «{identita}»: `prova` vuota. Una conversione senza "
                "la propria asserzione scritta e' un caso che passa perche' il "
                "comando esce con zero."
            )
    return errori


def _ogni_conversione_ha_il_proprio_test(registro: dict) -> list[str]:
    """Il nome del test compare davvero nella suite.

    Non e' un'esecuzione -- la esegue il checkpoint -- ma chiude il caso in cui
    il registro nomina un test che non esiste: li' la copertura sarebbe
    derivata da conversioni che nessuno prova.
    """
    if not SUITE.exists():
        return [f"{SUITE.relative_to(RADICE).as_posix()}: la suite non esiste"]
    testo = SUITE.read_text(encoding="utf-8")
    errori: list[str] = []
    for c in registro.get("conversioni", []):
        if not isinstance(c, dict) or "test" not in c:
            continue
        nome = str(c["test"]).rsplit("::", 1)[-1]
        if f"fn {nome}(" not in testo:
            errori.append(
                f"conversione «{c.get('id')}»: la suite non definisce «{nome}». "
                "Un caso dichiarato e non provato conta nella copertura e non "
                "prova niente."
            )
    return errori


def _ogni_fixture_e_dichiarata(registro: dict) -> list[str]:
    """La fixture di ogni caso e' fra quelle di cui la review ha risposto.

    Non e' un doppione del gate delle fixture: quello verifica che i byte
    presenti siano quelli dichiarati, questo che una conversione non nomini una
    fixture che non esiste. Il primo guarda l'albero, il secondo il registro, e
    un caso che nominasse un file inventato passerebbe il primo e conterebbe
    nella copertura senza poter essere eseguito.
    """
    if not FIXTURE.exists():
        return [f"{FIXTURE.relative_to(RADICE).as_posix()}: registro delle fixture assente"]
    dichiarate = {
        voce.get("percorso")
        for voce in json.loads(FIXTURE.read_text(encoding="utf-8")).get("fixture", [])
        if isinstance(voce, dict)
    }
    errori: list[str] = []
    for c in registro.get("conversioni", []):
        if not isinstance(c, dict):
            continue
        fixture = c.get("fixture")
        if not isinstance(fixture, str) or not fixture:
            errori.append(f"conversione «{c.get('id')}»: `fixture` assente")
            continue
        # Lo Shapefile e il FileGDB sono insiemi di file: la fixture nomina il
        # membro che si apre, e il registro dichiara ogni membro. Basta che il
        # nominato sia fra quelli, o ne sia il prefisso di directory.
        if fixture not in dichiarate and not any(
            percorso.startswith(f"{fixture}/") for percorso in dichiarate if percorso
        ):
            errori.append(
                f"conversione «{c.get('id')}»: la fixture «{fixture}» non e' dichiarata "
                "in fixture-canoniche.json"
            )
    return errori


def copertura(registro: dict) -> tuple[dict, list[str]]:
    """`(riassunto, motivi)` della copertura, **derivata** dal registro."""
    driver = registro.get("driver") or {}
    conversioni = [c for c in registro.get("conversioni", []) if isinstance(c, dict)]

    # Un rifiuto atteso non copre l'estremo: prova che il driver **non** e'
    # disponibile, e contarlo come copertura direbbe che quel formato e' stato
    # convertito quando la conversione e' proprio cio' che non e' avvenuto.
    riuscite = [c for c in conversioni if c.get("esito_atteso") == "successo"]

    sorgenti = {c.get("sorgente") for c in riuscite}
    destinazioni = {c.get("destinazione") for c in riuscite}
    bidirezionali = {n for n, d in driver.items() if d.get("direction") == "Bidirectional"}

    motivi: list[str] = []
    for nome in sorted(bidirezionali - sorgenti):
        motivi.append(
            f"«{nome}» e' bidirezionale e non compare come **sorgente** di nessuna "
            "conversione riuscita"
        )
    for nome in sorted(bidirezionali - destinazioni):
        motivi.append(
            f"«{nome}» e' bidirezionale e non compare come **destinazione** di "
            "nessuna conversione riuscita"
        )

    crs_coperte = {driver.get(c.get("sorgente"), {}).get("crs_handling") for c in riuscite}
    for classe in sorted(CRS - crs_coperte):
        motivi.append(
            f"nessuna conversione ha una sorgente con `crs_handling: {classe}`. "
            "Le tre classi si comportano in modo diverso al confine: chi porta il "
            "CRS, chi lo fissa, e chi lo esige da fuori."
        )

    fedelta_coperte = {
        driver.get(c.get("destinazione"), {}).get("fidelity_class") for c in riuscite
    }
    for classe in sorted(FEDELTA - fedelta_coperte):
        motivi.append(
            f"nessuna conversione ha una destinazione con `fidelity_class: {classe}`. "
            "E' la classe del **bersaglio** a decidere che cosa il `LossReport` deve "
            "dire, ed e' li' che le tre si distinguono."
        )

    if not any(driver.get(c.get("sorgente"), {}).get("multi_layer") for c in riuscite):
        motivi.append(
            "nessuna conversione parte da un formato multi-layer: la selezione di "
            "un layer, e la dichiarazione di quelli non scelti, resterebbe non provata"
        )

    profili = {c.get("profilo") for c in conversioni}
    for profilo in sorted(PROFILI - profili):
        motivi.append(f"nessuna conversione dichiara il profilo «{profilo}»")

    for esito in sorted(RIFIUTI):
        if not any(c.get("esito_atteso") == esito for c in conversioni):
            motivi.append(
                f"nessuna conversione attende un `{esito}`: un rifiuto che il "
                "contratto pretende, e che nessun caso prova, e' una promessa "
                "verificata soltanto dalla prosa"
            )

    riassunto = {
        "conversioni": len(conversioni),
        "riuscite": len(riuscite),
        "rifiuti_attesi": len(conversioni) - len(riuscite),
        "driver_bidirezionali": len(bidirezionali),
        "come_sorgente": len(bidirezionali & sorgenti),
        "come_destinazione": len(bidirezionali & destinazioni),
        "classi_crs_in_origine": sorted(x for x in crs_coperte if x),
        "classi_fedelta_in_destinazione": sorted(x for x in fedelta_coperte if x),
        "profili": sorted(x for x in profili if x),
    }
    return riassunto, motivi


def verifica() -> tuple[dict, list[str]]:
    if not REGISTRO.exists():
        return {}, [f"{REGISTRO.relative_to(RADICE).as_posix()}: registro assente"]
    registro = json.loads(REGISTRO.read_text(encoding="utf-8"))

    errori: list[str] = []
    if registro.get("schema_version") != 2:
        errori.append(f"schema_version «{registro.get('schema_version')}»: attesa 2")
    errori.extend(_driver_ben_formati(registro))
    errori.extend(_conversioni_ben_formate(registro))
    if errori:
        # Senza un registro ben formato la copertura non si deriva: proseguire
        # produrrebbe un riassunto costruito su voci che non si sanno leggere.
        return {}, errori

    errori.extend(_ogni_conversione_ha_il_proprio_test(registro))
    errori.extend(_ogni_fixture_e_dichiarata(registro))
    riassunto, motivi = copertura(registro)
    errori.extend(motivi)
    return riassunto, errori


def main() -> int:
    argparse.ArgumentParser(description=__doc__).parse_args()
    riassunto, errori = verifica()
    for messaggio in errori:
        print(f"ERRORE: {messaggio}", file=sys.stderr)
    if errori:
        return 1
    print(
        f"conversioni cross-format: {riassunto['conversioni']} dichiarate "
        f"({riassunto['riuscite']} riuscite, {riassunto['rifiuti_attesi']} rifiuti attesi)"
    )
    print(
        f"  driver bidirezionali: {riassunto['come_sorgente']}/"
        f"{riassunto['driver_bidirezionali']} come sorgente, "
        f"{riassunto['come_destinazione']}/{riassunto['driver_bidirezionali']} come destinazione"
    )
    print(f"  CRS in origine:        {', '.join(riassunto['classi_crs_in_origine'])}")
    print(f"  fedelta' in bersaglio: {', '.join(riassunto['classi_fedelta_in_destinazione'])}")
    print(f"  profili:               {', '.join(riassunto['profili'])}")
    print("  la copertura e' derivata dal registro, non dichiarata in esso")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
