//! Lo schema del footer: profondita' e costo dei percorsi, contro il tetto (#38).
//!
//! `parquet` converte lo schema del footer ricorsivamente, senza un tetto di
//! profondita', e `SchemaDescriptor::new` costruisce per ogni colonna foglia il
//! percorso intero dalla radice: profondita' per foglie, quadratico nei byte
//! del footer. Il fork (`vendor/parquet`) rifiuta uno schema troppo profondo
//! prima di convertirlo e addebita ogni prenotazione del decoder del footer al
//! tetto **prima** di farla; il driver gli passa il tetto delle pagine e
//! rifiuta un footer piu' lungo di quanto il tetto possa decodificare.
//!
//! Le prove costruiscono file **validi**: la profondita' e il costo vengono
//! dallo schema, non da byte alterati, quindi un decoder senza limiti li
//! leggerebbe -- a un costo che cresce senza tetto.

use std::sync::Arc;

use arrow_array::{ArrayRef, BinaryArray, Int32Array, RecordBatch, StructArray};
use arrow_schema::{DataType, Field, Fields, Schema};
use parquet::arrow::arrow_reader::{ArrowReaderOptions, ParquetRecordBatchReaderBuilder};
use parquet::arrow::ArrowWriter;
use parquet::file::metadata::KeyValue;
use parquet::file::properties::WriterProperties;
use plenora_io_core::FormatDriver as _;

const GEO: &str = r#"{"version":"1.1.0","primary_column":"geometry","columns":{"geometry":{"encoding":"WKB","geometry_types":["Point"]}}}"#;

/// Un `GeoParquet` con una geometria e la colonna `extra`, scritto con
/// `righe` righe (zero: il footer porta lo schema e nessun row group).
fn geoparquet(extra: Field, valori: ArrayRef, righe: usize) -> Vec<u8> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("geometry", DataType::Binary, false),
        extra,
    ]));
    let punto: Vec<u8> = [1_u8, 1, 0, 0, 0]
        .into_iter()
        .chain(0.5_f64.to_le_bytes())
        .chain(0.5_f64.to_le_bytes())
        .collect();
    let geometrie: ArrayRef = Arc::new(BinaryArray::from_iter_values(std::iter::repeat_n(
        punto.as_slice(),
        valori.len(),
    )));
    let batch = RecordBatch::try_new(Arc::clone(&schema), vec![geometrie, valori]).expect("batch");
    let proprieta = WriterProperties::builder()
        .set_key_value_metadata(Some(vec![KeyValue::new("geo".to_owned(), GEO.to_owned())]))
        .build();
    let mut byte = Vec::new();
    let mut scrittore = ArrowWriter::try_new(&mut byte, schema, Some(proprieta)).expect("writer");
    if righe > 0 {
        scrittore.write(&batch).expect("scrittura");
    }
    scrittore.close().expect("chiusura");
    byte
}

/// Una colonna di strutture annidate `livelli` volte attorno a un intero.
fn annidata(livelli: usize) -> (Field, ArrayRef) {
    let mut campo = Field::new("x", DataType::Int32, false);
    let mut valori: ArrayRef = Arc::new(Int32Array::from(vec![1]));
    for _ in 0..livelli {
        let campi = Fields::from(vec![campo]);
        valori = Arc::new(StructArray::new(campi.clone(), vec![valori], None));
        campo = Field::new("s", DataType::Struct(campi), false);
    }
    (campo, valori)
}

/// Una struttura dal nome lungo `nome` byte con `foglie` interi dentro: il
/// nome si ripete nel percorso di ogni foglia.
fn larga(nome: usize, foglie: usize) -> (Field, ArrayRef) {
    let campi: Fields = (0..foglie)
        .map(|i| Field::new(format!("f{i}"), DataType::Int32, false))
        .collect();
    let colonne: Vec<ArrayRef> = (0..foglie)
        .map(|_| Arc::new(Int32Array::from(vec![1])) as ArrayRef)
        .collect();
    let valori: ArrayRef = Arc::new(StructArray::new(campi.clone(), colonne, None));
    (
        Field::new("n".repeat(nome), DataType::Struct(campi), false),
        valori,
    )
}

fn leggi(byte: &[u8], opzioni: ArrowReaderOptions) -> Result<usize, String> {
    let temporanea = tempfile::NamedTempFile::new().expect("file temporaneo");
    std::fs::write(temporanea.path(), byte).expect("il file si scrive");
    let file = std::fs::File::open(temporanea.path()).expect("il file si apre");
    let lettore = ParquetRecordBatchReaderBuilder::try_new_with_options(file, opzioni)
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())?;
    lettore
        .map(|batch| batch.map(|b| b.num_rows()).map_err(|e| e.to_string()))
        .sum()
}

