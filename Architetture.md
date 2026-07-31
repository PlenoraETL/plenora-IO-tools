# plenora-IO-tools — Architettura

> **Nota di baseline (2026-07-27).** Le parti storiche di questo documento che
> chiamano `plenora-core` un crate già condiviso sono superate dal contratto
> trasversale `plenora-contracts` `v2.0-rc2`. Il modello oggi interno a questo
> repository si chiama `plenora-io-model`; il futuro package condiviso si
> chiamerà `plenora-contracts` e verrà estratto soltanto dopo la ratifica di
> §15.3. Questa distinzione chiude le collisioni R8.1/R8.4 senza anticipare
> l’API ancora proposta.

Libreria unica di **I/O** per formati tabellari e geografici. È il layer che
l'architettura di `plenora-data-tools` lascia esplicitamente fuori scope e
rimanda a un futuro `plenora-datafile` (§3.5 e Fase 4 di quel documento):
**plenora-IO-tools È quel `plenora-datafile`**. Legge un file e produce
`RecordBatch` Arrow conformi al contratto di input di `plenora-data-tools`
(§4.1); prende `RecordBatch` conformi e scrive un file. Non trasforma i dati:
li fa entrare e uscire.

Nasce dalla **convergenza** dei tool I/O già costruiti, ciascuno oggi binario a
sé con perno GeoJSON:

- `plenora-shp-tools` (Shapefile, puro Rust)
- `plenora-gdb-tools` (FileGDB, tier GDAL)
- `plenora-gpkg-tools` (GeoPackage, puro Rust, WKB in-tree)
- `plenora-kml-tools` (KML/KMZ, puro Rust)
- `plenora-dxf-tools` (DXF, puro Rust, approximating)
- `plenora-csv-tools` (CSV, puro Rust)
- `plenora-xls-tools` (XLSX, puro Rust)
- `plenora-geoparquet-tools` (GeoParquet, puro Rust, Arrow+WKB)
- il lettore/scrittore GeoJSON (finora l'hub) diventa **un driver come gli altri**

Diventano **driver** dietro un'unica astrazione, sopra un core condiviso.

Principi ereditati e non negoziabili — **criteri di accettazione**, non slogan
(gli stessi di `plenora-data-tools`, qui applicati al bordo I/O):

- `#![forbid(unsafe_code)]` in tutti i crate puro-Rust.
- **Fail-closed**: validazione dei contratti prima di toccare i dati, campi
  sconosciuti rifiutati, nessun output parziale, publish atomico (tempfile +
  persist no-clobber; staging dir per formati multi-file).
- Limiti di risorsa applicati **prima** delle allocazioni.
- Errori senza dati sensibili: contesto (driver, layer, motivo), mai valori di
  cella.
- **Nessuna riproiezione**: plenora-IO-tools *legge* e *scrive* il CRS, non lo
  trasforma mai. La riproiezione è lo step `geo.reproject` di
  `plenora-data-tools`. (Conseguenza di migrazione: il `--reproject-wgs84` oggi
  in `plenora-dxf-tools` esce dal driver e diventa un passo di pipeline.)
- **Puro Rust di default; GDAL solo dove il formato non lascia alternative —
  oggi soltanto FileGDB** (policy di famiglia). GDAL è un driver a feature,
  "tier GDB", con deploy a container e stream CVE separato.
- **Prestazioni e memoria sono criteri di accettazione**: streaming reale dove
  il formato lo consente, **pass-through/zero-copy per Arrow IPC** e
  **decodifica colonnare diretta per Parquet** (non zero-copy letterale),
  nessuna doppia conversione via GeoJSON, ≤1 decode/encode WKB per geometria nel
  percorso I/O. I vincoli completi sono in `Prestazioni.md`, documento compagno
  con lo stesso peso di questo.

---

## 1. Modello mentale

plenora-IO-tools è un **adattatore bidirezionale** file ⇄ Arrow. Non è un motore
di query: non filtra, non aggrega, non riproietta. Fa una cosa e la fa bene.

```
 file.shp  ─┐                               ┌─→ file.gpkg
 file.gpkg ─┤                               │
 file.dxf  ─┼─ driver.read ─→ RecordBatch ──┼─→ plenora-data-tools ─→ RecordBatch ─→ driver.write
 file.csv  ─┤   (geoarrow.wkb + geo.crs)    │
 geojson   ─┤                               └─→ geoparquet
 geoparquet─┘
```

Il `RecordBatch` prodotto **è esattamente** ciò che `plenora-data-tools`
accetta (§4.1 di quel documento): stesso set di tipi chiuso, geometria come
colonna `Binary` `geoarrow.wkb` + `geo.crs`, ≤1 colonna geometrica nella v1. I
due si innestano **senza conversione**: stessa versione di Arrow, stesso
`plenora-core` (§3, decisione D0). Se un domani esiste un unico eseguibile
`plenora`, un comando `convert a.shp b.gpkg` è `read` di un driver + `write` di
un altro, con in mezzo eventuali step di data-tools.

L'API pubblica ha, per ogni verso, **due fasi** (parallele a `validate`/`execute`
dei data-tools):

