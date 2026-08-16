//! Benchmark A/B del percorso `convert` con e senza spill.
//!
//! L'harness principale misura `read` e `write` separatamente e con i limiti
//! di default: con quelli lo spool bounded introdotto in S2 non si attiva
//! quasi mai, e un risultato verde non direbbe nulla sul costo che ci
//! interessa. Qui il percorso misurato e' un `convert` completo — lettura
//! attraverso l'adapter operation-atomic e scrittura via driver — eseguito in
//! due varianti **comparabili** sullo stesso fixture:
//!
//! - `no-spill`: quota di memoria abbondante, i batch verificati restano in
//!   RAM per tutta l'operazione;
//! - `forced-spill`: quota di memoria stretta, la migrazione su file
//!   temporaneo avviene di sicuro.
//!
//! Ogni esecuzione **dichiara se lo spill e' avvenuto davvero**, leggendo la
//! quota residua di `SpillBytes`. Un `forced-spill` che non spilla e' un
//! risultato invalido, non un risultato buono: senza quel controllo il
//! benchmark misurerebbe due volte lo stesso percorso.
//!
//! Uso:
//!   plenora-bench-spool-ab --rows 200000 --variant no-spill
//!   plenora-bench-spool-ab --rows 200000 --variant forced-spill

use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use plenora_io_core::driver::{ReadOptions, Sink, Source, WriteOptions};
use plenora_io_core::request::{BatchTarget, ProjectionMode, ReadRequest, ReadScope};
use plenora_io_core::{FormatDriver, WriteLayer, WritePlan};
use plenora_io_model::budget::{PipelineBudget, PipelineContext, PipelineLimits};
use plenora_io_model::contract::LayerId;
use plenora_io_model::CancellationToken;

/// Quota di memoria della variante senza spill: ampia abbastanza da tenere
/// l'intero dataset di prova in RAM.
const ROOMY_MEMORY_BYTES: u64 = 1024 * 1024 * 1024;

/// Quota di memoria della variante con spill forzato. La soglia adattiva e'
/// meta' di questo valore, quindi la migrazione scatta dopo pochi batch.
const TIGHT_MEMORY_BYTES: u64 = 8 * 1024 * 1024;

const SPILL_BYTES: u64 = 8 * 1024 * 1024 * 1024;

fn fixture_dir() -> PathBuf {
    PathBuf::from(std::env::var("PLENORA_BENCH_FIXDIR").unwrap_or_else(|_| "bench-fix".to_owned()))
}

fn write_csv_fixture(path: &Path, total: usize) {
    let file = std::fs::File::create(path).unwrap();
    let mut writer = BufWriter::new(file);
    writer.write_all(b"id,name,val,geometry\n").unwrap();
    for index in 0..total {
        #[allow(clippy::cast_precision_loss)]
        let x = (index % 3_600) as f64 / 10.0 - 180.0;
        #[allow(clippy::cast_precision_loss)]
        let y = (index % 1_700) as f64 / 10.0 - 85.0;
        #[allow(clippy::cast_precision_loss)]
        let val = index as f64 * 1.5;
        writeln!(writer, "{index},f{},{val},\"POINT ({x} {y})\"", index % 128).unwrap();
    }
    writer.flush().unwrap();
}

/// I due rami della conversione, dallo stesso context.
///
/// Fino a S4.d il benchmark costruiva due budget scollegati. Ora escono dalle
/// stesse parti: e' cio' che il percorso reale fa, e misurare una forma
/// diversa da quella spedita renderebbe il numero inutile.
fn pipeline_di_conversione(memory_bytes: u64) -> (ReadOptions, WriteOptions, PipelineContext) {
    let limiti = PipelineLimits::default()
        .with_memory_bytes(memory_bytes)
        .with_max_wkb_cell_bytes(1024 * 1024)
        .with_spill_bytes(SPILL_BYTES)
        .with_duration_ms(3_600_000);
    let bundle = PipelineBudget::builder().limits(limiti).build().unwrap();
    let contesto = bundle.context().clone();
    let (read, write) = bundle.into_convert_parts().into_parts();
    (
        ReadOptions::from_read_parts(read)
            .with_assume_crs("EPSG:4326")
            .with_format_option("wkt_column", "geometry"),
        WriteOptions::from_write_parts(write),
        contesto,
    )
}

/// Esito di una corsa.
///
/// `spill_peak_reserved_bytes` e' la quota **prenotata** al picco, non i byte
/// fisici del file: la prenotazione avviene a blocchi, quindi e' un limite
/// superiore all'occupazione reale del volume. I byte fisici li conosce solo
/// lo spool, e non sono osservabili da qui — il file non ha nome e il reader
/// non li espone. Il rapporto fra le due grandezze e' verificato dai test
/// dello spool, che asseriscono `scritti <= prenotati`.
struct Corsa {
    rows: usize,
    batches: usize,
    spill_peak_reserved_bytes: u64,
}

