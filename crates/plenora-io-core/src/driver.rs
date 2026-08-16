//! Il confine plug-in: `FormatDriver` + handle/reader/writer (ADR-IO 1).

use std::collections::BTreeMap;
use std::path::PathBuf;

use arrow_array::{Array, BinaryArray, LargeBinaryArray, RecordBatch};
use arrow_schema::{DataType, SchemaRef};
use plenora_io_model::contract::{
    CoordinateDimensions, FieldId, GeometryColumnContract, GeometryEncoding, GeometryType,
    LayerContract, LayerId,
};
use plenora_io_model::crs::CrsResolution;
use plenora_io_model::geometry::{is_geometry_field, read_geometry_contract_metadata};
use plenora_io_model::limits::Limits;
use plenora_io_model::resource::{ResourceBudget, ResourceKind, ResourceLease};
use plenora_io_model::wkb::{inspect_wkb, WkbInspection};
use plenora_io_model::{
    CancellationReason, CancellationToken, CapabilityReason, ErrorCategory, ErrorPhase,
    KnownOrUnknownCount, PlenoraIoError, RemoteEffect, Result, RetryDisposition,
    RowDiagnosticColumn, RowDiagnosticExample, RowDiagnosticScope, RowDiagnosticWriteOutcome,
    RowDiagnosticWriteState, RowDiagnostics, RowDiagnosticsCompleteness,
    WriteDiagnosticStateCounts, ROW_DIAGNOSTICS_CONTRACT, ROW_DIAGNOSTICS_INDEX_BASIS,
    ROW_DIAGNOSTIC_COLUMN_UNATTESTABLE,
};

use crate::descriptor::{
    ArrowTypeClass, AttributeWriteSupport, CrsRepresentationState, FormatDescriptor,
    GeometryWriteSupport, NullabilitySupport, TypeCoercionPolicy,
};
use crate::loss::{FidelityAssessment, FidelityReasonCode, LossExample, LossReport};
#[cfg(test)]
use crate::request::BatchTarget;
use crate::request::{incremental_batch_memory_size, ReadRequest, WritePlan};

mod batch_worker;
mod reader_adapters;
pub mod spool;
pub use batch_worker::{spawn_batch_reader, BatchEmitter};
pub use reader_adapters::{
    with_batch_target, with_cancellation, with_read_budget, SingleReaderGate,
};

/// Sorgente di lettura (scheletro Fase 0).
pub enum Source {
    Path(PathBuf),
}

impl Source {
    /// Risolve la sorgente e applica il limite complessivo prima che un parser
    /// possa materializzarla. Le directory-dataset sono conteggiate senza
    /// seguire symlink.
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::Unsupported`] se la sorgente (o un suo
    /// discendente) è un symlink, [`PlenoraIoError::LimitExceeded`] se i byte
    /// totali superano `max_input_bytes` o vanno in overflow, l'errore di
    /// cancellazione se il token è attivo, e l'errore di I/O se la sorgente
    /// non è ispezionabile.
    pub fn into_path_checked(
        self,
        limits: &Limits,
        cancellation: &CancellationToken,
        resource_budget: &ResourceBudget,
    ) -> Result<PathBuf> {
        let Self::Path(path) = self;
        let mut total = 0_u64;
        // L0.9: il tetto sulle entry si applica **prima** della somma dei
        // byte e conta anche le directory. I byte crescono solo sui file,
        // quindi da soli non bounderebbero una sorgente fatta di sole
        // directory annidate. Il conteggio avviene al momento della
        // *scoperta*, non del prelievo: contando in coda al pop, una singola
        // directory con milioni di voci avrebbe gia' allocato milioni di
        // `PathBuf` prima che il tetto potesse intervenire. Cosi' invece
        // `pending` non supera mai `max_input_entries`.
        let mut visited = 0_u64;
        let note_entry = |visited: &mut u64| -> Result<()> {
            *visited = visited.checked_add(1).ok_or_else(|| {
                PlenoraIoError::LimitExceeded("overflow nel conteggio delle entry".to_owned())
            })?;
            if *visited > limits.max_input_entries {
                return Err(PlenoraIoError::LimitExceeded(format!(
                    "scan della sorgente oltre il limite di {} entry",
                    limits.max_input_entries
                )));
            }
            Ok(())
        };
        note_entry(&mut visited)?;
        let mut pending = vec![path.clone()];
        while let Some(candidate) = pending.pop() {
            check_cancelled(cancellation, ErrorPhase::Probe)?;
            resource_budget.ensure_active()?;
            let metadata = std::fs::symlink_metadata(&candidate)?;
            if metadata.file_type().is_symlink() {
                return Err(PlenoraIoError::Unsupported(
                    "symlink non ammesso nella sorgente".to_owned(),
                ));
            }
            if metadata.is_dir() {
                for entry in std::fs::read_dir(&candidate)? {
                    note_entry(&mut visited)?;
                    pending.push(entry?.path());
                }
            } else if metadata.is_file() {
                total = total.checked_add(metadata.len()).ok_or_else(|| {
                    PlenoraIoError::LimitExceeded("overflow nel conteggio dell'input".to_owned())
                })?;
                if total > limits.max_input_bytes {
                    return Err(PlenoraIoError::LimitExceeded(format!(
                        "input da {total} byte oltre il limite di {}",
                        limits.max_input_bytes
                    )));
                }
            }
        }
        resource_budget.observe_input_bytes(total)?;
        Ok(path)
    }
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
    /// Knob specifici del driver (es. csv: `x_column`/`y_column`/`wkt_column`/
    /// `delimiter`).
    pub format_options: BTreeMap<String, String>,
    /// Limiti condivisi del bordo I/O.
    pub limits: Limits,
    /// Budget condivisibile fra più componenti della stessa pipeline (R7.5).
    pub resource_budget: ResourceBudget,
    pub cancellation: CancellationToken,
}

#[derive(Default)]
pub struct WriteOptions {
    /// Profilo `DurableAtomicPublish` (fsync) invece di `AtomicPublish` — ADR-IO 2.
    pub durable: bool,
    /// Knob specifici del driver.
    pub format_options: BTreeMap<String, String>,
    /// Limiti condivisi del bordo I/O.
    pub limits: Limits,
    /// Deve essere lo stesso handle del reader per una conversione composta.
    pub resource_budget: ResourceBudget,
    pub cancellation: CancellationToken,
}

impl WriteOptions {
    /// Limite fisico effettivo, incluso il fattore massimo di espansione R7.7.
    #[must_use]
    pub fn max_output_bytes(&self) -> u64 {
        self.limits
            .max_output_bytes
            .min(self.resource_budget.output_limit())
    }
}

/// Traduce lo stato del token di cancellazione in un errore tipizzato.
///
/// # Errors
///
/// Restituisce l'errore di cancellazione (categoria `Timeout` per la deadline,
/// `Cancelled` per una richiesta esplicita o propagata dal parent) quando il
/// token è già stato attivato.
pub fn check_cancelled(token: &CancellationToken, phase: ErrorPhase) -> Result<()> {
    match token.reason() {
        None => Ok(()),
        Some(CancellationReason::Deadline) => Err(PlenoraIoError::cancelled(phase, true)),
        Some(CancellationReason::Requested | CancellationReason::Parent) => {
            Err(PlenoraIoError::cancelled(phase, false))
        }
    }
}

/// Frequenza comune dei controlli cooperativi nei loop che materializzano.
/// È una potenza di due per mantenere trascurabile il costo del fast path.
pub const CANCELLATION_CHECK_INTERVAL: usize = 1024;

// La saturazione e' esplicita nel ramo precedente: quando si arriva al cast il
// valore e' gia' provato entro `usize::MAX`, quindi non puo' troncare.
// `usize::try_from` non e' utilizzabile in un `const fn`.
#[allow(clippy::cast_possible_truncation)]
const fn saturating_usize(value: u64) -> usize {
    if value > usize::MAX as u64 {
        usize::MAX
    } else {
        value as usize
    }
}

// Simmetrico a `saturating_usize`: il cast e' raggiunto solo con un valore gia'
// provato entro `u64::MAX`.
#[allow(clippy::cast_possible_truncation)]
const fn saturating_u64(value: usize) -> u64 {
    if usize::BITS > u64::BITS && value > u64::MAX as usize {
        u64::MAX
    } else {
        value as u64
    }
}

/// Controlla periodicamente il token senza imporre una lettura atomica per
/// ogni riga. Il chiamante deve passare un indice monotono a partire da zero.
///
/// # Errors
///
/// Gli stessi di [`check_cancelled`], valutati solo agli indici multipli di
/// [`CANCELLATION_CHECK_INTERVAL`].
pub fn check_cancelled_periodically(
    token: &CancellationToken,
    phase: ErrorPhase,
    index: usize,
) -> Result<()> {
    if index & (CANCELLATION_CHECK_INTERVAL - 1) == 0 {
        check_cancelled(token, phase)?;
    }
    Ok(())
}

/// Esegue una lettura Arrow convertendo in errore un panico della libreria.
///
/// `arrow-ipc` va in panico dentro `convert::fb_to_schema` su schemi che il
/// decoder `FlatBuffer` accetta: `fields` e' opzionale e viene scartato con
/// `unwrap()` (convert.rs:198), e la conversione dei tipi ha una ventina fra
/// `panic!` e `unimplemented!` sui valori di enum che non riconosce. Ogni
/// reader chiama quella funzione, e le API che la avvolgono si chiamano
/// `try_*` ma sono fallibili solo sul parsing esterno: appena ottengono lo
/// schema fanno `.map(fb_to_schema)`.
///
/// Non esiste quindi un percorso per leggere Arrow IPC — o un Parquet il cui
/// footer porti la chiave `ARROW:schema` — che non possa abortire il processo
/// su un file ostile. Questo componente legge file esterni non fidati per
/// mestiere e promette una busta d'errore a quattro assi: la barriera
/// ripristina il contratto, non lo aggira.
///
/// Segnalato a monte: apache/arrow-rs#10575. Va rimossa quando quella issue e'
/// chiusa e il pin di arrow sale a una versione che rende fallibile la
/// conversione dello schema.
///
/// # Correttezza dell'unwind safety
///
/// L'operazione costruisce stato locale che viene interamente scartato se il
/// panico avviene, perche' il chiamante riceve `Err` e non vede nulla di
/// parzialmente costruito. Nessun invariante osservabile puo' restare rotto.
///
/// # Nota per chi legge un fuzz target rosso
///
/// I target che esercitano questo percorso restano rossi **anche a barriera
/// funzionante**: `libfuzzer-sys` installa un hook che chiama
/// `std::process::abort()` prima che l'unwinding cominci (0.4.10,
/// `src/lib.rs:92-95`), apposta perche' un `catch_unwind` nel codice sotto
/// test non possa nascondere difetti al fuzzer. La copertura di questa
/// barriera sono i test unitari dei driver, non il fuzzing.
///
/// # Errors
///
/// Propaga l'errore dell'operazione, oppure `PlenoraIoError::format` con il
/// messaggio del panico se la libreria e' abortita.
pub fn leggendo_arrow<T>(
    driver: &'static str,
    operazione: impl FnOnce() -> Result<T>,
) -> Result<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(operazione)) {
        Ok(risultato) => risultato,
        Err(panico) => Err(PlenoraIoError::format(
            driver,
            format!(
                "arrow in panico durante la decodifica (impronta {})",
                impronta_del_panico(&messaggio_del_panico(&panico))
            ),
        )),
    }
}

/// Impronta stabile e non invertibile del messaggio di un panico.
///
/// `PlenoraIoError::message` dichiara di non contenere «payload, definizioni
/// CRS, percorsi assoluti o valori di cella». Il messaggio di un panico non
/// rispetta quella promessa: arriva da una libreria di terze parti, e' derivato
/// dal file in lettura e non ha un formato su cui poter fare affidamento. Il
/// panico di `arrow-buffer` che questa barriera cattura, per esempio, riporta
/// `slice offset=… length=… selflen=…`, cioe' misure prese dal file; un'altra
/// libreria potrebbe metterci un percorso o il contenuto di una cella.
///
/// L'impronta conserva quello che serve senza portare il contenuto: due
/// esecuzioni sullo stesso difetto danno lo stesso valore, quindi si possono
/// correlare le occorrenze e associarle a un reproducer, ma dall'impronta non
/// si risale al testo.
///
/// FNV-1a a 64 bit scritto qui invece di `DefaultHasher`: quest'ultimo non
/// garantisce lo stesso risultato fra versioni di Rust, e un'impronta che
/// cambia da sola non correla piu' niente.
///
/// Nota di perimetro: questo redige l'errore strutturato, cioe' quello che
/// viene serializzato, registrato e passato agli altri componenti. L'hook di
/// panico globale resta quello del processo e continua a scrivere il testo
/// completo su stderr; sostituirlo sarebbe scorretto per una libreria, che non
/// possiede quella risorsa.
fn impronta_del_panico(messaggio: &str) -> String {
    let mut stato: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in messaggio.as_bytes() {
        stato ^= u64::from(*byte);
        stato = stato.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{stato:016x}")
}

/// Estrae il messaggio da un payload di panico senza assumerne il tipo:
/// `panic!` con formato produce `String`, quello letterale `&'static str`.
fn messaggio_del_panico(panico: &Box<dyn std::any::Any + Send>) -> String {
    panico.downcast_ref::<&'static str>().map_or_else(
        || {
            panico
                .downcast_ref::<String>()
                .cloned()
                .unwrap_or_else(|| "panico senza messaggio".to_owned())
        },
        |testo| (*testo).to_owned(),
    )
}