```
Lettura:
  open(source, options)  -> Result<Box<dyn OpenDatasetHandle>>   // statico: header/schema/CRS, niente righe
  open_layer_reader(&OpenDatasetHandle, request) -> Result<Box<dyn LayerReader>>  // apre uno stream per layer
  next_batch(&mut LayerReader) -> Result<Option<RecordBatch>>    // dinamico: pull + validazione contenuti

Scrittura:
  create(sink, plan, options) -> Result<Box<dyn FormatWriter>>   // statico: il contratto è rappresentabile nel formato?
  write(&mut FormatWriter, batches) -> Result<()>                // streaming
  finish(FormatWriter)              -> Result<Published>         // publish atomico solo a successo
```

`open_layer_reader` accetta solo il prodotto di `open`; `write` solo il prodotto
di `create`. Non esiste un modo di leggere righe senza aver validato l'header, né
di scrivere senza aver verificato che il contratto sia esprimibile nel formato di
destinazione. (Firma completa dei `trait` in §5.1.)

---

## 2. I contratti

### 2.1 Contratto di formato (il file, bordo esterno)

Un file valido è ciò che il **driver** dichiara di saper leggere: firma/magic
riconosciuta, struttura del contenitore integra, entro i limiti di dimensione.
La validazione dell'header è statica (fase `open`); i contenuti delle celle
sono validati **incrementalmente** durante `read` (fase dinamica, §4). Un
formato multi-file (Shapefile: `.shp`+`.shx`+`.dbf`+`.prj`) è un solo
contratto: mancanze o incoerenze tra i componenti falliscono in `open`.

### 2.2 Contratto RecordBatch (bordo interno — definito in `plenora-core`)

**Non è ridefinito qui, e non "appartiene" ai data-tools.** La fonte normativa
è **`plenora-core`**: il contratto vive lì ed è **condiviso alla pari** da
plenora-IO-tools e plenora-data-tools. Il §4.1 dei data-tools è solo il luogo
*storico* in cui è stato specificato per primo; la definizione canonica è in
`plenora-core`, così nessuna delle due librerie di pari livello è subordinata
all'altra. plenora-IO-tools ne è produttore e consumatore:

- Arrow IPC / `RecordBatch` in memoria.
- Set di tipi **chiuso**: `Utf8`/`LargeUtf8` (normalizzato), `Int64`, `UInt64`,
  `Float64`, `Boolean`, `Date32`, `Timestamp(ms, tz)`, `Decimal128`, `Binary`,
  `Dictionary`, `List`/`Struct`.
- Geometria riconosciuta **solo** tramite metadati di estensione
  `ARROW:extension:name = geoarrow.wkb` + `geo.crs` obbligatorio e risolvibile.
  Una colonna `Binary` senza metadati è byte, non geometria.
- **v1: al massimo una colonna geometrica** (D16 dei data-tools).
- Validazione strutturale WKB per cella (64 MiB/cella, 100k componenti,
  profondità 64).

Poiché questo contratto è **condiviso**, vive in `plenora-core` (§3), non
duplicato: le due librerie non possono divergere su cosa sia un batch valido.

### 2.3 Contratto di driver (catalogo — macchina-leggibile)

Ogni driver dichiara un `FormatDescriptor` (parallelo all'`OperationDescriptor`
dei data-tools §4.3):

| Campo | Significato |
|---|---|
| `id` | `shp` / `gpkg` / `dxf` / `geojson` / `geoparquet` / … |
| `direction` | `read` / `write` / `bidirectional` |
| `streaming` | `{read: Streaming\|Materializing, write: Streaming\|Buffered}` |
| `multi_layer` | il formato ha più layer/tabelle (gpkg, gdb, dxf, kml, xlsx) o uno solo |
| `multi_file` | il formato è un set di file (shp) → publish a staging dir |
| `geometry_support` | tipi geometrici, singola/multipla colonna, dimensioni (v1: XY) |
| `type_support` | quali tipi del set chiuso il formato sa rappresentare |
| `crs_handling` | `Embedded` (shp/gpkg/geoparquet/dxf) / `FixedWgs84` (kml/geojson) / `None` (csv/xlsx) |
| `fidelity` | `Lossless` / `Approximating` (DXF: tassellazione, esplosione blocchi) |
| `runtime` | `PureRust` / `Gdal` (tier GDB) |
| `required_capabilities` | backend/feature necessari (`gdal`) — verificati in `open`/`create` |
| `semantic_version`, `driver_version`, `descriptor_version` | versioni esplicite; il fingerprint del catalogo deriva da queste, mai da hash del binario (come D17) |
| `maturity` / `support_level` | pipeline di promozione |

- **Fedeltà dichiarata** (dal lavoro DXF): un driver `Approximating` deve
  dichiararlo e **riportare cosa scarta** (skipped report), mai perdere in
  silenzio. La tassellazione archi/bulge e l'esplosione blocchi di DXF restano
  approssimazioni dichiarate.
- **`streaming` è una proprietà per-driver e per-versione, non permanente**
  (decisione D9): `Materializing` significa "materializzante *nella v1*", non
  "per sempre". Diversi formati oggi materializzanti hanno una via streaming
  futura: DXF può, dopo una passata preliminare sulla block table, emettere le
  entità progressivamente; KML si legge con un parser XML **pull/event-based**
  senza DOM completo; XLSX spesso si legge **per righe** — ma la **shared string
  table** può occupare molta memoria: uno "streaming XLSX" onesto è righe in
  streaming **+ dizionario condiviso bounded o spillabile + limiti specifici
  sulla shared string table** (le formule complicano, non impediscono). Il
  descrittore lascia aperta una futura modalità streaming/semi-streaming **senza
  cambiare il contratto** né il `trait`.
