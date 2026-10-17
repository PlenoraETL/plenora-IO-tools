"""L'esecuzione del binario, e la lettura dei due flussi.

# Perche' un modulo suo

Ogni comando fa la stessa cosa: costruisce una riga di argomenti, esegue,
decodifica una busta. La prima stesura teneva quel codice dentro il client,
accanto ai due metodi che lo usavano; con cinque comandi sarebbe stato copiato o
generalizzato in fretta, e il posto dove generalizzarlo e' questo.

# I due flussi hanno ruoli diversi, e il protocollo lo dice

Un flusso solo: la busta va su **stdout**, riuscita o fallita che sia, e
`stderr` resta vuoto in entrambi i casi. Lo `status` della busta dice quale dei
due esiti si ha in mano, e il codice d'uscita lo proietta.

Fino alla 3.0.0 la busta d'errore usciva su `stderr`, e questo esecutore la
cercava li'. Chi leggeva `stdout` -- cioe' cio' che il contratto descrive -- non
vedeva il fallimento affatto.

La prima stesura cercava un JSON prima su stdout e poi su stderr, prendendo il
primo che si decodificasse. Funzionava, e diceva una cosa piu' debole del vero:
che la busta sta su uno dei due. Con quella regola una busta d'errore scritta
per sbaglio su stdout sarebbe stata letta come un successo, e nessuno se ne
sarebbe accorto -- perche' `status` lo si guarda **dopo** aver scelto il flusso.

Qui il flusso si sceglie dal codice d'uscita, che e' cio' che il protocollo
lega all'esito, e il contenuto dell'altro flusso e' verificato invece che
ignorato.

# Perche' `stderr` non vuoto con successo **e'** un errore

Il protocollo v2 dice che con successo `stderr` resta vuoto, e questo esecutore
parla v2 e nient'altro. Qualunque cosa vi compaia e' percio' una violazione del
contratto, e va detta.

La prima stesura la tollerava, per il protocollo legacy: quello scrive su
`stderr` un avviso quando lo si sceglie. Era una tolleranza **implicita** --
l'SDK non espone quel flag, e nessuno l'aveva chiesta -- e il suo costo e' che
rendeva invisibile la sola forma in cui il v2 puo' sporcare quel flusso: un
avviso non previsto, una traccia di debug, la riga di una libreria che scrive
dove non deve. Un consumatore che compone la CLI in una pipeline se ne
accorgerebbe da un log corrotto, non da qui.

Se un giorno il v1 servira', avra' un percorso suo, dichiarato: un protocollo
diverso si sceglie, non si indovina dal fatto che qualcosa e' comparso su
`stderr`.

«Vuoto» vuol dire **zero byte**, non «niente di significativo»: anche uno spazio
e' contenuto. Un confine che ammettesse gli spazi lascerebbe passare una
scrittura accidentale vuota, che e' la forma in cui una violazione arriva senza
che nessuno l'abbia voluta.

# Il primo SIGINT si inoltra, e la cancellazione la fa il prodotto

Un Ctrl-C durante una conversione lunga deve fermarla **con grazia**: il
prodotto arma al primo segnale un token che la pipeline osserva ai propri punti
di verifica, e da li' torna un errore `CANCELLED` con la destinazione ripulita.
Al secondo segnale esce con 130.

Senza inoltro, il Ctrl-C arriverebbe soltanto al processo Python: la CLI
resterebbe viva a scrivere, o morirebbe di colpo lasciando uno staging sul
disco. L'esecutore installa percio' un gestore **per la durata della singola
esecuzione**, e lo rimette com'era subito dopo: una libreria che si
approprriasse del gestore del processo cambierebbe il comportamento di codice
che non l'ha chiamata.

Il gestore si puo' installare solo dal thread principale -- e' Python a
imporlo -- e fuori di li' l'esecuzione procede senza inoltro. Non e' un
fallimento chiuso e non deve esserlo: chiamare `convert()` da un thread e' un
uso legittimo, e rifiutarlo romperebbe programmi che funzionano per una
comodita' che non riguarda la correttezza di cio' che esce.
`Runner.sigint_forwarding_available` lo dice, per chi ha bisogno di saperlo
invece di scoprirlo.
"""

from __future__ import annotations

import contextlib
import signal
import subprocess
import sys
import threading
import time
import weakref
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from ._argomenti import percorso, testo
from ._argomenti import timeout as timeout_valido
from ._confine import registra_avvio
from ._errors import (
    InvalidArgumentError,
    ProtocolError,
    _tipo,
    carica_json,
    _failure_from_envelope,
    _ricostruito,
    costruito_dall_sdk,
    sottocomando,
)
from ._errors import _dallo_sdk
from ._contenitori import FrozenDict, _memo, _protocollo



@contextlib.contextmanager
def _inoltra_sigint(processo: "subprocess.Popen[str]"):
    """Inoltra SIGINT al figlio, per la durata di questo blocco.

    Il gestore precedente viene ripristinato all'uscita, sempre: una libreria
    non e' padrona del gestore dei segnali del processo che la ospita.

    Su Windows `SIGINT` a un processo figlio non si manda: `os.kill` con quel
    segnale uccide il processo invece di consegnarglielo, e
    `CTRL_C_EVENT` va all'intero gruppo -- compreso chi ci ospita. Li' l'inoltro
    non si arma, e la cancellazione con grazia non e' disponibile.
    """
    if not sigint_inoltrabile():
        yield
        return

    def inoltra(numero, frame):  # noqa: ARG001 - la firma la impone `signal`
        # Non si solleva `KeyboardInterrupt` qui: il figlio deve ricevere il
        # segnale e chiudere da se', e un'eccezione sollevata adesso
        # abbandonerebbe la `communicate` lasciando il processo vivo.
        with contextlib.suppress(ProcessLookupError, OSError):
            processo.send_signal(signal.SIGINT)

    primaria = False
    precedente = signal.signal(signal.SIGINT, inoltra)
    try:
        yield
    except BaseException:  # noqa: BLE001 - il ripristino non sostituisce l'attesa
        primaria = True
        raise
    finally:
        try:
            signal.signal(signal.SIGINT, precedente)
        except BaseException:  # noqa: BLE001
            if not primaria:
                raise


