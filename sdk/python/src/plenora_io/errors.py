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
from dataclasses import dataclass
from typing import Any


class PlenoraError(Exception):
    """La radice: un `except PlenoraError` prende tutto quel che l'SDK solleva.

    # I cinque assi stanno su **ogni** eccezione pubblica

    PYTHON-SDK-1.0 §6 pretende che ogni eccezione pubblica renda leggibili,
    senza analizzare testo, `category`, `phase`, `remote_effect`, `retry` e
    `message`, con i valori e il significato di `error-v1.schema.json`. Fino
    alla 4.1.1 li portava soltanto `CommandFailed`, che li ha dalla busta; gli
    errori nati nell'SDK -- binario introvabile, manifesto illeggibile, profilo
    sbagliato, risposta fuori protocollo -- erano eccezioni con il solo testo.

    Qui gli assi hanno un valore per classe, scelto una volta e dichiarato
    accanto alla classe: `_CATEGORIA` e `_FASE`. L'effetto remoto parte da
    `none`; `ProtocolError` e' l'unico che lo puo' alzare, perche' e' l'unico
    che puo' nascere **dopo** che il processo e' partito (vedi `process.py`).
    Il ritentativo e' sempre `never`: un errore dell'SDK non diventa un
    successo ripetendo la stessa chiamata, e dire il contrario sarebbe una
    promessa che nessuno ha verificato.
    """

    #: La categoria di `error-v1` per gli errori nati nell'SDK.
    _CATEGORIA = "internal"
    #: La fase di `error-v1` in cui l'SDK li solleva.
    _FASE = "validate"

    @property
    def category(self) -> str:
        """La categoria di `plenora-error-v1`."""
        return self._CATEGORIA

    @property
    def phase(self) -> str:
        """La fase di `plenora-error-v1`."""
        return self._FASE

    @property
    def remote_effect(self) -> str:
        """L'effetto remoto di `plenora-error-v1`: `none` se nessuno l'ha alzato."""
        return getattr(self, "_effetto_remoto", None) or "none"

    @property
    def retry(self) -> dict[str, Any]:
        """La disposizione al ritentativo, come oggetto `{kind}` nuovo a ogni lettura."""
        return {"kind": "never"}

    @property
    def message(self) -> str:
        """Il messaggio curato, lo stesso di `str(errore)`."""
        return str(self)

    @property
    def code(self) -> str | None:
        """Il codice stabile, quando l'errore ne ha uno: gli errori dell'SDK no."""
        return None

    def to_dict(self) -> dict[str, Any]:
        """Il documento `plenora-error-v1` dell'errore, con i soli campi che ha."""
        documento: dict[str, Any] = {
            "category": self.category,
            "phase": self.phase,
            "remote_effect": self.remote_effect,
            "retry": self.retry,
            "message": self.message,
        }
        if self.code is not None:
            documento["code"] = self.code
        return documento


class BinaryNotFound(PlenoraError):
    """Il binario `plenora-io` non e' stato trovato.

    Porta i posti in cui l'SDK ha cercato, in ordine. Un messaggio che dicesse
    soltanto «non trovato» lascerebbe indovinare se la variabile d'ambiente sia
    stata letta, se il `PATH` sia quello giusto, se il nome sia quello atteso.
    """

    #: La configurazione della macchina non porta al binario, e il comando
    #: non e' partito.
    _CATEGORIA = "invalid_configuration"
    _FASE = "probe"

    def __init__(self, searched: list[str]) -> None:
        # I **posti**, non i percorsi: il valore della variabile d'ambiente, il
        # percorso indicato e il `PATH` sono della macchina di chi chiama, e un
        # errore non porta dati. Chi li vuole li legge dal proprio ambiente.
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

    _CATEGORIA = "invalid_configuration"
    _FASE = "probe"


