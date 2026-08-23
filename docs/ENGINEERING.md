# Ingegneria — come funziona e come viene verificato

Questo documento descrive la costruzione interna e la verifica. La superficie
che un consumatore vede sta in [PRODUCT.md](PRODUCT.md); lo stato del percorso
di rilascio sta in [RELEASE.md](RELEASE.md).

---

## Struttura dei crate

```
plenora-io-model     tipi semantici: contratti, geometria, WKB/WKT, errori, budget
plenora-io-core      pipeline: registry, adapter di lettura, publish, capability
driver-common        codice condiviso fra driver: WKT lossless, prevalidazione Arrow
driver-<formato>     dieci driver, uno per formato
plenora-io-cli       binario `plenora-io` e le buste JSON
plenora-bench        misure di prestazione, non spedito
plenora-fuzz         attrezzaggio di fuzzing, non spedito
```

Le dipendenze vanno in una direzione sola: i driver dipendono da `core` e da
`model`, mai fra loro. `model` non conosce i driver.

`vendor/dxf` e `vendor/gdal` sono fork governati, risolti via
`[patch.crates-io]` e fissati da un lock più un registro di provenienza.

Il lock ne fissa il **tree hash**, calcolato **esclusivamente sull'insieme che
git traccia**. Un artefatto di build non può alterarlo — è il punto: un lock
ricalcolato con `vendor/<crate>/target/` presente registrerebbe un residuo come
contenuto del fork governato. Un file estraneo viene comunque **rifiutato a
parte**, perché non alterare l'impronta non vuol dire ignorare.

Due conseguenze operative:

* impacchettare un fork richiede `--target-dir` **fuori** dall'albero
  vendorizzato. `cargo package` scrive altrimenti in `vendor/<crate>/target/`.
  Quell'artefatto **non entra nel tree hash** — l'impronta guarda solo ciò che
  git traccia, ed è la proprietà che impedisce a un lock ricalcolato di
  registrarlo come contenuto del fork — ma viene **rifiutato a parte**, come
  file estraneo dentro un albero governato. Le due difese sono indipendenti:
  l'impronta non si può avvelenare, e l'estraneo non resta invisibile;
* `vendor/gdal` **non è impacchettabile** con `cargo package`: conserva
  `.cargo_vcs_info.json`, un nome che Cargo riserva. Il metadato è tenuto di
  proposito per l'attribuzione, e il crate non viene mai pubblicato — è
  risolto per path. Il limite è di `cargo package`, non del fork.

## Interfaccia dei driver

Un driver implementa `FormatDriver`, che ha due soli metodi statici:

```rust
fn open(&self, source: Source, opts: ReadOptions) -> Result<Box<dyn OpenDatasetHandle>>;
fn create(&self, sink: Sink, plan: &WritePlan, opts: &WriteOptions) -> Result<Box<dyn FormatWriter>>;
```

`open` non legge righe: legge header, schema e CRS, e restituisce un handle.
Lo **stato mutabile vive nel reader**, non nell'handle: da un handle si aprono
più reader indipendenti se il formato lo consente, e il descrittore dichiara
quanti.

`create` verifica che il contratto sia **rappresentabile** prima di toccare la
destinazione. Un piano che il formato non sa scrivere è rifiutato in fase di
validazione, non a metà scrittura.

`ReadOptions` è consumato **per valore**, e non è una preferenza stilistica: le
opzioni trasportano l'`InputPermit`, che il preflight estrae per `move`. Da un
riferimento condiviso non si estrae nulla, e le alternative che lo conservassero
reintrodurrebbero proprio ciò che il permit esiste per escludere — uno stato
mutabile dietro una firma immutabile, e la possibilità di osservare due volte la
stessa sorgente.

## Pipeline di lettura

```
Source ──preflight──> open ──> OpenDatasetHandle ──> open_layer_reader ──> next_batch*
           │                        │                                          │
           │                        └─ contratti di layer                      └─ RecordBatch
           └─ enumera, addebita, pubblica il footprint
```

Il **preflight** enumera la sorgente e ne addebita il costo al budget prima che
il driver la apra. Per i formati a lettura materializzata è l'unica difesa: il
parser ha bisogno di tutto l'input prima di consegnare la prima riga, e un
limite verificato dopo non è un limite.

