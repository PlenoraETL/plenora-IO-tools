#!/usr/bin/env python3
"""Verifica il **contratto corrente**: invarianti che governano ancora il codice.

# Che cosa questo gate non fa piu'

La versione precedente validava la provenienza della release 1.0.0-rc.2
leggendo **quarantuno documenti Markdown** come database. Quei documenti erano
la cronaca di come si era arrivati a una release gia' emessa, e un gate che
verifica una cronaca non verifica il codice.

Il gate legge ora un solo registro strutturato,
`assurance/registries/release-contract-current.json`, in cui una voce esiste
**solo se un test o un gate corrente la verifica**. Un'affermazione senza
verifica corrente non viene importata come verita': diventa `release_blocking`.

# Due modalita', e il verde dell'una non e' il verde dell'altra

* senza argomenti — il registro e' ben formato **e le prove dei verificati
  sono state eseguite**. Non dice che la release sia autorizzabile, e lo
  stampa;
* `--release` — le condizioni congiunte dell'autorizzazione, fra cui l'assenza
  di voci `release_blocking`.

# Una prova non e' un percorso che esiste

La stesura precedente controllava che il file citato da una prova fosse
presente. Un gate cancellato dal disco la faceva diventare rossa, ed era il
solo modo di accorgersene: uno strumento **presente e rotto**, un test
rinominato, un test sotto `#[ignore]`, un identificatore che non appartiene ad
alcun test — tutti passavano.

Le prove sono percio' tipizzate, e ogni tipo dice come si esegue:

* `test` — crate, configurazione, bersaglio del harness e identificatori
  esatti. Il test viene eseguito, deve comparire **una volta sola**
  nell'elenco del harness e passare;
* `gate` — comando strutturato, deduplicato fra invarianti e realmente
  eseguito: exit diverso da 0 significa invariante non verificato;
* `interna` — funzione di questo gate, eseguita in linea su un artefatto
  strutturato. Serve dove il comando sarebbe questo stesso gate;
* `esterna` — owner, artefatto e stato. Senza evidenza — stato diverso da
  `passed` — un invariante non puo' risultare `verified`: e' bloccante.

I bloccanti **non** si eseguono. Un bloccante puo' avere per definizione un
gate rosso, ed e' cio' che lo rende bloccante; cio' che deve avere e' `manca`,
la condizione che lo chiuderebbe.

E' la stessa separazione di ASSURANCE-N1, e per la stessa ragione: un verde che
significa due cose a seconda di chi lo legge e' la forma di falso verde che
questo repository ha incontrato piu' volte.

Il contratto del protocollo CLI resta verificato **nel merito**, non solo
nominato: `release/cli-protocol-v1.json` e' un artefatto strutturato, e la sua
validazione e' conservata qui.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))

# Lo stesso lettore che usa ASSURANCE-N1: due definizioni di «test
# eseguito» divergerebbero, e divergerebbero in silenzio.
from check_assurance_n1_prove import (  # noqa: E402
    BERSAGLI,
    BERSAGLIO_PREDEFINITO,
    CONFIGURAZIONI,
    analizza_uscita,
    comando_test,
)

ROOT = Path(__file__).resolve().parent.parent
REGISTRO = ROOT / "assurance" / "registries" / "release-contract-current.json"
CLI_PROTOCOL_V1 = ROOT / "release" / "cli-protocol-v1.json"

STATI = {"verified", "release_blocking"}
CAMPI = {"id", "superficie", "invariante", "prova", "stato"}

TIPI = {"test", "gate", "interna", "esterna"}
CAMPI_PER_TIPO = {
    "test": {"crate", "configurazione", "test"},
    "gate": {"comando"},
    "interna": {"funzione", "artefatto"},
    "esterna": {"owner", "artefatto", "stato"},
}

# Una prova esterna puo' rendere `verified` solo con questo stato: qualunque
# altro significa che l'evidenza non c'e', e un invariante senza evidenza e'
# bloccante, non vero.
STATO_ESTERNO_VALIDO = "passed"


def _percorsi(valore: Any) -> list[str]:
    """Un artefatto puo' essere uno o piu' percorsi; qui diventa sempre lista."""
    if valore is None:
        return []
    if isinstance(valore, str):
        return [valore]
    return list(valore)


def struttura(documento: dict[str, Any]) -> list[str]:
    """Il registro e' ben formato. Non dice che le prove passino."""
    errori: list[str] = []
    visti: set[str] = set()

    for voce in documento.get("invarianti", []):
        identita = voce.get("id", "<senza id>")
        mancanti = CAMPI - set(voce)
        if mancanti:
            errori.append(f"{identita}: campi mancanti {sorted(mancanti)}")
            continue
        if identita in visti:
            errori.append(f"{identita}: voce duplicata")
        visti.add(identita)

        stato = voce["stato"]
        if stato not in STATI:
            errori.append(f"{identita}: stato «{stato}» non ammesso; {sorted(STATI)}")
            continue

        prova = voce["prova"]
        if stato == "release_blocking":
            # Un bloccante puo' avere un gate rosso, o nessuna prova ancora.
            # Cio' che deve avere e' la **condizione di chiusura**: senza,
            # nessuno sa che cosa servirebbe per toglierlo.
            if not voce.get("manca"):
                errori.append(
                    f"{identita}: `release_blocking` senza campo `manca`. Un "
                    "blocco senza la sua condizione di chiusura non si puo' "
                    "chiudere."
                )
            continue

        if not prova:
            errori.append(
                f"{identita}: `verified` senza prova. Un invariante senza "
                "verifica corrente e' `release_blocking`, non una verita'."
            )
            continue
        if not voce.get("invariante"):
            errori.append(f"{identita}: `verified` senza invariante scritto")

        tipo = prova.get("tipo")
        if tipo not in TIPI:
            errori.append(f"{identita}: tipo di prova «{tipo}» non ammesso; {sorted(TIPI)}")
            continue
        senza = CAMPI_PER_TIPO[tipo] - set(prova)
        if senza:
            errori.append(f"{identita}: prova «{tipo}» senza {sorted(senza)}")
            continue

        if tipo == "test":
            if prova["configurazione"] not in CONFIGURAZIONI:
                errori.append(
                    f"{identita}: configurazione «{prova['configurazione']}» non ammessa"
                )
            bersaglio = prova.get("bersaglio", BERSAGLIO_PREDEFINITO)
            if bersaglio not in BERSAGLI:
                errori.append(
                    f"{identita}: bersaglio «{bersaglio}» non ammesso; "
                    f"scegliere fra {sorted(BERSAGLI)}"
                )
        if tipo == "esterna" and prova["stato"] != STATO_ESTERNO_VALIDO:
            errori.append(
                f"{identita}: prova esterna in stato «{prova['stato']}» ma "
                f"invariante `verified`. Senza evidenza — stato "
                f"«{STATO_ESTERNO_VALIDO}» — un invariante e' bloccante, non vero."
            )
        for relativo in _percorsi(prova.get("artefatto")):
            if not (ROOT / relativo).exists():
                errori.append(f"{identita}: artefatto «{relativo}» assente")
    return errori


