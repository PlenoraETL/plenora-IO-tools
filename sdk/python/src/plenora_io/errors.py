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


#: I sottocomandi della CLI che l'SDK chiama: un elenco **chiuso**. Un nome
#: entra in un messaggio o in un attributo solo se e' uno di questi; ogni altro
#: primo argomento -- anche alfanumerico, anche un `Runner.run(["SEGRETO"])` --
#: e' un dato di chi chiama, e compare come `?`.
SOTTOCOMANDI: frozenset[str] = frozenset(
    {"--version", "catalog", "capabilities", "inspect", "layers", "read", "write", "convert"}
)

#: I profili che la distribuzione produce: l'elenco chiuso di `discovery.PROFILI`,
#: qui perche' `ProfileError` lo usa per decidere che cosa conservare.
PROFILI_NOTI: tuple[str, ...] = ("base", "filegdb")


def sottocomando(argv: Any) -> str | None:
    """Il sottocomando, se il primo argomento e' nell'elenco chiuso."""
    if (type(argv) is not list and type(argv) is not tuple) or not argv:
        return None
    primo = argv[0]
    return primo if type(primo) is str and primo in SOTTOCOMANDI else None


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

    # Lo stato si fissa una volta, e il messaggio non sta in `args`

    Il messaggio, l'effetto remoto e gli attributi di ogni sottoclasse
    (`envelope`, `exit_code`, `command`, `searched`, `required`, `actual`...)
    si fissano nel costruttore, che non si ripete: dopo, assegnarli o
    cancellarli e' `InvalidArgumentError`, e lo e' anche aggiungere un
    attributo nuovo. Prima `errore.envelope = altra` o un secondo `__init__`
    cambiavano `retryable` di un'eccezione gia' costruita. Restano liberi solo
    gli attributi che l'interprete e la libreria standard impostano su
    un'eccezione: `__traceback__`, `__context__`, `__cause__`,
    `__suppress_context__`, `__notes__` e `args` (`ATTRIBUTI_LIBERI`).

    `str(errore)` e `message` leggono il messaggio fissato (`_messaggio`), non
    `args`: chi cambia `args` cambia la tupla, non il messaggio. La rete
    (`confine._confinato`) ricostruisce ogni errore dell'SDK che la attraversa
    da quello stato (`ricostruito`), senza mai leggere `args`, `__notes__` o
    `str()` dell'oggetto ricevuto: una callback che prende un errore
    dell'SDK, ne riscrive `args` e lo rilancia non fa uscire il proprio testo.

    Il sigillo lo mette `confine` attorno a `__init__` (`_init_sigillato`).
    La provenienza -- l'errore e' dell'SDK -- la marca soltanto la fabbrica
    privata `_dallo_sdk`: il costruttore pubblico fa sempre un errore esterno.
    """

    #: La categoria di `error-v1` per gli errori nati nell'SDK.
    _CATEGORIA = "internal"

    def __init__(self, message: Any = "") -> None:
        # Il messaggio si normalizza qui: `str(errore)` non deve chiamare un
        # `__str__` di chi ha costruito l'errore. Si conserva fuori da `args`.
        testo = _messaggio(message)
        self._messaggio = testo
        super().__init__(testo)

    #: La fase di `error-v1` in cui l'SDK li solleva.
    _FASE = "validate"

    def __str__(self) -> str:
        testo = dict.get(object.__getattribute__(self, "__dict__"), "_messaggio")
        return testo if type(testo) is str else "(errore dell'SDK senza messaggio)"

    def __setattr__(self, nome: str, valore: Any) -> None:
        if _modificabile(self, nome):
            object.__setattr__(self, nome, valore)
            return
        raise _sigillato(self)

    def __delattr__(self, nome: str) -> None:
        if _modificabile(self, nome):
            object.__delattr__(self, nome)
            return
        raise _sigillato(self)

    def __reduce__(self) -> Any:
        # `copy` e `pickle`: dallo stato fissato, mai da `args`. La copia non
        # e' «costruita dall'SDK» (vedi `_da_copia`).
        return (_da_copia, (type(self), _stato(self)))

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


def _messaggio(valore: Any) -> str:
    """Il messaggio di un errore come `str` esatta, senza chiamarne i metodi."""
    if type(valore) is str:
        return valore
    if issubclass(type(valore), str):
        return str.join("", [valore])
    return "(messaggio non testuale)"


#: Gli attributi di un'eccezione che l'interprete e la libreria standard
#: impostano dopo la costruzione (`raise ... from`, `add_note`, `contextlib`,
#: `unittest`): restano liberi anche su un errore sigillato. Nessuno di loro
#: entra in cio' che la rete ricostruisce.
ATTRIBUTI_LIBERI: frozenset[str] = frozenset(
    {"__traceback__", "__context__", "__cause__", "__suppress_context__", "__notes__", "args"}
)

#: Lo stato privato che `confine` scrive attorno al costruttore: il sigillo e
#: la provenienza.
SIGILLO = "_sigillato"
PROVENIENZA = "_dall_sdk"


def _sigillato(errore: Any) -> "InvalidArgumentError":
    # Il nome della classe dell'SDK, non `type(errore).__name__`: una
    # sottoclasse del chiamante puo' chiamarsi come vuole.
    from .confine import nome_sdk  # noqa: PLC0415 - confine importa questo modulo

    return _dallo_sdk(InvalidArgumentError, "errore.stato_sigillato", nome=nome_sdk(type(errore)))


def _modificabile(errore: Any, nome: Any) -> bool:
    if type(nome) is str and nome in ATTRIBUTI_LIBERI:
        return True
    return dict.get(object.__getattribute__(errore, "__dict__"), SIGILLO) is not True


def _stato(errore: Any) -> dict[str, Any]:
    """Lo stato fissato di un errore, senza `__notes__`: una copia del dizionario."""
    stato = dict(object.__getattribute__(errore, "__dict__"))
    stato.pop("__notes__", None)
    return stato


def classe_dell_sdk(tipo: type) -> bool:
    """`tipo` e' una classe d'errore definita qui, per identita'."""
    return any(
        tipo is classe
        for classe in list(globals().values())
        if type(classe) is type and issubclass(classe, PlenoraError)
    )


# --- il catalogo dei messaggi e il vocabolario dei parametri -----------------
#
# Tredicesimo giro di revisione: in dodici giri, ogni volta un nome dinamico
# diverso -- un `dove`, un nome di campo letto da `dataclasses.fields`, un
# attributo di una sottoclasse -- finiva nel messaggio di un errore fidato. La
# classe si chiude nella fabbrica: `_dallo_sdk` non accetta testo, ma una
# **chiave** di `MESSAGGI` e dei parametri. Quattordicesimo giro: ogni
# parametro ha un tipo dichiarato per la sua chiave (`PARAMETRI`) -- un insieme
# chiuso specifico, o un intero esatto in un intervallo -- e si scrive solo se
# lo rispetta; altrimenti e' `?`. Non c'e' un vocabolario globale che faccia da
# lasciapassare, e le stringhe numeriche non sono parole. Nessun errore smette
# di nascere per un parametro sbagliato, e nessun dato del chiamante arriva nel
# testo, da qualunque via.