«Materializzata» descrive **quanto input serve**, non dove sta. Il supporto
fisico è un asse separato — il buffering — e confondere i due è il difetto che
il modello tiene distinto per costruzione: un parser che riversa tutta la
sorgente in uno spool RAM-poi-disco è materializzato con buffering adattivo, e
la coppia dice che serve tutto l'input ma non tutta la memoria.

L'adapter comune impone l'**atomicità operativa**: qualunque sia la modalità
nativa del formato, il consumatore vede uno stream sequenziale, e un errore a
metà non consegna righe. Ciò che il driver produce in anticipo finisce nello
spool.

### Spool e memoria

Il buffering è adattivo: memoria finché il budget lo consente, poi un file
temporaneo. La soglia è `memory_bytes`, e lo spool è **bounded** — superarlo è
`ResourceLimit`, non una scrittura senza fine.

Il file temporaneo **non ha nome**. È creato con `tempfile::tempfile_in`, cioè
scollegato dal filesystem appena aperto su Unix e con `FILE_FLAG_DELETE_ON_CLOSE`
su Windows: nessun altro processo può aprirlo, perché non esiste un path da
aprire, e non restano orfani da spazzare nemmeno dopo un `SIGKILL` — il kernel
libera l'inode alla chiusura del descrittore.

`PLENORA_SPILL_DIR` sceglie la directory che ospita quell'inode, e serve a
metterlo su un volume capiente o veloce; senza la variabile si usa la directory
temporanea di sistema. Non vive accanto alla destinazione. Se la variabile è
impostata ma la directory non è utilizzabile, la creazione **fallisce chiuso**:
un ripiego silenzioso metterebbe dati su un volume che l'operatore non ha
scelto.

### Projection e pruning

`ReadRequest` distingue tre cose che è facile confondere:

| | |
|---|---|
| **projection** | quali colonne servono. Over-return ammesso, under-return vietato: un reader può restituire più colonne di quelle chieste, mai meno |
| **pruning** | quali blocchi si possono saltare senza leggerli. È un'ottimizzazione: saltare meno è sempre corretto |
| **filtering** | **non esiste**. La libreria non filtra righe: quella è responsabilità del chiamante |

`BatchTarget` è una **stima**, non un limite di memoria: le colonne a larghezza
variabile non consentono di conoscere in anticipo la dimensione di un batch.

La geometria è inclusa solo se la projection la richiede, se serve a un hint di
pruning spaziale, o se il contratto del consumatore la impone — mai forzata per
letture puramente tabellari.

## Pipeline di scrittura

```
WritePlan ──validate_write──> create ──> write_to_layer* ──> finish ──> Published
              │                                                 │
              └─ capability-check per formato                   └─ publish atomico
```

Il **capability-check** è fail-closed: ciò che il formato non dichiara di saper
scrivere viene rifiutato. I driver possono aggiungere vincoli propri, mai
rimuoverne.

Il **publish** è atomico in tre forme, secondo il formato: file singolo
(rename), multi-file (tutti visibili insieme o nessuno), multi-layer. Con
`--durable` il publish esegue `fsync` prima del rename, e l'esito di durabilità
è riportato: un `fsync` fallito **dopo** il rename è un caso distinto da un
publish non avvenuto, e il chiamante deve poterli separare.

Una scrittura rifiutata non lascia una destinazione.

## Difese sui formati ostili

| | |
|---|---|
| **prevalidazione** | i decoder compressi verificano rapporto e struttura **prima** di materializzare le celle |
| **barriera anti-panic** | i parser di terze parti che possono panicare girano dietro una barriera che converte il panico in errore tipizzato. Un panico catturato resta un panico avvenuto: la barriera è l'ultima difesa, non la prima |
| **GeoParquet** | larghezza del dizionario, allocazione di pagina e statistiche sono validate prima dell'uso; il pruning è fail-open, quindi una statistica sospetta fa leggere di più, mai di meno |
| **WKB/WKT** | ogni geometria passa da tetti su byte, componenti e profondità, in lettura e in scrittura |

## Gestione degli errori

