# plenora-io — SDK Python

Un client Python del **protocollo v2** della CLI `plenora-io`: costruisce gli
argomenti di un comando, ne decodifica le buste JSON, e solleva gli errori per
categoria. Il protocollo e' il confine pubblico -- `release/cli-protocol-v2.json`
lo ratifica campo per campo -- e lo SDK non ne usa altri.

## Due wheel, un pacchetto

* **La wheel nativa** (`cp310-abi3`, Linux manylinux 2_34 e Windows x86_64) porta
  il modulo `plenora_io._native`, costruito con PyO3 da `crates/plenora-io-py`.
  Il comando gira **nel processo**: il modulo esegue lo stesso dispatch del
  binario -- le stesse funzioni Rust, non una copia -- e rende lo stesso codice
  d'uscita e la stessa busta. Non serve un binario installato. Una sola wheel per
  piattaforma copre Python 3.10-3.14, e la CI la prova su tutte e cinque le
  versioni e sui due sistemi.
* **La wheel pura** (`py3-none-any`) e' lo stesso pacchetto senza il modulo, ed
  esegue un binario `plenora-io` che trova sulla macchina. Resta la strada per le
  piattaforme senza wheel nativa e per chi vuole un binario preciso.

`Client()` sceglie da se': un `binary=` esplicito o `PLENORA_IO_BIN` vincono, poi
il modulo nativo se c'e', poi il binario accanto al pacchetto o nel `PATH`.
`Client.backend` dice quale strada ha preso (`"native"` o `"process"`), e
`Client.binary` e' `None` quando il comando gira nel processo.

La versione dello SDK e' quella del workspace, cioe' del CLI:
`scripts/check_sdk_python.py` verifica che coincidano.

## Che cosa c'e' oggi

* la scoperta del binario, **fail-closed**, e il modulo nativo quando c'e';
* la lettura del `MANIFEST.json` dell'artefatto distribuito, quando c'e';
* il controllo del profilo, prima di eseguire invece che dopo;
* `version()` del pacchetto e `Client.capabilities()` del binario;
* i comandi -- `--version`, `catalog`, `inspect`, `layers`, `validate`,
  `read`, `write`, `convert` -- con i modelli tipizzati e i tetti in `Limits`;
* con l'extra `pyarrow`, `read_table()` e `write()` da un oggetto Arrow.

Il pacchetto non e' pubblicato su un indice: wheel e sdist si consegnano con
gli artefatti della release.

## `capabilities()` e `version()`

`Client.capabilities()` rende il documento `plenora-capabilities-v2` del
**binario** in uso, tipizzato: `Capabilities.operation("io.write")` porta
versione, stato, superfici, contratti d'ingresso e d'uscita, effetto e
controlli come attributi, senza testo da analizzare. Lo si chiede al binario e
non lo si scrive nell'SDK, perche' cio' che conta e' che cosa l'artefatto
installato espone.

`plenora_io.version()` e' un'altra cosa: la versione del **pacchetto**, letta
dai metadati installati (`importlib.metadata.version("plenora-io")`). Quella
del binario la dice `Client.version()`.

Ogni metodo del dominio corrisponde a un'operazione del catalogo
(`plenora_io._client.OPERAZIONI`): `catalog`, `inspect`, `layers`, `convert`
alle omonime; `validate`, `read` e `read_table` a `io.read`; `write` a
`io.write`. Una sonda confronta la mappa con i metodi pubblici e con il
documento del binario vero.

## L'adattatore Arrow, facoltativo

```
pip install "plenora_io-<versione>-py3-none-any.whl[pyarrow]"
```

L'extra installa `pyarrow>=25,<26`, la serie di plenora-data-tools: due
librerie Plenora nello stesso ambiente stanno sulla stessa pyarrow. Con
l'extra:

* `Client.read_table(source, ...)` legge in un file Arrow IPC temporaneo e
  rende `TableRead(table, result)`: la `pa.Table` **e** la busta di `read`,
  perche' la fedelta' della lettura non si perda per strada;
* `Client.write(source, ...)` accetta, al posto del percorso, un `pa.Table`, un
  `pa.RecordBatch`, un `pa.RecordBatchReader` o qualunque oggetto con
  `__arrow_c_stream__`, scritto a batch in un file IPC temporaneo.

Il file temporaneo sta in `temp_dir`, se indicata, e si cancella al ritorno,
riuscito o no. `read_table()` tiene la tabella intera in memoria, che e' cio'
che `pa.Table` e': per un dataset piu' grande della memoria la strada resta
`read()` su un percorso, letto poi a batch. Senza l'extra, o con un'altra serie
di pyarrow installata a mano, le due chiamate si rifiutano con
`OptionalDependencyError` (`unsupported`) prima di eseguire.

## Conformita' a `plenora-python-sdk-v1`

Il contratto e' PYTHON-SDK-1.0 di plenora-contracts (identico fra la revisione
fissata in `contracts/adoption-source.json` e la v1.1.0, `3c395a8`). Il profilo
io-tools non richiede la superficie Python e il catalogo comune non la dichiara
per IO-tools: il pacchetto e' scritto contro il contratto, e lo **reclamera'**
quando il catalogo lo fara'. Fino ad allora `contracts/adoption-source.json` lo
tiene fuori dal profilo.

