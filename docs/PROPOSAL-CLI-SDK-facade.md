# Proposta architetturale — CLI completo, SDK Python, facade Rust

Stato: **proposta**. Documento non normativo. Nessuna modifica al
codice, ai manifest `release/*.json`, o all'evidence base richiesta o
prevista prima della ratifica. **Nota di scope**: questo documento e'
di per se' un file nuovo non tracciato nel working tree
(`docs/PROPOSAL-CLI-SDK-facade.md`); "nessuna modifica al repository"
va inteso come "nessuna modifica a codice compilato o a
manifesti di release".

Reference implementation: `C:\Users\marco\Desktop\database-tools`
(workspace `plenora-database-*`, versione workspace 1.2.0, Python SDK
0.9.0 con versionamento decoupled).

Target: `C:\Users\marco\Desktop\IO-tools` (workspace
`plenora-io-*`, versione workspace 1.0.1, working tree post-hardening
1.1.0-candidate).

## Sommario esecutivo

Tre lavori interdipendenti, con **prerequisiti architetturali** che
vanno chiusi prima di poter congelare la facade e i comandi
`options`/`validate`:

**Prerequisiti (nessuno chiuso oggi)**:
- **PR-1 / ADR-IO 7** — ratifica della semantica streaming vs
  operation-atomicity del `BudgetedReader`. Oggi il reader
  materializza l'operazione (scan completo prima del primo batch).
  Uno SDK Python che espone iterazione batch senza chiudere questa
  ADR consegna un contratto ingannevole.
- **PR-2** — unificazione di `Limits` e `ResourceLimits`. La facade
  espone oggi due modelli quasi-duplicati (per-cell in `Limits`,
  cumulativi in `ResourceLimits`); congelarli entrambi nella
  superficie pubblica cristallizza un debito che ROADMAP-1.1.0 L3
  chiede di risolvere.
- **PR-3** — schema dichiarativo delle `format_options`. Il comando
  `options <format>` proposto in D.3 e il metodo `plenora_io.options()`
  proposto in E richiedono un registry versionato di chiavi/tipo/
  default per driver. Senza, l'output di `options` sarebbe un
  documento hard-coded per formato, con lo stesso rischio di
  dispersione che PR-3 chiede di chiudere.

**Piano incrementale (post-prerequisiti)**:

1. **Facade Rust `plenora-io-api`** — punto unico di accesso stabile
   con `#[non_exhaustive]` sulle enum pubbliche e
   costruttori/builder stabili per ogni tipo. CLI e binding Python
   sono i due unici consumer.
2. **CLI `plenora-io-cli` con estensioni additive** — aggiunge
   `formats`, `options`, `schema`, `validate`; aggiunge `--format
   json|markdown|junit`; conserva tutti i 6 envelope congelati in
   `cli-protocol-v1.json` invariati. Le estensioni sono **additive**
   e NON richiedono un bump `cli-protocol-v2`: introducono nuovi
   contract con nomi propri, che il manifest esistente puo' registrare
   accanto alla lista v1.
3. **SDK Python `plenora-io-py`** — cdylib PyO3 con abi3, matrice
   wheel documentata in G.4; API costruita attorno a
   `plenora_io.open(path) -> Dataset` context manager; boundary Arrow
   via **IPC bytes** per l'MVP (allineato al reference), valutazione
   C Data Interface per la v2 solo se un benchmark gate documenta un
   veto prestazionale.

Decisione architetturale forte: si sceglie il pattern **facade
dedicata** anziche' l'approccio del reference (core diretto +
contratti JSON congelati). IO-tools parte da zero per il binding
Python: strutturarla bene ora costa meno che rifattorizzarla dopo —
ma la facade non puo' essere congelata prima di PR-1/PR-2/PR-3.

**Limite dichiarato dell'MVP**: senza il comando `scan` (rimandato a
v2, vedi D.9 e "Decisioni ancora aperte"), l'MVP **non e' un CLI
completo per l'estrazione dei dati**. L'MVP copre catalog, ispezione,
schema, validazione e conversione file→file; l'estrazione streaming
di batch Arrow su stdout resta un capitolo separato.

## A. Parity matrix CLI

### A.1 Reference `plenora-database-cli` — cosa fa

- Binary: `plenora-database`, feature-gated (`postgres` default,
  `mysql`, `sqlserver`, `full`).
- Oltre 40 sottocomandi organizzati per categoria: bootstrap
  (`database-probe`, `doctor`, `pool-status`), conformance
  (`profile-check`), inspection (`inspect-database`,
  `inspect-schemas`, `inspect-tables`), read Arrow IPC
  (`postgres-read-summary`, `postgres-read-ipc`), write bulk+DML
  (`bulk-write`, `postgres-write-ipc`, `execute-ddl`), portable AST
  (`portable-compile`, `portable-execute`), testing e benchmark.
- Global flags: `--format json|markdown|junit`,
  `--allow-write-tests`, `--ephemeral-schema`, `--session-context`.
- Exit code: 0 successo, 1 errore. Variante `CliError::Silent` per
  operazioni che hanno gia' loggato l'esito.
- Envelope errori v1: `{status, protocol_version, error: {category,
  phase, remote_effect, retry, provider, execution_id, message,
  diagnostics}}`.
- Contratti congelati in `contracts/v1/`: `plan.schema.json`,
  `capabilities.schema.json`, `write-outcome.schema.json`,
  `loss-report.schema.json`, `common.schema.json`.
- Nessun NDJSON in output; Arrow IPC solo in input a `bulk-write`.

### A.2 Target `plenora-io-cli` con estensioni additive

Comandi attuali (v1, congelati): `catalog`, `inspect`, `layers`,
`read`, `convert`. Restano invariati per envelope e semantica.

**Nota di versionamento**: aggiungere comandi non richiede un bump
del CLI protocol a v2. Il contratto `cli-protocol-v1` congela **gli
envelope elencati oggi**; nuovi comandi introducono envelope
**nuovi** (`plenora-io-formats-v1`, `plenora-io-options-v1`, ecc.)
che vengono registrati nel manifest come contract additivi. Un bump
`cli-protocol-v2` sarebbe giustificato solo da un cambio incompatibile
degli envelope v1 esistenti (rimozione di un campo obbligatorio,
cambio di semantica di un valore), che qui non accade.

**Comandi nuovi additivi**:

| Comando | Semantica | Envelope | Prerequisito |
|---|---|---|---|
| `formats` | Elenco formati riconosciuti con capability sintetiche (bidirezionale/solo-lettura/solo-scrittura). Alias piu' amichevole di `catalog`. | Nuovo `plenora-io-formats-v1` | Nessuno |
| `options <format>` | Elenco `format_options` accettate dal driver, con tipo, default, semantica. Copre il gap "quali chiavi posso passare a `--in-opt`/`--out-opt`". | Nuovo `plenora-io-options-v1` | **PR-3** (schema dichiarativo `format_options`) |
| `schema <path>` | Solo lo schema Arrow del layer selezionato, senza sample di righe. Utile per pipeline di ingest che valutano compatibilita' prima di leggere. | Nuovo `plenora-io-schema-v1` | Nessuno |
| `validate <path>` | Apre e drena la sorgente senza scrivere; ritorna il `LossReport` osservato, i limiti applicati e l'esito `passed/rejected`. Pensato come pre-check per `convert`. | Nuovo `plenora-io-validate-v1` | **PR-2** (unificazione limiti) per esporre `limits` in modo canonico |

Nessuno di questi tocca gli envelope esistenti. La regola: **nuovi
comandi = nuovi contract JSON versionati**, mai estensioni in place.

**Global flags nuovi** (allineati al reference dove sensato):