#: I modelli fissi dei messaggi degli errori dell'SDK. Le chiavi sono le sole
#: ammesse da `_dallo_sdk`; una prova sull'AST verifica che ogni chiamata ne usi
#: una letterale, e una seconda prova prova ogni chiave con parametri ostili.
MESSAGGI: dict[str, str] = {
    # la fabbrica e la ricostruzione
    "sdk.sconosciuto": "errore dell'SDK senza un messaggio a catalogo.",
    "fabbrica.non_sdk": "solo un errore dell'SDK nasce dalla fabbrica.",
    "ricostruito.pubblico": "`ricostruito` pubblica non sostituisce lo stato di un errore.",
    "ricostruito.non_sdk": "solo un errore dell'SDK si ricostruisce.",
    "ricostruito.voce": "una voce dello stato di un errore non si sostituisce.",
    "copia.stato_estraneo": "lo stato non e' quello di un errore dell'SDK.",
    "copia.senza_messaggio": "lo stato di un errore dell'SDK non ha un messaggio.",
    "copia.voce": "lo stato di un errore dell'SDK ha una voce non ammessa.",
    "errore.stato_sigillato": (
        "`{nome}` e' gia' costruito: lo stato di un errore dell'SDK non si modifica."
    ),
    "errore.gia_costruito": (
        "`{nome}` e' gia' costruito: un errore dell'SDK non si reinizializza."
    ),
    "effetto.non_valido": "`remote_effect` non e' un effetto di `plenora-error-v1`.",
    "fallimento.envelope": "`envelope` non e' un `ErrorEnvelope`.",
    "fallimento.exit_code": "`exit_code` non e' un intero.",
    # il pacchetto
    "pacchetto.senza_versione": "i metadati del pacchetto non portano una versione.",
    "pacchetto.non_installato": (
        "i metadati del pacchetto `plenora-io` non si leggono: il pacchetto "
        "non e' installato in questo ambiente."
    ),
    # gli argomenti
    "arg.non_stringa": "`{nome}` non e' una stringa.",
    "arg.stringa_illeggibile": "`{nome}` non si lascia leggere come stringa.",
    "arg.nul": (
        "`{nome}` contiene un carattere NUL, che un argomento di processo non puo' portare."
    ),
    "arg.non_utf8": "`{nome}` contiene un carattere che non si codifica in UTF-8.",
    "arg.non_intero": "`{nome}` non e' un intero.",
    "arg.intero_illeggibile": "`{nome}` non si lascia leggere come intero.",
    "arg.fuori_64": "`{nome}` e' fuori dall'intervallo che la CLI rappresenta (64 bit).",
    "arg.non_percorso": "`{nome}` non e' un percorso: serve una `str` o un `os.PathLike`.",
    "arg.percorso_illeggibile": "`{nome}` non si lascia leggere come percorso.",
    "arg.percorso_non_testo": (
        "`{nome}` non e' un percorso di testo: serve una `str` o un `os.PathLike[str]`."
    ),
    "arg.non_booleano": "`{nome}` non e' un booleano.",
    "arg.non_dizionario": "`{nome}` non e' un dizionario di stringhe.",
    "arg.dizionario_illeggibile": "`{nome}` non si lascia leggere come dizionario.",
    "arg.non_coppie": "`{nome}` non rende coppie chiave-valore.",
    "arg.chiave_con_uguale": (
        "`{nome}` ha una chiave che contiene `=`: la CLI divide al primo `=`, e "
        "la coppia arriverebbe con un'altra chiave."
    ),
    "arg.chiave_ripetuta": "`{nome}` ripete una chiave.",
    "arg.non_durata": "`{nome}` non e' un `timedelta`.",
    "arg.durata_illeggibile": "`{nome}` non si lascia leggere come durata.",
    "arg.non_limits": "`{nome}` non e' un `Limits`.",
    "arg.illeggibile": "`{nome}` non si lascia leggere.",
    "arg.non_secondi": "`{nome}` non e' un numero di secondi.",
    "arg.numero_illeggibile": "`{nome}` non si lascia leggere come numero.",
    "arg.secondi_non_positivi": "`{nome}` non e' un numero di secondi positivo e finito.",
    # l'adattatore Arrow
    "arrow.assente": (
        "l'adattatore Arrow richiede pyarrow, che non e' installato o non "
        "si importa: installa l'extra `plenora-io[pyarrow]` (pyarrow>=25,<26)."
    ),
    "arrow.serie": (
        "l'adattatore Arrow e' provato con pyarrow {attesa}.x e l'ambiente ne ha "
        "un'altra serie ({serie}.x): installa l'extra `plenora-io[pyarrow]`."
    ),
    "arrow.non_arrow": (
        "la sorgente non e' un percorso e non espone `__arrow_c_stream__`: "
        "write() accetta un file Arrow IPC o un oggetto Arrow "
        "(pa.Table, pa.RecordBatch, pa.RecordBatchReader)."
    ),
    "arrow.illeggibile": (
        "la sorgente Arrow non si e' potuta leggere come stream: il produttore "
        "di `__arrow_c_stream__` ha sollevato, o pyarrow non ne legge lo stream."
    ),
    "arrow.non_scrivibile": (
        "il file Arrow IPC temporaneo non si e' potuto scrivere nella "
        "directory temporanea."
    ),
    "arrow.content_type": (
        "la consegna dichiara un content type che non e' una delle due "
        "serializzazioni Arrow IPC: l'adattatore non indovina come leggerla."
    ),
    "arrow.consegna_illeggibile": (
        "il file che `read` dichiara di aver consegnato non si legge come "
        "Arrow IPC nella serializzazione dichiarata."
    ),
    "arrow.cartella": (
        "la directory temporanea dell'adattatore Arrow non si e' potuta "
        "creare: `temp_dir` non esiste o non si puo' scrivere."
    ),
    "arrow.pulizia": (
        "l'operazione e' riuscita e la directory temporanea "
        "dell'adattatore Arrow non si e' potuta cancellare: va tolta a mano."
    ),
    # il client
    "client.consegna_non_dichiarata": (
        "`read` con una destinazione e' riuscito senza dichiarare "
        "la consegna: non si sa che cosa leggere."
    ),
    "client.non_decodifica": (
        "il risultato di `plenora-io {comando}` non si decodifica nel modello {modello}."
    ),
    # la rete e i modelli
    "rete.imprevisto": (
        "errore imprevisto dell'SDK in `{etichetta}`: un'eccezione "
        "esterna e' stata intercettata al confine, senza il suo testo."
    ),
    "rete.self_estraneo": (
        "un metodo dell'SDK e' stato chiamato su un oggetto che non e' della sua classe."
    ),
    "modello.non_sdk": "il modello non deriva da una classe dell'SDK.",
    "modello.oggetto_non_ammesso": (
        "`{dove}` contiene un oggetto di un tipo che un risultato dell'SDK non porta."
    ),
    "modello.alterato": (
        "il modello contiene un valore che un risultato dell'SDK non porta: "
        "e' stato alterato fuori contratto."
    ),
    "modello.immutabile": "`{nome}` e' immutabile: un risultato dell'SDK non si modifica.",
    "modello.gia_costruito": (
        "`{nome}` e' gia' costruito: un risultato dell'SDK non si reinizializza."
    ),
    "modello.post_init": (
        "`{nome}` e' gia' costruito: `__post_init__` gira solo nel costruttore, "
        "e un risultato dell'SDK non si rinormalizza."
    ),
    "modello.chiave_non_stringa": "`{dove}` ha una chiave che non e' una stringa.",
    "modello.chiavi_coincidenti": "`{dove}` ha due chiavi che normalizzate coincidono.",
    "modello.annidato": "`{dove}` e' annidato troppo a fondo.",
    # i contenitori
    "elenco.immutabile": "un elenco di un risultato dell'SDK e' immutabile.",
    "dizionario.immutabile": "un dizionario di un risultato dell'SDK e' immutabile.",
    "contenitore.argomenti": "`{nome}` vuole al piu' un argomento, per posizione.",
    "contenitore.non_si_estende": "`{nome}` non si estende.",
    "elenco.vuole_elenco": "`FrozenList` vuole un elenco.",
    "dizionario.vuole_dizionario": "`FrozenDict` vuole un dizionario.",
    "elenco.passo_non_intero": "il passo di uno slice e' un intero o `None`.",
    "elenco.inizio_non_intero": "l'inizio di uno slice e' un intero o `None`.",
    "elenco.fine_non_intero": "la fine di uno slice e' un intero o `None`.",
    "elenco.limite_non_intero": "un limite di `index()` e' un intero o `None`.",
    "elenco.passo_zero": "il passo di uno slice non puo' essere zero.",
    "elenco.indice_non_intero": "l'indice di un elenco e' un intero.",
    "elenco.indice_oltre": "indice oltre l'elenco.",
    "elenco.index_argomenti": (
        "`index()` vuole il valore e, per posizione, al piu' inizio e fine."
    ),
    "elenco.valore_assente": "valore assente dall'elenco.",
    "dizionario.chiave_non_testuale": "chiave non testuale.",
    "dizionario.chiave_assente": "chiave assente dal dizionario.",
    # le ricerche nei risultati
    "ricerca.driver": "nessun driver con l'id richiesto nel catalogo.",
    "ricerca.campo": "nessun campo con il nome richiesto nel layer.",
    "ricerca.layer": "nessun layer con il nome richiesto nel file.",
    "ricerca.layer_conversione": "nessun layer con il nome richiesto nella conversione.",
    "ricerca.operazione": "nessuna operazione con l'id richiesto.",
    # la scoperta e il manifesto
    "binario.ricerca_path": (
        "la ricerca del binario nel `PATH` non si e' potuta fare: una voce "
        "del `PATH` non si esamina."
    ),
    "fs.non_esaminabile": (
        "{che_cosa} non si e' potuto esaminare: un permesso o un guasto del "
        "filesystem impedisce di dire se c'e'."
    ),
    "fs.non_risolvibile": "{che_cosa} non si e' potuto risolvere in un percorso assoluto.",
    "manifesto.non_oggetto": "MANIFEST.json e' {tipo} e non un oggetto.",
    "manifesto.mancanti": (
        "MANIFEST.json senza i campi {mancanti}. Sono fra quelli che "
        "entrambi i costruttori devono scrivere, e uno che manca vuol "
        "dire che l'artefatto non e' stato prodotto dalla pipeline."
    ),
    "manifesto.tipo_campo": "MANIFEST.json: `{campo}` e' {tipo} e non {tipi}.",
    "manifesto.da_errore": "{errore}",
    "manifesto.illeggibile": "MANIFEST.json c'e' e non si legge.",
    "manifesto.non_utf8": "MANIFEST.json c'e' e non e' UTF-8: l'artefatto e' guasto.",
    "manifesto.non_json": (
        "MANIFEST.json c'e' e non si legge come JSON: {errore} Un manifesto "
        "rotto non e' un manifesto assente: l'artefatto e' guasto."
    ),
    "manifesto.non_oggetto_json": "MANIFEST.json non contiene un oggetto JSON.",
    # JSON e la busta d'errore
    "json.costante": "il documento contiene `{nome}`, che JSON non ha.",
    "json.non_testo": "il documento non e' testo.",
    "json.numero_infinito": (
        "il documento contiene un numero che in doppia precisione non e' finito."
    ),
    "json.chiave_ripetuta": (
        "il documento ripete una chiave nello stesso oggetto: "
        "`json.loads` terrebbe in silenzio l'ultima."
    ),
    "json.non_valido": (
        "il documento non e' JSON valido ({motivo}, riga {riga}, colonna {colonna})."
    ),
    "json.troppo_annidato": (
        "il documento non e' JSON valido (annidamento oltre la profondita' "
        "che il parser regge)."
    ),
    "json.non_convertibile": (
        "il documento non e' JSON valido (un valore che il parser non converte, "
        "come un intero oltre il limite di cifre)."
    ),
    "json.profondo": "`{dove}` e' annidato piu' a fondo di quanto l'SDK sappia copiare.",
    "json.chiave_non_stringa": (
        "`{dove}` ha, a profondita' {profondita}, una chiave {tipo} e non una stringa."
    ),
    "json.non_finito": (
        "`{dove}` ha, a profondita' {profondita}, un numero non finito, che JSON non ha."
    ),
    "json.non_esatto": (
        "`{dove}` ha, a profondita' {profondita}, un valore che non e' un tipo JSON esatto."
    ),
    "busta.non_oggetto": "la busta d'errore e' {tipo} e non un oggetto.",
    "busta.senza_error": "la busta d'errore non porta l'oggetto `error` (e' {tipo}).",
    "busta.mancanti": (
        "busta d'errore senza i campi obbligatori {mancanti}. "
        "`plenora-io-error-v1` ne dichiara sei, e ci sono sempre."
    ),
    "busta.details": (
        "`error.details` e' {tipo} e non un oggetto: lo schema `plenora-error-v1` "
        "lo dichiara oggetto quando c'e'."
    ),
    "busta.details_row_diagnostics": (
        "`error.details.row_diagnostics` e' {tipo} e non un oggetto."
    ),
    "busta.retry_non_oggetto": "`error.retry` e' {tipo} e non un oggetto `{{kind}}`.",
    "busta.row_diagnostics": "`row_diagnostics` e' {tipo} e non un oggetto.",
    "busta.campo_testuale": "`error.{campo}` e' {tipo} e non una stringa non vuota.",
    "busta.effetto": (
        "`error.remote_effect` ({tipo}) non e' nel vocabolario chiuso di "
        "`plenora-error-v1`: {ammessi}. Senza sapere che cosa e' successo "
        "dall'altra parte nessuna risposta su un ritentativo e' sicura."
    ),
    "busta.retry_kind": (
        "`error.retry.kind` ({tipo}) non e' nel vocabolario chiuso di "
        "`plenora-error-v1`: {ammessi}."
    ),
    "busta.retry_chiavi": (
        "`error.retry` di tipo «{tipo}» ha {chiavi} chiavi e non esattamente "
        "{attese}, come lo schema pretende."
    ),
    "busta.ritardo": (
        "`error.retry.delay_ms` ({tipo}) non e' un intero fra 0 e {massimo}."
    ),
    "busta.effetto_ignoto": (
        "`error.remote_effect` e' «unknown» e `error.retry.kind` e' «{tipo}»: lo "
        "schema ammette soltanto {ammessi}, perche' un ritentativo non si "
        "dichiara sicuro senza sapere che cosa e' successo."
    ),
    # i documenti dei modelli
    "protocollo.non_oggetto": "{dove}: e' {tipo} e non un oggetto.",
    "protocollo.mancanti": (
        "{dove}: mancano i campi {mancanti}, che il protocollo v2 dichiara "
        "sempre presenti. Il binario e' piu' vecchio dell'SDK, o il "
        "documento non e' quello che dice di essere."
    ),
    "protocollo.tipo_campo": (
        "{dove}.{campo} e' {tipo} e non {ammessi}, come il protocollo v2 lo dichiara."
    ),
    "protocollo.tipo_elemento": (
        "{dove}.{campo}[{posizione}] e' {tipo} e non {ammessi}, come il "
        "protocollo v2 lo dichiara."
    ),
    "protocollo.non_elenco": "{dove} e' {tipo} e non un elenco.",
    "versione.campi_in_piu": (
        "risultato di --version con {campi} campi in piu'. Il suo "
        "schema e' chiuso: due campi, ne' uno di meno ne' uno di piu'."
    ),
    "capabilities.schema": (
        "capabilities.schema_version non e' 2: l'SDK legge "
        "`plenora-capabilities-v2` e non indovina un'altra forma."
    ),
    # il processo
    "processo.argv": "`argv` non e' una lista di stringhe.",
    "processo.argomento_non_passabile": (
        "un argomento di `plenora-io {comando}` non si puo' passare a un "
        "processo: contiene un carattere NUL o non e' testo."
    ),
    "processo.non_eseguibile": (
        "il binario di plenora-io non si e' potuto eseguire per `plenora-io {comando}`."
    ),
    "processo.chiusura": (
        "la chiusura del processo di `plenora-io {comando}` non e' riuscita: "
        "l'esito non si conosce."
    ),
    "processo.timeout": (
        "`plenora-io {comando}` non ha risposto entro il tempo massimo del client "
        "ed e' stato terminato. Il timeout lo sceglie chi chiama, e questo errore non dice "
        "che il comando sia fallito: dice che non si sa, e che una destinazione "
        "parziale puo' essere rimasta."
    ),
    "processo.comunicazione": (
        "la comunicazione con `plenora-io {comando}` e' fallita mentre il "
        "processo era in corso: l'esito non si conosce."
    ),
    "processo.non_utf8": (
        "`plenora-io {comando}` ha scritto su {flusso} byte che non sono "
        "UTF-8 ({byte} byte, contenuto non riportato)."
    ),
    "processo.stderr_successo": (
        "`plenora-io {comando}` e' riuscito e ha scritto su stderr, dove il "
        "protocollo v2 non mette niente in caso di successo. L'SDK parla v2: un "
        "altro protocollo si sceglie, non si deduce. stderr: {caratteri} "
        "caratteri, contenuto non riportato."
    ),
    "processo.stato_successo": (
        "`plenora-io {comando}` e' uscito con zero e ha scritto su stdout una "
        "busta di stato {stato}: il protocollo non prevede questa combinazione."
    ),
    "processo.senza_result": (
        "`plenora-io {comando}` e' riuscito ma la sua busta non porta un oggetto "
        "`result`: il protocollo mette li' i dati dell'operazione, e senza non "
        "c'e' un risultato da consegnare."
    ),
    "processo.stato_errore": (
        "`plenora-io {comando}` e' uscito con {uscita} e ha scritto su stdout "
        "una busta di stato {stato}: un'uscita diversa da zero porta una busta "
        "d'errore."
    ),
    "processo.stderr_errore": (
        "`plenora-io {comando}` e' fallito e ha scritto su stderr, dove il "
        "protocollo non mette niente. stderr: {caratteri} caratteri, contenuto "
        "non riportato."
    ),
    "processo.flusso_vuoto": (
        "`plenora-io {comando}` e' uscito con {uscita} e non ha scritto niente "
        "su {flusso}, dove il protocollo mette la busta. {altro}: {caratteri} "
        "caratteri, contenuto non riportato."
    ),
    "processo.busta_illeggibile": (
        "cio' che `plenora-io {comando}` ha scritto su {flusso} non si legge: "
        "{errore}; {caratteri} caratteri, contenuto non riportato."
    ),
    "processo.busta_non_oggetto": "la busta su {flusso} e' {tipo} e non un oggetto JSON.",
}