class ProfileError(PlenoraError):
    """L'artefatto non ha il profilo che il chiamante pretende.

    Sollevata **prima** di eseguire: un profilo `base` non ha il backend GDAL, e
    scoprirlo dal fallimento di una conversione a meta' costa un file di uscita
    parziale e un errore che parla di un driver invece che di un pacchetto.
    """

    #: `unsupported`: e' il caso che PYTHON-SDK-1.0 §7 nomina -- un
    #: comportamento che questo artefatto non ha si rifiuta chiuso con quella
    #: categoria, prima di cominciare.
    _CATEGORIA = "unsupported"
    _FASE = "validate"

    def __init__(self, required: str, actual: str | None) -> None:
        self.required = required
        self.actual = actual
        # Il profilo richiesto non entra nel messaggio: e' un valore di chi
        # chiama, e sta in `required`. Quello dell'artefatto si nomina solo se
        # e' uno dei profili noti: il manifesto e' un file, e un valore fuori
        # vocabolario sarebbe un dato.
        noti = ("base", "filegdb")
        if actual is None:
            quale = "sconosciuto: nessun manifesto"
        elif actual in noti:
            quale = f"«{actual}»"
        else:
            quale = "fuori vocabolario"
        super().__init__(
            f"questo artefatto ha profilo {quale} e non quello richiesto "
            f"(profili noti: {', '.join(noti)}). I profili si scelgono al "
            "momento di installare, non a runtime."
        )


class ProtocolError(PlenoraError):
    """Cio' che il binario ha risposto non e' cio' che il protocollo dichiara.

    Un JSON che non si decodifica, un `contract` inatteso, un campo obbligatorio
    che non c'e'. E' fail-closed per scelta: un SDK che tirasse a indovinare i
    campi mancanti trasformerebbe l'incompatibilita' di versione in dati
    sbagliati piu' avanti, dove nessuno la riconosce piu'.

    # L'effetto remoto non e' sempre `none`

    Un `ProtocolError` puo' nascere prima che il processo parta -- il binario
    non si esegue -- o dopo: una busta illeggibile, un timeout che uccide il
    processo. Nel secondo caso, se il comando scrive (`write`, `convert`,
    `read` con una destinazione), non si sa che cosa sia rimasto sul disco, e
    l'effetto e' `unknown`: lo alza `Runner`, che e' l'unico a sapere se il
    processo e' partito e che comando fosse. Un `none` li' direbbe a chi
    decide se ripetere che non e' successo niente, ed e' esattamente cio' che
    nessuno sa.
    """

    _CATEGORIA = "protocol"
    _FASE = "finalize"


class LocalIoError(PlenoraError):
    """Un'operazione dell'SDK sul filesystem locale e' fallita prima di eseguire.

    Oggi e' la directory temporanea dell'adattatore Arrow: una `temp_dir` che
    non esiste o non si puo' scrivere, o il file IPC temporaneo che non si
    scrive. Il comando non e' partito, e l'effetto e' `none`. L'errore del
    sistema operativo non entra nel messaggio ne' nella catena: porta il
    percorso di chi chiama.
    """

    _CATEGORIA = "io"
    _FASE = "prepare"


class CleanupError(PlenoraError):
    """Il comando e' riuscito, e la pulizia dei file temporanei dell'SDK no.

    ERRORS-1.0, ERR-015: un fallimento di pulizia dopo che l'effetto e' gia'
    avvenuto non si fa passare per un fallimento dell'operazione. La fase e'
    `cleanup`, l'effetto e' quello gia' avvenuto -- `committed` per `write`,
    che ha pubblicato la destinazione -- e il ritentativo e' `never`: ripetere
    la chiamata ripeterebbe l'effetto. Resta da togliere a mano la directory
    temporanea, il cui percorso non e' nel messaggio.
    """

    _CATEGORIA = "io"
    _FASE = "cleanup"

    def __init__(self, message: str, *, remote_effect: str) -> None:
        super().__init__(message)
        self._effetto_remoto = remote_effect