def _comandi(prova: dict[str, Any]) -> list[list[str]]:
    return [prova["comando"], *prova.get("comandi_aggiuntivi", [])]


def esegui(documento: dict[str, Any]) -> list[str]:
    """Esegue le prove degli invarianti `verified`.

    I bloccanti non si eseguono: possono avere un gate rosso per definizione,
    ed e' cio' che li rende bloccanti. Eseguirli renderebbe il gate rosso su
    una condizione gia' dichiarata, e un rosso che si ripete smette di essere
    letto.
    """
    errori: list[str] = []
    verificati = [v for v in documento.get("invarianti", []) if v.get("stato") == "verified"]

    # --- gate: deduplicati, perche' ripetere una misura non la rende piu' vera
    visti: dict[tuple[str, ...], str] = {}
    for voce in verificati:
        prova = voce["prova"]
        if prova.get("tipo") != "gate":
            continue
        for comando in _comandi(prova):
            chiave = tuple(comando)
            if chiave in visti:
                continue
            visti[chiave] = voce["id"]
            esito = subprocess.run(comando, cwd=ROOT, capture_output=True, text=True, check=False)
            if esito.returncode != 0:
                errori.append(
                    f"{voce['id']}: la prova «{' '.join(comando)}» esce con "
                    f"{esito.returncode}. Un invariante la cui verifica fallisce "
                    "non e' verificato."
                )

    # --- test: eseguiti una volta per coppia, l'identita' deve comparire
    per_coppia: dict[tuple[str, str, str], list[dict[str, Any]]] = {}
    for voce in verificati:
        prova = voce["prova"]
        if prova.get("tipo") != "test":
            continue
        chiave = (
            prova["crate"],
            prova["configurazione"],
            prova.get("bersaglio", BERSAGLIO_PREDEFINITO),
        )
        per_coppia.setdefault(chiave, []).append(voce)

    for (crate, configurazione, bersaglio), voci in per_coppia.items():
        comando = comando_test(crate, configurazione, bersaglio)
        esito = subprocess.run(comando, cwd=ROOT, capture_output=True, text=True, check=False)
        eseguiti, duplicati = analizza_uscita(esito.stdout)
        errori.extend(f"{crate} ({configurazione}, {bersaglio}): {d}" for d in duplicati)
        if not eseguiti:
            errori.append(
                f"{crate} ({configurazione}, {bersaglio}): il harness non ha elencato alcun "
                "test. Un silenzio non e' un verde."
            )
        for voce in voci:
            for identita in voce["prova"]["test"]:
                risultato = eseguiti.get(identita)
                if risultato is None:
                    errori.append(
                        f"{voce['id']}: «{identita}» non compare fra i test "
                        f"eseguiti di {crate} ({configurazione}, {bersaglio}). Un simbolo che "
                        "esiste ma non viene eseguito non verifica niente."
                    )
                elif risultato == "ignored":
                    errori.append(f"{voce['id']}: «{identita}» e' marcato `#[ignore]`")
                elif risultato != "ok":
                    errori.append(f"{voce['id']}: «{identita}» non passa («{risultato}»)")

    # --- interna: la funzione di questo gate, in linea
    for voce in verificati:
        prova = voce["prova"]
        if prova.get("tipo") != "interna":
            continue
        funzione = globals().get(prova["funzione"])
        if funzione is None:
            errori.append(f"{voce['id']}: la funzione «{prova['funzione']}» non esiste")
            continue
        documento_artefatto = json.loads((ROOT / prova["artefatto"]).read_text(encoding="utf-8"))
        errori.extend(f"{voce['id']}: {m}" for m in funzione(documento_artefatto))

    return errori


