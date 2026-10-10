# plenora-io — SDK Python

Un wrapper Python puro sopra la CLI `plenora-io`. Non un binding: nessun codice
nativo, nessuna estensione compilata, nessuna `cffi`. L'SDK trova un binario che
esiste gia' sulla macchina, lo esegue e decodifica le buste JSON che il
protocollo v2 dichiara.

## Perche' un wrapper e non un binding

Il confine pubblico di questo prodotto e' **la busta JSON**, non l'API Rust:
`release/cli-protocol-v2.json` la ratifica campo per campo, e
`rust_api.status` dice `internal_unstable`. Un binding legherebbe l'SDK a una
superficie che nessuno si e' impegnato a mantenere, e costringerebbe a
distribuire ruote compilate per ogni combinazione di piattaforma e versione di
Python. Un wrapper si appoggia alla sola cosa che il progetto promette.

Il prezzo e' un processo per chiamata, ed e' accettabile per il lavoro che
questi comandi fanno: leggono e scrivono file, e il costo sta li'.

## Che cosa c'e' oggi

* la scoperta del binario, **fail-closed**;
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
(`plenora_io.client.OPERAZIONI`): `catalog`, `inspect`, `layers`, `convert`
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
| 5 | ciclo di vita | non applicabile: `Client` non tiene risorse aperte, ogni chiamata e' un processo atteso fino alla fine |
| 6 | radice `PlenoraError`, cinque assi su ogni eccezione | conforme dalla 4.2.0: prima li portava solo `CommandFailed` |
| 6 | redazione dei segreti | non applicabile: nessuna credenziale; i messaggi non riportano contenuti dei documenti |
| 7 | scoperta strutturata, fail-closed | conforme: `capabilities()` rende il documento del binario con l'interfaccia e la superficie `python_sdk` (CAP-003, CAP-006) |
| 8 | cancellazione, scadenza, budget (SHOULD) | scadenza e budget in `Limits`; cancellazione limitata, sotto |
| 9 | sicurezza di rete | non applicabile: nessuna rete |
| 10 | verifica dell'artefatto installato | conforme: `scripts/smoke-pacchetto-python.sh` e `smoke-sdk-installato.py` |
| 12 | identita' delle operazioni | conforme: `OPERAZIONI` e la sua sonda |

### Limiti dichiarati

**La cancellazione non e' un parametro.** Regola: PYTHON-SDK-1.0 §8 (SHOULD).
Ambito: tutti i metodi. Un Ctrl-C e' inoltrato al prodotto soltanto dal thread
principale e fuori da Windows (`Client.cancellable`); non c'e' un token che il
chiamante possa armare da un altro thread. Hazard: un lavoro lungo avviato da
un thread secondario o su Windows si ferma solo con `Client(timeout=...)`, che
uccide il processo e rende `ProtocolError` con `remote_effect: unknown` se il
comando scriveva. Rientro: un token di cancellazione dell'SDK inoltrato al
processo su entrambe le piattaforme.

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
tolta a mano.

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

Il segnale viene **inoltrato** al prodotto, che al primo arma un token
cooperativo: la pipeline lo osserva ai propri punti di verifica e torna un
`CancelledError` con la destinazione ripulita. Al secondo, il processo esce.

Il gestore vive per la durata della singola esecuzione e viene rimesso com'era:
una libreria non e' padrona del gestore dei segnali di chi la ospita. Fuori dal
thread principale, e su Windows, l'inoltro non si arma e il comando funziona lo
stesso -- `Client.cancellable` lo dice.

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

`envelope.retry` e `envelope.row_diagnostics` sono **copie** ordinarie di cio'
che si e' passato: cambiare dopo il dizionario da cui la busta e' nata non cambia
la busta. Le decisioni -- `retryable`, `retry_after_ms`,
`must_assume_remote_committed` -- si fissano alla costruzione, dopo la
validazione, e non rileggono quei campi: chi modifica `envelope.retry` cambia la
propria copia, non la decisione. `copy.deepcopy`, `pickle`,
`dataclasses.asdict` e `dataclasses.replace` funzionano, e `replace` rivalida.
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