@contextlib.contextmanager
def _sigint_nativo():
    """Come l'inoltro POSIX: sospende il gestore utente, poi lo ripristina.

    Il callback non solleva: Rust cancella e attende il comando. Funziona
    anche su Windows, dove non occorre inoltrare un segnale a un figlio.
    """
    richiesta = False

    def registra(numero, frame):
        nonlocal richiesta
        richiesta = True

    def interrotto():
        return richiesta

    if threading.current_thread() is not threading.main_thread():
        yield interrotto
        return
    precedente = signal.signal(signal.SIGINT, registra)
    primaria = False
    try:
        yield interrotto
    except BaseException:
        primaria = True
        raise
    finally:
        try:
            signal.signal(signal.SIGINT, precedente)
        except BaseException:
            if not primaria:
                raise


def sigint_inoltrabile() -> bool:
    """L'inoltro del segnale e' possibile in questo contesto.

    Due condizioni, ed e' Python a imporle entrambe: il gestore si installa solo
    dal thread principale, e su Windows non c'e' modo di consegnare un `SIGINT`
    a un figlio senza colpire chi lo ospita.
    """
    return (
        sys.platform != "win32"
        and threading.current_thread() is threading.main_thread()
    )


@dataclass(frozen=True)
class Completed:
    """Che cosa e' uscito da un'esecuzione, prima di darle un significato."""

    argv: list[str]
    exit_code: int
    stdout: str
    stderr: str

    def stream(self, name: str) -> str:
        return self.stdout if name == "stdout" else self.stderr


def _nome_del_comando(argv: Any) -> str:
    """Il sottocomando per i messaggi, dall'elenco chiuso; `?` altrimenti."""
    return sottocomando(argv) or "?"


def comando(argv: list[str]) -> str:
    """Il comando come compare nei messaggi: `plenora-io <nome>`, e basta.

    La riga intera portava percorsi e opzioni -- dati di chi chiama, e a volte
    valori della sorgente -- dentro un errore che finisce nei log. Il nome del
    sottocomando dice gia' quale operazione e' fallita; gli argomenti li ha chi
    ha fatto la chiamata.
    """
    if not argv:
        return "`plenora-io`"
    # Da un elenco chiuso: un primo argomento qualunque, anche alfanumerico,
    # e' un dato di chi chiama.
    return f"`plenora-io {sottocomando(argv) or '?'}`"


def _testo(dati: Any, argv: list[str], flusso: str) -> str:
    """I byte di un flusso come testo UTF-8, o `ProtocolError`.

    Il protocollo v2 scrive UTF-8. `communicate()` con `text=True` decodificava
    da se' e un byte fuori codifica usciva come `UnicodeDecodeError`, fuori
    dalla gerarchia dell'SDK e senza gli assi d'errore. Ora si leggono i byte e
    si decodificano qui, dove un fallimento ha un nome.
    """
    try:
        return dati.decode("utf-8")
    except UnicodeDecodeError:
        raise _dallo_sdk(
            ProtocolError, "processo.non_utf8",
            comando=_nome_del_comando(argv), flusso=flusso, byte=len(dati),
        ) from None


def _stato(valore: Any) -> str:
    """Lo `status` della busta se e' del vocabolario, altrimenti il suo tipo."""
    if _tipo(valore) == "string" and valore in ("ok", "error"):
        return f"«{valore}»"
    return f"fuori vocabolario ({_tipo(valore)})"


def scrive(argv: list[str]) -> bool:
    """Il comando pubblica qualcosa sul filesystem.

    `write` e `convert` sempre; `read` solo con una destinazione, che e' la
    forma che consegna. Gli altri leggono e basta.
    """
    if not argv:
        return False
    primo = argv[0]
    if type(primo) is not str:
        return False
    if primo == "write" or primo == "convert":
        return True
    return primo == "read" and any(
        type(a) is str and a == "--output" for a in argv
    )


def _dopo_l_avvio(
    errore: BaseException, argv: list[str], chiave: str = "processo.risposta_illeggibile",
    **parametri: Any,
) -> ProtocolError:
    """Il gestore unico dei guasti dopo che il processo e' partito.

    Un `ProtocolError` dell'SDK resta se stesso; ogni altro guasto -- un errore
    dell'SDK gia' fidato di un'altra classe, un'eccezione di Python o di una
    dipendenza, un `MemoryError` -- diventa un `ProtocolError` nuovo (`chiave`),
    che non porta niente dell'eccezione ricevuta. Per un comando che scrive
    l'effetto e' `unknown`, mai `none`: che cosa sia rimasto sul disco non lo
    dice una risposta che non si legge. Per un comando che legge soltanto
    resta `none`. Il ritentativo resta `never` (README).

    Ventesimo giro: prima `registra_avvio`, la decodifica dei flussi e
    `Completed` stavano fuori dal gestore, e un errore dell'SDK gia' fidato
    usciva con effetto `none`.
    """
    if type(errore) is ProtocolError and costruito_dall_sdk(errore):
        risultato = errore
    else:
        risultato = _dallo_sdk(
            ProtocolError, chiave, comando=_nome_del_comando(argv), **parametri)
    stato = object.__getattribute__(risultato, "__dict__")
    if scrive(argv) and dict.get(stato, "_non_partito") is not True:
        return _ricostruito(risultato, _effetto_remoto="unknown")
    return risultato