#: Le classi che non prendono un messaggio ma lo compongono da un vocabolario
#: chiuso proprio: i posti di ricerca, i profili, la busta del processo.
_STRUTTURATE = ("BinaryNotFound", "ProfileError")


def _dallo_sdk(classe: Any, chiave: Any, /, *, remote_effect: Any = None, **parametri: Any) -> Any:
    """La fabbrica privata degli errori dell'SDK: costruisce e marca la provenienza.

    E' l'**unica** via per cui un errore conta come dell'SDK, e la rete lo
    ricostruisce invece di renderlo `UnexpectedError`. Il costruttore pubblico
    di una classe d'errore fa sempre un errore esterno, chiunque lo chiami.

    Non accetta testo: `chiave` sceglie un modello fisso di `MESSAGGI`, e i
    `parametri` si scrivono solo se rispettano il tipo che `PARAMETRI`
    dichiara per quella chiave (`?` altrimenti). Una chiave fuori catalogo da' il
    messaggio di `sdk.sconosciuto`, non un crash. Le classi che compongono il
    messaggio da se' -- `BinaryNotFound`, `ProfileError` e `CommandFailed` --
    nascono da `_dallo_sdk_strutturato`.

    Chiamarla da fuori e' usare un nome privato: fuori contratto (README,
    «Limiti dichiarati»).
    """
    if (
        not classe_dell_sdk(classe)
        or classe.__name__ in _STRUTTURATE
        or issubclass(classe, CommandFailed)
    ):
        raise _dallo_sdk(InvalidArgumentError, "fabbrica.non_sdk")
    testo = _testo(chiave, parametri)
    errore = classe(testo) if remote_effect is None else classe(
        testo, remote_effect=remote_effect)
    object.__getattribute__(errore, "__dict__")[PROVENIENZA] = True
    return errore