- **GeoJSON è due varianti** (decisione D9): `geojson-nd` (newline-delimited),
  streamabile per feature; `geojson` (`FeatureCollection` standard). Quest'ultimo
  **non è materializzante per natura**: un parser **incrementale dell'array
  `features`** lo legge in streaming senza caricare l'intero documento
  (`StreamingSequential`). La classificazione nel descrittore deve riflettere il
  **driver reale**, non il formato in astratto: un'implementazione a DOM completo
  è `Materializing`, una a parser incrementale è `StreamingSequential`. Il
  descrittore distingue le due varianti come `id`/modalità del driver.
- Un driver senza `FormatDescriptor` completo non entra nel registro.

---

## 3. Rappresentazione e versioni (decisione D0)

**Arrow è l'unica rappresentazione interna. La geometria è `geoarrow.wkb`.**
Identico ai data-tools §2 — perché è **lo stesso `plenora-core`**.

- **`plenora-core` è condiviso con `plenora-data-tools`**, non una copia: re-export
  di Arrow, convenzione colonna geometria (metadati estensione, `geo.crs`),
  contratto CRS (`ResolvedCrs`, `resolve_crs`), codec + validazione WKB in-tree
  (già scritto e fuzzato in `plenora-geoparquet-tools`/`plenora-geo-tools-arrow`),
  `Limits`, base degli errori. **Unica fonte di verità del bordo.** Se `plenora`
  diventa un monorepo, `plenora-core` è una sola crate usata da entrambe le
  librerie; se restano repo separati, plenora-IO-tools dipende dalla `plenora-core`
  pubblicata dai data-tools.
- **Arrow e Parquet pinnati `=59.1.0`** — **lo stesso pin dei data-tools**
  (loro D0). È un vincolo duro, non una preferenza: se le due librerie usassero
  versioni Arrow diverse, i loro `RecordBatch` sarebbero tipi diversi e
  passarli richiederebbe conversione. Conseguenza concreta: il driver
  GeoParquet, oggi su `parquet =54.3.1`, va **portato a `parquet =59.1.0`**
  (arrow-rs rilascia `arrow` e `parquet` in lockstep). Niente crate `geoarrow`
  (richiederebbe Arrow ^58 e non serve: WKB in-tree).
- I tipi Arrow sono parte del **contratto pubblico**: un bump di Arrow è
  potenzialmente breaking e va dichiarato, mai nascosto.

---

## 4. Le due fasi e la validazione statica/dinamica (decisione D1, eredita D8)

Come nei data-tools la validazione si divide in due (loro D8):

- **Statica** (`open` / `create`): firma e struttura del contenitore, schema,
  metadati, presenza e risolvibilità del CRS, capability del backend (GDAL),
  **e in scrittura**: verifica che il contratto in ingresso sia
  **rappresentabile** nel formato di destinazione (§4.1). Nessuna riga letta.
- **Dinamica incrementale** (`read` / `write`, durante lo streaming): struttura
  WKB di ogni cella, coordinate finite, profondità/componenti, conformità dei
  tipi al set chiuso, dimensionalità e nullability della colonna geometrica,
  limiti di righe/byte/celle. Nessun output parziale mai prodotto.

Corollario onesto: gli errori di **formato/contratto** emergono prima di leggere
i dati; gli errori di **contenuto** emergono in streaming, prima che un batch
non valido raggiunga il consumatore.

### 4.1 Scrittura: capability-check statico (fail-closed)

Ogni formato rappresenta un sottoinsieme diverso del contratto. `create` rifiuta
in anticipo, con errore tipizzato, ciò che il formato non regge:

- **Shapefile**: nomi campo ≤10 char, tipi DBF limitati, **un solo tipo
  geometrico per file**, niente null espliciti su alcuni tipi → mappatura
  documentata o errore.
- **DXF**: nessuna tabella attributi generica → gli attributi diventano timbri
  INSERT o proprietà `dropped`, mai persi in silenzio (comportamento attuale).
- **CSV/XLSX**: nessun CRS, geometria da codificare (WKT o colonne x/y) →
  scelta esplicita.
- **KML/GeoJSON**: WGS84 per specifica → un CRS diverso in ingresso richiede
  riproiezione a monte (step data-tools), qui è un errore dichiarato.

La regola: **struttura aperta, comportamento chiuso** — il modello ammette tutto,
il singolo driver rifiuta ciò che non sa rappresentare, in `create`, non a metà
scrittura.

---

## 5. Struttura del workspace

Parallela ai data-tools (§3): un crate per driver + facade + CLI sottile, sopra
il `plenora-core` **condiviso**. Attenzione (criticità nota): `plenora-core`
**non è un crate locale duplicato** dentro plenora-IO-tools — sarebbe una seconda
crate con lo stesso ruolo, esattamente ciò che vogliamo evitare. È una
**dipendenza**:

- **se i progetti restano separati**: `plenora-core = "=X.Y.Z"` (dipendenza
  versionata pubblicata, non ricopiata);
- **se diventano un monorepo** (raccomandato, §5.2): un solo `plenora-core` nel
  workspace, usato sia dai data-tools sia dai driver I/O — è il modo più solido
  per **azzerare** il rischio di skew Arrow e di divergenza dei contratti.

```
# Layout consigliato: MONOREPO unico
plenora/
├── Cargo.toml                     # workspace + [workspace.dependencies] (Arrow/Parquet =59.1.0)
├── crates/
│   ├── plenora-core/              # UNICO: Arrow, geometria geoarrow.wkb, CRS, WKB, Limits, errori base, contratto RecordBatch
│   ├── plenora-data-tools/        # il motore (validate/execute, kernels, engine)
│   ├── plenora-io-core/           # trait FormatDriver/OpenDatasetHandle/LayerReader, ReadRequest, publish atomico, registro, capabilities
│   ├── driver-csv/  driver-geojson/  driver-geoparquet/
│   ├── driver-shp/  driver-gpkg/  driver-kml/  driver-dxf/  driver-xlsx/
│   ├── driver-filegdb/            # feature `gdal-backend` (tier GDB)
│   ├── plenora-io/                # facade: registra i driver, dispatch per formato/estensione/magic
│   └── plenora-cli/               # binario sottile (I/O + eventuale orchestrazione data-tools)
├── fuzz/                          # target per parser (WKB, CSV, KML/XML, DXF, geojson)
├── tests/                         # matrice avversaria, round-trip cross-formato
└── reference/                     # oracoli interop (ogrinfo/ogr2ogr, pyarrow)
```

Se si preferisce tenere i repo separati, `plenora-IO-tools/` è lo stesso albero
**senza** `plenora-core/` e `plenora-data-tools/` (entrambi dipendenze esterne).

### 5.1 plenora-io-core

- **`trait FormatDriver`** — il confine plug-in. I driver hanno stati molto
  diversi (file handle, mmap, parser XML pull, connessione SQLite, handle GDAL,
  indici, staging dir): il dataset aperto e il writer sono **trait object per
  driver**, non un unico tipo concreto (decisione D10). La facade resta dinamica;
  internamente ogni driver può monomorfizzare il proprio stato.

```rust
trait FormatDriver {
    fn descriptor(&self) -> &FormatDescriptor;
    fn open(&self, source: Source, opts: &ReadOptions) -> Result<Box<dyn OpenDatasetHandle>>;
    fn create(&self, sink: Sink, plan: &WritePlan, opts: &WriteOptions) -> Result<Box<dyn FormatWriter>>;
}

trait OpenDatasetHandle {
    fn layers(&self) -> &[LayerContract];                    // contratto per layer, statico (immutabile)
    /// Apre un reader INDIPENDENTE per un layer: è il reader a portare lo stato
    /// mutabile (cursore, parser streaming, connessione SQLite, handle GDAL,
    /// reader non clonabile), non l'handle condiviso. Più `open_layer_reader`
    /// danno stream indipendenti quando il formato lo consente (decisione D10).
    fn open_layer_reader(&self, request: &ReadRequest) -> Result<Box<dyn LayerReader>>;
}

trait LayerReader {
    /// Iteratore fallibile CON stato: `&mut self` copre cursori e parser.
    fn next_batch(&mut self) -> Result<Option<RecordBatch>>;  // validazione dinamica per batch
}

/// La richiesta di lettura porta già projection e (eventuale) pruning: senza,
/// il lettore Parquet non saprebbe quali colonne o row group saltare
/// (decisione D8). I nomi dicono "pruning", non "filter": l'API stessa impedisce
/// l'equivoco. Molti driver v1 ignorano i campi che non sanno onorare.
struct ReadRequest {
    layer: LayerId,
    projected_fields: Option<Vec<FieldId>>,        // projection pushdown (I/O puro)
    pruning_predicate: Option<PruningPredicate>,   // SUGGERIMENTO di pruning via metadati (§6.3), non un filtro
    spatial_pruning_hint: Option<Bbox>,            // pruning spaziale (indice/statistiche)
    batch_target: BatchTarget,                     // target_batch_bytes / max_rows
}
```

- `OpenDatasetHandle`: elenco layer, per ciascuno il `LayerContract` (schema
  Arrow + contratto geometria + CRS risolto + proprietà con
  `PropertyConfidence`/`Scope` come D25 dei data-tools — cardinalità/bbox/unicità
  **mai** `Proven` dagli header).
- `LayerReader`: reader pull-based con stato (`next_batch(&mut self) ->
  Option<RecordBatch>`), validazione dinamica per batch; per i formati
  **materializzanti nella v1** (DXF, KML, XLSX) rende uno o pochi batch,
  **bounded dai limiti**, non un caricamento illimitato — ma la classificazione
  è per-driver e per-versione, non permanente (§2.3, D9).
