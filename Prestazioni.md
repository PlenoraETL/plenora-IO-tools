# plenora-IO-tools — Vincoli prestazionali e di memoria

Documento compagno di `Architetture.md`. Definisce i vincoli prestazionali e di
memoria dell'I/O come **criteri di accettazione**, con lo stesso peso delle
invarianti di sicurezza. È il parallelo, sul bordo di I/O, di `Prestazioni.md`
dei `plenora-data-tools`.

## 1. Scopo

plenora-IO-tools deve essere progettata e valutata come una libreria di I/O:

- veloce in lettura e scrittura;
- a basso consumo di memoria;
- **streaming reale** dove il formato lo consente;
- **pass-through/zero-copy per Arrow IPC** e **decodifica colonnare diretta per
  Parquet** (non zero-copy letterale);
- con picco di memoria limitato anche su file molto grandi;
- prevedibile sotto input ostile (parser hardening).

Correttezza, fail-closed, publish atomico, fedeltà dichiarata restano
fondamentali, ma non devono introdurre overhead non necessario nel percorso di
lettura/scrittura.

Principio guida:

> Il bordo di I/O deve avvicinarsi al costo del formato stesso: leggere un
> Parquet non deve costare più che leggerlo, più il minimo indispensabile per
> renderlo un `RecordBatch` conforme.

---

## 2. Vincoli fondamentali

### V1 — Streaming reale dove il formato lo consente

Un driver classificato `Streaming` deve leggere/scrivere a memoria limitata
rispetto alla dimensione totale del file.

Formati **streamabili** (memoria quasi costante): CSV (a righe), GeoParquet (per
row group), Arrow IPC (per batch), `geojson-nd` (per feature), e il `geojson`
`FeatureCollection` **quando il driver usa un parser incrementale dell'array
`features`** (`StreamingSequential`). Formati **materializzanti *nella v1***
(bounded dai limiti, dichiarati nel descrittore): DXF, KML, XLSX (con la
**shared string table** come costo di memoria da limitare/spillare). La classe
riflette il **driver reale**, non il formato astratto, e **non è permanente**
(Architetture §2.3, D9): hanno una via streaming futura — DXF può, dopo una
passata preliminare sulla block table, emettere le entità progressivamente; KML
si legge con un parser XML pull/event senza DOM completo; XLSX spesso per righe
(shared strings e formule complicano, non impediscono). Un driver materializzante
**deve dichiararlo** e rispettare i limiti (`max_rows`, `max_input_bytes`), mai un
caricamento illimitato — e potrà passare a streaming senza cambiare il contratto.

Criterio di accettazione:

> Su un driver streamabile, aumentando la dimensione dell'input di 100 volte, il
> picco di memoria resta quasi costante, salvo la dimensione del batch e le
> strutture fisse.

### V2 — Da sorgenti colonnari: pass-through IPC, decodifica colonnare Parquet

Il termine "zero-copy" va usato con cautela: vale letteralmente per Arrow IPC,
**non** per Parquet, che è un formato codificato e spesso compresso e richiede
comunque decomprimere → decodificare → ricostruire array Arrow. La formulazione
corretta:

- **Arrow IPC** → **zero-copy / pass-through** dei buffer dove consentito (al più
  validazione del contratto), niente ricostruzione.
- **GeoParquet** → **decodifica colonnare diretta** di arrow-rs, senza
  rappresentazioni row-oriented intermedie e **senza copie ulteriori** oltre a
  quelle intrinseche alla decodifica del formato.

In entrambi i casi la regressione da evitare è **ricostruire il batch
cella-per-cella** da una sorgente colonnare: la decodifica di Parquet è
inevitabile, la ri-riga-orientazione no.

### V3 — Nessuna doppia conversione via GeoJSON

Il vecchio perno era GeoJSON; il nuovo è `RecordBatch`. Un driver **non** deve
passare per GeoJSON come intermediario:

```text
VIETATO:   file → GeoJSON → RecordBatch
RICHIESTO: file → RecordBatch
```

GeoJSON è un driver come gli altri; usarlo come ponte interno raddoppierebbe
parsing e allocazioni. Invariante verificata nei test.

### V4 — Decode/encode WKB minimizzato

Una geometria attraversa il bordo I/O con **al più un decode e un encode** WKB:

- in lettura: rappresentazione nativa del formato → WKB (una volta), scritto
  nella colonna `geoarrow.wkb`;