pub trait FormatDriver: Send + Sync {
    fn descriptor(&self) -> &FormatDescriptor;
    /// Statico: header/schema/CRS, nessuna riga.
    ///
    /// # Errors
    ///
    /// Restituisce un errore se la sorgente non è accessibile, non è nel
    /// formato atteso o eccede i limiti dichiarati in `opts`.
    fn open(&self, source: Source, opts: &ReadOptions) -> Result<Box<dyn OpenDatasetHandle>>;
    /// Statico: verifica che il contratto sia rappresentabile (ADR-IO 3).
    ///
    /// # Errors
    ///
    /// Restituisce un errore se il piano non è rappresentabile dalle
    /// capability del formato o se la destinazione non è preparabile.
    fn create(
        &self,
        sink: Sink,
        plan: &WritePlan,
        opts: &WriteOptions,
    ) -> Result<Box<dyn FormatWriter>>;
}

pub trait OpenDatasetHandle: Send + Sync {
    fn layers(&self) -> &[LayerContract];
    /// Valutazione di fedeltà concreta per il dataset aperto (ADR-IO 5).
    fn fidelity_assessment(&self) -> FidelityAssessment;
    /// Apre un reader indipendente per un layer; lo STATO mutabile vive nel
    /// reader (ADR-IO 1).
    ///
    /// # Errors
    ///
    /// Restituisce un errore se il layer richiesto non esiste, se la
    /// projection non è soddisfacibile o se il driver non ammette un ulteriore
    /// reader concorrente.
    fn open_layer_reader(&self, request: &ReadRequest) -> Result<Box<dyn LayerReader>>;
}

pub trait LayerReader {
    /// Schema effettivo del reader, autoritativo: il consumatore non lo inferisce
    /// (ADR-IO 6). Riflette la projection realmente applicata.
    fn contract(&self) -> &LayerContract;
    /// Pull-based con stato; `None` = fine dello stream.
    ///
    /// # Errors
    ///
    /// Restituisce un errore se il flusso sorgente è malformato, se un limite
    /// viene superato o se l'operazione viene annullata.
    fn next_batch(&mut self) -> Result<Option<RecordBatch>>;
    /// Report di perdita (vuoto per i driver Lossless) — ADR-IO 5.
    fn loss_report(&self) -> LossReport {
        LossReport::default()
    }
}