| Flag | Semantica | Default |
|---|---|---|
| `--format json\|markdown\|junit` | Formato del documento su stdout. `json` resta il default e la wire form autoritativa. `markdown`/`junit` sono view derivate, non canoniche. | `json` |
| `--progress` | Attiva il progress reporting su stderr (vedi A.3). | **Off** |
| `--timeout-ms N` | Alias per il campo `duration_ms` del `ResourceBudget`. | Ereditato da `Limits` |

**Progress reporting**: il reference non ne ha. Proposto per IO-tools
come **opt-in stretto**: righe JSON su **stderr** (non stdout, per
preservare il singolo documento canonico) con contract
`plenora-io-progress-v1`:

```json
{"contract":"plenora-io-progress-v1","phase":"read","rows":10000,"bytes":524288}
```

Emesso **solo** se il chiamante passa `--progress`. Nessun rilevamento
implicito del TTY, nessun default "attivo quando interattivo": chi lo
vuole lo chiede. Il flag e' additivo, non entra nell'envelope
autoritativo su stdout.

**Cancellazione**: SIGINT su Unix, `Ctrl+C` handler su Windows,
propagati al `CancellationToken` gia' presente nel core. La CLI
installa il proprio handler perche' possiede il processo (a
differenza della libreria Python, vedi B.2). Exit code riservato 130
(gia' fatto per la CLI attuale). Nessun cambio.

**Sicurezza / limiti**: nessuna modifica ai flag esistenti
(`--max-rows`, `--max-columns`, `--max-vertices`,
`--max-wkb-cell-bytes`, `--max-wkb-components`, `--max-wkb-depth`,
`--max-input-bytes`, `--max-output-bytes`, `--durable`). Un flag
`--allow-write-tests` NON serve: IO-tools non ha comandi destructive
paragonabili a DDL SQL.

**Nota su `read`**: il comando esistente `read` **non emette batch
materializzati su stdout**. Emette un envelope JSON con conteggi
(righe scandite, byte, `fidelity`, `LossReport`) e con un eventuale
sample-limit di tuple. L'estrazione batch-a-batch di dati Arrow non
e' coperta dai comandi v1; per averla serve `scan` (vedi D.9), che
resta **fuori MVP**.

### A.3 Riutilizzabile da `plenora-database-cli`

- Framework di formattazione `json|markdown|junit` (tre trait per
  `Renderable`).
- Snapshot testing su envelope (`tests/error_protocol.rs`,
  `tests/contract_snapshot.rs` come pattern).
- Gate CI: `clippy all|pedantic|nursery = deny`, `unsafe_code =
  forbid`, `overflow-checks = true`, `cargo-deny` per advisory e
  licenze.
- Redazione dei secret nei messaggi d'errore (IO-tools non ha secret
  DB, ma ha path assoluti e payload — la stessa policy si applica).

### A.4 Da NON copiare (database-specific)

- Feature flag per provider (`postgres`/`mysql`/`sqlserver`). IO-tools
  non ha provider intercambiabili; i driver sono attivati staticamente
  dal registry.
- Comandi `probe`, `doctor`, `pool-status`, `test-cancellation`,
  `test-streaming`, `benchmark-*`. Alcuni sono utili come test
  interni, non come CLI pubblica.
