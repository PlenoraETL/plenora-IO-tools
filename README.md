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
plenora-io read     <file> [--limit N]  # righe come JSON
plenora-io convert  <ingresso> <uscita> # conversione fra due formati
```

Ogni comando emette un documento JSON con un contratto versionato. Gli errori
escono su `stderr` come `plenora-io-error-v1`, con un codice stabile e un exit
code dedicato.

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
