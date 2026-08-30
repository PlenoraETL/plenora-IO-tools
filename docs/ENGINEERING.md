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
temporaneo. Lo spool è **bounded** — superare la quota di spill è
`ResourceLimit`, non una scrittura senza fine.

La soglia di migrazione **non è `memory_bytes`**: è la metà della capacità di
memoria *effettiva*, cioè metà del minimo fra la quota locale della pipeline e
quella del pool condiviso quando un pool c'è. L'altra metà resta al batch che il
reader sta materializzando: con la soglia al 100% il buffer potrebbe consumare
l'intera quota e far fallire la materializzazione del batch successivo, cioè
rendere lo spool inutile proprio nel caso che deve risolvere. Derivarla dalla
capacità effettiva e non dal solo limite locale ha la stessa ragione: con un pool
più stretto, metà del limite locale sarebbe irraggiungibile e la migrazione non
scatterebbe mai.

Ne segue un vincolo che chi stringe `--memory-bytes` incontra: la quota deve
restare **oltre il doppio** di un batch materializzato, il cui target di default
è 8 MiB. Sotto quel valore il buffer sotto soglia e il batch in arrivo non ci
stanno insieme, e l'operazione si ferma con `LIMIT_EXCEEDED` — «batch
materializzato oltre la quota prenotata». È un rifiuto corretto, e non dice
niente sul buffering.

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