- `--session-context`, `--ephemeral-schema`, `--allow-write-tests`.
- Campi errore `provider`, `execution_id`. IO-tools ha `driver` (gia'
  nell'error model) — ruolo analogo ma non intercambiabile: non
  rimappare, mantenere `driver`.
- Portable AST (`portable-compile`, `portable-execute`). IO-tools non
  ha un linguaggio di query proprio: le opzioni sono flag CLI.

## B. Parity matrix SDK Python

### B.1 Reference `plenora-database-py` — cosa fa

- Crate: `plenora-database-py`, cdylib `plenora_database_native`
  esposto come modulo Python `plenora_database._native`.
- PyO3 0.23, feature `["extension-module", "abi3-py310", "serde"]`.
  Un wheel `.abi3.so` per piattaforma copre Python 3.10-3.13.
- Struttura repo:
  ```
  python/plenora_database/
    __init__.py           # entry-point (connect, aconnect, connect_mysql)
    __init__.pyi
    _native.pyi           # PyO3 interface stubs
    py.typed              # PEP 561
    _session.py           # wrapper con context manager
    _transaction.py
    _async_session.py
    _async_transaction.py
    query.py              # AST builder Select/Insert/Update/Delete
    _arrow_io.py          # Arrow IPC bytes ↔ pyarrow
    errors.py             # re-export della gerarchia da _native
    spatial.py
    types.py
    _native.abi3.so       # build output
  ```
- Entry-point: `plenora_database.connect(dsn, tls_mode="require") ->
  Session`.
- Classi: Session, Transaction, AsyncSession, AsyncTransaction,
  BatchReader, SelectQuery builder, SpatialReference,
  SessionContext.
- Gerarchia errori: base `PlenoraError(RuntimeError)`, 17 sottoclassi
  con attributi `category`, `phase`, `retry`, `remote_effect`,
  `provider`, `execution_id`, `diagnostics`.
- Boundary Arrow: **IPC bytes lineari** (`arrow_ipc::StreamWriter` in
  Rust; `pyarrow.ipc.open_stream(io.BytesIO(chunk))` in Python). No C
  Data Interface, no PyCapsule.
- Lifecycle: `with connect() as s`; `close()` idempotente; runtime
  tokio globale `OnceLock`, mai droppato.
- Wheel CI: Linux manylinux_2_34 x86_64, macOS arm64, Windows
  x86_64; smoke test import + `version()` su ogni wheel.

### B.2 Target `plenora-io-py` — struttura proposta

**Nome crate**: `plenora-io-py`, cdylib `plenora_io_native`, modulo
Python `plenora_io._native`.

**Configurazione**: identica al reference:
- PyO3 = "0.23", features `["extension-module", "abi3-py310", "serde"]`.
- Maturin >= 1.7 < 2.0.
- Python >= 3.10.
- Rust 1.92 (workspace).

**Struttura repo** (adattata):
```
python/plenora_io/
  __init__.py           # entry-point: open()
  __init__.pyi
  _native.pyi
  py.typed
  _dataset.py           # Dataset wrapper con context manager
  _layer.py             # Layer wrapper (iter di RecordBatch)
  _convert.py           # convert() helper (equivalente CLI)
  _arrow_io.py          # IPC bytes ↔ pyarrow
  errors.py             # re-export gerarchia PyO3
  types.py              # Bbox, ReadOptions, WriteOptions dataclass
```

**Entry-point pubblica** (dettaglio in E):
- `plenora_io.open(path, **opts) -> Dataset` (sync).
- `plenora_io.convert(source, destination, **opts) -> Published`.
- `plenora_io.catalog() -> list[FormatDescriptor]`.
- `plenora_io.options(format_id) -> list[FormatOption]`.

**Classi**:
- `Dataset`: context manager, `.layers()`, `.inspect()`,
  `.read(layer, **opts) -> Iterator[pyarrow.RecordBatch]`.
  **Semantica reale finche' PR-1/ADR-IO 7 non e' ratificata**:
  `read()` restituisce un iteratore Python i cui batch sono gia'
  stati materializzati sotto — la prima `next()` blocca finche' il
  reader interno non ha drenato l'intera sorgente. Latenza al
  primo batch = tempo di scan completo; memoria bounded dal budget
  ma non dal batch. Questo va documentato nell'API prima del
  primo tag. Solo dopo la chiusura di PR-1/L2 (spool bounded
  oppure rilascio dell'operation-atomicity) il metodo puo'
  dichiarare streaming reale.
- `Layer`: rappresentazione read-only del contratto layer (nome,
  schema Arrow, CRS, geometry contract, capability).
- `Published`: risultato di `convert()` (bytes scritti, `LossReport`,
  `FidelityAssessment`, `PublishOutcome`).
- `Bbox`: NamedTuple `(minx, miny, maxx, maxy)`.

**Gerarchia errori** (parallela al reference, ma senza le classi
database-specific):

```
PlenoraError(RuntimeError)
├── PlenoraContractError          # schema/contract violati
├── PlenoraCapabilityError        # capability check fallito
├── PlenoraCrsError               # CRS non risolto o incoerente
├── PlenoraWkbError               # WKB malformato o oltre i limiti
├── PlenoraLimitExceededError     # rows/bytes/vertices/duration
├── PlenoraCancelledError         # cancellazione cooperativa
├── PlenoraNotFoundError          # layer/path non esiste
├── PlenoraOutputExistsError      # collision su publish
├── PlenoraUnsupportedError       # feature non compilata / driver
├── PlenoraIoError                # errore filesystem
├── PlenoraJsonError              # JSON malformato in GeoJSON
└── PlenoraInternalError          # invariante di libreria violato
```

Ogni istanza porta gli stessi attributi del reference:
`category`, `phase`, `remote_effect`, `retry`, `driver`
(non `provider`), `message`, `row_diagnostics` (equivalente di
`diagnostics`). Escluso `execution_id` (non applicabile a operazioni
locali sul filesystem).

**Lifecycle**:
- `Dataset.__enter__` / `__exit__` con chiusura automatica.
- `Dataset.close()` idempotente.
- **Cancellazione esplicita via `CancellationToken` passato dal
  chiamante**. Il binding NON installa signal handler globali: un
  handler installato dalla libreria interferirebbe con l'applicazione
  ospite (server WSGI, notebook Jupyter, altri worker). Il chiamante
  che vuole reagire a `SIGINT` costruisce il proprio handler e chiama
  `token.cancel()`; la libreria si limita a offrire un
  `CancellationToken` che l'application code puo' iscrivere ai propri
  segnali. Un esempio idiomatico e' documentato in E.
- Tokio runtime: **non necessario** per IO-tools. L'operazione e'
  sincrona e non ha primitive async. Il binding puo' essere
  interamente sync, senza `AsyncDataset` — semplifica e allinea al
  fatto che il core Rust non e' `async`.

**Wheel CI**: vedi la matrice esplicita in G.4.

**Test Python**: `python/tests/` con fixture di filesystem (fixture
GeoParquet/GeoJSON pre-generate durante il test setup, non env var).

### B.3 Riutilizzabile da `plenora-database-py`

- Configurazione Maturin e feature `abi3-py310` (copia-incolla).
- Struttura repo (`python/<pkg>/` + stubs + `py.typed`).
- Pattern di conversione error PyO3 → gerarchia Python
  (`PyErr::new::<PlenoraError, _>`).
- Boundary Arrow IPC (`arrow_reader.rs` come reference esatto per
  `_arrow_io.py`).
- GitHub Actions wheel matrix (`.github/workflows/python-wheel.yml`).
- Smoke test post-build (import + version).

### B.4 Da NON copiare (database-specific)

- Session / Transaction / SessionContext / AsyncSession /
  AsyncTransaction. IO-tools e' file-based, non ha una sessione
  persistente.
- `begin(isolation=..., read_only=..., deferrable=...)`. Nessuna
  transazione.
- Savepoint (`savepoint`, `rollback_to_savepoint`,
  `release_savepoint`) — Postgres-only.
- AST builder Select/Insert/Update/Delete. Le opzioni di lettura
  sono field projection, bbox pruning, limiti — non un linguaggio.
- `SessionContext` (variabili transaction-local).
- Runtime tokio globale. IO-tools e' sync end-to-end.
- Connection pool concept. Ogni `open()` apre un handle read-only al
  filesystem.
- `PostGIS`, `SpatialReference`. IO-tools ha CRS come stringa
  autoritativa; la geometria e' pass-through WKB, non computa
  predicati spaziali server-side.

## C. Facade Rust `plenora-io-api`

### C.1 Scelta di design

Il reference NON ha una facade dedicata: sia CLI sia Python binding
importano `plenora-database-core` direttamente, e la stabilita' e'
comunicata da tre canali paralleli: (a) release manifest esterno, (b)
ADR, (c) JSON schema in `contracts/v1/`.

**Proposta divergente**: IO-tools introduce una crate
`plenora-io-api` con `#[non_exhaustive]` obbligatorio sulle enum e
sugli struct pubblici. Motivazione:

- IO-tools parte da zero per il binding Python: non c'e' un pattern
  gia' consolidato con cui compatibilita' storica va rispettata.
- Un layer dedicato disaccoppia i due consumer (CLI e Python) dal
  core, che puo' evolvere internamente senza rompere la superficie
  esposta.
- `#[non_exhaustive]` sposta la garanzia di stabilita' dal
  documento (release manifest) al **tipo Rust**, che il compilatore
  applica.
- La duplicazione di superficie (facade + core) e' un costo, ma per
  un componente da esporre a Python vale la pena.

**Regola sui tipi `#[non_exhaustive]`**: ogni tipo pubblico marcato
`#[non_exhaustive]` deve avere una via stabile per la costruzione
dall'esterno. In pratica:

- **Struct**: costruttore `Type::new(...)` con gli argomenti minimi
  e/o builder `Type::builder() -> TypeBuilder` con method chaining
  e `build() -> Type`. Nessuno struct literal `Type { .. }` e'
  possibile per il chiamante, ma i test interni della facade
  restano liberi di usarlo.
- **Enum**: costruttori associati `EnumType::variant(payload)` per
  ogni variante che ha payload; gli chiamanti fanno `match` con un
  `_ => ...` esplicito (regola imposta dal marker).
- **Non e' accettabile** un `#[non_exhaustive] struct Foo { pub a:
  X, pub b: Y }` senza `new`/`builder`: il consumer non riuscirebbe
  a costruirlo e la superficie sarebbe read-only per accidente.

**Regola sui trait pubblici**: la facade **non espone trait
implementabili all'esterno**. Un `pub trait LayerReader` senza
sigillatura permetterebbe a consumer di terze parti di iniettare
implementazioni proprie nel flusso di lettura, con conseguenze
imprevedibili su validazione, budget e diagnostica. Due opzioni
alternative:

1. **Reader concreto opaco**: `pub struct DatasetReader { .. }` con
   metodi inherent (`next_batch`, `loss_report`, `contract`) e stato
   interno privato. Nessun trait pubblico. Preferita per l'MVP.
2. **Trait sealed**: `pub trait LayerReader: sealed::Sealed { .. }`
   dove `mod sealed { pub trait Sealed {} }` non e' esportato — il
   trait resta implementabile solo internamente. Utile se piu'
   reader concreti servissero l'API pubblica.

**Regola di enforcement dell'API boundary**: `plenora-io-cli` e
`plenora-io-py` NON possono dipendere direttamente da
`plenora-io-core`, `plenora-io-model` o dai driver. La dipendenza va
esclusivamente da `plenora-io-api`. Un gate CI dedicato
(`scripts/check_api_boundary.py`) verifica che i due `Cargo.toml`
dei consumer non citino altre crate del workspace.

### C.2 Superficie pubblica proposta

Modulo top-level `plenora_io_api`:

```rust
pub mod catalog { ... }
pub mod dataset { ... }
pub mod convert { ... }
pub mod options { ... }
pub mod error { ... }
pub mod fidelity { ... }
```

Legenda: ogni tipo qui e' `#[non_exhaustive]`. I costruttori/builder
mostrati sono la sola via pubblica di costruzione. Le rappresentazioni
`pub struct T { pub a, pub b }` che seguono descrivono i **campi
osservabili**, non consentono struct literals dall'esterno.

**`catalog`** — enumerazione formati:

```rust
#[non_exhaustive]
pub struct FormatDescriptor { /* campi osservabili */ }
impl FormatDescriptor {
    pub fn id(&self) -> &str;
    pub fn direction(&self) -> Direction;
    pub fn read_mode(&self) -> ReadMode;
    pub fn write_mode(&self) -> Option<WriteMode>;
    pub fn multi_layer(&self) -> bool;
    pub fn reader_concurrency(&self) -> ReaderConcurrency;
    pub fn required_feature(&self) -> Option<&str>;
    pub fn available(&self) -> bool;
}

pub fn formats() -> Vec<FormatDescriptor>;
pub fn options_for(format_id: &str) -> Result<Vec<FormatOption>, Error>;
```

`FormatOption` proviene dal registry PR-3 (schema dichiarativo delle
`format_options`, prerequisito): finche' PR-3 non chiude, `options_for`
non ha una fonte di verita' unica da esporre.

**`dataset`** — apertura e lettura:

```rust
pub fn open(path: impl AsRef<Path>, opts: &ReadOptions) -> Result<Dataset, Error>;

pub struct Dataset { /* opaque */ }
impl Dataset {
    pub fn layers(&self) -> &[Layer];
    pub fn inspect(&self) -> Inspection;
    /// Reader concreto opaco (nessun trait pubblico implementabile).
    /// Semantica corrente: operation-atomic; il primo `next_batch()`
    /// blocca finche' l'intera sorgente non e' stata drenata dal
    /// `BudgetedReader`. Streaming reale post-ADR-IO 7.
    pub fn read(&self, layer: LayerId, req: &ReadRequest)
        -> Result<DatasetReader, Error>;
    pub fn fidelity(&self) -> &FidelityAssessment;
}

pub struct DatasetReader { /* opaque, stato interno */ }
impl DatasetReader {
    pub fn contract(&self) -> &Layer;
    pub fn next_batch(&mut self) -> Result<Option<arrow_array::RecordBatch>, Error>;
    pub fn loss_report(&self) -> LossReport;
}

#[non_exhaustive]
pub struct Layer { /* campi osservabili */ }
impl Layer {
    pub fn id(&self) -> LayerId;
    pub fn name(&self) -> &str;
    pub fn schema(&self) -> &arrow_schema::SchemaRef;
    pub fn geometry(&self) -> Option<&GeometryColumnContract>;
}
```

**Risoluzione `projected_fields`** (nome ↔ `FieldId`, cross-cutting
per API Rust e Python):

- La forma nativa e' `Vec<FieldId>` (indice fisico nello schema del
  layer).