class InvalidArgumentError(PlenoraError):
    """Un argomento che l'SDK non sa trasformare in una riga di comando.

    Per esempio un oggetto che non e' un percorso e non espone
    `__arrow_c_stream__`, passato come sorgente di `write()`. Si rifiuta prima
    di eseguire, e non si ripiega su un'interpretazione: un argomento ignorato
    o tradotto con un altro significato e' cio' che PYTHON-SDK-1.0 §7 vieta.
    """

    _CATEGORIA = "invalid_configuration"
    _FASE = "validate"


class OptionalDependencyError(PlenoraError):
    """Serve una dipendenza facoltativa che non e' installata.

    L'adattatore Arrow (`read_table()`, `write()` da un oggetto Arrow) vuole
    `pyarrow`, che il pacchetto dichiara come extra `plenora-io[pyarrow]` e non
    come dipendenza: chi lavora per percorsi non lo paga. Senza, la chiamata si
    rifiuta con `unsupported` prima di eseguire niente.
    """

    _CATEGORIA = "unsupported"
    _FASE = "validate"


class UnexpectedError(PlenoraError):
    """Un'eccezione che nessun punto dell'SDK ha tradotto, presa dalla rete.

    Ogni metodo pubblico passa da `confine.confinato`: un'eccezione che non e'
    un `PlenoraError` -- di Python, di una dipendenza, di codice del chiamante
    -- diventa questa, senza messaggio originale e senza catena. I punti noti
    restano tradotti dove nascono, con l'effetto preciso; questa e' la rete
    per quelli che nessuno ha previsto, e il suo effetto e' **conservativo**:
    `none` se nessun comando e' partito, `unknown` se e' partito un comando che
    scrive.
    """

    _CATEGORIA = "internal"
    _FASE = "validate"

    def __init__(self, message: str, *, remote_effect: str = "none") -> None:
        super().__init__(message)
        self._effetto_remoto = remote_effect


class PackageMetadataError(PlenoraError):
    """I metadati del pacchetto installato non si leggono.

    `version()` legge i metadati della distribuzione: da un checkout non
    installato non ci sono, e la risposta non e' `__version__`, che direbbe una
    cosa diversa da cio' che e' installato.
    """

    _CATEGORIA = "invalid_configuration"
    _FASE = "probe"


