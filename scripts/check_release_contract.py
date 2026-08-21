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

* senza argomenti — il **registro e' coerente** e ogni prova dichiarata
  risolve. Non dice che la release sia autorizzabile, e lo stampa;
* `--release` — **rossa** finche' esiste una voce `release_blocking`.

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
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent
REGISTRO = ROOT / "assurance" / "registries" / "release-contract-current.json"
CLI_PROTOCOL_V1 = ROOT / "release" / "cli-protocol-v1.json"

STATI = {"verified", "release_blocking"}
CAMPI = {"id", "superficie", "invariante", "prova", "stato"}


def _percorsi(valore: Any) -> list[str]:
    if valore is None:
        return []
    if isinstance(valore, str):
        return [valore]
    return list(valore)


def verifica_registro(documento: dict[str, Any]) -> list[str]:
    """Il registro e' coerente e ogni prova dichiarata risolve."""
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
        if stato == "verified":
            if not prova:
                errori.append(
                    f"{identita}: `verified` senza prova. Un invariante senza "
                    "verifica corrente e' `release_blocking`, non una verita'."
                )
                continue
            if not voce.get("invariante"):
                errori.append(f"{identita}: `verified` senza invariante scritto")
            for chiave in ("comando", "artefatto"):
                for relativo in _percorsi(prova.get(chiave)):
                    percorso = ROOT / relativo.split()[0]
                    if not percorso.exists():
                        errori.append(
                            f"{identita}: la prova «{relativo}» non esiste. "
                            "Una prova che sopravvive al proprio strumento "
                            "verifica un invariante che nessuno controlla."
                        )
            if prova.get("tipo") not in {"gate", "test"}:
                errori.append(f"{identita}: tipo di prova non ammesso")
        else:
            # Un bloccante **puo'** avere una prova: sono due casi diversi, e
            # confonderli farebbe sparire il piu' interessante.
            #
            #   * il meccanismo di verifica esiste e **oggi fallisce** — e' il
            #     caso di ASSURANCE-N1, dove il gate c'e' ed e' rosso;
            #   * nessuna verifica esiste — e' il caso delle lacune fuzz e del
            #     contratto dei report di perdita.
            #
            # Cio' che entrambi devono avere e' `manca`: un blocco senza la sua
            # ragione non si puo' chiudere, perche' nessuno sa che cosa
            # servirebbe.
            for chiave in ("comando", "artefatto"):
                for relativo in _percorsi((prova or {}).get(chiave)):
                    if not (ROOT / relativo.split()[0]).exists():
                        errori.append(
                            f"{identita}: la prova «{relativo}» non esiste"
                        )
            if not voce.get("manca"):
                errori.append(
                    f"{identita}: `release_blocking` senza campo `manca`. Un "
                    "blocco senza la sua ragione non si puo' chiudere."
                )
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

    errori = verifica_registro(documento)
    if CLI_PROTOCOL_V1.exists():
        errori.extend(
            validate_cli_protocol_v1(json.loads(CLI_PROTOCOL_V1.read_text(encoding="utf-8")))
        )
    else:
        errori.append(f"{CLI_PROTOCOL_V1}: manifesto del protocollo CLI assente")

    for messaggio in errori:
        print(messaggio, file=sys.stderr)
    if errori:
        return 1

    bloccanti = debito(documento)
    totali = len(documento["invarianti"])
    if opzioni.release:
        if bloccanti:
            for voce in bloccanti:
                print(f"{voce['id']}: {voce['manca']}", file=sys.stderr)
            print(
                f"release non autorizzabile: {len(bloccanti)} invarianti su "
                f"{totali} restano bloccanti.",
                file=sys.stderr,
            )
            return 1
        print(f"contratto corrente: {totali} invarianti, nessun blocco.")
        return 0

    print(
        f"contratto corrente coerente: {totali} invarianti, "
        f"{totali - len(bloccanti)} verificati, {len(bloccanti)} bloccanti."
    )
    print("  Questo esito dice che il REGISTRO e' coerente e che ogni prova")
    print("  esiste, non che la release sia autorizzabile. Per quello serve")
    print("  --release.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