/// Esegue un `convert` da CSV a `GeoParquet` con **budget separati** per
/// lettura e scrittura, come fa `cmd_convert` della CLI.
///
/// Condividere un solo budget farebbe contare due volte la stessa riga sui
/// contatori cumulativi, e misurerebbe quindi un percorso che la CLI non
/// esegue.
///
/// Il picco di spill si campiona **durante** il drain, non alla fine: la
/// quota e' una prenotazione RAII, quindi torna al budget quando lo spool
/// rilascia il file — a fine rilettura, ancora prima del drop. Una misura a
/// posteriori vedrebbe zero.
fn convert(
    source: &Path,
    destination: &Path,
    ropts: ReadOptions,
    wopts: &WriteOptions,
    contesto: &PipelineContext,
) -> Corsa {
    std::fs::remove_file(destination).ok();
    let reader_driver = driver_csv::CsvDriver;
    let writer_driver = driver_geoparquet::GeoParquetDriver;

    let dataset = reader_driver
        .open(Source::Path(source.to_owned()), ropts)
        .unwrap();
    let contract = dataset.layers()[0].contract.clone();
    let mut reader = dataset
        .open_layer_reader(&ReadRequest {
            layer: LayerId(0),
            projected_fields: None,
            projection_mode: ProjectionMode::BestEffort,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            batch_target: BatchTarget::default(),
            scope: ReadScope::Complete,
            cancellation: CancellationToken::default(),
        })
        .unwrap();

    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "bench".to_owned(),
            contract,
        }],
    };
    let mut writer = writer_driver
        .create(Sink::Path(destination.to_owned()), &plan, wopts)
        .unwrap();

    let spill_iniziale = contesto.remaining_spill();
    let mut spill_minimo = spill_iniziale;
    let mut rows = 0;
    let mut batches = 0;
    while let Some(batch) = reader.next_batch().unwrap() {
        spill_minimo = spill_minimo.min(contesto.remaining_spill());
        rows += batch.num_rows();
        batches += 1;
        writer.write(&batch).unwrap();
    }
    writer.finish().unwrap();
    Corsa {
        rows,
        batches,
        spill_peak_reserved_bytes: spill_iniziale - spill_minimo,
    }
}

fn argomento(nome: &str) -> Option<String> {
    let argomenti: Vec<String> = std::env::args().collect();
    argomenti
        .iter()
        .position(|corrente| corrente == nome)
        .and_then(|indice| argomenti.get(indice + 1))
        .cloned()
}

fn main() {
    let rows: usize = argomento("--rows")
        .and_then(|valore| valore.parse().ok())
        .unwrap_or(200_000);
    let variante = argomento("--variant").unwrap_or_else(|| "no-spill".to_owned());
    let memory_bytes = match variante.as_str() {
        "no-spill" => ROOMY_MEMORY_BYTES,
        "forced-spill" => TIGHT_MEMORY_BYTES,
        altro => {
            eprintln!("variante sconosciuta: {altro} (attese: no-spill, forced-spill)");
            std::process::exit(2);
        }
    };

    let directory = fixture_dir();
    std::fs::create_dir_all(&directory).ok();
    let sorgente = directory.join(format!("spool-ab-{rows}.csv"));
    if !sorgente.exists() {
        write_csv_fixture(&sorgente, rows);
    }
    let destinazione = directory.join(format!("spool-ab-{variante}.parquet"));

    // Budget separati per lettura e scrittura, come `cmd_convert`.
    let (ropts, wopts, contesto) = pipeline_di_conversione(memory_bytes);

    let inizio = Instant::now();
    let corsa = convert(&sorgente, &destinazione, ropts, &wopts, &contesto);
    let durata = inizio.elapsed();

    let ha_spillato = corsa.spill_peak_reserved_bytes > 0;
    assert_eq!(
        contesto.remaining_spill(),
        SPILL_BYTES,
        "la quota di spill deve tornare interamente a fine rilettura"
    );

    println!(
        "{}",
        serde_json::json!({
            "variant": variante,
            "rows": corsa.rows,
            "batches": corsa.batches,
            "memory_bytes": memory_bytes,
            "wall_ms": durata.as_millis(),
            "spilled": ha_spillato,
            "spill_peak_reserved_bytes": corsa.spill_peak_reserved_bytes,
        })
    );

    // Un `forced-spill` che non spilla misurerebbe lo stesso percorso del
    // `no-spill`: il confronto sarebbe verde per costruzione e non direbbe
    // nulla sul costo dello spool.
    if variante == "forced-spill" && !ha_spillato {
        eprintln!("forced-spill non ha attivato lo spool: risultato invalido");
        std::process::exit(1);
    }
    if variante == "no-spill" && ha_spillato {
        eprintln!("no-spill ha attivato lo spool: quota di memoria insufficiente");
        std::process::exit(1);
    }
    std::fs::remove_file(&destinazione).ok();
}
