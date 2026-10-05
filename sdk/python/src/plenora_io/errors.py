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

import json
import math
from collections.abc import Mapping
from dataclasses import dataclass
from types import MappingProxyType
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

    `row_diagnostics` c'e' solo quando l'errore porta la diagnostica riga per
    riga, e sul filo vive dentro `details`: ROW-DIAGNOSTICS-1.0 dice che «when
    the enclosing serialized error uses `error-v1.schema.json`, the complete
    document is placed at `details.row_diagnostics`». Per un periodo la CLI lo
    scriveva al primo livello dell'errore, dove rendeva il documento invalido
    contro quello schema; l'attributo qui resta piatto perche' e' la comodita'
    che serve a chi lo legge, e la posizione sul filo la sa questa classe.

    Resta un documento grezzo: ha un contratto proprio --
    `plenora-row-diagnostics-v1` -- e modellarlo qui vorrebbe dire ratificare
    in questo ciclo una superficie che non e' stata censita per l'SDK.

    # Di sola lettura, e copiati

    `retry` e `row_diagnostics` sono **copie** congelate di cio' che si e'
    passato -- `MappingProxyType` per gli oggetti, tuple per gli elenchi -- e
    non il dizionario del chiamante. Prima la busta conservava quel
    dizionario: validata con `remote_effect: unknown` e `retry: {kind: never}`,
    bastava cambiare dopo il dizionario in `{kind: safe}` perche' `retryable`
    diventasse vero senza nessuna nuova validazione. Ora cio' che si valida e'
    cio' che resta.
    """

    code: str
    category: str
    phase: str
    remote_effect: str
    retry: Mapping[str, Any]
    message: str
    row_diagnostics: Mapping[str, Any] | None = None

    def __post_init__(self) -> None:
        # La validazione sta **qui** e non solo in `from_json`: una busta
        # costruita a mano -- in un test, in un adattatore -- arriva alle stesse
        # proprieta' di `CommandFailed`, e un valore fuori vocabolario deve
        # fermarsi prima di diventare una risposta su «ritentare e' sicuro».
        #
        # Prima si copia, poi si valida la copia, poi si conserva la copia: la
        # cosa validata e la cosa conservata sono lo stesso oggetto, e nessuno
        # fuori ne ha un riferimento.
        if _tipo(self.retry) != "object":
            raise ProtocolError(
                f"`error.retry` e' {_tipo(self.retry)} e non un oggetto `{{kind}}`."
            )
        object.__setattr__(self, "retry", copia_json(self.retry, "error.retry", congela=True))
        if self.row_diagnostics is not None:
            if _tipo(self.row_diagnostics) != "object":
                raise ProtocolError(
                    f"`row_diagnostics` e' {_tipo(self.row_diagnostics)} e non un oggetto."
                )
            object.__setattr__(
                self,
                "row_diagnostics",
                copia_json(self.row_diagnostics, "row_diagnostics", congela=True),
            )
        _valida_busta(self)

    @classmethod
    def from_json(cls, documento: dict[str, Any]) -> "ErrorEnvelope":
        if _tipo(documento) != "object":
            raise ProtocolError(
                f"la busta d'errore e' {_tipo(documento)} e non un oggetto."
            )
        errore = documento.get("error")
        if _tipo(errore) != "object":
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
        # `details` e' facoltativo, ma quando c'e' e' un oggetto: lo schema
        # `error-v1` non ammette `null`. Prima un `null` diventava `{}` in
        # silenzio mentre una stringa sollevava `AttributeError` -- due esiti
        # diversi per lo stesso difetto, nessuno dei due nominato.
        if "details" in errore and _tipo(errore["details"]) != "object":
            raise ProtocolError(
                f"`error.details` e' {_tipo(errore['details'])} e non un oggetto: "
                "lo schema `plenora-error-v1` lo dichiara oggetto quando c'e'."
            )
        dettagli = errore.get("details", {})
        if "row_diagnostics" in dettagli and _tipo(dettagli["row_diagnostics"]) != "object":
            raise ProtocolError(
                "`error.details.row_diagnostics` e' "
                f"{_tipo(dettagli['row_diagnostics'])} e non un oggetto."
            )
        return cls(
            code=errore["code"],
            category=errore["category"],
            phase=errore["phase"],
            remote_effect=errore["remote_effect"],
            retry=errore["retry"],
            message=errore["message"],
            row_diagnostics=dettagli.get("row_diagnostics"),
        )


# --- i due assi su cui si decide se ripetere ---------------------------------
#
# `remote_effect` e `retry.kind` sono **vocabolari chiusi** di
# `plenora-error-v1` (`plenora-contracts/schemas/error-v1.schema.json`), e
# sono gli unici due assi della busta da cui chi usa l'SDK ricava un'azione:
# ripetere o no, verificare lo stato remoto o no. Un valore assente, `null` o
# sconosciuto non ha una risposta sicura, e prima ne riceveva una **insicura**:
# `retry.get("kind") != "never"` diceva «ritentabile» per un `kind` mancante, e
# `remote_effect in ("committed", "unknown")` diceva «nessun effetto remoto da
# temere» per un valore che non conosceva. Ora sono `ProtocolError`.
#
# La categoria resta invece **aperta**: `failure_from_envelope` ripiega su
# `CommandFailed` per una categoria nuova, perche' la categoria sceglie solo la
# classe dell'eccezione e il ripiego non fa decidere niente di sbagliato a
# nessuno. Su questi due assi un ripiego deciderebbe, e un'estensione del
# vocabolario va accompagnata da un SDK che la conosce.

#: I valori di `remote_effect` in `plenora-error-v1`.
EFFETTI_REMOTI: frozenset[str] = frozenset(
    {"none", "rolled_back", "partial", "committed", "unknown"}
)

#: Gli effetti remoti dopo i quali dall'altra parte non resta niente: gli
#: unici per cui un ritentativo cieco non rifa' un lavoro gia' fatto.
EFFETTI_SENZA_RESIDUO: frozenset[str] = frozenset({"none", "rolled_back"})

#: I valori di `retry.kind` in `plenora-error-v1`. `quarantine` non lo emette
#: questo componente (`RetryDisposition` non lo ha), ma il contratto e'
#: condiviso: e' un valore valido, e trattarlo da ignoto rifiuterebbe una busta
#: corretta.
TIPI_DI_RITENTATIVO: frozenset[str] = frozenset(
    {
        "never",
        "quarantine",
        "safe",
        "requires_idempotency_key",
        "requires_recovery",
        "after",
    }
)

#: I tipi per cui `CommandFailed.retryable` e' vero. Sono quelli di prima --
#: tutto tranne `never` -- meno `quarantine`, che chiede di **isolare** il
#: lavoro e non di ripeterlo.
RITENTABILI: frozenset[str] = TIPI_DI_RITENTATIVO - {"never", "quarantine"}

#: Con `remote_effect: unknown` lo schema ammette soltanto questi tipi: non
#: sapendo che cosa e' successo di la', un ritentativo «sicuro» o «dopo un
#: ritardo» sarebbe una contraddizione nella stessa busta.
RITENTATIVI_CON_EFFETTO_IGNOTO: frozenset[str] = frozenset(
    {"never", "quarantine", "requires_recovery"}
)

#: Il tetto di `retry.delay_ms` nello schema: un giorno.
RITARDO_MASSIMO_MS = 86_400_000


#: I tipi Python che `json.loads` produce, e il tipo JSON di ciascuno.
#:
#: Il confronto e' sul tipo **esatto**, non con `isinstance`. Una sottoclasse
#: di `str` con `__eq__` e `__hash__` riscritti superava `in EFFETTI_REMOTI`
#: fingendosi `none`; una di `dict` con `get` riscritto rispondeva a `kind` con
#: un valore che non aveva. Il wire non le produce mai: chi le passa non sta
#: passando JSON, e la risposta giusta e' rifiutarle, non interpretarle.
_TIPI_JSON: dict[type, str] = {
    type(None): "null",
    bool: "boolean",
    int: "integer",
    float: "number",
    str: "string",
    list: "array",
    dict: "object",
}


def _tipo(valore: Any) -> str:
    """Il tipo JSON **esatto** di un valore, per controlli e messaggi.

    Mai il valore stesso. Per cio' che non e' un tipo prodotto da `json.loads`
    -- sottoclassi comprese -- rende un nome che nessun controllo accetta.
    """
    return _TIPI_JSON.get(type(valore), f"non JSON ({type(valore).__name__})")


def copia_json(valore: Any, dove: str, *, congela: bool = False) -> Any:
    """Una copia profonda di un valore JSON, con i soli tipi di `json.loads`.

    Rifiuta con `ProtocolError` ogni tipo non JSON a qualunque profondita',
    chiavi comprese (solo `str` esatte), e i numeri non finiti. Con `congela`
    gli oggetti diventano `MappingProxyType` e gli elenchi tuple: nessuno, ne'
    il chiamante ne' chi riceve la copia, puo' cambiarla dopo.
    """
    tipo = _tipo(valore)
    if tipo == "object":
        copia = {}
        for chiave, interno in valore.items():
            if type(chiave) is not str:
                raise ProtocolError(
                    f"`{dove}` ha una chiave {_tipo(chiave)} e non una stringa."
                )
            copia[chiave] = copia_json(interno, f"{dove}.{chiave}", congela=congela)
        return MappingProxyType(copia) if congela else copia
    if tipo == "array":
        copia = [
            copia_json(interno, f"{dove}[{posizione}]", congela=congela)
            for posizione, interno in enumerate(valore)
        ]
        return tuple(copia) if congela else copia
    if tipo == "number" and not math.isfinite(valore):
        raise ProtocolError(f"`{dove}` e' un numero non finito, che JSON non ha.")
    if tipo.startswith("non JSON"):
        raise ProtocolError(f"`{dove}` e' {tipo}.")
    return valore


def _rifiuta_costante(nome: str) -> Any:
    raise ProtocolError(f"il documento contiene `{nome}`, che JSON non ha.")


def _rifiuta_chiavi_doppie(coppie: list[tuple[str, Any]]) -> dict[str, Any]:
    oggetto: dict[str, Any] = {}
    for chiave, valore in coppie:
        if chiave in oggetto:
            raise ProtocolError(
                "il documento ripete una chiave nello stesso oggetto: "
                "`json.loads` terrebbe in silenzio l'ultima."
            )
        oggetto[chiave] = valore
    return oggetto


def carica_json(testo: str) -> Any:
    """`json.loads` senza le sue due tolleranze.

    `NaN`, `Infinity` e `-Infinity` non sono JSON, e `json.loads` li accetta;
    una chiave ripetuta nello stesso oggetto la risolve tenendo l'ultima. Sono
    due modi in cui un documento diverso da quello scritto arriverebbe ai
    modelli senza che nessuno lo veda: qui sono `ProtocolError`. Gli errori di
    sintassi restano `json.JSONDecodeError`, come prima.
    """
    return json.loads(
        testo,
        object_pairs_hook=_rifiuta_chiavi_doppie,
        parse_constant=_rifiuta_costante,
    )


def _valida_busta(busta: "ErrorEnvelope") -> None:
    """Tipi e vocabolari chiusi della busta, o `ProtocolError`.

    I messaggi nominano il campo e il tipo trovato, non il valore: il valore
    viene da un processo esterno e non c'e' ragione di ricopiarlo.
    """
    for campo in ("code", "category", "phase", "message"):
        valore = getattr(busta, campo)
        if _tipo(valore) != "string" or not valore:
            raise ProtocolError(
                f"`error.{campo}` e' {_tipo(valore)} e non una stringa non vuota."
            )

    effetto = busta.remote_effect
    if _tipo(effetto) != "string" or effetto not in EFFETTI_REMOTI:
        raise ProtocolError(
            f"`error.remote_effect` ({_tipo(effetto)}) non e' nel vocabolario "
            f"chiuso di `plenora-error-v1`: {sorted(EFFETTI_REMOTI)}. Senza "
            "sapere che cosa e' successo dall'altra parte nessuna risposta su "
            "un ritentativo e' sicura."
        )

    # `retry` e' gia' la copia congelata fatta in `__post_init__`.
    retry = busta.retry
    tipo = retry.get("kind")
    if _tipo(tipo) != "string" or tipo not in TIPI_DI_RITENTATIVO:
        raise ProtocolError(
            f"`error.retry.kind` ({_tipo(tipo)}) non e' nel vocabolario chiuso "
            f"di `plenora-error-v1`: {sorted(TIPI_DI_RITENTATIVO)}."
        )
    attese = {"kind", "delay_ms"} if tipo == "after" else {"kind"}
    if set(retry) != attese:
        raise ProtocolError(
            f"`error.retry` di tipo «{tipo}» ha le chiavi {sorted(retry)}, lo "
            f"schema ne pretende esattamente {sorted(attese)}."
        )
    if tipo == "after":
        ritardo = retry["delay_ms"]
        if _tipo(ritardo) != "integer" or not 0 <= ritardo <= RITARDO_MASSIMO_MS:
            raise ProtocolError(
                f"`error.retry.delay_ms` ({_tipo(ritardo)}) non e' un intero fra "
                f"0 e {RITARDO_MASSIMO_MS}."
            )
    if effetto == "unknown" and tipo not in RITENTATIVI_CON_EFFETTO_IGNOTO:
        raise ProtocolError(
            f"`error.remote_effect` e' «unknown» e `error.retry.kind` e' "
            f"«{tipo}»: lo schema ammette soltanto "
            f"{sorted(RITENTATIVI_CON_EFFETTO_IGNOTO)}, perche' un ritentativo "
            "non si dichiara sicuro senza sapere che cosa e' successo."
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
        """`retry.kind` fra i tipi che ammettono un nuovo tentativo.

        Sono `safe`, `after`, `requires_idempotency_key` e `requires_recovery`:
        non `never`, e non `quarantine`, che chiede di isolare il lavoro. Il
        tipo e' gia' stato validato contro il vocabolario chiuso, quindi un
        valore assente o ignoto non arriva qui: e' `ProtocolError` prima.

        Una comodita', non una politica: **quanto** aspettare lo dice
        `envelope.retry`, che porta `delay_ms` quando il tipo e' `after`, e le
        condizioni le dice il tipo stesso.
        """
        return self.envelope.retry["kind"] in RITENTABILI

    @property
    def retry_after_ms(self) -> int | None:
        """I millisecondi da attendere, quando la busta li dichiara.

        `None` non vuol dire «riprova subito»: vuol dire che il prodotto non ha
        detto quanto aspettare, e chi riprova sceglie da se'.
        """
        retry = self.envelope.retry
        return retry["delay_ms"] if retry["kind"] == "after" else None

    @property
    def must_assume_remote_committed(self) -> bool:
        """Un ritentativo cieco **non** e' sicuro: vada come deve andare.

        Vera per `committed`, dove il lavoro remoto e' andato a buon fine, per
        `unknown`, dove non si sa, e per `partial`, dove una parte e' andata a
        buon fine e ripetere da capo la rifarebbe. Falsa soltanto per `none` e
        `rolled_back`, gli unici due stati in cui dall'altra parte non resta
        niente. Un valore fuori vocabolario non arriva qui: e' `ProtocolError`
        quando la busta si costruisce. Le due cose non sono la stessa, e il nome non
        dice che lo siano: dice che chi deve decidere se ripetere l'operazione
        deve comportarsi allo stesso modo in entrambi i casi, perche'
        l'alternativa e' rifare un lavoro gia' fatto.

        Si chiamava `remote_committed`, e affermava una cosa che nessuno sa:
        davanti a `unknown` restituiva `True` come se il commit fosse
        accertato. Chi la leggeva imparava dal nome un fatto sbagliato, e chi
        avesse voluto **distinguere** i due stati avrebbe dovuto scoprire da se'
        che il nome non li distingueva.

        `envelope.remote_effect` resta intatto, ed e' li' che si guarda quando
        la differenza fra «commesso» e «ignoto» conta -- per esempio per
        decidere se **verificare** lo stato remoto invece di riprovare.
        """
        return self.envelope.remote_effect not in EFFETTI_SENZA_RESIDUO


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