#: La nota, fissa e senza dati, che un'interruzione porta quando ferma un
#: comando che scrive dopo l'avvio.
NOTA_DELL_INTERRUZIONE = (
    "plenora-io: un comando che scrive e' stato interrotto dopo l'avvio; il "
    "processo e' stato chiuso, e l'effetto remoto va considerato unknown."
)


#: Quanto si aspetta un processo dopo `kill`, per ciascun tentativo, e quanti
#: tentativi; quanto si aspettano i lettori dopo che il processo e' uscito.
_ATTESA_DELLA_CHIUSURA = 5.0
_ATTESA_DEI_LETTORI = 5.0

#: Il passo con cui un lettore che ha finito di leggere guarda se il run e'
#: finito: la conclusione impostata, o il gettone del run morto.
_PASSO_DEL_LETTORE = 0.1

#: L'attesa dell'ultimo tentativo di terminare il figlio, nel `finally` di
#: `run`.
_ATTESA_DELL_ULTIMO_TENTATIVO = 1.0


class _GettoneDelRun:
    """Un oggetto per run, tenuto **solo** da una variabile locale del frame
    di `run`; i lettori ne hanno un `weakref`. Quando muore il frame si e'
    srotolato: il run e' finito, anche se un'interruzione asincrona ha
    saltato `conclusione.set()`.

    Limite dichiarato (README, `docs/RELEASE.md`): un'eccezione conservata
    con il suo traceback trattiene il frame, e con lui il gettone e i
    lettori, finche' vive; senza conteggio dei riferimenti (PyPy) la morte
    del gettone puo' essere differita fino al garbage collector."""

    __slots__ = ("__weakref__",)

#: Gli oggetti di coordinamento dei lettori, costruiti **prima** di `Popen`:
#: dopo l'avvio non c'e' niente fuori dal blocco protetto. Funzioni del
#: modulo perche' le prove le possano sostituire.
_nuovo_evento = threading.Event

#: Il passo dell'attesa del processo: il thread chiamante aspetta a passi
#: brevi, e fra un passo e l'altro guarda i lettori e la scadenza.
_PASSO = 0.05

#: L'orologio dell'attesa: monotono, e una funzione del modulo perche' le
#: prove lo possano leggere senza toccare `time`.
_adesso = time.monotonic

#: La nota, fissa e senza dati, su un'interruzione la cui chiusura del figlio
#: non e' riuscita del tutto.
NOTA_DELLA_CHIUSURA = (
    "plenora-io: la chiusura del processo figlio non e' riuscita del tutto; "
    "potrebbe essere ancora in esecuzione."
)


