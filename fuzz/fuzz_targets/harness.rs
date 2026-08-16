//! Scaffolding condiviso dai target che esercitano un `FormatDriver` reale.
//!
//! I driver di file (GeoPackage, GeoParquet, XLSX, CSV, Arrow IPC) leggono da
//! `Source::Path`, non da uno slice: l'input del fuzzer va quindi materializzato
//! in un file temporaneo con l'estensione che il driver instrada. Il modulo
//! tiene in un solo posto questa materializzazione, i limiti stretti della
//! campagna e il ciclo di lettura completo (tutti i layer, tutti i batch), così
//! ogni target resta la sola dichiarazione della superficie che copre.
//!
//! Incluso via `mod harness;` da più bin: le funzioni non usate da un singolo
//! target sono attese, non morte.
#![allow(dead_code)]

use std::io::Write as _;
use std::path::PathBuf;

use plenora_io_core::driver::{FormatDriver, ReadOptions, Sink, Source, WriteOptions};
use plenora_io_core::request::{
    BatchTarget, ProjectionMode, ReadRequest, ReadScope, WriteLayer, WritePlan,
};
use plenora_io_model::contract::{DataContract, LayerContract, LayerId};
use plenora_io_model::budget::{PipelineBudget, PipelineLimits};
use plenora_io_model::PlenoraIoError;

/// Tetto sull'input accettato dal target, allineato a `__fuzz_read_dxf`: oltre
/// questa soglia il costo per esecuzione degrada senza aggiungere copertura.
pub const MAX_FUZZ_INPUT_BYTES: usize = 1_048_576;

/// Limiti della campagna: stessi rami di validazione dei limiti di produzione,
/// tarati perché un input ostile non possa allocare oltre il budget di libFuzzer.
///
/// Memoria e spill sono stretti apposta: un target che potesse prenotare
/// centinaia di MiB farebbe fallire libFuzzer per OOM invece di segnalare il
/// difetto, e la campagna misurerebbe il proprio budget invece del codice.
pub fn limits() -> PipelineLimits {
    PipelineLimits::default()
        .with_max_input_bytes(MAX_FUZZ_INPUT_BYTES as u64)
        .with_max_rows(100_000)
        .with_max_columns(256)
        .with_max_vertices(1_000_000)
        .with_max_output_bytes(16 * 1024 * 1024)
        .with_max_wkb_cell_bytes(MAX_FUZZ_INPUT_BYTES)
        .with_memory_bytes(64 * 1024 * 1024)
        .with_spill_bytes(64 * 1024 * 1024)
}

/// Una pipeline della campagna.
///
/// Ogni chiamata ne costruisce una nuova: le opzioni portano un permit
/// one-shot, e riusarne una gia' osservata farebbe fallire il preflight per
/// una ragione che non ha nulla a che vedere con l'input sotto test.
fn bundle() -> plenora_io_model::budget::PipelineBundle {
    match PipelineBudget::builder().limits(limits()).build() {
        Ok(bundle) => bundle,
        Err(error) => unreachable!("limiti della campagna non validi: {error:?}"),
    }
}

pub fn read_options() -> ReadOptions {
    ReadOptions::from_read_parts(bundle().into_read_parts())
}

/// Configurazioni per i formati tabellari (CSV/XLSX): senza `assume_crs` e
/// senza una colonna geometria dichiarata l'apertura fallisce prima di leggere
/// una sola cella, e il target non coprirebbe nulla. I nomi sono quelli usati
/// dalle fixture del repo, così un seme realistico li incontra subito.
pub fn declared_geometry_read_options() -> Vec<ReadOptions> {
    let wkt = [("wkt_column".to_owned(), "geometry".to_owned())];
    let xy = [
        ("x_column".to_owned(), "x".to_owned()),
        ("y_column".to_owned(), "y".to_owned()),
    ];
    [wkt.to_vec(), xy.to_vec()]
        .into_iter()
        .map(|options| {
            read_options()
                .with_assume_crs("EPSG:4326")
                .with_format_options(options.into_iter().collect())
        })
        .collect()
}

pub fn read_request(layer: LayerId) -> ReadRequest {
    ReadRequest {
        layer,
        projected_fields: None,
        projection_mode: ProjectionMode::BestEffort,
        pruning_predicate: None,
        spatial_pruning_hint: None,
        scope: ReadScope::Complete,
        batch_target: BatchTarget {
            target_bytes: 1024 * 1024,
            max_rows: 4_096,
        },
        cancellation: plenora_io_model::CancellationToken::default(),
    }
}

/// Input materializzato su disco, con la directory che lo contiene.
///
/// La directory è deliberatamente esclusiva: SQLite può creare `-wal`, `-shm` o
/// `-journal` accanto al database, e un formato multi-file può generare sidecar
/// arbitrari. Cancellare l'intera directory è l'unico modo per garantire che una
/// campagna lunga non lasci residui sul filesystem.
pub struct Spilled {
    _directory: tempfile::TempDir,
    path: std::path::PathBuf,
}