pub trait FormatWriter {
    /// Valutazione preventiva prodotta da `create`; il `Published` finale la
    /// aggiorna con le perdite osservate durante la scrittura.
    fn fidelity_assessment(&self) -> FidelityAssessment {
        FidelityAssessment::unassessed(
            "writer non avvolto dal validatore comune: assessment non disponibile",
        )
    }
    /// Dichiara la cardinalità completa della sorgente per un layer prima del
    /// primo write di quel layer.
    /// Il wrapper comune usa il valore per evitare partizioni diagnostiche di
    /// prefisso. I writer raw possono ignorarlo: in tal caso non devono
    /// inventare una diagnostica write completa.
    ///
    /// # Errors
    ///
    /// Restituisce un errore se il totale viene dichiarato dopo il primo write
    /// del layer o se contraddice un totale già dichiarato.
    fn declare_input_total(&mut self, _layer: LayerId, _total: u64) -> Result<()> {
        Ok(())
    }
    /// Scrive un batch nel layer primario (`LayerId(0)`).
    ///
    /// # Errors
    ///
    /// Restituisce un errore se il batch non rispetta il contratto dichiarato,
    /// se un limite viene superato o se il backend rifiuta la scrittura.
    fn write(&mut self, batch: &RecordBatch) -> Result<()>;
    /// Scrive un batch in uno specifico layer (multi-layer). Default: accetta solo
    /// `LayerId(0)` e delega a `write`; i driver multi-layer fanno override (ADR-IO 1:
    /// un dataset-writer coordina tutti i layer con un unico commit atomico).
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::Unsupported`] se il formato non è
    /// multi-layer e `layer` non è `LayerId(0)`; per il resto gli stessi
    /// errori di [`FormatWriter::write`].
    fn write_to_layer(&mut self, layer: LayerId, batch: &RecordBatch) -> Result<()> {
        if layer.0 != 0 {
            return Err(PlenoraIoError::Unsupported(
                "questo formato non supporta la scrittura multi-layer".to_owned(),
            ));
        }
        self.write(batch)
    }
    /// Publish del dataset a successo. `Ok(Published)` implica che tutte le
    /// componenti dichiarate dal driver sono visibili nella destinazione;
    /// `Err` implica il tentativo di non lasciare nulla di visibile.
    ///
    /// La garanzia d'atomicita' del publish e' documentata per driver
    /// (ADR-IO 2). I formati che pubblicano un file singolo o un
    /// directory-rename (per esempio `parquet`, `ipc`, `gpkg`, e la
    /// modalita' `ShapefileDirectoryDataset` di `shp`) sono
    /// crash-atomic. I formati con set di file loose (per esempio
    /// `shp` in modalita' compatibile `*.shp` + companion) *non* lo
    /// sono per definizione: il publish rinomina piu' file
    /// sequenzialmente e in caso di errore intermedio prova un rollback
    /// best-effort dei companion gia' spostati.
    ///
    /// # Errors
    ///
    /// Restituisce un errore se la finalizzazione o il publish non
    /// riescono. Il campo `remote_effect` dell'errore distingue:
    /// - `None`: nessuna destinazione visibile (fail-closed prima del
    ///   primo rename, o rollback riuscito interamente per i set loose);
    /// - `Partial`: alcune destinazioni potrebbero restare visibili
    ///   (set loose con rollback fallito su almeno un companion). Il
    ///   chiamante deve verificare/pulire manualmente prima di
    ///   ripetere l'operazione;
    /// - `RolledBack`/`Committed`/`Unknown` sono riservati per
    ///   backend transazionali futuri.
    fn finish(self: Box<Self>) -> Result<Published>;
}

/// Applica i limiti indipendenti dal formato a qualunque writer. I vincoli
/// specifici (WKB, vertici, dimensione fisica del dataset) restano nel driver.
#[must_use]
pub fn with_write_limits(writer: Box<dyn FormatWriter>, limits: Limits) -> Box<dyn FormatWriter> {
    Box::new(LimitedWriter {
        inner: writer,
        driver: "writer",
        limits,
        rows: 0,
        layer_rows: vec![0],
        input_totals: vec![None],
        failed: false,
        contracts: Vec::new(),
        geometry_validation: None,
        planned_loss: LossReport::default(),
        cancellation: CancellationToken::new(),
        resource_budget: ResourceBudget::default(),
        _operation_lease: None,
        fidelity: FidelityAssessment::unassessed(
            "writer con soli limiti globali: assessment di formato non disponibile",
        ),
    })
}

/// Applica i limiti globali e verifica che i byte geometrici di ogni batch
/// rispettino sia il contratto dichiarato sia le capability del driver.
///
/// È una seconda guardia runtime: impedisce che un batch dichiarato XY contenga
/// in realtà WKB Z/M o EWKB e venga normalizzato silenziosamente dal driver.
fn geometry_contracts_for_validation(
    plan: &WritePlan,
) -> Result<Vec<Option<GeometryColumnContract>>> {
    plan.layers
        .iter()
        .map(|layer| -> Result<Option<GeometryColumnContract>> {
            if let Some(geometry) = &layer.contract.geometry {
                return Ok(Some(geometry.clone()));
            }
            let mut fields = layer
                .contract
                .schema
                .fields()
                .iter()
                .enumerate()
                .filter(|(_, field)| is_geometry_field(field));
            let Some((index, field)) = fields.next() else {
                return Ok(None);
            };
            if fields.next().is_some() {
                return Err(PlenoraIoError::Contract(format!(
                    "layer '{}': più colonne GeoArrow senza contratto geometrico esplicito",
                    layer.name
                )));
            }
            // Il costruttore stabilisce il default storico XY prima di leggere
            // i metadati legacy. Un valore esplicito, incluso `unknown`, lo
            // sostituisce e non viene mai degradato dopo il parsing (R3.4).
            // `index` e' la posizione di un campo nello schema Arrow: e'
            // limitato dal numero di campi del layer, ordini di grandezza
            // sotto 2^32. Un cast controllato introdurrebbe un ramo d'errore
            // irraggiungibile in un punto del contratto.
            #[allow(clippy::cast_possible_truncation)]
            let mut geometry = GeometryColumnContract::wkb_xy(
                FieldId(index as u32),
                field.name(),
                CrsResolution::Missing,
                field.is_nullable(),
            );
            read_geometry_contract_metadata(field, &mut geometry)?;
            Ok(Some(geometry))
        })
        .collect()
}

/// Avvolge il writer con la validazione comune di contratto, geometria e
/// limiti.
///
/// # Errors
///
/// Restituisce un errore se il piano dichiara più colonne geometriche senza
/// contratto esplicito o se i metadati geometrici del campo non sono
/// interpretabili.
pub fn with_write_validation(
    writer: Box<dyn FormatWriter>,
    descriptor: &FormatDescriptor,
    plan: &WritePlan,
    limits: Limits,
    cancellation: CancellationToken,
    resource_budget: ResourceBudget,
) -> Result<Box<dyn FormatWriter>> {
    let geometry_support = descriptor
        .write_capabilities
        .as_ref()
        .map(|capabilities| capabilities.geometry);
    let layers = geometry_contracts_for_validation(plan)?;
    let planned_loss = planned_write_loss(descriptor, plan);
    let fidelity = assess_write_contract(descriptor, plan).with_loss_report(&planned_loss);
    resource_budget.ensure_active()?;
    let operation_lease = resource_budget.try_lease(ResourceKind::ConcurrentOperations, 1)?;
    let columns = plan.layers.iter().try_fold(0_u64, |total, layer| {
        total
            .checked_add(
                u64::try_from(layer.contract.schema.fields().len()).map_err(|_| {
                    PlenoraIoError::LimitExceeded("troppe colonne nel piano".to_owned())
                })?,
            )
            .ok_or_else(|| {
                PlenoraIoError::LimitExceeded("overflow nel conteggio delle colonne".to_owned())
            })
    })?;
    if columns > 0 {
        resource_budget
            .try_lease(ResourceKind::Columns, columns)?
            .commit(columns)?;
    }
    Ok(Box::new(LimitedWriter {
        inner: writer,
        driver: descriptor.id,
        limits,
        rows: 0,
        layer_rows: vec![0; plan.layers.len()],
        input_totals: vec![None; plan.layers.len()],
        failed: false,
        contracts: plan
            .layers
            .iter()
            .map(|layer| layer.contract.schema.clone())
            .collect(),
        fidelity,
        planned_loss,
        cancellation,
        resource_budget,
        _operation_lease: Some(operation_lease),
        geometry_validation: geometry_support.map(|support| GeometryValidation {
            driver: descriptor.id,
            support,
            layers,
        }),
    }))
}

fn planned_write_loss(descriptor: &FormatDescriptor, plan: &WritePlan) -> LossReport {
    let mut loss = LossReport::default();
    let Some(capabilities) = descriptor.write_capabilities else {
        return loss;
    };

    for layer in &plan.layers {
        if let Some(geometry) = &layer.contract.geometry {
            let (crs_id, crs_definition) = match &geometry.crs {
                CrsResolution::Resolved(crs) => (crs.id.as_deref(), crs.definition.as_deref()),
                CrsResolution::DeclaredButUnresolved(raw) => {
                    (raw.authority_hint.as_deref(), raw.definition.as_deref())
                }
                CrsResolution::Missing => (None, None),
            };
            record_crs_representation_loss(
                &mut loss,
                &layer.name,
                &geometry.name,
                "crs_id",
                crs_id.map(str::len),
                capabilities.crs_representations.crs_id,
            );
            record_crs_representation_loss(
                &mut loss,
                &layer.name,
                &geometry.name,
                "srid",
                geometry.srid.map(|srid| srid.to_string().len()),
                capabilities.crs_representations.srid,
            );
            record_crs_representation_loss(
                &mut loss,
                &layer.name,
                &geometry.name,
                "crs_definition",
                crs_definition.map(str::len),
                capabilities.crs_representations.crs_definition,
            );
        }

        let geometry_name = layer
            .contract
            .geometry
            .as_ref()
            .map(|geometry| geometry.name.as_str());
        for field in layer.contract.schema.fields() {
            if geometry_name == Some(field.name().as_str()) || is_geometry_field(field) {
                continue;
            }
            let type_class = crate::capabilities::arrow_type_class(field.data_type());
            let unsupported_text_coercion = !capabilities.allowed_types.contains(&type_class)
                && matches!(
                    capabilities.type_coercion,
                    TypeCoercionPolicy::ExplicitText | TypeCoercionPolicy::LossReported
                );
            let kml_scalar_to_text = descriptor.id == "kml" && type_class != ArrowTypeClass::Utf8;
            let gpkg_type_normalization = descriptor.id == "gpkg"
                && !matches!(
                    field.data_type(),
                    DataType::Int64 | DataType::Float64 | DataType::Utf8 | DataType::Binary
                );
            if unsupported_text_coercion || kml_scalar_to_text || gpkg_type_normalization {
                loss.record("coercion tipo attributo", 1);
                loss.add_example(LossExample {
                    category: "coercion tipo attributo".to_owned(),
                    context: format!("layer={} field={}", layer.name, field.name()),
                });
            }
        }
    }
    loss
}

fn record_crs_representation_loss(
    loss: &mut LossReport,
    layer: &str,
    field: &str,
    representation: &str,
    value_bytes: Option<usize>,
    state: CrsRepresentationState,
) {
    let (Some(value_bytes), category_suffix) = (
        value_bytes,
        match state {
            CrsRepresentationState::Preserved => return,
            CrsRepresentationState::Absent => "absent",
            CrsRepresentationState::Derived => "derived",
        },
    ) else {
        return;
    };
    let category = format!("{representation}_not_preserved_{category_suffix}");
    loss.record(&category, 1);
    loss.add_example(LossExample {
        category,
        context: format!(
            "layer={layer} field={field} representation={representation} \
             state={category_suffix} value_bytes={value_bytes}"
        ),
    });
}

fn assess_write_contract(descriptor: &FormatDescriptor, plan: &WritePlan) -> FidelityAssessment {
    let mut assessment = FidelityAssessment::for_format(descriptor.id, descriptor.fidelity_class);
    let Some(capabilities) = descriptor.write_capabilities else {
        return assessment;
    };

    for layer in &plan.layers {
        let geometry_name = layer
            .contract
            .geometry
            .as_ref()
            .map(|geometry| geometry.name.as_str());
        for field in layer.contract.schema.fields() {
            let is_geometry = geometry_name == Some(field.name().as_str());
            if !is_geometry && capabilities.attributes == AttributeWriteSupport::LossReported {
                assessment.add_reason(
                    FidelityReasonCode::AttributeLoss,
                    format!(
                        "{}: attributo '{}' non nativo o loss-reported",
                        layer.name,
                        field.name()
                    ),
                );
            }
            if !capabilities
                .allowed_types
                .contains(&crate::capabilities::arrow_type_class(field.data_type()))
                && capabilities.type_coercion == TypeCoercionPolicy::LossReported
            {
                assessment.add_reason(
                    FidelityReasonCode::TypeCoercion,
                    format!(
                        "{}: tipo {:?} di '{}' richiede coercion",
                        layer.name,
                        field.data_type(),
                        field.name()
                    ),
                );
            }
            if field.is_nullable() && capabilities.nullability == NullabilitySupport::FormatDefined
            {
                assessment.add_reason(
                    FidelityReasonCode::NullabilityChanged,
                    format!(
                        "{}: nullability di '{}' definita dal formato",
                        layer.name,
                        field.name()
                    ),
                );
            }
        }

        if descriptor.id == "dxf"
            && layer.contract.geometry.as_ref().is_some_and(|geometry| {
                geometry.geometry_types.iter().any(|geometry_type| {
                    matches!(
                        geometry_type,
                        GeometryType::MultiPoint
                            | GeometryType::MultiLineString
                            | GeometryType::MultiPolygon
                            | GeometryType::GeometryCollection
                    )
                })
            })
        {
            assessment.add_reason(
                FidelityReasonCode::StructureChanged,
                format!("{}: geometrie multipart esplose in entità DXF", layer.name),
            );
        }
    }
    assessment
}

struct GeometryValidation {
    driver: &'static str,
    support: GeometryWriteSupport,
    layers: Vec<Option<GeometryColumnContract>>,
}

struct LimitedWriter {
    inner: Box<dyn FormatWriter>,
    driver: &'static str,
    limits: Limits,
    rows: usize,
    layer_rows: Vec<u64>,
    input_totals: Vec<Option<u64>>,
    failed: bool,
    contracts: Vec<SchemaRef>,
    geometry_validation: Option<GeometryValidation>,
    fidelity: FidelityAssessment,
    planned_loss: LossReport,
    cancellation: CancellationToken,
    resource_budget: ResourceBudget,
    _operation_lease: Option<ResourceLease>,
}

struct WriteBatchResources {
    rows: u64,
    bytes: u64,
    rows_lease: Option<ResourceLease>,
    output_lease: Option<ResourceLease>,
    memory_lease: Option<ResourceLease>,
    geometry_components: u64,
    geometry_lease: Option<ResourceLease>,
}

impl WriteBatchResources {
    fn commit(self) -> Result<()> {
        if let Some(rows_lease) = self.rows_lease {
            rows_lease.commit(self.rows)?;
        }
        if let Some(output_lease) = self.output_lease {
            output_lease.commit(self.bytes)?;
        }
        drop(self.memory_lease);
        if self.geometry_components > 0 {
            self.geometry_lease
                .ok_or_else(|| {
                    PlenoraIoError::LimitExceeded("budget geometrico esaurito".to_owned())
                })?
                .commit(self.geometry_components)?;
        }
        Ok(())
    }
}

impl LimitedWriter {
    // Sequenza lineare di contabilizzazioni e guardie, una per limite: la
    // lunghezza e' nel numero di limiti, non in complessita' logica.
    #[allow(clippy::too_many_lines)]
    fn account(&mut self, layer: usize, batch: &RecordBatch) -> Result<WriteBatchResources> {
        self.resource_budget.ensure_active()?;
        if let Some(contract) = self.contracts.get(layer) {
            if batch.schema().as_ref() != contract.as_ref() {
                return Err(PlenoraIoError::new(
                    ErrorCategory::Schema,
                    ErrorPhase::Validate,
                    RemoteEffect::None,
                    RetryDisposition::Never,
                    format!(
                        "batch del layer {layer} diverso dal contratto dichiarato (schema, ordine, tipi, nullability o metadata)"
                    ),
                ));
            }
        } else if !self.contracts.is_empty() {
            return Err(PlenoraIoError::capability(
                self.driver,
                None,
                CapabilityReason::MultipleLayers,
                format!("layer runtime {layer} fuori dal WritePlan"),
            ));
        }
        if batch.num_columns() > self.limits.max_columns {
            return Err(PlenoraIoError::LimitExceeded(format!(
                "batch con {} colonne oltre il limite di {}",
                batch.num_columns(),
                self.limits.max_columns
            )));
        }
        let batch_rows = u64::try_from(batch.num_rows()).map_err(|_| {
            PlenoraIoError::LimitExceeded("batch oltre il conteggio supportato".to_owned())
        })?;
        let layer_rows = *self.layer_rows.get(layer).ok_or_else(|| {
            PlenoraIoError::Contract(format!("layer runtime {layer} fuori dal WritePlan"))
        })?;
        if self
            .input_totals
            .get(layer)
            .copied()
            .flatten()
            .is_some_and(|total| {
                layer_rows
                    .checked_add(batch_rows)
                    .is_none_or(|rows| rows > total)
            })
        {
            return Err(PlenoraIoError::Contract(
                "write oltre input_total dichiarato".to_owned(),
            ));
        }
        self.rows = self.rows.checked_add(batch.num_rows()).ok_or_else(|| {
            PlenoraIoError::LimitExceeded("overflow nel conteggio delle righe".to_owned())
        })?;
        if self.rows > self.limits.max_rows {
            return Err(PlenoraIoError::LimitExceeded(format!(
                "{} righe oltre il limite di {}",
                self.rows, self.limits.max_rows
            )));
        }
        let geometry_components = if let Some(validation) = &self.geometry_validation {
            let mut effective_limits = self.limits;
            effective_limits.wkb.max_cell_bytes = effective_limits
                .wkb
                .max_cell_bytes
                .min(saturating_usize(self.resource_budget.limits().cell_bytes));
            effective_limits.wkb.max_components =
                effective_limits.wkb.max_components.min(saturating_usize(
                    self.resource_budget
                        .remaining(ResourceKind::GeometryComponents),
                ));
            effective_limits.wkb.max_depth = effective_limits.wkb.max_depth.min(saturating_usize(
                self.resource_budget.limits().nesting_depth,
            ));
            validate_geometry_batch_at(
                validation.driver,
                validation.support,
                validation
                    .layers
                    .get(layer)
                    .ok_or_else(|| {
                        PlenoraIoError::capability(
                            validation.driver,
                            None,
                            CapabilityReason::MultipleLayers,
                            format!("layer runtime {layer} fuori dal WritePlan"),
                        )
                    })?
                    .as_ref(),
                batch,
                &effective_limits,
                *self.layer_rows.get(layer).ok_or_else(|| {
                    PlenoraIoError::Contract(format!("layer runtime {layer} fuori dal WritePlan"))
                })?,
                self.input_totals.get(layer).copied().flatten(),
            )?
        } else {
            0
        };
        let rows = batch_rows;
        if rows == 0 {
            return Ok(WriteBatchResources {
                rows: 0,
                bytes: 0,
                rows_lease: None,
                output_lease: None,
                memory_lease: None,
                geometry_components: 0,
                geometry_lease: None,
            });
        }
        let bytes = u64::try_from(incremental_batch_memory_size(batch)).map_err(|_| {
            PlenoraIoError::LimitExceeded("batch oltre il conteggio byte supportato".to_owned())
        })?;
        Ok(WriteBatchResources {
            rows,
            bytes,
            rows_lease: Some(self.resource_budget.try_lease(ResourceKind::Rows, rows)?),
            output_lease: (bytes > 0)
                .then(|| {
                    self.resource_budget
                        .try_lease(ResourceKind::OutputBytes, bytes)
                })
                .transpose()?,
            memory_lease: (bytes > 0)
                .then(|| {
                    self.resource_budget
                        .try_lease(ResourceKind::MemoryBytes, bytes)
                })
                .transpose()?,
            geometry_components,
            geometry_lease: (geometry_components > 0)
                .then(|| {
                    self.resource_budget
                        .try_lease(ResourceKind::GeometryComponents, geometry_components)
                })
                .transpose()?,
        })
    }
}

impl FormatWriter for LimitedWriter {
    fn fidelity_assessment(&self) -> FidelityAssessment {
        self.fidelity.clone()
    }

    fn declare_input_total(&mut self, layer: LayerId, total: u64) -> Result<()> {
        let layer_index = layer.0 as usize;
        if self
            .layer_rows
            .get(layer_index)
            .is_some_and(|rows| *rows > 0)
        {
            return Err(PlenoraIoError::Contract(
                "input_total deve essere dichiarato prima del primo write del layer".to_owned(),
            ));
        }
        let slot = self.input_totals.get(layer_index).ok_or_else(|| {
            PlenoraIoError::Contract(format!("layer runtime {} fuori dal WritePlan", layer.0))
        })?;
        if slot.is_some_and(|declared| declared != total) {
            return Err(PlenoraIoError::Contract(
                "input_total dichiarato in modo incoerente".to_owned(),
            ));
        }
        self.inner.declare_input_total(layer, total)?;
        self.input_totals[layer_index] = Some(total);
        Ok(())
    }

    fn write(&mut self, batch: &RecordBatch) -> Result<()> {
        check_cancelled(&self.cancellation, ErrorPhase::Write)?;
        if self.failed {
            return Err(PlenoraIoError::format(
                self.driver,
                "writer invalidato da un precedente errore di scrittura",
            )
            .during(plenora_io_model::ErrorPhase::Write));
        }
        let result = self.account(0, batch).and_then(|resources| {
            let rows = resources.rows;
            self.inner.write(batch)?;
            resources.commit()?;
            self.layer_rows[0] = self.layer_rows[0].checked_add(rows).ok_or_else(|| {
                PlenoraIoError::LimitExceeded("overflow nel conteggio righe layer".to_owned())
            })?;
            Ok(())
        });
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn write_to_layer(&mut self, layer: LayerId, batch: &RecordBatch) -> Result<()> {
        check_cancelled(&self.cancellation, ErrorPhase::Write)?;
        if self.failed {
            return Err(PlenoraIoError::format(
                self.driver,
                "writer invalidato da un precedente errore di scrittura",
            )
            .during(plenora_io_model::ErrorPhase::Write));
        }
        let result = self.account(layer.0 as usize, batch).and_then(|resources| {
            let rows = resources.rows;
            self.inner.write_to_layer(layer, batch)?;
            resources.commit()?;
            let layer_rows = self.layer_rows.get_mut(layer.0 as usize).ok_or_else(|| {
                PlenoraIoError::Contract(format!("layer runtime {} fuori dal WritePlan", layer.0))
            })?;
            *layer_rows = layer_rows.checked_add(rows).ok_or_else(|| {
                PlenoraIoError::LimitExceeded("overflow nel conteggio righe layer".to_owned())
            })?;
            Ok(())
        });
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn finish(self: Box<Self>) -> Result<Published> {
        check_cancelled(&self.cancellation, ErrorPhase::Finalize)?;
        self.resource_budget.ensure_active()?;
        if self.failed {
            return Err(PlenoraIoError::format(
                self.driver,
                "finish vietato dopo un errore di scrittura",
            )
            .during(plenora_io_model::ErrorPhase::Finalize));
        }
        if self
            .input_totals
            .iter()
            .zip(&self.layer_rows)
            .any(|(declared, observed)| declared.is_some_and(|total| total != *observed))
        {
            return Err(PlenoraIoError::Contract(
                "EOF prima dell'input_total esatto dichiarato".to_owned(),
            ));
        }
        let mut published = self.inner.finish()?;
        published.loss.merge(&self.planned_loss);
        published.fidelity = self.fidelity.with_loss_report(&published.loss);
        Ok(published)
    }
}

fn geometry_violation(
    driver: &'static str,
    field: &str,
    reason: CapabilityReason,
    detail: impl Into<String>,
) -> PlenoraIoError {
    PlenoraIoError::capability(driver, Some(field.to_owned()), reason, detail)
}

fn validate_inspected_geometry(
    driver: &'static str,
    support: GeometryWriteSupport,
    contract: &GeometryColumnContract,
    geometry: &WkbInspection,
) -> Result<()> {
    let actual_dimensions = geometry.dimensions;
    if contract.dimensions != CoordinateDimensions::Unknown
        && contract.dimensions != actual_dimensions
    {
        return Err(geometry_violation(
            driver,
            &contract.name,
            CapabilityReason::CoordinateDimensions,
            format!(
                "payload {:?} diverso dalle dimensioni dichiarate {:?}",
                actual_dimensions, contract.dimensions
            ),
        ));
    }
    if !support.dimensions.contains(&actual_dimensions) {
        return Err(geometry_violation(
            driver,
            &contract.name,
            CapabilityReason::CoordinateDimensions,
            format!("payload {actual_dimensions:?} non supportato dal driver"),
        ));
    }

    let allow_srid = contract.encoding == GeometryEncoding::Ewkb;
    if !geometry.nested_dimensions_coherent || (!allow_srid && geometry.contains_srid) {
        return Err(geometry_violation(
            driver,
            &contract.name,
            if allow_srid {
                CapabilityReason::CoordinateDimensions
            } else {
                CapabilityReason::GeometryEncoding
            },
            "componenti WKB con dimensioni incoerenti o SRID EWKB non dichiarato",
        ));
    }
    if contract.encoding == GeometryEncoding::Ewkb && geometry.srid != contract.srid {
        return Err(geometry_violation(
            driver,
            &contract.name,
            CapabilityReason::GeometryEncoding,
            format!(
                "SRID del payload {:?} diverso da quello dichiarato {:?}",
                geometry.srid, contract.srid
            ),
        ));
    }
    if !contract.geometry_types.is_empty()
        && !contract.geometry_types.contains(&geometry.geometry_type)
    {
        return Err(geometry_violation(
            driver,
            &contract.name,
            CapabilityReason::MixedGeometry,
            format!(
                "tipo {:?} assente dai tipi geometrici dichiarati",
                geometry.geometry_type
            ),
        ));
    }
    Ok(())
}

fn validate_geometry_batch_at(
    driver: &'static str,
    support: GeometryWriteSupport,
    contract: Option<&GeometryColumnContract>,
    batch: &RecordBatch,
    limits: &Limits,
    row_offset: u64,
    input_total: Option<u64>,
) -> Result<u64> {
    let Some(contract) = contract else {
        let violations = nullability_violations(batch, row_offset)?;
        return if violations.is_empty() {
            Ok(0)
        } else {
            Err(write_rejection_error(
                driver,
                saturating_u64(batch.num_rows()),
                row_offset,
                &violations,
                input_total,
            ))
        };
    };
    let index = batch
        .schema()
        .fields()
        .iter()
        .position(|field| field.name() == &contract.name)
        .ok_or_else(|| {
            geometry_violation(
                driver,
                &contract.name,
                CapabilityReason::GeometryNotSupported,
                "colonna geometrica dichiarata assente dal batch",
            )
        })?;
    let array = batch.column(index);
    let wkb_limits = limits.effective_wkb();

    let mut violations = nullability_violations(batch, row_offset)?;
    let mut components = 0_u64;
    if let Some(values) = array.as_any().downcast_ref::<BinaryArray>() {
        for row in 0..values.len() {
            inspect_geometry_row(
                driver,
                support,
                contract,
                &wkb_limits,
                row,
                if values.is_null(row) {
                    None
                } else {
                    Some(values.value(row))
                },
                row_offset,
                &mut violations,
                &mut components,
            )?;
        }
    } else if let Some(values) = array.as_any().downcast_ref::<LargeBinaryArray>() {
        for row in 0..values.len() {
            inspect_geometry_row(
                driver,
                support,
                contract,
                &wkb_limits,
                row,
                if values.is_null(row) {
                    None
                } else {
                    Some(values.value(row))
                },
                row_offset,
                &mut violations,
                &mut components,
            )?;
        }
    } else {
        return Err(geometry_violation(
            driver,
            &contract.name,
            CapabilityReason::GeometryEncoding,
            "colonna geometrica runtime non Binary/LargeBinary",
        ));
    }
    if violations.is_empty() {
        Ok(components)
    } else {
        Err(write_rejection_error(
            driver,
            saturating_u64(batch.num_rows()),
            row_offset,
            &violations,
            input_total,
        ))
    }
}

#[derive(Clone)]
struct WriteRowViolation {
    source_index: u64,
    cause: &'static str,
    column: String,
    capability_reason: CapabilityReason,
}

fn nullability_violations(
    batch: &RecordBatch,
    row_offset: u64,
) -> Result<BTreeMap<u64, WriteRowViolation>> {
    let mut violations = BTreeMap::new();
    for (column_index, field) in batch.schema().fields().iter().enumerate() {
        if field.is_nullable() {
            continue;
        }
        let array = batch.column(column_index);
        for row in 0..batch.num_rows() {
            if array.is_null(row) {
                let source_index = row_offset
                    .checked_add(u64::try_from(row).map_err(|_| {
                        PlenoraIoError::LimitExceeded("indice riga oltre u64".to_owned())
                    })?)
                    .ok_or_else(|| {
                        PlenoraIoError::LimitExceeded("overflow nell'indice riga".to_owned())
                    })?;
                violations
                    .entry(source_index)
                    .or_insert_with(|| WriteRowViolation {
                        source_index,
                        cause: "contract.nullability",
                        column: field.name().to_owned(),
                        capability_reason: CapabilityReason::Nullability,
                    });
            }
        }
    }
    Ok(violations)
}

#[allow(clippy::too_many_arguments)]
fn inspect_geometry_row(
    driver: &'static str,
    support: GeometryWriteSupport,
    contract: &GeometryColumnContract,
    limits: &plenora_io_model::limits::WkbLimits,
    row: usize,
    bytes: Option<&[u8]>,
    row_offset: u64,
    violations: &mut BTreeMap<u64, WriteRowViolation>,
    components: &mut u64,
) -> Result<()> {
    let source_index = row_offset
        .checked_add(
            u64::try_from(row)
                .map_err(|_| PlenoraIoError::LimitExceeded("indice riga oltre u64".to_owned()))?,
        )
        .ok_or_else(|| PlenoraIoError::LimitExceeded("overflow nell'indice riga".to_owned()))?;
    if violations.contains_key(&source_index) {
        return Ok(());
    }
    let Some(bytes) = bytes else {
        if !contract.nullable {
            violations.insert(
                source_index,
                WriteRowViolation {
                    source_index,
                    cause: "contract.nullability",
                    column: contract.name.clone(),
                    capability_reason: CapabilityReason::Nullability,
                },
            );
        }
        return Ok(());
    };
    let Ok(inspection) = inspect_wkb(bytes, limits) else {
        violations.insert(
            source_index,
            WriteRowViolation {
                source_index,
                cause: "conversion.invalid_geometry",
                column: contract.name.clone(),
                capability_reason: CapabilityReason::GeometryEncoding,
            },
        );
        return Ok(());
    };
    if let Err(error) = validate_inspected_geometry(driver, support, contract, &inspection) {
        let capability_reason = error
            .capability_reason
            .unwrap_or(CapabilityReason::GeometryEncoding);
        let cause = match capability_reason {
            CapabilityReason::Nullability => "contract.nullability",
            CapabilityReason::CoordinateDimensions => "contract.coordinate_dimensions",
            CapabilityReason::MixedGeometry => "contract.geometry_type",
            _ => "contract.geometry_encoding",
        };
        violations.insert(
            source_index,
            WriteRowViolation {
                source_index,
                cause,
                column: contract.name.clone(),
                capability_reason,
            },
        );
        return Ok(());
    }
    *components = components
        .checked_add(u64::try_from(inspection.components).map_err(|_| {
            PlenoraIoError::LimitExceeded("geometria oltre il conteggio supportato".to_owned())
        })?)
        .ok_or_else(|| {
            PlenoraIoError::LimitExceeded(
                "overflow nel conteggio dei componenti geometrici".to_owned(),
            )
        })?;
    Ok(())
}

fn write_rejection_error(
    driver: &'static str,
    _batch_rows: u64,
    row_offset: u64,
    violations: &BTreeMap<u64, WriteRowViolation>,
    input_total: Option<u64>,
) -> PlenoraIoError {
    const EXAMPLES_LIMIT: u64 = 64;
    let Some(input_total) = input_total.filter(|total| *total > 0) else {
        return PlenoraIoError::Contract(
            "input_total esatto richiesto prima della validazione row-scoped".to_owned(),
        );
    };
    let observed_total = saturating_u64(violations.len());
    let mut counts = BTreeMap::new();
    for violation in violations.values() {
        *counts.entry(violation.cause.to_owned()).or_insert(0_u64) += 1;
    }
    let mut column_name_unattestable = false;
    // `EXAMPLES_LIMIT` e' la costante letterale 64: la conversione e' esatta
    // su ogni target supportato.
    #[allow(clippy::cast_possible_truncation)]
    let examples = violations
        .values()
        .take(EXAMPLES_LIMIT as usize)
        .map(|violation| {
            let column = RowDiagnosticColumn::attest(violation.column.clone());
            column_name_unattestable |= !column.is_attested();
            RowDiagnosticExample {
                source_index: violation.source_index,
                cause: violation.cause.to_owned(),
                column: column.into_option(),
                key: None,
                write_state: Some(RowDiagnosticWriteState::CertainlyRejected),
            }
        })
        .collect::<Vec<_>>();
    let first_reason = violations
        .values()
        .next()
        .map(|violation| violation.capability_reason);
    let diagnostics = RowDiagnostics {
        contract: ROW_DIAGNOSTICS_CONTRACT.to_owned(),
        scope: RowDiagnosticScope::Write,
        index_basis: ROW_DIAGNOSTICS_INDEX_BASIS.to_owned(),
        completeness: RowDiagnosticsCompleteness::Partial,
        knowledge_limits: Some({
            let mut limits = vec!["write_validation_stopped_at_first_rejected_batch".to_owned()];
            if column_name_unattestable {
                limits.push(ROW_DIAGNOSTIC_COLUMN_UNATTESTABLE.to_owned());
            }
            limits
        }),
        observed_total,
        total: None,
        input_total: Some(input_total),
        counts,
        examples_limit: EXAMPLES_LIMIT,
        examples_truncated: observed_total > EXAMPLES_LIMIT,
        examples,
        diagnostic_state_counts: Some(WriteDiagnosticStateCounts {
            certainly_rejected: observed_total,
            certainly_not_attempted: 0,
            certainly_rolled_back: 0,
            effect_unknown: 0,
        }),
        write_outcome: Some(RowDiagnosticWriteOutcome {
            certainly_rejected: KnownOrUnknownCount::Known {
                value: observed_total,
            },
            certainly_not_attempted: KnownOrUnknownCount::Known {
                value: input_total
                    .saturating_sub(row_offset)
                    .saturating_sub(observed_total),
            },
            certainly_rolled_back: KnownOrUnknownCount::Unknown,
            effect_unknown: KnownOrUnknownCount::Unknown,
        }),
    };
    let mut error = PlenoraIoError::new(
        ErrorCategory::DataMapping,
        ErrorPhase::Write,
        RemoteEffect::None,
        RetryDisposition::Never,
        format!("righe rifiutate prima della scrittura {driver}"),
    );
    error = error.with_row_diagnostics(diagnostics);
    error.driver = Some(driver.to_owned());
    error.capability_reason = first_reason;
    error
}

/// Costruisce un rifiuto row-scoped per i vincoli runtime specifici di un
/// writer.
///
/// Gli indici relativi sono trasformati in indici fisici globali solo perché
/// il chiamante opera sul `RecordBatch` sorgente, prima di mutarlo o
/// consegnarlo al backend.
#[must_use]
pub fn write_row_rejection(
    driver: &'static str,
    row_offset: u64,
    batch_rows: usize,
    rejections: &[(usize, &'static str, &str)],
    input_total: Option<u64>,
) -> PlenoraIoError {
    let mut violations = BTreeMap::new();
    for (row, cause, column) in rejections {
        let Ok(relative) = u64::try_from(*row) else {
            continue;
        };
        let Some(source_index) = row_offset.checked_add(relative) else {
            continue;
        };
        violations
            .entry(source_index)
            .or_insert_with(|| WriteRowViolation {
                source_index,
                cause,
                column: (*column).to_owned(),
                capability_reason: row_rejection_capability_reason(cause),
            });
    }
    if violations.is_empty() {
        return PlenoraIoError::new(
            ErrorCategory::Internal,
            ErrorPhase::Write,
            RemoteEffect::None,
            RetryDisposition::Never,
            "rifiuto row-scoped richiesto senza righe attribuibili",
        );
    }
    write_rejection_error(
        driver,
        saturating_u64(batch_rows),
        row_offset,
        &violations,
        input_total,
    )
}

/// Attribuisce un errore di lettura a una riga sorgente soltanto quando il
/// driver ne attesta l'identita'.
///
/// L'errore tipizzato resta autorevole; il report e' sempre non-completo
/// perche' la scansione si interrompe.
pub fn read_row_error(
    mut error: PlenoraIoError,
    source_index: Option<u64>,
    cause: &'static str,
    column: Option<&str>,
) -> PlenoraIoError {
    const EXAMPLES_LIMIT: u64 = 64;
    if error.row_diagnostics.is_some() {
        return error;
    }
    let column = column.map(|value| RowDiagnosticColumn::attest(value.to_owned()));
    let column_attestable = column.as_ref().is_none_or(RowDiagnosticColumn::is_attested);
    let mut knowledge_limits = vec!["scan_terminated_before_eof".to_owned()];
    if source_index.is_none() {
        knowledge_limits.push("source_row_identity_unattestable".to_owned());
    }
    if !column_attestable {
        knowledge_limits.push(ROW_DIAGNOSTIC_COLUMN_UNATTESTABLE.to_owned());
    }
    knowledge_limits.sort();
    let examples = source_index
        .map(|source_index| RowDiagnosticExample {
            source_index,
            cause: cause.to_owned(),
            column: column.and_then(RowDiagnosticColumn::into_option),
            key: None,
            write_state: None,
        })
        .into_iter()
        .collect();
    let diagnostics = RowDiagnostics {
        contract: ROW_DIAGNOSTICS_CONTRACT.to_owned(),
        scope: RowDiagnosticScope::Read,
        index_basis: ROW_DIAGNOSTICS_INDEX_BASIS.to_owned(),
        completeness: if source_index.is_some() {
            RowDiagnosticsCompleteness::Partial
        } else {
            RowDiagnosticsCompleteness::Unknown
        },
        knowledge_limits: Some(knowledge_limits),
        observed_total: 1,
        total: None,
        input_total: None,
        counts: BTreeMap::from([(cause.to_owned(), 1)]),
        examples_limit: EXAMPLES_LIMIT,
        examples_truncated: false,
        examples,
        diagnostic_state_counts: None,
        write_outcome: None,
    };
    debug_assert!(diagnostics.validate().is_ok());
    error.row_diagnostics = Some(Box::new(diagnostics));
    error
}

fn row_rejection_capability_reason(cause: &str) -> CapabilityReason {
    if cause == "contract.nullability" {
        CapabilityReason::Nullability
    } else if cause.contains("coordinate_dimensions") || cause.contains("measure_ordinate") {
        CapabilityReason::CoordinateDimensions
    } else if cause.contains("encoding")
        || cause.contains("embedded_srid")
        || cause.contains("invalid_geometry")
    {
        CapabilityReason::GeometryEncoding
    } else if cause.contains("mixed_geometry") {
        CapabilityReason::MixedGeometry
    } else if cause.contains("cell_") || cause.contains("layer_") {
        CapabilityReason::TypeNotRepresentable
    } else {
        CapabilityReason::GeometryNotSupported
    }
}

pub struct Published {
    pub bytes: u64,
    pub loss: LossReport,
    /// Valutazione specifica della scrittura conclusa (ADR-IO 5).
    pub fidelity: FidelityAssessment,
    /// Esito di durabilità del publish (ADR-IO 2).
    pub outcome: crate::publish::PublishOutcome,
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::Write;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use arrow_array::{BinaryArray, Int64Array};
    use arrow_schema::{DataType, Field, Schema};
    use plenora_io_model::contract::{CoordinateDimensions, FieldId, GeometryColumnContract};
    use plenora_io_model::crs::{CrsKind, CrsResolution, ResolvedCrs};
    use plenora_io_model::geometry::{
        ARROW_EXTENSION_NAME_KEY, GEOARROW_WKB_EXTENSION, PLENORA_DIMENSIONS_KEY,
    };
    use plenora_io_model::wkb::{encode_wkb, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue};

    use super::*;
    use crate::descriptor::WKB_XY_GEOMETRY;

    fn scan_dir_with(entries: usize, limits: Limits) -> Result<PathBuf> {
        let root = tempfile::tempdir().expect("tempdir");
        for index in 0..entries {
            let mut file = std::fs::File::create(root.path().join(format!("entry-{index}.bin")))
                .expect("file");
            file.write_all(b"x").expect("write");
        }
        Source::Path(root.path().to_path_buf()).into_path_checked(
            &limits,
            &CancellationToken::new(),
            &ResourceBudget::default(),
        )
    }

    /// L0.9: senza tetto sulle entry una directory ostile fa crescere la coda
    /// dello scan senza limite, perche' i byte si sommano solo sui file.
    #[test]
    fn directory_scan_over_max_input_entries_rejects_with_typed_error() {
        let limits = Limits {
            max_input_entries: 4,
            ..Limits::default()
        };
        // La radice conta come entry: 4 file piu' la directory sono 5.
        let error = scan_dir_with(4, limits).expect_err("il quinto elemento deve far fallire");
        assert_eq!(error.code, plenora_io_model::IoErrorCode::LimitExceeded);
        assert_eq!(
            error.category,
            plenora_io_model::ErrorCategory::ResourceLimit
        );
    }

    #[test]
    fn directory_scan_within_max_input_entries_succeeds() {
        let limits = Limits {
            max_input_entries: 4,
            ..Limits::default()
        };
        assert!(
            scan_dir_with(3, limits).is_ok(),
            "radice + 3 file = 4 entry"
        );
    }

    #[test]
    fn max_input_entries_default_admits_a_realistic_directory() {
        // Il default non deve rifiutare una directory di file legittimi:
        // un tetto troppo stretto sarebbe un fail-closed inutile.
        assert!(scan_dir_with(64, Limits::default()).is_ok());
    }

    #[test]
    fn entry_cap_is_checked_before_the_byte_sum() {
        // Con un tetto di entry raggiunto e un limite di byte larghissimo,
        // deve vincere il tetto delle entry: e' l'ordine dichiarato da INV-9.
        let limits = Limits {
            max_input_entries: 2,
            max_input_bytes: u64::MAX,
            ..Limits::default()
        };
        let error = scan_dir_with(8, limits).expect_err("il tetto entry deve intervenire");
        assert!(
            error.message.contains("entry"),
            "messaggio: {}",
            error.message
        );
    }

    /// La barriera contro i panic di arrow deve restituire un errore del
    /// driver invece di far abortire il processo, e non deve interferire con
    /// il percorso normale.
    ///
    /// Questo test e' l'unica copertura possibile del meccanismo: i fuzz
    /// target che esercitano i percorsi Arrow e Parquet restano rossi anche a
    /// barriera funzionante, perche' `libfuzzer-sys` chiama
    /// `std::process::abort()` prima dell'unwinding (0.4.10, src/lib.rs:92-95)
    /// apposta perche' un `catch_unwind` non possa nascondergli difetti.
    #[test]
    fn la_barriera_arrow_converte_il_panico_in_errore_del_driver() {
        // L'hook del processo stamperebbe comunque su stderr e farebbe
        // sembrare la suite fallita: silenziato per la durata del test.
        let precedente = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));

        // Panico con messaggio formattato: il payload e' una `String`.
        let formattato = leggendo_arrow("parquet", || -> Result<()> {
            panic!("precisione {} non supportata", 194)
        });
        // Panico con letterale: il payload e' un `&'static str`.
        let letterale = leggendo_arrow("arrow", || -> Result<()> {
            panic!("Type NONE not supported")
        });
        // Percorso normale: la barriera non deve alterare nulla.
        let riuscito = leggendo_arrow("arrow", || Ok(7_u8));
        // Errore ordinario: deve passare invariato, non essere riclassificato.
        let fallito = leggendo_arrow("arrow", || -> Result<u8> {
            Err(PlenoraIoError::format("arrow", "payload troncato"))
        });