- **`FormatWriter` e semantica di scrittura v1** (decisione D11): un `create`
  produce un **dataset nuovo** e ne pubblica **tutti i layer insieme**, in un
  unico commit atomico. Il `WritePlan` descrive gli 1..N layer (per un
  contenitore multi-layer come GeoPackage) con i rispettivi contratti;
  `finish` li pubblica o fallisce in blocco. **Nella v1 niente append né
  modifica in place** su un dataset esistente: mantiene le garanzie semplici
  (o tutto il dataset o niente). Append/merge sono rimandati a una versione
  successiva.
- **Publish atomico** condiviso (non reimplementato per driver): tempfile +
  persist no-clobber per file singolo; **staging dir + rename** per formati
  multi-file (il set Shapefile appare o tutto o niente) e per i contenitori
  multi-layer (il GeoPackage con tutti i suoi layer appare o tutto o niente).
  Due profili come D22 dei data-tools: `AtomicPublish` / `DurableAtomicPublish`
  (fsync file+dir).
- **Registro dei driver** e dispatch: per estensione, magic bytes, o `--format`
  esplicito.

### 5.2 I driver

Ognuno riusa il parser/serializer già scritto nel tool corrispondente
(motore geometrico DXF con OCS/blocchi/tassellazione; codec WKB GeoPackage;
inferenza tipi CSV; normalizzazione LibreDWG; georeferenziazione DXF
RDN2008/EPSG:7794; ecc.), **ritargettato da GeoJSON a `RecordBatch`**. Nessuna
logica di formato in `plenora-io-core`; nessuna dipendenza Arrow nei parser di
formato oltre quella mediata da `plenora-core`.

### 5.3 plenora-io-cli

- `inspect --input F [--layer L]` → contratto + descrittore (JSON versionato).
- `layers --input F` → elenco layer.
- `read --input F [--layer L] --output O.arrow` → file → Arrow IPC.
- `write --input I.arrow --output F [--format …]` → Arrow IPC → file.
- `convert --input a.shp --output b.gpkg` → `read` + `write` in streaming,
  file→file, `RecordBatch` in mezzo. **Sostituisce** il vecchio
  import/export-verso-GeoJSON: GeoJSON è ora solo uno dei formati.
- `catalog` → descrittori dei driver, machine-readable.
- Busta d'errore versionata `plenora-io-error-v1`: `category`, `phase` ed
  `remote_effect` sono campi `snake_case`; `retry` è un oggetto taggato e
  conserva `delay_ms` per `after`; `message` resta diagnostico. Exit code stabili
  (2 CliUsage, 3 LimitExceeded, 4 Unsupported, 5 LayerNotFound, 6 OutputExists,
  7 Input/Io, 8 InvalidInput, 10 Internal) — allineati alla famiglia.

**Fuori scope (invariato)**: qualunque *trasformazione* dei dati (filter, join,
aggregate, reproject) resta a `plenora-data-tools`. plenora-IO-tools non ha un
formato di piano: ha driver.

---

## 6. Multi-layer e CRS

### 6.1 Multi-layer (decisione D2)

`gpkg`, `gdb`, `dxf` (layer), `kml` (cartelle), `xlsx` (fogli) hanno **più
layer**; `csv`, `geojson`, `shp`, `geoparquet` ne hanno uno. Modello unico: un
dataset ha 1..N layer, ognuno mappa a un `DataContract` + uno stream. `open`
elenca i layer; `read(layer)` ne scorre uno. Poiché data-tools consuma **un**
`RecordBatch` per volta (una geometria attiva, D16), il layer è l'unità di
scambio: la pipeline elabora un layer alla volta. La generalizzazione dei
comandi `layers`/`import-all` dei tool attuali.

### 6.2 CRS: letto e scritto, mai trasformato (decisione D3)

Ogni formato porta il CRS in modo diverso; il driver lo estrae in `ResolvedCrs`
e lo attacca alla colonna geometrica (`geo.crs`); in scrittura lo serializza nel
modo del formato. **Nessuna riproiezione.**

| Formato | CRS |
|---|---|
| Shapefile | `.prj` (WKT) |
| GeoPackage | tabella `gpkg_spatial_ref_sys` |
| GeoParquet | `geo.crs` PROJJSON (assente ⇒ OGC:CRS84) |
| DXF | WKT ESRI incorporata (es. RDN2008/EPSG:7794) — già estratta dal lavoro dxf |
| KML / GeoJSON | WGS84 per specifica |
| CSV / XLSX | nessuno: dichiarato dal chiamante |

Se il CRS manca o è non risolvibile e il formato lo richiede: fallimento chiuso
`CRS_UNRESOLVED`. La riproiezione (`geo.reproject`) è un passo di
`plenora-data-tools`, mai del driver.

### 6.3 Pruning, non filtering (decisione D8)

Il pushdown del `ReadRequest` **non** deve trasformare plenora-IO-tools in un
query engine. Distinzione netta:

- **Projection pushdown**: leggere solo le colonne richieste. È chiaramente I/O,
  sempre lecito dove il formato è colonnare (Parquet, IPC).
- **Pruning**: escludere blocchi **sicuramente** incompatibili usando i
  **metadati** del formato — statistiche di row group, min/max, indice spaziale,
  partizioni, capability native equivalenti. Non legge dati che di certo non
  servono. Lecito.
- **Filtering**: valutare la condizione **riga per riga**. **NON** è compito di
  plenora-IO-tools: resta in `plenora-data-tools` (lo step `table.filter`/
  `geo.*`), che possiede la semantica generale del filtro.

