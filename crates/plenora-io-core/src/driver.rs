//! Il confine plug-in: `FormatDriver` + handle/reader/writer (ADR-IO 1).

use std::collections::BTreeMap;
use std::path::PathBuf;

use arrow_array::RecordBatch;
use plenora_core::contract::{LayerContract, LayerId};
use plenora_core::{PlenoraError, Result};

use crate::descriptor::FormatDescriptor;
use crate::loss::LossReport;
use crate::request::{ReadRequest, WritePlan};

/// Sorgente di lettura (scheletro Fase 0).
pub enum Source {
    Path(PathBuf),
}

/// Destinazione di scrittura (scheletro Fase 0).
pub enum Sink {
    /// File singolo o directory-dataset (multi-file), risolto dal driver.
    Path(PathBuf),
}

#[derive(Default)]
pub struct ReadOptions {
    /// CRS dichiarato per i formati che non lo portano (CSV/XLSX) — ADR-IO 4.
    pub assume_crs: Option<String>,
    /// Knob specifici del driver (es. csv: x_column/y_column/wkt_column/delimiter).
    pub format_options: BTreeMap<String, String>,
}

#[derive(Default)]
pub struct WriteOptions {
    /// Profilo `DurableAtomicPublish` (fsync) invece di `AtomicPublish` — ADR-IO 2.
    pub durable: bool,
    /// Knob specifici del driver.
    pub format_options: BTreeMap<String, String>,
}

pub trait FormatDriver: Send + Sync {
    fn descriptor(&self) -> &FormatDescriptor;
    /// Statico: header/schema/CRS, nessuna riga.
    fn open(&self, source: Source, opts: &ReadOptions) -> Result<Box<dyn OpenDatasetHandle>>;
    /// Statico: verifica che il contratto sia rappresentabile (ADR-IO 3).
    fn create(
        &self,
        sink: Sink,
        plan: &WritePlan,
        opts: &WriteOptions,
    ) -> Result<Box<dyn FormatWriter>>;
}

pub trait OpenDatasetHandle {
    fn layers(&self) -> &[LayerContract];
    /// Apre un reader indipendente per un layer; lo STATO mutabile vive nel
    /// reader (ADR-IO 1).
    fn open_layer_reader(&self, request: &ReadRequest) -> Result<Box<dyn LayerReader>>;
}

pub trait LayerReader {
    /// Schema effettivo del reader, autoritativo: il consumatore non lo inferisce
    /// (ADR-IO 6). Riflette la projection realmente applicata.
    fn contract(&self) -> &LayerContract;
    /// Pull-based con stato; `None` = fine dello stream.
    fn next_batch(&mut self) -> Result<Option<RecordBatch>>;
    /// Report di perdita (vuoto per i driver Lossless) — ADR-IO 5.
    fn loss_report(&self) -> LossReport {
        LossReport::default()
    }
}

pub trait FormatWriter {
    /// Scrive un batch nel layer primario (`LayerId(0)`).
    fn write(&mut self, batch: &RecordBatch) -> Result<()>;
    /// Scrive un batch in uno specifico layer (multi-layer). Default: accetta solo
    /// `LayerId(0)` e delega a `write`; i driver multi-layer fanno override (ADR-IO 1:
    /// un dataset-writer coordina tutti i layer con un unico commit atomico).
    fn write_to_layer(&mut self, layer: LayerId, batch: &RecordBatch) -> Result<()> {
        if layer.0 != 0 {
            return Err(PlenoraError::Unsupported(
                "questo formato non supporta la scrittura multi-layer".to_owned(),
            ));
        }
        self.write(batch)
    }
    /// Publish atomico dell'intero dataset, solo a successo (D11, ADR-IO 2).
    fn finish(self: Box<Self>) -> Result<Published>;
}

pub struct Published {
    pub bytes: u64,
    pub loss: LossReport,
    /// Esito di durabilità del publish (ADR-IO 2).
    pub outcome: crate::publish::PublishOutcome,
}