        // Stesso panico del primo: l'impronta deve coincidere, altrimenti non
        // correla niente.
        let ripetuto = leggendo_arrow("parquet", || -> Result<()> {
            panic!("precisione {} non supportata", 194)
        });

        std::panic::set_hook(precedente);

        // `message` dichiara di non contenere payload: il testo del panico
        // arriva da una libreria di terze parti ed e' derivato dal file, quindi
        // non deve comparire.
        let formattato = formattato.expect_err("il panico deve diventare errore");
        assert!(
            !formattato.to_string().contains("precisione 194"),
            "il messaggio del panico non deve finire nell'errore pubblico: {formattato}"
        );
        assert!(
            formattato.to_string().contains("impronta"),
            "serve l'impronta per correlare le occorrenze: {formattato}"
        );

        let letterale = letterale.expect_err("il panico deve diventare errore");
        assert!(
            !letterale.to_string().contains("Type NONE"),
            "nemmeno il payload letterale deve comparire: {letterale}"
        );

        // Panici diversi devono dare impronte diverse, altrimenti non
        // distinguono nulla.
        assert_ne!(formattato.to_string(), letterale.to_string());

        // E lo stesso panico deve dare la stessa impronta.
        let ripetuto = ripetuto.expect_err("il panico deve diventare errore");
        assert_eq!(formattato.to_string(), ripetuto.to_string());

