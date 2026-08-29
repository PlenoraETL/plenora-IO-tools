# plenora-IO-tools

Libreria e CLI per leggere e scrivere dati geospaziali in dieci formati,
attraverso un modello semantico unico basato su Arrow.

Ogni formato è un driver dietro la stessa interfaccia. Chi legge non deve sapere
se dietro c'è un CSV o un GeoPackage: ottiene `RecordBatch` Arrow, un contratto
di layer che dichiara schema, geometria e CRS, e un report di ciò che il formato
non ha saputo rappresentare.

```
release_authorized: false
```

Il componente **non è autorizzato al rilascio**. Le condizioni ancora aperte, e
che cosa serve per chiuderle, sono in [docs/RELEASE.md](docs/RELEASE.md).

## Formati

| | Lettura | Scrittura | Feature richiesta |
|---|---|---|---|
| CSV | sì | sì | — |
| GeoJSON | sì | sì | — |
| KML | sì | sì | — |
| Shapefile | sì | sì | — |
| GeoPackage | sì | sì | — |
| GeoParquet | sì | sì | — |
| Arrow IPC | sì | sì | — |
| XLSX | sì | sì | — |
| DXF | sì | sì | — |
| FileGDB | sì | sì | `gdal-backend` |

Senza la feature `gdal-backend` il driver FileGDB resta uno stub tipizzato: le
sue chiamate falliscono con una capability mancante, non con un errore di
ambiente.

## Uso come libreria

```rust
use plenora_io_core::driver::{FormatDriver, ReadOptions, Source};
use plenora_io_core::request::{BatchTarget, ProjectionMode, ReadRequest, ReadScope};
use plenora_io_model::budget::{PipelineBudget, PipelineLimits};
use plenora_io_model::contract::LayerId;

let bundle = PipelineBudget::builder().limits(PipelineLimits::default()).build()?;
let opzioni = ReadOptions::from_read_parts(bundle.into_read_parts());

let dataset = driver_geojson::GeoJsonDriver.open(Source::Path(percorso), opzioni)?;
let contratto = &dataset.layers()[0];

let mut reader = dataset.open_layer_reader(&ReadRequest {
    layer: LayerId(0),
    projected_fields: None,
    projection_mode: ProjectionMode::BestEffort,
    pruning_predicate: None,
    spatial_pruning_hint: None,
    scope: ReadScope::Complete,
    batch_target: BatchTarget::default(),
    cancellation: Default::default(),
})?;

while let Some(batch) = reader.next_batch()? {
    // RecordBatch Arrow, con la geometria nella colonna dichiarata dal contratto
}
```

L'API Rust è **interna e instabile**: non porta garanzia semver e i crate non
sono pubblicati. La superficie con garanzia di compatibilità è il JSON della
CLI.

## Uso come CLI

```
plenora-io catalog                      # i dieci driver e le loro capacità
plenora-io inspect  <file>              # schema, geometria, CRS, fedeltà
plenora-io layers   <file>              # i layer del dataset
plenora-io read     <file> [--limit N]  # valida fino a EOF e conta ciò che passa
plenora-io convert  <ingresso> <uscita> # conversione fra due formati
```

Ogni comando emette un documento JSON con un contratto versionato. Gli errori
escono su `stderr` come `plenora-io-error-v1`, con un codice stabile e un exit
code dedicato.

**`read` non stampa le righe.** Attraversa la sorgente applicando l'intero
contratto di lettura e riporta `rows_read`, `batches`, `truncated`, il layer e la
fedeltà: è una validazione che conta, non un dump. I campi del documento sono
esattamente quelli fissati da [`release/cli-protocol-v2.json`](release/cli-protocol-v2.json)
e dal v1 congelato.

Una versione precedente di questo file prometteva «righe come JSON», e il comando
non le ha **mai** emesse: la prima implementazione della CLI restituiva già
`rows_read` e `batches`, come quella di oggi. A essere sbagliata era la riga, non
il comando, ed è la riga a cambiare — aggiungere le righe al documento sarebbe
stato cambiare un contratto pubblico per far tornare una frase. Esportarle sarà
semmai una funzionalità distinta, con la propria versione di contratto; oggi il
modo di ottenere i dati è `convert` verso un formato che si sa rileggere.

### Flag

Valgono per tutti i comandi che aprono una sorgente. Nessuno degrada a un
default quando il valore manca o è malformato: la CLI **fallisce chiuso**.

| flag | valore | effetto |
|---|---|---|
| `--assume-crs` | identificatore CRS | il CRS da assumere per i formati che non lo portano; `csv` lo esige |
| `--layer` | intero | opera su un solo layer invece che su tutti |
| `--limit` | intero | ferma `read` dopo N righe accettate e dichiara `truncated` |
| `--durable` | — | `fsync` di file e directory prima del rename; l'esito di durabilità è riportato |
| `--opt` | `chiave=valore` | opzione di formato per entrambe le direzioni |
| `--in-opt` | `chiave=valore` | come `--opt`, ma sovrascrive per chiave sull'ingresso |
| `--out-opt` | `chiave=valore` | come `--opt`, ma sovrascrive per chiave sull'uscita |
| `--legacy-protocol-v1-unsafe` | — | emette il protocollo v1 congelato invece del v2; il nome dice che cosa si sceglie |
| `--version`, `-V` | — | la versione del binario |