impl Spilled {
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

/// Materializza l'input in una directory temporanea sotto `name` (estensione
/// inclusa: è quella che il driver instrada). `None` = input oltre il tetto o
/// filesystem non disponibile, l'esecuzione va scartata.
pub fn spill(bytes: &[u8], name: &str) -> Option<Spilled> {
    if bytes.len() > MAX_FUZZ_INPUT_BYTES {
        return None;
    }
    let directory = tempfile::Builder::new()
        .prefix("plenora-fuzz-")
        .tempdir()
        .ok()?;
    let path = directory.path().join(name);
    let mut file = std::fs::File::create(&path).ok()?;
    file.write_all(bytes).ok()?;
    file.flush().ok()?;
    drop(file);
    Some(Spilled {
        _directory: directory,
        path,
    })
}

/// Materializza l'input e affianca il percorso di un output ancora inesistente
/// nella stessa directory temporanea: il publish atomico rinomina dentro la
/// directory, quindi sorgente e destinazione devono stare sullo stesso
/// filesystem. `None` = esecuzione da scartare.
pub fn spill_with_output(bytes: &[u8], name: &str, output: &str) -> Option<(Spilled, PathBuf)> {
    let spilled = spill(bytes, name)?;
    let output = spilled.path().with_file_name(output);
    Some((spilled, output))
}

/// Legge un dataset con un driver e lo riscrive con un altro: è il percorso
/// `convert` della CLI ridotto all'essenziale. Serve a portare sui writer
/// contratti che l'input controlla per intero — schema, tipi, nullabilità,
/// CRS, nomi di layer — invece dei soli contratti sintetizzati dai test.
///
/// I due rami escono dalle **stesse** parti, come nella CLI: contatori
/// indipendenti, `PipelineContext` condiviso. Costruirne due separati qui
/// farebbe misurare alla campagna una forma che il codice spedito non usa.
pub fn convert(
    reader_driver: &dyn FormatDriver,
    input: PathBuf,
    writer_driver: &dyn FormatDriver,
    output: PathBuf,
) -> plenora_io_model::Result<u64> {
    let (read_parts, write_parts) = bundle().into_convert_parts().into_parts();
    let write_options = WriteOptions::from_write_parts(write_parts);
    let dataset = reader_driver.open(Source::Path(input), ReadOptions::from_read_parts(read_parts))?;
    let layers: Vec<LayerContract> = dataset.layers().to_vec();
    let plan = WritePlan {
        layers: layers
            .iter()
            .map(|layer| WriteLayer {
                name: layer.name.clone(),
                contract: DataContract {
                    schema: layer.contract.schema.clone(),
                    geometry: layer.contract.geometry.clone(),
                },
            })
            .collect(),
    };
    let mut writer = writer_driver.create(Sink::Path(output), &plan, &write_options)?;
    for (index, layer) in layers.iter().enumerate() {
        let sink_layer = LayerId(u32::try_from(index).map_err(|_| {
            PlenoraIoError::LimitExceeded("numero di layer non rappresentabile".to_owned())
        })?);
        let mut reader = dataset.open_layer_reader(&read_request(layer.id))?;
        let mut batches = Vec::new();
        let mut rows = 0_u64;
        while let Some(batch) = reader.next_batch()? {
            rows = rows.saturating_add(batch.num_rows() as u64);
            batches.push(batch);
        }
        // La cardinalità va dichiarata prima del primo write del layer,
        // altrimenti il validatore comune non può attestare la diagnostica.
        writer.declare_input_total(sink_layer, rows)?;
        for batch in &batches {
            writer.write_to_layer(sink_layer, batch)?;
        }
    }
    Ok(writer.finish()?.bytes)
}

/// Apre il dataset e drena ogni layer fino a EOF. `Err` = rifiuto legittimo del
/// driver; un panic durante il drenaggio è un finding.
pub fn read_all(
    driver: &dyn FormatDriver,
    path: std::path::PathBuf,
    options: ReadOptions,
) -> plenora_io_model::Result<usize> {
    let dataset = driver.open(Source::Path(path), options)?;
    let layer_ids: Vec<LayerId> = dataset.layers().iter().map(|layer| layer.id).collect();
    let mut rows = 0_usize;
    for layer in layer_ids {
        let mut reader = dataset.open_layer_reader(&read_request(layer))?;
        while let Some(batch) = reader.next_batch()? {
            rows = rows.saturating_add(batch.num_rows());
        }
        // Il contratto del reader è autoritativo: leggerlo dopo il drenaggio
        // esercita anche i driver che lo ricostruiscono a posteriori.
        let _ = reader.contract().contract.schema.fields().len();
        let _ = reader.loss_report();
    }
    Ok(rows)
}