- in scrittura: WKB della colonna → rappresentazione del formato (una volta).

Niente catene ridondanti `WKT → WKB → geo::Geometry → WKB`. La garanzia precisa
è **zero decode geometrico quando il payload è già WKB compatibile**, *non* zero
parsing né zero slicing del buffer:

- **GeoPackage** non è WKB puro: il blob è `GeoPackageBinaryHeader` (magic, flag,
  SRS ID, envelope opzionale) + payload WKB. Il payload si riusa senza decode in
  `geo::Geometry`, ma l'header va estratto e il WKB validato.
- **GeoParquet** può avere encoding/metadati (`geo`) da normalizzare prima di
  considerare compatibile il payload.

Quindi il beneficio è **saltare la decodifica/ricodifica in `geo::Geometry`**
quando WKB e CRS coincidono, non saltare il parsing del contenitore.

### V5 — Parsing incrementale

I parser dei formati sequenziali (CSV, Parquet per row group, IPC) leggono a
blocchi, senza caricare l'intero file in memoria. Il buffer di lettura è
limitato; il produttore di batch applica backpressure verso il consumatore.

### V6 — Projection pushdown e pruning (non filtering)

Distinzione netta (Architetture §6.3, ADR-IO 6): plenora-IO-tools fa **pruning**,
non **filtering**.

- **Projection pushdown**: leggere solo le colonne dichiarate. Sempre lecito sui
  formati colonnari (Parquet/IPC).
- **Pruning**: saltare i blocchi **sicuramente** incompatibili usando i
  **metadati** (min/max dei row group Parquet, indice spaziale del GeoPackage,
  partizioni). Non leggere ciò che di certo non serve.
- **Filtering** riga-per-riga: **non** è compito dell'I/O, resta in
  `plenora-data-tools`.

Il `pruning_predicate`/`spatial_pruning_hint` del `ReadRequest` è un
*suggerimento di pruning* (i nomi lo dicono): onorato solo con capacità native
equivalenti e documentate, altrimenti ignorato (tutte le righe passano, il filtro
esatto lo fa data-tools). Un driver non deve mai **approssimare** un filtro. La
regressione da evitare è **leggere e poi scartare** ciò che i metadati
permettevano di saltare con certezza.

### V7 — Batch sizing controllato

Il batch size è configurabile e preferibilmente adattivo, in **byte oltre che
righe** (`target_batch_bytes`, `max_batch_bytes`). Per le geometrie il limite in
byte è prioritario: un batch di pochi multipoligoni enormi può superare la quota
pur avendo poche righe.

### V8 — Scrittura in streaming, publish senza copie extra

La scrittura non bufferizza l'intero output prima di scrivere: i batch fluiscono
verso il writer del formato man mano che arrivano. Il publish atomico
(tempfile/staging + rename) **non ricopia** il contenuto: sposta, non duplica.
La destinazione è toccata solo a successo.

### V9 — Overhead di dispatch trascurabile

La selezione del driver (estensione/magic/`--format`) avviene **una volta** in
`open`/`create`, mai nel percorso per-batch. Nessun dispatch dinamico per riga
o per cella.

### V10 — Limiti prima delle allocazioni

Ogni limite (dimensione file, righe, colonne, byte per cella WKB, componenti,
profondità, byte stringa, dimensione metadati Parquet, dimensione nodo XML) è
applicato **prima** dell'allocazione guidata dal contenuto. Un parser ostile —
CSV con una riga da gigabyte, KML/XML bomb, Parquet con footer gonfio, WKB con
conteggi enormi — deve fallire in modo limitato e tipizzato, non consumare
memoria. Le guardie WKB (celle, conteggi, profondità) sono già implementate e
fuzzate.

---

## 3. Vincoli sul consumo di memoria

### M1 — Budget principale in byte

Il contabilizzatore di risorse dell'I/O opera in byte: buffer di lettura, batch
in volo, dictionary, geometrie decodificate temporanee, tabelle CRS, indice
`.shx`/`.dbf` per lo Shapefile, DOM XML per KML, footer/metadati Parquet. I
limiti di righe proteggono da espansioni logiche, non rappresentano la memoria
reale.

### M2 — Materializzazione bounded per i formati non streamabili

Un driver materializzante non è un free-for-all: rispetta `max_input_bytes` e
`max_rows` **prima** di costruire la rappresentazione in memoria, e produce i
batch da lì. Il picco è dichiarato e misurato per driver (es. DXF: già misurato
~2.3 GiB/1M feature nel tool attuale).