Regola: **IO-tools fa pruning, non filtering.** Il `pruning_predicate` /
`spatial_pruning_hint` del `ReadRequest` è un *suggerimento di pruning* (il nome
lo dice): un driver lo onora **solo** se ha una capacità nativa chiaramente
equivalente e documentata (es. i min/max dei row group Parquet, un indice
spaziale del GeoPackage), altrimenti lo **ignora** e restituisce tutte le righe —
il filtro esatto lo applicherà data-tools. Un driver non deve mai *approssimare*
un filtro: o esclude un blocco con certezza dai metadati, o lascia passare.

### 6.4 Geometria: pass-through solo quando il payload è WKB compatibile

L'ottimizzazione "niente decode geometrico" (V4 di `Prestazioni.md`) va formulata
con precisione: la garanzia è **zero decode geometrico quando il payload è già
WKB compatibile**, *non* zero parsing né zero slicing del buffer.

- **GeoPackage**: la geometria non è WKB puro. Il blob è
  `GeoPackageBinaryHeader` (magic, flag, SRS ID, envelope opzionale) **+**
  payload WKB. Il driver deve **estrarre e validare** il payload; quello può
  essere riusato senza decode in `geo::Geometry`, ma l'header va comunque letto.
- **GeoParquet**: può avere encoding geometrici e metadati (`geo`) da
  normalizzare prima di considerare il WKB pass-through.
- Quindi: la colonna `geoarrow.wkb` in uscita può riusare i byte del payload WKB
  **quando compatibili** (stesso WKB 2D, CRS coincidente), ma passa comunque per
  estrazione e validazione strutturale. Il beneficio è saltare la
  decodifica/ricodifica in `geo::Geometry`, non saltare il parsing del
  contenitore.

---

## 7. Compatibilità e migrazione

- **Da 8 binari a 1 libreria + driver.** I tool attuali (`plenora-*-tools`)
  diventano driver dietro `FormatDriver`. Nella transizione possono restare come
  **wrapper sottili** (stessa CLI esterna) sopra i nuovi driver, così i flussi
  esistenti non si rompono; poi deprecati.
- **GeoJSON da hub a driver** (decisione D4): il perno passa da GeoJSON a
  `RecordBatch`. Il vecchio `import <fmt> → GeoJSON` diventa `read <fmt> →
  RecordBatch` (+ `write geojson` se serve davvero GeoJSON in uscita). È il
  cambiamento concettuale più grosso: va scritto nei test come invariante
  "nessuna doppia conversione via GeoJSON" (V3 di `Prestazioni.md`).
- **Contratto RecordBatch di proprietà di `plenora-core`** (non dei data-tools):
  qualunque evoluzione del set di tipi o della convenzione geometria si decide
  **una volta** in `plenora-core`; entrambe le librerie di pari livello la
  consumano.
- **Test ereditati**: le suite blackbox/CLI/fuzz dei tool esistenti migrano nel
  workspace; il round-trip cambia bersaglio (file ⇄ RecordBatch invece di file
  ⇄ GeoJSON) ma la copertura non cala. Gli **oracoli interop** (ogrinfo/ogr2ogr
  per shp/gpkg/geoparquet, pyarrow per geoparquet, GDAL DXF oracle) diventano
  criteri di accettazione per driver.

---

## 8. Fasi di lavoro

- **Fase 0 — Fondamenta**: workspace, dipendenza da `plenora-core` condivisa,
  `[workspace.dependencies]` con Arrow/Parquet `=59.1.0`, `trait FormatDriver`,
  `FormatDescriptor`, registro, publish atomico condiviso, CI/quality-gate, ADR.
  Tabella dei descrittori dei driver.
- **Fase 1 — Convergenza meccanica**: portare i parser/serializer esistenti come
  driver **ritargettati su `RecordBatch`** (non più GeoJSON); bump del driver
  GeoParquet a Parquet 59; errori e `Limits` unificati; catalogo dei driver.
  Nessun cambio di comportamento oltre il cambio di bersaglio. In parallelo:
  **baseline prestazionale archiviata in CI** per ogni driver (throughput
  read/write, picco RSS; dataset sintetici e reali) — riferimento del benchmark
  gate.
- **Fase 2A — Streaming reale**: streaming dove il formato lo consente (CSV,
  GeoParquet per row group, Arrow IPC, GeoJSON newline-delimited); driver
  materializzanti (DXF, KML, XLSX whole-sheet) dichiarati e bounded. Contratto
  statico/dinamico completo. CLI `inspect`/`layers`/`read`/`write`/`convert`.
- **Fase 2B — Multi-layer, multi-file, CRS, interop**: selezione layer; publish
  atomico multi-file (set Shapefile, staging dir); estrazione CRS completa per
  formato; catalogo fedeltà/capability completo; matrice avversaria; oracoli
  interop come gate.
- **Fase 2C — Ottimizzazioni fondamentali** (dietro benchmark): pass-through
  Arrow IPC e decodifica colonnare diretta su Parquet; **projection pushdown +
  row-group pruning** su Parquet (leggere solo le colonne richieste, saltare i
  row group esclusi dai metadati — mai filtering riga-per-riga); decode WKB
  minimizzato; batch sizing adattivo (byte + righe).