        assert_eq!(riuscito.expect("il percorso normale non e' toccato"), 7);
        assert!(fallito
            .expect_err("l'errore ordinario resta tale")
            .to_string()
            .contains("payload troncato"));
    }

    /// L'impronta deve essere stabile nel tempo, non solo dentro una singola
    /// esecuzione: un valore che cambia fra due versioni di Rust non permette
    /// piu' di correlare un'occorrenza di oggi con una di ieri. FNV-1a e'
    /// scritto a mano proprio per questo, e il valore atteso e' fissato qui.
    #[test]
    fn l_impronta_del_panico_e_stabile_fra_esecuzioni() {
        assert_eq!(impronta_del_panico(""), "cbf29ce484222325");
        assert_eq!(impronta_del_panico("a"), "af63dc4c8601ec8c");
        assert_eq!(
            impronta_del_panico("Type NONE not supported").len(),
            16,
            "l'impronta e' sempre di sedici cifre esadecimali"
        );
    }

    #[test]
    fn periodic_cancellation_has_a_bounded_check_interval() {
        let token = CancellationToken::new();
        token.cancel();
        assert!(matches!(
            check_cancelled_periodically(&token, ErrorPhase::Read, 0),
            Err(error) if error.code == plenora_io_model::IoErrorCode::Cancelled
        ));
        assert!(check_cancelled_periodically(&token, ErrorPhase::Read, 1).is_ok());
        assert!(matches!(
            check_cancelled_periodically(
                &token,
                ErrorPhase::Read,
                CANCELLATION_CHECK_INTERVAL
            ),
            Err(error) if error.code == plenora_io_model::IoErrorCode::Cancelled
        ));
    }

    struct FinishTrackingWriter {
        finished: Arc<AtomicBool>,
    }

    impl FormatWriter for FinishTrackingWriter {
        fn write(&mut self, _batch: &RecordBatch) -> Result<()> {
            Ok(())
        }

        fn finish(self: Box<Self>) -> Result<Published> {
            self.finished.store(true, Ordering::SeqCst);
            Ok(Published {
                bytes: 0,
                loss: LossReport::default(),
                fidelity: FidelityAssessment::lossless(),
                outcome: crate::publish::PublishOutcome::Published,
            })
        }
    }

    #[test]
    fn failed_write_poisons_writer_and_prevents_finish() {
        let finished = Arc::new(AtomicBool::new(false));
        let limits = Limits {
            max_rows: 0,
            ..Limits::default()
        };
        let mut writer = with_write_limits(
            Box::new(FinishTrackingWriter {
                finished: finished.clone(),
            }),
            limits,
        );
        let schema = Arc::new(Schema::new(vec![Field::new(
            "value",
            DataType::Int64,
            false,
        )]));
        let batch =
            RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from(vec![1]))]).unwrap();

        assert!(matches!(
            writer.write(&batch),
            Err(error) if error.code == plenora_io_model::IoErrorCode::LimitExceeded
        ));
        assert!(matches!(
            writer.write(&batch),
            Err(error) if error.code == plenora_io_model::IoErrorCode::Format
        ));
        assert!(matches!(
            writer.finish(),
            Err(error) if error.code == plenora_io_model::IoErrorCode::Format
        ));
        assert!(!finished.load(Ordering::SeqCst));
    }

    #[test]
    fn declared_input_total_rejects_extra_rows() {
        let finished = Arc::new(AtomicBool::new(false));
        let mut writer = with_write_limits(
            Box::new(FinishTrackingWriter {
                finished: finished.clone(),
            }),
            Limits::default(),
        );
        writer.declare_input_total(LayerId(0), 0).unwrap();
        let schema = Arc::new(Schema::new(vec![Field::new(
            "value",
            DataType::Int64,
            false,
        )]));
        let batch =
            RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from(vec![1]))]).unwrap();

        assert!(matches!(
            writer.write(&batch),
            Err(error) if error.code == plenora_io_model::IoErrorCode::Contract
        ));
        assert!(!finished.load(Ordering::SeqCst));
    }

    #[test]
    fn declared_input_total_rejects_early_eof_before_publish() {
        let finished = Arc::new(AtomicBool::new(false));
        let mut writer = with_write_limits(
            Box::new(FinishTrackingWriter {
                finished: finished.clone(),
            }),
            Limits::default(),
        );
        writer.declare_input_total(LayerId(0), 10).unwrap();
        let schema = Arc::new(Schema::new(vec![Field::new(
            "value",
            DataType::Int64,
            false,
        )]));
        let batch =
            RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from_iter_values(0..9))])
                .unwrap();
        writer.write(&batch).unwrap();

        let Err(error) = writer.finish() else {
            panic!("EOF anticipato pubblicato")
        };
        assert_eq!(error.code, plenora_io_model::IoErrorCode::Contract);
        assert_eq!(error.category, ErrorCategory::InvalidPlan);
        assert_eq!(error.phase, ErrorPhase::Validate);
        assert!(!finished.load(Ordering::SeqCst));
    }

    #[test]
    fn each_layer_can_declare_its_total_before_its_first_write() {
        let finished = Arc::new(AtomicBool::new(false));
        let mut writer: Box<dyn FormatWriter> = Box::new(LimitedWriter {
            inner: Box::new(FinishTrackingWriter { finished }),
            driver: "test",
            limits: Limits::default(),
            rows: 0,
            layer_rows: vec![0, 0],
            input_totals: vec![None, None],
            failed: false,
            contracts: Vec::new(),
            geometry_validation: None,
            fidelity: FidelityAssessment::lossless(),
            planned_loss: LossReport::default(),
            cancellation: CancellationToken::new(),
            resource_budget: ResourceBudget::default(),
            _operation_lease: None,
        });
        writer.declare_input_total(LayerId(0), 1).unwrap();
        let schema = Arc::new(Schema::new(vec![Field::new(
            "value",
            DataType::Int64,
            false,
        )]));
        let batch =
            RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from(vec![1]))]).unwrap();
        writer.write_to_layer(LayerId(0), &batch).unwrap();

        writer.declare_input_total(LayerId(1), 0).unwrap();
    }

    #[test]
    fn source_size_is_checked_before_parsing() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(&[0_u8; 8]).unwrap();
        let limits = Limits {
            max_input_bytes: 7,
            ..Limits::default()
        };
        let result = Source::Path(file.path().to_owned()).into_path_checked(
            &limits,
            &CancellationToken::new(),
            &ResourceBudget::default(),
        );
        assert!(matches!(
            result,
            Err(error) if error.code == plenora_io_model::IoErrorCode::LimitExceeded
        ));
    }

    #[test]
    fn cancelled_source_is_rejected_before_filesystem_probe() {
        let token = CancellationToken::new();
        token.cancel();
        let result = Source::Path(std::path::PathBuf::from("not-observed")).into_path_checked(
            &Limits::default(),
            &token,
            &ResourceBudget::default(),
        );
        assert!(matches!(
            result,
            Err(error)
                if error.code == plenora_io_model::IoErrorCode::Cancelled
                    && error.phase == ErrorPhase::Probe
        ));
    }

    #[test]
    fn cancellation_before_finish_never_publishes() {
        let finished = Arc::new(AtomicBool::new(false));
        let token = CancellationToken::new();
        let writer: Box<dyn FormatWriter> = Box::new(LimitedWriter {
            inner: Box::new(FinishTrackingWriter {
                finished: finished.clone(),
            }),
            driver: "test",
            limits: Limits::default(),
            rows: 0,
            layer_rows: vec![0],
            input_totals: vec![None],
            failed: false,
            contracts: Vec::new(),
            geometry_validation: None,
            fidelity: FidelityAssessment::lossless(),
            planned_loss: LossReport::default(),
            cancellation: token.clone(),
            resource_budget: ResourceBudget::default(),
            _operation_lease: None,
        });
        token.cancel();

        assert!(matches!(
            writer.finish(),
            Err(error)
                if error.code == plenora_io_model::IoErrorCode::Cancelled
                    && error.phase == ErrorPhase::Finalize
        ));
        assert!(!finished.load(Ordering::SeqCst));
    }

    struct TestReader {
        layer: LayerContract,
        batches: usize,
        fail: bool,
    }

    impl LayerReader for TestReader {
        fn contract(&self) -> &LayerContract {
            &self.layer
        }

        fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
            if self.fail {
                self.fail = false;
                return Err(PlenoraIoError::Contract("errore terminale".to_owned()));
            }
            if self.batches == 0 {
                return Ok(None);
            }
            self.batches -= 1;
            Ok(Some(RecordBatch::new_empty(Arc::new(Schema::empty()))))
        }
    }

    fn test_reader(batches: usize, fail: bool) -> Box<dyn LayerReader> {
        Box::new(TestReader {
            layer: test_layer(),
            batches,
            fail,
        })
    }

    fn test_layer() -> LayerContract {
        LayerContract {
            id: LayerId(0),
            name: "layer".to_owned(),
            contract: plenora_io_model::contract::DataContract {
                schema: Arc::new(Schema::empty()),
                geometry: None,
            },
        }
    }

    fn fixed_batch_reader(values: Vec<i64>) -> Box<dyn LayerReader> {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "value",
            DataType::Int64,
            false,
        )]));
        let batch =
            RecordBatch::try_new(schema.clone(), vec![Arc::new(Int64Array::from(values))]).unwrap();
        Box::new(FixedBatchReader {
            layer: LayerContract {
                id: LayerId(0),
                name: "layer".to_owned(),
                contract: plenora_io_model::contract::DataContract {
                    schema,
                    geometry: None,
                },
            },
            batch: Some(batch),
        })
    }

    struct FixedBatchReader {
        layer: LayerContract,
        batch: Option<RecordBatch>,
    }

    impl LayerReader for FixedBatchReader {
        fn contract(&self) -> &LayerContract {
            &self.layer
        }

        fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
            Ok(self.batch.take())
        }
    }

    #[test]
    fn batch_target_slices_without_reordering_and_releases_gate_at_eof() {
        let gate = SingleReaderGate::new("test");
        let inner = gate
            .open(LayerId(0), || Ok(fixed_batch_reader(vec![0, 1, 2, 3, 4])))
            .unwrap();
        let mut reader = with_batch_target(
            inner,
            BatchTarget {
                target_bytes: 16,
                max_rows: 100,
            },
            CancellationToken::new(),
        );
        assert!(matches!(
            gate.open(LayerId(0), || Ok(test_reader(1, false))),
            Err(error) if error.code == plenora_io_model::IoErrorCode::ReaderBusy
        ));

        let mut sizes = Vec::new();
        let mut values = Vec::new();
        while let Some(batch) = reader.next_batch().unwrap() {
            sizes.push(batch.num_rows());
            values.extend_from_slice(
                batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap()
                    .values(),
            );
        }
        assert_eq!(sizes, vec![2, 2, 1]);
        assert_eq!(values, vec![0, 1, 2, 3, 4]);
        assert!(gate.open(LayerId(0), || Ok(test_reader(1, false))).is_ok());
    }

    #[test]
    fn single_reader_gate_releases_on_drop_eof_and_error() {
        let gate = SingleReaderGate::new("test");
        let first = gate.open(LayerId(0), || Ok(test_reader(1, false))).unwrap();
        assert!(matches!(
            gate.open(LayerId(0), || Ok(test_reader(1, false))),
            Err(error)
                if error.code == plenora_io_model::IoErrorCode::ReaderBusy
                    && error.driver.as_deref() == Some("test")
        ));

        drop(first);
        assert!(gate
            .open(LayerId(0), || {
                Err(PlenoraIoError::Contract("costruzione fallita".to_owned()))
            })
            .is_err());
        let mut exhausted = gate.open(LayerId(0), || Ok(test_reader(1, false))).unwrap();
        assert!(exhausted.next_batch().unwrap().is_some());
        assert!(exhausted.next_batch().unwrap().is_none());
        let after_eof = gate.open(LayerId(0), || Ok(test_reader(1, false))).unwrap();
        drop(after_eof);

        let mut failed = gate.open(LayerId(0), || Ok(test_reader(0, true))).unwrap();
        assert!(failed.next_batch().is_err());
        assert!(gate.open(LayerId(0), || Ok(test_reader(1, false))).is_ok());
    }

    #[test]
    fn cancelled_reader_releases_single_reader_lease() {
        let gate = SingleReaderGate::new("test");
        let inner = gate.open(LayerId(0), || Ok(test_reader(1, false))).unwrap();
        let token = CancellationToken::new();
        let mut reader = with_cancellation(inner, token.clone());
        token.cancel();

        assert!(matches!(
            reader.next_batch(),
            Err(error)
                if error.code == plenora_io_model::IoErrorCode::Cancelled
                    && error.phase == ErrorPhase::Read
        ));
        assert!(gate.open(LayerId(0), || Ok(test_reader(1, false))).is_ok());
    }

    fn crs_reader(crs_id: &str, srid: i32) -> Box<dyn LayerReader> {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "geometry",
            DataType::Binary,
            true,
        )]));
        let mut geometry = GeometryColumnContract::wkb_xy(
            FieldId(0),
            "geometry",
            ResolvedCrs::new(Some(crs_id.to_owned()), CrsKind::Geographic, None),
            true,
        );
        geometry.srid = Some(srid);
        Box::new(TestReader {
            layer: LayerContract {
                id: LayerId(0),
                name: "conflicting".to_owned(),
                contract: plenora_io_model::contract::DataContract::new(schema, Some(geometry)),
            },
            batches: 0,
            fail: false,
        })
    }

    #[test]
    fn read_boundary_preserves_and_reports_conflicting_crs_representations() {
        let reader = with_cancellation(crs_reader("EPSG:4326", 3003), CancellationToken::new());

        assert_eq!(
            reader.contract().contract.geometry.as_ref().unwrap().srid,
            Some(3003)
        );
        assert_eq!(
            reader
                .contract()
                .contract
                .geometry
                .as_ref()
                .unwrap()
                .crs
                .id(),
            Some("EPSG:4326")
        );
        let loss = reader.loss_report();
        assert_eq!(
            loss.counts.get(crate::INCONSISTENT_CRS_REPRESENTATIONS),
            Some(&1)
        );
        assert_eq!(loss.examples().len(), 1);
        assert!(loss.examples()[0].context.contains("crs_id=EPSG:4326"));
        assert!(loss.examples()[0].context.contains("srid=3003"));
    }

    #[test]
    fn read_boundary_does_not_report_matching_crs_representations() {
        let reader = with_batch_target(
            crs_reader("EPSG:4326", 4326),
            BatchTarget::default(),
            CancellationToken::new(),
        );

        assert!(reader.loss_report().is_empty());
    }

    #[test]
    fn write_loss_names_each_non_preserved_crs_representation_and_state() {
        let mut loss = LossReport::default();
        record_crs_representation_loss(
            &mut loss,
            "layer",
            "geometry",
            "crs_id",
            Some(9),
            CrsRepresentationState::Derived,
        );
        record_crs_representation_loss(
            &mut loss,
            "layer",
            "geometry",
            "srid",
            Some(4),
            CrsRepresentationState::Absent,
        );
        record_crs_representation_loss(
            &mut loss,
            "layer",
            "geometry",
            "crs_definition",
            Some(42),
            CrsRepresentationState::Preserved,
        );

        assert_eq!(loss.counts.get("crs_id_not_preserved_derived"), Some(&1));
        assert_eq!(loss.counts.get("srid_not_preserved_absent"), Some(&1));
        assert!(!loss
            .counts
            .contains_key("crs_definition_not_preserved_absent"));
        assert!(loss
            .examples()
            .iter()
            .any(|example| example.context.contains("value_bytes=9")));
    }

    fn geometry_batch(bytes: Option<&[u8]>) -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "geometry",
            DataType::Binary,
            true,
        )]));
        let geometry = BinaryArray::from(vec![bytes]);
        RecordBatch::try_new(schema, vec![Arc::new(geometry)]).unwrap()
    }

    fn xy_contract(nullable: bool) -> GeometryColumnContract {
        GeometryColumnContract::wkb_xy(FieldId(0), "geometry", CrsResolution::Missing, nullable)
    }

    #[test]
    fn runtime_geometry_validation_rejects_hidden_z_payload() {
        let xyz = WkbGeometry {
            value: WkbValue::Point(WkbCoordinate {
                x: 1.0,
                y: 2.0,
                z: Some(3.0),
                m: None,
            }),
            dimensions: CoordinateDimensions::Xyz,
            srid: None,
        };
        let bytes = encode_wkb(&xyz, WkbFlavor::Iso).unwrap();
        let result = validate_geometry_batch_at(
            "test",
            WKB_XY_GEOMETRY,
            Some(&xy_contract(true)),
            &geometry_batch(Some(&bytes)),
            &Limits::default(),
            0,
            Some(1),
        );
        assert!(matches!(
            result,
            Err(error)
                if error.capability_reason == Some(CapabilityReason::CoordinateDimensions)
        ));
    }

    #[test]
    fn runtime_geometry_validation_rejects_undeclared_ewkb_srid() {
        let ewkb = WkbGeometry {
            value: WkbValue::Point(WkbCoordinate {
                x: 1.0,
                y: 2.0,
                z: None,
                m: None,
            }),
            dimensions: CoordinateDimensions::Xy,
            srid: Some(4326),
        };
        let bytes = encode_wkb(&ewkb, WkbFlavor::Ewkb).unwrap();
        let result = validate_geometry_batch_at(
            "test",
            WKB_XY_GEOMETRY,
            Some(&xy_contract(true)),
            &geometry_batch(Some(&bytes)),
            &Limits::default(),
            0,
            Some(1),
        );
        assert!(matches!(
            result,
            Err(error)
                if error.capability_reason == Some(CapabilityReason::GeometryEncoding)
        ));
    }

    #[test]
    fn runtime_geometry_validation_enforces_nullability() {
        let result = validate_geometry_batch_at(
            "test",
            WKB_XY_GEOMETRY,
            Some(&xy_contract(false)),
            &geometry_batch(None),
            &Limits::default(),
            0,
            Some(1),
        );
        assert!(matches!(
            result,
            Err(error) if error.capability_reason == Some(CapabilityReason::Nullability)
        ));
    }

    #[test]
    fn runtime_write_rejections_have_bounded_global_row_diagnostics() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "geometry",
            DataType::Binary,
            true,
        )]));
        let geometry =
            BinaryArray::from(vec![Some(&[1_u8, 1, 0][..]), None, Some(&[1_u8, 1, 0][..])]);
        let batch = RecordBatch::try_new(schema, vec![Arc::new(geometry)]).unwrap();

        let error = validate_geometry_batch_at(
            "test",
            WKB_XY_GEOMETRY,
            Some(&xy_contract(false)),
            &batch,
            &Limits::default(),
            1_000,
            Some(1_003),
        )
        .unwrap_err();

        assert_eq!(error.category, ErrorCategory::DataMapping);
        assert_eq!(error.phase, ErrorPhase::Write);
        let diagnostics = error.row_diagnostics.as_deref().unwrap();
        assert_eq!(diagnostics.observed_total, 3);
        assert_eq!(diagnostics.input_total, Some(1_003));
        assert_eq!(diagnostics.examples[0].source_index, 1_000);
        assert_eq!(diagnostics.examples[1].source_index, 1_001);
        assert_eq!(diagnostics.examples[2].source_index, 1_002);
        assert!(diagnostics.validate().is_ok());
    }

    #[test]
    fn row_scoped_write_rejection_without_input_total_is_a_contract_error() {
        let error = write_row_rejection("test", 0, 1, &[(0, "test.rejected", "value")], None);

        assert_eq!(error.code, plenora_io_model::IoErrorCode::Contract);
        assert_eq!(error.category, ErrorCategory::InvalidPlan);
        assert_eq!(error.phase, ErrorPhase::Validate);
        assert!(error.row_diagnostics.is_none());
    }

    #[test]
    fn row_diagnostics_hide_unattestable_columns_without_losing_causes() {
        for invalid in [String::new(), "private".repeat(40)] {
            let error = write_row_rejection(
                "test",
                0,
                1,
                &[(0, "test.cell_not_representable", invalid.as_str())],
                Some(1),
            );
            let diagnostics = error.row_diagnostics.as_deref().unwrap();
            assert_eq!(diagnostics.counts["test.cell_not_representable"], 1);
            assert_eq!(diagnostics.examples[0].column, None);
            assert!(diagnostics
                .knowledge_limits
                .as_deref()
                .unwrap()
                .contains(&ROW_DIAGNOSTIC_COLUMN_UNATTESTABLE.to_owned()));
            assert!(diagnostics.validate().is_ok());
        }

        let unicode = "citta_ðŸŒ";
        let error = write_row_rejection(
            "test",
            0,
            1,
            &[(0, "test.cell_not_representable", unicode)],
            Some(1),
        );
        let diagnostics = error.row_diagnostics.as_deref().unwrap();
        assert_eq!(diagnostics.examples[0].column.as_deref(), Some(unicode));
        assert!(diagnostics.validate().is_ok());
    }

    #[test]
    fn read_row_error_emits_examples_only_for_attestable_indices() {
        let attested = read_row_error(
            PlenoraIoError::format("test", "bad row"),
            Some(7),
            "test.invalid_row",
            Some("value"),
        );
        let diagnostics = attested.row_diagnostics.as_deref().unwrap();
        assert_eq!(
            diagnostics.completeness,
            RowDiagnosticsCompleteness::Partial
        );
        assert_eq!(diagnostics.examples[0].source_index, 7);
        assert!(diagnostics.validate().is_ok());

        let unknown = read_row_error(
            PlenoraIoError::format("test", "bad row"),
            None,
            "test.invalid_row",
            Some("value"),
        );
        let diagnostics = unknown.row_diagnostics.as_deref().unwrap();
        assert_eq!(
            diagnostics.completeness,
            RowDiagnosticsCompleteness::Unknown
        );
        assert!(diagnostics.examples.is_empty());
        assert!(diagnostics
            .knowledge_limits
            .as_deref()
            .unwrap()
            .contains(&"source_row_identity_unattestable".to_owned()));
        assert!(diagnostics.validate().is_ok());
    }

    #[test]
    fn non_geometry_row_rejection_uses_a_non_geometry_capability_reason() {
        let error = write_row_rejection(
            "test",
            0,
            1,
            &[(0, "test.cell_not_representable", "value")],
            Some(1),
        );
        assert_eq!(
            error.capability_reason,
            Some(CapabilityReason::TypeNotRepresentable)
        );
    }

    fn geoarrow_field(name: &str, dimensions: Option<&str>) -> Field {
        let mut metadata = HashMap::from([(
            ARROW_EXTENSION_NAME_KEY.to_owned(),
            GEOARROW_WKB_EXTENSION.to_owned(),
        )]);
        if let Some(dimensions) = dimensions {
            metadata.insert(PLENORA_DIMENSIONS_KEY.to_owned(), dimensions.to_owned());
        }
        Field::new(name, DataType::Binary, true).with_metadata(metadata)
    }

    fn legacy_plan(fields: Vec<Field>) -> WritePlan {
        WritePlan {
            layers: vec![crate::request::WriteLayer {
                name: "layer".to_owned(),
                contract: plenora_io_model::contract::DataContract {
                    schema: Arc::new(Schema::new(fields)),
                    geometry: None,
                },
            }],
        }
    }

    #[test]
    fn legacy_geometry_defaults_xy_only_when_dimensions_are_absent() {
        let absent =
            geometry_contracts_for_validation(&legacy_plan(vec![geoarrow_field("geometry", None)]))
                .unwrap();
        let explicit_unknown =
            geometry_contracts_for_validation(&legacy_plan(vec![geoarrow_field(
                "geometry",
                Some("unknown"),
            )]))
            .unwrap();

        assert_eq!(
            absent[0].as_ref().unwrap().dimensions,
            CoordinateDimensions::Xy
        );
        assert_eq!(
            explicit_unknown[0].as_ref().unwrap().dimensions,
            CoordinateDimensions::Unknown
        );
    }

    #[test]
    fn ambiguous_or_invalid_legacy_geometry_metadata_is_rejected() {
        let ambiguous = legacy_plan(vec![
            geoarrow_field("geometry_a", None),
            geoarrow_field("geometry_b", None),
        ]);
        let invalid = legacy_plan(vec![geoarrow_field("geometry", Some("future"))]);

        assert!(matches!(
            geometry_contracts_for_validation(&ambiguous),
            Err(error) if error.code == plenora_io_model::IoErrorCode::Contract
        ));
        assert!(matches!(
            geometry_contracts_for_validation(&invalid),
            Err(error) if error.code == plenora_io_model::IoErrorCode::Contract
        ));
    }
}