class _Lettore:
    """Un thread dell'SDK che legge un flusso fino alla fine.

    Cattura **ogni** eccezione e non la stampa mai; il guasto lo segnala su
    un `threading.Event` condiviso (`guasto`), e non tocca il processo: solo
    il thread chiamante chiama `wait` e `kill`.

    Il flusso e' del lettore finche' il chiamante non lo **reclama** e non
    conferma di averlo chiuso (ventiquattresimo giro). Il default e' sicuro:
    a fine run (`conclusione`, un `Event` impostato nel `finally` di `run`)
    il lettore guarda se il chiamante ha confermato la chiusura; se no --
    il reclamo e' fallito, e' stato interrotto, il lettore era ancora
    bloccato su un nipote -- il flusso e' suo: scarta cio' che legge ancora,
    libera il buffer, e lo chiude all'EOF. Nessun passaggio di
    responsabilita' si puo' interrompere a meta': la conferma
    (`chiuso_dal_chiamante`) si scrive solo dopo che `close()` e' tornata, e
    la conclusione arriva dopo di essa.
    """

    def __init__(self, flusso: Any, guasto: threading.Event,
                 conclusione: threading.Event, gettone: Any) -> None:
        self._flusso = flusso
        #: Un `weakref` al gettone del run, mai un riferimento forte.
        self._gettone = gettone
        self._dati: bytearray | None = bytearray()
        self._fallito = False
        self._guasto = guasto
        self._conclusione = conclusione
        self._stato = threading.Lock()
        self._finito = False
        #: La lettura e' finita (EOF o guasto): il thread puo' restare vivo
        #: ancora, in attesa della fine del run per chiudere il flusso.
        self._letto = threading.Event()
        self.chiuso_dal_chiamante = False
        self._thread = threading.Thread(target=self._leggi, daemon=True)

    def _leggi(self) -> None:
        try:
            while True:
                blocco = self._flusso.read(65536)
                if not blocco:
                    break
                if type(blocco) is not bytes:
                    self._fallito = True
                    break
                if self._run_finito():
                    # Il run e' finito e nessuno leggera' piu' questi byte:
                    # si scartano, e il buffer si libera.
                    self._dati = None
                    del blocco
                    continue
                # Il buffer si legge dall'attributo ogni volta, sotto lock, e
                # nessuna variabile locale lo trattiene: abbandonato, si libera
                # anche mentre la lettura successiva e' bloccata.
                with self._stato:
                    if self._dati is not None:
                        self._dati.extend(blocco)
                del blocco
        except BaseException:  # noqa: BLE001 - un lettore non stampa e non solleva
            self._fallito = True
        finally:
            try:
                if self._fallito:
                    self._guasto.set()
                with self._stato:
                    self._finito = True
                self._letto.set()
            except Exception:  # noqa: BLE001, S110 - libera comunque il buffer
                pass
            # Il flusso: lo chiude il chiamante, se lo ha reclamato e lo
            # conferma; altrimenti, a fine run, lo chiude il lettore. La fine
            # del run e' **certa**: la conclusione impostata, oppure il
            # gettone del run morto (il frame di `run` si e' srotolato, anche
            # per un'interruzione che ha saltato `conclusione.set()`). Prima
            # di allora il lettore non scarta niente: il run puo' ancora
            # prendere i suoi byte (ventiseiesimo giro -- un'attesa fissa
            # dall'EOF scartava lo stderr di un run ancora vivo).
            try:
                while not self._run_finito():
                    self._conclusione.wait(_PASSO_DEL_LETTORE)
            except Exception:  # noqa: BLE001, S110 - libera comunque il buffer
                pass
            self._dati = None
            if not self.chiuso_dal_chiamante:
                try:
                    self._flusso.close()
                except BaseException:  # noqa: BLE001, S110 - nessuno aspetta piu' il flusso
                    pass

    def _run_finito(self) -> bool:
        return self._conclusione.is_set() or self._gettone() is None

    def avvia(self) -> None:
        self._thread.start()

    def aspetta(self, secondi: float) -> None:
        if self._thread.ident is not None:
            self._letto.wait(timeout=max(secondi, 0.0))

    def concluso(self) -> bool:
        return self._letto.is_set() and not self._fallito

    def reclama(self) -> bool:
        """`True` solo se il lettore ha certamente finito (o non e' mai
        partito): allora il chiamante puo' chiudere il flusso, e conferma con
        `chiuso_dal_chiamante` dopo `close()`. Qualunque altro esito -- `False`,
        un'eccezione, un'interruzione mentre si prende il lock -- lascia il
        flusso al lettore."""
        with self._stato:
            return self._finito or self._thread.ident is None

    def prendi(self) -> bytearray:
        """I byte letti; il lettore non ne tiene piu' il riferimento."""
        with self._stato:
            dati, self._dati = self._dati, None
        return dati if dati is not None else bytearray()


def _aspetta_i_lettori(lettori: list[_Lettore], secondi: float) -> None:
    """Un'attesa sola, condivisa, per tutti i lettori."""
    scadenza = time.monotonic() + secondi
    for lettore in lettori:
        lettore.aspetta(scadenza - time.monotonic())


@dataclass
class _EsitoDellaChiusura:
    riuscita: bool = True
    vivo: bool = False
    interruzione: BaseException | None = None


def _chiudi(processo: Any, lettori: list[_Lettore]) -> _EsitoDellaChiusura:
    """La chiusura del processo, su ogni uscita: dal solo thread chiamante,
    sempre dentro attese limitate. L'esito si costruisce solo dopo la
    pulizia: un guasto della costruzione viene gestito da `_esito`.

    Se il processo e' ancora vivo: `kill`, `terminate`, `kill` -- uno per
    volta, ciascuno seguito da un'attesa limitata -- finche' non muore. Un
    tentativo che solleva non ferma i seguenti. Se resta vivo non si aspetta
    oltre: l'esito lo dice (`vivo`), e chi chiama esce con
    `processo.non_terminato`. Poi un'attesa limitata e condivisa dei lettori:
    il flusso di un lettore che ha **certamente** finito si chiude qui, ogni
    altro si abbandona al lettore, e la chiusura e' incompleta. Un'eccezione
    che non e' `Exception` (un secondo Ctrl-C) si conserva in `interruzione`.
    """
    riuscita = True
    vivo = False
    interruzione = None
    try:
        vivo = processo.poll() is None
    except BaseException as errore:  # noqa: BLE001
        riuscita = False
        vivo = True
        if not isinstance(errore, Exception):
            interruzione = errore
    tentativo = 0
    while vivo and tentativo < 3:
        try:
            if tentativo == 1:
                processo.terminate()
            else:
                processo.kill()
        except BaseException as errore:  # noqa: BLE001
            riuscita = False
            if interruzione is None and not isinstance(errore, Exception):
                interruzione = errore
        try:
            processo.wait(timeout=_ATTESA_DELLA_CHIUSURA)
            vivo = False
        except subprocess.TimeoutExpired:
            pass
        except BaseException as errore:  # noqa: BLE001
            riuscita = False
            if interruzione is None and not isinstance(errore, Exception):
                interruzione = errore
        tentativo += 1
    try:
        vivo = processo.poll() is None
        if vivo:
            riuscita = False
    except BaseException as errore:  # noqa: BLE001
        riuscita = False
        vivo = True
        if interruzione is None and not isinstance(errore, Exception):
            interruzione = errore
    try:
        _aspetta_i_lettori(lettori, _ATTESA_DEI_LETTORI)
    except BaseException as errore:  # noqa: BLE001
        riuscita = False
        if interruzione is None and not isinstance(errore, Exception):
            interruzione = errore
    # Niente liste, closure, lambda o metodi legati costruiti prima del try.
    # Ogni pipe ha una protezione propria: il guasto della prima non salta
    # la seconda. I contatori restano fra i piccoli interi di Python.
    posizione = 0
    while posizione < 2:
        lettore = None
        try:
            flusso = getattr(processo, "stdout" if posizione == 0 else "stderr", None)
            if flusso is None:
                continue
            lettore = lettori[posizione] if posizione < len(lettori) else None
            if lettore is not None and lettore.reclama() is not True:
                lettore._dati = None
                riuscita = False
                continue
            flusso.close()
            if lettore is not None:
                lettore.chiuso_dal_chiamante = True
        except BaseException as errore:  # noqa: BLE001
            riuscita = False
            if lettore is not None:
                lettore._dati = None
            if interruzione is None and not isinstance(errore, Exception):
                interruzione = errore
        finally:
            posizione += 1
    try:
        return _EsitoDellaChiusura(riuscita, vivo, interruzione)
    except BaseException:
        # Anche un'interruzione raccolta durante la pulizia resta primaria
        # rispetto a un guasto dell'allocazione dell'esito.
        if interruzione is not None:
            raise interruzione
        raise


