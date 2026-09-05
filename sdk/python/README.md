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
* `--version`, `catalog`, `inspect`, `layers` e `validate`, con i modelli
  tipizzati e i tetti in `Limits`.

`convert` **non** c'e' ancora.

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
non e' sicuro -- vera per `committed` e per `unknown`, che portano alla stessa
decisione pur essendo due fatti diversi. Quale dei due sia lo dice
`envelope.remote_effect`, che resta intatto: serve a chi deve scegliere se
**verificare** lo stato remoto invece di riprovare.

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
