#!/usr/bin/env python3
"""Firma e verifica l'entrypoint Windows, senza maneggiare il segreto del PFX.

Il PFX viene importato dal workflow nel certificate store dell'utente. Questo
modulo riceve soltanto l'impronta pubblica del certificato e il percorso di
SignTool: la password non entra nel costruttore, non finisce nella command line
e non puo' comparire nei suoi log.

Si firma soltanto ``bin/plenora-io.exe``. Le DLL di terzi conservano la propria
identita': rifirmarle come Plenora attribuirebbe al progetto byte che ha solo
ridistribuito. L'entrypoint e' il file che l'utente avvia e quello su cui
Windows applica Authenticode e SmartScreen.
"""

from __future__ import annotations

import hashlib
import os
import pathlib
import re
import subprocess
import sys
from collections.abc import Callable, Mapping

import distribuzione


URL_TIMESTAMP = "http://timestamp.digicert.com"
VAR_SIGNTOOL = "PLENORA_WINDOWS_SIGNTOOL"
VAR_CERTIFICATO = "PLENORA_WINDOWS_SIGNING_CERT_SHA1"


def _sha256(percorso: pathlib.Path) -> str:
    return hashlib.sha256(percorso.read_bytes()).hexdigest()


def _impronta(valore: str) -> str:
    normalizzata = re.sub(r"\s+", "", valore).upper()
    if not re.fullmatch(r"[0-9A-F]{40}", normalizzata):
        raise SystemExit(
            f"{VAR_CERTIFICATO} non e' un'impronta SHA-1 di certificato valida"
        )
    return normalizzata


def applica(
    percorso: pathlib.Path,
    canale: str,
    misura_della_firma: Callable[[pathlib.Path], dict],
    *,
    ambiente: Mapping[str, str] | None = None,
    esecutore: Callable[..., subprocess.CompletedProcess] = subprocess.run,
    piattaforma: str | None = None,
) -> dict:
    """Appone Authenticode su una candidate e restituisce lo stato misurato.

    Il canale di prova non consulta ambiente, certificate store o SignTool. Su
    una candidate ogni mancanza e' fatale: strumento, impronta, mutazione dei
    byte, validita' nativa, identita' del firmatario e timestamp.
    """
    if canale != "candidate":
        return distribuzione.stato_della_firma("windows-x86_64", canale)

    sistema = sys.platform if piattaforma is None else piattaforma
    if sistema != "win32":
        raise SystemExit("una candidate Windows si firma e si verifica su Windows")

    env = os.environ if ambiente is None else ambiente
    sign_tool = pathlib.Path(env.get(VAR_SIGNTOOL, ""))
    if not sign_tool.is_file():
        raise SystemExit(
            f"{VAR_SIGNTOOL} non indica un SignTool esistente: il workflow deve "
            "preparare il firmatario prima di costruire la candidate"
        )
    impronta = _impronta(env.get(VAR_CERTIFICATO, ""))
    if not percorso.is_file():
        raise SystemExit(f"entrypoint da firmare assente: {percorso}")

    prima = _sha256(percorso)
    comando = [
        str(sign_tool),
        "sign",
        "/sha1",
        impronta,
        "/s",
        "My",
        "/fd",
        "SHA256",
        "/tr",
        URL_TIMESTAMP,
        "/td",
        "SHA256",
        str(percorso),
    ]
    # Nessun valore segreto e' in questa command line: la chiave privata e'
    # nel certificate store e la password del PFX e' gia' uscita di scena.
    esecutore(comando, check=True)
    if _sha256(percorso) == prima:
        raise SystemExit("SignTool ha restituito successo senza cambiare l'entrypoint")

    # Una verifica indipendente prima della misura strutturata: entrambe
    # interrogano Windows, ma con due superfici diverse. Il manifesto verra'
    # scritto solo dopo che tutte e due hanno accettato i byte finali.
    esecutore([str(sign_tool), "verify", "/pa", "/all", str(percorso)], check=True)
    misura = misura_della_firma(percorso)
    letta = re.sub(r"\s+", "", misura.get("impronta_firmatario") or "").upper()
    if letta != impronta:
        raise SystemExit(
            "la firma valida non appartiene al certificato selezionato "
            f"(misurata {letta or 'nessuna impronta'})"
        )
    stato = distribuzione.stato_della_firma(
        "windows-x86_64", "candidate", misura=misura
    )
    if stato["stato"] != "apposta":
        raise SystemExit(
            "la firma Authenticode non soddisfa il contratto della candidate: "
            f"stato {stato['stato']}, mancanti {stato['mancanti']}"
        )
    return stato