Le quote finiscono tutte in `PipelineLimits` e condividono la stessa disciplina.
Zero è rifiutato da ciascuna; i default sono quelli del modello.

| flag | default | governa |
|---|---|---|
| `--deadline-ms` | `30000` | la deadline dell'operazione |
| `--memory-bytes` | `536870912` | la memoria della pipeline; lo spool migra su file a **metà** della capacità effettiva |
| `--max-input-bytes` | `268435456` | i byte della sorgente, contati prima di aprirla |
| `--max-input-entries` | `10000` | le entry di una sorgente che è una directory |
| `--max-output-bytes` | `1073741824` | i byte della destinazione |
| `--max-rows` | `10000000` | le righe dell'operazione |
| `--max-columns` | `4096` | le colonne del contratto |
| `--max-vertices` | `50000000` | i vertici complessivi |
| `--max-wkb-cell-bytes` | `67108864` | i byte di **una** geometria; non può superare `--memory-bytes` |
| `--max-wkb-components` | `100000` | i componenti di una geometria |
| `--max-wkb-depth` | `64` | l'annidamento di una geometria |

Due vincoli di coerenza si notano solo quando si stringe:

* `--max-wkb-cell-bytes` non può superare `--memory-bytes`, e la validazione lo
  rifiuta invece di ridurlo in silenzio;
* `--memory-bytes` deve restare **oltre il doppio** di un batch materializzato —
  il target di default è 8 MiB — perché sotto soglia lo spool non ha ancora
  migrato e la quota deve reggere buffer e batch insieme. Sotto quel valore
  l'errore è `LIMIT_EXCEEDED`, «batch materializzato oltre la quota prenotata».

### Ctrl+C

Il primo `SIGINT` **annulla in modo cooperativo**: la pipeline lo osserva ai
propri punti di verifica, ritorna un errore `CANCELLED` con exit code `130`, e
il rientro ordinato fa cadere staging e spool senza pubblicare nulla. Non è
un'interruzione immediata, e dentro una chiamata nativa che non ritorna — GDAL,
sul percorso FileGDB — nessuno guarda il token. Il **secondo** `SIGINT` esce
subito, e ciò che lo staging aveva in corso resta dov'è.

### Ambiente

| | |
|---|---|
| `PLENORA_SPILL_DIR` | la directory che ospita l'inode dello spool, per metterlo su un volume capiente o veloce. Il file non ha nome — è scollegato dal filesystem appena aperto — quindi non esistono orfani da spazzare. Se la variabile è impostata ma la directory non è utilizzabile, la creazione **fallisce chiuso**: nessun ripiego su un altro volume |

### Piattaforme

Ciò che la CI misura, e nient'altro.

| | build e test | note |
|---|---|---|
| Linux x86-64 | suite completa, fuzz, copertura, tutti i gate | è la piattaforma di riferimento; FileGDB in scrittura richiede GDAL ≥ quella di Ubuntu 24.04 |
| Windows x86-64 | `cargo test --workspace --all-targets`, FileGDB con GDAL nativo, publish cross-volume | il gestore di Ctrl+C c'è; la sonda che invia il segnale no, perché su Windows si applica a un gruppo di console e non a un PID |
| macOS | le sole prove di publish e i lint di sicurezza | il resto non è misurato su questa piattaforma, e non è dichiarato |

Il **recovery** di un publish Shapefile interrotto è in
[docs/PRODUCT.md § Publish](docs/PRODUCT.md#publish-che-cosa-diventa-visibile-e-quando).

## Documentazione

| | |
|---|---|
| [docs/PRODUCT.md](docs/PRODUCT.md) | che cosa offre e che cosa promette: driver, opzioni, contratti pubblici, limiti |
| [docs/ENGINEERING.md](docs/ENGINEERING.md) | come funziona e come viene verificato: architettura, pipeline, checkpoint, fuzzing |
| [docs/RELEASE.md](docs/RELEASE.md) | dove siamo e dove andiamo: stato misurato, blocchi aperti, ordine di lavoro |

Lo stato in forma strutturata è
[`assurance/current-state.json`](assurance/current-state.json); `docs/RELEASE.md`
ne riporta i numeri e un gate verifica che coincidano.

## Licenza

I due crate vendorizzati sotto `vendor/` conservano la propria licenza upstream
(MIT) e il proprio `LICENSE.txt`. La provenienza dei fork è registrata in
`assurance/registries/vendor-dxf-fork.json` e
`assurance/registries/vendor-gdal-fork.json`, e un gate la confronta con il
lockfile.
