"""Gli errori dell'SDK, e quello che viene dalla busta.

# Due famiglie, e la ragione per cui restano separate

`PlenoraError` e' la radice, e sotto ci sono due cose diverse:

* `BinaryNotFound`, `ManifestError`, `ProfileError`, `ProtocolError` sono
  dell'**SDK**: nascono prima che il comando parta, o quando cio' che torna non
  e' cio' che il protocollo promette. Nessuna di loro ha una busta dietro;
* `CommandFailed` porta la busta d'errore `plenora-io-error-v1`, con i suoi
  quattro assi -- categoria, fase, effetto remoto, disposizione al ritentativo
  -- e il codice d'uscita.

Confonderle costringerebbe chi scrive un `except` a distinguere per messaggio
«il binario non c'e'» da «il file e' malformato», che sono due problemi di due
persone diverse.

# Perche' gli assi arrivano interi

`CommandFailed` non riassume: espone `category`, `phase`, `remote_effect` e
`retry` come li scrive il wire. Un SDK che li appiattisse in un messaggio
toglierebbe a chi lo usa la sola informazione **machine-readable** che la busta
porta, e lo costringerebbe a leggere le stringhe che noi ci riserviamo di
riscrivere.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any


class PlenoraError(Exception):
    """La radice: un `except PlenoraError` prende tutto quel che l'SDK solleva."""


class BinaryNotFound(PlenoraError):
    """Il binario `plenora-io` non e' stato trovato.

    Porta i posti in cui l'SDK ha cercato, in ordine. Un messaggio che dicesse
    soltanto «non trovato» lascerebbe indovinare se la variabile d'ambiente sia
    stata letta, se il `PATH` sia quello giusto, se il nome sia quello atteso.
    """

    def __init__(self, searched: list[str]) -> None:
        self.searched = list(searched)
        posti = "\n".join(f"  - {dove}" for dove in self.searched)
        super().__init__(
            "il binario `plenora-io` non e' stato trovato. Cercato, in ordine:\n"
            f"{posti}\n"
            "L'SDK non lo scarica: indica il percorso con Client(binary=...) o "
            "con la variabile d'ambiente PLENORA_IO_BIN."
        )


class ManifestError(PlenoraError):
    """Il `MANIFEST.json` dell'artefatto c'e' e non si puo' leggere.

    Distinta dall'assenza, che non e' un errore: un binario costruito da
    `cargo` non ha un manifesto e resta perfettamente usabile. Un manifesto
    presente e illeggibile e' un'altra cosa -- l'artefatto e' rotto -- e
    trattarlo come assente nasconderebbe il guasto.
    """


class ProfileError(PlenoraError):
    """L'artefatto non ha il profilo che il chiamante pretende.

    Sollevata **prima** di eseguire: un profilo `base` non ha il backend GDAL, e
    scoprirlo dal fallimento di una conversione a meta' costa un file di uscita
    parziale e un errore che parla di un driver invece che di un pacchetto.
    """

    def __init__(self, required: str, actual: str | None) -> None:
        self.required = required
        self.actual = actual
        quale = f"«{actual}»" if actual else "sconosciuto: nessun manifesto"
        super().__init__(
            f"questo artefatto ha profilo {quale} e ne serve «{required}». "
            "I profili si scelgono al momento di installare, non a runtime."
        )


class ProtocolError(PlenoraError):
    """Cio' che il binario ha risposto non e' cio' che il protocollo dichiara.

    Un JSON che non si decodifica, un `contract` inatteso, un campo obbligatorio
    che non c'e'. E' fail-closed per scelta: un SDK che tirasse a indovinare i
    campi mancanti trasformerebbe l'incompatibilita' di versione in dati
    sbagliati piu' avanti, dove nessuno la riconosce piu'.
    """


@dataclass(frozen=True)
class ErrorEnvelope:
    """La busta `plenora-io-error-v1`, con i suoi campi obbligatori.

    `row_diagnostics` e' il settimo campo, e c'e' solo quando l'errore porta la
    diagnostica riga per riga. Resta un dizionario grezzo: ha un contratto
    proprio -- `plenora-row-diagnostics-v1` -- e modellarlo qui vorrebbe dire
    ratificare in questo ciclo una superficie che non e' stata censita per
    l'SDK.
    """

    code: str
    category: str
    phase: str
    remote_effect: str
    retry: dict[str, Any]
    message: str
    row_diagnostics: dict[str, Any] | None = None

    @classmethod
    def from_json(cls, documento: dict[str, Any]) -> "ErrorEnvelope":
        errore = documento.get("error")
        if not isinstance(errore, dict):
            raise ProtocolError(
                "busta d'errore senza l'oggetto `error`: "
                f"{sorted(documento)}"
            )
        mancanti = [
            campo
            for campo in ("code", "category", "phase", "remote_effect", "retry", "message")
            if campo not in errore
        ]
        if mancanti:
            raise ProtocolError(
                f"busta d'errore senza i campi obbligatori {mancanti}. "
                "`plenora-io-error-v1` ne dichiara sei, e ci sono sempre."
            )
        return cls(
            code=errore["code"],
            category=errore["category"],
            phase=errore["phase"],
            remote_effect=errore["remote_effect"],
            retry=errore["retry"],
            message=errore["message"],
            row_diagnostics=errore.get("row_diagnostics"),
        )


@dataclass
class CommandFailed(PlenoraError):
    """Il comando e' stato eseguito e ha risposto con una busta d'errore.

    Non e' un guasto dell'SDK: e' il prodotto che rifiuta, e il rifiuto e'
    un'informazione. `exit_code` sta accanto alla busta perche' la CLI lo usa
    per distinguere famiglie di rifiuti che la busta descrive in prosa.
    """

    envelope: ErrorEnvelope
    exit_code: int
    argv: list[str] = field(default_factory=list)

    def __post_init__(self) -> None:
        super().__init__(
            f"`plenora-io {' '.join(self.argv)}` e' uscito con {self.exit_code}: "
            f"[{self.envelope.category}/{self.envelope.phase}] "
            f"{self.envelope.code}: {self.envelope.message}"
        )

    @property
    def retryable(self) -> bool:
        """`retry.kind` diverso da `never`.

        Una comodita', non una politica: **quanto** aspettare lo dice
        `envelope.retry`, che porta `delay_ms` quando il tipo e' `after`.
        """
        return self.envelope.retry.get("kind") != "never"