- **Fase 3 — Formati aggiuntivi e avanzate**: GML (Catasto/INSPIRE),
  FlatGeobuf; encoding GeoArrow nativo opzionale; IPC canonico; **tier raster**
  come mondo separato. Solo dietro benchmark e necessità.

---

## 9. ADR

**Scritti** in `docs/adr/` (Fase 0, completata). Alcuni sono **rinvii** agli ADR
dei data-tools per non duplicare le regole del bordo condiviso:

- **ADR-IO 1 — `trait FormatDriver`, ciclo di vita e `WritePlan`**:
  `open`/`open_layer_reader`/`create`/`write`/`finish`; ownership degli handle;
  `LayerReader` con stato mutabile e stream indipendenti; streaming vs
  materializing; cancellazione a metà lettura/scrittura senza output parziale.
  **Semantica del `WritePlan`**: ordine canonico dei layer, unicità dei nomi e
  comportamento su nomi duplicati, compatibilità fra CRS multipli nello stesso
  contenitore, e se ogni layer ha un writer separato o se un dataset-writer li
  coordina verso il commit atomico unico (D11).
- **ADR-IO 2 — Publish atomico multi-file**: staging dir + rename per il set
  Shapefile e per i formati a più file; requisito same-filesystem; profili
  `AtomicPublish`/`DurableAtomicPublish`; Windows (share lock, antivirus) vs
  Linux. (Coerente con ADR 7 dei data-tools.)
- **ADR-IO 3 — Capability-check di scrittura per formato**: come ogni driver
  dichiara e verifica la rappresentabilità del contratto (nomi campo, tipi,
  tipo geometrico unico, CRS fisso), con errori tipizzati.
- **ADR-IO 4 — Estrazione e serializzazione CRS per formato**: mappatura
  formato→`ResolvedCrs`, casi di CRS assente/non risolvibile, niente
  riproiezione.
- **ADR-IO 5 — Fedeltà e report di perdita**: definizione di `Lossless` vs
  `Approximating`, cosa un driver deve riportare (skipped/dropped), come si
  verifica in test e con gli oracoli.
- **ADR-IO 6 — `ReadRequest`, pruning vs filtering**: semantica di
  `projected_fields`/`pruning_predicate`/`spatial_pruning_hint`; quando un driver
  può onorare un suggerimento di pruning (capacità native: min/max row group,
  indice spaziale, partizioni) e quando deve ignorarlo lasciando il filtering a
  data-tools; divieto di approssimare un filtro; interazione con il batch sizing.
- **Rinvii**: contratto RecordBatch e set di tipi → §4.1 + `plenora-core` dei
  data-tools; validazione WKB → §2; `PropertyConfidence`/`Scope` → D25;
  determinismo geometrico → ADR 1 dei data-tools.

---

## 10. Invarianti

Criteri di accettazione verificabili in test:

1. Nessuna riga viene letta prima della validazione dell'header (fase statica).
2. Nessun `RecordBatch` prodotto viola il contratto di `plenora-core` (§2.2)
   (verificato ai bordi nei test).
3. Nessun output parziale: a errore, cancellazione o limite superato, la
   destinazione non è mai toccata (publish solo a successo).
4. Un formato multi-file appare in modo atomico (tutti i componenti o nessuno).
5. Nessun driver riproietta: il CRS letto è il CRS scritto, salvo passaggio
   esplicito per `plenora-data-tools`.
6. Un driver `Approximating` riporta sempre ciò che scarta; nessuna perdita
   silenziosa.
7. Il CRS assente/non risolvibile, quando il formato lo richiede, fallisce
   chiuso, mai un default implicito sbagliato.
8. Le geometrie prodotte/consumate passano la validazione WKB condivisa (celle,
   componenti, profondità).

A queste si aggiungono le **invarianti prestazionali** di `Prestazioni.md` §6
(streaming a memoria quasi costante sui formati streamabili, zero-copy verso
Arrow da sorgenti colonnari, nessuna doppia conversione via GeoJSON, limiti
prima delle allocazioni, ≤1 decode/encode WKB per geometria), con lo stesso
status di criteri di accettazione.

---

## 11. Rischi e mitigazioni

| Rischio | Mitigazione |
|---|---|
| Skew di versione Arrow tra IO-tools e data-tools | Pin condiviso `=59.1.0` + `plenora-core` condiviso (D0) |
| Driver GeoParquet su Parquet 54 incompatibile con Arrow 59 | Bump a `parquet =59.1.0` in Fase 1 (lockstep arrow-rs) |
| Regressione "doppia conversione" via GeoJSON | Invariante V3: formato→RecordBatch diretto, GeoJSON è un driver (D4) |
| Set Shapefile pubblicato in modo non atomico | Staging dir + rename (ADR-IO 2) |
| Driver lossy (DXF) che perde in silenzio | Fedeltà dichiarata + skipped report (ADR-IO 5) |
| Riproiezione implicita che sporca il CRS | IO legge/scrive il CRS; reproject è step data-tools (D3, invariante 5) |
| Scrittura verso formato che non regge il contratto | Capability-check statico in `create`, fail-closed (ADR-IO 3) |
| GDAL che si allarga oltre FileGDB | Tier GDB a feature, un solo driver, deploy container (policy) |
| Confusione streaming vs materializing | Matrice esplicita nel descrittore + criteri di memoria (Prestazioni) |
| DoS da parser ostile (CSV a righe enormi, XML bomb, Parquet meta gonfi, WKB) | Limiti prima delle allocazioni + guardie WKB già esistenti |
| Deriva tra gli 8 tool legacy e i driver unificati | Wrapper sottili in transizione, poi deprecazione; core condiviso |
| Contratto del bordo che diverge tra le due librerie | Definito una sola volta in `plenora-core`, mai duplicato (D0, §2.2) |