| § | requisito | stato |
|---|---|---|
| 2 | nome `plenora-io` / `plenora_io` | conforme |
| 2 | Python 3.10 o piu' recente | conforme: `>=3.10,<3.15`, tutte provate dalla CI |
| 2 | `version()` = metadati = nome della wheel | conforme: lo smoke installato lo verifica |
| 3 | PEP 561, nomi pubblici intenzionali, risultati strutturati | conforme: `py.typed`, `__all__`, dataclass |
| 3 | Arrow al confine tabellare (SHOULD) | conforme: file Arrow IPC, e oggetti PyArrow con l'extra |
| 4 | parita' sync/async | solo sync, nessun metodo asincrono nominale; `api_modes: ["sync"]` quando l'artefatto entrera' nel manifesto |
| 5 | ciclo di vita | non applicabile: `Client` non tiene risorse aperte, ogni chiamata attende fino alla fine il processo o il worker nativo |
| 6 | radice `PlenoraError`, cinque assi su ogni eccezione | conforme dalla 4.2.0: prima li portava solo `CommandFailed` |
| 6 | redazione dei segreti | non applicabile: nessuna credenziale; i messaggi non riportano contenuti dei documenti |
| 7 | scoperta strutturata, fail-closed | conforme: `capabilities()` rende il documento del binario con l'interfaccia e la superficie `python_sdk` (CAP-003, CAP-006) |
| 8 | cancellazione, scadenza, budget (SHOULD) | scadenza e budget in `Limits`; cancellazione limitata, sotto |
| 9 | sicurezza di rete | non applicabile: nessuna rete |
| 10 | verifica dell'artefatto installato | conforme: `scripts/smoke-pacchetto-python.sh` e `smoke-sdk-installato.py` |
| 12 | identita' delle operazioni | conforme: `OPERAZIONI` e la sua sonda |

### Limiti dichiarati

**Gli esaurimenti e i guasti dell'interprete sono fuori garanzia.** Regola:
`MemoryError`, `RecursionError` interno, thread non avviabili e interruzioni
asincrone nei meccanismi interni dell'SDK sono esclusi dalla garanzia: quasi
ogni operazione Python alloca, quindi una protezione completa non e'
dimostrabile. Ambito: la gestione del processo, compresi lettori, notifiche
e pulizia. Hazard: con memoria esaurita o un guasto dell'interprete lo SDK
puo' restituire `UnexpectedError` al posto dell'errore atteso, lasciare
risorse non rilasciate o lasciare il figlio vivo. L'eccezione primaria
dell'utente va preservata per quanto ottenibile senza nuovo codice
speculativo; resta il residuo descritto. Rientro: ripristinare le risorse
dell'interprete e verificare e chiudere gli eventuali processi figli e le
risorse rimaste; la garanzia riprende nelle condizioni ordinarie.

**La cancellazione non e' un parametro.** Regola: PYTHON-SDK-1.0 §8 (SHOULD).
Ambito: tutti i metodi. Un Ctrl-C e' inoltrato al prodotto soltanto dal thread
principale e fuori da Windows (`Client.cancellable`); non c'e' un token che il
chiamante possa armare da un altro thread. Hazard: un lavoro lungo avviato da
un thread secondario o su Windows si ferma solo con `Client(timeout=...)`, che
uccide il processo e rende `ProtocolError` con `remote_effect: unknown` se il
comando scriveva. Rientro: un token di cancellazione dell'SDK inoltrato al
processo su entrambe le piattaforme.

Con `backend == "native"`, Ctrl-C e' raccolto dal thread principale anche
su Windows: annulla cooperativamente e attende il worker Rust. Anche il
timeout annulla e attende; non e' un limite rigido al tempo di ritorno. Se
la scadenza e' trascorsa al ritorno, anche un successo diventa timeout. Un timeout o un guasto
dopo l'avvio di una scrittura porta effetto `unknown`, come nel backend di
processo. Le esclusioni per memoria esaurita, guasti dell'interprete e
interruzioni asincrone nei meccanismi interni valgono per entrambi i backend.

**Un errore di protocollo dopo una scrittura dice `unknown`.** Non e' una
deviazione ma una scelta da conoscere: se il processo e' partito e il comando
scrive (`write`, `convert`, `read` con destinazione), un `ProtocolError` porta
`remote_effect: unknown` e `retry: never`, perche' che cosa sia rimasto sul
disco non lo dice una risposta che non si legge. Per i comandi che leggono
soltanto resta `none`. Vale per ogni guasto dopo l'avvio -- un flusso che non
e' UTF-8, un errore delle pipe, un timeout -- mentre un processo che non parte
resta `none`. I messaggi nominano il solo sottocomando (`plenora-io write`),
mai percorsi e opzioni, e non portano la catena delle eccezioni di pyarrow o
del sistema (`from None`): il loro testo puo' citare valori della sorgente.

**I temporanei dell'adattatore Arrow.** Una `temp_dir` che non c'e' o non si
scrive e' `LocalIoError` (`io`, `prepare`). Se la directory temporanea non si
cancella dopo che `write` ha pubblicato, e' `CleanupError` (ERRORS-1.0, ERR-015:
`cleanup`, `committed`, `never`): l'operazione e' avvenuta e la directory va
tolta a mano. `read_table()` che non trova la consegna dichiarata, o non la
legge come la busta dice, e' `ProtocolError` con `remote_effect: unknown`:
`read` ha gia' scritto il file, anche se la pulizia che segue fallisce.

**Gli argomenti si verificano prima di eseguire.** Ogni argomento dei metodi
pubblici e' controllato per tipo esatto prima di diventare una riga di comando:
un percorso e' `str` o `os.PathLike[str]`, un intero e' un `int` che non e' un
`bool`, `durable` e' un `bool`, le opzioni sono un dizionario di stringhe la cui
chiave non contiene `=` (la CLI divide al primo `=`, e la coppia arriverebbe con
un'altra chiave), e nessun argomento porta un carattere NUL. Il rifiuto e'
`InvalidArgumentError` con effetto `none`; il messaggio nomina l'argomento, non
il suo valore, e l'eccezione di `__fspath__` o `__str__` non resta nella
catena. Non e' una deviazione: prima le stesse chiamate uscivano come
`TypeError`, `ValueError` di `Popen` o l'eccezione di chi chiama, oppure -- un
`layer=True`, un `durable="no"` -- diventavano un altro argomento in silenzio.
Ogni argomento si converte all'ingresso nel tipo base esatto (`str`, `int`,
`bool`, `dict` di `str`, `Limits` e `timedelta` nuovi), con i metodi non
legati del tipo base: una sottoclasse con `__str__`, `__format__` o
`__contains__` ridefiniti non esegue codice dopo il confine. Un intero oltre
64 bit si rifiuta prima di ogni conversione. E' un cambiamento incompatibile,
dichiarato in `docs/RELEASE.md`, «Note della prossima release».