def _ultimo_tentativo(processo: Any) -> None:
    """Se il figlio e' ancora vivo: `kill` e un'attesa breve, senza sollevare
    un `Exception`. Un'interruzione asincrona qui dentro puo' lasciarlo vivo
    (README)."""
    try:
        if processo.poll() is None:
            processo.kill()
            processo.wait(timeout=_ATTESA_DELL_ULTIMO_TENTATIVO)
    except Exception:  # noqa: BLE001, S110 - e' l'ultimo tentativo, dopo la chiusura
        pass


def _annota(eccezione: BaseException, nota: str) -> None:
    """Una nota fissa, senza dati, su un'eccezione che resta la primaria.

    `add_note` da Python 3.11; prima, `__notes__` scritto a mano, che e' lo
    stesso attributo. Una nota che non si scrive non cambia l'eccezione.
    """
    try:
        aggiungi = getattr(BaseException, "add_note", None)
        if aggiungi is not None:
            aggiungi(eccezione, nota)
        else:
            precedenti = getattr(eccezione, "__notes__", None)
            note = list(precedenti) if type(precedenti) is list else []
            eccezione.__notes__ = [*note, nota]
    except Exception:  # noqa: BLE001, S110 - la nota non sostituisce l'eccezione
        pass


def _annota_interruzione(interruzione: BaseException) -> None:
    """La nota su un'interruzione dopo l'avvio di un comando che scrive."""
    _annota(interruzione, NOTA_DELL_INTERRUZIONE)


def _ricrea_runner(binario: Any, timeout: Any) -> "Runner":
    """Un `Runner` ricostruito dai soli parametri del costruttore."""
    return Runner(Path(binario), timeout=timeout)


def _argv_esatto(argv: Any) -> list[str]:
    """Una lista nuova di `str` esatte.

    `Runner.run` e' pubblico, e la riga di comando si normalizza al confine
    come gli argomenti dei metodi del client.
    """
    if type(argv) is not list and type(argv) is not tuple:
        raise _dallo_sdk(InvalidArgumentError, "processo.argv")
    return [testo(elemento, "argv") for elemento in argv]