Il **publish** ha tre forme, e **due sole** sono crash-atomic: il rename di un
file singolo e il rename di una directory-dataset. La terza — il set di file
sciolti dello Shapefile, `*.shp` più companion — non lo è e non può esserlo,
perché quattro file separati non diventano visibili in un atto solo. Il
contratto di quella forma, l'opt-in che la richiede e la procedura di recovery
stanno in [PRODUCT.md § Publish](PRODUCT.md#publish-che-cosa-diventa-visibile-e-quando).

Con `--durable` il publish esegue `fsync` prima del rename, e l'esito di
durabilità è riportato: un `fsync` fallito **dopo** il rename è un caso distinto
da un publish non avvenuto, e il chiamante deve poterli separare.

Una scrittura rifiutata non lascia una destinazione, **tranne** nel set sciolto
con rollback fallito: lì l'errore dichiara `RemoteEffect::Partial` e
`RetryDisposition::RequiresRecovery`, ed è l'unico posto del componente dove un
errore ammette che il filesystem possa essere rimasto sporco. Ammetterlo è la
ragione per cui quei due campi esistono: un errore che tacesse sarebbe peggiore
del set sciolto stesso.

### Che cosa invecchia quando cambia una dipendenza

Gli artefatti di profondità del fuzzing e il confine ASan non dichiarano una
proprietà: la **misurano**, e portano l'impronta del perimetro che la determina.
Quando quel perimetro cambia, l'impronta non coincide più e il gate diventa
rosso — è il meccanismo che impedisce a una misura di sopravvivere al codice che
descriveva.

`Cargo.lock` è **dentro il perimetro di tutti**. Ne segue un fatto operativo che
conviene conoscere prima di aggiungere un crate, invece di scoprirlo un rosso
per volta:

| che cosa rimisurare | come |
|---|---|
| `assurance/profondita-fuzz-shapefile.json` | `bash scripts/fuzz-profondita.sh shp_reader` |
| `assurance/profondita-fuzz-geojson.json` | `bash scripts/fuzz-profondita.sh geojson_reader` |
| `assurance/profondita-fuzz-wkt.json` | `bash scripts/fuzz-profondita.sh wkt_parse` |
| `assurance/profondita-fuzz-filegdb.json` | `bash scripts/fuzz-profondita.sh filegdb_reader` |
| `assurance/asan-filegdb.json` | `bash scripts/asan-filegdb.sh` |

**Sono cinque, e la quinta è quella che si dimentica**: `asan-filegdb.json`
porta la stessa impronta del perimetro FileGDB, quindi resta rossa da sola dopo
che le quattro profondità sono tornate verdi. Se la sua corsa cambia
`contatori_di_copertura`, il valore va allineato anche in
`assurance/current-state.json` (`chiuso.fuzz_filegdb`), perché
`stato.fonti-legate` pretende che i due coincidano.

Le misure si rifanno **dopo** aver finito ogni modifica al codice, non durante:
gli artefatti registrano anche la riga di ciascun simbolo, quindi invecchiano
pure quando il codice si limita a spostarsi. Lo stesso vale per una modifica ai
soli commenti dentro un file del perimetro.

### Dove finiscono i log di una corsa

`S9_CHECKPOINT_LOG_DIR` sceglie dove il checkpoint scrive i propri passi e il
`risultato.json`. Il default è `/tmp`, che dentro un container sparisce con il
container: l'evidenza di una misura non si ricostruisce da un rapporto, quindi
una corsa di cui contano gli artefatti va puntata su un percorso che sopravvive.

`/.s9-checkpoint/<sha>-attempt-<n>` è quel percorso, ed è ignorato da Git per
due ragioni distinte. La prima è ovvia: sono artefatti locali. La seconda no —
l'impronta dell'albero enumera i file non tracciati **non ignorati**, quindi una
directory di log visibile renderebbe rosso `albero_invariato` per colpa dei log
della corsa che lo sta calcolando.

Una sottodirectory **nuova e vuota per tentativo**, non una riusata: i log di
una corsa precedente verrebbero attribuiti a quella nuova, o entrerebbero nel suo
manifesto degli artefatti.

`target/` sarebbe altrettanto persistente — nella ricetta Docker è un *volume*,
che sopravvive alla rimozione del container, ed è ciò che le corse fino a
`f4f8471` usavano. La differenza è che dal bind mount i log si leggono
direttamente, mentre da un volume serve un secondo container che lo monti.

## Difese sui formati ostili

| | |
|---|---|
| **prevalidazione** | i decoder compressi verificano rapporto e struttura **prima** di materializzare le celle |
| **barriera anti-panic** | i parser di terze parti che possono panicare girano dietro una barriera che converte il panico in errore tipizzato. Un panico catturato resta un panico avvenuto: la barriera è l'ultima difesa, non la prima |
| **GeoParquet** | larghezza del dizionario, allocazione di pagina e statistiche sono validate prima dell'uso; il pruning è fail-open, quindi una statistica sospetta fa leggere di più, mai di meno |
| **WKB/WKT** | ogni geometria passa da tetti su byte, componenti e profondità, in lettura e in scrittura. Per il WKT i tetti si applicano **durante** il parse (S12): l'analisi costruisce la geometria mentre consuma il testo e addebita ogni coordinata quando la legge, quindi ciò che non è stato letto non è stato allocato |

#### La capability `hostile_input_hardened`

Il catalogo la dichiara per driver, e dice una cosa sola: ogni testo che quel
driver interpreta come geometria passa da un'analisi che applica i tetti
**durante** il parse. `false` non dice «insicuro» — dice **non dichiarato**: un
driver binario ha altre difese, e riassumerle in un booleano solo lo renderebbe
inutile.

Oggi la dichiarano `csv`, `xlsx` e `geojson`.
`scripts/check_capability_input_ostile.py` non le crede: confronta la
dichiarazione con gli entry point che il driver attraversa davvero, nei due
versi — un `true` senza il parser è rosso, e un parser senza il `true` pure,
perché una garanzia che non è dichiarata è una garanzia che nessuno può usare.

Il campo entra nel catalogo come additivo opzionale, che la regola del
protocollo consente **con un record d'impatto**: il record è in
`release/cli-protocol-v1.json`, e il gate del contratto lo pretende.

#### L'unica incompatibilità osservabile di S12

Il parser WKT progressivo accetta esattamente ciò che accettava il precedente —
una sonda lo confronta con esso su oltre trecento casi generati per
combinazione — **salvo una cosa**: il testo non-whitespace che segue la
geometria era ignorato e ora è rifiutato.

`POINT (1 2))` e `POINT (1 2) POINT (3 4)` erano un punto, e il resto non
c'era. Una cella WKT rappresenta una geometria completa: ignorare una parentesi
in più o una seconda geometria nasconde un input malformato e contraddirebbe la
garanzia `hostile_input_hardened`. È un bug del confine precedente, non una
sintassi da conservare.

Lo spazio finale resta accettato, perché non è testo. Il rifiuto è un errore di
**sintassi** (`DataMapping/Validate/Wkb/Never`), non di budget: dire «limite
superato» a chi ha una parentesi di troppo lo manderebbe ad allargare una quota
che non c'entra.

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

I target sono quindici. La corrispondenza con i driver **non è uno a uno**,
e la differenza conta:

| Driver | Target |
|---|---|
| csv, geojson, kml, dxf, xls, ipc, geoparquet, gpkg | reader dedicato |
| gpkg | anche `gpkg_geometry` |
| ipc | anche `ipc_to_gpkg`, che esercita la conversione |
| model | `from_wkb`, `wkt_parse` |
| shp | `shp_wkb` (conversione WKB ↔ forme ESRI) e `shp_reader` (il formato) |
| filegdb | `filegdb_reader`, con il confine misurato descritto sotto |

#### FileGDB, e un confine che va detto

`filegdb_reader` attraversa il percorso vero: entry point con `gdal-backend`,
catalogo, schema, righe. Un FileGDB però non è un file ma una **directory** di
tabelle che si citano per GUID, e il formato è proprietario: costruirne uno da
un blob significherebbe riscrivere `OpenFileGDB`. Il target parte quindi da una
fixture **vera** — prodotta da `ogr2ogr` da un GeoJSON versionato — e ne
sostituisce una parte per volta.

Il limite è misurato, non dichiarato: `libgdal.so` è di sistema e **non
strumentata**, un solo modulo porta contatori di copertura e zero file sorgente
C/C++ compaiono nei dati di copertura.

| | |
|---|---|
| AddressSanitizer **copre** | il nostro codice per intero, e l'intercettazione dell'allocatore al confine: un accesso di GDAL nella redzone di un'allocazione ASan viene visto |
| **non copre** | gli accessi interni a GDAL — stack, globali, o dentro l'allocazione |
| il fuzzer **non è guidato** | dentro GDAL: nessun contatore, nessun feedback |

Una campagna verde dice che il percorso Rust regge input ostili e che GDAL non è
stato portato a un crash **osservabile**. Non dice che GDAL sia stato esplorato.

`shp_reader` è arrivato dopo, e la ragione dice qualcosa sul metodo. Uno
Shapefile non è un file: il driver riceve il `.shp` e risale ai fratelli
cambiando estensione. Un target che consegni al fuzzer un solo blob non apre
niente, e per anni `shp_wkb` è stato contato come «copertura di shp» pur non
leggendo un header, una tabella `.dbf` o un `.prj`.

Il target divide perciò l'input in quattro parti — `.shp`, `.shx`, `.dbf`, il
resto come `.prj` — e le materializza in una directory temporanea **nuova a ogni
invocazione**: se fosse riusata, il `.prj` di una mutazione sopravvivrebbe alla
successiva e il fuzzer misurerebbe la propria directory. Le lunghezze dichiarate
si **saturano** invece di far scartare l'input, così le mutazioni
sull'intestazione restano casi di prova; nessuna allocazione deriva da un valore
dichiarato e nessun percorso dal payload.

Alle prime campagne il target ha aperto una **famiglia** di difetti, non un
difetto: ogni valore che i due decoder leggono dal file viene usato come se il
file l'avessero scritto loro. Offset e larghezze diventano indici di fetta,
conteggi diventano capacità di vettore, differenze fra voci d'indice diventano
numeri di punti da leggere. L'elenco per esteso è in
[RELEASE.md](RELEASE.md); qui conta la forma, che è la stessa già nota per
`arrow-ipc` e `parquet`.

Il driver faceva già alcuni controlli, ma **dopo** aver costruito il reader: il
panico arrivava prima. Le due prevalidazioni sono ora funzioni a sé, sorvegliate
dallo stesso gate degli altri decoder, che pretende la verifica **prima** della
costruzione e in nessun'altra crate.

I semi non sono blob committati: `scripts/genera_semi_shp.py` li **deriva** dalla
specifica del formato — deliberatamente non dal writer del driver, che li
renderebbe validi per costruzione anche il giorno in cui sbagliasse — e
`--verifica` li ricontrolla byte a byte in CI. Che raggiungano il parsing non è
dedotto dal fatto che il replay non crasha: le sonde di `driver-shp` chiamano lo
**stesso** entry point del target sui semi versionati e verificano il numero di
righe drenate e il messaggio esatto dei rifiuti.

### Copertura

Misurata con `cargo llvm-cov --all-features`, soglia 80%. Le feature contano:
`driver-filegdb` tiene l'intero percorso GDAL dietro `gdal-backend`, e senza
quella feature quel codice non era «scoperto» ma **invisibile** — fuori dal
denominatore, quindi incapace di abbassare la soglia. Il job di copertura
installa percio' GDAL, e `scripts/check_coverage_exclusions.py` verifica che
**ogni** misuratore porti `--all-features` — la verifica era globale, e con tre
misuratori bastavano gli altri due a tenerla verde — e che **ogni** ancora
compaia nel report. Un'ancora e' la prima funzione dentro un blocco `cfg` che
nomina positivamente una feature, derivata dal sorgente invece che scritta a
mano; i blocchi che non cominciano con una funzione, e quelli dentro un modulo
`cfg(test)`, restano fuori e il gate li **dichiara** invece di far finta di
guardarli.

Il checkpoint riporta **due proiezioni** dello stesso profdata — i record `DA:` del report LCOV e la colonna
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
| `assurance/registries/passi-del-checkpoint.json` | i passi del checkpoint, per identità | `s9-checkpoint.sh` (worktree), `check_release_contract.py` (`git show` dalla revisione misurata) |
| `assurance/registries/sonde-deterministiche.json` | i rami che dipendevano dallo scheduling, e la sonda che li esercita | `check_sonde_deterministiche.py` |
| `assurance/campagne-copertura.json` | il verbale della dimostrazione di riproducibilità della copertura | nessuno: è un fatto passato, non una fonte |
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

L'insieme dei passi è **dichiarato in un registro**, letto da entrambi i lati:
il checkpoint confronta a fine corsa ciò che ha eseguito, e il verificatore
confronta ciò che l'evidenza descrive. Chiuderlo da un lato solo lascerebbe
passare una rimozione coordinata dall'altro — togliere un gate, la sua voce, il
suo log e aggiornare contatori e digest è un'evidenza coerente con sé stessa che
descrive un checkpoint più debole. Il confronto è sull'**insieme e sull'ordine**, non sul
totale: un passo tolto e uno aggiunto lasciano il conto fermo, e due passi
scambiati lasciano l'insieme identico. L'ordine è canonico perché è un vincolo
reale — `sonde_checkpoint` per primo, la copertura dopo il fuzzing che ne
produce il profdata — e la diagnostica nomina la prima posizione divergente.

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