**La rete.** Ogni callable di ogni tipo esportato -- metodi speciali,
setter e deleter compresi, con le sole eccezioni di `_confine.NON_AVVOLTI` --
e ogni funzione esportata passano da `_confine._confinato`: un'eccezione che nessun punto ha
tradotto diventa `UnexpectedError` (`internal`), con effetto `none` prima
dell'avvio di un processo e `unknown` dopo l'avvio di un comando che scrive,
senza testo e senza catena. I punti noti restano tradotti dove nascono, con
l'effetto preciso; una prova per introspezione fa fallire un metodo pubblico
nuovo senza rete. Un errore dell'SDK non attraversa la rete com'e': la rete ne
costruisce uno **nuovo**, della stessa classe esatta, dallo stato fissato nel
costruttore (`_errors._ricostruito`), e non legge mai `args`, `__notes__` o
`str()` dell'oggetto ricevuto. Una callback che prende un errore dell'SDK, ne
riscrive `args` e lo rilancia non fa uscire il proprio testo; `ResultLookupError`
resta un `KeyError` e un `IndexError`, quindi `Mapping.get` e `in` funzionano.
Si ricostruisce solo un errore **nato nell'SDK**: un errore conta come
dell'SDK solo se l'SDK l'ha costruito con una delle sue due fabbriche private.
Il costruttore pubblico, chiamato in qualunque modo -- da una callback, da
`map` o `functools.partial` in C, da `eval` -- fa sempre un errore
**esterno**, che la rete rende `UnexpectedError`, come un errore di una
sottoclasse del chiamante. Le due fabbriche danno garanzie diverse sul testo:

* la **fabbrica a catalogo** (`_errors._dallo_sdk`) costruisce ogni altro
  errore dell'SDK, con un messaggio del catalogo e parametri tipati (sotto);
* la **fabbrica strutturata** (`_errors._dallo_sdk_strutturato`) costruisce i
  tre tipi che compongono il messaggio da se': `BinaryNotFound` dai posti di
  ricerca, un elenco chiuso; `ProfileError` dai profili noti; `CommandFailed`
  e le sue sottoclassi dalla **busta d'errore della CLI**. Di quella busta
  l'SDK controlla solo la forma -- tipi esatti, campi obbligatori, `message`
  stringa non vuota, vocabolari chiusi di effetto e ritentativo -- e ne
  riproduce il messaggio: che sia testo curato, senza dati, lo garantisce la
  CLI (`PublicMessage` in `crates/plenora-io-model/src/error.rs`, il cui solo
  testo di runtime e' il token limitato di un'opzione rifiutata), non l'SDK.

**Il testo di un errore della fabbrica a catalogo.** La fabbrica non
accetta testo: prende una
chiave di un catalogo chiuso di modelli fissi (`_errors.MESSAGGI`) e dei
parametri, e ogni parametro ha un tipo dichiarato per la sua chiave
(`_errors.PARAMETRI`): un insieme chiuso specifico -- i nomi delle classi
dell'SDK per un nome di classe, i campi degli schemi dei modelli per un campo,
`MOTIVI_JSON` per il motivo di un errore JSON, i contesti del protocollo per un
contesto, i sottocomandi per un comando -- oppure un intero esatto in un
intervallo dichiarato. Un valore fuori dal suo tipo, una stringa numerica, un
intero fuori intervallo diventano `?`, e l'errore nasce comunque. Ogni testo
che arriva a un parser si copia prima in una `str` esatta, e la posizione di
un errore JSON la ricalcola l'SDK da quel testo. Cio' che e' provato, in
`tests/test_confini_esterni.py`:

* ogni chiamata alla fabbrica nei sorgenti usa una chiave letterale del
  catalogo (inventario sull'AST), e nessun sorgente chiama direttamente una
  classe d'errore;
* i parametri dichiarati di ogni chiave sono esattamente i segnaposto del
  suo modello, e una parola dell'SDK nel posto di un'altra, una stringa
  numerica o un intero oltre ogni limite si scrivono `?`;
* una sottoclasse di `str` passata a `carica_json`, o un `Path` derivato
  passato a `leggi_manifesto`, non scelgono ne' il motivo ne' la posizione
  dell'errore JSON;
* ogni chiave, con ogni parametro sostituito da dati del chiamante di ogni
  forma (testo, sottoclassi di `str`, composti, elenchi, oggetti, errori
  esterni), da' un testo che non li contiene;
* la superficie pubblica e' chiusa (`LaSuperficiePubblicaChiusa`): ogni
  nome senza `_` di ogni modulo senza `_` del pacchetto -- cioe' solo la
  radice, perche' i moduli sono privati -- sta in `__all__`; ogni funzione e
  ogni classe di `__all__`, chiamata con combinazioni di argomenti ostili
  dentro un `except` del chiamante, da' soltanto `PlenoraError`, senza
  `__context__` ne' `__cause__`;
* ogni metodo, `__init__`, classmethod, staticmethod e proprieta' delle
  classi di `__all__` -- anche su una sottoclasse con ogni attributo di
  classe ridefinito, e `__init__` anche non legato su un oggetto estraneo --
  chiamati con combinazioni di argomenti ostili (fino a tre, posizionali e
  keyword-only) dentro un `except` del chiamante non danno un errore fidato
  con quei dati, ne' un'eccezione che non sia un `PlenoraError`, ne' un
  errore con `__context__` o `__cause__`. `Client` e `Runner` avviano il
  binario e hanno prove proprie (i loro argomenti sono provati da
  `GliArgomenti` e da `test_confini_sdk`).

Ogni metodo avvolto rifiuta, con un messaggio fisso, un primo argomento che
non ha la propria classe nell'MRO; lo schema e i campi di un modello si
leggono dalla classe dell'SDK riconosciuta per identita', mai da `cls` o da
una sottoclasse. **I moduli dell'SDK sono privati** (`plenora_io._errors`,
`_confine`, `_models`, `_client`, `_process`, `_discovery`, `_arrow`,
`_contenitori`, `_limits`, `_argomenti`): la superficie pubblica e' la radice
del pacchetto, con il suo `__all__`, e ogni nome che vi sta passa dalla
rete. Le funzioni di quei moduli -- la fabbrica, la ricostruzione,
`carica_json`, la scoperta del binario, gli aiuti del processo e
dell'adattatore Arrow -- non sono API. `str(errore)`, `message` e
`to_dict()` vengono dallo stesso stato: per `CommandFailed` `str()` contiene
`message` preceduto da sottocomando, uscita, categoria, fase e codice, che
vengono dalla busta del processo; per gli altri errori coincidono.