class Runner:
    """Esegue il binario e restituisce la busta, o solleva.

    Non sa niente dei comandi: prende argomenti e rende un documento. I metodi
    che sanno quale busta aspettarsi stanno nel client, ed e' li' che il tipo
    si costruisce.
    """

    def __init__(self, binary: Path, *, timeout: float | None = None) -> None:
        self._binary = Path(percorso(binary, "binary"))
        self._timeout = timeout_valido(timeout)

    @property
    def binary(self) -> Path:
        return self._binary

    # Il protocollo di serializzazione: lo stato sono i soli parametri del
    # costruttore, e si ricostruisce chiamandolo (ventesimo giro).

    def __getstate__(self) -> dict[str, Any]:
        return {"binary": str(self._binary), "timeout": self._timeout}

    def __reduce__(self) -> Any:
        return (_ricrea_runner, (str(self._binary), self._timeout))

    def __reduce_ex__(self, protocollo: Any) -> Any:
        _protocollo(protocollo)
        return self.__reduce__()

    def __copy__(self) -> "Runner":
        return _ricrea_runner(str(self._binary), self._timeout)

    def __deepcopy__(self, memo: Any = None) -> "Runner":
        _memo(memo)
        return _ricrea_runner(str(self._binary), self._timeout)

    def run(self, argv: list[str]) -> FrozenDict:
        """Il risultato della busta di successo, o `CommandFailed` con quella
        d'errore.

        Un `FrozenDict`, congelato in profondita' (`FrozenList` per gli
        elenchi), come ogni altra risposta dell'SDK: prima era il `dict`
        decodificato, con le sue `list`, un valore restituito fuori dal
        confine (diciottesimo giro). I modelli lo leggono con `from_json`.
        """
        argv = _argv_esatto(argv)
        scrittura = scrive(argv)
        # Prima di `Popen`, **tutti** gli oggetti del run: una costruzione
        # che fallisce qui non lascia un processo senza pulizia (ventisettesimo
        # giro: il gettone e il suo `weakref` stavano fra `Popen` e il `try`).
        # Fra `_avvia` e il `try` non c'e' nessuna istruzione, e una prova
        # d'inventario sull'AST lo tiene fermo.
        lettori: list[_Lettore] = []
        conclusione = _nuovo_evento()
        guasto = _nuovo_evento()
        gettone = _GettoneDelRun()
        riferimento = weakref.ref(gettone)
        primaria = None
        processo = self._avvia(argv)
        # Il confine: `Popen` ha reso il processo. Da qui **tutto** passa da
        # un solo gestore, e la chiusura del processo gira su ogni uscita.
        #
        # Ventunesimo giro: niente `with Popen` e niente `communicate`. Il loro
        # `__exit__` girava prima del gestore -- un `close()` che falliva
        # sostituiva un `KeyboardInterrupt`, e l'uscita aspettava senza
        # limite prima di uccidere il figlio -- e su Windows i thread lettori
        # di `communicate` stampavano le proprie eccezioni su stderr, mentre
        # `run` rendeva comunque un successo. Qui i lettori sono dell'SDK
        # (`_Lettore`), l'attesa ha un tempo esplicito, e la chiusura
        # (`_chiudi`) protegge ogni passo; `_esito` protegge anche la sua
        # chiamata: i guasti non sostituiscono l'eccezione primaria.
        # Da qui al `try`, niente. La fine del run per i lettori si
        # imposta nel `finally`, dopo la chiusura; `Event.set` non aspetta.
        try:
            return self._esito(processo, argv, lettori, conclusione, guasto, riferimento, scrittura)
        except BaseException as errore:  # noqa: BLE001 - anche nel finally resta primaria
            primaria = errore
            raise
        finally:
            try:
                # L'ultimo tentativo, dal thread chiamante, comunque si sia
                # usciti da `_chiudi`: un'interruzione al suo ingresso saltava
                # la terminazione del figlio.
                _ultimo_tentativo(processo)
            except BaseException as errore:  # noqa: BLE001
                if primaria is None:
                    primaria = errore
                    if isinstance(errore, Exception):
                        raise _dopo_l_avvio(errore, argv) from None
                    raise
                _annota(primaria, NOTA_DELLA_CHIUSURA)
            finally:
                gettone = None  # noqa: F841 - il frame non trattiene piu' il gettone
                try:
                    conclusione.set()
                except BaseException as errore:  # noqa: BLE001
                    if primaria is None:
                        if isinstance(errore, Exception):
                            raise _dopo_l_avvio(errore, argv) from None
                        raise
                    _annota(primaria, NOTA_DELLA_CHIUSURA)

    def _esito(
        self, processo: Any, argv: list[str], lettori: list["_Lettore"],
        conclusione: threading.Event, guasto: threading.Event, gettone: Any,
        scrittura: bool,
    ) -> FrozenDict:
        """Il resto di `run`, dentro il `finally` che conclude i lettori."""
        primaria: BaseException | None = None
        risultato: Any = None
        try:
            registra_avvio(argv)
            lettori += [
                _Lettore(processo.stdout, guasto, conclusione, gettone),
                _Lettore(processo.stderr, guasto, conclusione, gettone),
            ]
            stdout, stderr = self._attendi(processo, lettori, guasto, argv)
            testo_stdout = _testo(stdout, argv, "stdout")
            del stdout  # i byte non servono piu': non restano vivi col testo
            testo_stderr = _testo(stderr, argv, "stderr")
            del stderr
            completed = Completed(
                argv=list(argv),
                exit_code=processo.returncode,
                stdout=testo_stdout,
                stderr=testo_stderr,
            )
            del testo_stdout, testo_stderr
            if completed.exit_code == 0:
                risultato = FrozenDict(self._success(completed))
            else:
                documento = self._failure(completed)
                risultato = _failure_from_envelope(documento, completed.exit_code, argv)
        except BaseException as errore:  # noqa: BLE001 - il gestore unico, apposta
            primaria = errore
        esito = None
        try:
            esito = _chiudi(processo, lettori)
        except BaseException as errore:  # noqa: BLE001 - conserva la primaria
            if primaria is None:
                primaria = errore
        if primaria is not None:
            if isinstance(primaria, Exception):
                if esito is not None and esito.vivo:
                    # Un processo rimasto vivo prevale sul guasto che ha
                    # portato a chiuderlo: chi chiama deve saperlo per primo.
                    raise _dopo_l_avvio(_dallo_sdk(
                        ProtocolError, "processo.non_terminato",
                        comando=_nome_del_comando(argv),
                    ), argv) from None
                raise _dopo_l_avvio(primaria, argv) from None
            # Un'interruzione resta tale, dopo la chiusura del figlio.
            if scrittura:
                _annota(primaria, NOTA_DELL_INTERRUZIONE)
            if esito is None or not esito.riuscita:
                _annota(primaria, NOTA_DELLA_CHIUSURA)
            raise primaria
        if esito.interruzione is not None:
            # Un'interruzione arrivata durante la chiusura, dopo un esito:
            # e' l'eccezione primaria, e resta tale. La chiusura che ha
            # interrotto non e' riuscita del tutto, e lo dice la nota.
            if scrittura:
                _annota(esito.interruzione, NOTA_DELL_INTERRUZIONE)
            _annota(esito.interruzione, NOTA_DELLA_CHIUSURA)
            raise esito.interruzione
        if isinstance(risultato, BaseException):
            # `CommandFailed`: e' l'esito del comando, e resta tale.
            raise risultato
        if not esito.riuscita:
            # Una chiusura che non riesce dopo un successo non e' un successo.
            if esito.vivo:
                guasto = _dallo_sdk(
                    ProtocolError, "processo.non_terminato", comando=_nome_del_comando(argv))
            else:
                guasto = _dallo_sdk(
                    ProtocolError, "processo.chiusura", comando=_nome_del_comando(argv))
            raise _dopo_l_avvio(guasto, argv) from None
        return risultato

    # --- l'esecuzione -----------------------------------------------------

    @property
    def sigint_forwarding_available(self) -> bool:
        """Un Ctrl-C durante un comando arriva al prodotto.

        Falso da un thread che non sia il principale e su Windows, e in
        entrambi i casi il comando funziona lo stesso: cambia solo che non lo si
        puo' fermare con grazia.
        """
        return sigint_inoltrabile()

    def _avvia(self, argv: list[str]) -> Any:
        # `Popen` e non `run`: senza il pid non c'e' niente a cui inoltrare il
        # segnale, e l'inoltro e' il modo in cui un Ctrl-C diventa una
        # cancellazione cooperativa invece di un file mezzo scritto.
        #
        # I flussi si leggono in **byte**: la decodifica e' `_testo`, che
        # trasforma un byte fuori codifica in un `ProtocolError`.
        try:
            processo = subprocess.Popen(  # noqa: S603 - argv e' costruito qui
                [str(self._binary), *argv],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
        except (ValueError, TypeError):
            # Un argomento che il sistema non sa passare -- un NUL in un
            # percorso o in un'opzione (`ValueError`), un elemento che non e'
            # testo (`TypeError`) -- e `Popen` rifiuta prima di partire. Il
            # client li rifiuta gia' prima, argomento per argomento; questo e'
            # il confine per chi usa `Runner` direttamente. Niente e' partito,
            # quindi l'effetto e' `none`, e il testo dell'eccezione -- che
            # puo' citare l'argomento -- resta fuori.
            raise _dallo_sdk(
                InvalidArgumentError, "processo.argomento_non_passabile",
                comando=_nome_del_comando(argv),
            ) from None
        except OSError:
            # Il processo non e' partito: niente puo' essere stato scritto, e
            # l'effetto resta `none`. Il testo dell'errore del sistema non
            # entra nel messaggio.
            errore = _dallo_sdk(
                ProtocolError, "processo.non_eseguibile",
                comando=_nome_del_comando(argv),
            )
            raise _ricostruito(errore, _non_partito=True) from None

        return processo

    def _attendi(
        self, processo: Any, lettori: list["_Lettore"], guasto: threading.Event,
        argv: list[str],
    ) -> tuple[bytearray, bytearray]:
        """I due flussi, letti dai lettori dell'SDK mentre il processo gira.

        Il thread chiamante e' il solo che aspetta il processo, a passi
        brevi (`_PASSO`): fra un passo e l'altro guarda l'`Event` dei lettori
        e la scadenza del timeout del client (`None`: nessuna scadenza, ma il
        ciclo continua). Un lettore guasto o la scadenza fanno uscire con un
        errore, e la chiusura -- dallo stesso thread -- uccide il processo:
        nessun `wait` concorrente. I lettori si aspettano poi con un tempo
        limitato e condiviso; uno che fallisce o che non finisce (un nipote
        che tiene la pipe) e' `processo.comunicazione`, mai un successo.
        """
        for lettore in lettori:
            lettore.avvia()
        scadenza = None if self._timeout is None else _adesso() + self._timeout
        with _inoltra_sigint(processo):
            while True:
                if scadenza is None:
                    durata = _PASSO
                else:
                    durata = max(0.0, min(_PASSO, scadenza - _adesso()))
                try:
                    processo.wait(timeout=durata)
                    break
                except subprocess.TimeoutExpired:
                    pass
                except OSError:
                    raise _dallo_sdk(
                        ProtocolError, "processo.comunicazione",
                        comando=_nome_del_comando(argv),
                    ) from None
                if guasto.is_set():
                    raise _dallo_sdk(
                        ProtocolError, "processo.comunicazione",
                        comando=_nome_del_comando(argv),
                    )
                if scadenza is not None and _adesso() >= scadenza:
                    raise _dallo_sdk(
                        ProtocolError, "processo.timeout", comando=_nome_del_comando(argv),
                    )
        _aspetta_i_lettori(lettori, _ATTESA_DEI_LETTORI)
        if any(not lettore.concluso() for lettore in lettori):
            raise _dallo_sdk(
                ProtocolError, "processo.comunicazione", comando=_nome_del_comando(argv),
            )
        return lettori[0].prendi(), lettori[1].prendi()

    # --- i due flussi, ciascuno al proprio posto ---------------------------

    def _success(self, completed: Completed) -> dict[str, Any]:
        # `!= ""`, non `.strip()`: anche uno spazio e' contenuto. Uno `strip()`
        # lascerebbe passare una scrittura accidentale vuota o di soli spazi --
        # un `eprintln!("")` di troppo, un a capo rimasto -- che e' proprio la
        # forma in cui una violazione arriva senza che nessuno l'abbia voluta.
        # Il confine dev'essere quello che si puo' verificare guardando i byte.
        if completed.stderr != "":
            # Il v2 tace su stderr quando riesce. Non e' pignoleria: e' la sola
            # affermazione che rende quel flusso utilizzabile da chi compone la
            # CLI in una pipeline, e tollerarne la violazione la toglierebbe.
            raise _dallo_sdk(
                ProtocolError, "processo.stderr_successo",
                comando=_nome_del_comando(completed.argv),
                caratteri=len(completed.stderr),
            )
        documento = self._decode(completed, "stdout")
        stato = documento.get("status")
        if stato != "ok":
            # Un'uscita a zero con una busta che non si dichiara riuscita: il
            # protocollo non lo prevede, e leggerla come successo darebbe per
            # buono un documento che il prodotto non ha dichiarato tale.
            raise _dallo_sdk(
                ProtocolError, "processo.stato_successo",
                comando=_nome_del_comando(completed.argv),
                stato=_stato(stato),
            )
        # Al chiamante interessa il **risultato**, non la busta che lo avvolge.
        #
        # Fino alla 3.0.0 i campi dell'operazione uscivano al primo livello,
        # accanto a `status` e `contract`, e questo esecutore rendeva il
        # documento intero. Ora stanno in `result`, e i modelli che li leggono
        # non devono sapere dove la busta finisce e il risultato comincia.
        #
        # `--help` non passa di qui: non e' una busta, e il client non lo
        # chiama.
        risultato = documento.get("result")
        if _tipo(risultato) != "object":
            raise _dallo_sdk(
                ProtocolError, "processo.senza_result",
                comando=_nome_del_comando(completed.argv),
            )
        return risultato

    def _failure(self, completed: Completed) -> dict[str, Any]:
        # La busta d'errore esce su **stdout**, come quella di successo.
        #
        # Usciva su stderr, e l'SDK la leggeva di li'. CLI 2.0 vuole invece un
        # documento su stdout e **niente** su stderr, qualunque sia l'esito: un
        # consumatore che legge un flusso solo li trova entrambi, e lo `status`
        # della busta gli dice quale dei due ha in mano.
        documento = self._decode(completed, "stdout")
        if documento.get("status") != "error":
            raise _dallo_sdk(
                ProtocolError, "processo.stato_errore",
                comando=_nome_del_comando(completed.argv),
                uscita=completed.exit_code,
                stato=_stato(documento.get('status')),
            )
        if completed.stderr != "":
            # Vale anche sul percorso d'errore, e non solo su quello riuscito:
            # una diagnostica scritta accanto alla busta e' esattamente cio'
            # che rende il flusso inutilizzabile per chi lo analizza.
            raise _dallo_sdk(
                ProtocolError, "processo.stderr_errore",
                comando=_nome_del_comando(completed.argv),
                caratteri=len(completed.stderr),
            )
        return documento

    @staticmethod
    def _decode(completed: Completed, stream: str) -> dict[str, Any]:
        testo = completed.stream(stream)
        if not testo.strip():
            altro = "stderr" if stream == "stdout" else "stdout"
            raise _dallo_sdk(
                ProtocolError, "processo.flusso_vuoto",
                comando=_nome_del_comando(completed.argv),
                uscita=completed.exit_code,
                flusso=stream,
                altro=altro,
                caratteri=len(completed.stream(altro)),
            )
        try:
            documento = carica_json(testo)
        except ProtocolError as errore:
            # `carica_json` traduce ogni guasto del parser -- sintassi,
            # annidamento, interi oltre il limite di cifre -- e qui gli si
            # aggiunge da dove viene. `from None`: la catena non serve a chi
            # legge, e il messaggio di `carica_json` non porta il testo.
            raise _dallo_sdk(
                ProtocolError, "processo.busta_illeggibile",
                comando=_nome_del_comando(completed.argv),
                flusso=stream,
                errore=errore,
                caratteri=len(testo),
            ) from None
        if _tipo(documento) != "object":
            raise _dallo_sdk(
                ProtocolError, "processo.busta_non_oggetto",
                flusso=stream,
                tipo=_tipo(documento),
            )
        return documento


def _ricrea_nativo(timeout: Any) -> "NativeRunner":
    from ._discovery import modulo_nativo

    return NativeRunner(modulo_nativo(), timeout=timeout)


class NativeRunner(Runner):
    """Lo stesso protocollo di Runner, dal dispatch Rust nel processo.

    Il timeout annulla cooperativamente e aspetta il termine del comando.
    Le interruzioni Python risalgono solo dopo che il worker si e' fermato.
    """

    def __init__(self, nativo: Any, *, timeout: float | None = None) -> None:
        self._timeout = timeout_valido(timeout)
        self._nativo = nativo

    @property
    def binary(self) -> None:
        return None

    @property
    def sigint_forwarding_available(self) -> bool:
        return threading.current_thread() is threading.main_thread()

    def __getstate__(self) -> dict[str, Any]:
        return {"timeout": self._timeout}

    def __reduce__(self) -> Any:
        return (_ricrea_nativo, (self._timeout,))

    def __copy__(self) -> "NativeRunner":
        return NativeRunner(self._nativo, timeout=self._timeout)

    def __deepcopy__(self, memo: Any = None) -> "NativeRunner":
        _memo(memo)
        return self.__copy__()

    def run(self, argv: list[str]) -> FrozenDict:
        argv = _argv_esatto(argv)
        try:
            registra_avvio(argv)
            with _sigint_nativo() as interrotto:
                codice, stdout, scaduto = self._nativo.esegui(list(argv), self._timeout, interrotto)
            if scaduto:
                raise _dallo_sdk(
                    ProtocolError, "nativo.timeout", comando=_nome_del_comando(argv)
                )
            completed = Completed(argv=list(argv), exit_code=codice, stdout=stdout, stderr="")
            if codice == 0:
                return FrozenDict(self._success(completed))
            errore = _failure_from_envelope(self._failure(completed), codice, argv)
        except Exception as errore:
            raise _dopo_l_avvio(errore, argv) from None
        except BaseException as errore:
            if scrive(argv):
                _annota_interruzione(errore)
            raise
        raise errore