Ogni errore pubblico porta il quartetto `(category, phase, code, retry)` più
`remote_effect`. Il testo è scelto a compile time: vedi
[PRODUCT.md § PublicMessage](PRODUCT.md#publicmessage--il-testo-è-scelto-a-compile-time).

I nomi di campo e di layer non entrano nel testo: entrano in campi tipizzati che
non raggiungono il wire.

---

## Verifica

### Due livelli

| | Quando | Che cosa esegue |
|---|---|---|
| **livello 1** | durante una tranche | tutti i passi del checkpoint **tranne** fuzz e copertura |
| **livello 2** | a chiusura di una tranche | tutto |

```
S9_LIVELLO=1 bash scripts/s9-checkpoint.sh    # livello 1
bash scripts/s9-checkpoint.sh                 # livello 2
```

Il livello 1 **non è un elenco a parte**: è lo stesso script con i passi pesanti
omessi. Aggiungere un passo al livello 2 lo aggiunge al livello 1 per omissione,
che è il verso giusto dell'errore — si può dimenticare di alleggerire un passo,
non di includerlo.

Una batteria composta a mano diverge dal checkpoint, e diverge in silenzio.

#### I nove passi pesanti

L'elenco è **chiuso**: `fuzz_replay`, `fuzz_smoke`, `coverage_pulizia`,
`coverage_misura`, `coverage_export`, `coverage_report_non_vuoto`,
`check_coverage_exclusions`, `coverage_soglia_dal_report`,
`coverage_soglia_controprova`.

Un decimo nome è un errore di programmazione dello script e rossa in entrambe le
modalità: marcare pesante un gate lo farebbe sparire dal livello 1 in silenzio.

*Omesso* non è *saltato*: un passo saltato doveva girare e conta fra i falliti;
un passo omesso non doveva girare, ma resta **stampato**. Un esito che non
elenca ciò che non ha misurato è il difetto che la modalità esiste per chiudere.

#### Integrità della misura

Due passi, e sono distinti perché misurano cose diverse:

| | |
|---|---|
| `revisione_invariata` | `git rev-parse HEAD` **riletto** a fine corsa e confrontato con quello iniziale |
| `albero_invariato` | impronta sha256 dell'albero, confrontata in testa e in coda |

L'impronta combina `git diff --cached` e `git diff`, entrambi con `--binary
--no-ext-diff --no-textconv`, e per ogni file non tracciato e non ignorato il
percorso più l'hash del contenuto, delimitati. Porta un prefisso versionato:
impronte di versioni diverse **non sono confrontabili**, e ciò che conta è il
confronto inizio/fine della stessa corsa.

Un conteggio di file sporchi non basterebbe: un passo che modifichi un file già
`M` lo lascia identico.

L'impronta **fallisce** invece di restituire il vuoto se un comando git non
acquisisce. Acquisire prima e hashare dopo è la differenza fra «albero pulito» e
«git rotto», che altrimenti darebbero lo stesso valore.

Al livello 2 l'albero deve essere pulito in partenza; al livello 1 può essere
sporco, ma deve restare sporco **allo stesso modo**.

#### Il risultato sta su disco

L'esito viveva solo sullo stdout. Una corsa è già stata scartata per intero
perché il container girava con `--rm`: il verdetto era stato osservato, le
misure no, e non si combinano il verdetto di una corsa e i numeri di un'altra.

Ogni corsa scrive ora `risultato.json` — livello, esito, revisione e impronta
iniziali e finali, passi, verdi, omessi e i nomi dei passi rossi — accanto agli
artefatti, oppure dove punta `S9_CHECKPOINT_RISULTATO`. La scrittura è
**atomica**: il file viene prodotto a parte nella stessa directory e poi
rinominato, quindi chi legge vede la versione precedente o quella nuova, mai
metà della nuova.

Il file compare all'avvio con esito `in_corso`, e viene sostituito alla fine. È
la differenza fra «non è mai partita» e «è morta a metà», che nessun altro
artefatto distingue. Se il risultato non è scrivibile la corsa **fallisce**
invece di proseguire: un esito che vive solo sullo stdout è ciò che il file
esiste per evitare.

Ogni uscita terminale lo sostituisce, non solo quelle che misurano:
`albero_sporco`, `impronta_iniziale_non_calcolabile`, `non_superato`,
`livello_1_verificato`, `superato`. Un rifiuto in partenza ha una causa nota, e
lasciare `in_corso` lo farebbe leggere come una corsa morta a metà.

Il file porta anche l'**elenco dei passi** — identità, esito e log di ciascuno —
e non solo i contatori. I contatori dicono quanti; l'elenco dice quali, ed è ciò
che permette di riconciliare gli artefatti con i passi invece di crederli sulla
parola.

`exit` **non compare fuori da `concludi`**, e la regola è verificata invece che
ricordata: una sonda rimuove il corpo di quella funzione dal testo dello script
e pretende che in ciò che resta non ce ne sia nessuna. Una seconda sonda le
inietta un'uscita vietata su un file costruito apposta, perché una regola che
non può fallire non verifica niente.

### Fuzzing

| | |
|---|---|
| **replay** | rigioca semi, corpus e artefatti noti su tutti i target. Deterministico: trova sempre ciò che c'è |
| **smoke** | cerca input nuovi per un tempo limitato. Ritrova il noto solo per fortuna, ed è la ragione per cui il replay viene **prima** |
| **quarantena** | target esclusi dallo smoke ma **compilati comunque**. Deve essere vuota |

I target sono tredici. La corrispondenza con i driver **non è uno a uno**, e la
differenza conta:

| Driver | Target |
|---|---|
| csv, geojson, kml, dxf, xls, ipc, geoparquet, gpkg | reader dedicato |
| gpkg | anche `gpkg_geometry` |
| ipc | anche `ipc_to_gpkg`, che esercita la conversione |
| model | `from_wkb`, `wkt_parse` |
| **shp** | soltanto `shp_wkb`, che converte fra WKB e forme ESRI. **Non è un reader di Shapefile**: il parsing di `.shp` e `.dbf` non è esercitato da nulla |
| **filegdb** | **nessuno** |

### Copertura

Misurata con `cargo llvm-cov`, soglia 80%. Il checkpoint riporta **due
proiezioni** dello stesso profdata — i record `DA:` del report LCOV e la colonna
«Lines» di `llvm-cov` — che contano insiemi diversi di righe strumentate e non
sono intercambiabili. Entrambe sono richieste.

La catena è **concatenata**: pulizia, misura, export, report non vuoto,
esclusioni, soglia dal report, controprova. Ogni passo dipende dal **precedente**,
non dal primo: un export fallito non deve lasciar girare la soglia su un file
vecchio.

Il checkpoint riporta anche la **copertura delle sole righe cambiate** rispetto
a una baseline. Non è una soglia e non ne ha una: distingue la crescita
meccanica del denominatore da un ramo semantico non esercitato.

### ASSURANCE-N1 — copertura dei rami negativi

Registro dei rami d'errore mai eseguiti, in
`assurance/registries/assurance-n1-copertura-negativa.json`. Tre stati, e la
distinzione è il punto:

| | |
|---|---|
| **aperto** | il ramo non è coperto e nessuno ha dimostrato che non possa esserlo |
| **coperto** | un test **eseguito** lo attraversa e ne verifica il contratto |
| **irraggiungibile** | il ramo non è raggiungibile dall'API pubblica perché una guardia a monte rifiuta prima. Deve dichiarare le **righe** che restano scoperte e la **guardia**, e un test prova la precedenza |

Un ramo irraggiungibile **non è un ramo coperto**, e presentarlo come tale
sarebbe la compensazione che il registro esiste per escludere.

La precedenza delle guardie è un invariante **interno** di ASSURANCE-N1: serve a
mantenere vera l'irraggiungibilità. Non è una promessa di compatibilità
pubblica, e se cambiasse il test rossa per dire che il gruppo va riaperto.

#### Una prova è un test eseguito

Un simbolo che esiste può essere un helper senza `#[test]`, un test `#[ignore]`,
un test sotto un `cfg` inattivo o un omonimo. Nessuno dei quattro copre un ramo.

`scripts/check_assurance_n1_prove.py` **esegue** il harness per ogni coppia
`(crate, configurazione)` distinta — deduplicata — e pretende che ogni identità
dichiarata compaia una volta sola e con esito `ok`. La configurazione fa parte
dell'identità: `--all-features` abilita `gdal-backend`, e il ramo stub di
`driver-filegdb` esiste solo senza.

Due modalità, e il verde dell'una non è il verde dell'altra: `--integrita` dice
che il registro è coerente, `--release` è rossa finché il debito non è a zero.

### Il vecchio gate di readiness della patch 1.0.1

È stato **ritirato** perché non governa il percorso di release corrente; gli
invarianti ancora applicabili sono nel registro del contratto corrente.

---

## Misure di prestazione

`plenora-bench` misura throughput, picco RSS, allocazioni e codifica/decodifica
WKB per driver, e archivia una baseline sotto `baseline/`.

**Non è un gate di rilascio**, ed è escluso dai gate di CI insieme a
`plenora-fuzz`: è attrezzaggio, non componente distribuibile. Una regressione
prestazionale non blocca oggi nulla automaticamente — va confrontata a mano
contro la baseline archiviata.

L'unico benchmark cablato in CI è quello **Windows / FileGDB narrow**, che
produce un artefatto misurato a ogni corsa.

## Registri macchina

Nessun documento è un database. I gate leggono file strutturati:

| Percorso | Contenuto | Gate |
|---|---|---|
| `assurance/current-state.json` | stato corrente misurato | `check_docset` |
| `assurance/registries/release-contract-current.json` | invarianti che governano ancora | `check_release_contract.py` |
| `assurance/registries/fallback-register.json` | ogni degradazione a un ripiego, con la ragione | `check_assurance_fallbacks.sh` |
| `assurance/registries/dependency-exceptions.json` | advisory accettati, con condizione di chiusura | `audit_ignores.py`, CI |
| `assurance/registries/vendor-{dxf,gdal}-fork.json` | provenienza dei fork | `check_{dxf,gdal}_fork.py` |
| `assurance/registries/assurance-n1-copertura-negativa.json` | rami negativi | `check_assurance_n1*.py` |
| `assurance/registries/quartetto-siti.json` | quartetto per sito di costruzione | `check_quartetto_sito.py` |
| `release/cli-protocol-v1.json` | le sei buste della CLI | `check_release_contract.py` |
| `release/system-rc-gate.json` | qualifica cross-component | esterno |
| `assurance/evidence/checkpoint-<sha>.json` | la corsa che ha prodotto i numeri dello stato — **una sola**, la corrente | `check_release_contract.py` |

La directory delle evidenze contiene soltanto quella citata da
`current-state.json`. Un'evidenza che nessun gate legge è un documento che
nessuno rilegge, e la sua presenza invita a confronti fra corse che l'albero non
permette di ricostruire — una delle precedenti portava un digest anteriore alla
forma canonica, quindi non ricalcolabile. Git conserva la storia; l'albero di
lavoro dice che cosa vale oggi.

Gli artefatti della corsa sono **riconciliati con i passi**: l'evidenza porta
l'elenco delle 57 identità con esito e log, i conteggi ne sono il riassunto
derivato, e il manifest dev'essere esattamente i 55 log dei passi più
`catalog.json`, `coverage_diff.log`, `lcov.info` e `risultato.json`. Un manifest
ridotto lascerebbe la riconciliazione a dichiarare passi di cui non resta
traccia. `coverage_diff.log` ne fa parte, quindi la diagnostica differenziale è
obbligatoria per qualificare: un'evidenza descrive una corsa completa.

L'evidenza citata da `current-state.json` è verificata **nella propria coerenza
interna** prima di essere usata come fonte: una sola revisione fra inizio e
fine, una sola impronta, conteggi dei passi che tornano fra loro e con un esito
superato, misure che non lo contraddicono, e digest degli artefatti ricalcolato
dal manifest che lo accompagna. Una copia fedele di un documento che si
contraddice non è una verifica.

---

## Comandi canonici

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-features
cargo test --workspace
cargo test --workspace --all-features --doc
```

Le due configurazioni sono entrambe necessarie: `--all-features` abilita
`gdal-backend`, e il ramo stub di `driver-filegdb` **non viene compilato** con
quella feature attiva.

Il fuzzing gira in container, con la toolchain nightly e gli strumenti LLVM
richiesti da `cargo fuzz coverage`:

```bash
cargo +nightly fuzz build
bash scripts/fuzz-replay.sh [target]
bash scripts/fuzz-smoke.sh [target]
```

## Flusso di lavoro

```
tranche ──> livello 1 ──> commit ──> livello 2 ──> evidenza in un commit separato
```

**Una tranche per commit.** Non si comincia la successiva nello stesso commit:
il valore dello staging è che ogni passo sia verde da solo, e due cambi
insieme non si possono più separare quando uno rossa.

**L'evidenza sta in un commit distinto da quello misurato**, e dichiara di non
ereditare la misura: i numeri valgono per lo SHA misurato e per nessun altro
albero.

**Le sonde distruttive girano su copie isolate.** Una sonda che muta l'albero
per verificare un gate deve lavorare su una copia, con il workspace montato in
sola lettura: un test che rompe l'albero che sta misurando non è un test. Il
ripristino non usa `git checkout`, che cancellerebbe modifiche non committate.