Le ricerche nei risultati (`layer`,
`field`, `driver`, `operation`) sollevano `ResultLookupError`, anche
`KeyError`, senza nomi nel messaggio.

**Senza catena vuol dire `__context__` e `__cause__` a `None`.** Python mette
come `__context__` di un'eccezione quella che si sta gestendo nel momento del
`raise`: se il chiamante usa l'SDK dentro un proprio `except ValueError(...)`,
quel `ValueError` diventerebbe il contesto dell'errore dell'SDK, e `from None`
lo nasconderebbe soltanto. La rete solleva il proprio errore, ne toglie il
`__context__` e lo rilancia con un `raise` nudo, che non concatena: cio' che
esce da un metodo pubblico ha `__cause__` e `__context__` a `None` e
`__suppress_context__` vero. I messaggi nominano le classi dell'SDK, mai
`type(oggetto).__name__`: una sottoclasse del chiamante -- `type("nome",
(Version,), {})` -- puo' chiamarsi come vuole, e il messaggio usa il nome della
prima classe dell'SDK nel suo MRO (`_confine.nome_sdk`).

**Il confine delle garanzie.** Le garanzie di questa sezione -- solo
`PlenoraError`, nessuna catena, nessun dato del chiamante in un errore fidato
-- valgono per ogni **chiamata a un callable dell'SDK**: le funzioni di
`__all__`, i costruttori, i metodi, le proprieta' e gli operatori delle classi
dell'SDK, i metodi di `FrozenList` e `FrozenDict` (compresi `get`, `keys`,
`items` e `values`, che sono dell'SDK; `keys`, `items` e `values` rendono
`FrozenList`). Fuori dal confine, e dichiarati:

* gli **scalari nativi immutabili** che l'SDK rende o porta in un attributo
  (`str`, `int`, `float`, `bool`, `bytes`, `None`): i loro metodi sono di
  Python, e `version.component_version.index("x")` solleva il `ValueError`
  di Python;
* tre valori immutabili non dell'SDK: `timedelta` (`Limits.deadline`),
  `pathlib.Path` (`Client.binary`, `Runner.binary`) e la tabella pyarrow di
  `TableRead.table`;
* cio' che un'eccezione dell'SDK eredita da `BaseException` (`args`,
  `add_note`, `with_traceback`, `__traceback__`...): e' l'API delle eccezioni
  di Python;
* le **copie native** che rendono i metodi di conversione, per cui esistono:
  `to_plain` e `json_default`, `PlenoraError.to_dict()`, `Limits.to_argv()`
  e `Limits.opzioni()` rendono `dict` e `list` nuovi a ogni chiamata, che
  appartengono a chi li riceve; `to_plain` copia in profondita' anche un
  ingresso gia' nativo;
* `FrozenDict.get(chiave, predefinito)` rende il **predefinito fornito dal
  chiamante, identico**, quando la chiave non c'e': e' suo, non porta dati
  dell'SDK, e normalizzarlo romperebbe `get(...) is predefinito`;
* i metodi del **protocollo di serializzazione** -- `__reduce__`,
  `__reduce_ex__`, `__getstate__`, `__copy__`, `__deepcopy__` -- rendono le
  `tuple`, i `dict` e le `list` che `pickle` e `copy` consumano: i loro
  valori restituiti sono fuori dal confine. Sono pero' definiti dall'SDK in
  ogni sua classe -- non ereditati da `object` -- e passano dalla rete: un
  protocollo che non e' un intero esatto non negativo, un `memo` che non e'
  un `dict` o un argomento in piu' sono `PlenoraError`, senza catena. Lo
  stato che serializzano e' esplicito: per un modello i soli campi dello
  schema dell'SDK, ricostruiti chiamando il costruttore (che rivalida); per
  `Client` e `Runner` i soli parametri del costruttore (il binario e il
  timeout), e la ricostruzione chiama il costruttore -- `Client` ritrova il
  binario e rilegge il manifesto;
* gli **iteratori** che rendono `iter()` e `reversed()` di un contenitore
  sono oggetti di Python, e producono valori dentro il confine; `NotImplemented`
  che `__eq__` e `__ne__` rendono per un altro tipo e' il protocollo dei
  confronti;
* cio' che ogni classe eredita da `object` senza ridefinirlo (`__format__`,
  `__sizeof__`, `__dir__`, `__init_subclass__`, `__subclasshook__`...) e' di
  Python, come l'API di `BaseException` per gli errori.

Tutti gli altri valori restituiti stanno dentro il confine: `Runner.run`
rende un `FrozenDict` congelato in profondita', e `IValoriRestituiti` lo
verifica con un `Popen` finto, insieme ai valori resi dai metodi dei
contenitori e dai loro iteratori. Congelare una risposta visita ogni nodo una
volta sola, e un sottoalbero gia' congelato non si rivisita
(`LeVisiteDelCongelamento` conta le visite).