- Il binding Python accetta anche `list[str]` (nomi di campo).
  L'entry-point la traduce in `Vec<FieldId>` con queste regole
  esplicite:
  - **Nome inesistente**: se `projection_mode = Required` →
    `PlenoraContractError` con messaggio che elenca i nomi non
    trovati. Se `projection_mode = BestEffort` → nome scartato con
    voce dedicata nel `LossReport` (`projection.field_missing`).
  - **Nome duplicato nella lista** (`["id", "id"]`): il duplicato
    e' silenziosamente dedotto una volta sola. Non e' un errore
    perche' non altera l'output.
  - **Nome ambiguo nello schema** (piu' campi con lo stesso nome —
    condizione che i driver del bordo IO gia' rifiutano
    all'apertura del layer): non puo' arrivare qui. Se dovesse,
    fail-closed con `PlenoraContractError`.
  - **Lista vuota `[]`**: significato "nessun attributo, geometria
    inclusa se presente" (compatibile con l'attuale
    `driver-gpkg::project_gpkg_layer`).
  - **`None`** (nessuna projection dichiarata): tutti i campi.

**`convert`**:

```rust
pub fn convert(
    source: impl AsRef<Path>,
    destination: impl AsRef<Path>,
    opts: &ConvertOptions,
) -> Result<Published, Error>;

#[non_exhaustive]
pub struct Published { /* campi osservabili */ }
impl Published {
    pub fn bytes(&self) -> u64;
    pub fn outcome(&self) -> PublishOutcome;
    pub fn read_loss(&self) -> &LossReport;
    pub fn write_loss(&self) -> &LossReport;
    pub fn conversion_fidelity(&self) -> &FidelityAssessment;
}
```

**`options`** — `ReadOptions`, `WriteOptions`, `ConvertOptions`,
`ResourceLimits`:

```rust
#[non_exhaustive]
pub struct ReadOptions { /* opaque */ }
impl ReadOptions {
    pub fn new() -> Self;
    pub fn builder() -> ReadOptionsBuilder;
    // getter espliciti
    pub fn assume_crs(&self) -> Option<&str>;
    pub fn limits(&self) -> &ResourceLimits;
    pub fn format_options(&self) -> &BTreeMap<String, String>;
    pub fn cancellation(&self) -> &CancellationToken;
}

pub struct ReadOptionsBuilder { /* opaque */ }
impl ReadOptionsBuilder {
    pub fn assume_crs(self, crs: impl Into<String>) -> Self;
    pub fn limits(self, limits: ResourceLimits) -> Self;
    pub fn format_option(self, key: impl Into<String>, value: impl Into<String>) -> Self;
    pub fn cancellation(self, token: CancellationToken) -> Self;
    pub fn build(self) -> ReadOptions;
}

#[non_exhaustive]
pub struct ResourceLimits { /* opaque, unico modello post-PR-2 */ }
```

**Nota di prerequisito su `ResourceLimits`**: la facade **non deve
congelare due modelli** duplicati (l'attuale `Limits` per-cella +
`ResourceLimits` cumulativo di `plenora-io-model`). PR-2 della
ROADMAP-1.1.0 chiede l'unificazione. Fino a chiusura di PR-2, la
facade **non** puo' essere ratificata: cristallizzeremmo il debito
che stiamo pianificando di rimuovere. Il piano G.1 riflette questo
prerequisito.

```rust
#[non_exhaustive]
pub struct ReadRequest { /* opaque */ }
impl ReadRequest {
    pub fn new(layer: LayerId) -> Self;
    pub fn builder(layer: LayerId) -> ReadRequestBuilder;
    // getter espliciti
    pub fn projected_fields(&self) -> Option<&[FieldId]>;
    pub fn projection_mode(&self) -> ProjectionMode;
    pub fn spatial_pruning_hint(&self) -> Option<&Bbox>;
    pub fn scope(&self) -> ReadScope;
    pub fn batch_target(&self) -> BatchTarget;
}

pub struct ReadRequestBuilder { /* opaque */ }
impl ReadRequestBuilder {
    pub fn project(self, fields: impl IntoIterator<Item = FieldId>) -> Self;
    pub fn projection_mode(self, mode: ProjectionMode) -> Self;
    pub fn spatial_hint(self, bbox: Bbox) -> Self;
    pub fn scope(self, scope: ReadScope) -> Self;
    pub fn batch_target(self, target: BatchTarget) -> Self;
    pub fn build(self) -> ReadRequest;
}

pub struct CancellationToken { /* opaque, Clone */ }
impl CancellationToken {
    pub fn new() -> Self;
    pub fn cancel(&self);
    pub fn is_cancelled(&self) -> bool;
    pub fn with_deadline(self, deadline: std::time::Instant) -> Self;
}
```

**`fidelity`**:

```rust
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct FidelityAssessment {
    pub format: &'static str,
    pub level: Fidelity,
    pub reasons: Vec<FidelityReason>,
}

#[non_exhaustive]
pub enum Fidelity { Lossless, Conditional, Approximating }

#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct LossReport {
    pub counts: BTreeMap<String, u64>,
    pub examples: Vec<LossExample>,
}
```

**`error`**:

```rust
#[non_exhaustive]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Error {
    pub code: IoErrorCode,
    pub category: ErrorCategory,
    pub phase: ErrorPhase,
    pub remote_effect: RemoteEffect,
    pub retry: RetryDisposition,
    pub driver: Option<String>,
    pub message: String,
    pub row_diagnostics: Option<RowDiagnostics>,
}

#[non_exhaustive]
pub enum ErrorCategory { /* enum canonico */ }
// ... stesse enum del model interno, ma `#[non_exhaustive]`
```

**Nota**: la facade **non ridefinisce** i tipi Arrow (`RecordBatch`,
`SchemaRef`): li ri-esporta dalla stessa versione pinnata del
workspace. Un consumer riceve `arrow_array::RecordBatch` reale, non un
wrapper.

### C.3 Cosa NON entra nella facade

- Trait interni `FormatDriver`, `FormatWriter`. Restano in
  `plenora-io-core`, non esposti.
- Registry dei driver. La facade lo usa internamente.
- Adapter interni (`BudgetedReader`, `LimitedWriter`,
  `with_cancellation`).
- Codec WKB (`plenora-io-model::wkb`). Restano interni: la facade
  espone i tipi contrattuali, non i decoder.

## D. API CLI

Le sei buste congelate di `cli-protocol-v1.json` restano immutate.
Nuovi comandi introducono nuovi contract nomi
(`plenora-io-formats-v1`, `plenora-io-options-v1`,
`plenora-io-schema-v1`, `plenora-io-validate-v1`,
`plenora-io-progress-v1`) — additivo, mai in place.

### D.1 `catalog` (v1 esistente, invariato)

```
plenora-io catalog
```

### D.2 `formats` (nuovo)

```
plenora-io formats
plenora-io formats --format markdown
```

Envelope `plenora-io-formats-v1`:

```json
{
  "status": "ok",
  "protocol_version": 1,
  "contract": "plenora-io-formats-v1",
  "formats": [
    {"id":"geoparquet","direction":"bidirectional","available":true,"required_feature":null},
    ...
  ]
}
```

Vista `markdown`:

```
| id          | direction     | available | required_feature |
|-------------|---------------|-----------|------------------|
| geoparquet  | bidirectional | true      | -                |
| filegdb     | bidirectional | false     | gdal-backend     |
```

### D.3 `options <format>` (nuovo)

```
plenora-io options csv
plenora-io options geoparquet --format markdown
```

Envelope `plenora-io-options-v1`:

```json
{
  "status": "ok",
  "protocol_version": 1,
  "contract": "plenora-io-options-v1",
  "format": "csv",
  "options": [
    {"key":"delimiter","type":"char","default":",","semantics":"CSV field delimiter"},
    {"key":"wkt_column","type":"string","default":null,"semantics":"Column name containing WKT geometry"}
  ]
}
```

### D.4 `inspect` (v1 esistente, invariato)

### D.5 `layers` (v1 esistente, invariato)

### D.6 `schema <path>` (nuovo)

```
plenora-io schema input.parquet --layer 0
plenora-io schema input.gpkg --layer 2 --format markdown
```

Envelope `plenora-io-schema-v1`:

```json
{
  "status": "ok",
  "protocol_version": 1,
  "contract": "plenora-io-schema-v1",
  "format": "geoparquet",
  "layer": {"id":0,"name":"buildings"},
  "schema": {
    "fields": [
      {"name":"geometry","type":"Binary","nullable":false,"is_geometry":true,"crs":"EPSG:4326"},
      {"name":"id","type":"Int64","nullable":false,"is_geometry":false}
    ]
  }
}
```

### D.7 `validate <path>` (nuovo)

```
plenora-io validate input.gpkg
plenora-io validate input.csv --in-opt delimiter=';' --assume-crs=EPSG:4326
```

Envelope `plenora-io-validate-v1`:

```json
{
  "status": "ok",
  "protocol_version": 1,
  "contract": "plenora-io-validate-v1",
  "format": "gpkg",
  "layers_validated": 2,
  "rows_scanned": 148213,
  "fidelity": { /* FidelityAssessment */ },
  "loss": { /* LossReport */ },
  "verdict": "passed"
}
```

Errore: envelope standard `plenora-io-error-v1` con
`category=DataMapping|Contract|…` e `verdict` implicito
(l'errore stesso e' il verdetto).

### D.8 `read` (v1 esistente, invariato)

Il comando esistente `read` **non emette batch materializzati su
stdout**. Legge la sorgente, applica limiti e projection, e produce
un envelope `plenora-io-read-v1` con: conteggio righe scandite,
`FidelityAssessment`, `LossReport`, eventuale campione di righe
soggetto al limite. E' un comando di **osservazione**, non di
estrazione bulk. Non e' modificato ne' esteso in questo MVP.

Per l'estrazione bulk di batch Arrow su stdout serve un comando
distinto (`scan`), documentato in D.9 come **fuori MVP**.

### D.9 `scan` (fuori MVP, proposta per v2)

```
plenora-io scan input.parquet --output arrow-ipc > out.arrow
plenora-io scan input.parquet --output ndjson --layer 0 | jq .
```

Envelope: nessuno (stream binario o NDJSON), con envelope d'errore
standard su stderr in caso di fail.

**Motivo del rinvio a v2**: `scan` introduce un percorso che non
produce JSON canonico su stdout, spezzando l'invariante "un singolo
documento su stdout" congelato da `cli-protocol-v1`. La
formalizzazione richiede una CIA dedicata e possibilmente un nuovo
contratto di stream (non un semplice envelope).

**Conseguenza per l'MVP**: senza `scan`, il CLI copre catalog,
ispezione, schema, validazione e conversione file→file, ma **non
copre l'estrazione streaming di dati Arrow**. Un utente che vuole
ingerire un dataset in un processo downstream via pipe Unix deve
usare `convert` verso un file intermedio, o attendere v2. Va
comunicato esplicitamente nelle note di release: "MVP non e' un CLI
completo di data extraction; il caso d'uso batch-a-stdout e' target
di v2".

### D.10 `convert` (v1 esistente, invariato)

## E. API Python

### E.1 Esempi di uso

**Apertura e ispezione**:

```python
import plenora_io

with plenora_io.open("input.gpkg") as ds:
    for layer in ds.layers():
        print(layer.name, layer.schema)
    fidelity = ds.inspect().fidelity
    print(fidelity.level)  # "Conditional"
```

**Lettura iterativa** (semantica pre-ADR-IO 7):

```python
with plenora_io.open("input.parquet") as ds:
    layer = ds.layers()[0]
    # Nota: finche' ADR-IO 7 non e' ratificata, la prima chiamata
    # a next(iter(...)) blocca finche' l'intera sorgente non e' stata
    # scandita dal BudgetedReader interno. Memoria bounded dal
    # budget, ma non dal singolo batch. Vedi B.2.
    for batch in ds.read(layer.id,
                          projected_fields=["geometry", "id"],
                          bbox=(12.0, 45.0, 13.0, 46.0),
                          max_rows=100_000):
        # batch e' un pyarrow.RecordBatch ricostruito dai bytes IPC
        # emessi dal binding Rust: non e' pass-through zero-copy.
        process(batch)
```

**Cancellazione esplicita** (nessun signal handler globale
installato dalla libreria):

```python
import signal
import plenora_io

token = plenora_io.CancellationToken()

def handle_sigint(signum, frame):
    # L'application code decide come reagire; la libreria
    # non installa nulla nel processo host.
    token.cancel()

signal.signal(signal.SIGINT, handle_sigint)

with plenora_io.open("big.gpkg") as ds:
    layer = ds.layers()[0]
    try:
        for batch in ds.read(layer.id, cancellation_token=token):
            process(batch)
    except plenora_io.errors.PlenoraCancelledError:
        # Il chiamante gestisce l'interruzione come preferisce.
        cleanup()