def _dallo_sdk_strutturato(classe: Any, *argomenti: Any) -> Any:
    """La fabbrica delle classi che compongono il messaggio da un vocabolario
    proprio: `BinaryNotFound` (i posti di `POSTI_DI_RICERCA`), `ProfileError`
    (i profili di `PROFILI_NOTI`) e `CommandFailed` con le sue sottoclassi (la
    busta che il processo ha scritto, validata da `ErrorEnvelope`)."""
    if not classe_dell_sdk(classe) or not (
        classe.__name__ in _STRUTTURATE or issubclass(classe, CommandFailed)
    ):
        raise _dallo_sdk(InvalidArgumentError, "fabbrica.non_sdk")
    errore = classe(*argomenti)
    object.__getattribute__(errore, "__dict__")[PROVENIENZA] = True
    return errore


def costruito_dall_sdk(errore: Any) -> bool:
    """`errore` e' di una classe dell'SDK, sigillato, e nato da `_dallo_sdk`."""
    if not classe_dell_sdk(type(errore)):
        return False
    stato = object.__getattribute__(errore, "__dict__")
    return dict.get(stato, SIGILLO) is True and dict.get(stato, PROVENIENZA) is True


def ricostruito(errore: Any, *argomenti: Any, **modifiche: Any) -> Any:
    """Un'eccezione **nuova** della stessa classe esatta, dallo stato fissato.

    Mai da `args`, `__notes__` o `str()` dell'oggetto ricevuto: il messaggio e'
    `_messaggio`, scritto dal costruttore e poi immutabile. La provenienza
    resta quella dell'originale, e lo stato non si sostituisce: la versione
    pubblica non accetta modifiche, perche' una sostituzione su un errore
    fidato -- `envelope=` di una busta del chiamante -- dava un errore fidato
    con i dati di chi chiama (undicesimo giro). Le sostituzioni che servono
    all'SDK passano da `_ricostruito`, privata.
    """
    if argomenti or modifiche:
        raise _dallo_sdk(InvalidArgumentError, "ricostruito.pubblico")
    return _ricostruito(errore)


#: Le sole voci dello stato che l'SDK sostituisce ricostruendo un errore, e
#: il valore che ciascuna ammette. `envelope`, `exit_code`, `command`, il
#: messaggio e la provenienza non si sostituiscono mai.
_SOSTITUIBILI: dict[str, Any] = {
    "_effetto_remoto": lambda valore: _effetto(valore),
    "_non_partito": lambda valore: valore is True,
    "cleanup_failed": lambda valore: valore is True,
}


def _ricostruito(errore: Any, **modifiche: Any) -> Any:
    """`ricostruito`, con le sostituzioni dell'SDK: l'effetto remoto che
    `Runner` alza a `unknown`, `_non_partito`, `cleanup_failed`. Ogni altra
    voce e' `InvalidArgumentError`: lo stato di un'eccezione si fissa una
    volta, e cambiarlo vuol dire farne un'altra, mai con dati nuovi."""
    tipo = type(errore)
    if not classe_dell_sdk(tipo):
        raise _dallo_sdk(InvalidArgumentError, "ricostruito.non_sdk")
    stato = _stato(errore)
    for nome, valore in modifiche.items():
        verifica = dict.get(_SOSTITUIBILI, nome) if type(nome) is str else None
        if verifica is None or verifica(valore) is False:
            raise _dallo_sdk(InvalidArgumentError, "ricostruito.voce")
        stato[nome] = valore
    return _da_copia(tipo, stato, provenienza=dict.get(stato, PROVENIENZA) is True)


def _da_copia(tipo: Any, stato: Any, provenienza: bool = False) -> Any:
    """L'eccezione di classe `tipo` con `stato`, senza passare dal costruttore.

    Da `copy` e `pickle` la provenienza e' sempre falsa: lo stato viene da un
    flusso che chiunque puo' scrivere, e una copia che attraversa la rete
    diventa `UnexpectedError` come un errore costruito dal chiamante.
    """
    if type(tipo) is not type or not classe_dell_sdk(tipo) or type(stato) is not dict:
        raise _dallo_sdk(InvalidArgumentError, "copia.stato_estraneo")
    testo = dict.get(stato, "_messaggio")
    if type(testo) is not str:
        raise _dallo_sdk(InvalidArgumentError, "copia.senza_messaggio")
    nuovo = BaseException.__new__(tipo)
    BaseException.__init__(nuovo, testo)
    dati = object.__getattribute__(nuovo, "__dict__")
    for nome, valore in dict.items(stato):
        if type(nome) is not str or nome in ATTRIBUTI_LIBERI:
            raise _dallo_sdk(InvalidArgumentError, "copia.voce")
        dati[nome] = valore
    dati[PROVENIENZA] = provenienza is True
    dati[SIGILLO] = True
    return nuovo


#: I posti in cui la scoperta cerca il binario: un elenco **chiuso** di
#: etichette fisse. `BinaryNotFound` conserva e riporta solo queste; ogni altro
#: testo -- per esempio un costruttore chiamato a mano con dati qualunque -- e'
#: redatto in `?`.
POSTI_DI_RICERCA: tuple[str, ...] = (
    "il percorso indicato con Client(binary=...)",
    "la variabile d'ambiente PLENORA_IO_BIN",
    "PLENORA_IO_BIN (non impostata)",
    "bin/plenora-io accanto al pacchetto, con MANIFEST.json",
    "il `PATH`",
)


def _posto(valore: Any) -> str:
    if type(valore) is str:
        for posto in POSTI_DI_RICERCA:
            if valore == posto:
                return posto
    return "?"


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

    def __init__(self, searched: Any) -> None:
        # I **posti**, non i percorsi: il valore della variabile d'ambiente, il
        # percorso indicato e il `PATH` sono della macchina di chi chiama, e un
        # errore non porta dati. Chi li vuole li legge dal proprio ambiente.
        voci = list.__iter__(searched) if type(searched) is list else ()
        # Una `FrozenList`: l'elenco e' stato dell'errore, e non si modifica.
        self.searched = FrozenList([_posto(dove) for dove in voci])
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
        # Gli attributi conservano un profilo solo se e' uno di quelli noti:
        # `required` viene da chi chiama e `actual` dal manifesto, un file, e
        # un valore fuori vocabolario sarebbe un dato. Al suo posto `None`.
        noti = PROFILI_NOTI
        self.required = required if type(required) is str and required in noti else None
        self.actual = actual if type(actual) is str and actual in noti else None
        # Il messaggio dai soli valori gia' filtrati: `actual` originale e'
        # del manifesto e `required` di chi chiama, e formattarli eseguirebbe
        # il loro `__format__`.
        if actual is None:
            quale = "sconosciuto: nessun manifesto"
        elif self.actual is not None:
            quale = f"«{self.actual}»"
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
        self._effetto_remoto = _effetto(remote_effect)


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

    Ogni metodo pubblico passa da `confine._confinato`: un'eccezione che non e'
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
        self._effetto_remoto = _effetto(remote_effect)


