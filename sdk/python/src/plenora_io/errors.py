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

from dataclasses import dataclass
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


class CommandFailed(PlenoraError):
    """Il comando e' stato eseguito e ha risposto con una busta d'errore.

    Non e' un guasto dell'SDK: e' il prodotto che rifiuta, e il rifiuto e'
    un'informazione. `exit_code` sta accanto alla busta perche' la CLI lo usa
    per distinguere famiglie di rifiuti che la busta descrive in prosa.

    Non si costruisce a mano: `failure_from_envelope` sceglie la sottoclasse
    dalla **categoria**, che e' un vocabolario chiuso del contratto.
    """

    def __init__(
        self,
        envelope: "ErrorEnvelope",
        exit_code: int,
        argv: list[str] | None = None,
    ) -> None:
        self.envelope = envelope
        self.exit_code = exit_code
        self.argv = list(argv or [])
        super().__init__(
            f"`plenora-io {' '.join(self.argv)}` e' uscito con {exit_code}: "
            f"[{envelope.category}/{envelope.phase}] "
            f"{envelope.code}: {envelope.message}"
        )

    @property
    def retryable(self) -> bool:
        """`retry.kind` diverso da `never`.

        Una comodita', non una politica: **quanto** aspettare lo dice
        `envelope.retry`, che porta `delay_ms` quando il tipo e' `after`.
        """
        return self.envelope.retry.get("kind") != "never"

    @property
    def retry_after_ms(self) -> int | None:
        """I millisecondi da attendere, quando la busta li dichiara.

        `None` non vuol dire «riprova subito»: vuol dire che il prodotto non ha
        detto quanto aspettare, e chi riprova sceglie da se'.
        """
        retry = self.envelope.retry
        return retry.get("delay_ms") if retry.get("kind") == "after" else None

    @property
    def remote_committed(self) -> bool:
        """Il lavoro remoto potrebbe essere andato a buon fine.

        Vero per `committed` **e** per `unknown`: chi non sa deve comportarsi
        come chi sa di si', perche' l'alternativa e' rifare un lavoro gia'
        fatto. Trattare `unknown` come `none` e' l'errore che questa proprieta'
        esiste per non far commettere.
        """
        return self.envelope.remote_effect in ("committed", "unknown")


# --- una classe per categoria, e la ragione per cui sono tante --------------
#
# La categoria e' un **vocabolario chiuso** del contratto, e chi usa l'SDK
# reagisce a quella: `except NotFoundError` e' cio' che si vuole scrivere, non
# un `if errore.envelope.category == "not_found"`. Le due forme dicono la stessa
# cosa; la prima la dice al lettore e la seconda al debugger.
#
# Il testo del messaggio, invece, non e' un asse su cui reagire: e' curato per
# chi legge, e ci riserviamo di riscriverlo. Un SDK che offrisse
# `if "non trovato" in str(errore)` inviterebbe a dipendere da una stringa che
# cambia senza preavviso, ed e' il motivo per cui questa gerarchia esiste.
#
# `scripts/check_sdk_python.py` confronta questo elenco con `ErrorCategory` del
# contratto: una categoria nuova senza classe, o una classe senza categoria,
# sono entrambe rosse.


class InvalidPlanError(CommandFailed):
    """`invalid_plan`: il piano di scrittura non e' coerente."""


class InvalidConfigurationError(CommandFailed):
    """`invalid_configuration`: opzioni, argomenti o percorsi non ammessi."""


class SchemaError(CommandFailed):
    """`schema`: lo schema dei dati non regge il contratto."""


class DataMappingError(CommandFailed):
    """`data_mapping`: un valore non si puo' rappresentare nel formato."""


class CrsError(CommandFailed):
    """`crs`: il sistema di riferimento manca, non si risolve o non si scrive."""


class UnsupportedError(CommandFailed):
    """`unsupported`: il prodotto non fa questa cosa, e il file va bene.

    Distinta da `SchemaError` per una ragione che costa: il primo dice che il
    file e' corretto e noi no, il secondo che il file e' sbagliato. Mandare chi
    legge a correggere un file corretto e' il danno che la distinzione evita.
    """