**Dopo l'avvio di un comando che scrive, l'effetto e' `unknown`.** Ogni guasto
dopo che il processo e' partito -- la decodifica dei flussi, il JSON, il
congelamento della risposta, la busta d'errore, il modello, la lettura della
consegna di `read_table` -- e' un `ProtocolError` con `remote_effect:
unknown` per `write`, `convert` e `read` con destinazione, mai `none`
(`DopoLAvvio` inietta un guasto in ciascun punto, per ogni comando che
scrive). Il confine e' il punto esatto in cui `Popen` ha reso il processo:
da li' `registra_avvio`, la comunicazione, la decodifica dei flussi,
`Completed`, il JSON, il congelamento, la busta d'errore, la chiusura e il
timeout passano da un solo gestore (`_process._dopo_l_avvio`), che conserva
un `ProtocolError` dell'SDK e traduce ogni altro guasto -- anche un errore
dell'SDK gia' fidato di un'altra classe, o un `MemoryError` -- in un
`ProtocolError` nuovo, e per un comando che scrive porta l'effetto a
`unknown` (`IlConfineDellAvvio`, `DopoLAvvio`).

Il ritentativo resta `never`, come per ogni errore dell'SDK, anche con
`unknown`. E' una scelta, e conforme: ERRORS-1.0 ammette `unknown` con
`never`, `quarantine` o `requires_recovery` (ERR-006), e ERR-016 dice che un
effetto ignoto «normally» chiede `requires_recovery`, senza imporlo. ERR-014
(SHOULD) chiede di dire come ci si riprende: l'SDK non sa come verificare lo
stato di una destinazione scritta da un altro processo, e
`requires_recovery` prometterebbe un ripristino che l'SDK non offre. Chi
riceve `unknown` verifica la destinazione prima di decidere; il
ritentativo cieco e' escluso.

**Le interruzioni restano interruzioni.** `KeyboardInterrupt`, `SystemExit` e
le altre `BaseException` che non sono `Exception` attraversano l'SDK senza
diventare un `PlenoraError`. Questo riguarda le eccezioni asincrone generiche,
non il SIGINT raccolto dal gestore cooperativo durante il comando. Prima di
rilanciarle, se il processo e' partito, l'SDK lo chiude (lo termina e ne
raccoglie i flussi); per un comando che scrive vi aggiunge una nota fissa,
senza dati (`add_note`, o `__notes__` su Python 3.10), che dice che l'effetto
remoto va considerato `unknown`: una scrittura interrotta puo' aver lasciato
una destinazione parziale. La stessa nota la mette il client quando
l'interruzione arriva mentre decodifica la risposta di una scrittura, o legge
la consegna di `read_table`.

**La gestione del processo e' dell'SDK.** Niente `with Popen` e niente
`communicate`: due thread lettori dell'SDK accumulano stdout e stderr e
catturano ogni loro eccezione in un attributo, senza mai stamparla; l'attesa
del processo ha il timeout del client; la chiusura gira su ogni uscita --
`kill` e un'attesa limitata, ritentati; la chiusura delle pipe; l'attesa
limitata dei lettori -- e non solleva mai. Un lettore che fallisce o non
finisce e' un guasto della comunicazione, mai un successo. I guasti della
chiusura non sostituiscono l'eccezione primaria, interruzioni comprese, con
una sola eccezione: un processo che resta vivo (`processo.non_terminato`)
prevale sull'errore che ha portato a chiuderlo, e chi chiama perde la
distinzione fra timeout e guasto della comunicazione -- sapere che il figlio
puo' essere ancora in esecuzione conta di piu'. A un'interruzione si aggiunge
una nota fissa se la chiusura non e' riuscita del tutto; dopo un esito, una
chiusura che fallisce e' un `ProtocolError` (`unknown` per una scrittura). L'SDK non scrive sullo stdin del figlio.
`LaGestioneDelProcesso` prova ciascun caso -- interruzione con una pipe che
non si chiude, `SystemExit` e `MemoryError` durante l'attesa (il `kill`
precede ogni altra attesa), lettore che fallisce, `kill` che fallisce,
processo che non muore, chiusura che fallisce dopo il successo, interruzione
durante la chiusura -- catturando stderr e il gestore delle eccezioni dei
thread.

Il ritorno e' garantito dentro attese limitate, anche nei casi peggiori.
Solo il thread chiamante -- quello che ha chiamato `run` -- aspetta e uccide
il processo: lo aspetta a passi
brevi e, fra un passo e l'altro, guarda un `threading.Event` su cui i
lettori segnalano un guasto, e la scadenza del timeout del client (senza
timeout il ciclo continua, ma solo finche' i lettori sono sani). Un lettore
non tocca mai il processo: un `kill` dal suo thread correva con `wait`, e su
POSIX, con il PID gia' raccolto e riciclato, poteva colpire un altro
processo. Al guasto o alla scadenza la chiusura -- dallo stesso thread --
prova `kill`, `terminate` e `kill`, ciascuno con un'attesa limitata; **se il
processo resta vivo l'SDK non aspetta oltre**: esce con
`processo.non_terminato` (`unknown` per una scrittura), che prevale sul
guasto che ha portato a chiuderlo, e il processo puo' restare in
esecuzione. Un lettore ancora bloccato quando l'attesa limitata dei lettori
scade -- un processo nipote che tiene aperta la pipe -- e' un guasto della
comunicazione (`processo.comunicazione`, `unknown` per una scrittura). Il suo
flusso si chiude solo con la conferma esplicita che il lettore ha finito; in
ogni altro caso, anche un'interruzione mentre si decide, si abbandona al
lettore, che scarta cio' che legge ancora (continua a leggere, per non
bloccare il nipote), libera subito il buffer, e chiude il flusso da se'
quando la sua lettura torna. Il flusso e' del lettore finche' il chiamante
non lo reclama con successo e non conferma di averlo chiuso: a fine run un
`threading.Event` dice al lettore che il run e' concluso, e se la conferma
manca -- reclamo fallito, interrotto da un Ctrl-C mentre si prende il lock,
lettore ancora bloccato -- il lettore scarta, libera e chiude da se'. Nessun
passaggio di responsabilita' resta a meta': la conferma si scrive solo se
`close()` e' tornata normalmente, e il lettore non trattiene il buffer in
nessuna variabile propria. Il lettore non scarta mai byte che il run potrebbe
ancora prendere: aspetta, a passi, che il run sia **certamente finito**, cioe'
che la conclusione sia impostata **oppure** che sia morto il gettone del run,
un oggetto creato da `run` e tenuto solo da una variabile locale del suo
frame (il lettore ne ha un `weakref`). Se un'interruzione asincrona salta la
segnalazione della fine, la morte del gettone basta: nessun tempo fisso.
**Limiti dichiarati del gettone:** il gettone muore solo quando nessuno
trattiene piu' il frame di `run`. Se un'interruzione arriva prima che il
`finally` di `run` lo rilasci e imposti la conclusione, e il chiamante
conserva l'eccezione con il suo traceback (o la conservano `logging`, un
debugger, `sys.last_exc`), il frame resta vivo, e con lui il gettone: i
lettori restano vivi, con il loro flusso e il buffer, **finche' quell'eccezione
vive**. Su implementazioni senza conteggio dei riferimenti (PyPy) la morte
del gettone, e quindi il rilascio, puo' essere **differita** fino al
passaggio del garbage collector. Il `finally` di `run`, dal thread chiamante e comunque
si sia usciti dalla chiusura, fa un **ultimo tentativo**: se il processo e'
ancora vivo (`poll()` e' `None`) lo uccide e lo aspetta per un tempo breve
(1 s), senza sollevare. **Residuo dichiarato:** un'interruzione asincrona (un
Ctrl-C) che colpisce proprio quell'ultimo tentativo **puo' lasciare il figlio
vivo**. Tutti gli oggetti del run -- liste, `Event`, gettone e il suo
`weakref` -- si costruiscono prima di `Popen`: dal ritorno di `Popen` all'ingresso nel blocco
protetto non c'e' nessuna operazione. Il ciclo a passi non consuma
CPU: un processo che dorme si aspetta con `wait` di un passo alla volta, e
una prova conta le attese. **Risorsa che puo' restare occupata:** quella
pipe e il suo thread lettore (un thread daemon) restano aperti finche' il
nipote non chiude la propria estremita', senza accumulare memoria. L'output
si accumula in un solo `bytearray` per flusso, rilasciato appena
decodificato. `IProcessiVeri` e `UnSoloResponsabile` lo provano, anche con
processi veri -- l'interprete Python come figlio -- su Linux e Windows: un
nipote che tiene la pipe (il ritorno arriva dentro le attese configurate,
molto prima che la pipe si liberi), un nipote che continua a scrivere dopo
l'abbandono, un lettore guasto con un figlio che scrive 4 MiB, il drenaggio
di alcuni MiB di testo multibyte, il `kill` mai dal thread di un lettore, un
`kill` che fallisce e un processo che non muore, un'interruzione mentre si
decide se chiudere.

Nessun **attributo pubblico** -- di classe, d'istanza o proprieta' -- e' una
`tuple`, una `list` o un `dict` di Python: e' un callable dell'SDK nella
rete, un contenitore dell'SDK, un modello o un errore dell'SDK, o uno dei
valori sopra. Gli schemi dei modelli (`_OBBLIGATORI`, `_PROPRI`,
`_OPZIONALI`, `_RINOMINATI`, `_ANNULLABILI`) e `Limits._DURATA` sono privati;
`PROFILES`, `retry`, `Catalog.available`, `Layer.attributes` e
`LossReport.categories` sono contenitori dell'SDK. Lo verifica
`IlConfineDegliAttributi`, che scende in ogni classe di `__all__` -- `Client`,
`Runner` e ogni eccezione compresi -- e in un esemplare di ciascuna.

**Le eccezioni hanno lo stato fissato.** Il messaggio, l'effetto remoto e gli
attributi di ogni errore (`envelope`, `exit_code`, `command`, `searched`,
`required`, `actual`, `cleanup_failed`) si fissano nel costruttore, che non si
ripete: assegnarli, cancellarli o aggiungerne uno e' `InvalidArgumentError`, e
cosi' un secondo `__init__`. `retryable` di un `CommandFailed` non cambia piu'
dopo la costruzione. Restano liberi gli attributi che impostano l'interprete e
la libreria standard: `__traceback__`, `__context__`, `__cause__`,
`__suppress_context__`, `__notes__` e `args`; `str(errore)` e `message` leggono
il messaggio fissato, non `args`. `BinaryNotFound.searched` e' una
`FrozenList`. Dove l'SDK deve cambiare uno stato -- l'effetto che `Runner` alza
a `unknown`, il segno `cleanup_failed` -- solleva una copia nuova. `copy`,
`deepcopy` e `pickle` ricostruiscono dallo stato fissato; la copia non conta
come costruita dall'SDK.

I modelli -- un elenco chiuso di classi, `_confine.NOMI_DEI_MODELLI`,
riconosciute per identita' -- normalizzano i propri campi nel costruttore:
tipi base esatti, `FrozenList` al posto di `list`, `FrozenDict` al posto di
`dict`, modelli dell'SDK rinormalizzati, nient'altro. `FrozenList` e
`FrozenDict` sono classi proprie su `collections.abc.Sequence` e `Mapping`,
con `__slots__` e i dati in un attributo privato: nessun metodo modifica, e
ogni metodo di modifica di `list` e `dict` (`append`, `update`, `__setitem__`,
`__ior__`...) risponde con `InvalidArgumentError`. I loro costruttori
prendono al piu' un argomento, per posizione (ogni altra forma e'
`InvalidArgumentError`, senza nomi nel messaggio), normalizzano gli elementi,
e `copy`, `deepcopy` e `pickle` passano dagli stessi costruttori. Gli indici,
i componenti di uno slice e i limiti di `FrozenList.index()` sono interi
esatti o `None` prima dell'uso: il loro `__index__` non gira, un altro tipo o
un passo zero sono `InvalidArgumentError`, e un valore assente
dall'intervallo e' `ResultLookupError`. Sono uguali a una `list` o a un `dict`
con gli stessi elementi (l'altro operando si normalizza prima del confronto),
e il loro `hash` e' coerente con `==`. Non sono piu' `list` e `dict`:
`json.dumps` vuole `default=plenora_io.json_default`, oppure
`plenora_io.to_plain(...)`.
I modelli sono immutabili con lo stesso errore, non si reinizializzano, e il
loro `__post_init__` gira solo dal costruttore: richiamato a mano e'
`InvalidArgumentError`, prima di toccare un campo. Hanno `==` e `hash`
canonici: `repr`, `==` e `hash` non eseguono mai codice del chiamante, e
l'`hash` non cambia dopo la costruzione finche' l'immutabilita' non si
scavalca fuori contratto (sotto). In `TableRead` la tabella pyarrow entra
nell'`hash` come costante, e nell'`==` con l'uguaglianza di pyarrow. Due chiavi che normalizzate
coincidono sono un errore.

Gli errori non conservano dati negli attributi: `CommandFailed` porta il solo
sottocomando (`command`), da un elenco chiuso.

**La rete non e' una barriera contro codice ostile nello stesso interprete.**
Regola: PYTHON-SDK-1.0 §6 e la regola «errori senza dati». Ambito: le garanzie
di questa sezione -- immutabilita' di modelli, contenitori ed eccezioni,
`hash` stabile, ricostruzione degli errori alla rete. Valgono contro valori e
callback del chiamante, non contro codice che lavora sotto le astrazioni di
Python. Fuori contratto: scrivere con `object.__setattr__` o
`object.__delattr__`, nel `__dict__`, nel descrittore dello slot o negli
attributi privati (`FrozenDict._dati`, `FrozenList._dati`, `_messaggio`,
`_sigillato`, `_dall_sdk`, `_effetto_remoto`, le decisioni private di
`ErrorEnvelope`); chiamare la fabbrica privata degli errori
(`plenora_io._errors._dallo_sdk` e `_dallo_sdk_strutturato`), che e' cio' che
rende un errore «dell'SDK» alla rete, o un'altra funzione privata che la usa
(`_copia_json`, `_failure_from_envelope`, `_ricostruito`, `_confinato`, il
modulo `plenora_io._argomenti`); modificare il catalogo dei messaggi e i tipi
dei parametri (`MESSAGGI`, `PARAMETRI` e gli insiemi che usano, `ETICHETTE`),
il modulo `plenora_io._errors` o un altro modulo del
pacchetto. Hazard: un modello o un errore alterato cosi' non e' piu' un risultato
dell'SDK. Il suo `hash` puo' cambiare (se contiene un valore che un risultato
non porta e' `InvalidArgumentError`), e un errore puo' portare testo del
chiamante oltre la rete. Quando un modello alterato entra come campo in un
altro modello si rinormalizza; dentro un `FrozenList` o un `FrozenDict` gia'
congelato no, perche' un contenitore congelato non si rivisita. In nessun
caso le garanzie lo coprono. Rientro: nessuno
previsto dentro Python, che non ha un confine di memoria fra codice dello stesso
processo; chi ha bisogno di isolamento esegue il codice non fidato in un altro
processo.

L'inventario dei punti in cui puo' nascere un'eccezione esterna, con la prova
di ciascuno, e' `PUNTI` in `tests/test_confini_esterni.py`; nella CI con
pyarrow (`PLENORA_INVENTARIO_STRETTO=1`) un punto la cui prova non e' stata
eseguita e' rosso.

## `convert()` e le tre famiglie di opzioni

`read_options` va al driver che legge, `write_options` a quello che scrive,
`options` a entrambi. La stessa chiave puo' esistere per tutti e due con
significati diversi -- `delimiter` fra due CSV -- e un unico dizionario
costringerebbe a indovinare a chi vada.

Il ritorno non e' una pubblicazione: `publish_outcome` lo dice con il proprio
vocabolario, e `ConvertResult.published` lo legge. `LossReport` dice che cosa e'
andato perso davvero, con i conteggi e gli esempi: `lossless` e' una
scorciatoia, non l'unica informazione.

## Un Ctrl-C ferma la conversione con grazia

Nel thread principale, durante un comando, il backend nativo e quello di
processo su Unix sospendono il gestore SIGINT dell'applicazione e lo
ripristinano all'uscita. Il primo SIGINT richiede la cancellazione cooperativa:
lo SDK attende l'arresto e traduce la busta del comando in `CancelledError`,
anziche' sollevare `KeyboardInterrupt`. Un comando gia' concluso puo' ancora
restituire il proprio esito. I gestori personalizzati non vengono chiamati
in questo intervallo: vale sia per quelli che ritornano normalmente, sia per
quelli che solleverebbero `Exception` o `BaseException`. Nel nativo ulteriori
SIGINT mantengono la richiesta; il processo Unix conserva il comportamento
del CLI al secondo segnale (uscita immediata).

Su Windows il nativo offre la stessa cancellazione cooperativa. Il backend di
processo non puo' inoltrare SIGINT al figlio senza coinvolgere il processo
ospite: mantiene il gestore dell'applicazione. Se questo solleva, lo SDK
termina e attende il figlio; un'eccezione ordinaria viene redatta nel consueto
errore SDK, mentre `KeyboardInterrupt` e le altre `BaseException` risalgono.
Se il gestore ritorna, il comando continua. Non si promette in questo caso
il cleanup cooperativo dello staging. Fuori dal thread principale non si
installano gestori. Restano fuori garanzia le interruzioni asincrone generiche
(#44); le eccezioni ordinarie attraversano sempre il confine di redazione,
mentre le `BaseException` conservano la loro identita'.

Il timeout del client limita ciascuna attesa al tempo residuo. Anche un risultato
ricevuto dopo la scadenza produce l'errore SDK di timeout (`ProtocolError`);
il nativo cancella cooperativamente e attende l'arresto, mentre il backend di
processo termina il figlio. Non e' la deadline governata del motore, che ha
una propria busta `TimeoutError`.

## `validate()` conta, non consegna

E' il comando `read` della CLI, col nome che dice che cosa fa: legge il file per
intero -- ogni geometria decodificata, ogni tetto applicato -- e restituisce
quante righe ha letto, in quanti batch, con quale fedelta'. Non una riga di dati.
Chiamarlo `read()` avrebbe promesso righe che non arrivano.

Due semantiche che i nomi non suggeriscono, e che il contratto ora ratifica in
`envelopes.read.semantica`: `limit` e' una soglia verificata **fra un batch e il
successivo**, quindi `rows_read` puo' superarlo della parte residua del batch
corrente; e `truncated` significa **arresto per limite con EOF non accertato**,
non «ci sono altre righe».

## La deadline non e' il timeout

`Limits(deadline=...)` e' un budget che il **prodotto** rispetta: quando scade
risponde con una busta che descrive il lavoro fatto. `Client(timeout=...)`
uccide il processo da fuori, e quel che resta e' un `ProtocolError` che dice che
non si sa.

## Gli errori si distinguono per categoria, non per messaggio

```python
try:
    client.inspect("dati.shp")
