# Prodotto — che cosa offre e che cosa promette

<!-- generato da codice/versione: inizio -->

Versione del prodotto: **10.0.0**.

| Artefatto Python | Nome |
| --- | --- |
| wheel | plenora_io-10.0.0-py3-none-any.whl |
| sdist | plenora_io-10.0.0.tar.gz |

<!-- generato da codice/versione: fine -->

Questo documento descrive la superficie che un consumatore vede: driver,
opzioni, contratti pubblici e limiti. Come è costruita e come è verificata sta
in [ENGINEERING.md](ENGINEERING.md); dove siamo nel percorso di rilascio sta in
[RELEASE.md](RELEASE.md).

---

## I dieci driver

<!-- generato da codice/formati: inizio -->

| Id | Estensioni | Direzione | Feature | Lettura nativa | Lettura effettiva | Pruning spaziale / predicati | Fedeltà | Tipi scrivibili |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| csv | ["csv"] | bidirectional | assente | streaming_sequential | streaming_sequential | none / none | conditional | ["boolean","signed_integer","unsigned_integer","floating","utf8","temporal"] |
| dxf | ["dxf"] | bidirectional | assente | materialized | streaming_sequential | none / none | approximating | ["boolean","signed_integer","unsigned_integer","floating","utf8","binary","temporal","decimal","nested","other"] |
| filegdb | ["gdb"] | bidirectional | gdal-backend | streaming_sequential | materializing | none / none | conditional | ["signed_integer","floating","utf8"] |
| geojson | ["geojson","json"] | bidirectional | assente | streaming_sequential | streaming_sequential | none / none | conditional | ["boolean","signed_integer","unsigned_integer","floating","utf8","temporal"] |
| geoparquet | ["parquet"] | bidirectional | assente | streaming_random | streaming_columnar | bounding_box_statistics / numeric_min_max_statistics | lossless | ["boolean","signed_integer","unsigned_integer","floating","utf8","binary","temporal","decimal","nested","other"] |
| gpkg | ["gpkg"] | bidirectional | assente | streaming_random | streaming_sequential | optional_rtree_index / none | conditional | ["boolean","signed_integer","unsigned_integer","floating","utf8","binary"] |
| ipc | ["arrow","arrows"] | bidirectional | assente | streaming_random | streaming_sequential | none / none | lossless | ["boolean","signed_integer","unsigned_integer","floating","utf8","binary","temporal","decimal","nested","other"] |
| kml | ["kml"] | bidirectional | assente | materialized | streaming_sequential | none / none | conditional | ["boolean","signed_integer","unsigned_integer","floating","utf8","temporal"] |
| shp | ["shp","shp.d"] | bidirectional | assente | streaming_sequential | streaming_sequential | none / none | conditional | ["boolean","signed_integer","unsigned_integer","floating","utf8","temporal"] |
| xls | ["xlsx"] | bidirectional | assente | materialized | streaming_sequential | none / none | conditional | ["boolean","signed_integer","unsigned_integer","floating","utf8","temporal"] |

<!-- generato da codice/formati: fine -->

**Consegna e buffering sono uguali per tutti**: `operation_atomic` con buffering
adattivo — memoria finché il budget lo consente, poi spool su disco. Un errore a
metà lettura non consegna righe parziali.