---

## 12. Decisioni registrate

I numeri non vengono riassegnati. Dove una decisione è **ereditata** dai
data-tools è indicato; qui si registra solo ciò che è specifico dell'I/O.

### 12.1 Fondamentali (modello e contratto — stabili)

- **D0 — `plenora-core` condiviso e Arrow/Parquet `=59.1.0`.** Il bordo
  (contratto RecordBatch, geometria `geoarrow.wkb`, CRS, WKB, Limits) è definito
  una sola volta e condiviso con `plenora-data-tools`; stesso pin Arrow perché i
  `RecordBatch` passino senza conversione. Driver GeoParquet portato a Parquet 59.
- **D1 — Driver dietro `FormatDriver`, due fasi `open`/`read` e `create`/`write`**,
  con validazione statica/dinamica distinta (eredita D2/D8 dei data-tools).
- **D2 — Multi-layer come unità di scambio**: 1..N layer per dataset, un
  `RecordBatch`/contratto per layer; una geometria attiva per la v1 (eredita D16).
- **D3 — Il driver legge e scrive il CRS, non lo trasforma mai.** La riproiezione
  è uno step di `plenora-data-tools`; CRS assente/non risolvibile ⇒ fallimento
  chiuso.
- **D4 — Perno `RecordBatch`, GeoJSON è un driver.** plenora-IO-tools È
  `plenora-datafile`; GeoJSON perde lo status di hub. Nessuna doppia conversione.
- **D5 — Fedeltà dichiarata per driver** (`Lossless`/`Approximating`), con report
  di perdita obbligatorio per i driver approssimanti (dal lavoro DXF).
- **D6 — Puro Rust di default, tier GDB a feature** per FileGDB (policy di
  famiglia): un solo driver GDAL, deploy a container, dichiarato.

### 12.2 Dell'implementazione (sostituibili senza cambiare i contratti)

- **D7 — Publish atomico**: tempfile+persist per file singolo, staging dir per
  multi-file e per contenitori multi-layer; profili
  `AtomicPublish`/`DurableAtomicPublish` (ADR-IO 2).
- **D8 — `ReadRequest` con projection e pruning, non filtering.** La lettura
  porta projection pushdown (I/O puro) e *suggerimenti di pruning*
  (`pruning_predicate` / `spatial_pruning_hint` — i nomi evitano l'equivoco
  "filter") onorati solo se il formato ha capacità native equivalenti e
  documentate (statistiche row group, indice spaziale). Il **filtering
  riga-per-riga resta a `plenora-data-tools`**: IO-tools esclude blocchi con
  certezza dai metadati o lascia passare, mai approssima un filtro (§6.3,
  ADR-IO 6).
- **D9 — `streaming` per-driver e per-versione, non permanente.** DXF/KML/XLSX
  sono `Materializing` **nella v1**, bounded dai limiti; il descrittore lascia
  aperta una via streaming/semi-streaming futura (block-table pass + emissione
  progressiva per DXF, parser XML pull per KML, lettura per righe per XLSX)
  senza cambiare contratto né `trait` (§2.3).
- **D10 — Dataset e writer come trait object per driver, lettura con stato in
  un `LayerReader` per layer.** `OpenDatasetHandle` (immutabile, condivisibile)
  espone `open_layer_reader(&ReadRequest) -> Box<dyn LayerReader>`; lo stato
  mutabile della lettura (cursore, parser, connessione SQLite, handle GDAL,
  reader non clonabile) vive nel `LayerReader` (`&mut self`), non nell'handle —
  così si aprono **stream indipendenti** quando il formato lo consente. Gli
  stati sono troppo eterogenei per un tipo unico: facade dinamica, stato del
  singolo driver monomorfizzato internamente (§5.1).
- **D11 — Scrittura v1: dataset nuovo, tutti i layer insieme, commit atomico.**
  `create`+`WritePlan`+`finish` pubblicano l'intero dataset o niente; **niente
  append né modifica in place** nella v1 (§5.1).
- **D12 — Tool legacy come wrapper sottili in transizione**, poi deprecati; i
  test migrano ritargettati su `RecordBatch`.

Ottimizzazioni zero-copy/pushdown solo dietro benchmark (Fase 2C).

### 12.3 Ereditate (non ridecise qui)

Contratto di input e set di tipi (§4.1), canone geometrie `geoarrow.wkb` (D0/D1
data-tools), validazione statica/dinamica (D8), `PropertyConfidence`/`Scope`
(D25), profili di publish (D22/ADR 7), determinismo geometrico (ADR 1),
semantica dei limiti (ADR 6). plenora-IO-tools **conforma**, non ridefinisce.
