#!/usr/bin/env python3
"""Firma e verifica l'entrypoint Windows con Azure Trusted Signing.

# Che cosa riceve, e che cosa non riceve mai

Nessun segreto. Il workflow si autentica ad Azure con l'identita' federata
OIDC di GitHub Actions (`azure/login`), e il client di Trusted Signing -- la
`dlib` che SignTool carica -- prende il token dalla sessione della CLI di
Azure. Questo modulo riceve soltanto tre percorsi e un'identita' attesa, dalle
variabili d'ambiente che il workflow prepara:

* `PLENORA_WINDOWS_SIGNTOOL`: SignTool x64 del Windows SDK del runner;
* `PLENORA_TRUSTED_SIGNING_DLIB`: `Azure.CodeSigning.Dlib.dll` del pacchetto
  fissato in `scripts/trusted-signing-lock.json`;
* `PLENORA_TRUSTED_SIGNING_METADATA`: il JSON con endpoint, account e profilo;
* `PLENORA_WINDOWS_FIRMATARIO_ATTESO`: il soggetto esatto del certificato che
  il profilo emette.

# Che cosa si firma

Soltanto `bin/plenora-io.exe`. Le DLL di terzi conservano la propria identita':
rifirmarle come Plenora attribuirebbe al progetto byte che ha solo
ridistribuito. L'entrypoint e' il file che l'utente avvia e quello su cui
Windows applica Authenticode e SmartScreen.

# Perche' il soggetto e non l'impronta

I certificati di Trusted Signing durano tre giorni e si rinnovano: l'impronta
cambia, e fissarla romperebbe la firma al primo rinnovo. Si misura e si scrive
nel manifesto; l'identita' che si confronta e' il soggetto, che il profilo
mantiene stabile.

# Quando non fa niente

Quando la politica non pretende la firma (`PLENORA_FIRMA_WINDOWS` assente o
`nessuna`, oppure canale di prova): non legge l'ambiente della firma, non
chiama strumenti, e rende lo stato `non_richiesta`.
"""

from __future__ import annotations

import hashlib
import json
import os
import pathlib
import subprocess
import sys
from collections.abc import Callable, Mapping

import distribuzione


URL_TIMESTAMP = "http://timestamp.acs.microsoft.com"
VAR_SIGNTOOL = "PLENORA_WINDOWS_SIGNTOOL"
VAR_DLIB = "PLENORA_TRUSTED_SIGNING_DLIB"
VAR_METADATA = "PLENORA_TRUSTED_SIGNING_METADATA"
VAR_FIRMATARIO = "PLENORA_WINDOWS_FIRMATARIO_ATTESO"

# Le chiavi del file di metadati del client, e nessun'altra: una chiave in piu'
# -- un segreto incollato per errore -- non deve arrivare al client.
CHIAVI_METADATI = {"Endpoint", "CodeSigningAccountName", "CertificateProfileName", "ExcludeCredentials"}


def _sha256(percorso: pathlib.Path) -> str:
    return hashlib.sha256(percorso.read_bytes()).hexdigest()


def _file_richiesto(env: Mapping[str, str], variabile: str) -> pathlib.Path:
    percorso = pathlib.Path(env.get(variabile, ""))
    if not env.get(variabile) or not percorso.is_file():
        raise SystemExit(
            f"{variabile} non indica un file esistente: il workflow deve preparare "
            "il firmatario prima di costruire la candidate"
        )
    return percorso


def _metadati_validi(percorso: pathlib.Path) -> None:
    try:
        dati = json.loads(percorso.read_text(encoding="utf-8"))
    except (OSError, ValueError) as errore:
        raise SystemExit(f"{VAR_METADATA}: il file non e' JSON ({type(errore).__name__})") from None
    if not isinstance(dati, dict):
        raise SystemExit(f"{VAR_METADATA}: il file non e' un oggetto JSON")
    estranee = sorted(set(dati) - CHIAVI_METADATI)
    if estranee:
        raise SystemExit(f"{VAR_METADATA}: chiavi non previste {estranee}")
    for chiave in ("Endpoint", "CodeSigningAccountName", "CertificateProfileName"):
        if not isinstance(dati.get(chiave), str) or not dati[chiave].strip():
            raise SystemExit(f"{VAR_METADATA}: manca {chiave}")
    if not dati["Endpoint"].startswith("https://"):
        raise SystemExit(f"{VAR_METADATA}: l'endpoint non e' https")


def applica(
    percorso: pathlib.Path,
    canale: str,
    misura_della_firma: Callable[[pathlib.Path], dict],
    *,
    ambiente: Mapping[str, str] | None = None,
    esecutore: Callable[..., subprocess.CompletedProcess] = subprocess.run,
    piattaforma: str | None = None,
) -> dict:
    """Appone Authenticode quando la politica lo pretende, e rende lo stato misurato.

    Ogni mancanza e' fatale: strumento, client, metadati, byte invariati,
    verifica nativa, identita' del firmatario e timestamp. Lo stato reso e'
    sempre `non_richiesta` o `apposta`: gli altri due non escono da qui.
    """
    env = os.environ if ambiente is None else ambiente
    politica = distribuzione.politica_di_firma("windows-x86_64", canale, dict(env))
    if not politica["richiesta"]:
        return distribuzione.stato_della_firma("windows-x86_64", canale, ambiente=dict(env))

    sistema = sys.platform if piattaforma is None else piattaforma
    if sistema != "win32":
        raise SystemExit("una candidate Windows si firma e si verifica su Windows")

    sign_tool = _file_richiesto(env, VAR_SIGNTOOL)
    dlib = _file_richiesto(env, VAR_DLIB)
    metadati = _file_richiesto(env, VAR_METADATA)
    _metadati_validi(metadati)
    atteso = env.get(VAR_FIRMATARIO, "").strip()
    if not atteso:
        raise SystemExit(
            f"{VAR_FIRMATARIO} e' vuota: senza l'identita' attesa una firma valida "
            "di chiunque passerebbe"
        )
    if not percorso.is_file():
        raise SystemExit("entrypoint da firmare assente")

    prima = _sha256(percorso)
    esecutore(
        [
            str(sign_tool),
            "sign",
            "/v",
            "/fd",
            "SHA256",
            "/tr",
            URL_TIMESTAMP,
            "/td",
            "SHA256",
            "/dlib",
            str(dlib),
            "/dmdf",
            str(metadati),
            str(percorso),
        ],
        check=True,
    )
    if _sha256(percorso) == prima:
        raise SystemExit("SignTool ha restituito successo senza cambiare l'entrypoint")

    # Due superfici di Windows che devono accettare gli stessi byte: SignTool
    # con la politica Authenticode, poi la misura strutturata.
    esecutore([str(sign_tool), "verify", "/pa", "/all", str(percorso)], check=True)
    misura = misura_della_firma(percorso)
    if (misura.get("firmatario") or "") != atteso:
        raise SystemExit(
            "la firma valida non porta il soggetto atteso: il profilo di Trusted Signing "
            f"o {VAR_FIRMATARIO} non sono quelli giusti"
        )
    stato = distribuzione.stato_della_firma(
        "windows-x86_64", canale, misura=misura, ambiente=dict(env)
    )
    if stato["stato"] != "apposta":
        raise SystemExit(
            "la firma Authenticode non soddisfa il contratto della candidate: "
            f"stato {stato['stato']}, mancanti {stato['mancanti']}"
        )
    return stato