class PackageMetadataError(PlenoraError):
    """I metadati del pacchetto installato non si leggono.

    `version()` legge i metadati della distribuzione: da un checkout non
    installato non ci sono, e la risposta non e' `__version__`, che direbbe una
    cosa diversa da cio' che e' installato.
    """

    _CATEGORIA = "invalid_configuration"
    _FASE = "probe"


class ResultLookupError(PlenoraError, KeyError, IndexError):
    """Un nome cercato in un risultato -- layer, campo, driver, operazione -- non c'e'.

    E' anche un `KeyError`, come prima, perche' chi lo intercettava cosi' non
    debba cambiare, e un `IndexError`: un indice oltre un `FrozenList`. Il messaggio non riporta il nome cercato ne' quelli che ci
    sono: sono nomi del file o del chiamante, e un errore non porta dati. Chi
    vuole sapere che cosa c'e' legge il risultato.
    """

    _CATEGORIA = "not_found"
    _FASE = "validate"

    # `KeyError` mostra il `repr` dell'argomento; qui vale `PlenoraError.__str__`,
    # che viene prima nell'ordine di risoluzione e rende il messaggio fissato.


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
    esatti e resa immutabile: `retry` e `row_diagnostics` sono `FrozenDict`,
    con `FrozenList` e `FrozenDict` dentro (vedi `contenitori`), ricostruiti
    valore per valore. La busta non tiene mai un oggetto del chiamante, cio'
    che si valida e' cio' che si conserva, e nessun campo si modifica dopo:
    `busta.retry["kind"] = "safe"` e' `InvalidArgumentError`, come assegnare
    un campo.

    Le **decisioni** -- `retryable`, `retry_after_ms`,
    `must_assume_remote_committed` di `CommandFailed` -- non rileggono `retry`:
    usano il tipo di ritentativo, il ritardo e l'effetto remoto fissati in
    `__post_init__` dopo la validazione, in attributi privati che non sono
    campi della dataclass. `__post_init__` gira **solo** dal costruttore
    (`confine._solo_nel_costruttore`): richiamato a mano su una busta gia'
    costruita e' `InvalidArgumentError` prima di toccare un campo. Prima lo si
    poteva richiamare: rimetteva `retry` come `dict` ordinario, e una seconda
    chiamata dopo averlo cambiato riscriveva la decisione privata, anche
    quella di un `CommandFailed` gia' costruito. Il `CommandFailed` che porta
    la busta non la lascia sostituire: `errore.envelope = altra` e un secondo
    `__init__` sono `InvalidArgumentError` (vedi `PlenoraError`).

    `copy`, `deepcopy`, `pickle`, `dataclasses.asdict` e `dataclasses.replace`
    funzionano: i contenitori passano dai propri costruttori (`__reduce__`), e
    `replace` costruisce una busta nuova, che quindi si rivalida.

    Le decisioni private e i dati dei contenitori si raggiungono ancora con
    `object.__setattr__` o leggendo `FrozenDict._dati`: e' fuori contratto,
    come scritto nel README dell'SDK fra i limiti dichiarati («La rete non e'
    una barriera contro codice ostile nello stesso interprete»).
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
                self, campo, _copia_json(getattr(self, campo), f"error.{campo}")
            )
        if _tipo(self.retry) != "object":
            raise _dallo_sdk(ProtocolError, "busta.retry_non_oggetto", tipo=_tipo(self.retry))
        # La copia diventa subito un `FrozenDict`: nessun contenitore mutabile
        # entra mai in un campo, neanche fra qui e la normalizzazione del
        # costruttore, che sui contenitori gia' immutabili e' idempotente.
        object.__setattr__(
            self, "retry", FrozenDict(_copia_json(self.retry, "error.retry"))
        )
        if self.row_diagnostics is not None:
            if _tipo(self.row_diagnostics) != "object":
                raise _dallo_sdk(
                    ProtocolError, "busta.row_diagnostics",
                    tipo=_tipo(self.row_diagnostics),
                )
            object.__setattr__(
                self,
                "row_diagnostics",
                FrozenDict(
                    _copia_json(self.row_diagnostics, "error.details.row_diagnostics")
                ),
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
            raise _dallo_sdk(ProtocolError, "busta.non_oggetto", tipo=_tipo(documento))
        # Tutto il documento, non solo i campi che si leggono: una chiave che
        # non e' una `str` esatta o un valore che non e' JSON in un ramo
        # ignorato restano un documento che il wire non produce.
        documento = _copia_json(documento, "busta d'errore")
        errore = documento.get("error")
        if _tipo(errore) != "object":
            raise _dallo_sdk(ProtocolError, "busta.senza_error", tipo=_tipo(errore))
        mancanti = [
            campo
            for campo in ("code", "category", "phase", "remote_effect", "retry", "message")
            if campo not in errore
        ]
        if mancanti:
            raise _dallo_sdk(ProtocolError, "busta.mancanti", mancanti=mancanti)
        # `details` e' facoltativo, ma quando c'e' e' un oggetto: lo schema
        # `error-v1` non ammette `null`. Prima un `null` diventava `{}` in
        # silenzio mentre una stringa sollevava `AttributeError` -- due esiti
        # diversi per lo stesso difetto, nessuno dei due nominato.
        if "details" in errore and _tipo(errore["details"]) != "object":
            raise _dallo_sdk(ProtocolError, "busta.details", tipo=_tipo(errore['details']))
        dettagli = errore.get("details", {})
        if "row_diagnostics" in dettagli and _tipo(dettagli["row_diagnostics"]) != "object":
            raise _dallo_sdk(
                ProtocolError, "busta.details_row_diagnostics",
                tipo=_tipo(dettagli['row_diagnostics']),
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


#: I contesti dei documenti del protocollo: dove `_pretendi` e i `from_json`
#: dei modelli trovano un difetto. Un elenco chiuso di stringhe intere.
CONTESTI_DEL_PROTOCOLLO: frozenset[str] = frozenset({
    "risultato di --version", "catalog", "catalog.drivers[]", "catalog.drivers",
    "format", "fidelity", "fidelity.omesse", "fidelity.reasons[]", "fidelity.reasons",
    "layer.fields[]", "layers[].fields", "layer.geometry", "layer.geometry.crs_resolution",
    "inspect", "inspect.layers[]", "inspect.layers", "layers", "layers.layers[]",
    "layers.layers", "read", "read.delivered", "loss", "loss.counts[]", "loss.esempi[]",
    "loss.counts", "loss.esempi", "convert", "convert.layers[]", "convert.layers",
    "write", "write.input", "write.layers", "capabilities", "capabilities.interfaces[]",
    "capabilities.operations[]", "capabilities.operations[].controls",
    "capabilities.operations[].input",
})

#: I contenitori che `_copia_json` nomina: i contesti del protocollo, la busta
#: d'errore e le sue parti, il manifesto.
CONTESTI_JSON: frozenset[str] = CONTESTI_DEL_PROTOCOLLO | frozenset({
    "busta d'errore", "error.retry", "error.details.row_diagnostics", "MANIFEST.json",
    "error.code", "error.category", "error.phase", "error.remote_effect", "error.message",
})

#: I nomi degli argomenti che i metodi pubblici normalizzano.
ARGOMENTI: frozenset[str] = frozenset({
    "binary", "temp_dir", "options", "read_options", "write_options", "output",
    "destination", "source", "target", "source_format", "target_format", "assume_crs",
    "durable", "layer", "limit", "limits", "timeout", "argv", "format", "profile",
    "name", "identificatore", "categoria",
})

#: Che cosa la scoperta del binario esamina.
COSE_ESAMINATE: frozenset[str] = frozenset({
    "il pacchetto installato", "un candidato del binario", "il percorso indicato",
    "PLENORA_IO_BIN", "il binario accanto al pacchetto", "il binario dal PATH",
    "il manifesto dell'artefatto",
})

#: Le chiavi del manifesto sul filo.
CHIAVI_DEL_MANIFESTO: frozenset[str] = frozenset({
    "nome", "versione", "piattaforma", "profilo", "canale", "non_release", "revisione",
})

#: I campi obbligatori della busta d'errore.
CAMPI_DELLA_BUSTA: frozenset[str] = frozenset({
    "code", "category", "phase", "remote_effect", "retry", "message",
})

#: I tipi JSON come `_tipo` li nomina, e come `models._NOMI_DEI_TIPI` li scrive.
TIPI_JSON: frozenset[str] = frozenset({
    "null", "boolean", "integer", "number", "string", "array", "object", "non JSON",
})
NOMI_DEI_TIPI_JSON: frozenset[str] = frozenset({
    "un elenco", "un oggetto", "una stringa", "un intero", "un booleano", "null",
})

#: Lo stato della busta, come `process._stato` lo descrive.
STATI_DELLA_BUSTA: frozenset[str] = frozenset(
    {"«ok»", "«error»"} | {f"fuori vocabolario ({tipo})" for tipo in TIPI_JSON}
)

#: I motivi che `json.JSONDecodeError` scrive, per `json.non_valido`.
MOTIVI_JSON: frozenset[str] = frozenset({
    "Expecting value", "Expecting ',' delimiter", "Expecting ':' delimiter",
    "Expecting property name enclosed in double quotes", "Extra data",
    "Unterminated string starting at", "Invalid control character at",
    "Invalid \\escape", "Invalid \\uXXXX escape", "Invalid control character",
    "Illegal trailing comma before end of object",
    "Illegal trailing comma before end of array",
})


class _Parola:
    """Un parametro che e' una parola di un insieme chiuso, esatta."""

    __slots__ = ("insieme",)

    def __init__(self, insieme: Any) -> None:
        self.insieme = insieme  # un frozenset, o una funzione che lo rende

    def rendi(self, valore: Any) -> str:
        if type(valore) is not str:
            return "?"
        insieme = self.insieme() if callable(self.insieme) else self.insieme
        return valore if valore in insieme else "?"


class _Intero:
    """Un parametro intero esatto, in un intervallo dichiarato."""

    __slots__ = ("minimo", "massimo")

    def __init__(self, minimo: int, massimo: int) -> None:
        self.minimo, self.massimo = minimo, massimo

    def rendi(self, valore: Any) -> str:
        if type(valore) is not int or not self.minimo <= valore <= self.massimo:
            return "?"
        return int.__repr__(valore)


class _Alternative:
    """Una `tuple` di parametri dello stesso tipo, resa «a o b»."""

    __slots__ = ("tipo",)

    def __init__(self, tipo: Any) -> None:
        self.tipo = tipo

    def rendi(self, valore: Any) -> str:
        if type(valore) is not tuple or len(valore) > 16:
            return "?"
        return " o ".join(self.tipo.rendi(voce) for voce in valore)


class _Elenco:
    """Una `list` di parametri dello stesso tipo, resa `['a', 'b']`."""

    __slots__ = ("tipo",)

    def __init__(self, tipo: Any) -> None:
        self.tipo = tipo

    def rendi(self, valore: Any) -> str:
        if type(valore) is not list or len(valore) > 64:
            return "?"
        return "[" + ", ".join(f"'{self.tipo.rendi(voce)}'" for voce in valore) + "]"


class _ErroreFidato:
    """Un altro errore dell'SDK fidato: si rende con il suo messaggio."""

    __slots__ = ()

    def rendi(self, valore: Any) -> str:
        return str(valore) if costruito_dall_sdk(valore) else "?"


def _classi_dell_sdk() -> frozenset[str]:
    """I nomi delle classi dell'SDK, per identita' degli oggetti classe."""
    from .client import Client  # noqa: PLC0415 - il client importa questo modulo
    from .confine import MODELLI  # noqa: PLC0415 - confine importa questo modulo
    from .process import Runner  # noqa: PLC0415 - process importa questo modulo

    classi = [
        classe for classe in list(globals().values())
        if type(classe) is type and issubclass(classe, PlenoraError)
    ]
    classi += [*MODELLI, FrozenList, FrozenDict, Client, Runner]
    return frozenset(type.__dict__["__name__"].__get__(c, type) for c in classi)


def _luoghi_dei_modelli() -> frozenset[str]:
    """`Modello.campo` per ogni campo di ogni modello, e i due contenitori:
    i luoghi che la normalizzazione nomina."""
    import dataclasses  # noqa: PLC0415

    from .confine import MODELLI  # noqa: PLC0415

    luoghi = {"FrozenList", "FrozenDict"}
    for modello in MODELLI:
        nome = type.__dict__["__name__"].__get__(modello, type)
        luoghi.update(f"{nome}.{campo.name}" for campo in dataclasses.fields(modello))
    return frozenset(luoghi)


def _argomenti_e_luoghi() -> frozenset[str]:
    """I nomi d'argomento, i campi di `Limits` come `limits.campo` e i luoghi
    dei modelli (una durata in un modello passa da `_argomenti.durata`)."""
    import dataclasses  # noqa: PLC0415

    from .limits import Limits  # noqa: PLC0415

    nomi = set(ARGOMENTI) | {f"limits.{c.name}" for c in dataclasses.fields(Limits)}
    return frozenset(nomi) | _luoghi_dei_modelli()


def _campi_del_protocollo() -> frozenset[str]:
    """I nomi dei campi dei documenti, dagli schemi dei modelli per identita'."""
    import dataclasses  # noqa: PLC0415

    from .confine import MODELLI  # noqa: PLC0415

    campi: set[str] = set()
    for modello in MODELLI:
        campi.update(campo.name for campo in dataclasses.fields(modello))
        for schema in ("OBBLIGATORI", "PROPRI", "OPZIONALI", "ANNULLABILI"):
            campi.update(v for v in vars(modello).get(schema, ()) if type(v) is str)
        for chiave, valore in dict(vars(modello).get("RINOMINATI", {})).items():
            campi.update(v for v in (chiave, valore) if type(v) is str)
    return frozenset(campi)


def _etichette() -> Any:
    """Le etichette che la rete ha dato ai metodi e alle funzioni esportate."""
    from .confine import ETICHETTE  # noqa: PLC0415

    return ETICHETTE


_GRANDE = 1 << 62
_CLASSE = _Parola(_classi_dell_sdk)
_ARGOMENTO = _Parola(_argomenti_e_luoghi)
_LUOGO = _Parola(_luoghi_dei_modelli)
_CONTESTO = _Parola(CONTESTI_DEL_PROTOCOLLO)
_CONTESTO_JSON = _Parola(CONTESTI_JSON)
_CAMPO = _Parola(_campi_del_protocollo)
_TIPO = _Parola(TIPI_JSON)
_COMANDO = _Parola(SOTTOCOMANDI)
_FLUSSO = _Parola(frozenset({"stdout", "stderr"}))
_ERRORE = _ErroreFidato()
_CONTEGGIO = _Intero(0, _GRANDE)
_PROFONDITA = _Intero(0, 1_000_000)

#: Il tipo di ogni parametro di ogni modello di `MESSAGGI`. Una prova verifica
#: che i parametri dichiarati siano esattamente i segnaposto del modello.
PARAMETRI: dict[str, dict[str, Any]] = {
    "errore.stato_sigillato": {"nome": _CLASSE},
    "errore.gia_costruito": {"nome": _CLASSE},
    **{
        chiave: {"nome": _ARGOMENTO}
        for chiave in MESSAGGI
        if chiave.startswith("arg.")
    },
    "arrow.serie": {"attesa": _Intero(0, 10_000), "serie": _Intero(0, 10_000)},
    "client.non_decodifica": {"comando": _COMANDO, "modello": _CLASSE},
    "rete.imprevisto": {"etichetta": _Parola(_etichette)},
    "modello.oggetto_non_ammesso": {"dove": _LUOGO},
    "modello.immutabile": {"nome": _CLASSE},
    "modello.gia_costruito": {"nome": _CLASSE},
    "modello.post_init": {"nome": _CLASSE},
    "modello.chiave_non_stringa": {"dove": _LUOGO},
    "modello.chiavi_coincidenti": {"dove": _LUOGO},
    "modello.annidato": {"dove": _LUOGO},
    "contenitore.argomenti": {"nome": _CLASSE},
    "contenitore.non_si_estende": {"nome": _CLASSE},
    "fs.non_esaminabile": {"che_cosa": _Parola(COSE_ESAMINATE)},
    "fs.non_risolvibile": {"che_cosa": _Parola(COSE_ESAMINATE)},
    "manifesto.non_oggetto": {"tipo": _TIPO},
    "manifesto.mancanti": {"mancanti": _Elenco(_Parola(CHIAVI_DEL_MANIFESTO))},
    "manifesto.tipo_campo": {
        "campo": _Parola(CHIAVI_DEL_MANIFESTO), "tipo": _TIPO, "tipi": _Alternative(_TIPO),
    },
    "manifesto.da_errore": {"errore": _ERRORE},
    "manifesto.non_json": {"errore": _ERRORE},
    "json.costante": {"nome": _Parola(frozenset({"NaN", "Infinity", "-Infinity"}))},
    "json.non_valido": {
        "motivo": _Parola(MOTIVI_JSON), "riga": _Intero(1, _GRANDE),
        "colonna": _Intero(1, _GRANDE),
    },
    "json.profondo": {"dove": _CONTESTO_JSON},
    "json.chiave_non_stringa": {
        "dove": _CONTESTO_JSON, "profondita": _PROFONDITA, "tipo": _TIPO,
    },
    "json.non_finito": {"dove": _CONTESTO_JSON, "profondita": _PROFONDITA},
    "json.non_esatto": {"dove": _CONTESTO_JSON, "profondita": _PROFONDITA},
    "busta.non_oggetto": {"tipo": _TIPO},
    "busta.senza_error": {"tipo": _TIPO},
    "busta.mancanti": {"mancanti": _Elenco(_Parola(CAMPI_DELLA_BUSTA))},
    "busta.details": {"tipo": _TIPO},
    "busta.details_row_diagnostics": {"tipo": _TIPO},
    "busta.retry_non_oggetto": {"tipo": _TIPO},
    "busta.row_diagnostics": {"tipo": _TIPO},
    "busta.campo_testuale": {"campo": _Parola(CAMPI_DELLA_BUSTA), "tipo": _TIPO},
    "busta.effetto": {"tipo": _TIPO, "ammessi": _Elenco(_Parola(EFFETTI_REMOTI))},
    "busta.retry_kind": {"tipo": _TIPO, "ammessi": _Elenco(_Parola(TIPI_DI_RITENTATIVO))},
    "busta.retry_chiavi": {
        "tipo": _Parola(TIPI_DI_RITENTATIVO), "chiavi": _CONTEGGIO,
        "attese": _Elenco(_Parola(frozenset({"kind", "delay_ms"}))),
    },
    "busta.ritardo": {"tipo": _TIPO, "massimo": _CONTEGGIO},
    "busta.effetto_ignoto": {
        "tipo": _Parola(TIPI_DI_RITENTATIVO), "ammessi": _Elenco(_Parola(TIPI_DI_RITENTATIVO)),
    },
    "protocollo.non_oggetto": {"dove": _CONTESTO, "tipo": _TIPO},
    "protocollo.mancanti": {"dove": _CONTESTO, "mancanti": _Elenco(_CAMPO)},
    "protocollo.tipo_campo": {
        "dove": _CONTESTO, "campo": _CAMPO, "tipo": _TIPO,
        "ammessi": _Alternative(_Parola(NOMI_DEI_TIPI_JSON)),
    },
    "protocollo.tipo_elemento": {
        "dove": _CONTESTO, "campo": _CAMPO, "posizione": _CONTEGGIO, "tipo": _TIPO,
        "ammessi": _Alternative(_Parola(NOMI_DEI_TIPI_JSON)),
    },
    "protocollo.non_elenco": {"dove": _CONTESTO, "tipo": _TIPO},
    "versione.campi_in_piu": {"campi": _CONTEGGIO},
    "processo.argomento_non_passabile": {"comando": _COMANDO},
    "processo.non_eseguibile": {"comando": _COMANDO},
    "processo.chiusura": {"comando": _COMANDO},
    # Senza il valore del timeout: e' un numero scelto da chi chiama, e un
    # decimale scritto per intero e' un dato (quindicesimo giro).
    "processo.timeout": {"comando": _COMANDO},
    "processo.comunicazione": {"comando": _COMANDO},
    "processo.non_utf8": {"comando": _COMANDO, "flusso": _FLUSSO, "byte": _CONTEGGIO},
    "processo.stderr_successo": {"comando": _COMANDO, "caratteri": _CONTEGGIO},
    "processo.stato_successo": {"comando": _COMANDO, "stato": _Parola(STATI_DELLA_BUSTA)},
    "processo.senza_result": {"comando": _COMANDO},
    "processo.stato_errore": {
        "comando": _COMANDO, "uscita": _Intero(-(1 << 63), 1 << 63),
        "stato": _Parola(STATI_DELLA_BUSTA),
    },
    "processo.stderr_errore": {"comando": _COMANDO, "caratteri": _CONTEGGIO},
    "processo.flusso_vuoto": {
        "comando": _COMANDO, "uscita": _Intero(-(1 << 63), 1 << 63), "flusso": _FLUSSO,
        "altro": _FLUSSO, "caratteri": _CONTEGGIO,
    },
    "processo.busta_illeggibile": {
        "comando": _COMANDO, "flusso": _FLUSSO, "errore": _ERRORE, "caratteri": _CONTEGGIO,
    },
    "processo.busta_non_oggetto": {"flusso": _FLUSSO, "tipo": _TIPO},
}


def _testo(chiave: Any, parametri: dict[str, Any]) -> str:
    """Il messaggio di `chiave`, con ogni parametro reso dal tipo che il
    catalogo gli dichiara (`PARAMETRI`). Un parametro senza tipo dichiarato,
    fuori dal suo insieme o fuori intervallo, o che fa sollevare il rendering,
    e' `?`: nessun errore smette di nascere per un parametro."""
    import string  # noqa: PLC0415

    modello = dict.get(MESSAGGI, chiave) if type(chiave) is str else None
    if modello is None:
        chiave, modello = "sdk.sconosciuto", MESSAGGI["sdk.sconosciuto"]
    tipi = dict.get(PARAMETRI, chiave, {})
    pezzi = []
    for letterale, campo, _formato, _conversione in string.Formatter().parse(modello):
        pezzi.append(letterale)
        if campo is None:
            continue
        tipo = dict.get(tipi, campo)
        if tipo is None or campo not in parametri:
            pezzi.append("?")
            continue
        try:
            pezzi.append(tipo.rendi(dict.get(parametri, campo)))
        except Exception:  # noqa: BLE001 - un parametro non fa mai fallire l'errore
            pezzi.append("?")
    return "".join(pezzi)


def _effetto(valore: Any) -> str:
    """Un effetto remoto del vocabolario chiuso, o `InvalidArgumentError`."""
    if type(valore) is not str or valore not in EFFETTI_REMOTI:
        raise _dallo_sdk(InvalidArgumentError, "effetto.non_valido")
    return valore


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
    if tipo is list or tipo is FrozenList or tipo is tuple:
        # `tuple` e `FrozenDict` sono le forme immutabili dei modelli: chi
        # ricostruisce un modello da un altro (`dataclasses.replace`) le passa.
        return "array"
    if tipo is dict or tipo is FrozenDict:
        return "object"
    return "non JSON"


def _copia_json(valore: Any, dove: str) -> Any:
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
        return _copia_ricorsiva(valore, dove, 0)
    except RecursionError:
        raise _dallo_sdk(ProtocolError, "json.profondo", dove=dove) from None


def _copia_ricorsiva(valore: Any, dove: str, _profondita: int) -> Any:
    tipo = _tipo(valore)
    if tipo == "object":
        copia = {}
        for chiave, interno in valore.items():
            if _tipo(chiave) != "string":
                raise _dallo_sdk(
                    ProtocolError, "json.chiave_non_stringa",
                    dove=dove,
                    profondita=_profondita + 1,
                    tipo=_tipo(chiave),
                )
            copia[chiave] = _copia_ricorsiva(interno, dove, _profondita + 1)
        return copia
    if tipo == "array":
        return [_copia_ricorsiva(interno, dove, _profondita + 1) for interno in valore]
    if tipo == "number" and not math.isfinite(valore):
        raise _dallo_sdk(ProtocolError, "json.non_finito", dove=dove, profondita=_profondita)
    if tipo == "non JSON":
        raise _dallo_sdk(ProtocolError, "json.non_esatto", dove=dove, profondita=_profondita)
    return valore


def _rifiuta_costante(nome: str) -> Any:
    raise _dallo_sdk(ProtocolError, "json.costante", nome=nome)


def _float_finito(testo: str) -> float:
    numero = float(testo)
    if not math.isfinite(numero):
        # `1e400` non e' una costante: e' un numero che in doppia precisione
        # trabocca in infinito, e `parse_constant` non lo vede.
        raise _dallo_sdk(ProtocolError, "json.numero_infinito")
    return numero


def _rifiuta_chiavi_doppie(coppie: list[tuple[str, Any]]) -> dict[str, Any]:
    oggetto: dict[str, Any] = {}
    for chiave, valore in coppie:
        if chiave in oggetto:
            raise _dallo_sdk(ProtocolError, "json.chiave_ripetuta")
        oggetto[chiave] = valore
    return oggetto


def _riga_e_colonna(testo: str, posizione: Any) -> tuple[Any, Any]:
    """Riga e colonna (da 1) di `posizione` in `testo`, una `str` esatta.

    Con una posizione che non e' un intero esatto dentro il testo rende
    `(None, None)`, che il messaggio scrive `?`.
    """
    if type(posizione) is not int or not 0 <= posizione <= len(testo):
        return None, None
    riga = str.count(testo, "\n", 0, posizione) + 1
    colonna = posizione - str.rfind(testo, "\n", 0, posizione)
    return riga, colonna


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
    # Una `str` esatta prima del parser: con una sottoclasse, `json.loads`
    # chiama i suoi metodi (`count`, `rfind`, `startswith`...) e l'eccezione
    # che ne esce porterebbe numeri e motivo scelti dal chiamante.
    if type(testo) is not str:
        if not issubclass(type(testo), str):
            raise _dallo_sdk(ProtocolError, "json.non_testo")
        testo = str.join("", [testo])
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
        # Il motivo solo se e' uno di quelli che `json` scrive (`MOTIVI_JSON`);
        # riga e colonna le ricalcola l'SDK dal testo esatto e dalla posizione,
        # verificata, invece di prenderle dall'eccezione.
        riga, colonna = _riga_e_colonna(testo, errore.pos)
        sostituto = _dallo_sdk(
            ProtocolError, "json.non_valido", motivo=errore.msg, riga=riga, colonna=colonna,
        )
    except RecursionError:
        sostituto = _dallo_sdk(ProtocolError, "json.troppo_annidato")
    except ValueError:
        sostituto = _dallo_sdk(ProtocolError, "json.non_convertibile")
    raise sostituto from None


def _valida_busta(busta: "ErrorEnvelope") -> None:
    """Tipi e vocabolari chiusi della busta, o `ProtocolError`.

    I messaggi nominano il campo e il tipo trovato, non il valore: il valore
    viene da un processo esterno e non c'e' ragione di ricopiarlo.
    """
    for campo in ("code", "category", "phase", "message"):
        valore = getattr(busta, campo)
        if _tipo(valore) != "string" or not valore:
            raise _dallo_sdk(
                ProtocolError, "busta.campo_testuale",
                campo=campo,
                tipo=_tipo(valore),
            )

    effetto = busta.remote_effect
    if _tipo(effetto) != "string" or effetto not in EFFETTI_REMOTI:
        raise _dallo_sdk(
            ProtocolError, "busta.effetto",
            tipo=_tipo(effetto),
            ammessi=sorted(EFFETTI_REMOTI),
        )

    # `retry` e' gia' la copia fatta in `__post_init__`.
    retry = busta.retry
    tipo = retry.get("kind")
    if _tipo(tipo) != "string" or tipo not in TIPI_DI_RITENTATIVO:
        raise _dallo_sdk(
            ProtocolError, "busta.retry_kind",
            tipo=_tipo(tipo),
            ammessi=sorted(TIPI_DI_RITENTATIVO),
        )
    attese = {"kind", "delay_ms"} if tipo == "after" else {"kind"}
    if set(retry) != attese:
        # Le chiavi in piu' non si nominano: vengono dal documento.
        raise _dallo_sdk(
            ProtocolError, "busta.retry_chiavi",
            tipo=tipo,
            chiavi=len(retry),
            attese=sorted(attese),
        )
    if tipo == "after":
        ritardo = retry["delay_ms"]
        if _tipo(ritardo) != "integer" or not 0 <= ritardo <= RITARDO_MASSIMO_MS:
            raise _dallo_sdk(
                ProtocolError, "busta.ritardo",
                tipo=_tipo(ritardo),
                massimo=RITARDO_MASSIMO_MS,
            )
    if effetto == "unknown" and tipo not in RITENTATIVI_CON_EFFETTO_IGNOTO:
        raise _dallo_sdk(
            ProtocolError, "busta.effetto_ignoto",
            tipo=tipo,
            ammessi=sorted(RITENTATIVI_CON_EFFETTO_IGNOTO),
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
        if type(envelope) is not ErrorEnvelope:
            raise _dallo_sdk(InvalidArgumentError, "fallimento.envelope")
        if type(exit_code) is bool or not issubclass(type(exit_code), int):
            raise _dallo_sdk(InvalidArgumentError, "fallimento.exit_code")
        self.envelope = envelope
        self.exit_code = int.__pos__(exit_code)
        # Il solo sottocomando, da un elenco chiuso. Fino alla 4.1.1 l'errore
        # conservava `argv` intero -- percorsi, opzioni, valori di chi chiama
        # -- in un attributo che finisce nei log e nei pickle. Chi ha fatto la
        # chiamata ha gia' gli argomenti.
        self.command = sottocomando(argv)
        super().__init__(_testo_del_fallimento(envelope, self.exit_code, self.command))

    def __str__(self) -> str:
        # Dallo stesso stato di `message` e `to_dict()`: la busta, l'uscita e
        # il sottocomando fissati, non un testo conservato a parte che una
        # ricostruzione potesse far divergere.
        stato = object.__getattribute__(self, "__dict__")
        return _testo_del_fallimento(
            dict.get(stato, "envelope"), dict.get(stato, "exit_code"), dict.get(stato, "command")
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
        # La decisione fissata alla costruzione, dopo la validazione, e non il
        # campo pubblico: e' cio' che le altre decisioni leggono.
        return self.envelope._effetto_remoto

    @property
    def retry(self) -> dict[str, Any]:
        """La disposizione fissata alla costruzione, come oggetto nuovo.

        Ricostruita dalle decisioni private della busta, fissate dopo la
        validazione, e resa come `dict` nuovo a ogni lettura: chi la cambia
        cambia la propria copia, non l'eccezione ne' la busta.
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


def _testo_del_fallimento(envelope: Any, exit_code: Any, command: Any) -> str:
    if type(envelope) is not ErrorEnvelope or type(exit_code) is not int:
        return "(errore dell'SDK senza messaggio)"
    nome = command if type(command) is str else "?"
    return (
        f"`plenora-io {nome}` e' uscito con {exit_code}: "
        f"[{envelope.category}/{envelope.phase}] "
        f"{envelope.code}: {envelope.message}"
    )


def _classe_della_busta(documento: Any) -> tuple[type[CommandFailed], ErrorEnvelope]:
    envelope = ErrorEnvelope.from_json(documento)
    return CATEGORIE.get(envelope.category, CommandFailed), envelope


def failure_from_envelope(
    documento: dict[str, Any], exit_code: int, argv: list[str]
) -> CommandFailed:
    """La busta d'errore, come eccezione della classe che le compete.

    Una categoria **sconosciuta** non e' un errore dell'SDK: le regole di
    compatibilita' del protocollo consentono di estendere un vocabolario
    chiuso, e un SDK che si rifiutasse di leggere la busta trasformerebbe
    un'estensione in un guasto. Si ripiega su `CommandFailed`, che porta la
    categoria intatta: chi la conosce la legge da `envelope.category`.

    Pubblica, e quindi **esterna**: la busta e' di chi chiama, e l'errore che
    ne nasce non e' dell'SDK. Attraverso la rete diventa `UnexpectedError`.
    L'SDK, per le buste che legge dal processo, usa `_failure_from_envelope`.
    """
    classe, envelope = _classe_della_busta(documento)
    # Chiamata dinamica, apposta senza la fabbrica: l'errore e' esterno.
    return classe(envelope, exit_code, argv)


def _failure_from_envelope(
    documento: dict[str, Any], exit_code: int, argv: list[str]
) -> CommandFailed:
    """`failure_from_envelope` per la busta che il processo ha scritto: fidato."""
    classe, envelope = _classe_della_busta(documento)
    return _dallo_sdk_strutturato(classe, envelope, exit_code, argv)


# In fondo: `contenitori` importa `InvalidArgumentError` da qui, e `_tipo` deve
# riconoscere i due contenitori per identita'.
from .contenitori import FrozenDict, FrozenList  # noqa: E402
