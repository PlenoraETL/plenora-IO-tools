"""Il client: trova il binario, lo esegue, decodifica la busta.

# Perche' la busta si cerca su stdout **e poi** stderr

Il protocollo le colloca cosi': il successo su stdout, l'errore su stderr. Il
client legge nell'ordine che il protocollo dichiara invece di indovinare dal
codice d'uscita, perche' il codice d'uscita distingue famiglie di rifiuti e non
dice quale flusso porti il documento.

# Perche' nessun comando ha un timeout predefinito

`convert` legge e scrive file, e quanto ci metta dipende da quanto sono grandi:
un timeout scelto da noi sarebbe un limite arbitrario travestito da difesa, e
scatterebbe sul lavoro grosso invece che sul guasto. Il parametro c'e', e chi
sa quanto puo' durare il proprio lavoro lo imposta.

# Perche' l'ambiente si passa intero

`subprocess` eredita l'ambiente del processo, e va bene: la CLI ne legge poco e
quel poco -- `PROJ_DATA`, per esempio -- e' quello che chi installa
l'artefatto ha configurato. Ripulirlo qui romperebbe installazioni che
funzionano, per una difesa che l'SDK non e' il posto giusto per fare.
"""

from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path
from typing import Any

from .discovery import Manifest, leggi_manifesto, trova_binario, verifica_profilo
from .errors import CommandFailed, ErrorEnvelope, ProtocolError
from .models import Catalog, Version


class Client:
    """L'ingresso dell'SDK.

    La scoperta avviene nel costruttore, non alla prima chiamata: un client che
    esiste e' un client che ha trovato il proprio binario, e chi lo costruisce
    scopre subito che manca invece di scoprirlo a meta' di un lavoro.
    """

    def __init__(
        self,
        binary: str | os.PathLike[str] | None = None,
        *,
        timeout: float | None = None,
    ) -> None:
        self._binary = trova_binario(binary)
        self._timeout = timeout
        self._manifest = leggi_manifesto(self._binary)

    @property
    def binary(self) -> Path:
        return self._binary

    @property
    def manifest(self) -> Manifest | None:
        """Il manifesto dell'artefatto, o `None` se il binario non ne ha uno.

        `None` non e' un guasto: un binario costruito da `cargo` e' usabile e
        non porta un manifesto. Cio' che manca e' la capacita' di dire da quale
        artefatto venga -- profilo, canale, revisione -- e i metodi che ne hanno
        bisogno lo dicono invece di indovinare.
        """
        return self._manifest

    def require_profile(self, profile: str) -> None:
        """Solleva `ProfileError` se l'artefatto non ha quel profilo.

        Da chiamare **prima** del lavoro. Il driver FileGDB vuole il profilo
        `filegdb`, e scoprirlo dal fallimento di una conversione a meta' costa
        un file di uscita parziale e un errore che parla di un driver invece che
        di un pacchetto.
        """
        verifica_profilo(self._manifest, profile)

    # --- le due buste di questo ciclo -------------------------------------

    def version(self) -> Version:
        """La busta di bootstrap.

        E' la prima chiamata che ha senso fare: dice che binario si ha in mano,
        e lo dice senza pretendere di conoscere il protocollo.
        """
        return Version.from_json(self._esegui(["--version"]))

    def catalog(self) -> Catalog:
        """Il catalogo dei driver di **questa** installazione."""
        return Catalog.from_json(self._esegui(["catalog"]))

    # --- l'esecuzione ------------------------------------------------------

    def _esegui(self, argomenti: list[str]) -> dict[str, Any]:
        try:
            esito = subprocess.run(
                [str(self._binary), *argomenti],
                capture_output=True,
                text=True,
                encoding="utf-8",
                timeout=self._timeout,
                check=False,
            )
        except OSError as errore:
            raise ProtocolError(
                f"`{self._binary}` non si e' potuto eseguire: {errore}"
            ) from errore

        documento, flusso = self._decodifica(esito, argomenti)
        if documento.get("status") == "error" or flusso == "stderr":
            raise CommandFailed(
                envelope=ErrorEnvelope.from_json(documento),
                exit_code=esito.returncode,
                argv=list(argomenti),
            )
        if esito.returncode != 0:
            # Un codice diverso da zero con una busta di successo: il
            # protocollo non lo prevede, e passarlo oltre farebbe consumare
            # come buono un documento che il prodotto non ha dichiarato tale.
            raise ProtocolError(
                f"`plenora-io {' '.join(argomenti)}` e' uscito con "
                f"{esito.returncode} e ha scritto su stdout una busta di "
                f"stato «{documento.get('status')}»: il protocollo non "
                "prevede questa combinazione."
            )
        return documento

    @staticmethod
    def _decodifica(
        esito: subprocess.CompletedProcess[str], argomenti: list[str]
    ) -> tuple[dict[str, Any], str]:
        for flusso, testo in (("stdout", esito.stdout), ("stderr", esito.stderr)):
            if not testo.strip():
                continue
            try:
                documento = json.loads(testo)
            except json.JSONDecodeError:
                continue
            if isinstance(documento, dict):
                return documento, flusso
        raise ProtocolError(
            f"`plenora-io {' '.join(argomenti)}` e' uscito con "
            f"{esito.returncode} senza una busta JSON su nessuno dei due "
            f"flussi.\nstdout: {esito.stdout[:200]!r}\n"
            f"stderr: {esito.stderr[:200]!r}"
        )