def debito(documento: dict[str, Any]) -> list[dict[str, Any]]:
    return [v for v in documento.get("invarianti", []) if v.get("stato") == "release_blocking"]


def validate_cli_protocol_v1(document: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    if document.get("manifest_version") != 1:
        errors.append("cli-protocol-v1: manifest_version inattesa")
    if document.get("component") != "plenora-IO-tools":
        errors.append("cli-protocol-v1: componente inatteso")
    if document.get("protocol_version") != 1:
        errors.append("cli-protocol-v1: protocol_version inattesa")
    if document.get("status") != "frozen_for_1_0":
        errors.append("cli-protocol-v1: stato inatteso")
    if document.get("compatibility_scope") != "cli_json_only":
        errors.append("cli-protocol-v1: superficie non limitata alla CLI JSON")

    rust_api = document.get("rust_api", {})
    if rust_api != {
        "status": "internal_unstable",
        "semver_guarantee": False,
        "crates_publish": False,
        "reason": (
            "R15.4.1 prevede l'estrazione dei tipi di confine da "
            "plenora-io-model; una garanzia pubblica 1.x renderebbe "
            "quell'estrazione una rottura."
        ),
    }:
        errors.append("cli-protocol-v1: stato API Rust inatteso")

    expected_contracts = {
        "error": "plenora-io-error-v1",
        "catalog": "plenora-io-catalog-v1",
        "inspect": "plenora-io-inspect-v1",
        "layers": "plenora-io-layers-v1",
        "read": "plenora-io-read-v1",
        "convert": "plenora-io-convert-v1",
    }
    envelopes = document.get("envelopes", {})
    if set(envelopes) != set(expected_contracts):
        errors.append("cli-protocol-v1: devono essere dichiarate sei buste")
    for name, contract in expected_contracts.items():
        if envelopes.get(name, {}).get("contract") != contract:
            errors.append(f"cli-protocol-v1: contratto inatteso per {name}")

    error_envelope = envelopes.get("error", {})
    if error_envelope.get("optional_error_fields") != ["row_diagnostics"]:
        errors.append("cli-protocol-v1: campi errore opzionali inattesi")
    if error_envelope.get("row_diagnostics_semantics") != {
        "contract": "plenora-row-diagnostics-v1",
        "present_when": (
            "read_row_scoped_rejections_are_observed_or_write_row_scoped_"
            "rejections_are_observed_after_exact_input_total_declaration"
        ),
        "missing_write_input_total": (
            "contract_precondition_error_without_row_diagnostics"
        ),
        "absent_for_other_errors": True,
    }:
        errors.append("cli-protocol-v1: semantica row diagnostics inattesa")
    if error_envelope.get("emitted_error_codes") != [
        "CANCELLED",
        "INVALID_ROW_DIAGNOSTICS",
    ]:
        errors.append("cli-protocol-v1: token errore emessi inattesi")
    if error_envelope.get("exit_codes") != {
        "data_mapping": 2,
        "cancelled_by_caller": 130,
    }:
        errors.append("cli-protocol-v1: exit code additivi inattesi")

    catalog = envelopes.get("catalog", {})
    catalog_fields = ["available", "required_feature"]
    if catalog.get("optional_driver_fields") != catalog_fields:
        errors.append("cli-protocol-v1: campi catalogo additivi opzionali inattesi")
    if catalog.get("current_producer") != {
        "required_driver_fields": catalog_fields,
    }:
        errors.append("cli-protocol-v1: campi obbligatori del producer corrente inattesi")
    if "required_driver_fields" in catalog:
        errors.append("cli-protocol-v1: producer v1 legacy resi incompatibili")
    if catalog.get("driver_field_semantics") != {
        "available": {
            "type": "boolean",
            "true_when": "runtime_probe_satisfies_descriptor",
        },
        "required_feature": {
            "type": ["string", "null"],
            "filegdb": "gdal-backend",
            "other_drivers": None,
        },
    }:
        errors.append("cli-protocol-v1: semantica campi driver inattesa")

    convert = envelopes.get("convert", {})
    required_convert = {
        "conversion_fidelity",
        "read_fidelity",
        "write_fidelity",
        "read_loss",
        "write_loss",
    }
    if not required_convert.issubset(set(convert.get("required_top_level", []))):
        errors.append("cli-protocol-v1: osservabilità convert incompleta")
    if convert.get("forbidden_legacy_fields") != ["loss"]:
        errors.append("cli-protocol-v1: campo legacy loss non vietato")
    return errors


def main(argv: list[str] | None = None) -> int:
    argomenti = argparse.ArgumentParser(description=__doc__)
    argomenti.add_argument(
        "--release",
        action="store_true",
        help="rossa se resta anche un solo invariante release_blocking",
    )
    opzioni = argomenti.parse_args(argv)

    if not REGISTRO.exists():
        print(f"{REGISTRO}: registro assente.", file=sys.stderr)
        return 2
    documento = json.loads(REGISTRO.read_text(encoding="utf-8"))

    errori = struttura(documento)
    if not errori:
        errori = esegui(documento)

    for messaggio in errori:
        print(messaggio, file=sys.stderr)
    if errori:
        return 1

    bloccanti = debito(documento)
    totali = len(documento["invarianti"])
    if opzioni.release:
        mancate: list[str] = []
        for voce in bloccanti:
            print(f"{voce['id']}: {voce['manca']}", file=sys.stderr)
        if bloccanti:
            mancate.append(
                f"{len(bloccanti)} invarianti su {totali} restano bloccanti"
            )

        # Le condizioni sono **congiunte**, e due non sono invarianti di questo
        # registro: contare i bloccanti non basterebbe. In particolare
        # `release_authorized` e' una decisione scritta, non l'esito automatico
        # di caselle verdi — ed e' l'unica che nessun gate puo' derivare.
        stato = ROOT / "assurance" / "current-state.json"
        if not stato.exists():
            mancate.append(f"{stato}: fonte strutturata dello stato assente")
        elif json.loads(stato.read_text(encoding="utf-8")).get("release_authorized") is not True:
            mancate.append(
                "`release_authorized` non e' true in assurance/current-state.json: "
                "e' una decisione scritta, e non e' stata presa"
            )

        if mancate:
            print("", file=sys.stderr)
            print("release non autorizzabile:", file=sys.stderr)
            for motivo in mancate:
                print(f"  - {motivo}", file=sys.stderr)
            print("", file=sys.stderr)
            print(
                "Le condizioni sono congiunte: nessuna implica le altre, e un "
                "verde parziale non e' un verde.",
                file=sys.stderr,
            )
            return 1
        print(f"contratto corrente: {totali} invarianti, nessun blocco.")
        return 0

    print(
        f"contratto corrente coerente: {totali} invarianti, "
        f"{totali - len(bloccanti)} verificati, {len(bloccanti)} bloccanti."
    )
    print("  Le prove dei verificati sono state ESEGUITE: gate con exit 0,")
    print("  test elencati dal harness una volta sola e passati. Non dice")
    print("  che la release sia autorizzabile: per quello serve --release.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