except NotFoundError:
    ...
except CrsError:
    ...
```

La categoria e' un vocabolario chiuso del contratto, e le diciotto sottoclassi
di `CommandFailed` le corrispondono una a una -- `scripts/check_sdk_python.py`
lo verifica. Il messaggio invece e' curato per chi legge e ci riserviamo di
riscriverlo: un SDK che invitasse a `if "non trovato" in str(errore)` inviterebbe
a dipendere da una stringa che cambia senza preavviso.

L'errore porta i quattro assi interi. `retryable` e `retry_after_ms` dicono se e
quanto aspettare; `must_assume_remote_committed` dice che un ritentativo cieco
non e' sicuro -- vera per `committed`, `unknown` e `partial`, che portano alla
stessa decisione pur essendo fatti diversi, falsa solo per `none` e
`rolled_back`. Quale sia lo dice `envelope.remote_effect`, che resta intatto:
serve a chi deve scegliere se **verificare** lo stato remoto invece di riprovare.

`remote_effect` e `retry.kind` sono vocabolari **chiusi** di `plenora-error-v1`,
e sono gli assi da cui si decide se ripetere: un valore assente, `null`,
sconosciuto o di tipo sbagliato e' `ProtocolError`, non un ritentativo
permesso. `retryable` e' vero per `safe`, `after`, `requires_idempotency_key` e
`requires_recovery`; falso per `never` e `quarantine`. La **categoria** invece
resta aperta: una sconosciuta ripiega su `CommandFailed`, perche' sceglie solo
la classe dell'eccezione e non decide niente al posto di chi la riceve.

`envelope.retry` e `envelope.row_diagnostics` sono **copie immutabili**
(`FrozenDict`) di cio' che si e' passato: cambiare dopo il dizionario da cui la
busta e' nata non cambia la busta, e modificare la copia e'
`InvalidArgumentError`. Le decisioni -- `retryable`, `retry_after_ms`,
`must_assume_remote_committed` -- si fissano alla costruzione, dopo la
validazione, e non rileggono quei campi; `__post_init__` richiamato a mano non
le rifissa, perche' fuori dal costruttore e' `InvalidArgumentError`, e
`CommandFailed.envelope` non si sostituisce dopo la costruzione.
`copy.deepcopy`, `pickle`, `dataclasses.asdict` e `dataclasses.replace`
funzionano, e `replace` costruisce una busta nuova, che si rivalida.
L'SDK accetta solo i tipi che `json.loads` produce, confrontati per
**identita'**: una sottoclasse di `str` o di `dict` e' `ProtocolError`, come una
chiave ripetuta, un `NaN` o un numero che trabocca in infinito. I modelli
tengono una copia propria del documento da cui nascono. I messaggi d'errore non
riportano chiavi o contenuti del documento ne' dei flussi del binario: nominano
campi del protocollo, tipi, profondita' e lunghezze.

## L'SDK parla v2, e basta

Con successo il v2 non scrive niente su `stderr`, e l'SDK lo pretende: qualunque
cosa vi compaia e' un errore di protocollo. Non c'e' tolleranza implicita per il
v1 -- un protocollo diverso si sceglie, non si deduce dal fatto che qualcosa sia
comparso su un flusso.

## `inspect()` costa piu' di `layers()`

Per dire di che tipo e' ogni colonna il driver deve inferire lo schema, e su un
formato che non lo dichiara -- CSV, GeoJSON -- vuol dire leggere righe. Chi ha
bisogno solo dei nomi dei layer chieda `layers()`, che non paga quell'inferenza.

`assume_crs` non e' una preferenza: alcuni file dichiarano un CRS che non si
risolve, e il driver rifiuta chiuso invece di indovinare. Passarlo e' dire «lo so
io», e resta distinguibile -- `crs_resolution.status` dice da dove il CRS viene.

## Niente download impliciti

L'SDK non scarica niente, mai. Se il binario non c'e', dice dove ha cercato e
si ferma: un pacchetto Python che tirasse giu' un eseguibile da internet
sarebbe una via d'esecuzione di codice che nessuno ha chiesto e che nessun
lockfile controlla. Il binario si installa a parte -- dall'artefatto
distribuito, dalla propria pipeline, dal proprio gestore di pacchetti -- e
all'SDK si dice dove sta.

## La lingua dei nomi

L'API pubblica e' in **inglese**, come il wire che riflette: `version()`,
`catalog()`, `Driver.hostile_input_hardened` sono i nomi che stanno nelle buste,
e tradurli costringerebbe chi legge il contratto a tenere due vocabolari.
Commenti e messaggi d'errore sono in italiano, come il resto del repository.

## I modelli non divergono dal contratto

I campi delle dataclass sono confrontati con `release/cli-protocol-v2.json` da
`scripts/check_sdk_python.py`: un campo che il protocollo dichiara e il modello
non ha e' un pezzo di busta che l'SDK butta via in silenzio, e uno che il
modello ha e il protocollo non dichiara e' un campo inventato. Il gate e' rosso
in entrambi i casi.

## Uso

```python
from plenora_io import Client

client = Client()                     # cerca il binario, fail-closed
print(client.version().version)       # "2.0.0"

catalog = client.catalog()
for driver in catalog.drivers:
    if driver.available:
        print(driver.id, driver.fidelity_class)

info = client.inspect("dati.gpkg")
for layer in info.layers:
    print(layer.name, layer.geometry.crs)
    for campo in layer.attributes:
        print("  ", campo.name, campo.type)
```

Il binario si indica esplicitamente quando non sta dove l'SDK guarda:

```python
client = Client(binary="/opt/plenora-io/bin/plenora-io")
```