fn apri_col_driver(byte: &[u8], memoria: u64) -> Result<(), plenora_io_model::PlenoraIoError> {
    let temporanea = tempfile::NamedTempFile::new().expect("file temporaneo");
    std::fs::write(temporanea.path(), byte).expect("il file si scrive");
    let limiti = plenora_io_model::budget::PipelineLimits::default()
        .with_memory_bytes(memoria)
        .with_max_wkb_cell_bytes(1024);
    let opzioni = match plenora_io_model::budget::PipelineBudget::builder()
        .limits(limiti)
        .build()
    {
        Ok(bundle) => plenora_io_core::ReadOptions::from_read_parts(bundle.into_read_parts()),
        Err(errore) => panic!("bundle non costruibile: {errore:?}"),
    };
    driver_geoparquet::GeoParquetDriver
        .open(
            plenora_io_core::Source::Path(temporanea.path().to_path_buf()),
            opzioni,
        )
        .map(|_| ())
}

/// La controprova: uno schema annidato come quelli reali si legge.
#[test]
fn una_profondita_reale_si_legge() {
    let (campo, valori) = annidata(8);
    let byte = geoparquet(campo, valori, 1);
    assert_eq!(leggi(&byte, ArrowReaderOptions::new()), Ok(1));
    assert!(apri_col_driver(&byte, 256 * 1024 * 1024).is_ok());
}

/// Cento livelli: oltre il limite, un errore prima della conversione ricorsiva.
#[test]
fn uno_schema_troppo_profondo_e_un_errore() {
    // La scrittura di uno schema cosi' profondo e' ricorsiva anche nel writer
    // (e la costruzione e il rilascio degli array annidati): si fa su un thread
    // con uno stack largo, perche' e' la preparazione della prova e non cio'
    // che la prova misura. La lettura resta sul thread della prova.
    let byte = std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(|| {
            let (campo, valori) = annidata(100);
            geoparquet(campo, valori, 1)
        })
        .expect("thread di preparazione")
        .join()
        .expect("la preparazione non panica");
    let esito = leggi(&byte, ArrowReaderOptions::new());
    assert!(
        esito
            .as_ref()
            .is_err_and(|e| e.contains("deeper than 64 levels")),
        "il rifiuto atteso e' il limite di profondita' del fork: {esito:?}"
    );
    let errore = apri_col_driver(&byte, 256 * 1024 * 1024).expect_err("il driver rifiuta");
    assert_eq!(errore.phase, plenora_io_model::ErrorPhase::Read, "{errore}");
    assert_eq!(
        errore.category,
        plenora_io_model::ErrorCategory::DataMapping,
        "{errore}"
    );
}

/// Il costo dei percorsi delle foglie, contro il tetto.
///
/// Un nome di 32 KiB su 256 foglie: ogni percorso lo ripete, e il costo
/// supera gli 8 MiB con un footer di poche decine di KiB. Con un tetto di
/// 4 MiB il decoder rifiuta; con 64 MiB legge.
#[test]
fn il_costo_dei_percorsi_oltre_il_tetto_e_un_errore() {
    let (campo, valori) = larga(32 * 1024, 256);
    let byte = geoparquet(campo, valori, 0);
    let stretto = leggi(
        &byte,
        ArrowReaderOptions::new().with_footer_memory_budget(4 << 20),
    );
    assert!(
        stretto
            .as_ref()
            .is_err_and(|e| e.contains("exceeds the decoding memory budget")),
        "il rifiuto atteso e' il tetto di memoria del footer: {stretto:?}"
    );
    let largo = leggi(
        &byte,
        ArrowReaderOptions::new().with_footer_memory_budget(64 << 20),
    );
    assert_eq!(largo, Ok(0), "con un tetto sufficiente il file si legge");
}

/// Dal driver: lo stesso file con poca memoria e' un errore tipizzato, con
/// abbastanza memoria si apre. Il tetto e' meta' della capacita' effettiva.
#[test]
fn il_driver_passa_il_tetto_al_decoder() {
    let (campo, valori) = larga(32 * 1024, 256);
    let byte = geoparquet(campo, valori, 0);
    let errore = apri_col_driver(&byte, 8 << 20).expect_err("con 8 MiB il driver rifiuta");
    assert_eq!(errore.phase, plenora_io_model::ErrorPhase::Read, "{errore}");
    assert!(
        apri_col_driver(&byte, 256 << 20).is_ok(),
        "con 256 MiB si apre"
    );
}

/// Un footer piu' lungo di meta' del tetto si rifiuta prima di leggerlo.
#[test]
fn un_footer_oltre_meta_del_tetto_si_rifiuta_dalla_coda() {
    let (campo, valori) = larga(32 * 1024, 4);
    let byte = geoparquet(campo, valori, 0);
    let n = byte.len();
    let lunghezza = u32::from_le_bytes([byte[n - 8], byte[n - 7], byte[n - 6], byte[n - 5]]);
    // Una memoria il cui tetto (la meta') sta sotto il doppio del footer.
    let memoria = u64::from(lunghezza) * 2;
    let errore = apri_col_driver(&byte, memoria).expect_err("il footer non sta nel tetto");
    assert!(
        errore
            .to_string()
            .contains("footer Parquet che dichiara piu' metadati della memoria disponibile"),
        "il rifiuto atteso e' quello della coda: {errore}"
    );
}