class NotFoundError(CommandFailed):
    """`not_found`: la sorgente, il layer o il campo non esistono."""


class ConflictError(CommandFailed):
    """`conflict`: la destinazione esiste, o una risorsa e' occupata."""


class AuthenticationError(CommandFailed):
    """`authentication`: le credenziali mancano o non valgono."""


class AuthorizationError(CommandFailed):
    """`authorization`: le credenziali valgono e non bastano."""


class TimeoutError(CommandFailed):  # noqa: A001 - il nome del contratto vince
    """`timeout`: il tempo e' scaduto.

    Ombreggia il `TimeoutError` incorporato dentro questo modulo, ed e'
    voluto: il nome viene dal vocabolario del contratto, e rinominarlo
    costringerebbe chi legge il contratto a tenere due parole per una cosa.
    Chi ha bisogno di quello di Python lo prende da `builtins`.
    """


class CancelledError(CommandFailed):
    """`cancelled`: qualcuno ha chiesto di fermarsi."""


class ResourceLimitError(CommandFailed):
    """`resource_limit`: un tetto dichiarato e' stato raggiunto.

    Non e' un guasto: e' una difesa che ha funzionato. Chi la incontra alza il
    tetto o riduce il lavoro, e in entrambi i casi decide -- che e' la ragione
    per cui questa categoria non sta con `execution`.
    """


class IoError(CommandFailed):
    """`io`: il filesystem o la rete hanno detto di no."""


class ProtocolViolationError(CommandFailed):
    """`protocol`: un contratto fra componenti e' stato violato.

    Il nome non e' `ProtocolError` perche' quello e' gia' preso, e le due cose
    sono diverse: `ProtocolError` e' dell'SDK -- la risposta non e' quella che
    il protocollo promette -- mentre questa e' del **prodotto**, che ha
    riconosciuto una violazione e l'ha riportata in una busta regolare.
    """


class TransientError(CommandFailed):
    """`transient`: e' andata male e potrebbe andare bene."""


class ExecutionError(CommandFailed):
    """`execution`: il lavoro e' fallito mentre lo si faceva."""


class InternalError(CommandFailed):
    """`internal`: un invariante nostro non ha retto. E' un difetto."""


#: Dalla categoria del wire alla classe. Le chiavi sono `ErrorCategory`
#: serializzata in snake_case, che e' come arriva nella busta.
CATEGORIE: dict[str, type[CommandFailed]] = {
    "invalid_plan": InvalidPlanError,
    "invalid_configuration": InvalidConfigurationError,
    "schema": SchemaError,
    "data_mapping": DataMappingError,
    "crs": CrsError,
    "unsupported": UnsupportedError,
    "not_found": NotFoundError,
    "conflict": ConflictError,
    "authentication": AuthenticationError,
    "authorization": AuthorizationError,
    "timeout": TimeoutError,
    "cancelled": CancelledError,
    "resource_limit": ResourceLimitError,
    "io": IoError,
    "protocol": ProtocolViolationError,
    "transient": TransientError,
    "execution": ExecutionError,
    "internal": InternalError,
}


def failure_from_envelope(
    documento: dict[str, Any], exit_code: int, argv: list[str]
) -> CommandFailed:
    """La busta d'errore, come eccezione della classe che le compete.

    Una categoria **sconosciuta** non e' un errore dell'SDK: le regole di
    compatibilita' del protocollo consentono di estendere un vocabolario
    chiuso, e un SDK che si rifiutasse di leggere la busta trasformerebbe
    un'estensione in un guasto. Si ripiega su `CommandFailed`, che porta la
    categoria intatta: chi la conosce la legge da `envelope.category`.
    """
    envelope = ErrorEnvelope.from_json(documento)
    classe = CATEGORIE.get(envelope.category, CommandFailed)
    return classe(envelope, exit_code, argv)