Una scrittura rifiutata non lascia una destinazione, con **una** eccezione
dichiarata: il set di file sciolti dello Shapefile, che si ottiene solo
accettandolo esplicitamente e il cui contratto è più sotto, in
[§ Publish](#publish-che-cosa-diventa-visibile-e-quando).

**Fin dove arriva il supporto di una specifica**: il catalogo lo dichiara nel
campo `spec_version_supported` di ciascun driver. `geoparquet` dichiara
`1.1.0`: legge per intero i metadati `geo` delle versioni 1.0.0 e 1.1.0 — i due
valori che gli schemi ufficiali fissano — e scrive 1.1.0. Una versione oltre
viene rifiutata come **funzionalità non supportata**, che è cosa diversa da
metadati non conformi: il primo errore dice che il file va bene e noi no, il
secondo che il file è sbagliato. Con lo stesso criterio sono rifiutate le
codifiche native GeoArrow e i bordi sferici, entrambi validi in GeoParquet 1.1
e non implementati qui. Gli altri driver dichiarano `null`: i loro formati non
si versionano in un modo che il driver possa dichiarare per intero.

### CSV: presenza, testo e tipi

La convenzione di IO distingue il **campo vuoto non quotato** (null) da `""`
(stringa vuota). Il writer quota sempre gli attributi testuali, raddoppia le
virgolette interne e termina i record con CRLF. Spazi, separatori e newline
interni sono conservati. Una riga vuota rappresenta un record con un solo
null, non viene saltata. Il quoting malformato e UTF-8 invalido sono errori.
Il separatore deve essere ASCII, diverso da virgolette, CR e LF.

In lettura un attributo quotato forza la colonna a Utf8, anche se contiene
`"true"`, `"001"` o `"NaN"`. Nessun trim si applica agli attributi. Fra i valori
non quotati, l'inferenza accetta Int64 o Float64 soltanto quando parse e
formattazione canonica Rust (`Display`) restituiscono **gli stessi byte**;
la promozione di interi a Float64 esclude gli interi oltre la precisione esatta.
Tutte le righe partecipano: un solo valore incompatibile rende testuale tutta
la colonna. `001`, `1e3`, `1.0`, ` 12 `, `+1`, `TRUE` restano testo;
`true`/`false` non quotati possono diventare Boolean. Una colonna solo null
si legge Utf8 nullable. Le colonne geometriche dichiarate sono interpretate
come WKT o X/Y: un vuoto quotato o soli spazi non è una geometria null valida.

| Tipo Arrow attributo in scrittura | Esito |
|---|---|
| Utf8 | valore e validità esatti, compresi vuoto, spazi e testo numerico |
| LargeUtf8 | testo esatto; perdita della larghezza del tipo dichiarata |
| Boolean; Int8/16/32/64; UInt8/16/32/64 | cifre e valori esatti nel file; schema originale non conservato, `coercion tipo attributo` nel report |
| Float32/64 finiti | rappresentazione decimale del valore, incluso il segno di zero; tipo originale e possibile rilettura testuale dichiarati con `coercion tipo attributo` |
| Float32/64 NaN (anche payload diversi), +inf, -inf | rifiuto senza pubblicazione, nessun payload NaN appiattito |
| Float16, Null, Utf8View | rifiuto del piano anche senza righe |
| Date32/64; Time32 secondi/millisecondi, Time64 micro/nanosecondi; Timestamp con/senza fuso, tutte le unità | testo ISO quotato, tipo/unità/fuso originali non conservati: perdita pianificata `coercion tipo attributo`; timestamp con fuso espresso in UTC con `Z` |
| Date64 non multiplo di un giorno; orari negativi o fuori dal giorno | rifiuto senza pubblicazione, nessun troncamento |
| Time32 micro/nanosecondi, Time64 secondi/millisecondi; Duration; Interval | rifiuto del piano |
| Decimal32/64/128/256 (precisione e scala) | rifiuto del piano |
| Binary, LargeBinary, FixedSizeBinary, BinaryView | rifiuto come attributi; la geometria Binary dichiarata resta WKB→WKT/X/Y |
| List, LargeList, FixedSizeList, ListView, LargeListView, Struct, Map, Union, Dictionary, RunEndEncoded | rifiuto del piano |

Il CSV non porta uno schema Arrow: gli scalari diversi da Utf8 dichiarano
prudenzialmente la normalizzazione anche quando un particolare file si rilegge
con lo stesso tipo. Non confondere testo esatto con ricostruzione dello schema.
Le prove permanenti sono in `crates/driver-csv/src/tests/presenza.rs`.

La sintassi segue [RFC 4180](https://www.rfc-editor.org/rfc/rfc4180), che conserva
gli spazi ma **non definisce null né tipi**: la distinzione tramite quoting è
una convenzione di IO, non una garanzia universale. [GDAL CSV](https://gdal.org/en/stable/drivers/vector/csv.html)
offre `QUOTED_FIELDS_AS_STRING=YES` e `EMPTY_STRING_AS_NULL`, da impostare e
verificare sul proprio percorso. [pandas read_csv](https://pandas.pydata.org/docs/reference/api/pandas.read_csv.html)
riconosce valori mancanti per default e non preserva questa distinzione tramite
quoting: `dtype=str` da solo non basta; `keep_default_na=False` conserva il testo
ma non ricostruisce la validità originale. [Excel](https://support.microsoft.com/en-us/excel/keeping-leading-zeros-and-large-numbers)
richiede importazione come testo per conservare zeri iniziali e numeri lunghi.
Non affidare a un'apertura e successivo salvataggio in Excel la conservazione di null, stringhe vuote e tipi: usare uno
schema e un formato tipizzato (Arrow IPC) per lo scambio esatto.
Il sentinel `NA` usato da R viene letto da IO come stringa `"NA"`, anche
quando in R rappresentava un valore mancante: e' un limite di interoperabilita',
non un null riconosciuto automaticamente. Un sentinel null
non è stato scelto perché collide con testo legittimo e richiede escaping privato.

### Determinismo

I driver garantiscono **determinismo semantico**, non byte per byte: a parità di
input, opzioni e versione dell'implementazione, stessi valori e stesso insieme
di righe, **senza** assumere un ordine o una rappresentazione fisica identici.

È la garanzia minima della scala dichiarata dal modello — `semantic`, `ordered`,
`byte_for_byte`, `unordered` — e i dieci driver dichiarano `semantic` sia in
lettura sia in scrittura.

L'unica superficie con determinismo **byte per byte** è la busta di
`plenora-io catalog`, che serializzata due volte produce gli stessi byte. È una
proprietà di quel comando, non dei driver, e vale la pena non confonderle:
riconvertire due volte lo stesso file può produrre due output diversi byte per
byte e ugualmente corretti.

`gpkg` e `filegdb` sono multi-layer.

### Publish: che cosa diventa visibile, e quando

Il publish ha tre forme. **Due sono crash-atomic e una no**, e la differenza non
è un dettaglio di implementazione: decide che cosa un altro processo può vedere
sul disco se il nostro muore a metà.

| forma | chi la usa | garanzia |
|---|---|---|
| file singolo | `csv`, `geojson`, `kml`, `xls`, `dxf`, `gpkg`, `geoparquet`, `ipc` | un rename: il file c'è per intero o non c'è |
| directory-dataset | `filegdb`, e `shp` con destinazione `*.shp.d` | un rename della directory: tutti i file insieme, o nessuno |
| set di file sciolti | `shp` con destinazione `*.shp` | **non atomica**: fino a cinque rename in sequenza, con rollback best-effort |

La forma raccomandata in produzione per lo Shapefile è la **directory-dataset**,
e si ottiene chiedendo una destinazione che finisce in `*.shp.d`. Dentro ci sono
`data.shp`, `data.shx`, `data.dbf`, `data.cpg` e `data.prj`, e la directory diventa visibile
con un rename solo.

#### Il set sciolto va accettato, non subìto

La forma compatibile — `dati.shp` accanto a `dati.shx`, `dati.dbf`, `dati.prj`, `dati.cpg` —
**non ha quella garanzia e non può averla**: cinque file separati non si rendono
visibili in un atto solo su nessuno dei filesystem supportati. Per questo non si
ottiene più deducendola dall'estensione: una destinazione `*.shp` è **rifiutata**
finché non la si accetta con l'opzione di formato
`publish_mode=loose_shapefile_set`. Il rifiuto è `InvalidConfiguration` — la
richiesta è incompleta, non il prodotto incapace.

Che cosa quell'accettazione comporta, per intero:

| | |
|---|---|
| **ordine** | i companion prima, il `.shp` per ultimo: chi cerca il file marker non lo trova finché il resto non c'è |
| **errore durante il publish** | i companion già spostati vengono riportati nello staging, best-effort |
| **rollback riuscito** | l'errore porta `remote_effect: none`: nessuna destinazione è rimasta visibile |
| **rollback fallito** | l'errore porta `remote_effect: partial` e `retry: requires_recovery`: sul disco **possono** esserci companion senza il `.shp` |
| **processo ucciso a metà** | nessun rollback avviene: lo stesso stato parziale, e nessun errore che lo dichiari |

L'ultima riga è la ragione per cui questa forma non è raccomandata: le prime
quattro descrivono ciò che il codice fa, la quinta ciò che nessun codice in
spazio utente può fare.

#### Recovery di un set sciolto interrotto

Vale per `remote_effect: partial` e per un processo ucciso durante il publish. È
la stessa procedura, perché è lo stesso stato: un insieme incompleto di file
accanto alla destinazione.

1. **Non ripetere la scrittura senza guardare.** Il publish è no-clobber su ogni
   file del set: un companion sopravvissuto fa fallire il tentativo successivo
   con `OutputExists`, e quel fallimento è corretto — non è il segno che si può
   riprovare.
2. **Riconoscere lo stato.** Un set completo ha `.shp`, `.shx`, `.dbf`, `.cpg` e, se il
   layer porta un CRS, `.prj`. Un set senza `.shp` è incompleto per costruzione:
   il marker è l'ultimo a essere pubblicato.
3. **Rimuovere i companion rimasti**, cioè i file con lo stesso stem della
   destinazione che il publish avrebbe scritto. Nessuno di essi è un dato
   valido: appartengono a una scrittura che non si è conclusa.
4. **Ripetere la conversione**, preferibilmente verso `*.shp.d`, dove questa
   procedura non serve.

Lo staging viene rimosso al rientro ordinato, anche in errore. Un arresto
brusco può lasciare sia file pubblicati parzialmente sia una directory temporanea
adiacente: dopo aver accertato che nessuna scrittura sia attiva, isolare i residui
attribuibili al tentativo fallito e ripetere la conversione. Non cancellare
accessori di provenienza incerta: scegliere una destinazione libera.

Il no-clobber riguarda l'intero insieme dichiarato dal driver, inclusi accessori
che IO non scrive ma altri programmi rileggono. Il principale e i suffissi SQLite seguono le regole reali del filesystem.
Su filesystem sensibili alle maiuscole, un altro dataset con grafia diversa
non blocca la destinazione e un `-WAL` non equivale al `-wal` aperto da SQLite.
Un membro effettivamente presente produce `OUTPUT_EXISTS`, senza cancellarlo. Per SHP comprende `.qix`, `.sbn`,
`.sbx`, `.shp.xml`, `.prj` e `.cpg`; per CSV `.prj` e `.csvt`; per GPKG
`-wal`, `-shm` e `-journal`. `VEC.SHP` produce `VEC.SHP`, `VEC.SHX`,
`VEC.DBF`, `VEC.PRJ`, `VEC.CPG`: ogni lettera dell'estensione conserva la
forma richiesta (anche `ShP` → `ShX`). Il nome del layer di `nome.shp.d` è
`nome`; i file interni mantengono il prefisso `data`.

In lettura la corrispondenza esatta vince. La ricerca alternativa degli
accessori per estensione ignora le maiuscole dell'estensione, mantenendo il
prefisso esatto del dataset su filesystem sensibili; più candidati senza
corrispondenza esatta sono un errore di ambiguità. I suffissi SQLite seguono
sempre le regole reali del filesystem. Quota, footprint,
identità e rifiuto dei symlink comprendono tutti i membri presenti, anche quelli
estranei noti. SHP richiede `.shp` e `.dbf`; `.shx` è facoltativo, ma se presente
deve indicizzare tutti i record in ordine, con lunghezze coerenti. Il conteggio
SHP/DBF viene verificato anche senza SHX. Un `.prj` vuoto va ripristinato oppure
rimosso fornendo `--assume-crs`; un `.cpg` vuoto va sostituito con una codepage
supportata oppure rimosso per usare LDID. L'assenza non equivale a un file vuoto.
I nomi CPG non distinguono maiuscole e spazi ASCII: `ANSI 1250`–`ANSI 1258`,
`CP1250`–`CP1258` e `WINDOWS-1250`–`WINDOWS-1258` indicano le rispettive
Windows codepage già supportate; restano UTF-8, ASCII e ISO-8859-1 con gli
alias numerici. `System` e `ANSI` senza numero non identificano una codifica
portabile e sono rifiutati: indicare la codepage originale nel `.cpg`, oppure
rimuoverlo soltanto quando LDID identifica correttamente quella codifica.


FileGDB comprende l'intero albero, inclusi indici e lock. Gli staging adiacenti
`<stem>.plenora-tmp-*.gdb` e i corrispondenti `.lock`, anche maiuscoli, impediscono
una nuova scrittura con `OUTPUT_EXISTS`. Dopo un crash, verificare che il
processo proprietario sia terminato, conservare o isolare il relativo staging e
lock e ripetere verso una destinazione libera. Non rimuovere lock di processi
attivi. Le directory continuano a essere pubblicate con un solo rename.

### Che cosa significano le classi di fedeltà

| | |
|---|---|
| **lossless** | il formato rappresenta il contratto senza perdita |
| **condizionale** | la perdita dipende dal contratto, non dal formato: il CSV conserva i valori testuali; la perdita dello schema Arrow degli altri scalari è dichiarata |
| **approssimante** | l’esito osservato può approssimare anche un formato condizionale quando avviene una coercizione. DXF tassella archi ed ellissi, esplode le geometrie multipart, rappresenta il testo come punto |

Ciò che si perde è dichiarato nel report di perdita, non taciuto, e il
**contratto** di quel report è ratificato: vedi
[LossReport](#lossreport--ratificato-con-il-protocollo-2).

### Modalità di lettura nativa

La modalità nativa è ciò che il formato consente; la modalità effettiva è
indicata da `read_mode` nel catalogo. L’atomicità operativa è un asse distinto.

* **sequenziale** — il file si legge in avanti una volta sola;
* **ad accesso casuale** — il formato indicizza e consente di saltare;
* **materializzata** — il parser ha bisogno di **tutto l'input** prima di
  consegnare la prima riga. Non vuol dire «tutto in RAM»: dove stia quell'input
  lo descrive il buffering, che è un asse separato e adattivo — RAM sotto una
  soglia, poi spool su file. La coppia dice esattamente che cosa succede: serve
  tutto l'input, non serve tutta la memoria, e confondere i due assi è il
  difetto che il modello tiene distinto per costruzione. Per questi driver i
  limiti di input sono l'unica difesa contro un file ostile, e sono applicati
  **prima** che il parser veda il contenuto.

## Opzioni di formato

Le opzioni sono dichiarate in uno **schema**: chiave, fase (`read`, `write`,
`both`), tipo del valore e default. Un valore fuori schema è rifiutato prima di
raggiungere il driver, con categoria `invalid_configuration`, codice
`CLI_USAGE` e il vocabolario ammesso. Chiavi e valori ricevuti sono redatti.

`plenora-io catalog` emette lo schema completo. Le opzioni con un default
diverso da «assente»:

<!-- generato da codice/opzioni: inizio -->

| Driver | Chiave | Fase | Dominio | Default | Significato |
| --- | --- | --- | --- | --- | --- |
| csv | delimiter | both | char | , | separatore di campo ASCII, esclusi virgolette, CR e LF |
| csv | geometry_encoding | write | {"enum":["wkt","xy"]} | wkt | come scrivere la geometria: colonna WKT o colonne x/y |
| csv | wkt_column | read | text | assente | colonna che contiene la geometria in WKT |
| csv | x_column | read | text | assente | colonna dell'ascissa, da usare insieme a y_column |
| csv | y_column | read | text | assente | colonna dell'ordinata, da usare insieme a x_column |
| geoparquet | compression | write | {"enum":["brotli","gzip","lz4","none","snappy","uncompressed","zstd"]} | snappy | codec di compressione delle pagine |
| ipc | serialization | write | {"enum":["file","stream"]} | file | serializzazione IPC: contenitore `file` con footer, o flusso `stream` |
| shp | publish_mode | write | {"enum":["shapefile_directory_dataset","loose_shapefile_set"]} | assente | forma di pubblicazione; .shp.d si deduce, .shp va accettata |
| shp | row_diagnostics.examples_limit | read | {"integer":{"max":64,"min":1}} | 64 | numero massimo di righe di esempio per diagnostica |
| shp | row_diagnostics.key_field | read | text | assente | campo DBF usato come chiave nelle diagnostiche di riga |
| shp | row_diagnostics.key_policy | read | {"enum":["emit","redact"]} | assente | chiave sempre redatta; emit alias di redact; richiede row_diagnostics.key_field |
| xls | geometry_encoding | write | {"enum":["wkt","xy"]} | wkt | come scrivere la geometria: colonna WKT o colonne x/y |
| xls | sheet | read | text | assente | nome esatto del foglio da leggere; in assenza valgono le regole comuni di selezione |
| xls | wkt_column | read | text | assente | colonna che contiene la geometria in WKT |
| xls | x_column | read | text | assente | colonna dell'ascissa, da usare insieme a y_column |
| xls | y_column | read | text | assente | colonna dell'ordinata, da usare insieme a x_column |

<!-- generato da codice/opzioni: fine -->

I driver senza opzioni dichiarate non ne accettano: passarne una è un errore di
configurazione, non un valore ignorato.

### CRS

| | |
|---|---|
| **incorporato** | il formato porta il CRS, e viene letto da lì |
| **WGS84 fisso** | il formato lo impone; un CRS diverso è rifiutato |
| **nessuno** | il formato non lo porta. Se il contratto ha una geometria, `--assume-crs` è **obbligatorio**: senza, la lettura fallisce in fase CRS invece di indovinare |

Nessun driver riproietta. Il CRS viene letto, scritto o dichiarato — mai
trasformato.

CSV e XLSX senza `wkt_column`, `x_column` e `y_column` si leggono come tabelle
senza geometria e senza `--assume-crs`. La presenza di una sola coordinata
configurata resta un errore; per leggere una geometria occorrono la colonna WKT
oppure entrambe le colonne X/Y, e il CRS esplicito. Anche la scrittura XLSX
accetta tabelle senza geometria. Una stringa vuota viene scritta come cella
stringa presente, distinta dalla cella assente che rappresenta null.

La tabella `loss_table` del core governa le classi di attributi e le dimensioni
del catalogo, la normalizzazione dei tipi in scrittura e il trasporto dei
metadati Arrow. `arrow_metadata_not_preserved` conta soltanto le chiavi sorgenti
che differiscono dallo schema effettivamente riletto, con conteggi e indici
strutturali. La ricostruzione identica non costituisce una perdita;
i valori e i nomi delle chiavi opache non entrano nella diagnostica. GPKG
mantiene le categorie specifiche del proprio piano. Il confronto copre anche tipi, nomi, ordine e nullabilità dei campi consegnati.
La verifica dello schema non sostituisce la qualifica dei singoli membri nativi
ignorati dai parser.

Una geometria con `shp.measure_no_data` nei metadati nativi è rifiutata con
`unsupported` verso destinazioni diverse da SHP, IPC e GeoParquet: il controllo
è conservativo sul contratto, anche se un batch non contiene la sentinella.
La misura mancante non viene convertita in una misura reale. KML scrive Z con
`altitudeMode=absolute` e rifiuta Point vuoti, LineString senza coordinate e
aggregati geometrici vuoti. Per FileGDB, OGC:CRS84 non conservato come
identificatore è dichiarato `crs_id_not_preserved_absent`.

#### `--assume-crs` e il CRS che la sorgente dichiara

L'opzione **nomina** un CRS che la sorgente non dichiara. Quando la sorgente ne
dichiara uno, i due si confrontano per ogni colonna geometrica, in un punto
solo che ogni driver attraversa all'apertura (`with_read_budget` in
`plenora-io-core`, regola in `plenora_io_model::crs::verifica_crs_assunto`):

| la sorgente dichiara | `--assume-crs` |
|---|---|
| un identificatore uguale (GEO-005: autorità senza maiuscole, codice come testo) | ammesso; il contratto conserva la grafia della sorgente |
| un identificatore diverso | rifiutato con `crs` |
| un CRS senza identificatore (una definizione senza autorità) | rifiutato con `crs`: non si può confrontare |
| niente, o un CRS dichiarato ignoto (Arrow IPC senza CRS, GeoParquet con `crs: null`) | rifiutato con `crs`: l'opzione resterebbe senza effetto. Il messaggio dice che la geometria resta senza CRS «perché la sorgente lo dichiara ignoto o il formato non usa l'opzione»: fino alla rilettura 10 diceva «il formato non usa l'opzione» anche per GeoParquet, che la usa per un Parquet senza metadato `geo` (prova `un_crs_ignoto_si_riscrive_in_geoparquet_come_null`) |

Fino alla 5.0.1 GeoPackage, GeoParquet, GeoJSON, KML e IPC ignoravano
l'opzione, e DXF e FileGDB la ignoravano quando il CRS incorporato dichiarava
il proprio identificatore: chi chiedeva `EPSG:3003` riceveva il CRS del file
senza che la busta lo dicesse. Il confronto non è un resolver: `OGC:CRS84` ed
`EPSG:4326` sono identificatori diversi, e su un GeoJSON l'opzione ammessa è
`OGC:CRS84` (prove `un_assume_crs_che_contraddice_il_crs_dichiarato_e_rifiutato`
in `plenora-io-tools`, `verifica_crs_assunto_rifiuta_cio_che_non_puo_confermare`
in `plenora-io-model`).

Un layer senza geometrie rifiuta `--assume-crs` come configurazione senza effetto
(`invalid_configuration/validate/CLI_USAGE`). Il controllo EPSG richiede un
codice intero positivo canonico nei 32 bit con segno: testo, segno `+`, zeri
iniziali e zero sono rifiutati come configurazione. Non si consulta un registro:
un codice sintatticamente valido, anche `EPSG:999999`, resta una dichiarazione
del chiamante, senza attestazione di esistenza e con genere `unknown`.

#### `--assume-crs` dichiara, non risolve

`--assume-crs EPSG:3003` dice **quale** CRS hanno i dati. Non è un resolver
EPSG→WKT: il prodotto non porta un database di definizioni e non ne inventa
una. Per i formati che scrivono una definizione — lo Shapefile nel `.prj`, il
DXF nel `GEODATA` — l'identificatore da solo basta soltanto per WGS84, l'unico
per cui il writer sintetizza il WKT.

Fuori da quel caso i due formati si comportano in modo diverso, ed entrambi i
comportamenti sono voluti:

| | |
|---|---|
| **DXF** | **rifiuta**: senza definizione non scrive un file monco |
| **Shapefile** | **scrive** i dati senza `.prj`, e lo **dichiara** nel `LossReport` |

Ciò che lo Shapefile dichiara è `crs_id_not_preserved_absent` — e
`srid_not_preserved_absent` se la sorgente portava un SRID. Non
`..._derived`: *derivata* significa che il valore si rilegge dal file
attraverso un'altra rappresentazione, e senza `.prj` non c'è niente da cui
rileggerlo. La distinzione conta per chi automatizza: `absent` dice che il CRS
va riportato **fuori** dal file, `derived` direbbe che basta rileggerlo.

**GeoParquet è il caso più stretto**: la specifica vuole un documento PROJJSON,
e il driver non ne sintetizza uno da un identificatore. Una scrittura il cui
CRS è noto solo come `EPSG:3003` viene quindi **rifiutata**, non degradata — e
lo è anche quando la sorgente porta un WKT, che GeoParquet non ha un modo
conforme di scrivere. Un PROJJSON si scrive **byte per byte** e si rilegge con
gli stessi byte: GeoParquet **conserva** la definizione PROJJSON, e dell'SRID —
che il metadato `geo` non ha dove mettere — dichiara la sola derivazione
dall'identificatore.

L'identificatore, in GeoParquet, vive **solo** dentro il documento (`id`), e il
lettore lo ricava da lì. Per questo il writer scrive un PROJJSON soltanto se il
file lo rilegge come il contratto lo dichiara, e rifiuta con `unsupported`,
prima di ogni effetto, tre casi che fino alla rilettura 11 si scrivevano come
`lossless`: un documento senza `id` (il lettore lo rifiuta come CRS non
risolto), un `id` diverso dall'identificatore del contratto anche solo nella
grafia (`crs_representations_inconsistent`: il file direbbe quello del
documento), un `id` che contraddice il genere del documento (`EPSG:4326` su un
`EngineeringCRS`, regola «tutti: genere del CRS»). Un contratto senza
identificatore con un documento che ne porta uno si scrive: il file ne dice
uno in più, non uno diverso. Un documento con spazi ai bordi si rifiuta: si
incorpora come valore JSON, e gli spazi non sarebbero conservati.

**`EPSG:4326` senza PROJJSON si rilegge `OGC:CRS84`.** Senza documento il
writer omette il campo `crs`, che per la specifica vale `OGC:CRS84`. Le
coordinate non cambiano — GeoParquet memorizza longitudine prima qualunque sia
il CRS, e l'ordine `lon_lat` dichiarato resta vero — ma l'identificatore sì:
per GEO-005 `EPSG:4326` e `OGC:CRS84` sono identificatori diversi. Dalla
rilettura 11 la perdita si dichiara (`crs_id_not_preserved_absent`: va
riportato da fuori, il file non lo restituisce); fino a db1307d un IPC con il
solo `EPSG:4326` convertito in GeoParquet era `lossless`, e un GeoPackage in
`EPSG:4326` dichiarava la sola definizione. Si è scelto di
dichiarare invece di scrivere un PROJJSON di `EPSG:4326` preso da una
costante: senza PROJ non c'è un documento canonico da cui copiarlo (le
versioni di PROJ emettono `datum` o `datum_ensemble` con membri diversi), e un
documento scritto a mano sarebbe una definizione che nessuno ci ha dato, con
il rischio di dirla sbagliata. Dichiarare non dice mai il falso; il file che
esce è quello di prima, letto da GDAL e da ogni lettore GeoParquet come WGS 84
`lon_lat`. Rientro: un resolver CRS, o un PROJJSON canonico di `EPSG:4326`
fissato da plenora-contracts.

#### Che cosa ogni bersaglio fa della definizione che riceve

Un Arrow conforme porta la definizione in una qualunque delle tre grammatiche
(WKT, WKT 2, PROJJSON), e GEO-005 confronta solo gli identificatori. Dalla
rilettura 10 `crs_definition` vale **per grammatica**
(`CrsRepresentationCapabilities::verbatim_solo` in `plenora-io-core`), e il
verdetto per piano la legge: una definizione che il bersaglio non scrive tale e
quale è una perdita dichiarata o un rifiuto, mai una conversione `lossless`.
Dalla rilettura 11 la restrizione vale anche per un bersaglio che la
definizione la riscrive (il GeoPackage), e il catalogo la pubblica come
`crs_definition: "conditional"` (sezione «Il vocabolario CRS del catalogo»).

| bersaglio | conserva | un'altra grammatica |
|---|---|---|
| IPC | tutte | — |
| GeoParquet | PROJJSON, byte per byte | con `OGC:CRS84` il campo `crs` si omette (il file si rilegge `OGC:CRS84`, senza definizione) e la perdita è `crs_definition_not_preserved_absent`; con `EPSG:4326` lo stesso, più `crs_id_not_preserved_absent` (rilettura 11); con un altro identificatore, o senza, `unsupported`. Fino a db1307d il documento passava da `serde_json::Value`: le chiavi uscivano riordinate e i numeri riscritti come `f64` (`0.99960000000000000000000001` diventava `0.9996`, `9E0` diventava `9.0`), con la definizione dichiarata conservata; anche il lettore consegnava la riscrittura invece del testo del file |
| Shapefile (`.prj`), DXF (`GEODATA`) | WKT, WKT 2, byte per byte (anche gli spazi ai bordi e un CRLF finale: il DXF codifica gli a capo come `^M^J`, e fino alla rilettura 12 il lettore del `GEODATA` rifilava il testo, così un `.prj` chiuso da CRLF usciva dal DXF diverso con la definizione dichiarata conservata) | un PROJJSON con `OGC:CRS84` o `EPSG:4326` si sostituisce con il WKT che il writer sintetizza, e la perdita è `crs_definition_not_preserved_derived`; con ogni altro identificatore `unsupported` («il `.prj` porta un WKT»). Convertire PROJJSON in WKT servirebbe un resolver: un GeoParquet in EPSG:3003 non si converte in Shapefile né in DXF |
| GeoPackage | WKT 1, riscritto | la colonna `definition` di `gpkg_spatial_ref_sys` porta un WKT 1 (GPKG 1.3, tabella 21), e il writer ci scrive solo quello: la definizione si riscrive dalla riga (`crs_definition_not_preserved_derived`). Un PROJJSON o un WKT 2 non ci vanno: per `EPSG:4326` la riga, che il writer registra sempre, porta il WKT canonico (`derived`), per ogni altro identificatore la colonna vale `undefined`, come la specifica ammette, e la definizione non si rilegge (`crs_definition_not_preserved_absent`). Fino a db1307d un PROJJSON finiva nella colonna WKT, dichiarato derivato |
| FileGDB | — | la definizione si riscrive da GDAL: `crs_definition_not_preserved_derived`, come prima |
| GeoJSON, KML, CSV, XLSX | — | `crs_definition_not_preserved_absent`, come prima |

La riga di `EPSG:4326` che ogni GeoPackage scritto da IO porta ha, dalla
rilettura 11, il testo della tabella 21 di GPKG 1.3 — un WKT 1 con
l'autorità di radice e **senza** `AXIS`, non la forma del registro EPSG
(`WKT_WGS84_GPKG` in `driver-gpkg`) —, non più `GEOGCS["WGS 84"]`: quel WKT la
regola dei lettori non lo risolve, e un GeoPackage in WGS 84 convertito in
Shapefile o DXF si rileggeva non risolto. Il GeoPackage rifiuta con
`unsupported`, non più `crs`, ciò che `gpkg_spatial_ref_sys` non sa
rappresentare: un CRS senza identificatore, un codice che non è un intero
canonico (`EPSG:04326`) o un'autorita' con codice testuale non rappresentabile.
L'eccezione esplicita e' `OGC:CRS84`: il GeoJSON RFC 7946 si scrive nella riga
WGS84 `EPSG:4326`, mantenendo x=longitudine e y=latitudine, senza riproiezione.
La diversa rappresentazione dell'identificatore viene dichiarata nella perdita.
Gli altri rifiuti sono limiti del bersaglio su un CRS risolto (REJ-001 `support`).
Il DXF, per la stessa ragione, rifiuta con `unsupported` un CRS noto
solo per un identificatore di cui non sa scrivere il WKT (prima `crs`).

**L'identificatore che Shapefile e DXF scrivono è quello che il WKT dice.**
I due formati non scrivono l'identificatore: scrivono il WKT, e i lettori lo
ricavano con la regola di `identificatore_wkt`. Dalla rilettura 11 il verdetto
per piano rilegge il WKT che il writer emette con quella regola
(`CrsRepresentationCapabilities::identificatore_riletto`): se ne esce lo stesso
identificatore (GEO-005) la perdita è `crs_id_not_preserved_derived`, altrimenti
`crs_id_not_preserved_absent` — un `PROJCS` anonimo con `EPSG:32632` si rilegge
non risolto, e `OGC:CRS84` si rilegge dal WKT sintetizzato `EPSG:4326`; in
entrambi i casi l'identificatore va riportato con `--assume-crs`. Un CRS
risolto **senza** identificatore il cui WKT si rileggerebbe non risolto, o un
WKT che i lettori non rileggono affatto, si rifiuta con `unsupported`
(`crs_unresolved`): non c'è un identificatore con cui dichiararne la perdita.
Fino a db1307d le due cose erano `crs_id_not_preserved_derived` e una
conversione `conditional` senza ragioni CRS, e il file si rileggeva «dichiarato
ma non risolto». È il rientro della deviazione «Shapefile, DXF: identificatore
dal WKT» della rilettura 10.

Un `missing` si scrive in GeoParquet come `crs: null` (capability
`embedded_or_unknown`): era la promessa della riga «GeoParquet: `crs: null`»,
e fino a b7a6c94 la validazione la rifiutava («il formato richiede un CRS
risolto»). Il dichiarato ma non risolto resta un rifiuto: GeoParquet non ha un
modo di dirlo.

**Il bersaglio non sceglie l'identificatore contro la definizione.** Un
identificatore che il bordo sa orizzontale su una radice che non lo è
(`EPSG:4326` su un `LOCAL_CS` o su un PROJJSON `EngineeringCRS`) o di genere
opposto (`EPSG:4326` su un `PROJCS` anonimo, `EPSG:3857` su un geografico) è
la contraddizione che i lettori rifiutano (`crs_kind_for`, riga «tutti: genere
del CRS»). Un Arrow conforme la può portare, e IPC la conserva intera e la
rilegge com'è. Ogni altro bersaglio che porta un CRS la rifiuta con
`unsupported` e la ragione `crs_representations_inconsistent`, prima di ogni
effetto; restano fuori solo FileGDB, che consegna la coppia a GDAL, e CSV e
XLSX, che l'identificatore non lo scrivono. È un limite del bersaglio su un
ingresso conforme (GEO-000 punto 2, REJ-001 «support»), non un difetto
dell'ingresso: per questo non è `crs`. Fino a b7a6c94 GeoParquet scriveva
`EPSG:4326` per un `LOCAL_CS` dichiarandosi `lossless`; fino a db1307d la
regola saltava ogni bersaglio che conserva la definizione, e GeoParquet
scriveva `EPSG:4326` dentro un `EngineeringCRS` (un file che non si rileggeva,
`lossless`), Shapefile e DXF un `LOCAL_CS` per `EPSG:4326` (un file che si
rileggeva non risolto, con l'identificatore dichiarato derivato).

#### Il file si rilegge come la busta lo dichiara

È l'invariante della classe, e una prova permanente lo verifica su ogni
sorgente verso ogni bersaglio senza GDAL (`rilettura_dei_crs.rs` in
`plenora-io-tools`), in due passaggi (tre, e con le sorgenti native, dalla
rilettura 12: sotto): da venti sorgenti IPC (WKT, WKT ESRI,
WKT 2, PROJJSON con numeri oltre `f64` ed escape, solo identificatore, senza
identificatore, coppie contraddette, `missing`) e poi dai file GeoParquet,
GeoPackage, Shapefile, DXF e GeoJSON che il primo passaggio ha scritto. Per
ogni scrittura riuscita: senza `crs_id_not_preserved_*` il file si rilegge con
lo stesso identificatore, byte per byte; con `..._derived` con uno uguale per
GEO-005; senza `crs_definition_not_preserved_*` con la stessa definizione,
byte per byte (una definizione che la sorgente non aveva può comparire); un
file che non si rilegge senza opzioni ha dichiarato `crs_id_not_preserved_absent`
e `--assume-crs` lo riporta, o — senza identificatore — ha dichiarato perduta
la definizione; un `missing` si rilegge `missing`. CSV e XLSX non portano un
CRS, e per loro la regola è che ogni perdita sia dichiarata. Su db1307d la
prova fallisce con trenta violazioni al primo passaggio e cinquantacinque al
secondo, che ricadono nei sette rilievi della rilettura 11.

Dalla rilettura 12 la prova confronta anche lo stato della risoluzione (una
sorgente non risolta si rilegge nello stesso stato, una risolta si rilegge
risolta), la grammatica (`definition_format`: la stessa definizione non si
rilegge con un'altra etichetta), l'ordine degli assi (uguale, oppure `unknown`
quando la perdita della definizione che lo portava è dichiarata — mai un altro
ordine — con la sola eccezione dichiarata di `other` verso i formati con l'est
per primo, sotto «Ordine degli assi») e l'SRID (dove un file lo porta in chiaro,
Arrow IPC); pretende che un GeoParquet scritto non porti un `geo` in
`ARROW:schema` e che un Arrow IPC scritto da un'altra sorgente non porti un
`geo` di schema. Parte anche da sorgenti native, scritte senza i writer di IO:
GeoPackage alla GDAL (WKT con `AXIS`, `organization` minuscola, definizione
chiusa da CRLF, `undefined`), Shapefile con il `.prj` ESRI, chiuso da CRLF o
preceduto da spazi, GeoParquet alla geopandas (`geo` anche nello schema
incorporato, PROJJSON con `", "`, autorità minuscola, `crs` omesso o `null`),
oltre ai casi IPC con identificatori minuscoli, definizioni con spazi ai bordi
o CRLF, `other`, SRID, un `geo` di schema stantio e una sorgente
`declared_unresolved`; e ha un terzo passaggio sulle catene GPKG→SHP,
GeoParquet→GPKG, SHP→GPKG, GPKG→DXF, DXF→GeoParquet e SHP→DXF. Su 777c5a1
fallisce con diciannove violazioni al primo passaggio, cinquantuno al secondo e
trentaquattro al terzo, in tre classi:

* **il `geo` della sorgente nello schema scritto.** Il writer GeoParquet
  copiava i metadati dello schema del contratto in `ARROW:schema`, accanto al
  `geo` del footer scritto dal contratto: da un IPC con un `geo` di schema che
  dichiara `EPSG:4326` e un campo `EPSG:32632` usciva un file `lossless` il cui
  footer dice 32632 e che pyarrow (`read_schema`, `read_table`) legge 4326. Lo
  stesso da GeoParquet a GeoParquet, e da GeoParquet a IPC il `geo` restava nei
  metadati dello schema. Ora il lettore GeoParquet consuma `geo` nel contratto
  e non lo lascia nello schema, e il writer non copia nello schema le chiavi
  che appartengono al formato (`geo`, `ARROW:schema`) né il CRS GeoArrow
  (`ARROW:extension:metadata`) della colonna geometria. Prima del retag le
  dichiarazioni CRS di ogni campo si confrontano fra loro e col footer:
  una contraddizione viene rifiutata. Le
  altre chiavi di schema passano (ARROW-010). Lo schema incorporato porta lo
  **stesso** `geo` del footer, byte per byte: pyarrow (`read_schema`,
  `read_table`), quando il footer porta `ARROW:schema`, rende i soli metadati
  dello schema incorporato, e geopandas (`read_parquet`) cerca `geo` lì. Fino
  alla rilettura 12 un GeoParquet scritto da IO da una sorgente senza `geo`
  (IPC, GeoPackage, Shapefile, DXF, GeoJSON) non lo portava affatto nello schema
  incorporato — per l'ecosistema pyarrow non era un GeoParquet («Missing geo
  metadata») —, e da un GeoParquet portava quello della sorgente. Il `geo` si
  decide alla chiusura (tipi visti, covering) e `ArrowWriter` fissa lo schema
  all'apertura: il writer salta lo schema incorporato della libreria
  (`with_skip_arrow_metadata`) e lo codifica alla chiusura con la stessa
  funzione (`add_encoded_arrow_schema_to_metadata`), sullo stesso schema più
  `geo`, così i tipi Arrow incorporati (large, timezone, dictionary, decimal,
  nullabilità, metadati di campo) restano quelli di prima;
* **il DXF che rifilava la definizione** (sopra, tabella delle grammatiche);
* **il GeoPackage che scrive l'autorità in maiuscolo.** `organization` si
  scrive in maiuscolo, e `epsg:4326` si rilegge `EPSG:4326`: uguale per
  GEO-005, ma non byte per byte, e la conversione lo dichiarava conservato. Ora
  un identificatore con l'autorità minuscola verso GeoPackage dichiara
  `crs_id_not_preserved_derived` (la capability `autorita_in_maiuscolo` di
  `CrsRepresentationCapabilities`, che non va sul filo).

Prove: `rilettura_dei_crs.rs` (regola 9: il `geo` di `ARROW:schema` è quello
del footer; su c5abbdb fallisce in tutti e tre i passaggi, 140 violazioni);
`le_chiavi_del_formato_della_sorgente_non_entrano_nello_schema_incorporato`,
`il_geo_letto_non_resta_nei_metadati_del_contratto` (in `driver-geoparquet`).

#### L'epoca delle coordinate e i `geo` discordi (rilettura 13)

Un CRS dinamico (ITRF2014, un `DynamicGeodeticReferenceFrame`) dà alle
coordinate un significato solo insieme all'**epoca** a cui valgono: GeoParquet
1.1 la porta nel campo `epoch` della colonna, un numero (anno decimale). Fino a
b988e7f il lettore la validava e la metteva nel contratto
(`geoparquet.epoch`), il writer GeoParquet non la scriveva, e nessun bersaglio
ne dichiarava la perdita: GeoParquet→GeoParquet, IPC→GeoParquet e verso
GeoPackage l'epoca spariva in conversioni `lossless` (o con le sole perdite
dell'identificatore). Arrow Vocabulary 1.0 e Arrow Geometry Semantics 1.0 non
hanno una chiave per l'epoca (GEO-019 rende «indecidibile» il frame dinamico,
che passa tale e quale): il contratto la porta nella chiave nativa
`plenora.geometry.native.geoparquet.epoch`, e non se ne inventa una nel
vocabolario. Ora:

| bersaglio | l'epoca |
|---|---|
| GeoParquet | si scrive nel campo `epoch` della colonna **byte per byte**, con il testo che il file sorgente portava (`2021.50000000000000001`, `2.0215E3` restano tali: da `f64` uscivano riscritti, come i numeri del PROJJSON fino alla rilettura 11); un testo che non è un numero JSON, o con spazi ai bordi, è `schema` all'apertura del writer, mai omesso |
| Arrow IPC | passa nella chiave nativa del campo; un consumatore generico che ignora le chiavi native ignora anche l'epoca (limite dichiarato: il vocabolario comune non la nomina) |
| GeoPackage, Shapefile, DXF, GeoJSON, KML, CSV, XLSX, FileGDB | non hanno dove portarla: la perdita `crs_coordinate_epoch_not_preserved` è dichiarata in `write_loss`, e il livello non è più `lossless` |

L'elenco dei formati che la portano è chiuso, nel codice
(`perdita_pianificata::PORTANO_L_EPOCA`): un driver nuovo dichiara la perdita
finché non entra nell'elenco. La chiave nativa dell'epoca **cambia valore**: era
il `f64` riscritto (`2021.5` per `2.0215E3`), ora è il testo del file. Lo stesso
vale per `geoparquet.column`, il documento della colonna: passava da
`serde_json::Value`, con le chiavi riordinate e i numeri (anche quelli del
PROJJSON e l'epoca) riscritti da `f64`, e ora è il testo del file.

Le chiavi native `geoparquet.*` (`column`, `version`, `edges`, `orientation`,
`bbox`, `epoch`) descrivono il `geo` della sorgente, e il `geo` del file scritto
le sostituisce: il writer GeoParquet non le copia più nello schema incorporato
(fino a b988e7f pyarrow vi leggeva un'epoca o una colonna che il footer non
dichiarava), e il lettore GeoParquet non le prende dallo schema incorporato di
un file (un'epoca nativa lì, senza `epoch` nel `geo`, entrava nel contratto e
da IPC sarebbe tornata nel `geo` di un GeoParquet). `edges` resta `planar`
(lo sferico si rifiuta in lettura), cioè il default. `bbox` e `orientation`
sono affermazioni sui valori: il writer non le verifica sui batch ricevuti,
che possono essere stati filtrati o modificati. Perciò non le copia e dichiara
rispettivamente `geometry_bbox_not_preserved` e
`geometry_orientation_not_preserved` in `planned_write_loss`, una volta per
colonna che le porta, con posizione di layer e campo. La regola vale per la
primaria e per ogni secondaria, anche verso gli altri formati; Arrow IPC
conserva entrambe nelle chiavi native e non dichiara questa perdita.
Il `covering` della primaria viene invece ricalcolato per ogni riga;
`creator` (al livello del documento) descrive chi ha scritto il file e non
si eredita. L'epoca conserva il significato temporale delle coordinate.

Le identità `plenora.field_id` dichiarate nello schema Arrow incorporato
si conservano per primaria e secondarie tramite il parser comune; gli indici
fisici restano distinti. CRS e tipi continuano a provenire dal `geo` del footer.
La fixture `multigeometria-metadati-pyarrow.parquet`, generata con PyArrow e
`store_schema=True`, prova identità diverse dagli indici, bbox per entrambe
le colonne e orientamento antiorario di una secondaria Polygon.

**`geo` del footer contro `geo` dello schema incorporato.** Il lettore legge il
`geo` del footer (in `arrow-rs` le chiavi del footer vincono); pyarrow, e
geopandas con lui, rende quello dello schema incorporato (`ARROW:schema`). Un
file che li porta diversi — come quelli che IO scriveva fino a 777c5a1, con il
`geo` stantio della sorgente — si legge in due modi, e il file non dice quale
valga: fino a b988e7f il lettore usava il footer in silenzio. Ora si rifiuta,
come due `GEODATA` DXF discordi: `crs` quando i documenti discordano nel `crs`
della colonna primaria (la classe CRS di REJ-001: il file dichiara due CRS),
`schema` per ogni altra differenza, per un `geo` presente solo nello schema
incorporato (per pyarrow il file è un GeoParquet, per il footer no) e per una
chiave `geo` ripetuta con valori diversi (la classe del vocabolario: due
dichiarazioni dello stesso documento). Il confronto è byte per byte; documenti
identici, o un `geo` solo nel footer, passano. È un restringimento degli
ingressi accettati.

Prove: `rilettura_dei_crs.rs` (regola 10: l'epoca si rilegge uguale, byte per
byte, o la perdita è dichiarata, e nessun file ne porta una che la sorgente non
aveva; sorgenti `p_itrf_epoca`, `p_crs84_epoca`, `p_4326_epoca`,
`4326_pj_epoca`; terza catena GeoParquet→IPC; su b988e7f fallisce in tutti e
tre i passaggi); `l_epoca_si_scrive_byte_per_byte_e_si_rilegge_uguale`,
`un_epoca_che_non_e_un_numero_si_rifiuta`,
`le_chiavi_native_geoparquet_non_passano_nello_schema_scritto`,
`l_epoca_nativa_dello_schema_incorporato_non_entra_nel_contratto`,
`i_geo_di_footer_e_schema_incorporato_devono_concordare`,
`un_geoparquet_con_il_geo_stantio_nello_schema_si_rifiuta` (in
`driver-geoparquet`); `l_epoca_delle_coordinate_perduta_si_dichiara` (in
`plenora-io-core`).

La regola 1 della prova si è stretta: una perdita `crs_id_not_preserved_*`
dichiarata la spegneva per prefisso, e un file che si rileggeva con un
**altro** identificatore passava. Ora il file si rilegge senza identificatore,
con uno uguale per GEO-005, o con uno dell'elenco chiuso `EQUIVALENTI`:
`EPSG:4326` e `OGC:CRS84`, nei due versi, quando il riletto dichiara
`longitude_latitude` (le coordinate memorizzate sono le stesse). Le forme ESRI
(`GCS_WGS_1984`) non entrano: sono definizioni, non identificatori, e si
rileggono `EPSG:4326` o senza identificatore. Sulla matrice attuale nessuna
scrittura viola la regola stretta.

**Limiti residui alla chiusura delle riletture (rilettura 14, nessun difetto silenzioso).** Le seconde letture del ciclo si sono chiuse quando una lettura indipendente non ha trovato più risultati sbagliati senza errore; quel che resta è dichiarato qui:

- **`geo` confrontati come testo.** Due `geo` (footer e `ARROW:schema`) che differiscono solo per spazi o ordine delle chiavi si rifiutano: un rifiuto in più, mai una lettura diversa. Se gli spazi cadono dentro il `crs`, la categoria è `crs` invece di `schema`. Rientro: un confronto semantico del documento `geo`.
- **`geo` solo nel footer.** IO lo legge, come prescrive la specifica; pyarrow, che con `ARROW:schema` presente rende i metadati dello schema incorporato, non lo vede e il file per lui non è GeoParquet: un errore di geopandas, non una lettura diversa. IO scrive il `geo` in entrambi i posti.
- **Chiavi native diverse da `geoparquet.*` nello schema incorporato** di un GeoParquet esterno (per esempio `plenora.geometry.native.gpkg.srs_id`) entrano nel contratto. Nessun writer di IO ne consuma oggi; un writer che ne consumasse una dovrà prima applicare loro lo stesso trattamento di `geoparquet.epoch`.
- **IPC → IPC con un'epoca nativa non numerica** passa tale e quale (ARROW-010): la rifiuta la prima scrittura GeoParquet, con `schema`.
- **`orientation` e `bbox` della colonna** non si ereditano da chiavi native in GeoParquet; la perdita viene dichiarata per ogni colonna (vedi sopra).
- **IPC → IPC con un `geo` di schema stantio** e un'epoca nativa diversa conserva entrambi (ARROW-010): un lettore che legge il `geo` di schema dell'IPC vede l'epoca stantia. È il limite del `geo` di schema IPC già dichiarato, esteso all'epoca.

#### Il vocabolario CRS del catalogo

`io.catalog` pubblica per ogni writer due capability CRS, enumerate dalla
rilettura 11 nello schema `plenora-io-catalog-v2` (prima i valori erano liberi
nello schema e scritti solo nel codice).

`write_capabilities.crs` dice che cosa il writer fa del CRS:

| valore | significato | formati |
|---|---|---|
| `embedded` | incorpora il CRS, e pretende un CRS risolto | GeoPackage, Shapefile, DXF, FileGDB |
| `embedded_optional` | incorpora anche il dichiarato ma non risolto e il mancante, senza perdita | IPC |
| `embedded_or_unknown` | incorpora un CRS risolto e scrive il mancante come CRS ignoto (`crs: null`); il dichiarato ma non risolto si rifiuta | GeoParquet |
| `{"fixed": id}` | impone quel CRS; un altro si rifiuta (`reprojection_required`) | GeoJSON, KML (`OGC:CRS84`) |
| `none` | non porta un CRS | CSV, XLSX |

`write_capabilities.crs_representations` dice, per ciascuna delle tre
rappresentazioni del contratto (`crs_id`, `srid`, `crs_definition`), che cosa
ne esce; è la capability statica, e il verdetto per piano sta nel `LossReport`
(`..._not_preserved_derived`, `..._not_preserved_absent`):

| valore | significato |
|---|---|
| `preserved` | si scrive e si rilegge tale e quale |
| `derived` | non si scrive, e si rilegge da un'altra rappresentazione scritta nel file; per piano può decadere ad `absent` quando quella manca o non restituisce lo stesso valore |
| `absent` | non si scrive, e va riportata da fuori |
| `conditional` | solo `crs_definition`: l'esito dipende dalla grammatica della definizione. IPC conserva tutte le grammatiche (`preserved`); GeoParquet conserva il PROJJSON e perde le altre; Shapefile e DXF conservano WKT e WKT 2 e sostituiscono un PROJJSON con il WKT sintetizzato per `EPSG:4326` e `OGC:CRS84` (altrimenti rifiutano); il GeoPackage riscrive un WKT 1 dalla propria riga e perde le altre grammatiche, tranne per `EPSG:4326` |

Fino alla rilettura 11 GeoParquet, Shapefile e DXF pubblicavano
`crs_definition: "preserved"` e il GeoPackage `"derived"`, mentre l'esito
dipendeva dalla grammatica; il valore che serviva non esisteva, ed è stato
aggiunto allo schema di IO (`conditional`) e alle note di rilascio.

Prove (in `plenora-io-tools`, `definizione_crs_in_scrittura.rs`; su db1307d
falliscono le sette della rilettura 11 e `geoparquet_dichiara_il_wkt_che_non_scrive`):
`geoparquet_dichiara_il_wkt_che_non_scrive`,
`geoparquet_rifiuta_un_wkt_senza_identificatore`,
`l_identificatore_non_si_sceglie_contro_la_definizione`,
`shp_e_dxf_rifiutano_un_projjson_che_non_sanno_scrivere`,
`shp_e_dxf_sostituiscono_il_projjson_e_lo_dichiarano`,
`un_crs_ignoto_si_riscrive_in_geoparquet_come_null`,
`geoparquet_dichiara_l_identificatore_che_diventa_crs84`,
`geoparquet_conserva_il_projjson_byte_per_byte`,
`la_coppia_contraddetta_si_rifiuta_anche_dove_la_definizione_si_conserva`,
`gpkg_non_scrive_un_projjson_nella_colonna_wkt`,
`un_crs_che_il_file_rilegge_non_risolto_non_si_scrive_in_silenzio`,
`gpkg_porta_wgs84_intero_e_rifiuta_crs84_come_limite`,
`il_catalogo_dice_condizionata_la_definizione_che_dipende_dalla_grammatica`;
`rilettura_dei_crs.rs`; `le_definizioni_che_non_sono_projjson_non_spariscono_in_silenzio`,
`un_projjson_che_il_file_non_rilegge_uguale_non_si_scrive`,
`il_projjson_si_scrive_e_si_rilegge_byte_per_byte`,
`il_lettore_consegna_il_testo_del_documento` (in `driver-geoparquet`);
`la_riga_di_wgs84_porta_il_wkt_canonico` (in `driver-gpkg`);
`l_identificatore_dal_wkt_e_derivato_solo_se_si_rilegge_uguale`,
`senza_definizione_l_identificatore_si_rilegge_come_il_default` (in
`plenora-io-core`); `geoparquet_proiettato_a_dxf_rifiuta_il_projjson`
(matrice delle conversioni).

#### Ordine degli assi

`plenora.geometry.axis_order` dichiara l'ordine delle prime due ordinate
**come memorizzate** nei valori WKB (Arrow Geometry Semantics 1.0, GEO-001),
non l'ordine con cui il registro elenca gli assi del CRS: `EPSG:4326` letto da
un GeoPackage è `lon_lat`, anche se EPSG registra la latitudine per prima.
Fino alla 5.x il prodotto dichiarava l'ordine del registro (`lat_lon` per ogni
`EPSG:4326`) mentre i valori portavano la longitudine prima.

| in lettura | ordine dichiarato |
|---|---|
| GeoPackage, GeoParquet, Shapefile, FileGDB, DXF | il formato memorizza easting o longitudine prima: `lon_lat` per un geografico, `easting_northing` per un proiettato i cui assi sono est/nord, `other` se la definizione dice assi che non lo sono (una polare come EPSG:3031), `unknown` se il genere è ignoto, se la definizione non dice gli assi o se un proiettato non ha definizione (GeoPackage con `definition = 'undefined'`) |
| GeoJSON, KML | `lon_lat` (RFC 7946, KML 2.2) |
| CSV, XLSX con `x_column`/`y_column` | la richiesta nomina la `x`, e la `x` è l'est: `lon_lat` con `--assume-crs` geografico |
| CSV, XLSX con `wkt_column` | `unknown`: il WKT non fissa l'ordine e la richiesta non lo dichiara |
| Arrow IPC | quello che il campo dichiara; con la sola chiave `crs`, `unknown` |

In scrittura, un formato che fissa easting o longitudine prima — tutti tranne
Arrow IPC, compresi CSV e XLSX — **rifiuta** un campo `lat_lon` o
`northing_easting` con `unsupported`, ragione `axis_order_not_representable`,
prima di creare la destinazione: scritto com'è uscirebbe invertito, e il bordo
non ha una conversione delle ordinate qualificata. Arrow IPC porta la
dichiarazione con i valori e li accetta tutti.

Limiti dichiarati (regola 4):

* **`other` e `unknown` passano verso i formati con l'est per primo.** Il
  bordo non sa provare che contraddicano il formato, e i valori si scrivono
  come sono. Garanzia indebolita: la rilettura dichiara l'ordine del formato,
  né `other` né `unknown`. Hazard: un campo `unknown` o `other` che
  memorizzava la latitudine (o il nord) prima esce dalla rilettura come
  `lon_lat` (o `easting_northing`), cioè con un ordine che i valori non hanno.
  Rientro: rifiutare `other` e `unknown` in scrittura, quando una versione del
  contratto lo decide.
* **Proiezioni polari in WKT 1 senza `AXIS`.** Il default OGC 01-009 è est poi
  nord; il prodotto dichiara `other` solo quando il nome della proiezione dice
  polare (`Polar_Stereographic`, `..._Pole`) o la latitudine d'origine è ±90.
  Hazard: una polare che non lo dichiara esce `easting_northing`. Rientro: un
  resolver CRS al bordo.
* **CSV e XLSX rifiutano `lat_lon` anche in WKT**, dove il formato non fissa
  l'ordine: il prodotto non distingue la codifica in validazione, e la
  rilettura con `x_column`/`y_column` dichiarerebbe `lon_lat`.
* **`io.catalog` non pubblica gli ordini ammessi**: il protocollo v2 è
  ratificato, e la chiave nuova è una decisione di protocollo. Il limite si
  legge dal rifiuto.

---

## Contratti pubblici

### Consumo Rust

Il gate del consumatore esterno verifica la costruzione dall'archivio sorgente
senza un lockfile iniziale nel progetto consumatore. Calamine e rust_xlsxwriter
usano ora ZIP 9; non serve copiare il lockfile distribuito per riutilizzare
versioni ZIP ritirate.

### Compatibilità

La superficie con garanzia di compatibilità è **il JSON della CLI**. L'API Rust
è interna e instabile: non porta garanzia semver e i crate non sono pubblicati.

### Le buste

<!-- generato da codice/buste: inizio -->

| Comando | Contratto | Protocollo |
| --- | --- | --- |
| catalog | plenora-io-catalog-v2 | 2 |
| inspect | plenora-io-inspect-v1 | 2 |
| layers | plenora-io-layers-v1 | 2 |
| read | plenora-io-read-result-v2 | 2 |
| write | plenora-io-write-result-v2 | 2 |
| convert | plenora-io-convert-v1 | 2 |
| capabilities | plenora-capabilities-v2 | 2 |
| --version | plenora-io-version-v2 | 2 |
| errori | plenora-error-v1 | 2 |

<!-- generato da codice/buste: fine -->

Il contratto identifica la struttura dell’operazione, mentre `protocol_version`
identifica la busta comune. I due numeri di versione sono indipendenti.
Il blocco è ricavato dai nomi emessi dal codice, inclusi versione ed errori.

### `plenora-error-v1`

L'oggetto `error` porta **sei chiavi obbligatorie**; `details` è la settima quando sono
presenti dettagli strutturati (per esempio la diagnostica di riga):

```json
{
  "status": "error",
  "protocol_version": 2,
  "contract": "plenora-error-v1",
  "error": {
    "category": "...",
    "phase": "...",
    "remote_effect": "...",
    "retry": { "kind": "..." },
    "code": "...",
    "message": "...",
    "details": {}
  }
}
```

`error.details.row_diagnostics` è presente quando si osservano rifiuti per riga.
`details` può contenere anche `driver`, `field` e `capability_reason`.

I dettagli strutturati conservano il contesto senza inserirlo nel messaggio.

### Il quartetto è la chiave di compatibilità

```
(category, phase, code, retry)
```

Un consumatore deve decidere su questi quattro. **`message` non è una chiave di
compatibilità**: il suo testo può cambiare senza preavviso, ed è cambiato.

Il quartetto di ogni sito di costruzione è fissato da uno snapshot e verificato
a ogni checkpoint.

### `PublicMessage` — il testo è scelto a compile time

Nessun costruttore pubblico di errore accetta testo libero. Il messaggio si
costruisce da varianti che accettano solo `&'static str` e numeri strutturali:

| Variante | Contenuto |
|---|---|
| `Curated` | un testo nostro |
| `CuratedPair` | due testi nostri |
| `CuratedWith` | un testo e un numero strutturale (indice, conteggio, limite) |
| `CuratedBetween` | due testi e due numeri |
| `Capability` | una ragione tipizzata |
| `OpzioneRifiutata` | condizione ed elenco statico degli ammessi; il token ricevuto non viene emesso |
| `CuratedList` | elenco di termini statici derivato dal catalogo, mai valori del chiamante |

Il valore **decodificato** di `message` non supera **2048 byte UTF-8**. Il tetto
non è promesso sul JSON serializzato, dove l'escaping espande: una virgoletta
diventa due byte, un carattere di controllo sei.

#### Che cosa questa garanzia non è

`&'static str` garantisce la **durata**, non la **provenienza**. Un chiamante
deliberato può promuovere testo runtime a `'static` con `Box::leak` e infilarlo
in un messaggio curato senza che il compilatore obietti.

La promessa realistica è quindi:

> impedire la propagazione **accidentale** di testo runtime nel workspace, non
> rendere crittograficamente inconiabile un messaggio dinamico da codice ostile.

I crate sono interni e `publish = false`: l'avversario di questo invariante è la
distrazione, non un aggressore. La documentazione del tipo porta un esempio
eseguibile che dimostra il limite invece di ammetterlo a parole.

### Perdite osservate e isolamento dei membri

Gli attributi DXF ricostruiti identici (`layer`, `dxf_type`, `text` nullo)
non sono perdite. Il confronto comune prima della pubblicazione rilegge il
file completato e confronta gli attributi originali conservati nello spool
soggetto alle quote; conta le celle diverse, mancanti e le righe aggiunte.
Le ragioni `attribute_loss` derivano dalle differenze osservate, non dalla
sola capability del formato. Il confronto dei metadati resta unico per i
dieci driver, incluso IPC che confronta lo schema effettivamente serializzato.

In FileGDB il catalogo resta un prerequisito globale. La struttura dei membri
dei singoli oggetti viene invece verificata localmente: `.gdbtable` e `.gdbtablx`
assenti o vuoti sono `object_missing`/`object_empty`; membri troncati o malformati riportano
l'errore strutturale nell'oggetto enumerato. I fratelli sani restano selezionabili.
Nessun recupero implicito GDAL sostituisce questa dichiarazione.

I nomi fisici FileGDB hanno una sola identità ASCII indipendente dal caso:
prefisso `a`, otto cifre esadecimali e suffisso (anche gli indici `nome.atx`).
Il parser comune associa tutti i suffissi al relativo oggetto; i controlli di
header restano quelli previsti per ciascun tipo. `gdb` e `timestamps` usano
la stessa normalizzazione. Alias della stessa identità sono ambigui.
Questa è una regola di interoperabilità di IO-TOOLS, non una promessa generale
del filesystem o di Esri. [GDAL 3.9.3, OpenFileGDBDataSource::Open](https://github.com/OSGeo/gdal/blob/v3.9.3/ogr/ogrsf_frmts/openfilegdb/ogropenfilegdbdatasource.cpp#L180-L290)
riconosce il prefisso diretto minuscolo e costruisce percorsi canonici minuscoli.
Per le varianti IO-TOOLS prepara una copia temporanea canonica, conservata
anche dai lettori attivi; non rinomina l'ingresso. La scrittura nativa e la
materializzazione usano la stessa identità. La copia richiede spazio temporaneo
proporzionale al dataset, già soggetto alla quota di ingresso.

Le regole dipendono dal formato: Shapefile risolve le estensioni dei companion
con il resolver comune; GPKG conserva i suffissi SQLite esatti `-wal`, `-shm`
e `-journal` sul filesystem ospite. DXF e KML sono file singoli: l'estensione
riconosce il formato senza cambiare l'identità del percorso richiesto.

### Diagnostiche: riservatezza dei testi e dei valori

I testi degli errori e delle diagnostiche non contengono dati del chiamante:
nessun valore, nome di campo, nome di layer o percorso. Le cause di riga
appartengono al vocabolario chiuso `DiagnosticCause`; il deserializzatore
rifiuta codici arbitrari. Anche conteggi e limiti di conoscenza sono validati
contro vocabolari chiusi prima della serializzazione.

I nomi di campo e layer sono schema dichiarato dal chiamante: possono comparire
soltanto in campi strutturati dedicati e tipizzati (`field`, `layer`, `column`),
mai nei testi. `column` usa `DiagnosticFieldName`, separato da `DiagnosticCause`.
I valori non compaiono mai, neppure nelle chiavi diagnostiche: `key.value` non
ha valori costruibili e lo stato emesso resta `redacted`. Per compatibilita
sintattica `row_diagnostics.key_policy=emit` viene accettato come alias di
`redact`; non autorizza l'esposizione del valore.

### `ContractIdentifier` ed `ErrorContext`

Un nome di campo o di layer non entra nel testo dell'errore: entra in un campo
**tipizzato**. `ContractIdentifier` è costruibile solo da un contratto validato —
non esiste una conversione da `String` — e i nomi non attestabili sono
**rifiutati**, non troncati.

`ErrorContext` porta driver, campo e ragione di capability accanto all'errore,
per il chiamante Rust; sul wire corrente il contesto strutturato entra in `error.details`.

### Budget e limiti

Un'operazione riceve un `PipelineBudget` che porta i limiti e un **permit di
input**, consumato per `move`: la stessa sorgente non può essere osservata due
volte.

<!-- generato da codice/quote: inizio -->

| Flag / campo | Default | Unità | Ambito | Fasi |
| --- | --- | --- | --- | --- |
| --max-input-bytes | 268435456 | byte | totale sorgente e spool dei decoder | ["read"] |
| --max-input-entries | 10000 | entry | totale sorgente | ["read"] |
| --max-rows | 10000000 | righe | totale operazione | ["read","write"] |
| --max-columns | 4096 | colonne | schema layer e totale del piano di scrittura | ["validate","prepare","read","write"] |
| --max-geometry-components | 16777216 | componenti | totale operazione | ["read","write"] |
| --max-output-bytes | 1073741824 | byte | totale operazione | ["read","write","finalize","commit"] |
| --output-expansion-ratio | 1073741824 | moltiplicatore | uscita / ingresso osservato non vuoto | ["read","write","finalize","commit"] |
| --max-wkb-cell-bytes | 67108864 | byte | per geometria | ["read","write"] |
| --max-wkb-components | 100000 | componenti | per geometria | ["read","write"] |
| --max-wkb-depth | 32 | livelli | per geometria | ["read","write"] |
| --max-vertices | 50000000 | componenti (coordinate e membri) | per geometria; alias storico che stringe max_wkb_components | ["read","write"] |
| --memory-bytes | 536870912 | byte | memoria viva pipeline e pool | ["prepare","read","write","finalize"] |
| --spill-bytes | 4294967296 | byte | spill vivo pipeline e pool | ["read","write","finalize"] |
| --decompression-ratio | 1000 | moltiplicatore | totale archivio compresso | ["read"] |
| concurrent_operations | 64 | operazioni | pool condiviso | ["prepare","read","write"] |
| --deadline-ms | 30000 | ms | operazione; scadenza distinta dalle quote | timeout |

<!-- generato da codice/quote: fine -->

I limiti si applicano **prima** dell'allocazione che dovrebbero impedire. Un
tetto verificato dopo aver materializzato la cella non è un tetto.

Le quote configurabili sono enumerate da `plenora_io_model::quota::QUOTAS`.
Zero, negativi, testo e incoerenze sono `invalid_configuration/validate/CLI_USAGE`;
per memoria inferiore a 64 MiB occorre abbassare anche `max_wkb_cell_bytes`.
Il superamento è `ResourceLimit` e nomina la quota della tabella nella fase
effettiva. Alla misura dello staging (`finalize`) riporta anche byte e limite.
La deadline conserva la categoria `timeout`.

### Compatibilità

| | |
|---|---|
| **vincolante** | il contratto e la `protocol_version` di ciascuna busta; l'insieme delle chiavi di `plenora-error-v1`; il quartetto degli errori |
| **non vincolante** | il testo di `message`; l'ordine delle chiavi; l'API Rust |
| **cambio breaking** | rimuovere o rinominare una chiave, cambiare un valore del quartetto per un sito esistente, cambiare la semantica di un codice |

Un cambio breaking richiede una nuova versione di contratto, non una nota.

---

## Arrow Primary Geometry 1.0 (8.0.0)

Tutte le superfici Arrow adottano PRIM-001..009: la chiave di schema
`plenora.primary_geometry` viene risolta prima del completamento degli ID e
normalizzata secondo PRIM-004. IPC trasporta piu' geometrie anche senza primaria.
GeoParquet mappa la primaria nei due versi con `primary_column` e rifiuta
l'assenza di primaria. GEO-012 resta il limite dei sink singoli GPKG, Shapefile,
CSV, XLSX, FileGDB, GeoJSON, DXF e KML. Una singola geometria con `none` resta
geometria ai fini di validazione, quote e perdite. I sink singoli rifiutano
`none` perche' non possono conservarne lo stato. Vedere `../CHANGELOG.md`
per i nuovi rifiuti e la migrazione dalla 7.x. La matrice interop e le campagne
8.0.0 richiedono esecuzione sul nuovo congelamento.

## Limitazioni di prodotto

| | |
|---|---|
| GeoPackage: sorgente stabile | **Regola:** file consolidato, nessuno scrittore concorrente sull'insieme principale/companion. **Ambito:** letture immutable; controllo dei companion sul nome aperto prima dell'apertura e al termine del layer. **Hazard:** un companion creato e rimosso durante la lettura puo sfuggire; modifiche concorrenti possono dare dati incoerenti. **Rientro:** lock sull'intero insieme o snapshot coerente verificata. Su Windows nomi non estesi invariati da Win32 e canonicalizzati allo stesso file; nomi riservati, punti/spazi finali e almeno 260 unita UTF-16 sono rifiutati. Hazard: file legittimi lunghi non leggibili. Rientro: qualificare il supporto SQLite ai percorsi lunghi mantenendo l'identita del nome controllato/aperto. |
| DXF | approssima per costruzione: archi ed ellissi tassellati, multipart esplose, testo come punto. La perdita è dichiarata, non silenziosa |
| XLSX | interi scrivibili nell'intervallo inclusivo [-2^53, 2^53]; oltre, rifiuto prima della pubblicazione. Float32 ampliato esattamente a Float64; float non finiti e decimali rifiutati. In lettura date native: Date32 per formato solo data e millisecondi multipli di 86_400_000, Timestamp(Microsecond, senza fuso) con ora, Duration(Microsecond) per stili durata. Epoca 1900/1904 dal workbook; giorni 1-59 del 1900 esatti, giorni 0 e 60 ambigui/inesistenti rifiutati. Seriali negativi, fuori dominio o non canonici al millisecondo rifiutati: candidato ms arrotondato razionalmente e accettato solo se ms/86_400_000 arrotondato a binary64 (round-half-even) riproduce gli stessi bit. Timestamp e Duration Arrow conservano ms*1000 microsecondi. Colonne temporali miste ed errori Excel rifiutati; solo il vuoto diventa null. Date/durate ISO restano testo. Le date native diventavano silenziosamente null nelle versioni 1.0.0-6.0.0 elencate in RELEASE.md; ora si leggono |
| KML, XLSX, DXF | lettura materializzata: la libreria sottostante ha bisogno di **tutto l'input** prima della prima riga. Dove stia quell'input lo decide il buffering, che è un asse separato: non è una promessa che stia tutto in RAM. I limiti di input sono l'unica difesa, e vengono applicati prima del parser |
| DXF | un `DATATABLE` oltre 2^20 righe, colonne o celle, e un'anteprima (`THUMBNAILIMAGE`) la cui decodifica supererebbe 16 MiB, rendono il documento illeggibile (`FORMAT_ERROR`). Sono tetti espliciti del prodotto contro le prenotazioni dimensionate dall'input, **non** una garanzia di leggere ogni file AutoCAD. L'ordine delle celle del `DATATABLE` (per colonne, un valore per ogni riga) segue la specifica Autodesk come la scrive il fork ed è verificato solo in round-trip interno, non contro file AutoCAD reali: il driver non legge `DATATABLE`, ma il lettore li attraversa. In scrittura il fork accetta solo `DATATABLE` completi (un nome per colonna, un valore per ogni riga di ogni colonna, un tipo per colonna) e rifiuta gli altri con un errore; il driver scrive entità e `GEODATA`, non `DATATABLE` |
| DXF | le coordinate si scrivono in ASCII con dodici decimali; `native_coordinate_precision` dichiara arrotondamento, possibile collasso di vertici distinti e normalizzazione di -0 |
| GeoParquet | dalla 4.2.0 il decoder del footer (`parquet` 60 più il fork `vendor/parquet`) lega ai byte residui ogni elenco Thrift e i row group, rifiuta uno schema più profondo di 64 livelli prima di convertirlo, e addebita ogni prenotazione -- elenchi, column chunk, percorsi delle foglie, il footer stesso -- al tetto delle pagine (metà della capacità effettiva) **prima** di farla; un footer più lungo di metà del tetto si rifiuta dalla coda. Il caso della 4.1.1 (3966 byte, `malloc(2315255472)`) è un errore. Restano fuori dal tetto le espansioni vere dei dati (un dizionario ripetuto su molte righe), che governa il budget delle pagine, e il page index, che il driver non legge. Anche i decoder delle pagine non si fidano piu' dei valori del file: lunghezze, indici di dizionario, prefissi e suffissi delta, corse RLE e livelli v2 o `BIT_PACKED` oltre i byte della pagina sono un errore invece di un panico, e i quattro casi che prima si leggevano in silenzio con un valore sbagliato (un prefisso delta oltre il valore precedente, il resto di una pagina `BYTE_STREAM_SPLIT` o FLBA `PLAIN`, una corsa RLE oltre `u32`) si rifiutano. Il delta e' lo stesso del fork di plenora-data-tools2 (`vendor/parquet`, registro `vendor-parquet-fork.json`) |
| Runtime `io.read`: vettore di richiesta | Deviazione, regola 4, da RUNTIME-VECTORS-1.0 §5 (`io-read-request`), limitata al payload illustrativo: `artifact_source`, `format` e `layer` stringa non rispettano `plenora-io-read-input-v1`, che richiede `source` e `layer` intero. Il binding rifiuta con `RUNTIME_PAYLOAD_INVALID` prima del risolutore. Conseguenza per il consumatore e hazard: copiare la fixture impedisce l'invocazione, anche con metadati validi; usare il payload dello schema IO sullo stesso riferimento consente la lettura. Rientro: allineare la fixture in plenora-contracts oppure pubblicare una v2 dello schema di ingresso. Registro: `contracts/adozione-4.0.0.json`; prove: `runtime_vettori` |
| Runtime `io.read`: vettore di cancellazione | Deviazione, regola 4, da RUNTIME-VECTORS-1.0 §3 (`io-read-cancelled-error`), per la lettura con token gia' cancellato: `CANCELLED`, fase `probe`, retry `never`, contro `READ_CANCELLED`, `read`, `safe` della fixture. Categoria `cancelled` ed effetto `none` coincidono. Il binding conserva gli assi delle operazioni comuni a CLI e Rust (SURFACE-BINDINGS-1.0 §1). Conseguenza per il consumatore: nessun retry automatico. Hazard: chi instrada per codice/fase della fixture puo' non riconoscere l'errore; chi presume `safe` puo' ritentare contro l'indicazione effettiva. Rientro: allineare la fixture in plenora-contracts oppure cambiare gli assi su tutte le superfici in una release dichiarata. Registro: `contracts/adozione-4.0.0.json`; prove: `runtime_vettori` |
| GeoParquet: compatibilita' di lettura | Il fork comune con data-tools2 `6492733` rifiuta anche file validi prima leggibili: `DELTA_BINARY_PACKED`, `DELTA_LENGTH_BYTE_ARRAY`, `DELTA_BYTE_ARRAY`, `BYTE_STREAM_SPLIT` e `BIT_PACKED` per valori e livelli non sono qualificati. Sono rifiutati anche la rappresentazione Arrow `Dictionary` di `FIXED_LEN_BYTE_ARRAY` (distinta dalla codifica Parquet a dizionario) e gli `INTERVAL` validi non rappresentabili nell'unita' Arrow scelta, anche in `PLAIN`: mesi non nulli con `DayTime` (default senza schema Arrow incorporato), giorni o millisecondi non nulli con `YearMonth`, componenti conservate unsigned oltre `i32::MAX`. Il delta esclude inoltre dizionari Arrow binari su colonne testuali (`STRING`/`UTF8`, `JSON`, `ENUM`) e tipi Arrow testuali su byte senza annotazione testuale; rifiuta hint Arrow discordanti (fuso su timestamp locale, precisione/scala decimale diversa, risoluzione diversa su timestamp locale non INT96, reinterpretazione temporale di interi annotati). Questi vincoli sulla rappresentazione si applicano anche a pagine PLAIN valide. Restano inoltre i limiti di rappresentazione del fork: FLBA di larghezza zero rifiutato; DECIMAL in BYTE_ARRAY limitato a 16/32 byte per Decimal128/256, anche con estensione del segno ridondante; statistiche FLBA piu' corte della larghezza ammesse solo con flag di esattezza esplicitamente falso (l'assenza del flag e' trattata come esatta). Il controllo di fine stream ammette solo il padding RLE/bit-packed qualificato, incluse le forme DuckDB e fastparquet, e richiede varint minimi. Questi vincoli non equivalgono al supporto di ogni rappresentazione permessa dal formato Parquet. Sono rifiutati anche `INDEX_PAGE`, lettura di page index (`ColumnIndex`/`OffsetIndex`), bloom filter e salti non qualificati (selezioni/filtri di righe, offset, salto di record/pagine). IO non usa questi ultimi percorsi; usa `with_row_groups` per il pruning. Indici/bloom presenti ma non richiesti non causano da soli il rifiuto. Per Dictionary FLBA: messaggio statico `lettura Parquet fallita`; per INTERVAL non rappresentabili e codifiche escluse nei batch: errore tipizzato `DataMapping`, fase `Read`, retry `Never`, effetto `None`, messaggio statico `batch Parquet non leggibile`, senza dati; nessun panico o risultato vuoto. Rientro: qualificare ogni percorso nel fork condiviso (fuzz e regressioni), adottare la base qualificata e verificarla al confine IO. Vedi il [registro del fork](../assurance/registries/vendor-parquet-fork.json) |
| IPC | Piu colonne `geoarrow.wkb`, con o senza primaria, file e stream. Stato e forma canonica sono governati da Arrow Primary Geometry 1.0. GEO-012 resta per i sink singoli. |
| IPC | **un `geo` di schema passa tale e quale.** Un Arrow IPC può portare nei metadati dello schema un documento `geo` (geopandas lo scrive con `to_feather`). IO non lo legge né lo confronta con il contratto della colonna, che resta l'unica fonte del CRS; da IPC a IPC la chiave passa byte per byte, come ogni chiave che IO non possiede (ARROW-010), anche quando contraddice il campo (prova `rilettura_dei_crs.rs`, caso `32632_pj_geo_stantio`). Non arriva mai a un GeoParquet scritto da IO, né a un IPC scritto da un'altra sorgente: il writer GeoParquet non copia `geo` nello schema, e il lettore GeoParquet lo consuma nel contratto. Hazard: un consumatore che legge il `geo` di un IPC (geopandas `read_feather`) da un IPC→IPC vede il CRS stantio della sorgente. Rientra quando il lettore IPC confronta un `geo` di schema con il contratto e rifiuta la contraddizione (ARROW-006) |
| IPC, GeoParquet | Il CRS per campo usa un unico lettore per primaria e secondarie: blocco canonico, chiave legacy `crs`, estensione `ARROW:extension:metadata`. Nell'estensione sono letti `authority_code`, PROJJSON (anche senza `crs_type`) e `wkt2:2019`; forme non interpretabili e dichiarazioni discordanti sono errori `crs`, mai `missing`. Due definizioni testualmente diverse sono rifiutate anche quando potrebbero essere equivalenti: il bordo non offre un resolver di equivalenze. Un CRS noto che GeoParquet non può esprimere senza PROJJSON è un rifiuto esplicito; `crs: null` resta riservato a un CRS realmente mancante. Prove `crs_only_in_geoarrow_extension_metadata_is_preserved` e `crs_legacy_secondario_senza_projjson_non_diventa_null`. |
| IPC | **solo i sette tipi semplici e la semantica `geometry`.** Il sink IPC (`WKB_PASSTHROUGH_GEOMETRY`) dichiara Point, LineString, Polygon, i tre Multi e GeometryCollection: una colonna che dichiara CircularString, Triangle o altri tipi estesi, o la semantica `geography`, si rifiuta con `unsupported` prima di ogni effetto (GEO-000, verdetto di supporto; GEO-010). Lo stesso valore dentro una GeometryCollection resta un valore e segue GEO-011. Il catalogo dichiara lo stesso insieme. Rientro: un sink IPC che dichiari i tipi estesi. |
| tutti: identificatore di radice di un WKT | lo SRID di radice di un WKT (capability e perdita di fedeltà) si legge con la stessa grammatica della verifica GEO-005, WKT 1 o WKT 2: `AUTHORITY` in WKT 1, `ID` in WKT 2, solo alla radice, con parentesi quadre o tonde. Fino alla 5.x uno scanner di byte più largo contava solo le parentesi quadre (un CRS annidato fra tonde esponeva il suo identificatore come di radice), trovava `ID[` dentro `XID[` e accettava le parole dell'altra grammatica: due letture della stessa radice davano due CRS. Un WKT non ben formato non ha SRID di radice. **L'identificatore che lo Shapefile (`.prj`) e il DXF (`GEODATA` in WKT) danno a una definizione WKT, con o senza `--assume-crs`, lo decide una regola sola** (`crs_definizione::identificatore_wkt` in `plenora-io-model`), sull'albero del lettore stretto: (1) il testo si legge nella grammatica WKT 1 o WKT 2 in cui è ben formato; malformato in entrambe è `crs`, con o senza opzione, e `io.inspect` e `io.read` rispondono allo stesso modo; (2) le dichiarazioni sono gli identificatori della radice con la parola della grammatica **più** quelli con la parola dell'altra (`ID` in WKT 1, `AUTHORITY` in WKT 2), che per GEO-005 non dichiarano ma chi legge il file legge come dichiarazioni; un nodo malformato dell'una o dell'altra parola (`ID[EPSG,3003]`, `ID["EPSG"]`, `ID["EPSG",3003.5]`) rende la radice indecidibile: `crs`; (3) una radice `BOUNDCRS`, riconosciuta dalla parola della radice con ogni spaziatura e maiuscola, non si risolve mai: senza opzione è non risolta, con l'opzione è `crs`; (4) due dichiarazioni diverse (autorità ASCII case-insensitive, codice come testo) sono `crs`, con o senza opzione; (5) una dichiarazione: senza opzione si risolve solo un `EPSG` con codice intero canonico, come `EPSG:<codice>`, e ogni altra autorità (`ESRI`, `OGC:CRS84` di radice) resta non risolta; con l'opzione, l'opzione deve essere uguale alla dichiarazione, e l'identificatore ha la grafia del file; (6) nessuna dichiarazione: la forma ESRI di WGS 84 **intera** — `GEOGCS[nome, DATUM[nome, SPHEROID[nome, 6378137, 298.257223563]], PRIMEM["Greenwich", 0], UNIT["degree", 0.0174532925199433]]` e nessun altro nodo (né `AXIS`, né `TOWGS84`, né un `AUTHORITY` annidato), con i nomi `WGS 84` o `GCS_WGS_1984` per la radice, `WGS_1984` o `D_WGS_1984` per il datum, `WGS 84` o `WGS_1984` per l'ellissoide, interi e ASCII case-insensitive, e i numeri confrontati come decimali esatti (`6378137.0` vale `6378137`; una forma esponenziale o `0.017453292519943295` non sono riconosciute) — è `EPSG:4326` senza opzione, e con l'opzione ammette solo `EPSG:4326` e `OGC:CRS84`; ogni altra definizione resta non risolta senza opzione, e con l'opzione l'opzione la nomina, salvo un identificatore che il bordo sa di genere contrario alla radice (riga «tutti: genere del CRS»). Le dichiarazioni (2) si confrontano ciascuna con la prima, in tempo lineare. Fino a c0e84cb la forma ESRI si riconosceva dai due soli nomi (`GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["GRS 1980",...]],PRIMEM["Paris",2.337],UNIT["grad",...]]` e `GEOGCS("WGS 84",DATUM("WGS_1984"))` erano `EPSG:4326`), la grafia di ArcGIS no, e le dichiarazioni si deduplicavano in tempo quadratico. Fino a a651844 ogni driver decideva con più letture parziali del testo, e sette riletture di fila ne trovarono una nuova che dava un'etichetta sbagliata senza errore: un `ID["EPSG",4258]` alla radice del WGS 84 ESRI usciva `EPSG:4326`; `AUTHORITY["EPSG","3003"],ID["EPSG",3004]` usciva `EPSG:3003`; un `ID` malformato lasciava passare l'opzione; nel DXF `--assume-crs EPSG:4326` passava su un `ID` in WKT 1, un'autorità ESRI, un `BOUNDCRS`, un `AUTHORITY` in WKT 2; il `DATUM["WGS_1984"` si cercava ovunque nel testo; un `.prj` malformato dava a `io.inspect` un identificatore che `io.read` poi rifiutava. Il formato di una definizione (`wkt` o `wkt2`) si decide con lo stesso lettore, WKT 2 e poi WKT 1; il prefisso maiuscolo esatto (`PROJCRS[`) resta solo per un testo che non è ben formato in nessuna delle due: prima `PROJCRS [`, `projcrs[` o `PROJECTEDCRS[` erano `wkt` e la verifica GEO-005 li rifiutava. Hazard residuo (6): il bordo non è un resolver, e un'opzione su una definizione senza dichiarazioni e diversa dalla forma ESRI intera di WGS 84 la nomina anche quando la contraddice, purché il genere non si contraddica in modo visibile: `EPSG:3003` su un UTM anonimo, `EPSG:4258` su un geografico anonimo qualunque, `EPSG:32632` su un altro proiettato. Si rifiuta soltanto un identificatore di cui il bordo conosce il genere (`EPSG:4326` e `OGC:CRS84` geografici, `EPSG:3857` proiettato) su una radice del genere opposto o che non è un CRS orizzontale (riga «tutti: genere del CRS»). Rientra con un resolver CRS. Due divergenze conservative dichiarate: un `.prj` vuoto o di soli spazi è `crs` anche con l'opzione (non è un WKT ben formato); un `ID["EPSG",4326.0]` in un WKT 1 è `crs` perché la regola (2) lo legge come dichiarazione malformata, mentre per GEO-005 e per l'oracolo in un WKT 1 `ID` non è un identificatore e si ignora. Sono rifiuti in più, mai un CRS sbagliato. Prove: `regola_wkt_senza_dichiarazioni_e_forma_esri`, `regola_wkt_con_una_dichiarazione`, `regola_wkt_con_l_altra_parola_chiave`, `regola_wkt_ambigua_o_legata`, `regola_wkt_malformata`, `definition_format_decide_la_grammatica_con_il_lettore`, `il_prj_segue_la_regola_unica_dell_identificatore`, `il_wkt_del_geodata_segue_la_regola_unica`, `inspect_e_read_concordano_su_un_prj_malformato`, `definition_epsg_rejects_ambiguous_or_nested_only_ids`, `una_definizione_wkt_multibyte_non_fa_panicare_il_bordo` |
| tutti: genere del CRS | il genere (geografico, proiettato, ignoto) che sceglie fra `lon_lat` ed `easting_northing` lo decide una funzione sola, `plenora_io_model::crs::crs_kind_for`, per Shapefile, DXF, GeoPackage, GeoParquet, CSV, XLSX e la rilettura dei metadati Arrow IPC (FileGDB lo chiede a GDAL). La definizione decide con la parola della **radice** letta dal lettore stretto (`crs_definizione::genere_radice`): `GEOGCS`, `GEOGCRS`, `GEOGRAPHICCRS` sono geografici, `PROJCS`, `PROJCRS`, `PROJECTEDCRS` proiettati, `GEODCRS` e `GEODETICCRS` geografici solo con un `CS[ellipsoidal,...]` figlio diretto; in PROJJSON decide il `type` della radice (`GeodeticCRS` solo con `coordinate_system.subtype` `ellipsoidal`). Ogni altra radice — `GEOCCS`, `GEODCRS` senza `CS[ellipsoidal,...]`, `LOCAL_CS`, `ENGCRS`, `VERT_CS`, `VERTCRS`, `COMPD_CS`, `COMPOUNDCRS`, `DERIVEDPROJCRS`, `FITTED_CS`, `TIMECRS`, `PARAMETRICCRS` e gli stessi `type` PROJJSON — ha genere **ignoto** con un identificatore che il bordo non conosce, ed è `crs` con uno di quelli che conosce (`EPSG:4326`, `OGC:CRS84`, `EPSG:3857`, in qualunque maiuscola), dichiarato o nominato da `--assume-crs`: quell'identificatore nomina un CRS orizzontale 2D, e la radice dice che non lo è. Fino a b4d8064 il genere restava ignoto e il CRS usciva **risolto**: un `.prj` `LOCAL_CS["site",LOCAL_DATUM["d",0],UNIT["metre",1],AXIS["X",EAST],AXIS["Y",NORTH]]` con `--assume-crs EPSG:4326` era `EPSG:4326`. Un `BOUNDCRS` (`BoundCRS` in PROJJSON) resta ignoto qualunque sia l'identificatore, come prima: non ha un genere proprio, e `--assume-crs` su di lui è già `crs` (regola (3) dell'identificatore). Una definizione malformata resta ignota qui e la rifiuta la verifica GEO-005. Un genere ignoto dà sempre `axis_order` `unknown`. Senza definizione, o con un PROJJSON senza `type`, decide l'identificatore, solo se il bordo lo conosce già: `OGC:CRS84` ed `EPSG:4326` geografici, `EPSG:3857` proiettato. Un identificatore noto di genere contrario alla radice — `EPSG:4326` o `OGC:CRS84` su un `PROJCS`, dichiarato o nominato da `--assume-crs`; `EPSG:3857` su un `GEOGCS`; un `id` EPSG:4326 su un `type` `ProjectedCRS` — è `crs`. Fino a c0e84cb ogni driver cercava `PROJCS[` e `GEOGCS[` nel testo, con un suo elenco: da quando `PROJECTEDCRS[`, `projcrs [` e `PROJCRS(` si leggono come `wkt2` gli assi uscivano est/nord e il genere geografico (il `BASEGEOGCRS[` annidato contiene `GEOGCRS[`), cioè `lon_lat` su dati UTM, e la conversione in IPC lo scriveva (Shapefile EPSG:32632, GeoPackage 3003, DXF); già prima `PROJCS [`, `PROJCS(` e `PROJCS\t[` uscivano geografici, `GEOGCS["PROJCS[",...]` proiettato, un composto con un `PROJCRS[` dentro proiettato e, nel DXF, un `GEODCRS` geocentrico geografico; in GeoPackage e GeoParquet l'identificatore `EPSG:4326` vinceva sulla definizione; CSV e XLSX distinguevano le maiuscole (`epsg:4326` ignoto). Hazard: nessun genere sbagliato dalla radice; un `GEODCRS` ellissoidale senza `CS` e un PROJJSON con un altro `type` restano ignoti, un'informazione in meno. Prove: `il_genere_viene_dalla_parola_della_radice`, `il_genere_segue_la_radice_e_rifiuta_l_identificatore_contrario`, `il_genere_del_prj_viene_dalla_radice`, `assume_crs_di_genere_contrario_alla_radice_e_rifiutato`, `il_genere_del_geodata_viene_dalla_radice`, `assume_crs_geografico_su_un_proiettato_anonimo_e_rifiutato`, `il_genere_della_riga_srs_viene_dalla_radice`, `il_genere_del_documento_crs_viene_dal_type`, `un_identificatore_orizzontale_su_una_radice_che_non_lo_e_si_rifiuta`, `un_identificatore_orizzontale_su_un_prj_non_orizzontale_e_crs`, `un_identificatore_geografico_su_un_local_cs_e_crs`, `geodata_local_cs_e_bom`, `il_genere_dell_opzione_segue_la_regola_comune` (CSV e XLSX: `epsg:4326` minuscolo geografico, `EPSG:3857` proiettato) |
| tutti: GEO-005 e GEO-020 in lettura | il CRS che un driver emette passa, in `with_read_budget` e per ogni driver, le verifiche che la scrittura Arrow IPC fa sul campo (`crs::verifica_crs_emesso`): una definizione malformata nel formato che dichiara, o con un identificatore di radice dell'autorità di `crs_id` diverso da `crs_id`, e un identificatore di catalogo locale dichiarato risolto (`POSTGIS:4326` di un GeoPackage) sono `crs` già in `io.inspect` e `io.read`. Fino a c0e84cb li rifiutava solo la scrittura IPC, e una conversione verso un altro formato li portava con sé. Le verifiche sono **del layer** (riga «più layer: un layer rifiutato»): in un GeoPackage a più layer il layer con la riga di `gpkg_spatial_ref_sys` malformata o di un catalogo locale non si legge, gli altri sì. Hazard: quel layer non si legge più, dove prima si leggeva e non si convertiva in IPC. Prove: `with_read_budget_rifiuta_una_definizione_che_geo_005_rifiuta`, `verifica_definizione_emessa_applica_geo_005` |
| più oggetti: un oggetto rifiutato | GPKG, FileGDB, XLSX e DXF conservano ogni oggetto enumerato e il suo indice. `rifiuto_del_layer` e `status.error` restituiscono lo stesso errore tipizzato. `layers` e `inspect` riescono anche quando tutti gli oggetti sono rifiutati. `read`, `write` e `convert` verso formati a layer singolo selezionano automaticamente soltanto un dataset composto da un unico oggetto leggibile; con piu' oggetti, anche se uno solo e' leggibile, richiedono `--layer` con errore `LAYER_SELECTION_REQUIRED`, indici e stati. `convert` verso GPKG/FileGDB copia tutti gli oggetti leggibili e dichiara gli esclusi (`source_object_not_readable`, conteggio, indici, motivo tipizzato); `--layer` seleziona un indice esplicito. Le selezioni esplicite `xls.sheet` e `--layer` devono concordare. KML usa automaticamente la vista aggregata; Document/Folder/Placemark restano viste selezionabili per indice, non oggetti da copiare insieme all'aggregato. Ordini e selezione sono documentati nel README. Quote, scadenze, cancellazioni e struttura illeggibile del contenitore restano errori globali. |
| tutti: tetto della definizione | **ogni** definizione CRS oltre 1 MiB (`crs_definizione::MAX_BYTE_DEFINIZIONE_CRS`), da qualunque sorgente, è `limit_exceeded`: il `.prj`, il `GEODATA`, la riga di `gpkg_spatial_ref_sys` (che `SQLite` non consegna oltre una guardia di due volte il tetto nella codifica del database, e il tetto esatto si applica poi sui byte UTF-8: fino alla rilettura 10 la guardia era il tetto stesso nella codifica del database, e in un GeoPackage UTF-16 una definizione ASCII di 600 KB si rifiutava sotto il tetto), il `crs` di GeoParquet, il metadato `plenora.geometry.crs_definition` di un campo Arrow IPC (anche in scrittura, che lo rilegge), la definizione di FileGDB. La regola è una: la applicano `crs_kind_for`, `verifica_crs_emesso`, la lettura dei metadati Arrow e la verifica dello schema IPC (`vocabolario::verifica_schema`), sempre **prima** del lettore stretto (`verifica_dimensione_definizione`), che non apre un testo più lungo e lo direbbe malformato. Fino alla rilettura 10 la verifica dello schema IPC chiamava il lettore prima del tetto, e una definizione di 1 MiB e un byte era `crs` «non ben formata» invece di `limit_exceeded`. Il lettore tiene un token per elemento, qualche decina di byte per byte d'ingresso nel caso peggiore. Fino a c0e84cb il `.prj` si leggeva intero; fino a b4d8064 le definizioni di un metadato erano limitate solo dai byte del metadato (16 MiB in Arrow IPC): una riga di `gpkg_spatial_ref_sys` di 8 MiB costava 698 MB di picco e 12 s in debug, una di 32 MiB esauriva la scadenza. Deviazione dichiarata (regola 4): ARROW-VOCABULARY-1.0 non pone un tetto alla definizione, e un campo IPC conforme con una definizione oltre 1 MiB è un rifiuto di supporto (`limit_exceeded`), mai un risultato diverso; rientra con un lettore che non materializzi i token. La stessa definizione si legge ancora più volte per apertura (formato, genere, assi, GEO-005): il costo è lineare e limitato dal tetto, e non è stato ridotto. Prove: `un_prj_oltre_il_tetto_e_un_limite`, `una_definizione_del_geodata_oltre_il_tetto_e_un_limite`, `una_definizione_oltre_il_tetto_e_un_limite`, `la_definizione_della_riga_srs_ha_il_tetto`, `il_tetto_della_definizione_e_in_utf8_anche_in_un_database_utf16`, `la_definizione_di_un_metadato_ha_il_tetto_comune`, `una_definizione_oltre_il_tetto_e_un_limite_non_un_crs_malformato` |
| tutti: BOM della definizione | un BOM UTF-8 iniziale (uno solo) di un `.prj`, di una riga di `gpkg_spatial_ref_sys` o di un `GEODATA` si toglie all'ingresso (`crs_definizione::senza_bom`): non fa parte del WKT, e la definizione emessa è quella senza. Fino a b4d8064 la definizione si leggeva malformata. Un secondo BOM resta, e la definizione resta malformata. Più `GEODATA` si confrontano **dopo** aver tolto il BOM: fino alla rilettura 10 si confrontavano prima, e due definizioni uguali, una con il BOM, erano un conflitto `crs`. Prove: `un_prj_con_il_bom_si_legge`, `un_bom_iniziale_della_definizione_si_toglie`, `geodata_local_cs_e_bom`, `due_geodata_uguali_a_meno_del_bom_concordano` |
| GeoParquet: `crs: null` | `null` è un CRS ignoto (GeoParquet 1.1), e nel vocabolario è `missing`, l'unico stato senza identificatore né definizione (ARROW-VOCABULARY-1.0 §4). Fino a b4d8064 era un `resolved` senza nessuno dei due: `io.read` lo emetteva e la conversione in IPC lo rifiutava («metadato CRS non valido»). Ora lettura e conversione concordano, e un `missing` si riscrive in GeoParquet come `null` (dalla rilettura 10: fino a b7a6c94 la validazione comune lo rifiutava, `unsupported` «il formato richiede un CRS risolto», contro ciò che questa riga diceva; prova end-to-end `un_crs_ignoto_si_riscrive_in_geoparquet_come_null`). Con `--assume-crs` è `crs` («non applicato»), come prima era `crs` («non confrontabile»). Prova: `un_crs_mancante_si_scrive_null_e_si_rilegge_sconosciuto` |
| Arrow IPC: genere riletto | il genere di un CRS `resolved` riletto dai metadati segue `crs_kind_for` (identificatore e radice della definizione); solo se la definizione manca e l'identificatore è ignoto decide l'ordine memorizzato (`lon_lat`/`lat_lon` geografico, `easting_northing`/`northing_easting` proiettato). Fino a b4d8064 decideva sempre l'ordine, e un `EPSG:3031` polare, proiettato per lo Shapefile con assi `other`, si rileggeva ignoto. Un identificatore che contraddice la radice qui lascia il genere ignoto senza rifiutare: il genere non è un campo del vocabolario, e il campo è quello che GEO-005 ha già giudicato. Prova: `il_genere_riletto_segue_la_regola_comune` |
| tutti: WKB ben formato (GEO-011) | ogni WKB o EWKB che IO decodifica, ispeziona o codifica rispetta la buona forma di Arrow Geometry Semantics 1.0 come la decide l'oracolo dei contratti (`arrow_data.py`): una linestring ha zero o almeno due punti, una circularstring zero o un numero dispari di almeno tre, un anello lineare di poligono o triangolo almeno quattro punti con l'ultimo uguale al primo **byte per byte** (Z e M comprese: `-0.0` non chiude `0.0`, un NaN con gli stessi bit chiude), un triangolo al più un anello, al più 32 livelli di annidamento, e ogni membro di un aggregato ha le dimensioni del contenitore. Il rifiuto è del codec stesso -- `decode_wkb`, `inspect_wkb` e l'encoder -- e non dei percorsi che lo consultano dopo: fino alla 5.0.1 il membro di altre dimensioni era solo annotato (`nested_dimensions_coherent`) e rifiutato a valle, dalla validazione dei batch e da quella delle scritture, e un percorso che non leggesse il flag lo avrebbe accettato o emesso (prova `un_membro_di_altre_dimensioni_e_malformato_nel_codec` in `wkb_lossless/tests.rs`). Vale per **ogni formato**, non solo per l'ingresso Arrow: un valore che un componente del contratto rifiuterebbe non esce da IO. Ambito: un file nativo con una polilinea di un vertice, un anello non chiuso o vuoto (KML, GeoJSON, DXF, Shapefile, GeoPackage scritti da altri) è rifiutato con `data_mapping` invece di essere letto; fino alla 5.x passava. Hazard: file prima leggibili ora falliscono, con un errore esplicito e nessun dato pubblicato; nessuna chiusura o riparazione implicita. Rientro: nessuno previsto, è la conformità; una riparazione esplicita sarebbe un'opzione nuova e dichiarata. Prove: `geo_011_*` in `plenora-io-model` (`wkb/tests.rs`), `un_valore_malformato_con_un_altro_srid_e_data_mapping` in `plenora-io-tools` Un membro che porta insieme uno SRID e dimensioni diverse dal contenitore è rifiutato per le dimensioni (GEO-011) dove l'oracolo cita prima lo SRID del membro (GEO-009): la categoria, `data_mapping`, è la stessa, cambia solo la causa di riga riportata. |
| tutti: `max_wkb_depth` | la profondità di GEO-011 (32) non è una quota: il parser la applica qualunque sia la configurazione, e `max_wkb_depth` (`--max-wkb-depth`, budget runtime) può solo stringerla. Un valore oltre 32 si rifiuta alla validazione dei limiti (`invalid_configuration/CLI_USAGE`) invece di restare un flag senza effetto; il default passa da 64 a 32. Hazard: una configurazione che chiedeva 64 ora fallisce. Prove: `geo_011_l_annidamento_si_ferma_a_32_qualunque_sia_il_tetto`, `max_wkb_depth_oltre_il_contratto_si_rifiuta` |
| CRS: PROJJSON (GEO-005) | gli identificatori di un PROJJSON si leggono con un lettore dedicato che replica `json.loads` con i ganci dell'oracolo: numeri di qualunque grandezza (`1e400`), surrogati isolati, codici `-0` (vale `0`) o oltre `u64`, qualunque annidamento. Restano tre differenze, dove l'oracolo in CPython non dà un verdetto perché si ferma con un'eccezione dell'interprete e non con `Malformed`: `RecursionError` oltre circa mille livelli, `ValueError` per un intero di più di 4300 cifre, `decimal.InvalidOperation` per un decimale con un esponente oltre i limiti di `decimal` (`9e9999999999999999999999999999`). Lì IO dà il verdetto della grammatica, perché legge i numeri come testo senza valutarli: il documento è ben formato, un intero lungo è un codice valido, un decimale -- con qualunque esponente -- come `code` è malformato e altrove è un valore qualunque (prove `projjson_*` in `crs_definizione/tests.rs`). **Memoria**: lineare nei byte del documento con un fattore dichiarato -- un byte per contenitore aperto, 16 byte per chiave di un oggetto aperto (liberati alla sua chiusura), della radice soltanto `id` e `ids` e di ogni identificatore soltanto `authority` e `code`, in UTF-8. Fino alla 5.0.1 la verifica teneva un `HashSet` per oggetto aperto e 48 byte per contenitore, e la lettura materializzava ogni membro della radice fino al terzo livello. Misurato in release su documenti di 16 MiB, picco delle allocazioni prima e dopo: 403 MB e 8 MB (`{"a":` e 8 milioni di `[`), 591 MB e 71 MB (3,3 milioni di `{"":` annidati, il caso peggiore: 4,3 volte i byte a 16 MiB, fino a circa 6,8 volte su dimensioni dove le capacità dei vettori raddoppiano appena oltre il necessario, con un tetto assoluto di circa 71 MB fino a 16 MiB), 277 MB e meno di 1 KB (8 milioni di numeri in una lista), 170 MB e 34 MB (2,4 milioni di chiavi in un oggetto), 233 MB e 56 MB (600 mila `ids`, quasi tutto il risultato). Il documento è già limitato dai byte del metadato che lo porta. Il doppione di chiave si cerca alla chiusura dell'oggetto, su impronte SipHash a chiavi casuali e con il confronto esatto delle sole chiavi di impronta uguale: il verdetto non dipende dall'impronta (prova `le_impronte_uguali_non_cambiano_il_verdetto`). Lo **SRID** ricavato da un PROJJSON (`definition_authority_srid`, usato dalle capability e dal confronto delle rappresentazioni nel `LossReport`) usa ora lo stesso lettore: un codice numerico è un intero, un codice stringa è testo e vale solo se canonico, come nel WKT. Fino alla 5.0.1 una stringa passava da `str::parse`, e `"04326"` o `"+4326"` davano 4326 mentre lo stesso codice in un WKT non dava SRID (prova `definition_authority_srid_projjson_distingue_il_numero_dalla_stringa`). Gli **altri** usi di un PROJJSON in IO -- l'ordine degli assi dichiarato verso un formato con l'est prima, il documento `crs` di GeoParquet -- leggono ancora con `serde_json` senza `arbitrary_precision`: per un documento con un numero oltre `f64`, un surrogato isolato o più di 128 livelli l'ordine resta `unknown`, e GeoParquet tratta la definizione come una che non è PROJJSON (come un WKT): con `EPSG:4326` o `OGC:CRS84` omette il `crs`, che per il formato vale CRS84, e dalla rilettura 10 dichiara la definizione perduta (`crs_definition_not_preserved_absent`), dalla rilettura 11 con `EPSG:4326` anche l'identificatore (`crs_id_not_preserved_absent`), altrimenti rifiuta con `unsupported`. Quando invece il documento si scrive, si scrive e si rilegge con i suoi byte (`RawValue`): `serde_json` serve a validarlo, non a riscriverlo. Hazard: nessun risultato sbagliato, un rifiuto o un'informazione in meno. Rientro: portare quei due usi sul lettore dedicato. Prove: `projjson_*` in `crs_definizione/tests.rs` |
| CRS: spazi e maiuscole | lo spazio della grammatica di `crs_id` (GEO-016) e del WKT è `str.isspace()` di Python 3.11 (Unicode 14), scritto come elenco chiuso: il `White_Space` di Unicode più U+001C..U+001F. Le autorità si confrontano ASCII case-insensitive, come scrivono GEO-005 e GEO-020, anche nella lettura di `gpkg_spatial_ref_sys`: `EPſG` (U+017F) è un'altra autorità. L'oracolo al pin 3aec9f7 usa `str.upper()` e qui diverge; la correzione dell'oracolo è plenora-contracts #53, e nessun vettore copre il caso. Hazard: nessuno per autorità ASCII. Prove: `lo_spazio_e_quello_di_python`, `l_autorita_si_confronta_solo_in_ascii`, `reader_uppercases_the_organization_in_ascii_only` |
| GeoPackage: codice CRS non canonico | `srs_id` e `organization_coordsys_id` sono interi: un `crs_id` il cui codice non è un intero canonico (`EPSG:04326`, `EPSG:+4326`) non ha un intero che lo rappresenti senza cambiarne l'identità (GEO-005 confronta i codici come testo), e si rifiuta con `unsupported/validate` prima di creare la destinazione. Fino alla 5.x diventava 4326 e si rileggeva `EPSG:4326`. Lo stesso vale per lo SRID ricavato da un identificatore (`authority_srid`, GeoParquet) e per un codice **stringa** di un `AUTHORITY` WKT: `"04326"` non diventa 4326; un codice **numerico** resta un intero, come per l'oracolo. Hazard: un identificatore così non si scrive in GeoPackage. Prove: `gpkg_rifiuta_un_codice_non_canonico_invece_di_cambiarne_l_identita`, `authority_srid_solo_per_un_codice_canonico` |
| FileGDB | richiede `gdal-backend`. Senza, ogni chiamata fallisce come capability mancante |
| FileGDB | **si apre con il solo driver GDAL `OpenFileGDB`.** Una directory `.gdb` che GDAL saprebbe aprire con un altro driver (per esempio una directory di CSV) è rifiutata invece di essere letta con le regole di un altro formato (prova `una_directory_gdb_che_non_e_un_filegdb_non_si_apre`). Un layer con più campi geometrici resta enumerato, ne dichiara il numero in `native_metadata["filegdb.geometry_field_count"]`, e la sua lettura è rifiutata con `unsupported` (GEO-012; prova `piu_campi_geometrici_rendono_il_layer_non_supportato`): prima si prendeva il primo campo e gli altri sparivano. Rientra con un contratto multi-geometria per layer |
| GeoPackage | **una sola colonna geometrica per feature table.** Una tabella con più righe in `gpkg_geometry_columns` è rifiutata all'apertura con `unsupported`, exit 3 (GeoPackage requisito 30; prova `due_colonne_geometriche_per_la_stessa_tabella_sono_rifiutate`): prima ogni riga diventava un layer con lo stesso nome, e in ciascuno l'altra geometria usciva come BLOB. «La stessa tabella» e «la colonna geometrica» si giudicano come li risolve SQLite, senza distinzione di maiuscole ASCII: `features` e `FEATURES` in `gpkg_contents` sono una tabella sola (il join con `gpkg_geometry_columns` usa `NOCASE`), e un `column_name` `GEOM` è la colonna `geom`. Fino alla 5.0.1 il confronto era esatto: la stessa tabella si apriva come due layer, e una colonna geometrica dichiarata con altre maiuscole usciva una seconda volta come attributo binario (prove `due_geometrie_sotto_nomi_di_tabella_che_differiscono_per_maiuscole_sono_rifiutate`, `la_colonna_geometrica_si_riconosce_come_la_riconosce_sqlite`). Hazard: il rifiuto chiude l'intero file, anche le altre tabelle. Vincolo del formato: requisito 30, una geometria per tabella o vista; nessuna suddivisione automatica in altre tabelle |
| GeoPackage: layer dichiarati | una riga `features` di `gpkg_contents` senza la sua riga in `gpkg_geometry_columns` (GPKG 1.3, requisito 22), e nel verso opposto una colonna di `gpkg_geometry_columns` la cui tabella non è una riga `features` di `gpkg_contents` (assente, `attributes`, o `Features` scritto con un'altra grafia), rendono il file non leggibile: errore `data_mapping` all'apertura, invece di un elenco di layer con quella tabella mancante. Fino alla 5.x quei layer sparivano in silenzio. Le tabelle `attributes` senza geometria non sono layer di IO e non si elencano: IO legge feature table. Un file con righe orfane in `gpkg_geometry_columns` (lasciate, per esempio, da un `DROP TABLE` fatto a mano) si rifiuta per intero, dove altri lettori le ignorano: è il requisito della specifica, ed è un rifiuto in più. Prove: `una_feature_table_senza_colonna_geometrica_dichiarata_e_rifiutata`, `una_geometria_dichiarata_fuori_da_una_feature_table_e_rifiutata` |
| GeoParquet senza metadato `geo` | la geometria si riconosce dal nome (`geometry`, `geom`, `wkb`); con **più** di quei nomi nel file l'apertura è rifiutata con `unsupported` invece di prendere il primo dell'elenco (prova `un_parquet_senza_geo_con_due_nomi_di_geometria_e_ambiguo`). Il CRS è `missing`, oppure quello che `--assume-crs` nomina, con ordine degli assi `unknown` (la convenzione longitudine prima è del metadato `geo`): il default `OGC:CRS84` di GeoParquet 1.1 vale per una colonna dichiarata nel metadato senza la chiave `crs`, e un Parquet senza `geo` non è GeoParquet. Fino alla 5.0.1 il driver lo etichettava WGS84 (prova `un_parquet_senza_geo_non_afferma_un_crs`). Hazard: un sink che pretende un CRS ora rifiuta questi file finché l'opzione non lo nomina |
| CSV, XLSX | `wkt_column` esclude `x_column` e `y_column`: dichiararli insieme è un errore di configurazione (`invalid_configuration`), non più una precedenza silenziosa del WKT (prove `wkt_column_insieme_a_x_y_e_una_configurazione_ambigua` in `driver-csv`, `n1_resolve_geometry_nomina_la_colonna_che_manca_e_rifiuta_due_geometrie` in `driver-xls`). Un nome di `wkt_column`, `x_column` o `y_column` che l'intestazione ripete è ambiguo ed è rifiutato con `invalid_configuration/validate`: prima la geometria usciva dalla prima occorrenza, scelta in silenzio, e le altre diventavano attributi (prove `una_colonna_geometrica_dal_nome_ripetuto_e_rifiutata`, `n1_resolve_geometry_rifiuta_un_nome_che_l_intestazione_ripete`) |
| tutti | una colonna geometrica il cui nome compare **più volte** nello schema del layer è rifiutata all'apertura con `schema`, in `with_read_budget`, per ogni driver: il contratto nomina la geometria per nome, e la validazione dei batch, il conteggio dei componenti e gli esempi di perdita la cercano per nome, trovando la prima occorrenza. Un Arrow IPC con un campo `g` binario qualunque prima del campo `g` GeoArrow validava il campo sbagliato, e un GeoParquet con due colonne `geometry` le esponeva entrambe come geometria (prova `with_read_budget_rifiuta_una_colonna_geometrica_dal_nome_ripetuto`). Hazard residuo: due **attributi** con lo stesso nome restano leggibili, come la sorgente li porta, e chi li consuma per nome trova il primo; la scrittura li rifiuta già (`collisione dopo la normalizzazione`). Rientra con una regola di unicità dei nomi nel contratto |
| GeoJSON | il membro `crs` di GeoJSON 2008, su collezione, feature, geometria o figlia di una `GeometryCollection`, è ammesso solo se è un CRS con nome che nomina CRS84 o EPSG:4326, in forma breve, URN OGC (con o senza versione) o URI `opengis.net`; ogni altro nome, un CRS per link, `null`, una forma malformata o con chiavi ripetute o estranee è rifiutato con `crs`. Prima il membro si ignorava e il documento usciva `OGC:CRS84` qualunque CRS dichiarasse (prova `il_membro_crs_storico_deve_nominare_wgs84`). EPSG:4326 si accetta come CRS84 perché GeoJSON 2008 fissa l'ordine x, y, cioè longitudine prima, qualunque CRS dichiari |
| KML | nessun analogo: KML 2.2 non ha un elemento che dichiari un CRS, e il formato resta WGS84 fisso |
| GeoJSON | una chiave ripetuta nell'oggetto geometria (`type`, `coordinates`, `geometries`) o al livello del `FeatureCollection` (`type`, `features`) rifiuta il documento, come già le chiavi ripetute della feature: prima valeva l'ultima, e due `features` si concatenavano (prova `chiavi_ripetute_nella_geometria_e_nella_collezione_sono_rifiutate`) |
| KML | un `Point` con più tuple, un `coordinates` ripetuto nella stessa geometria e un `Polygon` senza un unico anello esterno (due anelli nello stesso `outerBoundaryIs`, o due `outerBoundaryIs`) sono rifiutati: prima se ne teneva una parte e il resto spariva (prova `event_parser_rejects_geometries_it_used_to_truncate`) |
| DXF | più oggetti `GEODATA` con definizioni del CRS diverse sono rifiutati con `crs`: prima valeva il primo (prova `due_geodata_discordi_sono_rifiutati`). Definizioni uguali restano ammesse |
| DXF | una definizione WKT del `GEODATA` (un testo che comincia con una parola seguita da `[` o `(`) segue la regola unica di «tutti: identificatore di radice di un WKT», la stessa del `.prj`: dichiarazioni con l'una e l'altra parola chiave, `BOUNDCRS`, forma ESRI di WGS 84 (che ora dà `EPSG:4326` anche qui), malformata `crs` (prova `il_wkt_del_geodata_segue_la_regola_unica`); un codice EPSG di radice non canonico (`"04326"`), prima letto come tale, ora resta non risolto come nello Shapefile. Le regole proprie del `GEODATA` restano del driver: più `GEODATA` discordi sono `crs`, e una definizione non WKT che dichiara codici EPSG diversi — fra gli `EPSG:<codice>` del testo e gli `<Alias>` EPSG del sistema di coordinate di un XML Autodesk — è rifiutata con `crs`: prima valeva la prima occorrenza della prima forma trovata (prova `codici_epsg_discordi_nella_definizione_sono_rifiutati`). Degli alias conta solo quello del sistema di coordinate: gli attributi del tag d'apertura si leggono tutti (`nome = "valore"` o `nome='valore'`, spazi facoltativi attorno a `=`), e conta un alias con `type` uguale a `CoordinateSystem` (ASCII case-insensitive) e `id` di sole cifre. Un alias di datum o di ellissoide non conta, e nemmeno uno senza `type`, che non dice che cosa nomina; un tag che non si legge (virgolette non chiuse, valore senza virgolette, attributo ripetuto) è `crs`; `<Aliases>` è un altro elemento. Fino a b4d8064 il tipo si cercava come sottostringa `TYPE="`: `type = "Datum"` sembrava senza tipo, e l'alias del datum 6326 usciva come il CRS `EPSG:6326`; un alias senza `type` contava (prova `il_tipo_dell_alias_si_legge_come_attributo`). Un codice testuale o di alias non canonico (`EPSG:032632`) segue la regola (5) del WKT: senza opzione resta non risolto, con un'opzione uguale come testo vale la grafia del file, diversa è `crs`; fino a b4d8064 usciva risolto così com'era (prova `un_codice_testuale_non_canonico_segue_la_regola_del_wkt`). La forma testuale vale solo per una definizione che **è** l'identificatore (`EPSG:<cifre>` o `urn:ogc:def:crs:EPSG:<versione>:<cifre>`, a meno degli spazi): fino alla 5.0.1 ogni `EPSG:` seguito da cifre **ovunque** nel testo era una dichiarazione, e un proiettato senza `AUTHORITY` di radice che nel nome citava il proprio CRS base (`PROJCS["Locale basato su EPSG:4326",...]`) usciva `EPSG:4326` senza alcun segnale. Ora il nome è testo libero: quel `GEODATA` è non risolto e si nomina con `--assume-crs` (prova `un_epsg_nel_testo_della_definizione_non_e_l_identificatore`). Hazard: un testo come `EPSG:3003 EPSG:3003`, prima letto come EPSG:3003, ora richiede l'opzione. Un `--assume-crs` diverso dal codice che il `GEODATA` dichiara è rifiutato già nel resolver, oltre che nel controllo comune (prova `assume_crs_in_conflitto_col_geodata_e_rifiutato`). Una definizione che non comincia come un WKT vale solo in quattro forme — `EPSG:<cifre>`, `urn:ogc:def:crs:EPSG:<versione>:<cifre>`, `OGC:CRS84` (tutto il testo, a meno degli spazi) e un XML (testo che comincia con `<`) con un `<Alias>` EPSG del sistema di coordinate — che danno l'identificatore e **nessuna** definizione, perché non sono WKT né PROJJSON. Ogni altro testo è `crs` con `--assume-crs` e non risolto senza: un WKT preceduto da U+200B o fra virgolette, un `1` davanti a `GEOGCS[`, un XML senza alias, `OGC:CRS84 approssimato`. Fino a c0e84cb l'opzione nominava qualunque testo, il testo usciva come definizione `wkt` malformata, e un `GEODATA` così convertito in GeoParquet diventava `OGC:CRS84` su dati UTM (prova `un_geodata_non_riconosciuto_non_si_nomina`). Gli alias si cercano solo in un testo che comincia con `<`; l'XML non si valida (nessun lettore XML nel bordo), quindi un XML malformato con un alias EPSG del sistema di coordinate ne dà il codice: hazard dichiarato, rientra con un lettore XML |
| Shapefile | l'identificatore di un `.prj` lo decide la regola unica di «tutti: identificatore di radice di un WKT», la stessa del DXF; il `.prj` che è soltanto `OGC:CRS84` lo dichiara, e l'opzione lo può solo ripetere. `--assume-crs` diverso dall'identificatore che la radice del `.prj` dichiara, di qualunque autorità (`AUTHORITY["ESRI","4265"]` compreso), con la parola chiave dell'una o dell'altra grammatica (`ID[...]` in WKT 2 o in WKT 1, `AUTHORITY` in WKT 2), è rifiutato con `crs`; concordi, l'identificatore ha la grafia del file. Con l'opzione si rifiutano anche due identificatori di radice diversi e un `BOUNDCRS` (prova `assume_crs_non_nomina_radici_che_dichiarano_altro_o_niente_di_proprio`): prima vinceva l'opzione, e il contratto univa il suo identificatore alla definizione del `.prj` (GEO-005; prova `prj_e_assume_crs_in_conflitto_sono_rifiutati`). Senza opzione, due identificatori di radice diversi sono ora `crs` (prima valeva uno dei due, in silenzio) e un `ID` in WKT 1 è una dichiarazione (`EPSG:3003` si risolve, prima restava non risolto). L'opzione resta il modo di nominare un `.prj` che non dichiara un'autorità: la forma ESRI di WGS 84 senza `AUTHORITY` (quella che il writer scrive) dà `EPSG:4326` per **deduzione** del driver, che vale soltanto senza l'opzione e solo se la radice non dichiara un identificatore (`AUTHORITY["ESRI","104000"]` sopra il nome e il datum di WGS 84 non diventa `EPSG:4326`); con l'opzione, su quella forma sono ammessi solo i due nomi di WGS 84 geografico, `EPSG:4326` e `OGC:CRS84`, e ogni altro è `crs` (prova `assume_crs_non_rietichetta_un_prj_riconosciuto_o_dichiarato`). Fino alla 5.0.1 la deduzione si confrontava con l'opzione come una dichiarazione, e `--assume-crs OGC:CRS84` su quel `.prj` era rifiutato (prove `assume_crs_nomina_un_prj_esri_che_non_dichiara_autorita` in `driver-shp`, `un_prj_senza_autorita_si_nomina_con_assume_crs` in `plenora-io-tools`). Un `.prj` che non è un WKT ben formato è `crs` già all'apertura, con o senza opzione, per `io.inspect` come per `io.read` (prova `inspect_e_read_concordano_su_un_prj_malformato`): prima l'opzione lo nominava e la definizione si rifiutava solo a valle, alla verifica GEO-005, e senza opzione la forma ESRI si riconosceva anche su un testo malformato. Un `.prj` che esiste ma non si legge come testo UTF-8 è `crs` (prova `un_prj_non_utf8_e_un_errore_non_un_assenza`): prima valeva come assente, e l'opzione nominava il CRS che il file dichiarava. Rifiuti in più, mai risultati sbagliati: un `.prj` con un NUL finale si rifiuta come definizione non ben formata, con o senza opzione; un BOM UTF-8 iniziale invece si toglie (riga «tutti: BOM della definizione»). Un `.prj` che è soltanto `OGC:CRS84` dà l'identificatore `OGC:CRS84`, geografico, `lon_lat`, **senza** definizione: non è un WKT, e la chiave `crs_definition` porta solo WKT, WKT 2 o PROJJSON (ARROW-VOCABULARY-1.0). Fino a c0e84cb usciva come definizione `wkt` con ordine `unknown`: `io.read` lo emetteva e la conversione in IPC lo rifiutava (GEO-005) (prova `un_prj_ogc_crs84_non_emette_una_definizione`). Un WKT 2 con le parole chiave lunghe (`GEOGRAPHICCRS[`), minuscole o spaziate (`PROJCRS [`) è ora `wkt2` e si legge. Hazard residuo: quello della regola (6), un'opzione su un `.prj` senza dichiarazioni e diverso dalla forma ESRI di WGS 84 lo nomina anche quando ne contraddice la definizione. Gli altri driver non deducono un identificatore: GeoJSON e KML hanno il CRS fissato dal formato, GeoPackage, GeoParquet e IPC lo leggono dichiarato, il DXF da una definizione che lo dichiara o dalla stessa forma ESRI (righe sotto); il FileGDB riporta l'autorità che GDAL attribuisce allo `SpatialRef` del file, e un'attribuzione di GDAL diversa dall'opzione è un rifiuto in più, mai un CRS sbagliato. Gli altri formati seguono la stessa regola (vedi [`--assume-crs` e il CRS che la sorgente dichiara](#--assume-crs-e-il-crs-che-la-sorgente-dichiara)) |
| tutti | `--assume-crs` si confronta col CRS dichiarato per **uguaglianza** di identificatore (GEO-005), non per equivalenza: `OGC:CRS84` contro `EPSG:4326`, o un codice scritto con zeri iniziali, è un conflitto anche quando i due nomi descrivono lo stesso CRS. Hazard: un rifiuto in più, mai un CRS sbagliato; si rimedia passando l'identificatore come la sorgente lo scrive (`io.inspect` senza opzione lo mostra). Rientra con un resolver di equivalenze, che il bordo I/O non porta |
| CSV, XLSX | non portano CRS: con una geometria, `--assume-crs` è obbligatorio |
| tutti | nessuna riproiezione |
| tutti | nessuna proiezione delle colonne: lo schema consegnato è quello della sorgente |

### Geometrie multiple: formati e limite residuo

Il registro [io-geometrie-multiple.json](../assurance/registries/io-geometrie-multiple.json)
separa il supporto sviluppato dai rifiuti del formato. `io-una-geometria` è
chiuso per GeoParquet WKB, non per tutta la matrice interop.
Anche nei piani Rust diretti lo schema ricevuto determina lo stato della
primaria: senza chiave e' implicita con una geometria, assente con piu geometrie.
Un contratto interno discorde viene rifiutato con `schema` prima della
scrittura o della proiezione. Non completa una dichiarazione canonica incompleta.

`DataContract.geometry` indica la primaria; `geometries()` enumera tutte le
geometrie dai metadati per campo. Il metadato nativo per campo
`geoparquet.primary_column` conserva il nome della primaria anche attraverso
una projection. CRS e quote cumulative si verificano su tutte le geometrie.
Non si cambia il vocabolario canonico Arrow Geometry Semantics 1.0.
La vista JSON `inspect.layers[].geometry`, e quindi `Layer.geometry` dello SDK,
riporta soltanto la primaria; `fields[].geometry` riconosce invece ogni campo
geometrico. Non è una vista completa dei CRS per colonna: i metadati completi
restano nei contratti Rust e nei dati Arrow. Questo limite di introspezione
non autorizza a perdere le secondarie in conversione. Lo SDK inoltra i dati
Arrow senza riscrivere i CRS; IPC conserva tutte le geometrie e lo stato della primaria.

| Formato | Specifica e decisione |
|---|---|
| GeoParquet 1.1 | [Più colonne con primaria esplicita](https://geoparquet.org/releases/v1.1.0/): implementato per WKB, ogni CRS indipendente. |
| GeoPackage | [Requisito 30](https://www.geopackage.org/spec/#feature_user_tables): una geometria per tabella/vista. Rifiuto `unsupported`, exit 3, senza output. |
| FlatGeobuf | [Schema Feature](https://github.com/flatgeobuf/flatgeobuf/blob/master/src/fbs/feature.fbs): un campo geometry, eventualmente composto. Nessun driver in questo repository; proposta: rifiutare più colonne, senza convertirle in properties. |
| Shapefile | [Descrizione Esri](https://www.esri.com/library/whitepapers/pdfs/shapefile.pdf): una shape per record, attributi nel DBF. Rifiuto `unsupported`, exit 3; eventuali dataset separati richiedono una scelta del chiamante. |
| GeoJSON | [RFC 7946 §3.2](https://www.rfc-editor.org/rfc/rfc7946#section-3.2): un membro geometry per Feature. GeometryCollection non rappresenta colonne con CRS indipendenti. Rifiuto `unsupported`, exit 3. |
| DXF | [Entità e sezioni](https://help.autodesk.com/cloudhelp/2020/ENU/AutoCAD-DXF/files/GUID-995ABB55-571A-4D0F-882E-8A74A738643E.htm): entità grafiche, non colonne geometriche indipendenti di una riga. Rifiuto `unsupported`, exit 3; proposta futura: mapping esplicito verso entità/layer, con identità e CRS definiti. |
| FileGDB | [Modello Esri](https://pro.arcgis.com/en/pro-app/3.6/tool-reference/tool-errors-and-warnings/001001-010000/tool-errors-and-warnings-00301-00325-000311.htm): una geometria per feature class. Il controllo comune rifiuta più colonne; la verifica del backend richiede `gdal-backend`. |
| IPC/GeoArrow | Piu' campi `geoarrow.wkb` con metadati indipendenti. Arrow Primary Geometry 1.0: primaria per `plenora.field_id`, `none` singola conservata, nessuna promozione in proiezione. File e stream condividono la semantica. |

La fixture `multigeometria-pyarrow.parquet` è generata con PyArrow 25.0.1 da
`scripts/genera_fixture_multigeometria.py`: il censimento dei 38 Parquet già
tracciati non ha trovato file multigeometria. Non usa il writer del prodotto.
Le prove confrontano buffer, null, tipi, encoding, CRS e assi dopo il round-trip,
e verificano i rifiuti in scrittura e lettura su una secondaria.

### Le due serializzazioni di Arrow IPC

Arrow IPC ne definisce due, e questo prodotto le produce e le legge entrambe:

| | Come si chiede | Come si riconosce |
|---|---|---|
| `application/vnd.apache.arrow.file` | il default | `ARROW1` in testa e in coda, più il footer che indicizza i blocchi |
| `application/vnd.apache.arrow.stream` | `--out-opt serialization=stream` | una sequenza di messaggi, senza magic né footer |

La scelta è un'**opzione di formato** del driver IPC, e `io.catalog` la
pubblica accanto alle altre: non è un campo nuovo di uno schema d'ingresso.
`delivered.content_type` nel risultato dice quale delle due è uscita.

In lettura non serve chiederlo: il driver guarda i **byte**. Un flusso
chiamato `.arrow` si legge, e un contenitore chiamato `.arrows` pure — il nome
del file è del chiamante, e serve solo a scegliere il driver quando il formato
non è dichiarato. `io.write` accetta il payload in entrambe le forme e riferisce
quale ha letto.

#### Che cosa uno stream **non** cambia

La consegna resta **atomica sull'operazione**. Se una violazione emerge in un
punto qualsiasi della sorgente, l'operazione viene rifiutata come blocco unico e
nessun prefisso accettato viene consegnato — o tutto, o niente. Il descrittore
lo dice con `attributes.delivery: operation_atomic`, e vale per tutti e dieci i
driver.

È una distinzione che è facile confondere, e per un periodo l'abbiamo confusa:
lo stream è una **serializzazione**, cioè come i byte sono disposti; la consegna
incrementale è un'altra cosa, cioè il consumatore che elabora i primi batch
mentre gli ultimi non esistono ancora. Produrre un flusso non richiede la
seconda, e qui non la si fa: i byte si scrivono per intero in un file di
staging, e la pubblicazione è l'ultima operazione. Un'operazione fallita non
lascia una destinazione, in nessuna delle due forme.

Il descrittore dichiara anche `attributes.materialization: bounded`, che è la
forma in cui ARROW-011 è soddisfatto: il requisito si applica a chi annuncia lo
stream, e ammette per iscritto che il descrittore dichiari la materializzazione
limitata.

### Una sorgente senza geometrie, e una di cui non si sanno i tipi

Non sono lo stesso caso, e per un po' il prodotto le ha trattate uguali.

Quando la destinazione accetta solo alcuni dei sedici tipi geometrici canonici —
il sink Arrow IPC ne dichiara sette, perché trasporta WKB senza interpretarlo —
pretende di saperli in anticipo. Se il contratto della sorgente arriva senza
tipi, la scrittura viene rifiutata: è prudente, perché altrimenti la
scoprirebbe a scrittura iniziata.

«Senza tipi» però copriva due situazioni diverse. Una sorgente percorsa fino in
fondo e priva di geometrie — un GeoJSON con zero feature — non ha niente da
dichiarare, e pretendere da lei una dichiarazione preventiva rifiutava una
conversione che il formato può fare benissimo. Una sorgente di cui i tipi non si
sono potuti determinare è un'altra cosa, e lì il rifiuto serve.

Il contratto interno ora le distingue, e il rifiuto resta solo sulla seconda.

Sul **filo** la distinzione non passa, ed è bene saperlo:
`ARROW-VOCABULARY-1.0` è un vocabolario chiuso, e
`plenora.geometry.types_declaration` ammette i soli `exact`, `mixed`,
`unresolved`. Un'assenza accertata si scrive quindi `unresolved`, come
un'ignoranza. La conseguenza si vede: un GeoJSON vuoto si consegna in Arrow, e
quel file consegnato — riletto — viene rifiutato verso lo stesso sink, perché
chi lo rilegge non ha modo di sapere che l'assenza era stata accertata.
Scioglierlo richiede un vocabolario successore, che non è una decisione di
questo componente.

---

## LossReport — ratificato con il protocollo 2

Il contratto del report di perdita è **ratificato**, ed è il
[protocollo 2](../release/cli-protocol-v2.json). Le cinque decisioni che lo
tenevano aperto — struttura, limiti, redazione, comportamento al limite,
versionamento — sono chiuse, e ciascuna è applicata dal codice e verificata da
un gate.

Che cosa esce oggi, nel v2:

```json
"read_loss": {
  "lossless": false,
  "troncato": false,
  "omesse_esatte": true,
  "omesse": { "categorie_omesse": 0, "ragioni_omesse": 0,
              "esempi_omessi": 0, "omesse_per_byte": 0 },
  "counts": [ { "categoria": "coercion tipo attributo", "conteggio": 3 } ],
  "esempi": [ { "category": "coercion tipo attributo",
                "layer_index": 0, "field_index": 4, "type_class": "decimal",
                "context": "il tipo dell'attributo richiede una coercizione" } ]
}
```

### I limiti, e chi li decide

| Grandezza | Tetto |
|---|---|
| categorie in `counts` | **64** |
| byte di un identificatore di categoria | **128**, ovunque compaia |
| ragioni in `reasons` | **64** |
| esempi in `esempi` | **64** |
| byte di un `detail` o di un `context` | **512** |
| byte di una sezione | **12 KiB**, sulla serializzazione effettiva |
| byte della diagnostica di una busta | **64 KiB** |

Nessuno di questi numeri lo decide chi fornisce il file, ed è il cambiamento
che il v2 porta. Il budget non speso da una sezione **non** passa a un'altra:
con un consumo sequenziale una sezione grande affamerebbe quelle che la
seguono, e la stessa sezione produrrebbe un output diverso a seconda di quanto
ha occupato un'altra.

### Nessun nome preso dal file

`reasons[].detail` e `esempi[].context` portano testo **curato**: stabile,
descrittivo, scritto da noi. Dove si è persa una cosa lo dicono `layer_index` e
`field_index`, che sono indici e non nomi — e nemmeno un hash dei nomi, che
resterebbe un identificatore controllato da chi fornisce il file. Il tipo di un
attributo passa da `type_class`, un vocabolario chiuso nostro, mai dalla forma
`Debug` di un tipo di dipendenza.

L'unica eccezione è dichiarata: i **codici numerici** di un registro di
autorità CRS possono comparire in un `context`, perché senza di loro
un'incoerenza fra `crs_definition`, `crs_id` e `srid` non è leggibile. Le
stringhe libere che li accompagnano restano vietate.

### Il troncamento è sempre dichiarato

Quattro cause separate, perché «sono più di sessantaquattro» e «lo spazio è
finito» portano a decisioni diverse. I conteggi pubblicati restano esatti: si
omette una voce intera, mai si riscrive un numero. Se nemmeno la dichiarazione
di troncamento entra nel budget, la CLI **fallisce** invece di pubblicare una
sezione che tace.

`omesse_esatte` qualifica i quattro contatori: vale `false` quando una perdita
di esattezza interna li rende limiti inferiori.

### Canali della CLI

La CLI usa il protocollo 2. La risposta JSON, anche in caso di errore,
va su stdout; stderr resta vuoto. Il payload di un panico non viene esposto.

### Adozione locale contracts 7f82263

Il pin passa da `3aec9f7` a `7f82263` senza cambiare comportamento. La regola di
profilo IO-ARROW-001 copre l'accettazione della sorgente Arrow esterna (schema
senza `plenora.contract.version` e senza chiavi `plenora.`), e la deviazione
ARROW-001 / REJ-001 si ritira. I vettori arrow-data-v1 del pin, compresi quelli
della decisione 0022, sono 176 esiti conformi; nessuna esenzione IPC multi (8.0.0).

- **plenora-python-sdk-v1** (python_sdk): garanzia indebolita. Il catalogo
  dichiara la superficie `conditional` e IO spedisce la wheel, quindi la
  superficie si applica; il manifesto non la rivendica e la tiene
  `not_applicable` (lo schema v4 non ha uno stato intermedio). Mancano la wheel
  con `api_modes` fra gli artefatti e `plenora.surface_side_effects` con
  `python_sdk: local` per `io.read` (IO-PY-001). Hazard: nessuna dichiarazione
  di conformità dell'SDK nel manifesto. Rientro: rivendicare il contratto con
  l'artefatto wheel e le deviazioni di IO-PY-001.

### Adozione locale contracts 3aec9f7

Profilo io-tools v3, catalogo v3, Public Catalogs e Runtime Binding 2.0.
Wire runtime 1; catalog/read/write passano a versione operativa 2.
Gli schemi v1 restano immutati. Python SDK #32 non adottato.

- **IO-NULL-001** (cli): Regola 4; ambito risultati CLI/Rust/runtime: non e completata la qualifica del significato di ogni campo nullable. Gli schemi legacy e le prove di forma non dimostrano che null significhi sempre non applicabile e mai ignoto. Hazard: il consumatore del profilo v3 potrebbe interpretare come non applicabile un valore non qualificato. Non si afferma una violazione osservata per ogni null; la garanzia uniforme IO-NULL-001 non e rivendicata. Rientro: censire ogni campo nullable e rappresentare lo stato ignoto con un membro tipizzato o errore; registro docs/PRODUCT.md.
- **RT-024** (runtime): Regola 4; il flusso Arrow usa max_output_bytes della pipeline, senza successi troncati, ma manca un tetto uniforme per la materializzazione JSON e una prenotazione fallibile del Vec Arrow. Hazard: allocazione abortita invece del resource_limit tipizzato. Nessuna conformita completa a RT-024 e rivendicata. Rientro: budget uniforme e allocazioni fallibili prima della materializzazione, prove al limite con assi coerenti dopo pubblicazione. Limiti di trasporto a carico host; docs/PRODUCT.md.
- **RUNTIME-VECTORS-1.0 sezione 5 (versioni operative storiche)** (runtime): Regola 4; i vettori io-read-v1 fissano io.read@1 (il registro runtime-v1 include anche @2); il profilo v3 serve io.read@2. Gli originali restano immutati: @1 e rifiutato unsupported; i test applicano esplicitamente @2 e il contratto risultato v2 ai casi di esecuzione. Hazard: copiare il selettore storico impedisce la lettura. Rientro: vettori successori del catalogo v3 in contracts; prove runtime_vettori e docs/PRODUCT.md.

Il passaggio dal profilo 1 cambia i contratti di risultato: richiede una release
maggiore prima della distribuzione. Questa modifica del worktree non costituisce
una release e non modifica la versione del pacchetto. La qualifica semantica di
IO-NULL-001 resta aperta; i test degli schemi verificano la forma, non quel significato.

### GeoPackage: storage dinamico e schema SQL

La lettura distingue i NULL originali dagli storage incompatibili scartati
(`gpkg_incompatible_storage_discarded`) e dal testo UTF-8 riparato
(`gpkg_invalid_utf8_replaced`). Le perdite contano le celle interessate; gli
esempi sono limitati per campo e categoria, senza contenuto di riga.

La PK INTEGER entra nello schema Arrow come attributo non nullable con
`gpkg.primary_key=true`; la riscrittura GeoPackage la usa come PK e ne conserva
i valori. `gpkg.declared_type` porta BOOLEAN, TINYINT, SMALLINT, MEDIUMINT, INT,
INTEGER, FLOAT, DOUBLE, REAL, TEXT, BLOB, DATE o DATETIME. Il writer rifiuta un
metadato non riconosciuto o incompatibile con il tipo Arrow prima di creare il
file. BOOLEAN e' Int64 nel batch, DATE/DATETIME sono Utf8: i metadati portano la
distinzione SQL. Le dichiarazioni esterne non riconosciute seguono il mapping
storico per famiglie (INT, REAL/FLOA/DOUB, BLOB, altrimenti Utf8) con perdita
`gpkg_schema_normalized`. I vincoli NOT NULL sono conservati; default, ulteriori
vincoli SQL, indici e trigger non vengono ricostruiti e sono dichiarati nella
stessa categoria, con la ragione effettiva. Anche l'ordine delle colonne viene
segnalato quando cambia: geometria prima, PK ultima. Le colonne generate
restano valori ordinari, con perdita dichiarata della regola SQL. Un contratto
senza PK esplicita riceve una chiave generata.

La matrice di regressione enumera NULL, INTEGER, REAL, TEXT e BLOB contro i 13
tipi SQL riconosciuti e sei rappresentanti delle famiglie esterne normalizzate:
95 coppie sul convertitore, piu' le stesse fixture su SQLite reale, dove
l'affinita' puo' cambiare la classe prima della lettura. Un layer che ammette
Point e LineString dichiara GEOMETRY nel catalogo; il limite sulle colonne
geometriche multiple resta distinto.

### Domini temporali dei lettori

DXF rifiuta date fuori dagli anni 0001–9999, giorno giuliano zero, valori non
finiti e durate negative. Le durate di editing sono giorni; le date vengono
arrotondate al secondo, le durate ai nanosecondi dopo la conversione binary64
in secondi. I secondi SUNSTUDY devono appartenere al giorno indicato.
GeoParquet rifiuta il tipo fisico legacy INT96 nell'intero file, anche quando
una proiezione escluderebbe la colonna. Usare timestamp INT64 con unita'
esplicita per file da scambiare con IO. Arrow IPC conserva i tick interi,
anche estremi, senza convertirli implicitamente in date calendariali.

Gli XLSX elencano anche chartsheet e worksheet vuoti, rifiutati rispettivamente
con `object_chart_sheet` e `object_empty`. `xlsx.visibility` distingue visible,
hidden e very_hidden. `sheet` distingue maiuscole e minuscole; un nome inesistente
e' configurazione non valida. I fogli nascosti sono leggibili con selezione esplicita.

KML conserva i Placemark con Model, Track, MultiTrack o estensione gx sconosciuta
con geometria nulla; conta le geometrie omesse in `kml.unsupported_geometry`.
Il documento resta leggibile; le perdite native sono dettagliate sotto.
DXF espone anche i blocchi separati:
un blocco esterno o non rappresentabile ha un rifiuto locale.

GPKG `srs_id` 0 e -1 sono righe obbligatorie e valide della specifica OGC
(requisito 11), rispettivamente geografico e cartesiano indefiniti. Non sono
identificatori risolti: il driver legge questi layer con CRS assente;
`--assume-crs` permette di nominarlo. Non si assegna implicitamente EPSG:4326.

### Trasporto nativo GeoJSON e KML

La tabella comune `plenora-io-core::loss_table` dichiara, per lettura e scrittura,
conservazione, trasformazione con categoria o rifiuto. Le classi Arrow sono
raffinate dai tipi stabili della stessa tabella: ad esempio Int64 GeoJSON resta
Int64, mentre Int8 richiede `coercion tipo attributo`. Le righe sui valori
conservati valgono per i campi effettivamente rappresentati; KML conserva
`name` e `description`, mentre gli altri attributi scritti in ExtendedData
non vengono letti e sono dichiarati in entrambe le direzioni. Se mancano nello
schema sorgente, le colonne standard name/description aggiunte in rilettura
sono dichiarate con `native_fields_reconstructed`.

GeoJSON dichiara ID di feature, bbox, membri esterni di collezioni, feature e
geometrie, incluse quelle annidate. L'inferenza dichiara valori convertiti a
un tipo comune, oggetti/array riscritti come testo JSON, proprietà assenti rese
null e riordino alfabetico. Un null esplicito resta null. NOT NULL e il nome
non canonico della geometria non sopravvivono al giro e hanno categorie proprie.

KML dichiara ExtendedData/SchemaData, ID, metadati temporali e visibility,
stili, metadati di Document/Folder, gerarchie appiattite, NetworkLink e
GroundOverlay non seguiti. Le osservazioni dei discendenti entrano nel report
dell'aggregato e dei loro contenitori; selezionare un Placemark non include le
perdite di un altro Placemark. LinearRing autonomo e MultiGeometry dichiarano
la ricostruzione del tipo geometrico. Una quarta ordinata è rifiutata.

`kml_altitude_mode_not_preserved` indica quote Z con modalità implicita o
non assoluta: i numeri sono restituiti senza conversione e **non rappresentano
quote assolute**. Il report lo esplicita. Il writer continua a emettere
`absolute` per le geometrie Z. Le geometrie vuote non rappresentabili sono
rifiutate; una geometria null resta null.

I conteggi sono strutturali, non misure dell'errore numerico:

- `native_number_precision`: valori numerici il cui decimale ricostruito
  differisce dal testo sorgente; numeri esatti e grafie equivalenti non contano.
- `native_dimensions_not_declared`: una dichiarazione dimensionale sorgente
  presente che differisce da quella riletta. Una sorgente senza dichiarazione
  non perde una dichiarazione; una ricostruzione identica non produce perdita.
- `native_type_inference`: valori scalari convertiti al tipo comune;
  `native_structured_value_rewritten`: valori oggetto/array riscritti;
  `native_missing_as_null`: celle assenti, oppure elementi testuali KML
  autochiusi, resi null. Conta inoltre i contenitori properties null/assenti
  e i membri geometry assenti normalizzati alla rappresentazione tabellare.
- `native_field_order_changed`: in scrittura, campi spostati nel giro; in
  lettura GeoJSON, inversioni adiacenti osservate nelle proprietà, oppure una
  normalizzazione dell'ordine di prima comparsa se non vi erano inversioni.
- Le omissioni native contano i membri o elementi presenti; ExtendedData conta
  i contenitori letti e i campi persi nel giro in scrittura. Le dichiarazioni
  di vincoli, nomi e metadati dello schema contano campi o chiavi.

Gli esempi riportano indici di layer/campo, mai nomi, ID, URI o valori del file.
Gli esempi restano limitati; i conteggi sono aggregati. I metadati Arrow e le
rappresentazioni CRS continuano a seguire i piani comuni per chiave.

GeoPackage conserva `gpkg_contents.identifier` e `description` nelle chiavi
Arrow `gpkg.identifier` e `gpkg.description` e le riscrive. Il writer dispone
le colonne nell'ordine del reader (geometria, attributi, chiave primaria):
una permutazione dell'ingresso è dichiarata in scrittura; la rilettura non
segnala una nuova perdita. Il documento interno di identità validato e
consumato dal reader non è metadato scartato; gli altri metadati restano
soggetti alla tabella comune. SRS 0 e -1 indicano CRS assente, senza SRID Arrow
e senza assumere EPSG:4326; `--assume-crs` permette di dichiarare il CRS
quando richiesto dalla destinazione.

### Perdite native Shapefile, DXF e FileGDB

Le celle di lettura e scrittura di questi driver sono nella stessa tabella
`loss_table` usata da GeoJSON e KML. Le classi Arrow e la nullabilità del catalogo
derivano dalle celle; i limiti su valori e sottotipi geometrici restano quelli
delle capability e dei validatori del formato. Una cella di classe non rende
scrivibile ogni valore: per esempio FileGDB rifiuta LineString e Polygon singoli
e accetta i corrispondenti multi, mentre Shapefile promuove i singoli.

Shapefile conta i campi effettivamente spostati dal piano di scrittura.
La lettura conserva l'ordine DBF e non dichiara il solo assemblaggio SHP/DBF.
`native_geometry_type_changed` conta le geometrie singole promosse in scrittura;
non conta come perdita la lettura di geometrie già multi.
`native_ring_orientation_changed` conta gli anelli il cui orientamento cambia. Un campo
DBF D letto come Utf8 produce `coercion tipo attributo`, con indice del campo:
il valore testuale della data resta quello restituito dal reader. Il Point vuoto
NaN si conserva; le collezioni vuote non rappresentabili sono rifiutate.

DXF dichiara handle, colore, linetype, lineweight, thickness, larghezze, XDATA,
attributi grafici TEXT/MTEXT e paper space. Le categorie sono rispettivamente
`native_feature_id_not_preserved`, `native_styles_not_preserved`,
`native_layer_metadata_not_preserved` e `native_hierarchy_flattened`; il dettaglio
curato identifica l'elemento, senza riportarne il valore. Si contano gli elementi
non predefiniti osservati, i contenitori XDATA e le entità testuali/paper space.
Il testo TEXT resta una colonna; altezza, rotazione e stile non sono conservati.
Le entità paper space rimangono nel medesimo oggetto del model space, con perdita
esplicita della distinzione. Un blocco XREF o il model space che lo usa tramite
INSERT è rifiutato localmente: nessun riferimento esterno viene seguito.
`dxf_attribute_not_represented` conta le differenze degli attributi rispetto alla rilettura:
celle diverse o mancanti e celle delle righe aggiunte, anche nelle colonne conservate;
gli esempi ne riportano l'indice, mai il nome nella categoria o nel dettaglio.

FileGDB dichiara l'ordine geometria-prima e i vincoli NOT NULL non trasportati,
per campo. La lettura esamina anche la definizione XML nativa ottenuta da GDAL:
alias e riferimenti a domini non vuoti, e sottotipi, sono contati come
`native_layer_metadata_not_preserved`. Le definizioni non diventano metadati
Arrow; i loro nomi e contenuti non entrano nel report. Le categorie restano
separate dalla lettura degli attributi Binary, Date e DateTime: Binary diventa
Arrow Binary; OFTDate diventa Date32; OFTDateTime diventa Timestamp in microsecondi.
Conta il tipo effettivo restituito da OGR: un Date FileGDB esposto da GDAL come
DateTime resta un timestamp. Il fuso UTC o l'offset esplicito fornito da OGR
entra nel tipo Arrow; sconosciuto e locale non diventano UTC. Per GDAL 3.9,
che lascia sconosciuto il flag di schema, viene letta la dichiarazione nativa
IsTimeInUTC. Il flag originale di schema e' conservato nei metadati del campo.
Fusi misti non rappresentabili in un singolo tipo Arrow sono rifiutati.
I secondi frazionari restituiti da OGR sono arrotondati ai microsecondi:
`native_number_precision` conta solo le celle effettivamente modificate.
Non si ricostruiscono frazioni che il driver GDAL non fornisce. Null e Binary
non passano attraverso conversioni testuali. In scrittura Binary e tipi
temporali restano rifiutati prima della creazione, coerentemente con il catalogo.
Le geometrie lette usano ISO WKB anche per Z, M e ZM.

`native_coordinate_precision` conta una geometria non null sottoposta al
renderer DXF o alla griglia FileGDB, non cifre perse o errore misurato; comprende
la normalizzazione di -0. FileGDB conta una geometria poligonale per la
normalizzazione degli anelli e una geometria vuota per `native_missing_as_null`.
I conteggi dichiarano il processo nativo anche quando un valore è già esatto.
NOT NULL e ordine sono dichiarati anche per layer senza righe. Tutti gli esempi
usano indici di layer/campo; il limite degli esempi non cambia.


### Perdite native CSV, XLSX, GeoPackage, GeoParquet e IPC

La matrice `loss_table` comprende tutti i dieci driver e le due direzioni.
L'inventario indipendente dei test richiede una riga per ogni elemento e un
driver qualificato per ogni descrittore. Le classi `allowed_types` vengono
calcolate dalle celle di scrittura; le dimensioni dalle celle M e dimensione
sconosciuta. GeoParquet accetta XY/XYZ/unknown e rifiuta M.

CSV e XLSX dichiarano il nome WKT ricostruito come `geometry`, il consumo
delle colonne X/Y e il riordino. I writer collocano la geometria dopo gli
attributi; i reader la espongono prima. In scrittura il conteggio è quello
dei campi spostati; in lettura è una ricostruzione dell'ordine quando la
prima colonna geometrica non era la prima colonna nativa. Il consumo X/Y
conta due colonne, la rinomina WKT una colonna. Il report usa indici delle
colonne sorgenti, senza nomi. La precisione X/Y conta solo valori decimali ricostruiti
differenti dal testo sorgente, ignorando zeri ed esponenti equivalenti.
CSV dichiara inoltre l'inferenza dei campi numerici e booleani.

XLSX dichiara i formati numerici per cella con formato diverso da General e
le formule per elemento formula. Restituisce il valore in cache senza
ricalcolarlo: la dichiarazione non promette che la cache sia aggiornata.
La precisione numerica conta valori effettivamente cambiati rispetto al testo XML;
stringhe, null e stringhe vuote seguono le regole di trasporto già descritte.
Le osservazioni dei fogli conservano l'indice del foglio selezionato.

GeoPackage dichiara identifier e description non vuoti, righe di
`gpkg_data_columns`, `layer_styles`, `gpkg_extensions` e
`gpkg_metadata_reference` riferite al layer (incluse le riferite all'intero
contenitore). Sono conteggi di membri/contenitori omessi, non delle loro
singole stringhe. Identifier coincidente col nome della tabella non costituisce perdita.
I metadati di identità del writer consumati nel contratto non sono omessi. Il writer genera `INTEGER PRIMARY KEY` senza AUTOINCREMENT:
la politica AUTOINCREMENT di un file esterno resta una normalizzazione SQL
dichiarata dal reader, mentre non viene introdotta dal writer.

GeoParquet confronta il covering con quello ricostruito dalla geometria e
dichiara solo le colonne radice differenti,
con indice fisico della colonna. La colonna bbox generata in scrittura è un
accessorio derivato, non un attributo sorgente perduto. La normalizzazione
dei metadati viene confrontata con lo schema finale del footer, dopo il
completamento dei tipi e delle dimensioni osservati nei batch.

La chiave `geo` ricostruibile dai metadati nativi del contratto non è persa.
IPC e GeoParquet confrontano i metadati originali con quelli esposti o
scritti: ogni chiave sorgente rimossa o cambiata produce
`arrow_metadata_not_preserved`, con indice di campo quando pertinente.
Versione del contratto, identità, dichiarazione dei tipi, encoding,
semantica spaziale, precisione, SRID, risoluzione CRS, formato della
definizione, ordine degli assi, primaria ed estensioni hanno celle esplicite.
Una chiave invariata non genera una perdita; le annotazioni nuove del
contratto non sono contate come rimozioni di dati sorgenti. Le chiavi opache
conservate non vengono esposte nei dettagli del report.

### Lettura di un prefisso con `--limit`

`read --limit N` arresta il decoder alla riga N, includendo nel limite le
passate di inferenza. N deve essere positivo. `rows_read` non supera N.
`truncated: true` significa «limite raggiunto, coda non esaminata», anche se
il file contiene esattamente N righe. `truncated: false` significa che la
lettura ha raggiunto EOF prima del limite (oppure senza un limite).
Lo schema inferito, le osservazioni di fedeltà e la diagnostica riguardano
il prefisso osservato. La coda non è attestata. Nell'SDK Python `complete`
è `not truncated`: è vero soltanto quando EOF è stato osservato.

CSV, GeoJSON, KML e DXF interrompono la scansione progressiva; XLSX prepara
un prefisso XML dei fogli prima dell'inferenza e della decodifica delle celle.
Shapefile limita anche la prevalidazione dei record e l'inferenza DBF.
GeoPackage limita la query e FileGDB l'iteratore delle feature.
Il limite è trasmesso ai decoder, non applicato soltanto al batch restituito.
Parquet deve acquisire pagine/chunk del row group che contiene N; Arrow IPC
acquisisce l'intero record batch che contiene N. I controlli strutturali
necessari alla decodifica sicura del blocco restano applicati al blocco;
la validazione dei valori e delle geometrie riguarda soltanto le prime N righe.
I blocchi successivi non vengono decodificati. Cataloghi, footer, dizionari e
metadati condivisi possono richiedere letture indipendenti dalle righe; il
buffering del sistema o della decompressione può acquisire byte anticipati.
`--limit` non sostituisce le quote di risorse e non è ammesso con `--output`.

I tipi Arrow `LargeUtf8`/`Utf8View`, `LargeBinary`/`BinaryView` e
`LargeList`/`ListView`/`LargeListView` hanno rispettivamente le classi pubbliche
`utf8`, `binary` e `nested`, come i tipi base. Un writer che non li rappresenta
usa la situazione `UnrepresentableWriteType`, con un messaggio che nomina la
classe, senza riportare valori della cella.


## Tabelle operative verificate

<!-- generato da codice/uscite: inizio -->

| Categoria | Exit |
| --- | --- |
| successo | 0 |
| invalid_plan | 2 |
| invalid_configuration | 2 |
| schema | 3 |
| data_mapping | 3 |
| crs | 3 |
| unsupported | 3 |
| resource_limit | 4 |
| io | 5 |
| not_found | 5 |
| conflict | 5 |
| protocol | 5 |
| authentication | 5 |
| authorization | 5 |
| timeout | 5 |
| transient | 5 |
| execution | 6 |
| internal | 70 |
| cancelled | 130 |

<!-- generato da codice/uscite: fine -->

<!-- generato da codice/errori: inizio -->

| Situazione | Driver | Caso eseguito | Categoria | Fase | Codice | Exit | Messaggio |
| --- | --- | --- | --- | --- | --- | --- | --- |
| MissingSource | csv | sorgente assente | not_found | read | SOURCE_NOT_FOUND | 5 | sorgente o membro obbligatorio del dataset non trovato |
| MissingSource | shp | sorgente assente | not_found | read | SOURCE_NOT_FOUND | 5 | sorgente o membro obbligatorio del dataset non trovato |
| MissingSource | gpkg | sorgente assente | not_found | read | SOURCE_NOT_FOUND | 5 | sorgente o membro obbligatorio del dataset non trovato |
| MissingSource | filegdb | sorgente assente | not_found | read | SOURCE_NOT_FOUND | 5 | sorgente o membro obbligatorio del dataset non trovato |
| MissingSource | geojson | sorgente assente | not_found | read | SOURCE_NOT_FOUND | 5 | sorgente o membro obbligatorio del dataset non trovato |
| MissingSource | kml | sorgente assente | not_found | read | SOURCE_NOT_FOUND | 5 | sorgente o membro obbligatorio del dataset non trovato |
| MissingSource | dxf | sorgente assente | not_found | read | SOURCE_NOT_FOUND | 5 | sorgente o membro obbligatorio del dataset non trovato |
| MissingSource | xls | sorgente assente | not_found | read | SOURCE_NOT_FOUND | 5 | sorgente o membro obbligatorio del dataset non trovato |
| MissingSource | geoparquet | sorgente assente | not_found | read | SOURCE_NOT_FOUND | 5 | sorgente o membro obbligatorio del dataset non trovato |
| MissingSource | ipc | sorgente assente | not_found | read | SOURCE_NOT_FOUND | 5 | sorgente o membro obbligatorio del dataset non trovato |
| InvalidContent | csv | contenitore vuoto | data_mapping | read | FORMAT_ERROR | 3 | contenuto vuoto o struttura non valida; formato atteso: CSV |
| InvalidContent | shp | contenitore vuoto | data_mapping | read | FORMAT_ERROR | 3 | contenuto vuoto o struttura non valida; formato atteso: Shapefile |
| InvalidContent | gpkg | contenitore vuoto | data_mapping | read | FORMAT_ERROR | 3 | contenuto vuoto o struttura non valida; formato atteso: GeoPackage |
| InvalidContent | filegdb | contenitore vuoto | data_mapping | read | FORMAT_ERROR | 3 | contenuto vuoto o struttura non valida; formato atteso: FileGDB |
| InvalidContent | geojson | contenitore vuoto | data_mapping | read | FORMAT_ERROR | 3 | contenuto vuoto o struttura non valida; formato atteso: GeoJSON |
| InvalidContent | kml | contenitore vuoto | data_mapping | read | FORMAT_ERROR | 3 | contenuto vuoto o struttura non valida; formato atteso: KML |
| InvalidContent | dxf | contenitore vuoto | data_mapping | read | FORMAT_ERROR | 3 | contenuto vuoto o struttura non valida; formato atteso: DXF |
| InvalidContent | xls | contenitore vuoto | data_mapping | read | FORMAT_ERROR | 3 | contenuto vuoto o struttura non valida; formato atteso: XLSX |
| InvalidContent | geoparquet | contenitore vuoto | data_mapping | read | FORMAT_ERROR | 3 | contenuto vuoto o struttura non valida; formato atteso: Parquet |
| InvalidContent | ipc | contenitore vuoto | data_mapping | read | FORMAT_ERROR | 3 | flusso Arrow IPC troppo corto per il formato |
| DirectoryInsteadOfFile | csv | directory al posto del file | invalid_configuration | validate | CLI_USAGE | 2 | directory al posto del file; formato atteso: CSV |
| DirectoryInsteadOfFile | shp | directory al posto del file | invalid_configuration | validate | CLI_USAGE | 2 | directory al posto del file; formato atteso: Shapefile |
| DirectoryInsteadOfFile | gpkg | directory al posto del file | invalid_configuration | validate | CLI_USAGE | 2 | directory al posto del file; formato atteso: GeoPackage |
| DirectoryInsteadOfFile | geojson | directory al posto del file | invalid_configuration | validate | CLI_USAGE | 2 | directory al posto del file; formato atteso: GeoJSON |
| DirectoryInsteadOfFile | kml | directory al posto del file | invalid_configuration | validate | CLI_USAGE | 2 | directory al posto del file; formato atteso: KML |
| DirectoryInsteadOfFile | dxf | directory al posto del file | invalid_configuration | validate | CLI_USAGE | 2 | directory al posto del file; formato atteso: DXF |
| DirectoryInsteadOfFile | xls | directory al posto del file | invalid_configuration | validate | CLI_USAGE | 2 | directory al posto del file; formato atteso: XLSX |
| DirectoryInsteadOfFile | geoparquet | directory al posto del file | invalid_configuration | validate | CLI_USAGE | 2 | directory al posto del file; formato atteso: Parquet |
| DirectoryInsteadOfFile | ipc | directory al posto del file | invalid_configuration | validate | CLI_USAGE | 2 | directory al posto del file; formato atteso: Arrow IPC |
| InvalidConfiguration | csv | WKT e XY insieme | invalid_configuration | validate | CLI_USAGE | 2 | csv: wkt_column esclude x_column e y_column; dichiarare una sola geometria |
| MissingOptionColumn | csv | colonna WKT assente | invalid_configuration | validate | CLI_USAGE | 2 | colonna WKT assente dall'intestazione |
| ContradictedCrs | geojson | assunzione EPSG:3857 su WGS84 | crs | validate | CRS_CONFLICT | 3 | il CRS assunto contraddice quello dichiarato |
| ContradictedCrs | kml | assunzione EPSG:3857 su WGS84 | crs | validate | CRS_CONFLICT | 3 | il CRS assunto contraddice quello dichiarato |
| ContradictedCrs | shp | assunzione EPSG:3857 su WGS84 | crs | validate | CRS_CONFLICT | 3 | il CRS assunto contraddice quello dichiarato |
| ContradictedCrs | dxf | assunzione EPSG:3857 su WGS84 | crs | validate | CRS_CONFLICT | 3 | il CRS assunto contraddice quello dichiarato |
| ContradictedCrs | gpkg | assunzione EPSG:3857 su WGS84 | crs | validate | CRS_CONFLICT | 3 | il CRS assunto contraddice quello dichiarato |
| ContradictedCrs | geoparquet | assunzione EPSG:3857 su WGS84 | crs | validate | CRS_CONFLICT | 3 | il CRS assunto contraddice quello dichiarato |
| ContradictedCrs | ipc | assunzione EPSG:3857 su WGS84 | crs | validate | CRS_CONFLICT | 3 | il CRS assunto contraddice quello dichiarato |
| MissingGeometry | shp | schema senza geometria | unsupported | write | UNSUPPORTED | 3 | il formato di destinazione richiede una geometria |
| MissingGeometry | kml | schema senza geometria | unsupported | write | UNSUPPORTED | 3 | il formato di destinazione richiede una geometria |
| MissingGeometry | dxf | schema senza geometria | unsupported | write | UNSUPPORTED | 3 | il formato di destinazione richiede una geometria |
| UnrepresentableWriteType | csv | attributo Duration | unsupported | write | UNSUPPORTED | 3 | tipo Arrow non rappresentabile in fase write, classe: temporal |
| UnrepresentableWriteType | shp | attributo Duration | unsupported | write | UNSUPPORTED | 3 | tipo Arrow non rappresentabile in fase write, classe: temporal |
| UnrepresentableWriteType | kml | attributo Duration | unsupported | write | UNSUPPORTED | 3 | tipo Arrow non rappresentabile in fase write, classe: temporal |
| UnrepresentableWriteType | geojson | attributo Duration | unsupported | write | UNSUPPORTED | 3 | tipo Arrow non rappresentabile in fase write, classe: temporal |
| UnrepresentableWriteType | gpkg | attributo Duration | unsupported | write | UNSUPPORTED | 3 | tipo Arrow non rappresentabile in fase write, classe: temporal |
| UnrepresentableWriteType | xls | attributo Duration | unsupported | write | UNSUPPORTED | 3 | tipo Arrow non rappresentabile in fase write, classe: temporal |
| UnrepresentableWriteValue | gpkg | UInt64 oltre INTEGER | data_mapping | write | FORMAT_ERROR | 3 | UInt64 oltre i64::MAX non rappresentabile come INTEGER GeoPackage |
| NonFiniteWriteValue | geojson | attributo NaN | data_mapping | write | FORMAT_ERROR | 3 | Float64 non finito non rappresentabile in GeoJSON |
| ReadMemoryExceeded | ipc | memoria 1 byte | resource_limit | read | LIMIT_EXCEEDED | 4 | memory_bytes quota superata |
| IneffectiveCrsAssumption | ipc | assunzione senza geometria | invalid_configuration | validate | CLI_USAGE | 2 | --assume-crs senza effetto: il layer non contiene geometrie |
| MissingLayer | csv | layer 999 | not_found | prepare | NO_LAYER | 5 | layer inesistente all'indice 999 |
| MissingLayer | shp | layer 999 | not_found | prepare | NO_LAYER | 5 | layer inesistente all'indice 999 |
| MissingLayer | kml | layer 999 | not_found | prepare | NO_LAYER | 5 | layer inesistente all'indice 999 |
| MissingLayer | dxf | layer 999 | not_found | prepare | NO_LAYER | 5 | layer inesistente all'indice 999 |
| MissingLayer | gpkg | layer 999 | not_found | prepare | NO_LAYER | 5 | layer inesistente all'indice 999 |
| MissingLayer | geojson | layer 999 | not_found | prepare | NO_LAYER | 5 | layer inesistente all'indice 999 |
| MissingLayer | geoparquet | layer 999 | not_found | prepare | NO_LAYER | 5 | layer inesistente all'indice 999 |
| MissingLayer | ipc | layer 999 | not_found | prepare | NO_LAYER | 5 | layer inesistente all'indice 999 |
| MissingLayer | xls | layer 999 | not_found | prepare | NO_LAYER | 5 | layer inesistente all'indice 999 |
| InvalidQuota | csv | max_rows zero | invalid_configuration | validate | CLI_USAGE | 2 | max_rows deve essere un intero positivo e coerente con le altre quote |
| InvalidQuota | shp | max_rows zero | invalid_configuration | validate | CLI_USAGE | 2 | max_rows deve essere un intero positivo e coerente con le altre quote |
| InvalidQuota | gpkg | max_rows zero | invalid_configuration | validate | CLI_USAGE | 2 | max_rows deve essere un intero positivo e coerente con le altre quote |
| InvalidQuota | filegdb | max_rows zero | invalid_configuration | validate | CLI_USAGE | 2 | max_rows deve essere un intero positivo e coerente con le altre quote |
| InvalidQuota | geojson | max_rows zero | invalid_configuration | validate | CLI_USAGE | 2 | max_rows deve essere un intero positivo e coerente con le altre quote |
| InvalidQuota | kml | max_rows zero | invalid_configuration | validate | CLI_USAGE | 2 | max_rows deve essere un intero positivo e coerente con le altre quote |
| InvalidQuota | dxf | max_rows zero | invalid_configuration | validate | CLI_USAGE | 2 | max_rows deve essere un intero positivo e coerente con le altre quote |
| InvalidQuota | xls | max_rows zero | invalid_configuration | validate | CLI_USAGE | 2 | max_rows deve essere un intero positivo e coerente con le altre quote |
| InvalidQuota | geoparquet | max_rows zero | invalid_configuration | validate | CLI_USAGE | 2 | max_rows deve essere un intero positivo e coerente con le altre quote |
| InvalidQuota | ipc | max_rows zero | invalid_configuration | validate | CLI_USAGE | 2 | max_rows deve essere un intero positivo e coerente con le altre quote |
| InvalidSpillDirectory | csv | directory spill assente | invalid_configuration | validate | CLI_USAGE | 2 | PLENORA_SPILL_DIR non indica una directory utilizzabile |
| InvalidSpillDirectory | shp | directory spill assente | invalid_configuration | validate | CLI_USAGE | 2 | PLENORA_SPILL_DIR non indica una directory utilizzabile |
| InvalidSpillDirectory | gpkg | directory spill assente | invalid_configuration | validate | CLI_USAGE | 2 | PLENORA_SPILL_DIR non indica una directory utilizzabile |
| InvalidSpillDirectory | geojson | directory spill assente | invalid_configuration | validate | CLI_USAGE | 2 | PLENORA_SPILL_DIR non indica una directory utilizzabile |
| InvalidSpillDirectory | kml | directory spill assente | invalid_configuration | validate | CLI_USAGE | 2 | PLENORA_SPILL_DIR non indica una directory utilizzabile |
| InvalidSpillDirectory | dxf | directory spill assente | invalid_configuration | validate | CLI_USAGE | 2 | PLENORA_SPILL_DIR non indica una directory utilizzabile |
| InvalidSpillDirectory | xls | directory spill assente | invalid_configuration | validate | CLI_USAGE | 2 | PLENORA_SPILL_DIR non indica una directory utilizzabile |
| InvalidSpillDirectory | geoparquet | directory spill assente | invalid_configuration | validate | CLI_USAGE | 2 | PLENORA_SPILL_DIR non indica una directory utilizzabile |
| InvalidSpillDirectory | ipc | directory spill assente | invalid_configuration | validate | CLI_USAGE | 2 | PLENORA_SPILL_DIR non indica una directory utilizzabile |
| MissingWriteBackend | filegdb | build senza gdal-backend | unsupported | write | UNSUPPORTED | 3 | scrittura FileGDB non disponibile: compilare con --features gdal-backend |

<!-- generato da codice/errori: fine -->

<!-- generato da codice/perdite: inizio -->

Regole per elemento: Preserved conserva, Rejected rifiuta, Transformed dichiara la trasformazione indicata. NotApplicable indica un elemento non esprimibile nella grammatica sorgente, con motivo e riferimento: non e' una prova passata. La scrittura riguarda la rappresentazione Arrow dell'elemento, inclusi i metadati nativi di formati diversi; non richiede un oggetto nativo del formato destinazione.

| Driver | Elemento | Lettura | Scrittura |
| --- | --- | --- | --- |
| csv | NullGeometry | Preserved | Preserved |
| csv | EmptyPoint | Rejected | Rejected |
| csv | SignedZero | Preserved | Preserved |
| csv | Subtypes | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | Aliases | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | Domains | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | TextStyle | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | PaperSpace | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | Xdata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | Width | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | Thickness | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | Lineweight | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | Linetype | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | Color | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | Handle | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | DbfDate | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Preserved |
| csv | Utf8 | Preserved | Preserved |
| csv | Null | Preserved | Preserved |
| csv | EmptyString | Preserved | Preserved |
| csv | SignedInteger | Transformed(Inference) | Transformed(ArrowCoercion) |
| csv | UnsignedInteger | Transformed(Inference) | Transformed(ArrowCoercion) |
| csv | Floating | Transformed(Inference) | Transformed(ArrowCoercion) |
| csv | Boolean | Preserved | Transformed(ArrowCoercion) |
| csv | Binary | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Rejected |
| csv | Decimal | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Rejected |
| csv | Temporal | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowCoercion) |
| csv | NestedDictionary | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Rejected |
| csv | FieldNames | Transformed(FieldNames) | Transformed(FieldNames) |
| csv | NotNull | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(NotNull) |
| csv | FieldOrder | Transformed(FieldOrder) | Transformed(FieldOrder) |
| csv | SchemaMetadata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | FieldMetadata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | NativeMetadata | Preserved | Transformed(ArrowMetadata) |
| csv | Z | Preserved | Preserved |
| csv | M | Preserved | Preserved |
| csv | Dimensions | Preserved | Preserved |
| csv | GeometryType | Preserved | Preserved |
| csv | RingOrientation | Preserved | Preserved |
| csv | EmptyGeometry | Preserved | Preserved |
| csv | CoordinatePrecision | Transformed(NumberPrecision) | Preserved |
| csv | CrsId | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(CrsIdAbsent) |
| csv | CrsDefinition | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(CrsDefinition) |
| csv | FeatureId | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | Bbox | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | ForeignMembers | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | LayerMetadata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | Styles | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | Hierarchy | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | Extensions | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | ExternalReference | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | ExtendedData | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | FeatureMetadata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | AltitudeMode | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | Inference | Transformed(Inference) | Transformed(ArrowCoercion) |
| csv | NumberPrecision | Transformed(NumberPrecision) | Preserved |
| csv | StructuredValue | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Preserved |
| csv | MissingValue | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Preserved |
| csv | CoordinateColumns | Transformed(FieldNames) | Transformed(ArrowMetadata) |
| csv | NumericFormat | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | Formula | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | FormulaCache | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowCoercion) |
| csv | Identifier | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | Description | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | DataColumns | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | SqlConstraints | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | ContractVersion | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | FieldIdentity | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | TypesDeclaration | Preserved | Transformed(ArrowMetadata) |
| csv | GeometryEncoding | Preserved | Transformed(ArrowMetadata) |
| csv | SpatialSemantics | Preserved | Transformed(ArrowMetadata) |
| csv | Precision | Preserved | Transformed(ArrowMetadata) |
| csv | Srid | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | CrsResolution | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | DefinitionFormat | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | AxisOrder | Preserved | Transformed(ArrowMetadata) |
| csv | PrimaryGeometry | Preserved | Transformed(ArrowMetadata) |
| csv | ExtensionName | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | ExtensionMetadata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| csv | UnknownDimensions | Preserved | Preserved |
| csv | Covering | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 4180 section 2 (records and text fields; no schema metadata)) | Transformed(ArrowMetadata) |
| shp | NullGeometry | Preserved | Preserved |
| shp | EmptyPoint | Preserved | Preserved |
| shp | SignedZero | Preserved | Preserved |
| shp | Subtypes | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | Aliases | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | Domains | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | TextStyle | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | PaperSpace | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | Xdata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | Width | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | Thickness | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | Lineweight | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | Linetype | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | Color | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | Handle | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | DbfDate | Transformed(ArrowCoercion) | Preserved |
| shp | Utf8 | Preserved | Preserved |
| shp | Null | Preserved | Preserved |
| shp | EmptyString | Rejected | Rejected |
| shp | SignedInteger | Transformed(Inference) | Transformed(ArrowCoercion) |
| shp | UnsignedInteger | NotApplicable: Native integer domains are signed; positive values do not declare an unsigned type (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowCoercion) |
| shp | Floating | Preserved | Transformed(ArrowCoercion) |
| shp | Boolean | Preserved | Preserved |
| shp | Binary | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Rejected |
| shp | Decimal | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Rejected |
| shp | Temporal | Transformed(ArrowCoercion) | Transformed(ArrowCoercion) |
| shp | NestedDictionary | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Rejected |
| shp | FieldNames | Preserved | Transformed(FieldNames) |
| shp | NotNull | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(NotNull) |
| shp | FieldOrder | Transformed(FieldOrder) | Transformed(FieldOrder) |
| shp | SchemaMetadata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | FieldMetadata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | NativeMetadata | Preserved | Transformed(ArrowMetadata) |
| shp | Z | Preserved | Preserved |
| shp | M | Preserved | Preserved |
| shp | Dimensions | Preserved | Preserved |
| shp | GeometryType | Transformed(GeometryType) | Transformed(GeometryType) |
| shp | RingOrientation | Preserved | Transformed(RingOrientation) |
| shp | EmptyGeometry | Rejected | Rejected |
| shp | CoordinatePrecision | Preserved | Preserved |
| shp | CrsId | Preserved | Transformed(CrsId) |
| shp | CrsDefinition | Preserved | Transformed(CrsDefinition) |
| shp | FeatureId | Transformed(FeatureId) | Transformed(ArrowMetadata) |
| shp | Bbox | Transformed(Bbox) | Transformed(ArrowMetadata) |
| shp | ForeignMembers | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | LayerMetadata | NotApplicable: SHP/SHX/DBF grammar has no layer metadata or extension registry; vendor sidecars are separate formats (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | Styles | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | Hierarchy | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | Extensions | NotApplicable: SHP/SHX/DBF grammar has no layer metadata or extension registry; vendor sidecars are separate formats (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | ExternalReference | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | ExtendedData | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | FeatureMetadata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | AltitudeMode | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | Inference | Transformed(Inference) | Transformed(ArrowCoercion) |
| shp | NumberPrecision | Preserved | Preserved |
| shp | StructuredValue | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Preserved |
| shp | MissingValue | NotApplicable: Fixed record schemas have null slots, not absent members distinct from NULL (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Preserved |
| shp | CoordinateColumns | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | NumericFormat | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | Formula | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | FormulaCache | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowCoercion) |
| shp | Identifier | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | Description | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | DataColumns | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | SqlConstraints | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | ContractVersion | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | FieldIdentity | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | TypesDeclaration | Preserved | Transformed(ArrowMetadata) |
| shp | GeometryEncoding | Preserved | Transformed(ArrowMetadata) |
| shp | SpatialSemantics | Preserved | Transformed(ArrowMetadata) |
| shp | Precision | Preserved | Transformed(ArrowMetadata) |
| shp | Srid | NotApplicable: No independent per-geometry SRID slot; spatial-reference authorities are covered by CrsId/CrsDefinition (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | CrsResolution | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | DefinitionFormat | Preserved | Transformed(ArrowMetadata) |
| shp | AxisOrder | Preserved | Transformed(ArrowMetadata) |
| shp | PrimaryGeometry | Preserved | Transformed(ArrowMetadata) |
| shp | ExtensionName | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | ExtensionMetadata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| shp | UnknownDimensions | Preserved | Preserved |
| shp | Covering | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI Shapefile Technical Description, July 1998 (SHP/SHX/DBF)) | Transformed(ArrowMetadata) |
| gpkg | NullGeometry | Preserved | Preserved |
| gpkg | EmptyPoint | Preserved | Preserved |
| gpkg | SignedZero | Preserved | Preserved |
| gpkg | Subtypes | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC GeoPackage 1.3.1, core tables and registered extensions) | Transformed(GpkgMetadata) |
| gpkg | Aliases | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC GeoPackage 1.3.1, core tables and registered extensions) | Transformed(GpkgMetadata) |
| gpkg | Domains | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC GeoPackage 1.3.1, core tables and registered extensions) | Transformed(GpkgMetadata) |
| gpkg | TextStyle | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC GeoPackage 1.3.1, core tables and registered extensions) | Transformed(GpkgMetadata) |
| gpkg | PaperSpace | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC GeoPackage 1.3.1, core tables and registered extensions) | Transformed(GpkgMetadata) |
| gpkg | Xdata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC GeoPackage 1.3.1, core tables and registered extensions) | Transformed(GpkgMetadata) |
| gpkg | Width | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC GeoPackage 1.3.1, core tables and registered extensions) | Transformed(GpkgMetadata) |
| gpkg | Thickness | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC GeoPackage 1.3.1, core tables and registered extensions) | Transformed(GpkgMetadata) |
| gpkg | Lineweight | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC GeoPackage 1.3.1, core tables and registered extensions) | Transformed(GpkgMetadata) |
| gpkg | Linetype | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC GeoPackage 1.3.1, core tables and registered extensions) | Transformed(GpkgMetadata) |
| gpkg | Color | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC GeoPackage 1.3.1, core tables and registered extensions) | Transformed(GpkgMetadata) |
| gpkg | Handle | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC GeoPackage 1.3.1, core tables and registered extensions) | Transformed(GpkgMetadata) |
| gpkg | DbfDate | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC GeoPackage 1.3.1, core tables and registered extensions) | Preserved |
| gpkg | Utf8 | Preserved | Preserved |
| gpkg | Null | Preserved | Preserved |
| gpkg | EmptyString | Preserved | Preserved |
| gpkg | SignedInteger | Preserved | Transformed(ArrowCoercion) |
| gpkg | UnsignedInteger | NotApplicable: Native integer domains are signed; positive values do not declare an unsigned type (OGC GeoPackage 1.3.1, core tables and registered extensions) | Transformed(ArrowCoercion) |
| gpkg | Floating | Preserved | Transformed(ArrowCoercion) |
| gpkg | Boolean | Preserved | Transformed(ArrowCoercion) |
| gpkg | Binary | Preserved | Preserved |
| gpkg | Decimal | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC GeoPackage 1.3.1, core tables and registered extensions) | Rejected |
| gpkg | Temporal | Transformed(ArrowCoercion) | Rejected |
| gpkg | NestedDictionary | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC GeoPackage 1.3.1, core tables and registered extensions) | Rejected |
| gpkg | FieldNames | Preserved | Preserved |
| gpkg | NotNull | Preserved | Preserved |
| gpkg | FieldOrder | Transformed(SqlSchema) | Transformed(SqlSchema) |
| gpkg | SchemaMetadata | Preserved | Transformed(GpkgMetadata) |
| gpkg | FieldMetadata | Preserved | Transformed(GpkgMetadata) |
| gpkg | NativeMetadata | Preserved | Transformed(GpkgMetadata) |
| gpkg | Z | Preserved | Preserved |
| gpkg | M | Preserved | Preserved |
| gpkg | Dimensions | Preserved | Preserved |
| gpkg | GeometryType | Preserved | Preserved |
| gpkg | RingOrientation | Preserved | Preserved |
| gpkg | EmptyGeometry | Preserved | Preserved |
| gpkg | CoordinatePrecision | Preserved | Preserved |
| gpkg | CrsId | Preserved | Preserved |
| gpkg | CrsDefinition | Preserved | Preserved |
| gpkg | FeatureId | Preserved | Transformed(GpkgMetadata) |
| gpkg | Bbox | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC GeoPackage 1.3.1, core tables and registered extensions) | Transformed(GpkgMetadata) |
| gpkg | ForeignMembers | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC GeoPackage 1.3.1, core tables and registered extensions) | Transformed(GpkgMetadata) |
| gpkg | LayerMetadata | Transformed(LayerMetadata) | Transformed(GpkgMetadata) |
| gpkg | Styles | Transformed(NativeStyles) | Transformed(GpkgMetadata) |
| gpkg | Hierarchy | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC GeoPackage 1.3.1, core tables and registered extensions) | Transformed(GpkgMetadata) |
| gpkg | Extensions | Transformed(LayerMetadata) | Transformed(GpkgMetadata) |
| gpkg | ExternalReference | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC GeoPackage 1.3.1, core tables and registered extensions) | Transformed(GpkgMetadata) |
| gpkg | ExtendedData | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC GeoPackage 1.3.1, core tables and registered extensions) | Transformed(GpkgMetadata) |
| gpkg | FeatureMetadata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC GeoPackage 1.3.1, core tables and registered extensions) | Transformed(GpkgMetadata) |
| gpkg | AltitudeMode | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC GeoPackage 1.3.1, core tables and registered extensions) | Transformed(GpkgMetadata) |
| gpkg | Inference | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC GeoPackage 1.3.1, core tables and registered extensions) | Transformed(ArrowCoercion) |
| gpkg | NumberPrecision | Preserved | Preserved |
| gpkg | StructuredValue | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC GeoPackage 1.3.1, core tables and registered extensions) | Preserved |
| gpkg | MissingValue | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC GeoPackage 1.3.1, core tables and registered extensions) | Preserved |
| gpkg | CoordinateColumns | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC GeoPackage 1.3.1, core tables and registered extensions) | Transformed(GpkgMetadata) |
| gpkg | NumericFormat | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC GeoPackage 1.3.1, core tables and registered extensions) | Transformed(GpkgMetadata) |
| gpkg | Formula | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC GeoPackage 1.3.1, core tables and registered extensions) | Transformed(GpkgMetadata) |
| gpkg | FormulaCache | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC GeoPackage 1.3.1, core tables and registered extensions) | Transformed(ArrowCoercion) |
| gpkg | Identifier | Preserved | Transformed(GpkgMetadata) |
| gpkg | Description | Preserved | Transformed(GpkgMetadata) |
| gpkg | DataColumns | Transformed(LayerMetadata) | Transformed(GpkgMetadata) |
| gpkg | SqlConstraints | Transformed(SqlSchema) | Transformed(GpkgMetadata) |
| gpkg | ContractVersion | Preserved | Preserved |
| gpkg | FieldIdentity | Preserved | Preserved |
| gpkg | TypesDeclaration | Preserved | Transformed(GpkgMetadata) |
| gpkg | GeometryEncoding | Preserved | Transformed(GpkgMetadata) |
| gpkg | SpatialSemantics | Preserved | Transformed(GpkgMetadata) |
| gpkg | Precision | Preserved | Transformed(GpkgMetadata) |
| gpkg | Srid | Preserved | Transformed(GpkgMetadata) |
| gpkg | CrsResolution | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC GeoPackage 1.3.1, core tables and registered extensions) | Transformed(GpkgMetadata) |
| gpkg | DefinitionFormat | Preserved | Transformed(GpkgMetadata) |
| gpkg | AxisOrder | Preserved | Transformed(GpkgMetadata) |
| gpkg | PrimaryGeometry | Preserved | Transformed(GpkgMetadata) |
| gpkg | ExtensionName | Preserved | Transformed(GpkgMetadata) |
| gpkg | ExtensionMetadata | Preserved | Transformed(GpkgMetadata) |
| gpkg | UnknownDimensions | Preserved | Preserved |
| gpkg | Covering | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC GeoPackage 1.3.1, core tables and registered extensions) | Transformed(GpkgMetadata) |
| filegdb | NullGeometry | Preserved | Preserved |
| filegdb | EmptyPoint | NotApplicable: Empty shapes use the native null-shape representation; no distinct typed EMPTY value survives native storage (NullGeometry is exercised) (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(MissingValue) |
| filegdb | SignedZero | NotApplicable: Coordinates are stored on an integer precision grid, which has no negative-zero encoding (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(CoordinatePrecision) |
| filegdb | Subtypes | Transformed(LayerMetadata) | Transformed(ArrowMetadata) |
| filegdb | Aliases | Transformed(LayerMetadata) | Transformed(ArrowMetadata) |
| filegdb | Domains | Transformed(LayerMetadata) | Transformed(ArrowMetadata) |
| filegdb | TextStyle | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(ArrowMetadata) |
| filegdb | PaperSpace | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(ArrowMetadata) |
| filegdb | Xdata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(ArrowMetadata) |
| filegdb | Width | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(ArrowMetadata) |
| filegdb | Thickness | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(ArrowMetadata) |
| filegdb | Lineweight | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(ArrowMetadata) |
| filegdb | Linetype | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(ArrowMetadata) |
| filegdb | Color | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(ArrowMetadata) |
| filegdb | Handle | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(ArrowMetadata) |
| filegdb | DbfDate | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Preserved |
| filegdb | Utf8 | Preserved | Preserved |
| filegdb | Null | Preserved | Preserved |
| filegdb | EmptyString | Preserved | Preserved |
| filegdb | SignedInteger | Preserved | Transformed(ArrowCoercion) |
| filegdb | UnsignedInteger | NotApplicable: Native integer domains are signed; positive values do not declare an unsigned type (ESRI File Geodatabase API, geodatabase schema XML) | Rejected |
| filegdb | Floating | Preserved | Transformed(ArrowCoercion) |
| filegdb | Boolean | NotApplicable: FileGDB field types have no Boolean domain; Short integers are not Boolean declarations (ESRI File Geodatabase API, geodatabase schema XML) | Rejected |
| filegdb | Binary | Preserved | Rejected |
| filegdb | Decimal | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Rejected |
| filegdb | Temporal | Transformed(NumberPrecision) | Rejected |
| filegdb | NestedDictionary | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Rejected |
| filegdb | FieldNames | Preserved | Transformed(FieldNames) |
| filegdb | NotNull | Transformed(NotNull) | Transformed(NotNull) |
| filegdb | FieldOrder | Transformed(FieldOrder) | Transformed(FieldOrder) |
| filegdb | SchemaMetadata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(ArrowMetadata) |
| filegdb | FieldMetadata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(ArrowMetadata) |
| filegdb | NativeMetadata | Preserved | Transformed(ArrowMetadata) |
| filegdb | Z | Preserved | Preserved |
| filegdb | M | Preserved | Preserved |
| filegdb | Dimensions | Preserved | Preserved |
| filegdb | GeometryType | Preserved | Preserved |
| filegdb | RingOrientation | Preserved | Transformed(RingOrientation) |
| filegdb | EmptyGeometry | NotApplicable: Empty shapes use the native null-shape representation; no distinct typed EMPTY value survives native storage (NullGeometry is exercised) (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(MissingValue) |
| filegdb | CoordinatePrecision | Preserved | Transformed(CoordinatePrecision) |
| filegdb | CrsId | Preserved | Transformed(CrsId) |
| filegdb | CrsDefinition | Preserved | Transformed(CrsDefinition) |
| filegdb | FeatureId | Transformed(FeatureId) | Transformed(ArrowMetadata) |
| filegdb | Bbox | Transformed(Bbox) | Transformed(ArrowMetadata) |
| filegdb | ForeignMembers | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(ArrowMetadata) |
| filegdb | LayerMetadata | Transformed(LayerMetadata) | Transformed(ArrowMetadata) |
| filegdb | Styles | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(ArrowMetadata) |
| filegdb | Hierarchy | Transformed(NativeHierarchy) | Transformed(ArrowMetadata) |
| filegdb | Extensions | Transformed(LayerMetadata) | Transformed(ArrowMetadata) |
| filegdb | ExternalReference | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(ArrowMetadata) |
| filegdb | ExtendedData | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(ArrowMetadata) |
| filegdb | FeatureMetadata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(ArrowMetadata) |
| filegdb | AltitudeMode | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(ArrowMetadata) |
| filegdb | Inference | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(ArrowCoercion) |
| filegdb | NumberPrecision | Preserved | Transformed(CoordinatePrecision) |
| filegdb | StructuredValue | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Preserved |
| filegdb | MissingValue | NotApplicable: Fixed record schemas have null slots, not absent members distinct from NULL (ESRI File Geodatabase API, geodatabase schema XML) | Preserved |
| filegdb | CoordinateColumns | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(ArrowMetadata) |
| filegdb | NumericFormat | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(ArrowMetadata) |
| filegdb | Formula | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(ArrowMetadata) |
| filegdb | FormulaCache | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(ArrowCoercion) |
| filegdb | Identifier | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(ArrowMetadata) |
| filegdb | Description | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(ArrowMetadata) |
| filegdb | DataColumns | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(ArrowMetadata) |
| filegdb | SqlConstraints | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(ArrowMetadata) |
| filegdb | ContractVersion | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(ArrowMetadata) |
| filegdb | FieldIdentity | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(ArrowMetadata) |
| filegdb | TypesDeclaration | Preserved | Transformed(ArrowMetadata) |
| filegdb | GeometryEncoding | Preserved | Transformed(ArrowMetadata) |
| filegdb | SpatialSemantics | Preserved | Transformed(ArrowMetadata) |
| filegdb | Precision | Preserved | Transformed(ArrowMetadata) |
| filegdb | Srid | NotApplicable: No independent per-geometry SRID slot; spatial-reference authorities are covered by CrsId/CrsDefinition (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(ArrowMetadata) |
| filegdb | CrsResolution | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(ArrowMetadata) |
| filegdb | DefinitionFormat | Preserved | Transformed(ArrowMetadata) |
| filegdb | AxisOrder | Preserved | Transformed(ArrowMetadata) |
| filegdb | PrimaryGeometry | Preserved | Transformed(ArrowMetadata) |
| filegdb | ExtensionName | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(ArrowMetadata) |
| filegdb | ExtensionMetadata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(ArrowMetadata) |
| filegdb | UnknownDimensions | NotApplicable: Native shape/entity coordinates determine dimensions; no explicit unknown-dimensions declaration (ESRI File Geodatabase API, geodatabase schema XML) | Rejected |
| filegdb | Covering | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ESRI File Geodatabase API, geodatabase schema XML) | Transformed(ArrowMetadata) |
| geojson | NullGeometry | Preserved | Preserved |
| geojson | EmptyPoint | Rejected | Rejected |
| geojson | SignedZero | Preserved | Preserved |
| geojson | Subtypes | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowMetadata) |
| geojson | Aliases | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowMetadata) |
| geojson | Domains | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowMetadata) |
| geojson | TextStyle | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowMetadata) |
| geojson | PaperSpace | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowMetadata) |
| geojson | Xdata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowMetadata) |
| geojson | Width | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowMetadata) |
| geojson | Thickness | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowMetadata) |
| geojson | Lineweight | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowMetadata) |
| geojson | Linetype | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowMetadata) |
| geojson | Color | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowMetadata) |
| geojson | Handle | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowMetadata) |
| geojson | DbfDate | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Preserved |
| geojson | Utf8 | Preserved | Preserved |
| geojson | Null | Preserved | Preserved |
| geojson | EmptyString | Preserved | Preserved |
| geojson | SignedInteger | Transformed(Inference) | Transformed(ArrowCoercion) |
| geojson | UnsignedInteger | Transformed(Inference) | Transformed(ArrowCoercion) |
| geojson | Floating | Transformed(NumberPrecision) | Transformed(ArrowCoercion) |
| geojson | Boolean | Preserved | Preserved |
| geojson | Binary | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Rejected |
| geojson | Decimal | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Rejected |
| geojson | Temporal | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowCoercion) |
| geojson | NestedDictionary | Transformed(StructuredValue) | Rejected |
| geojson | FieldNames | Preserved | Transformed(FieldNames) |
| geojson | NotNull | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(NotNull) |
| geojson | FieldOrder | Transformed(FieldOrder) | Transformed(FieldOrder) |
| geojson | SchemaMetadata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowMetadata) |
| geojson | FieldMetadata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowMetadata) |
| geojson | NativeMetadata | Transformed(ForeignMembers) | Transformed(ArrowMetadata) |
| geojson | Z | Preserved | Preserved |
| geojson | M | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Rejected |
| geojson | Dimensions | Transformed(Dimensions) | Transformed(Dimensions) |
| geojson | GeometryType | Preserved | Preserved |
| geojson | RingOrientation | Preserved | Preserved |
| geojson | EmptyGeometry | Rejected | Rejected |
| geojson | CoordinatePrecision | Transformed(NumberPrecision) | Preserved |
| geojson | CrsId | Preserved | Transformed(CrsId) |
| geojson | CrsDefinition | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(CrsDefinition) |
| geojson | FeatureId | Transformed(FeatureId) | Transformed(ArrowMetadata) |
| geojson | Bbox | Transformed(Bbox) | Transformed(ArrowMetadata) |
| geojson | ForeignMembers | Transformed(ForeignMembers) | Transformed(ArrowMetadata) |
| geojson | LayerMetadata | Transformed(ForeignMembers) | Transformed(ArrowMetadata) |
| geojson | Styles | Transformed(ForeignMembers) | Transformed(ArrowMetadata) |
| geojson | Hierarchy | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowMetadata) |
| geojson | Extensions | Transformed(ForeignMembers) | Transformed(ArrowMetadata) |
| geojson | ExternalReference | Transformed(ForeignMembers) | Transformed(ArrowMetadata) |
| geojson | ExtendedData | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowMetadata) |
| geojson | FeatureMetadata | Transformed(ForeignMembers) | Transformed(ArrowMetadata) |
| geojson | AltitudeMode | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowMetadata) |
| geojson | Inference | Transformed(Inference) | Transformed(ArrowCoercion) |
| geojson | NumberPrecision | Transformed(NumberPrecision) | Preserved |
| geojson | StructuredValue | Transformed(StructuredValue) | Preserved |
| geojson | MissingValue | Transformed(MissingValue) | Preserved |
| geojson | CoordinateColumns | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowMetadata) |
| geojson | NumericFormat | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowMetadata) |
| geojson | Formula | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowMetadata) |
| geojson | FormulaCache | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowCoercion) |
| geojson | Identifier | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowMetadata) |
| geojson | Description | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowMetadata) |
| geojson | DataColumns | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowMetadata) |
| geojson | SqlConstraints | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowMetadata) |
| geojson | ContractVersion | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowMetadata) |
| geojson | FieldIdentity | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowMetadata) |
| geojson | TypesDeclaration | Preserved | Transformed(ArrowMetadata) |
| geojson | GeometryEncoding | Preserved | Transformed(ArrowMetadata) |
| geojson | SpatialSemantics | Preserved | Transformed(ArrowMetadata) |
| geojson | Precision | Preserved | Transformed(ArrowMetadata) |
| geojson | Srid | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowMetadata) |
| geojson | CrsResolution | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowMetadata) |
| geojson | DefinitionFormat | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowMetadata) |
| geojson | AxisOrder | Preserved | Transformed(ArrowMetadata) |
| geojson | PrimaryGeometry | Preserved | Transformed(ArrowMetadata) |
| geojson | ExtensionName | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowMetadata) |
| geojson | ExtensionMetadata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowMetadata) |
| geojson | UnknownDimensions | Preserved | Preserved |
| geojson | Covering | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (RFC 7946 sections 3, 6 (GeoJSON objects and foreign members)) | Transformed(ArrowMetadata) |
| kml | NullGeometry | Preserved | Preserved |
| kml | EmptyPoint | Rejected | Rejected |
| kml | SignedZero | Preserved | Preserved |
| kml | Subtypes | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Transformed(ArrowMetadata) |
| kml | Aliases | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Transformed(ArrowMetadata) |
| kml | Domains | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Transformed(ArrowMetadata) |
| kml | TextStyle | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Transformed(ArrowMetadata) |
| kml | PaperSpace | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Transformed(ArrowMetadata) |
| kml | Xdata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Transformed(ArrowMetadata) |
| kml | Width | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Transformed(ArrowMetadata) |
| kml | Thickness | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Transformed(ArrowMetadata) |
| kml | Lineweight | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Transformed(ArrowMetadata) |
| kml | Linetype | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Transformed(ArrowMetadata) |
| kml | Color | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Transformed(ArrowMetadata) |
| kml | Handle | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Transformed(ArrowMetadata) |
| kml | DbfDate | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Preserved |
| kml | Utf8 | Preserved | Preserved |
| kml | Null | Preserved | Preserved |
| kml | EmptyString | Preserved | Preserved |
| kml | SignedInteger | Rejected | Transformed(ArrowCoercion) |
| kml | UnsignedInteger | Rejected | Transformed(ArrowCoercion) |
| kml | Floating | Rejected | Transformed(ArrowCoercion) |
| kml | Boolean | Rejected | Transformed(ArrowCoercion) |
| kml | Binary | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Rejected |
| kml | Decimal | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Rejected |
| kml | Temporal | Rejected | Transformed(ArrowCoercion) |
| kml | NestedDictionary | Rejected | Rejected |
| kml | FieldNames | Transformed(ExtendedData) | Transformed(FieldNames) |
| kml | NotNull | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Transformed(NotNull) |
| kml | FieldOrder | Transformed(FieldOrder) | Transformed(FieldOrder) |
| kml | SchemaMetadata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Transformed(ArrowMetadata) |
| kml | FieldMetadata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Transformed(ArrowMetadata) |
| kml | NativeMetadata | Transformed(ForeignMembers) | Transformed(ArrowMetadata) |
| kml | Z | Transformed(AltitudeMode) | Preserved |
| kml | M | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Rejected |
| kml | Dimensions | Transformed(Dimensions) | Transformed(Dimensions) |
| kml | GeometryType | Transformed(GeometryType) | Transformed(GeometryType) |
| kml | RingOrientation | Preserved | Preserved |
| kml | EmptyGeometry | Rejected | Rejected |
| kml | CoordinatePrecision | Transformed(NumberPrecision) | Preserved |
| kml | CrsId | Preserved | Transformed(CrsId) |
| kml | CrsDefinition | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Transformed(CrsDefinition) |
| kml | FeatureId | Transformed(FeatureId) | Transformed(ArrowMetadata) |
| kml | Bbox | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Transformed(ArrowMetadata) |
| kml | ForeignMembers | Transformed(ForeignMembers) | Transformed(ArrowMetadata) |
| kml | LayerMetadata | Transformed(LayerMetadata) | Transformed(ArrowMetadata) |
| kml | Styles | Transformed(Styles) | Transformed(ArrowMetadata) |
| kml | Hierarchy | Transformed(Hierarchy) | Transformed(ArrowMetadata) |
| kml | Extensions | Transformed(ForeignMembers) | Transformed(ArrowMetadata) |
| kml | ExternalReference | Transformed(ExternalReference) | Transformed(ArrowMetadata) |
| kml | ExtendedData | Transformed(ExtendedData) | Transformed(ArrowMetadata) |
| kml | FeatureMetadata | Transformed(FeatureMetadata) | Transformed(ArrowMetadata) |
| kml | AltitudeMode | Transformed(AltitudeMode) | Transformed(ArrowMetadata) |
| kml | Inference | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Transformed(ArrowCoercion) |
| kml | NumberPrecision | Transformed(NumberPrecision) | Preserved |
| kml | StructuredValue | Transformed(ExtendedData) | Preserved |
| kml | MissingValue | Transformed(MissingValue) | Preserved |
| kml | CoordinateColumns | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Transformed(ArrowMetadata) |
| kml | NumericFormat | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Transformed(ArrowMetadata) |
| kml | Formula | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Transformed(ArrowMetadata) |
| kml | FormulaCache | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Transformed(ArrowCoercion) |
| kml | Identifier | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Transformed(ArrowMetadata) |
| kml | Description | Rejected | Transformed(ArrowMetadata) |
| kml | DataColumns | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Transformed(ArrowMetadata) |
| kml | SqlConstraints | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Transformed(ArrowMetadata) |
| kml | ContractVersion | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Transformed(ArrowMetadata) |
| kml | FieldIdentity | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Transformed(ArrowMetadata) |
| kml | TypesDeclaration | Preserved | Transformed(ArrowMetadata) |
| kml | GeometryEncoding | Preserved | Transformed(ArrowMetadata) |
| kml | SpatialSemantics | Preserved | Transformed(ArrowMetadata) |
| kml | Precision | Preserved | Transformed(ArrowMetadata) |
| kml | Srid | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Transformed(ArrowMetadata) |
| kml | CrsResolution | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Transformed(ArrowMetadata) |
| kml | DefinitionFormat | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Transformed(ArrowMetadata) |
| kml | AxisOrder | Preserved | Transformed(ArrowMetadata) |
| kml | PrimaryGeometry | Preserved | Transformed(ArrowMetadata) |
| kml | ExtensionName | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Transformed(ArrowMetadata) |
| kml | ExtensionMetadata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Transformed(ArrowMetadata) |
| kml | UnknownDimensions | Preserved | Preserved |
| kml | Covering | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (OGC 07-147r2 (KML 2.2), Feature, Geometry and ExtendedData) | Transformed(ArrowMetadata) |
| dxf | NullGeometry | NotApplicable: DXF has no NULL or EMPTY geometry value; omitted entities and malformed coordinates are not empty geometries (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Rejected |
| dxf | EmptyPoint | NotApplicable: DXF has no NULL or EMPTY geometry value; omitted entities and malformed coordinates are not empty geometries (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Rejected |
| dxf | SignedZero | Preserved | Transformed(CoordinatePrecision) |
| dxf | Subtypes | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Transformed(ArrowMetadata) |
| dxf | Aliases | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Transformed(ArrowMetadata) |
| dxf | Domains | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Transformed(ArrowMetadata) |
| dxf | TextStyle | Transformed(NativeStyles) | Transformed(ArrowMetadata) |
| dxf | PaperSpace | Transformed(NativeHierarchy) | Transformed(ArrowMetadata) |
| dxf | Xdata | Transformed(LayerMetadata) | Transformed(ArrowMetadata) |
| dxf | Width | Transformed(NativeStyles) | Transformed(ArrowMetadata) |
| dxf | Thickness | Transformed(NativeStyles) | Transformed(ArrowMetadata) |
| dxf | Lineweight | Transformed(NativeStyles) | Transformed(ArrowMetadata) |
| dxf | Linetype | Transformed(NativeStyles) | Transformed(ArrowMetadata) |
| dxf | Color | Transformed(NativeStyles) | Transformed(ArrowMetadata) |
| dxf | Handle | Transformed(FeatureId) | Transformed(ArrowMetadata) |
| dxf | DbfDate | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Transformed(DxfAttribute) |
| dxf | Utf8 | Preserved | Transformed(DxfAttribute) |
| dxf | Null | Preserved | Transformed(DxfAttribute) |
| dxf | EmptyString | Preserved | Transformed(DxfAttribute) |
| dxf | SignedInteger | Rejected | Transformed(DxfAttribute) |
| dxf | UnsignedInteger | NotApplicable: Native integer domains are signed; positive values do not declare an unsigned type (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Transformed(DxfAttribute) |
| dxf | Floating | Rejected | Transformed(DxfAttribute) |
| dxf | Boolean | Rejected | Transformed(DxfAttribute) |
| dxf | Binary | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Transformed(DxfAttribute) |
| dxf | Decimal | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Transformed(DxfAttribute) |
| dxf | Temporal | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Transformed(DxfAttribute) |
| dxf | NestedDictionary | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Transformed(DxfAttribute) |
| dxf | FieldNames | Transformed(FieldNames) | Transformed(FieldNames) |
| dxf | NotNull | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Transformed(NotNull) |
| dxf | FieldOrder | Transformed(FieldOrder) | Transformed(FieldOrder) |
| dxf | SchemaMetadata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Transformed(ArrowMetadata) |
| dxf | FieldMetadata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Transformed(ArrowMetadata) |
| dxf | NativeMetadata | Preserved | Transformed(ArrowMetadata) |
| dxf | Z | Preserved | Preserved |
| dxf | M | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Rejected |
| dxf | Dimensions | Transformed(Dimensions) | Transformed(Dimensions) |
| dxf | GeometryType | Transformed(GeometryType) | Transformed(GeometryType) |
| dxf | RingOrientation | Preserved | Preserved |
| dxf | EmptyGeometry | NotApplicable: DXF has no NULL or EMPTY geometry value; omitted entities and malformed coordinates are not empty geometries (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Rejected |
| dxf | CoordinatePrecision | Preserved | Transformed(CoordinatePrecision) |
| dxf | CrsId | Preserved | Transformed(CrsId) |
| dxf | CrsDefinition | Preserved | Transformed(CrsDefinition) |
| dxf | FeatureId | Transformed(FeatureId) | Transformed(ArrowMetadata) |
| dxf | Bbox | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Transformed(ArrowMetadata) |
| dxf | ForeignMembers | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Transformed(ArrowMetadata) |
| dxf | LayerMetadata | Transformed(LayerMetadata) | Transformed(ArrowMetadata) |
| dxf | Styles | Transformed(NativeStyles) | Transformed(ArrowMetadata) |
| dxf | Hierarchy | Transformed(NativeHierarchy) | Transformed(ArrowMetadata) |
| dxf | Extensions | Transformed(LayerMetadata) | Transformed(ArrowMetadata) |
| dxf | ExternalReference | Transformed(ExternalReference) | Transformed(ArrowMetadata) |
| dxf | ExtendedData | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Transformed(ArrowMetadata) |
| dxf | FeatureMetadata | Transformed(LayerMetadata) | Transformed(ArrowMetadata) |
| dxf | AltitudeMode | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Transformed(ArrowMetadata) |
| dxf | Inference | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Transformed(DxfAttribute) |
| dxf | NumberPrecision | Preserved | Transformed(CoordinatePrecision) |
| dxf | StructuredValue | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Transformed(DxfAttribute) |
| dxf | MissingValue | Preserved | Transformed(DxfAttribute) |
| dxf | CoordinateColumns | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Transformed(ArrowMetadata) |
| dxf | NumericFormat | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Transformed(ArrowMetadata) |
| dxf | Formula | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Transformed(ArrowMetadata) |
| dxf | FormulaCache | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Transformed(DxfAttribute) |
| dxf | Identifier | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Transformed(ArrowMetadata) |
| dxf | Description | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Transformed(ArrowMetadata) |
| dxf | DataColumns | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Transformed(ArrowMetadata) |
| dxf | SqlConstraints | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Transformed(ArrowMetadata) |
| dxf | ContractVersion | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Transformed(ArrowMetadata) |
| dxf | FieldIdentity | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Transformed(ArrowMetadata) |
| dxf | TypesDeclaration | Preserved | Transformed(ArrowMetadata) |
| dxf | GeometryEncoding | Preserved | Transformed(ArrowMetadata) |
| dxf | SpatialSemantics | Preserved | Transformed(ArrowMetadata) |
| dxf | Precision | Preserved | Transformed(ArrowMetadata) |
| dxf | Srid | NotApplicable: No independent per-geometry SRID slot; spatial-reference authorities are covered by CrsId/CrsDefinition (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Transformed(ArrowMetadata) |
| dxf | CrsResolution | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Transformed(ArrowMetadata) |
| dxf | DefinitionFormat | Preserved | Transformed(ArrowMetadata) |
| dxf | AxisOrder | Preserved | Transformed(ArrowMetadata) |
| dxf | PrimaryGeometry | Preserved | Transformed(ArrowMetadata) |
| dxf | ExtensionName | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Transformed(ArrowMetadata) |
| dxf | ExtensionMetadata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Transformed(ArrowMetadata) |
| dxf | UnknownDimensions | NotApplicable: Native shape/entity coordinates determine dimensions; no explicit unknown-dimensions declaration (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Preserved |
| dxf | Covering | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (Autodesk DXF Reference (HEADER, TABLES, BLOCKS, ENTITIES, OBJECTS)) | Transformed(ArrowMetadata) |
| xls | NullGeometry | Preserved | Preserved |
| xls | EmptyPoint | Rejected | Rejected |
| xls | SignedZero | Preserved | Preserved |
| xls | Subtypes | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | Aliases | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | Domains | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | TextStyle | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | PaperSpace | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | Xdata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | Width | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | Thickness | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | Lineweight | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | Linetype | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | Color | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | Handle | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | DbfDate | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Preserved |
| xls | Utf8 | Preserved | Preserved |
| xls | Null | Preserved | Preserved |
| xls | EmptyString | Preserved | Preserved |
| xls | SignedInteger | Transformed(Inference) | Transformed(ArrowCoercion) |
| xls | UnsignedInteger | Transformed(Inference) | Transformed(ArrowCoercion) |
| xls | Floating | Transformed(Inference) | Transformed(ArrowCoercion) |
| xls | Boolean | Preserved | Preserved |
| xls | Binary | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Rejected |
| xls | Decimal | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Rejected |
| xls | Temporal | Transformed(Inference) | Transformed(ArrowCoercion) |
| xls | NestedDictionary | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Rejected |
| xls | FieldNames | Transformed(FieldNames) | Transformed(FieldNames) |
| xls | NotNull | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(NotNull) |
| xls | FieldOrder | Transformed(FieldOrder) | Transformed(FieldOrder) |
| xls | SchemaMetadata | Preserved | Transformed(ArrowMetadata) |
| xls | FieldMetadata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | NativeMetadata | Preserved | Transformed(ArrowMetadata) |
| xls | Z | Preserved | Preserved |
| xls | M | Preserved | Preserved |
| xls | Dimensions | Preserved | Preserved |
| xls | GeometryType | Preserved | Preserved |
| xls | RingOrientation | Preserved | Preserved |
| xls | EmptyGeometry | Preserved | Preserved |
| xls | CoordinatePrecision | Transformed(NumberPrecision) | Preserved |
| xls | CrsId | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(CrsIdAbsent) |
| xls | CrsDefinition | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(CrsDefinition) |
| xls | FeatureId | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | Bbox | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | ForeignMembers | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | LayerMetadata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | Styles | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | Hierarchy | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | Extensions | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | ExternalReference | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | ExtendedData | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | FeatureMetadata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | AltitudeMode | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | Inference | Transformed(Inference) | Transformed(ArrowCoercion) |
| xls | NumberPrecision | Transformed(NumberPrecision) | Preserved |
| xls | StructuredValue | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Preserved |
| xls | MissingValue | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Preserved |
| xls | CoordinateColumns | Transformed(FieldNames) | Transformed(ArrowMetadata) |
| xls | NumericFormat | Transformed(NativeStyles) | Transformed(ArrowMetadata) |
| xls | Formula | Transformed(LayerMetadata) | Transformed(ArrowMetadata) |
| xls | FormulaCache | Preserved | Transformed(ArrowCoercion) |
| xls | Identifier | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | Description | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | DataColumns | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | SqlConstraints | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | ContractVersion | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | FieldIdentity | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | TypesDeclaration | Preserved | Transformed(ArrowMetadata) |
| xls | GeometryEncoding | Preserved | Transformed(ArrowMetadata) |
| xls | SpatialSemantics | Preserved | Transformed(ArrowMetadata) |
| xls | Precision | Preserved | Transformed(ArrowMetadata) |
| xls | Srid | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | CrsResolution | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | DefinitionFormat | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | AxisOrder | Preserved | Transformed(ArrowMetadata) |
| xls | PrimaryGeometry | Preserved | Transformed(ArrowMetadata) |
| xls | ExtensionName | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | ExtensionMetadata | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| xls | UnknownDimensions | Preserved | Preserved |
| xls | Covering | NotApplicable: No native grammar construct for this element; foreign Arrow transport is a Write case (ECMA-376 Part 1, 18 SpreadsheetML (cells, formulas and styles)) | Transformed(ArrowMetadata) |
| geoparquet | NullGeometry | Preserved | Preserved |
| geoparquet | EmptyPoint | Preserved | Preserved |
| geoparquet | SignedZero | Preserved | Preserved |
| geoparquet | Subtypes | Rejected | Transformed(ArrowMetadata) |
| geoparquet | Aliases | Rejected | Transformed(ArrowMetadata) |
| geoparquet | Domains | Rejected | Transformed(ArrowMetadata) |
| geoparquet | TextStyle | Rejected | Transformed(ArrowMetadata) |
| geoparquet | PaperSpace | Rejected | Transformed(ArrowMetadata) |
| geoparquet | Xdata | Rejected | Transformed(ArrowMetadata) |
| geoparquet | Width | Rejected | Transformed(ArrowMetadata) |
| geoparquet | Thickness | Rejected | Transformed(ArrowMetadata) |
| geoparquet | Lineweight | Rejected | Transformed(ArrowMetadata) |
| geoparquet | Linetype | Rejected | Transformed(ArrowMetadata) |
| geoparquet | Color | Rejected | Transformed(ArrowMetadata) |
| geoparquet | Handle | Rejected | Transformed(ArrowMetadata) |
| geoparquet | DbfDate | Rejected | Preserved |
| geoparquet | Utf8 | Preserved | Preserved |
| geoparquet | Null | Preserved | Preserved |
| geoparquet | EmptyString | Preserved | Preserved |
| geoparquet | SignedInteger | Preserved | Preserved |
| geoparquet | UnsignedInteger | Preserved | Preserved |
| geoparquet | Floating | Preserved | Preserved |
| geoparquet | Boolean | Preserved | Preserved |
| geoparquet | Binary | Preserved | Preserved |
| geoparquet | Decimal | Preserved | Preserved |
| geoparquet | Temporal | Preserved | Preserved |
| geoparquet | NestedDictionary | Preserved | Preserved |
| geoparquet | FieldNames | Preserved | Preserved |
| geoparquet | NotNull | Preserved | Preserved |
| geoparquet | FieldOrder | Preserved | Preserved |
| geoparquet | SchemaMetadata | Transformed(ArrowMetadata) | Transformed(ArrowMetadata) |
| geoparquet | FieldMetadata | Transformed(ArrowMetadata) | Transformed(ArrowMetadata) |
| geoparquet | NativeMetadata | Transformed(ArrowMetadata) | Transformed(ArrowMetadata) |
| geoparquet | Z | Preserved | Preserved |
| geoparquet | M | Rejected | Rejected |
| geoparquet | Dimensions | Preserved | Preserved |
| geoparquet | GeometryType | Preserved | Preserved |
| geoparquet | RingOrientation | Preserved | Preserved |
| geoparquet | EmptyGeometry | Preserved | Preserved |
| geoparquet | CoordinatePrecision | Preserved | Preserved |
| geoparquet | CrsId | Preserved | Preserved |
| geoparquet | CrsDefinition | Preserved | Preserved |
| geoparquet | FeatureId | Preserved | Transformed(ArrowMetadata) |
| geoparquet | Bbox | Transformed(Bbox) | Transformed(ArrowMetadata) |
| geoparquet | ForeignMembers | Rejected | Transformed(ArrowMetadata) |
| geoparquet | LayerMetadata | Rejected | Transformed(ArrowMetadata) |
| geoparquet | Styles | Rejected | Transformed(ArrowMetadata) |
| geoparquet | Hierarchy | Rejected | Transformed(ArrowMetadata) |
| geoparquet | Extensions | Rejected | Transformed(ArrowMetadata) |
| geoparquet | ExternalReference | Rejected | Transformed(ArrowMetadata) |
| geoparquet | ExtendedData | Rejected | Transformed(ArrowMetadata) |
| geoparquet | FeatureMetadata | Rejected | Transformed(ArrowMetadata) |
| geoparquet | AltitudeMode | Rejected | Transformed(ArrowMetadata) |
| geoparquet | Inference | Rejected | Preserved |
| geoparquet | NumberPrecision | Preserved | Preserved |
| geoparquet | StructuredValue | Rejected | Preserved |
| geoparquet | MissingValue | Rejected | Preserved |
| geoparquet | CoordinateColumns | Rejected | Transformed(ArrowMetadata) |
| geoparquet | NumericFormat | Rejected | Transformed(ArrowMetadata) |
| geoparquet | Formula | Rejected | Transformed(ArrowMetadata) |
| geoparquet | FormulaCache | Rejected | Preserved |
| geoparquet | Identifier | Rejected | Transformed(ArrowMetadata) |
| geoparquet | Description | Rejected | Transformed(ArrowMetadata) |
| geoparquet | DataColumns | Rejected | Transformed(ArrowMetadata) |
| geoparquet | SqlConstraints | Rejected | Transformed(ArrowMetadata) |
| geoparquet | ContractVersion | Transformed(ArrowMetadata) | Transformed(ArrowMetadata) |
| geoparquet | FieldIdentity | Transformed(ArrowMetadata) | Transformed(ArrowMetadata) |
| geoparquet | TypesDeclaration | Transformed(ArrowMetadata) | Transformed(ArrowMetadata) |
| geoparquet | GeometryEncoding | Transformed(ArrowMetadata) | Transformed(ArrowMetadata) |
| geoparquet | SpatialSemantics | Transformed(ArrowMetadata) | Transformed(ArrowMetadata) |
| geoparquet | Precision | Transformed(ArrowMetadata) | Transformed(ArrowMetadata) |
| geoparquet | Srid | Transformed(ArrowMetadata) | Transformed(ArrowMetadata) |
| geoparquet | CrsResolution | Transformed(ArrowMetadata) | Transformed(ArrowMetadata) |
| geoparquet | DefinitionFormat | Transformed(ArrowMetadata) | Transformed(ArrowMetadata) |
| geoparquet | AxisOrder | Transformed(ArrowMetadata) | Transformed(ArrowMetadata) |
| geoparquet | PrimaryGeometry | Transformed(ArrowMetadata) | Transformed(ArrowMetadata) |
| geoparquet | ExtensionName | Transformed(ArrowMetadata) | Transformed(ArrowMetadata) |
| geoparquet | ExtensionMetadata | Transformed(ArrowMetadata) | Transformed(ArrowMetadata) |
| geoparquet | UnknownDimensions | Preserved | Preserved |
| geoparquet | Covering | Transformed(Bbox) | Transformed(ArrowMetadata) |
| ipc | NullGeometry | Preserved | Preserved |
| ipc | EmptyPoint | Preserved | Preserved |
| ipc | SignedZero | Preserved | Preserved |
| ipc | Subtypes | Rejected | Transformed(ArrowMetadata) |
| ipc | Aliases | Rejected | Transformed(ArrowMetadata) |
| ipc | Domains | Rejected | Transformed(ArrowMetadata) |
| ipc | TextStyle | Rejected | Transformed(ArrowMetadata) |
| ipc | PaperSpace | Rejected | Transformed(ArrowMetadata) |
| ipc | Xdata | Rejected | Transformed(ArrowMetadata) |
| ipc | Width | Rejected | Transformed(ArrowMetadata) |
| ipc | Thickness | Rejected | Transformed(ArrowMetadata) |
| ipc | Lineweight | Rejected | Transformed(ArrowMetadata) |
| ipc | Linetype | Rejected | Transformed(ArrowMetadata) |
| ipc | Color | Rejected | Transformed(ArrowMetadata) |
| ipc | Handle | Rejected | Transformed(ArrowMetadata) |
| ipc | DbfDate | Rejected | Preserved |
| ipc | Utf8 | Preserved | Preserved |
| ipc | Null | Preserved | Preserved |
| ipc | EmptyString | Preserved | Preserved |
| ipc | SignedInteger | Preserved | Preserved |
| ipc | UnsignedInteger | Preserved | Preserved |
| ipc | Floating | Preserved | Preserved |
| ipc | Boolean | Preserved | Preserved |
| ipc | Binary | Preserved | Preserved |
| ipc | Decimal | Preserved | Preserved |
| ipc | Temporal | Preserved | Preserved |
| ipc | NestedDictionary | Preserved | Preserved |
| ipc | FieldNames | Preserved | Preserved |
| ipc | NotNull | Preserved | Preserved |
| ipc | FieldOrder | Preserved | Preserved |
| ipc | SchemaMetadata | Transformed(ArrowMetadata) | Transformed(ArrowMetadata) |
| ipc | FieldMetadata | Transformed(ArrowMetadata) | Transformed(ArrowMetadata) |
| ipc | NativeMetadata | Transformed(ArrowMetadata) | Transformed(ArrowMetadata) |
| ipc | Z | Preserved | Preserved |
| ipc | M | Preserved | Preserved |
| ipc | Dimensions | Preserved | Preserved |
| ipc | GeometryType | Preserved | Preserved |
| ipc | RingOrientation | Preserved | Preserved |
| ipc | EmptyGeometry | Preserved | Preserved |
| ipc | CoordinatePrecision | Preserved | Preserved |
| ipc | CrsId | Preserved | Preserved |
| ipc | CrsDefinition | Preserved | Preserved |
| ipc | FeatureId | Preserved | Transformed(ArrowMetadata) |
| ipc | Bbox | Rejected | Transformed(ArrowMetadata) |
| ipc | ForeignMembers | Rejected | Transformed(ArrowMetadata) |
| ipc | LayerMetadata | Rejected | Transformed(ArrowMetadata) |
| ipc | Styles | Rejected | Transformed(ArrowMetadata) |
| ipc | Hierarchy | Rejected | Transformed(ArrowMetadata) |
| ipc | Extensions | Rejected | Transformed(ArrowMetadata) |
| ipc | ExternalReference | Rejected | Transformed(ArrowMetadata) |
| ipc | ExtendedData | Rejected | Transformed(ArrowMetadata) |
| ipc | FeatureMetadata | Rejected | Transformed(ArrowMetadata) |
| ipc | AltitudeMode | Rejected | Transformed(ArrowMetadata) |
| ipc | Inference | Rejected | Preserved |
| ipc | NumberPrecision | Preserved | Preserved |
| ipc | StructuredValue | Rejected | Preserved |
| ipc | MissingValue | Rejected | Preserved |
| ipc | CoordinateColumns | Rejected | Transformed(ArrowMetadata) |
| ipc | NumericFormat | Rejected | Transformed(ArrowMetadata) |
| ipc | Formula | Rejected | Transformed(ArrowMetadata) |
| ipc | FormulaCache | Rejected | Preserved |
| ipc | Identifier | Rejected | Transformed(ArrowMetadata) |
| ipc | Description | Rejected | Transformed(ArrowMetadata) |
| ipc | DataColumns | Rejected | Transformed(ArrowMetadata) |
| ipc | SqlConstraints | Rejected | Transformed(ArrowMetadata) |
| ipc | ContractVersion | Transformed(ArrowMetadata) | Transformed(ArrowMetadata) |
| ipc | FieldIdentity | Transformed(ArrowMetadata) | Transformed(ArrowMetadata) |
| ipc | TypesDeclaration | Transformed(ArrowMetadata) | Transformed(ArrowMetadata) |
| ipc | GeometryEncoding | Transformed(ArrowMetadata) | Transformed(ArrowMetadata) |
| ipc | SpatialSemantics | Transformed(ArrowMetadata) | Transformed(ArrowMetadata) |
| ipc | Precision | Transformed(ArrowMetadata) | Transformed(ArrowMetadata) |
| ipc | Srid | Transformed(ArrowMetadata) | Transformed(ArrowMetadata) |
| ipc | CrsResolution | Transformed(ArrowMetadata) | Transformed(ArrowMetadata) |
| ipc | DefinitionFormat | Transformed(ArrowMetadata) | Transformed(ArrowMetadata) |
| ipc | AxisOrder | Transformed(ArrowMetadata) | Transformed(ArrowMetadata) |
| ipc | PrimaryGeometry | Transformed(ArrowMetadata) | Transformed(ArrowMetadata) |
| ipc | ExtensionName | Transformed(ArrowMetadata) | Transformed(ArrowMetadata) |
| ipc | ExtensionMetadata | Transformed(ArrowMetadata) | Transformed(ArrowMetadata) |
| ipc | UnknownDimensions | Preserved | Preserved |
| ipc | Covering | Rejected | Transformed(ArrowMetadata) |

<!-- generato da codice/perdite: fine -->

### Confini nativi e diagnostiche

Il writer GeoPackage registra `OGC:CRS84` in `gpkg_spatial_ref_sys` come
`EPSG:4326`: per GeoJSON RFC 7946 le coordinate restano longitudine/latitudine,
senza inversione o riproiezione. Il confronto dello staging dichiara le effettive
differenze delle rappresentazioni CRS; non cambia la regola di uguaglianza di
`--assume-crs`.

La codepage DBF dichiarata da `.cpg` viene applicata, ma il companion non contiene
un'attestazione di appartenenza al dataset. Se due decoder ammettono gli stessi
byte, un `.cpg` estraneo non e' riconoscibile per inferenza. La fedelta' di lettura
riporta questo limite come `format_constraint`, senza inventare una perdita.

La disponibilita' del tier precede la validazione strutturale: senza
`gdal-backend`, ogni input indirizzato al driver FileGDB (per formato esplicito
o riconoscimento `.gdb`) produce `UNSUPPORTED`, categoria `unsupported`, con
un messaggio che nomina `gdal-backend`, anche se vuoto o corrotto. Il profilo
base non interpreta il catalogo o i membri per diagnosticare la struttura.

Con il tier disponibile, la regola di apertura FileGDB e' unica: il dataset e' un catalogo
(`a00000001.gdbtable` e accessori, con la sola tabella condivisa `GDB_Items` per schemi e metadati
nel formato v10, identificata dal catalogo e non da un ID fisso) piu' oggetti. Il parser unico dei basename stabilisce
l'identita' e i membri di ciascun oggetto.

- Un catalogo difettoso o illeggibile produce `InvalidContent` del dataset.
- Un membro vuoto, troncato, di contenuto non valido, non regolare,
  illeggibile, bloccato o con identita' ambigua riguarda soltanto il proprietario.
  L'oggetto resta elencato, non leggibile, con la categoria del rifiuto;
  i fratelli restano elencabili e selezionabili. Nessuna scelta implicita del
  solo oggetto leggibile tra piu' oggetti totali.
- Le varianti ASCII di caso di prefisso, cifre esadecimali ed estensione
  conservano l'identita' se univoche. Un file estraneo attribuibile a un oggetto
  e' rifiutato localmente; quelli non attribuibili, inclusi gli accessori globali
  non necessari alla lettura, producono una diagnostica di conteggio senza
  bloccare gli oggetti. Nessun contenuto estraneo viene interpretato come dato.
- Ogni apertura nativa e ogni lettore ricevono una vista privata costruita
  dal piano dichiarativo `operazione ? oggetto ? membri necessari`. La vista
  dell'elenco contiene il catalogo; quella di un oggetto contiene il catalogo
  e soltanto i suoi membri. Nessuna copia dell'intera directory, nessuna
  apertura, copia o lettura dei membri di un fratello da parte dell'oggetto.
  La vista resta viva fino alla chiusura del lettore.

Il piano e' condiviso da validazione strutturale, `layers`, `inspect`, `read`
e `convert`. Con `--layer`, `read` e `convert` trasmettono la selezione
all'apertura: anche la scoperta dello schema riguarda soltanto quell'oggetto,
con il suo ID originale. Nessuno schema o vista viene preparato per i fratelli.
Senza selezione, la scoperta completa conserva tutti gli stati degli oggetti. Le estensioni attribuibili senza decoder strutturale sono
rifiutate localmente: l'apertura non ne dichiara in silenzio l'integrita'.
Un `.gdbtable` o `.gdbtablx` assente produce `object_missing`; un membro
vuoto produce `object_empty`. Non si attiva il recupero euristico GDAL degli
indici: una lettura possibile con quel recupero non attesta l'integrita'.

Gli avvisi GDAL sono intercettati da un solo handler. Le operazioni e i worker
condividono soltanto un contatore di warning; testo, percorsi e nomi nativi non
raggiungono il chiamante. I warning compaiono come ragioni `format_constraint`
con classe e conteggio; le perdite sono accertate separatamente sullo staging.
Il confronto usa lo stesso schema della copia Arrow pubblica: versione e identita'
ricostruite identiche non sono perdite. I nomi dei layer FileGDB modificati sono
segnalati con `native_layer_metadata_not_preserved` e indice, senza il nome.

La tabella esaustiva `arrow_type_class` qualifica tutte le varianti fisiche Arrow;
la famiglia e' distinta dal sottoinsieme fisico supportato dal singolo formato.
I rifiuti di tipo indicano la famiglia e la fase `write`.
Il gate delle buste esegue casi distinti per le posizioni delle perdite native e
per le coercizioni di tipo; il manifesto viene confrontato con le forme osservate.