class ResultLookupError(PlenoraError, KeyError):
    """Un nome cercato in un risultato -- layer, campo, driver, operazione -- non c'e'.

    E' anche un `KeyError`, come prima, perche' chi lo intercettava cosi' non
    debba cambiare. Il messaggio non riporta il nome cercato ne' quelli che ci
    sono: sono nomi del file o del chiamante, e un errore non porta dati. Chi
    vuole sapere che cosa c'e' legge il risultato.
    """

    _CATEGORIA = "not_found"
    _FASE = "validate"

    # `KeyError` mostra il `repr` dell'argomento; qui il messaggio e' testo.
    __str__ = Exception.__str__


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

    # Copiati, e decisi una volta sola

    Ogni campo e' una **copia** di cio' che si e' passato, fatta di tipi JSON
    esatti: `retry` e `row_diagnostics` sono dizionari ed elenchi ordinari,
    ricostruiti valore per valore. La busta non tiene mai un oggetto del
    chiamante, e cio' che si valida e' cio' che si conserva.

    Le **decisioni** -- `retryable`, `retry_after_ms`,
    `must_assume_remote_committed` di `CommandFailed` -- non rileggono `retry`:
    usano il tipo di ritentativo, il ritardo e l'effetto remoto fissati in
    `__post_init__` dopo la validazione, in attributi privati che non sono
    campi della dataclass. Prima la busta conservava il dizionario del
    chiamante, e cambiarlo dopo da `{kind: never}` a `{kind: safe}` rendeva
    `retryable` vero senza rivalidazione. Ora chi cambia `envelope.retry`
    cambia la propria copia, non la decisione.

    Perche' dizionari ordinari e non `MappingProxyType`: con il proxy
    `copy.deepcopy`, `pickle` e `dataclasses.asdict` fallivano, e
    `dataclasses.replace` rifiutava il proxy prodotto dall'SDK stesso. Con le
    copie ordinarie le quattro operazioni funzionano come prima, e la
    garanzia sta dove serve: nelle decisioni, che nessuno puo' riscrivere
    attraverso un campo pubblico. `replace` ricostruisce la busta e quindi la
    rivalida.
    """

    code: str
    category: str
    phase: str
    remote_effect: str
    retry: dict[str, Any]
    message: str
    row_diagnostics: dict[str, Any] | None = None

    def __post_init__(self) -> None:
        # La validazione sta **qui** e non solo in `from_json`: una busta
        # costruita a mano -- in un test, in un adattatore, da
        # `dataclasses.replace` -- arriva alle stesse proprieta' di
        # `CommandFailed`, e un valore fuori vocabolario deve fermarsi prima di
        # diventare una risposta su «ritentare e' sicuro».
        #
        # Prima si copia, poi si valida la copia, poi si conserva la copia.
        for campo in ("code", "category", "phase", "remote_effect", "message"):
            object.__setattr__(
                self, campo, copia_json(getattr(self, campo), f"error.{campo}")
            )
        if _tipo(self.retry) != "object":
            raise ProtocolError(
                f"`error.retry` e' {_tipo(self.retry)} e non un oggetto `{{kind}}`."
            )
        object.__setattr__(self, "retry", copia_json(self.retry, "error.retry"))
        if self.row_diagnostics is not None:
            if _tipo(self.row_diagnostics) != "object":
                raise ProtocolError(
                    f"`row_diagnostics` e' {_tipo(self.row_diagnostics)} e non un oggetto."
                )
            object.__setattr__(
                self,
                "row_diagnostics",
                copia_json(self.row_diagnostics, "error.details.row_diagnostics"),
            )
        _valida_busta(self)
        # Le decisioni, fissate sulla copia validata. Sono stringhe e interi
        # di tipo esatto, cioe' immutabili; non sono campi, quindi `asdict`,
        # l'uguaglianza e `replace` non le vedono, mentre `copy` e `pickle` le
        # portano con lo stato dell'istanza.
        object.__setattr__(self, "_tipo_di_ritentativo", self.retry["kind"])
        object.__setattr__(self, "_ritardo_ms", self.retry.get("delay_ms"))
        object.__setattr__(self, "_effetto_remoto", self.remote_effect)

    @classmethod
    def from_json(cls, documento: dict[str, Any]) -> "ErrorEnvelope":
        if _tipo(documento) != "object":
            raise ProtocolError(
                f"la busta d'errore e' {_tipo(documento)} e non un oggetto."
            )
        # Tutto il documento, non solo i campi che si leggono: una chiave che
        # non e' una `str` esatta o un valore che non e' JSON in un ramo
        # ignorato restano un documento che il wire non produce.
        documento = copia_json(documento, "busta d'errore")
        errore = documento.get("error")
        if _tipo(errore) != "object":
            raise ProtocolError(
                f"la busta d'errore non porta l'oggetto `error` (e' {_tipo(errore)})."
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


def _tipo(valore: Any) -> str:
    """Il tipo JSON **esatto** di un valore, per controlli e messaggi.

    Mai il valore stesso. Il confronto e' per **identita'** del tipo, con
    `is`: ne' `isinstance`, che accetta le sottoclassi, ne' una ricerca in un
    dizionario di tipi, che passa per `__hash__` e `__eq__` del tipo e che una
    metaclasse puo' riscrivere. Una sottoclasse di `str` con `__eq__` e
    `__hash__` riscritti superava `in EFFETTI_REMOTI` fingendosi `none`; una di
    `dict` con `get` riscritto rispondeva a `kind` con un valore che non aveva.
    Il wire non le produce mai: chi le passa non sta passando JSON, e la
    risposta giusta e' rifiutarle, non interpretarle.
    """
    tipo = type(valore)
    if valore is None:
        return "null"
    if tipo is bool:
        return "boolean"
    if tipo is int:
        return "integer"
    if tipo is float:
        return "number"
    if tipo is str:
        return "string"
    if tipo is list:
        return "array"
    if tipo is dict:
        return "object"
    return "non JSON"


def copia_json(valore: Any, dove: str) -> Any:
    """Una copia profonda di un valore JSON, ricostruita con i soli tipi esatti.

    Ogni oggetto ed elenco e' nuovo; le foglie sono di tipo **identico** a
    `str`, `int`, `float`, `bool` o `None`, cioe' immutabili, e solo allora si
    conservano. Ogni altro tipo, a qualunque profondita' e nelle chiavi, e i
    numeri non finiti sono `ProtocolError`.

    `dove` e' il nome di un contenitore del protocollo, scelto da chi chiama.
    Il messaggio non vi aggiunge le chiavi del documento -- sono dati di chi
    l'ha scritto, una colonna di `row_diagnostics` per esempio -- ma soltanto
    la profondita' a cui il difetto sta.

    La copia e' ricorsiva, e un documento che `json.loads` accetta puo' essere
    piu' profondo di quanto lo stack di Python regga qui: `RecursionError`
    diventa `ProtocolError`, invece di attraversare il confine.
    """
    try:
        return _copia_json(valore, dove, 0)
    except RecursionError:
        raise ProtocolError(
            f"`{dove}` e' annidato piu' a fondo di quanto l'SDK sappia copiare."
        ) from None


def _copia_json(valore: Any, dove: str, _profondita: int) -> Any:
    tipo = _tipo(valore)
    if tipo == "object":
        copia = {}
        for chiave, interno in valore.items():
            if _tipo(chiave) != "string":
                raise ProtocolError(
                    f"`{dove}` ha, a profondita' {_profondita + 1}, una chiave "
                    f"{_tipo(chiave)} e non una stringa."
                )
            copia[chiave] = _copia_json(interno, dove, _profondita + 1)
        return copia
    if tipo == "array":
        return [_copia_json(interno, dove, _profondita + 1) for interno in valore]
    if tipo == "number" and not math.isfinite(valore):
        raise ProtocolError(
            f"`{dove}` ha, a profondita' {_profondita}, un numero non finito, "
            "che JSON non ha."
        )
    if tipo == "non JSON":
        raise ProtocolError(
            f"`{dove}` ha, a profondita' {_profondita}, un valore che non e' "
            "un tipo JSON esatto."
        )
    return valore


def _rifiuta_costante(nome: str) -> Any:
    raise ProtocolError(f"il documento contiene `{nome}`, che JSON non ha.")


def _float_finito(testo: str) -> float:
    numero = float(testo)
    if not math.isfinite(numero):
        # `1e400` non e' una costante: e' un numero che in doppia precisione
        # trabocca in infinito, e `parse_constant` non lo vede.
        raise ProtocolError(
            "il documento contiene un numero che in doppia precisione non e' finito."
        )
    return numero


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
    """`json.loads` senza le sue tolleranze.

    `NaN`, `Infinity` e `-Infinity` non sono JSON, e `json.loads` li accetta;
    un numero come `1e400` diventa infinito per trabocco; una chiave ripetuta
    nello stesso oggetto si risolve tenendo l'ultima. Sono modi in cui un
    documento diverso da quello scritto arriverebbe ai modelli senza che
    nessuno lo veda: qui sono `ProtocolError`.

    Lo sono anche **tutti** i modi in cui il parser fallisce, non solo la
    sintassi: `json.loads` solleva `RecursionError` su un annidamento profondo
    e `ValueError` su un intero oltre il limite di cifre di Python, e prima
    uscivano com'erano -- fuori dalla gerarchia, e per un comando che scrive
    senza l'effetto `unknown`. Il messaggio dice posizione e motivo, mai il
    testo; `from None` taglia la catena, che porterebbe il documento in `doc`.
    """
    try:
        return json.loads(
            testo,
            object_pairs_hook=_rifiuta_chiavi_doppie,
            parse_constant=_rifiuta_costante,
            parse_float=_float_finito,
        )
    except ProtocolError:
        raise
    except json.JSONDecodeError as errore:
        motivo = f"{errore.msg}, riga {errore.lineno}, colonna {errore.colno}"
    except RecursionError:
        motivo = "annidamento oltre la profondita' che il parser regge"
    except ValueError:
        motivo = "un valore che il parser non converte, come un intero oltre il limite di cifre"
    raise ProtocolError(f"il documento non e' JSON valido ({motivo}).") from None


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

    # `retry` e' gia' la copia fatta in `__post_init__`.
    retry = busta.retry
    tipo = retry.get("kind")
    if _tipo(tipo) != "string" or tipo not in TIPI_DI_RITENTATIVO:
        raise ProtocolError(
            f"`error.retry.kind` ({_tipo(tipo)}) non e' nel vocabolario chiuso "
            f"di `plenora-error-v1`: {sorted(TIPI_DI_RITENTATIVO)}."
        )
    attese = {"kind", "delay_ms"} if tipo == "after" else {"kind"}
    if set(retry) != attese:
        # Le chiavi in piu' non si nominano: vengono dal documento.
        raise ProtocolError(
            f"`error.retry` di tipo «{tipo}» ha {len(retry)} chiavi e non "
            f"esattamente {sorted(attese)}, come lo schema pretende."
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
        # Il solo nome del sottocomando: la riga intera porta percorsi e opzioni
        # di chi chiama, che restano in `argv` e non entrano nel messaggio.
        nome = self.argv[0] if self.argv and self.argv[0].replace("-", "").isalnum() else "?"
        super().__init__(
            f"`plenora-io {nome}` e' uscito con {exit_code}: "
            f"[{envelope.category}/{envelope.phase}] "
            f"{envelope.code}: {envelope.message}"
        )

    # I cinque assi vengono dalla busta, non dai default della radice.

    @property
    def category(self) -> str:
        return self.envelope.category

    @property
    def phase(self) -> str:
        return self.envelope.phase

    @property
    def remote_effect(self) -> str:
        # La decisione fissata alla costruzione, non il campo pubblico: chi
        # cambia `envelope.remote_effect` cambia la propria copia.
        return self.envelope._effetto_remoto

    @property
    def retry(self) -> dict[str, Any]:
        """La disposizione fissata alla costruzione, come oggetto nuovo.

        Ricostruita dalle decisioni private della busta e non copiata da
        `envelope.retry`: quel campo e' una copia che chi la tiene puo'
        cambiare, e l'asse dell'eccezione deve dire cio' che e' stato validato.
        """
        busta = self.envelope
        if busta._tipo_di_ritentativo == "after":
            return {"kind": "after", "delay_ms": busta._ritardo_ms}
        return {"kind": busta._tipo_di_ritentativo}

    @property
    def message(self) -> str:
        """Il messaggio della busta, senza la riga di comando che lo precede."""
        return self.envelope.message

    @property
    def code(self) -> str | None:
        return self.envelope.code

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
        return self.envelope._tipo_di_ritentativo in RITENTABILI

    @property
    def retry_after_ms(self) -> int | None:
        """I millisecondi da attendere, quando la busta li dichiara.

        `None` non vuol dire «riprova subito»: vuol dire che il prodotto non ha
        detto quanto aspettare, e chi riprova sceglie da se'.
        """
        busta = self.envelope
        return busta._ritardo_ms if busta._tipo_di_ritentativo == "after" else None

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
        return self.envelope._effetto_remoto not in EFFETTI_SENZA_RESIDUO


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