### M3 — Overhead di contabilizzazione limitato

Nessuna scansione ricorsiva del batch a ogni passaggio; nessun conteggio per
riga; nessuna copia per rendere la memoria contabilizzabile. La contabilità è
per batch/buffer.

### M4 — Nessuno spill necessario nell'I/O puro

L'I/O è per lo più streaming: non richiede spill. L'eccezione sono i formati ad
accesso casuale che devono tenere strutture indice (Shapefile `.dbf`, KML DOM):
lì vale M2 (bounded), non lo spill. Lo spill resta un problema dei data-tools,
non del bordo.

---

## 4. Vincoli specifici per formato

### 4.1 Colonnari (GeoParquet, Arrow IPC)

- Lettura colonnare nativa, **projection pushdown + row-group pruning** (V6),
  pass-through dove possibile (V2). Sono i formati dove il bordo deve costare
  **quasi zero** oltre la decodifica del formato stesso.

### 4.2 Sequenziali testuali (CSV, GeoJSON-nd)

- Parsing incrementale a righe (V5), inferenza tipi in streaming, WKB dalla
  colonna geometria dichiarata senza materializzare l'intero file.

### 4.3 WKB-nativi (GeoPackage)

- Idealmente byte pass-through della geometria (V4): il payload geometrico del
  GeoPackage contiene WKB compatibile **dopo estrazione e validazione
  dell'header GeoPackage** (magic/flag/SRS ID/envelope); con CRS coincidente non
  serve ridecodificare in `geo::Geometry`, solo validare.

### 4.4 Materializzanti (DXF, KML, XLSX)

- Bounded (M2), fedeltà dichiarata; il costo di tassellazione (DXF
  `--arc-segments`) è governato dal chiamante e misurato (G-benchmark).

---

## 5. Metriche obbligatorie

Ogni benchmark per driver raccoglie almeno:

```text
rows/s              MB/s               wall time
CPU time            peak RSS           bytes allocated
allocation count    bytes copied       WKB decode count
WKB encode count    average batch bytes  max batch bytes
```

Per i driver geografici, anche:

```text
geometries/s        coordinates/s      average WKB bytes
features skipped (driver approssimanti)   parquet row groups skipped (pushdown)
```

---

## 6. Invarianti prestazionali

Criteri di accettazione verificabili, complementari alle invarianti di sicurezza
(§10 di `Architetture.md`):

1. Un driver streamabile non cresce linearmente in memoria con la dimensione del
   file.
2. Da sorgente colonnare/IPC non si ricostruisce il batch riga per riga.
3. Nessuna conversione passa per GeoJSON come intermediario.
4. Ogni geometria attraversa il bordo con ≤1 decode e ≤1 encode WKB; **0 decode
   geometrico** quando il payload WKB è compatibile (l'header del contenitore —
   es. GeoPackage — va comunque letto e validato).
5. Nessun parsing carica in memoria un intero file sequenziale.
6. Il **pruning** (colonne + blocchi esclusi via metadati) è effettivo quando il
   formato lo consente; il **filtering riga-per-riga non è compito dell'I/O**
   (resta a data-tools), e nessun driver approssima un filtro.
7. La scrittura non bufferizza tutto l'output prima di pubblicare.
8. Il publish sposta, non ricopia; la destinazione è toccata solo a successo.
9. Il dispatch del driver non avviene nel percorso per-batch.
10. Ogni limite è applicato prima dell'allocazione guidata dal contenuto.
11. I driver materializzanti rispettano i limiti prima di costruire la
    rappresentazione in memoria.
12. Ogni regressione significativa rispetto ai tool originari blocca il rilascio.
13. Il picco di memoria è misurato e riportato per ogni benchmark principale.

---

## 7. Benchmark gate

### 7.1 Per driver (lettura e scrittura)

- CSV: filter-free read/write su 1, 10, 100 M righe; inferenza tipi.
- GeoParquet: read con e senza projection pushdown e pruning per row group
  (metadati min/max); write per compressione (snappy/gzip/…); confronto con
  pyarrow.
- GeoPackage: read/write, geometria WKB pass-through vs ridecodifica.
- Shapefile: read/write set multi-file; publish atomico staging.
- GeoJSON: read/write, streaming vs whole-file.
- DXF: read con tassellazione a densità crescenti (`--arc-segments`), picco RSS;
  fedeltà (feature skipped).