```

**Conversione**:

```python
from plenora_io import convert

published = convert("input.gpkg", "output.parquet",
                    durable=True,
                    limits={"max_rows": 10_000_000})
print(published.bytes, published.conversion_fidelity.level)
```

**Errori**:

```python
from plenora_io import errors

try:
    with plenora_io.open("hostile.arrow") as ds:
        ...
except errors.PlenoraContractError as e:
    print(e.category, e.phase, e.remote_effect, e.driver)
except errors.PlenoraLimitExceededError as e:
    print("limit hit:", e.message)
```

### E.2 Type signatures (`_native.pyi`)

```python
from typing import Iterator, Optional, Mapping, Sequence
import pyarrow

class Layer:
    id: int
    name: str
    schema: pyarrow.Schema
    crs: Optional[str]
    geometry_field: Optional[str]

class FidelityAssessment:
    format: str
    level: str  # "Lossless" | "Conditional" | "Approximating"
    reasons: list[dict]

class LossReport:
    counts: Mapping[str, int]
    examples: list[dict]

class Published:
    bytes: int
    outcome: str  # "Published" | "PublishedButDurabilityUnconfirmed"
    read_loss: LossReport
    write_loss: LossReport
    conversion_fidelity: FidelityAssessment

class Dataset:
    def __enter__(self) -> "Dataset": ...
    def __exit__(self, *args) -> None: ...
    def close(self) -> None: ...
    @property
    def is_closed(self) -> bool: ...
    def layers(self) -> list[Layer]: ...
    def inspect(self) -> dict: ...
    def read(
        self,
        layer_id: int,
        *,
        projected_fields: Optional[Sequence[str]] = None,
        bbox: Optional[tuple[float, float, float, float]] = None,
        max_rows: Optional[int] = None,
        cancellation_token: Optional["CancellationToken"] = None,
    ) -> Iterator[pyarrow.RecordBatch]: ...

def open(path: str, *, assume_crs: Optional[str] = None,
         format_options: Optional[Mapping[str, str]] = None,
         limits: Optional[Mapping[str, int]] = None) -> Dataset: ...

def convert(source: str, destination: str, *, durable: bool = False,
            in_options: Optional[Mapping[str, str]] = None,
            out_options: Optional[Mapping[str, str]] = None,
            limits: Optional[Mapping[str, int]] = None) -> Published: ...

def catalog() -> list[dict]: ...
def options(format_id: str) -> list[dict]: ...

class CancellationToken:
    def __init__(self) -> None: ...
    def cancel(self) -> None: ...
    def is_cancelled(self) -> bool: ...
```

## F. Decisione Arrow: IPC vs C Data Interface

### F.1 Opzione 1 — Arrow IPC bytes (reference)

Il binding Rust **serializza** ogni `RecordBatch` in un messaggio IPC
completo (schema header + batch + EOS marker) via
`arrow_ipc::StreamWriter` su un `Vec<u8>`. Il side Python **legge e
deserializza** i bytes con `pyarrow.ipc.open_stream(io.BytesIO(bytes))`,
ricostruendo un nuovo `pyarrow.RecordBatch` a partire dai buffer IPC.
**Non e' pass-through**: i buffer di memoria del `RecordBatch` Rust
non vengono condivisi con Python.

- **Copie**: almeno **una copia esplicita per buffer** durante la
  serializzazione (arrow-rs scrive i buffer nel Vec IPC) piu' una
  **ricostruzione a partire dagli offset IPC** in pyarrow.
  Il costo effettivo dipende dal profilo del batch (numero di
  campi, presenza di stringhe/binari con offset, compressione), e
  va misurato caso per caso.
- **Lifetime**: nessun ownership condiviso. I bytes IPC sono
  self-contained e vengono droppati dopo la deserializzazione.
- **Compatibilita' `forbid(unsafe_code)`**: totale. Nessun raw
  pointer, nessun FFI oltre PyBytes.
- **Complessita'**: bassa. Codice del reference (~80 righe in
  `arrow_reader.rs`) e' un modello utilizzabile.
- **Prestazioni**: **non misurate** in questo repository. Il costo
  atteso ha due componenti separate — CPU per la serializzazione
  IPC in Rust e CPU per la deserializzazione in pyarrow — la cui
  somma va confrontata con l'alternativa in F.2 su carichi
  rappresentativi (vedi benchmark gate in F.3).
- **Packaging**: nessun vincolo di versione forte su pyarrow. La
  compatibilita' e' data dalla stabilita' del formato IPC.

### F.2 Opzione 2 — Arrow C Data / C Stream Interface

Il binding Rust espone un `ArrowArrayStream` C-compatibile; il side
Python lo importa con `pyarrow.RecordBatchReader._import_from_c` (o
la relativa API pubblica di piu' recente introduzione).

- **Copie**: zero-copy dei buffer dati; struct C piccole (`ArrowSchema`,
  `ArrowArray`) vengono comunque materializzate.
- **Lifetime**: gestione del `release` callback ownership-safe se il
  binding usa un wrapper safe. Errori qui causano use-after-free o
  memory leak.
- **Compatibilita' `forbid(unsafe_code)`**: **da valutare per crate**.
  L'FFI verso C richiede tipicamente `unsafe` per costruire e
  smontare i puntatori C-ABI. Nel nostro workspace `unsafe_code`
  e' `forbid` sulle librerie: se l'implementazione riesce a
  poggiarsi interamente su wrapper safe forniti da `arrow-array`
  (che gia' espone `arrow_array::ffi::{FFI_ArrowArray,
  FFI_ArrowSchema}` con API safe) e da `pyo3` (per il passaggio
  della `PyCapsule`), la deroga non e' strettamente necessaria. Se
  invece richiede blocchi `unsafe` diretti, andrebbe isolato in una
  crate dedicata `plenora-io-py-ffi` con `unsafe_code = "allow"`,
  escluso dal gate anti-panic e coperto da suite dedicata. La
  decisione tra "safe wrapper" e "isolamento con deroga" e'
  concreta solo dopo un prototipo minimo.
- **Complessita'**: significativa. La sequenza `get_schema` →
  `get_next` → `release` va rispettata con precisione; gli errori
  devono attraversare il confine C senza panic.
- **Prestazioni**: attese ottimali per batch grandi con pochi campi
  variabili; il vantaggio si riduce su batch molto frammentati.
- **Packaging**: vincolo su pyarrow >= 14 (Arrow PyCapsule Interface
  stabile) — verificare compatibilita' col target `abi3-py310`.

### F.3 Raccomandazione

**MVP: Opzione 1 (IPC bytes)**. Motivazioni:
- Allineata al reference — codice riusabile 1:1.
- Compatibile con `forbid(unsafe_code)` senza deroghe ne'
  incertezza (nessun prototipo richiesto per validarla).
- Superficie di errore minima (nessuna gestione manuale di
  callback C).
- Nessun vincolo di versione forte su pyarrow.

**Benchmark gate (requisito prima del tag MVP)**: un microbenchmark
committato che misuri il costo end-to-end su tre workload — batch
piccoli (~1k righe), batch medi (~64k righe), batch grandi (~1M
righe) — e riporti tempo CPU e RSS per il boundary IPC. Il numero
diventa la baseline. Nessuna dichiarazione prestazionale nel
documento senza un benchmark corrispondente.

**Versione definitiva: valutazione dopo l'MVP**. Se il benchmark
gate documenta un delta prestazionale problematico per un caso
d'uso concreto (es. batch grandi con molti campi di dimensione
fissa), si valuta l'Opzione 2 come sperimentazione parallela. La
scelta di consolidare l'Opzione 2 richiede:

1. Prototipo che verifichi se `forbid(unsafe_code)` regge poggiando
   sui wrapper safe di `arrow-array::ffi` + `pyo3::PyCapsule`.
2. CIA dedicata sul flusso `release` (correttezza, memoria).
3. Confronto misurato con l'MVP sui tre workload del benchmark
   gate.
4. Se serve una deroga a `unsafe_code`, isolamento in
   `plenora-io-py-ffi` con perimetro dichiarato.

Nessuna di queste decisioni e' presa oggi.

## G. Piano di consegna

Sei fasi, in sequenza. La fase 0 raccoglie i prerequisiti architetturali
elencati nel sommario esecutivo; senza chiudere la fase 0, la facade
(fase 1) non puo' essere ratificata.

### G.0 Fase 0 — Prerequisiti architetturali

**Ambito**: chiusura di tre lotti gia' tracciati nella
`ROADMAP-1.1.0.md`, condizione per potere congelare una superficie
pubblica coerente.

- **PR-1** — ratifica ADR-IO 7 (draft in
  `docs/adr/ADR-IO-7-streaming-vs-operation-atomicity.md`) e
  implementazione della scelta. La facade deve documentare la
  semantica reale del `Dataset::read`; oggi e' operation-atomic e
  cristallizzarla nel binding Python senza ratifica consegna un
  contratto ingannevole.
- **PR-2** — unificazione di `Limits` e `ResourceLimits`. La facade
  espone `ResourceLimits` come modello unico; oggi convivono due
  modelli quasi-duplicati.
- **PR-3** — schema dichiarativo delle `format_options`. Serve al
  comando CLI `options <format>` e al metodo Python
  `plenora_io.options()`; senza, l'output sarebbe hard-coded per
  driver.

**Dipendenze**: nessuna esterna alla governance.

**Criteri di accettazione**:
- ADR-IO 7 in stato **Accepted** (non Draft).
- Unico modello `ResourceLimits` nel workspace (verificato da grep
  gate CI).
- Registry `format_options` per driver, con test snapshot.

**Rischi**: PR-1 richiede una scelta di implementazione (spool
bounded vs streaming con errore terminale) che ha impatto
cross-component. PR-2 tocca il core del budget system.

**Stima**: 8-12 giorni-persona (dipende da opzione scelta in PR-1).

### G.1 Fase 1 — Facade Rust `plenora-io-api`

**Ambito**: creazione della crate, superficie pubblica come da C.2
(tipi opachi + costruttori/builder stabili, reader concreto opaco,
nessun trait pubblico implementabile), gate CI di boundary
(`scripts/check_api_boundary.py`), suite di test sulla facade (non
replica dei test dei driver: verifica solo il contratto).

**Dipendenze**: G.0 completata e ratificata.

**Criteri di accettazione**:
- Tutti i tipi pubblici `#[non_exhaustive]` **e** dotati di
  costruttore/builder pubblico; verifica CI che il crate compili
  con `deny(missing_docs)` e che ogni tipo pubblico abbia almeno
  un metodo `new` o `builder`.