- KML/XLSX: read materializzante bounded.

### 7.2 Round-trip e interop

- Round-trip `file → RecordBatch → file` per ogni driver bidirezionale:
  uguaglianza semantica (geometrie con ADR 1 dei data-tools, non byte WKB).
- Cross-formato: `shp → RecordBatch → gpkg`, `gpkg → RecordBatch → geoparquet`,
  ecc.; confronto estensioni/feature con **GDAL `ogr2ogr`/`ogrinfo`** e
  **pyarrow** come oracoli indipendenti (già disciplina della famiglia:
  DXF↔GDAL, GeoParquet↔pyarrow).

### 7.3 Memoria e input ostile

- Streaming su input crescente (memoria quasi costante).
- Parser hardening: CSV a righe enormi, XML bomb, Parquet footer gonfio, WKB
  malevolo → fallimento limitato e tipizzato, picco controllato.

---

## 8. Budget di regressione

Ogni rilascio si confronta con:

- i tool originari (`plenora-*-tools`);
- la release precedente;
- una baseline archiviata (Fase 1).

Soglie esplicite (definitive dopo la baseline), per esempio:

- nessuna regressione > 5% sul throughput read/write dei driver principali;
- nessun aumento del picco RSS > 5% sui driver streamabili;
- nessuna copia aggiuntiva di buffer nei casi dichiarati zero-copy/pass-through;
- nessun aumento del numero di decode/encode WKB per geometria;
- pushdown Parquet effettivo (row group saltati > 0 quando il filtro lo consente).

---

## 9. Roadmap prestazionale

Allineata alle fasi di `Architetture.md` §8.

- **Fase 1 — Baseline**: driver ritargettati su `RecordBatch`; benchmark
  read/write di ogni driver; throughput, allocazioni, picco RSS; dataset
  sintetici e reali (corpus RFI DXF, GeoParquet reali se disponibili); baseline
  in CI.
- **Fase 2A — Streaming**: streaming reale sui formati che lo consentono;
  materializzanti bounded; metriche.
- **Fase 2B — Interop e multi-file**: oracoli GDAL/pyarrow come gate; publish
  multi-file misurato.
- **Fase 2C — Ottimizzazioni fondamentali**: pass-through IPC, decodifica
  colonnare Parquet con **projection pushdown + row-group pruning**, WKB
  pass-through GeoPackage, batch sizing adattivo.
- **Fase 3 — Avanzate**: GeoArrow nativo opzionale, formati aggiuntivi (GML,
  FlatGeobuf), solo dietro benchmark.

---

## 10. Criterio di successo

plenora-IO-tools è conforme ai propri obiettivi solo se:

- il costo del bordo si avvicina al costo del formato (poco overhead oltre la
  decodifica/codifica);
- i driver streamabili girano a memoria quasi costante;
- legge Arrow IPC in pass-through/zero-copy e Parquet con decodifica colonnare
  diretta, projection pushdown e row-group pruning;
- non introduce doppie conversioni né decode/encode WKB ridondanti;
- i driver materializzanti restano bounded e dichiarati;
- l'interop è dimostrata contro oracoli indipendenti (GDAL, pyarrow);
- dimostra tutto con benchmark riproducibili e budget di regressione.

Principio finale:

> plenora-IO-tools non deve essere solo corretta e fedele: deve dimostrare, con
> benchmark e limiti verificabili, di leggere e scrivere **al costo del formato,
> più il minimo per essere Arrow**.

---

## 11. Evidenza 2026-07-28 — pulizia e contratti trasversali

Baseline `1c1ee61` e post compilati ed eseguiti nello stesso container
Linux/Rust 1.92.0 e con lo stesso harness: 100.000 righe, geometria Point e
mediana di cinque ripetizioni. Soglia di veto: regressione throughput o aumento
RSS oltre il 5%.

| Driver/operazione | Throughput | Picco RSS | Esito |
|---|---:|---:|---|
| DXF read | +1,31% | +0,13% | OK |
| DXF write | +3,51% | +0,08% | OK |
| KML read | +2,50% | +0,03% | OK |
| KML write | +64,32% | -85,63% | OK |
| XLSX read | -0,62% | +0,29% | OK |
| XLSX write | -0,79% | +0,00% | OK |

Il gate comparativo è superato. La baseline versionata è
`baseline/streaming-before.json`; il post locale è
`target/paired-after.json`.