- `plenora-io-cli` NON compila piu' se importa direttamente
  `plenora-io-core` o `plenora-io-model` (regressione bloccante).
- Suite di test dedicata (~30 test) copre catalog, open, read,
  convert, error mapping, risoluzione `projected_fields`.
- CI verde su tutti i gate (fmt, clippy, anti-panic,
  cross-component).

**Rischi**: la traduzione dei tipi interni verso la facade richiede
mapping bidirezionali; una svista rompe il gate anti-panic.
Mitigazione: tests-first sui costruttori pubblici.

**Stima**: 3-5 giorni-persona.

### G.2 Fase 2 — CLI `plenora-io-cli` estensioni additive

**Ambito**: `plenora-io-cli` migra da import diretto del core a
import esclusivo della facade `plenora-io-api`. Nuovi comandi
`formats`, `options`, `schema`, `validate` con envelope dedicati.
Global flag `--format json|markdown|junit`. `--progress` opt-in
stretto (default off).

Non si introduce `cli-protocol-v2`: le estensioni sono additive e
convivono col v1 come contract paralleli registrati nel manifest.

**Dipendenze**: Fase 1 completata e ratificata. Fase 0 chiusa
implicitamente (PR-3 e' prerequisito del comando `options`;
PR-2 e' prerequisito del comando `validate`).

**Criteri di accettazione**:
- I sei envelope congelati esistenti sono byte-per-byte identici
  (snapshot test).
- I nuovi envelope sono aggiunti al manifest `cli-protocol-v1.json`
  come contract paralleli (aggiunta additiva alla lista degli
  envelope conosciuti, non modifica di quelli esistenti).
- Test end-to-end su ogni nuovo comando.
- Nessun import diretto del core (verificato dal gate).

**Rischi**: `--format markdown|junit` introduce viste derivate; il
rendering deve essere deterministic. Mitigazione: test snapshot su
output rendered.

**Stima**: 5-7 giorni-persona.

### G.3 Fase 3 — SDK Python `plenora-io-py`

**Ambito**: cdylib PyO3 come da B.2, gerarchia errori, boundary
Arrow IPC (opzione F.1) con **benchmark gate obbligatorio prima del
tag** (vedi F.3), context manager, cancellazione via
`CancellationToken` esplicito (nessun signal handler globale), test
Python su fixture filesystem.

**Dipendenze**: Fase 1 completata. Fase 2 non e' bloccante
(possibile in parallelo, ma sconsigliato: vale la pena chiudere il
CLI prima per ridurre WIP).

**Criteri di accettazione**:
- `import plenora_io; plenora_io.open("test.parquet")` funziona su
  Linux/macOS/Windows.
- Test Python (>= 20) passano su fixture generate al setup.
- `.pyi` completi, `py.typed` presente.
- Nessuna deroga a `forbid(unsafe_code)` sulla crate PyO3.
- Il binding **non** installa signal handler globali (verificato
  da un test che confronta gli handler prima/dopo l'import).
- Benchmark gate F.3 committato con baseline pubblicata.

**Rischi**: PyO3 0.23 ha API in evoluzione — pinnare la versione
esatta e verificare la riproducibilita' con `Cargo.lock` committato.

**Stima**: 5-7 giorni-persona.

### G.4 Fase 4 — Wheel & packaging

**Ambito**: GitHub Actions per la matrice wheel documentata sotto.
Smoke test import + `version()` post-build. Pubblicazione opt-in su
PyPI (test index prima, poi prod).

**Matrice wheel supportata** (allineata al reference database-tools):

| Piattaforma | Architettura | Toolchain | Wheel tag |
|---|---|---|---|
| Linux (manylinux 2.34) | x86_64 | Ubuntu 24.04 runner | `manylinux_2_34_x86_64` |
| macOS 14 | arm64 (Apple Silicon) | macos-14 runner | `macosx_14_0_arm64` |
| Windows Server | x86_64 | windows-2022 runner | `win_amd64` |

Per ognuna: **una sola wheel** `.abi3.so`/`.abi3.pyd` con tag Python
`cp310-abi3` (compatibile Python 3.10 → 3.13+ tramite abi3).

**Non supportati in MVP** (allineato al reference; da valutare
esplicitamente se emerge domanda documentata):
- Linux aarch64, musllinux (x86_64 o aarch64), Windows arm64,
  macOS x86_64 (Intel).

**Politica GDAL**:
- La wheel default e' **slim, pure-Rust**: il feature
  `gdal-backend` di `driver-filegdb` NON e' abilitato. Il driver
  FileGDB compare nel catalogo come `available=false,
  required_feature="gdal-backend"`, coerentemente con la CLI
  esistente.
- Una variante `plenora-io-full` (o wheel affiancata) con
  `gdal-backend` abilitato e libgdal linkata **e' considerata solo
  dopo verifica del packaging** (glibc target, dimensioni,
  ridistribuzione libgdal su tutte le piattaforme). Fuori MVP.

**Dipendenze**: Fase 3 completata.

**Criteri di accettazione**:
- Tre wheel slim prodotte per ogni tag di release Python.
- Smoke test verde su tutte le tre piattaforme.
- Nessuna dipendenza C non stabile nella slim wheel.
- Nota di release che spiega esplicitamente l'assenza di GDAL
  nella wheel default.

**Rischi**: manylinux glibc version — 2.34 e' ragionevole nel
2026, ma verificare compatibilita' con i sistemi target dichiarati
dagli utilizzatori.

**Stima**: 3-4 giorni-persona (slim MVP; +5-8 giorni-persona se e
quando si aggiunge la variante `full` con GDAL, non conteggiati
nel totale MVP).

### G.5 Fase 5 — Conformance e release

**Ambito**: catena cross-component (IO ↔ data-tools ↔
database-tools) rieseguita sui nuovi artefatti (facade + CLI + SDK
Python). Nuova baseline `release/1.2.0.json` (assumendo che il
release manifest bumpi minor per la nuova superficie SDK). CIA
formale per l'introduzione della facade e del binding Python. ADR
dedicato all'API boundary.

**Dipendenze**: Fasi 0-4 completate.

**Criteri di accettazione**:
- CI same-SHA verde su tutte le matrici (Linux, Windows, macOS,
  GDAL matrix del CLI, wheel Python).
- Cross-component roundtrip su dati reali passa.
- Coverage LCOV >= 80% sul solo codice di libreria (incluso il
  binding).
- Tag `v1.2.0` proposto (non creato senza ratifica).

**Rischi**: il release manifest e la CIA della facade sono attivita'
di governance, non tecniche — il rischio e' politico piu' che
implementativo.

**Stima**: 3-5 giorni-persona.

**Totale stimato**: **27-40 giorni-persona sequenziali** inclusi i
prerequisiti (G.0 = 8-12, G.1 = 3-5, G.2 = 5-7, G.3 = 5-7, G.4 =
3-4, G.5 = 3-5), in un arco temporale di **6-9 settimane
calendariali** con overlap parziale fra Fase 2 e Fase 3 dove il team
lo consente. La stima **non include** la variante wheel `full` con
GDAL (fuori MVP).

## Decisioni nette per l'MVP

Queste sono le scelte proposte come vincolanti se la proposta viene
ratificata. Nessuna e' presa oggi.

1. **Prerequisiti (G.0) chiusi prima di aprire G.1**. Nessuna facade
   ratificata prima di ADR-IO 7 (PR-1), unificazione `Limits` (PR-2),
   registry `format_options` (PR-3).
2. **Facade dedicata `plenora-io-api`** con tutti i tipi pubblici
   `#[non_exhaustive]` **e** costruttore/builder stabile per ognuno.
3. **Nessun trait pubblico implementabile**: reader concreto opaco
   `DatasetReader` (con opzione sealed trait pronta se in futuro
   servissero piu' reader).
4. **CLI: estensioni additive**, non `cli-protocol-v2`. Quattro
   comandi nuovi (`formats`, `options`, `schema`, `validate`) con
   contract dedicati. Sei envelope v1 esistenti invariati.
5. **`--progress` opt-in stretto** (default off). Nessun autodetect
   TTY, nessun default "attivo quando interattivo".
6. **`scan` fuori MVP**. L'MVP non e' un CLI completo per l'estrazione
   dati: copre catalog/inspection/schema/validate/convert file→file,
   non batch-a-stdout. La nota di release lo dichiara esplicitamente.
7. **Binding Python sync-only**. Nessun `AsyncDataset`, nessun runtime
   tokio.
8. **Nessun signal handler globale installato dalla libreria Python**.
   Cancellazione esclusivamente via `CancellationToken` esplicito.
9. **Arrow IPC bytes** come boundary Python↔Rust per l'MVP, con
   **benchmark gate committato** prima del tag. Nessuna
   dichiarazione prestazionale senza misura.
10. **Wheel matrix**: Linux `manylinux_2_34_x86_64`, macOS
    `macosx_14_0_arm64`, Windows `win_amd64`, un solo tag Python
    `cp310-abi3` per piattaforma.
11. **Wheel default slim, pure-Rust**. GDAL non abilitato di default.
    Variante `full` con GDAL come lavoro separato solo dopo verifica
    del packaging.
12. **Gerarchia errori parallela al reference** (12 classi meno le 5
    database-specific), attributi `category/phase/remote_effect/retry`
    coerenti con `plenora-io-error-v1`.
13. **Versionamento binding Python decoupled** dal workspace
    (`plenora-io-py` 0.1.0 al primo rilascio).
14. **`plenora-io-api` ri-esporta i tipi Arrow** pinnati dal
    workspace, cosi' il consumer non deve conoscere la versione
    esatta.

## Decisioni ancora aperte

1. Quale opzione di ADR-IO 7 va scelta in PR-1: spool bounded
   (opzione A dell'ADR) vs streaming con errore terminale (opzione B)
   vs ibrido opt-in (opzione C). **Decisione di governance**, non
   tecnica; blocca G.0 → G.1.
2. Comando `benchmark-*` nel CLI: replicare la famiglia
   `benchmark-oltp|read|write|spatial` del reference? **Raccomandazione:
   NO** — appartiene a `plenora-bench`, non al CLI pubblico. Da
   confermare.
3. Quando ratificare la variante wheel `full` con GDAL: dipende
   dalla domanda utenti. **Aperto**.
4. Quando ratificare la valutazione C Data Interface (opzione F.2):
   dipende dal benchmark gate dell'MVP. **Aperto**.
5. Estensione della matrice wheel (Linux aarch64, musllinux,
   Windows arm64, macOS Intel): dipende dalla base utenti dichiarata.
   **Aperto**.
6. Test end-to-end del rollback filesystem per il finding #10 SHP
   (mock di file system che rifiuta selettivamente rename
   simmetrici): trasversale a IO-tools, non specifico al piano
   CLI/SDK. **Aperto**.

## Fuori scope

- Modifiche a codice compilato del working tree attuale.
- Modifiche a `release/*.json` o a `Cargo.toml` del workspace.
- Creazione di commit.
- Implementazione di codice (proposta puramente documentale).
- Bump di versione del workspace.
- Dichiarazioni di production-readiness — CLI completo e SDK Python
  restano lavori separati, con propri cicli di maturazione, e sono
  contenuto di questo piano ma non oggetto della sua ratifica.
- Modifiche a `plenora-database-tools` (reference letto in
  sola lettura).

## Nota di scope sul repository IO-tools

Questa proposta e' un **file nuovo non tracciato** nel working tree
(`docs/PROPOSAL-CLI-SDK-facade.md`). Nessun codice compilato, nessun
manifest di release, nessun `Cargo.toml` e' stato toccato per
produrla. La dichiarazione "nessuna modifica al repository" va letta
come "nessuna modifica a codice compilato, manifesti di release,
configurazione del workspace o `.claude/` state".

## Riferimenti

- `docs/REVIEW-2026-08-15.md`
- `docs/ROADMAP-1.1.0.md`
- `docs/PROPOSAL-L6-progressive-wkt-geojson.md`
- `docs/adr/ADR-IO-7-streaming-vs-operation-atomicity.md`
- Reference: `C:\Users\marco\Desktop\database-tools`
  - `crates/plenora-database-cli/` (CLI reference)
  - `crates/plenora-database-py/` (Python SDK reference)
  - `crates/plenora-database-core/` (core diretto senza facade)
  - `contracts/v1/` (contratti JSON congelati del reference)
