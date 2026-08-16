//! Adattatori comuni applicati ai `LayerReader`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use arrow_array::{Array, BinaryArray, LargeBinaryArray, RecordBatch};
use plenora_io_model::contract::{CoordinateDimensions, GeometryEncoding, LayerContract, LayerId};
use plenora_io_model::limits::WkbLimits;
use plenora_io_model::resource::{ResourceBudget, ResourceKind, ResourceLease};
use plenora_io_model::wkb::inspect_wkb;
use plenora_io_model::{
    CancellationToken, ErrorCategory, ErrorPhase, PlenoraIoError, RemoteEffect, Result,
    RetryDisposition, RowDiagnosticColumn, RowDiagnosticExample, RowDiagnosticScope,
    RowDiagnostics, RowDiagnosticsCompleteness, ROW_DIAGNOSTICS_CONTRACT,
    ROW_DIAGNOSTICS_INDEX_BASIS, ROW_DIAGNOSTIC_COLUMN_UNATTESTABLE,
};

use crate::loss::{declare_crs_inconsistency, LossReport};
use crate::request::{effective_batch_rows, incremental_batch_memory_size, BatchTarget, ReadScope};

use super::{saturating_usize, LayerReader, OpenDatasetHandle};
use crate::driver::spool::StagedSpool;

/// Collega a un dataset un budget condiviso. Ogni reader consuma colonne,
/// righe, byte e una quota di concorrenza dagli stessi contatori, anche quando
/// il budget attraversa più componenti della pipeline.
#[must_use]
pub fn with_read_budget(
    dataset: Box<dyn OpenDatasetHandle>,
    budget: ResourceBudget,
    physical_row_indices_attestable: bool,
) -> Box<dyn OpenDatasetHandle> {
    Box::new(BudgetedDataset {
        dataset,
        budget,
        physical_row_indices_attestable,
    })
}

struct BudgetedDataset {
    dataset: Box<dyn OpenDatasetHandle>,
    budget: ResourceBudget,
    physical_row_indices_attestable: bool,
}

impl OpenDatasetHandle for BudgetedDataset {
    fn layers(&self) -> &[LayerContract] {
        self.dataset.layers()
    }

    fn fidelity_assessment(&self) -> crate::loss::FidelityAssessment {
        self.dataset.fidelity_assessment()
    }

    fn open_layer_reader(
        &self,
        request: &crate::request::ReadRequest,
    ) -> Result<Box<dyn LayerReader>> {
        self.budget.ensure_active()?;
        // Il lease precede intenzionalmente la creazione del reader: diversi
        // driver avviano qui il worker parser e non devono farlo fuori quota.
        let operation_lease = self
            .budget
            .try_lease(ResourceKind::ConcurrentOperations, 1)?;
        let reader = self.dataset.open_layer_reader(request)?;
        let physical_row_indices_attestable = self.physical_row_indices_attestable
            && request.pruning_predicate.is_none()
            && request.spatial_pruning_hint.is_none();
        BudgetedReader::new(
            reader,
            self.budget.clone(),
            physical_row_indices_attestable,
            request.cancellation.clone(),
            request.batch_target,
            request.scope,
            operation_lease,
        )
        .map(|reader| Box::new(reader) as Box<dyn LayerReader>)
    }
}

/// Adapter di lettura *operation-atomic*, non streaming.
///
/// **Finding #2 review 2026-08-15**: nonostante l'API `LayerReader::next_batch`
/// suggerisca un modello streaming (batch per batch, con backpressure), questo
/// adapter esegue `drain_operation` durante la *prima* chiamata di
/// `next_batch`: itera la sorgente fino a EOF, verifica il contratto su tutti
/// i batch e li accumula in `buffered`. Solo dopo aver drenato l'intera
/// sorgente restituisce il primo batch al chiamante.
///
/// La ragione e' l'atomicita' operativa (commento a `drain_operation`,
/// paragrafo "Non è più possibile esporre alcun prefisso accepted"): se una
/// violazione emerge in un qualsiasi punto della sorgente, il chiamante non
/// deve aver mai visto un prefisso accepted; l'intera operazione viene
/// rigettata come un blocco unico. Il pattern semplifica il rollback lato
/// consumatore (writer, aggregazioni) al costo di:
///
/// - latenza al primo batch pari alla lettura completa della sorgente;
/// - memoria O(dataset), limitata dal budget `MemoryBytes` (default 512
///   MiB); superata quella soglia l'operazione fallisce fail-closed.
///
/// L'alternativa (spool bounded su file temporaneo, o cambio di contratto
/// verso streaming reale con errore terminale *dopo* batch consegnati) e'
/// tracciata come lotto L2 di `docs/ROADMAP-1.1.0.md` e richiede prima
/// l'ADR-IO 7 (PR-1). Fino ad allora questo adapter resta la semantica
/// autoritativa per tutti i driver che passano dal budget comune.
struct BudgetedReader {
    inner: Box<dyn LayerReader>,
    budget: ResourceBudget,
    rows_scanned: u64,
    physical_row_indices_attestable: bool,
    /// Costruito al primo batch, con lo schema effettivo di lettura: prima
    /// di allora non c'e' uno schema da dichiarare allo spool.
    spool: Option<StagedSpool>,
    drained: bool,
    terminal_error: Option<PlenoraIoError>,
    cancellation: CancellationToken,
    batch_target: BatchTarget,
    scope: ReadScope,
    _operation_lease: ResourceLease,
}

impl BudgetedReader {
    fn new(
        inner: Box<dyn LayerReader>,
        budget: ResourceBudget,
        physical_row_indices_attestable: bool,
        cancellation: CancellationToken,
        batch_target: BatchTarget,
        scope: ReadScope,
        operation_lease: ResourceLease,
    ) -> Result<Self> {
        let columns = u64::try_from(inner.contract().contract.schema.fields().len())
            .map_err(|_| PlenoraIoError::LimitExceeded("troppe colonne nel reader".to_owned()))?;
        if columns > 0 {
            budget
                .try_lease(ResourceKind::Columns, columns)?
                .commit(columns)?;
        }
        Ok(Self {
            inner,
            budget,
            rows_scanned: 0,
            physical_row_indices_attestable,
            spool: None,
            drained: false,
            terminal_error: None,
            cancellation,
            batch_target,
            scope,
            _operation_lease: operation_lease,
        })
    }

    // Ciclo di drenaggio con contabilizzazione, diagnostica e stati terminali
    // in sequenza: la lunghezza e' negli stati da coprire, non in complessita'
    // logica.
    #[allow(clippy::too_many_lines)]
    fn drain_operation(&mut self) -> Result<()> {
        if self.scope == ReadScope::AcceptedRows(0) {
            return Ok(());
        }
        let mut violations = ReadViolationAccumulator::new(
            self.physical_row_indices_attestable,
            READ_DIAGNOSTIC_EXAMPLES_LIMIT,
        );
        loop {
            self.budget.ensure_active().map_err(|error| {
                terminal_scan_error(error, &violations, self.physical_row_indices_attestable)
            })?;
            let batch_bytes = u64::try_from(self.batch_target.target_bytes)
                .unwrap_or(u64::MAX)
                .saturating_add(self.budget.limits().cell_bytes);
            let batch_rows = u64::try_from(self.batch_target.max_rows).unwrap_or(u64::MAX);
            let available_memory =
                bounded_reservation(&self.budget, ResourceKind::MemoryBytes, batch_bytes);
            let available_rows = bounded_reservation(&self.budget, ResourceKind::Rows, batch_rows);
            let available_output =
                bounded_reservation(&self.budget, ResourceKind::OutputBytes, batch_bytes);
            if available_memory == 0 || available_rows == 0 || available_output == 0 {
                // Quota esaurita non significa ancora violazione: puo' essere
                // semplicemente la fine della sorgente. Chiedere una
                // prenotazione non nulla solo per scoprire l'EOF faceva
                // fallire un dataset di N righe letto con quota esattamente N
                // — l'ultimo batch veniva consegnato, poi il giro successivo
                // trovava zero righe residue e trasformava la fine in errore.
                //
                // Si prova quindi a leggere: se la sorgente e' finita si esce
                // pulito, se produce ancora un batch la quota e' davvero
                // superata e l'errore resta lo stesso di prima.
                let next = self.inner.next_batch().map_err(|error| {
                    terminal_scan_error(error, &violations, self.physical_row_indices_attestable)
                })?;
                if next.is_some() {
                    return Err(terminal_scan_error(
                        PlenoraIoError::LimitExceeded(
                            "budget esaurito prima della materializzazione del batch".to_owned(),
                        ),
                        &violations,
                        self.physical_row_indices_attestable,
                    ));
                }
                break;
            }
            // Ogni operazione prenota al massimo il target bounded del proprio
            // batch, non tutto il residuo condiviso. L'inutilizzato torna
            // subito al budget.
            let memory_lease = self
                .budget
                .try_lease(ResourceKind::MemoryBytes, available_memory)
                .map_err(|error| {
                    terminal_scan_error(error, &violations, self.physical_row_indices_attestable)
                })?;
            let rows_lease = self
                .budget
                .try_lease(ResourceKind::Rows, available_rows)
                .map_err(|error| {
                    terminal_scan_error(error, &violations, self.physical_row_indices_attestable)
                })?;
            let output_lease = self
                .budget
                .try_lease(ResourceKind::OutputBytes, available_output)
                .map_err(|error| {
                    terminal_scan_error(error, &violations, self.physical_row_indices_attestable)
                })?;
            let next = self.inner.next_batch();
            let Some(batch) = next.map_err(|error| {
                terminal_scan_error(error, &violations, self.physical_row_indices_attestable)
            })?
            else {
                drop(memory_lease);
                drop(rows_lease);
                break;
            };
            let rows = u64::try_from(batch.num_rows()).map_err(|_| {
                terminal_scan_error(
                    PlenoraIoError::LimitExceeded("batch oltre il conteggio supportato".to_owned()),
                    &violations,
                    self.physical_row_indices_attestable,
                )
            })?;
            let bytes = u64::try_from(incremental_batch_memory_size(&batch)).map_err(|_| {
                terminal_scan_error(
                    PlenoraIoError::LimitExceeded(
                        "batch oltre il conteggio byte supportato".to_owned(),
                    ),
                    &violations,
                    self.physical_row_indices_attestable,
                )
            })?;
            if rows > rows_lease.amount()
                || bytes > memory_lease.amount()
                || bytes > output_lease.amount()
            {
                return Err(terminal_scan_error(
                    PlenoraIoError::LimitExceeded(
                        "batch materializzato oltre la quota prenotata".to_owned(),
                    ),
                    &violations,
                    self.physical_row_indices_attestable,
                ));
            }
            let batch_violations = collect_read_violations(
                self.inner.contract(),
                &batch,
                self.rows_scanned,
            )
            .map_err(|error| {
                terminal_scan_error(error, &violations, self.physical_row_indices_attestable)
            })?;
            let batch =
                with_effective_read_schema(self.inner.contract(), batch).map_err(|error| {
                    terminal_scan_error(error, &violations, self.physical_row_indices_attestable)
                })?;
            violations.record_all(batch_violations).map_err(|error| {
                terminal_scan_error(error, &violations, self.physical_row_indices_attestable)
            })?;
            let geometry_components = if violations.is_empty() {
                geometry_components(self.inner.contract(), &batch, &self.budget).map_err(
                    |error| {
                        terminal_scan_error(
                            error,
                            &violations,
                            self.physical_row_indices_attestable,
                        )
                    },
                )?
            } else {
                0
            };
            let geometry_lease = (geometry_components > 0)
                .then(|| {
                    self.budget
                        .try_lease(ResourceKind::GeometryComponents, geometry_components)
                })
                .transpose()
                .map_err(|error| {
                    terminal_scan_error(error, &violations, self.physical_row_indices_attestable)
                })?;
            if rows > 0 {
                rows_lease.commit(rows).map_err(|error| {
                    terminal_scan_error(error, &violations, self.physical_row_indices_attestable)
                })?;
            } else {
                drop(rows_lease);
            }
            // L0.3: la memoria **non** si committa. `commit` la consumerebbe
            // per sempre, e i batch bufferizzati la accumulerebbero fino a
            // O(dataset). La lease di materializzazione resta viva finche' il
            // batch non e' entrato nello spool, che prende la propria lease di
            // residenza: da quel momento la memoria e' contabilizzata dove il
            // batch vive davvero, e torna quando lo lascia.
            //
            // `OutputBytes` invece resta cumulativo: e' quota consumata, non
            // occupazione trattenuta.
            if bytes > 0 {
                output_lease.commit(bytes).map_err(|error| {
                    terminal_scan_error(error, &violations, self.physical_row_indices_attestable)
                })?;
            } else {
                drop(output_lease);
            }
            if let Some(lease) = geometry_lease {
                lease.commit(geometry_components).map_err(|error| {
                    terminal_scan_error(error, &violations, self.physical_row_indices_attestable)
                })?;
            }
            self.rows_scanned = self.rows_scanned.checked_add(rows).ok_or_else(|| {
                terminal_scan_error(
                    PlenoraIoError::LimitExceeded("overflow nel conteggio righe lette".to_owned()),
                    &violations,
                    self.physical_row_indices_attestable,
                )
            })?;
            // La prenotazione di materializzazione ha esaurito il suo scopo:
            // il batch esiste ed e' misurato. Va rilasciata **prima** che lo
            // spool prenda la lease di residenza, altrimenti lo stesso batch
            // resterebbe contabilizzato due volte — una per la prenotazione
            // larga (target + cella) e una per l'occupazione reale — e una
            // quota stretta fallirebbe su un batch che ci sta.
            drop(memory_lease);
            if violations.is_empty() {
                let spool = match self.spool.as_mut() {
                    Some(spool) => spool,
                    None => self
                        .spool
                        .insert(StagedSpool::new(batch.schema(), self.budget.clone())),
                };
                spool.push(batch, bytes).map_err(|error| {
                    terminal_scan_error(error, &violations, self.physical_row_indices_attestable)
                })?;
            } else {
                // Non è più possibile esporre alcun prefisso accepted. Si
                // continua il drain soltanto per completare i conteggi, e lo
                // spool rilascia subito quota di memoria e batch.
                if let Some(spool) = self.spool.as_mut() {
                    spool.clear();
                }
            }
            if matches!(self.scope, ReadScope::AcceptedRows(limit) if self.rows_scanned >= limit) {
                return if violations.is_empty() {
                    self.seal_spool()
                } else {
                    Err(violations.into_error(false, Some("read_scope_row_limit_reached")))
                };
            }
        }
        if violations.is_empty() {
            self.seal_spool()
        } else {
            Err(violations.into_error(true, None))
        }
    }

    /// Sigilla lo spool: da qui in poi si legge soltanto. Separare le due fasi
    /// e' cio' che rende l'atomicita' operativa verificabile invece che
    /// sperata — nessun batch puo' entrare dopo che il primo e' uscito.
    fn seal_spool(&mut self) -> Result<()> {
        self.spool.as_mut().map_or(Ok(()), StagedSpool::seal)
    }
}

const READ_DIAGNOSTIC_EXAMPLES_LIMIT: u64 = 64;

fn bounded_reservation(budget: &ResourceBudget, kind: ResourceKind, batch_cap: u64) -> u64 {
    budget.remaining(kind).min(batch_cap.max(1))
}

impl LayerReader for BudgetedReader {
    fn contract(&self) -> &LayerContract {
        self.inner.contract()
    }

    fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        if let Some(error) = &self.terminal_error {
            return Err(error.clone());
        }
        if let Err(error) = super::check_cancelled(&self.cancellation, ErrorPhase::Read) {
            self.spool = None;
            self.terminal_error = Some(error.clone());
            return Err(error);
        }
        if !self.drained {
            self.drained = true;
            if let Err(error) = self.drain_operation() {
                self.spool = None;
                self.terminal_error = Some(error.clone());
                return Err(error);
            }
        }
        match self.spool.as_mut() {
            None => Ok(None),
            Some(spool) => match spool.next_batch() {
                Ok(batch) => Ok(batch),
                Err(error) => {
                    // Un errore di replay dopo la validazione e' terminale e
                    // tipizzato: il consumer non deve poter proseguire su uno
                    // spool che non sa piu' rileggere (INV-8).
                    self.spool = None;
                    self.terminal_error = Some(error.clone());
                    Err(error)
                }
            },
        }
    }

    fn loss_report(&self) -> LossReport {
        reader_loss(self.inner.as_ref())
    }
}

#[cfg(test)]
fn validate_read_batch(
    contract: &LayerContract,
    batch: &RecordBatch,
    row_offset: u64,
    physical_row_indices_attestable: bool,
) -> Result<()> {
    let violations = collect_read_violations(contract, batch, row_offset)?;
    if violations.is_empty() {
        Ok(())
    } else {
        Err(read_rejection_error(
            violations,
            physical_row_indices_attestable,
            true,
            None,
        ))
    }
}

// Sequenza lineare di controlli, uno per vincolo del contratto di lettura: la
// lunghezza e' nel numero di vincoli, non in complessita' logica.
#[allow(clippy::too_many_lines)]
fn collect_read_violations(
    contract: &LayerContract,
    batch: &RecordBatch,
    row_offset: u64,
) -> Result<BTreeMap<u64, (&'static str, String)>> {
    if !read_schemas_are_compatible(batch.schema().as_ref(), contract.contract.schema.as_ref()) {
        return Err(read_schema_mismatch());
    }
    let mut violations = BTreeMap::<u64, (&'static str, String)>::new();
    for (column_index, field) in batch.schema().fields().iter().enumerate() {
        if field.is_nullable() {
            continue;
        }
        let array = batch.column(column_index);
        for row in 0..batch.num_rows() {
            if array.is_null(row) {
                let index = physical_index(row_offset, row)?;
                violations
                    .entry(index)
                    .or_insert_with(|| ("contract.nullability", field.name().to_owned()));
            }
        }
    }
    if let Some(geometry) = &contract.contract.geometry {
        let index = usize::try_from(geometry.field_id.0).map_err(|_| {
            PlenoraIoError::Schema("field_id geometrico fuori intervallo".to_owned())
        })?;
        // Finding #1 review 2026-08-15: `RecordBatch::column` panica su OOB.
        // Il driver IPC verifica ora `field_id` contro la posizione fisica
        // dello schema all'`open`, ma la barriera runtime qui e' comunque
        // necessaria: batch prodotti da driver che non applicano la stessa
        // verifica, batch materializzati in test, o schemi che divergono da
        // quelli attesi dal contratto non devono terminare il processo.
        let array = batch.columns().get(index).ok_or_else(|| {
            PlenoraIoError::Schema(format!(
                "field_id geometrico {index} fuori dai {} campi del batch",
                batch.num_columns()
            ))
        })?;
        let limits = WkbLimits::default();
        let mut inspect = |row: usize, bytes: Option<&[u8]>| -> Result<()> {
            let source_index = physical_index(row_offset, row)?;
            if violations.contains_key(&source_index) {
                return Ok(());
            }
            let Some(bytes) = bytes else {
                if !geometry.nullable {
                    violations.insert(
                        source_index,
                        ("contract.nullability", geometry.name.clone()),
                    );
                }
                return Ok(());
            };
            let Ok(inspected) = inspect_wkb(bytes, &limits) else {
                violations.insert(
                    source_index,
                    ("conversion.invalid_geometry", geometry.name.clone()),
                );
                return Ok(());
            };
            let cause = if geometry.dimensions != CoordinateDimensions::Unknown
                && (geometry.dimensions != inspected.dimensions
                    || !inspected.nested_dimensions_coherent)
            {
                Some("contract.coordinate_dimensions")
            } else if (geometry.encoding != GeometryEncoding::Ewkb && inspected.contains_srid)
                || (geometry.encoding == GeometryEncoding::Ewkb && inspected.srid != geometry.srid)
            {
                Some("contract.geometry_encoding")
            } else if !geometry.geometry_types.is_empty()
                && !geometry.geometry_types.contains(&inspected.geometry_type)
            {
                Some("contract.geometry_type")
            } else {
                None
            };
            if let Some(cause) = cause {
                violations.insert(source_index, (cause, geometry.name.clone()));
            }
            Ok(())
        };
        if let Some(values) = array.as_any().downcast_ref::<BinaryArray>() {
            for row in 0..values.len() {
                inspect(
                    row,
                    if values.is_null(row) {
                        None
                    } else {
                        Some(values.value(row))
                    },
                )?;
            }
        } else if let Some(values) = array.as_any().downcast_ref::<LargeBinaryArray>() {
            for row in 0..values.len() {
                inspect(
                    row,
                    if values.is_null(row) {
                        None
                    } else {
                        Some(values.value(row))
                    },
                )?;
            }
        } else {
            return Err(PlenoraIoError::new(
                ErrorCategory::Schema,
                ErrorPhase::Read,
                RemoteEffect::None,
                RetryDisposition::Never,
                "colonna geometrica letta non Binary/LargeBinary",
            ));
        }
    }
    if violations.is_empty() {
        return Ok(violations);
    }
    Ok(violations)
}

fn read_schemas_are_compatible(
    physical: &arrow_schema::Schema,
    effective: &arrow_schema::Schema,
) -> bool {
    physical.fields().len() == effective.fields().len()
        && physical
            .metadata()
            .iter()
            .all(|(key, value)| effective.metadata().get(key) == Some(value))
        && physical
            .fields()
            .iter()
            .zip(effective.fields())
            .all(|(physical, effective)| {
                physical.name() == effective.name()
                    && physical.data_type() == effective.data_type()
                    && physical.is_nullable() == effective.is_nullable()
                    && physical
                        .metadata()
                        .iter()
                        .all(|(key, value)| effective.metadata().get(key) == Some(value))
            })
}

fn with_effective_read_schema(contract: &LayerContract, batch: RecordBatch) -> Result<RecordBatch> {
    let effective = &contract.contract.schema;
    if batch.schema().as_ref() == effective.as_ref() {
        return Ok(batch);
    }
    if !read_schemas_are_compatible(batch.schema().as_ref(), effective.as_ref()) {
        return Err(read_schema_mismatch());
    }
    let options = arrow_array::RecordBatchOptions::new().with_row_count(Some(batch.num_rows()));
    RecordBatch::try_new_with_options(effective.clone(), batch.columns().to_vec(), &options)
        .map_err(|_| read_schema_mismatch())
}

fn read_schema_mismatch() -> PlenoraIoError {
    PlenoraIoError::new(
        ErrorCategory::Schema,
        ErrorPhase::Read,
        RemoteEffect::None,
        RetryDisposition::Never,
        "schema del batch letto diverso dal contratto effettivo dichiarato",
    )
}

fn physical_index(row_offset: u64, row: usize) -> Result<u64> {
    row_offset
        .checked_add(
            u64::try_from(row)
                .map_err(|_| PlenoraIoError::LimitExceeded("indice riga oltre u64".to_owned()))?,
        )
        .ok_or_else(|| PlenoraIoError::LimitExceeded("overflow nell'indice riga".to_owned()))
}

#[cfg(test)]
fn read_rejection_error(
    violations: BTreeMap<u64, (&'static str, String)>,
    physical_row_indices_attestable: bool,
    reached_eof: bool,
    knowledge_limit: Option<&'static str>,
) -> PlenoraIoError {
    let mut accumulator = ReadViolationAccumulator::new(
        physical_row_indices_attestable,
        READ_DIAGNOSTIC_EXAMPLES_LIMIT,
    );
    accumulator
        .record_all(violations)
        .expect("a materialized batch cannot exceed u64 diagnostic counts");
    accumulator.into_error(reached_eof, knowledge_limit)
}

struct ReadViolationAccumulator {
    physical_row_indices_attestable: bool,
    examples_limit: u64,
    observed_total: u64,
    counts: BTreeMap<String, u64>,
    examples: BTreeMap<u64, (&'static str, Option<String>)>,
    column_names_attestable: bool,
}

impl ReadViolationAccumulator {
    const fn new(physical_row_indices_attestable: bool, examples_limit: u64) -> Self {
        Self {
            physical_row_indices_attestable,
            examples_limit,
            observed_total: 0,
            counts: BTreeMap::new(),
            examples: BTreeMap::new(),
            column_names_attestable: true,
        }
    }

    const fn is_empty(&self) -> bool {
        self.observed_total == 0
    }

    fn record_all(&mut self, violations: BTreeMap<u64, (&'static str, String)>) -> Result<()> {
        for (source_index, (cause, column)) in violations {
            let column = RowDiagnosticColumn::attest(column);
            self.column_names_attestable &= column.is_attested();
            self.observed_total = self.observed_total.checked_add(1).ok_or_else(|| {
                PlenoraIoError::LimitExceeded(
                    "overflow nel conteggio delle righe diagnosticate".to_owned(),
                )
            })?;
            let cause_count = self.counts.entry(cause.to_owned()).or_default();
            *cause_count = cause_count.checked_add(1).ok_or_else(|| {
                PlenoraIoError::LimitExceeded(
                    "overflow nel conteggio delle cause diagnostiche".to_owned(),
                )
            })?;
            if self.physical_row_indices_attestable
                && self.examples.len() < saturating_usize(self.examples_limit)
            {
                self.examples
                    .entry(source_index)
                    .or_insert_with(|| (cause, column.into_option()));
            }
        }
        Ok(())
    }

    fn diagnostics(
        &self,
        reached_eof: bool,
        knowledge_limit: Option<&'static str>,
    ) -> RowDiagnostics {
        let observed_total = self.observed_total;
        if !self.physical_row_indices_attestable {
            let mut knowledge_limits = vec!["source_row_identity_unattestable".to_owned()];
            if !self.column_names_attestable {
                knowledge_limits.push(ROW_DIAGNOSTIC_COLUMN_UNATTESTABLE.to_owned());
            }
            if !reached_eof {
                knowledge_limits.push(
                    knowledge_limit
                        .unwrap_or("scan_terminated_before_eof")
                        .to_owned(),
                );
            }
            return RowDiagnostics {
                contract: ROW_DIAGNOSTICS_CONTRACT.to_owned(),
                scope: RowDiagnosticScope::Read,
                index_basis: ROW_DIAGNOSTICS_INDEX_BASIS.to_owned(),
                completeness: RowDiagnosticsCompleteness::Unknown,
                knowledge_limits: Some(knowledge_limits),
                observed_total,
                total: None,
                input_total: None,
                counts: self.counts.clone(),
                examples_limit: self.examples_limit,
                examples_truncated: false,
                examples: Vec::new(),
                diagnostic_state_counts: None,
                write_outcome: None,
            };
        }
        let examples = self
            .examples
            .iter()
            .map(|(source_index, (cause, column))| RowDiagnosticExample {
                source_index: *source_index,
                cause: (*cause).to_owned(),
                column: column.clone(),
                key: None,
                write_state: None,
            })
            .collect::<Vec<_>>();
        RowDiagnostics {
            contract: ROW_DIAGNOSTICS_CONTRACT.to_owned(),
            scope: RowDiagnosticScope::Read,
            index_basis: ROW_DIAGNOSTICS_INDEX_BASIS.to_owned(),
            completeness: if reached_eof && self.column_names_attestable {
                RowDiagnosticsCompleteness::Complete
            } else {
                RowDiagnosticsCompleteness::Partial
            },
            knowledge_limits: (!reached_eof || !self.column_names_attestable).then(|| {
                let mut limits = Vec::new();
                if !reached_eof {
                    limits.push(
                        knowledge_limit
                            .unwrap_or("scan_terminated_before_eof")
                            .to_owned(),
                    );
                }
                if !self.column_names_attestable {
                    limits.push(ROW_DIAGNOSTIC_COLUMN_UNATTESTABLE.to_owned());
                }
                limits
            }),
            observed_total,
            total: reached_eof.then_some(observed_total),
            input_total: None,
            counts: self.counts.clone(),
            examples_limit: self.examples_limit,
            examples_truncated: observed_total > examples.len() as u64,
            examples,
            diagnostic_state_counts: None,
            write_outcome: None,
        }
    }

    fn into_error(
        self,
        reached_eof: bool,
        knowledge_limit: Option<&'static str>,
    ) -> PlenoraIoError {
        let observed_total = self.observed_total;
        let diagnostics = self.diagnostics(reached_eof, knowledge_limit);
        let error = PlenoraIoError::new(
            ErrorCategory::DataMapping,
            ErrorPhase::Read,
            RemoteEffect::None,
            RetryDisposition::Never,
            format!("{observed_total} righe lette non conformi al contratto dichiarato"),
        );
        error.with_row_diagnostics(diagnostics)
    }
}

fn terminal_scan_error(
    mut error: PlenoraIoError,
    violations: &ReadViolationAccumulator,
    physical_row_indices_attestable: bool,
) -> PlenoraIoError {
    if violations.is_empty() {
        return error;
    }
    debug_assert_eq!(
        violations.physical_row_indices_attestable,
        physical_row_indices_attestable
    );
    let knowledge_limit = if error.category == ErrorCategory::Cancelled {
        "scan_cancelled_before_eof"
    } else if error.category == ErrorCategory::Timeout {
        "scan_deadline_exceeded_before_eof"
    } else {
        "scan_terminated_before_eof"
    };
    let common = violations.diagnostics(false, Some(knowledge_limit));
    error.row_diagnostics = Some(Box::new(match error.row_diagnostics.take() {
        Some(driver) => {
            merge_interrupted_read_diagnostics(common.clone(), *driver).unwrap_or_else(|| {
                let mut fallback = common;
                let limits = fallback.knowledge_limits.get_or_insert_default();
                if !limits
                    .iter()
                    .any(|value| value == "driver_row_diagnostics_invalid")
                {
                    limits.push("driver_row_diagnostics_invalid".to_owned());
                    limits.sort();
                }
                fallback.completeness = if physical_row_indices_attestable {
                    RowDiagnosticsCompleteness::Partial
                } else {
                    RowDiagnosticsCompleteness::Unknown
                };
                fallback.total = None;
                fallback
            })
        }
        None => common,
    }));
    error
}

fn merge_interrupted_read_diagnostics(
    common: RowDiagnostics,
    driver: RowDiagnostics,
) -> Option<RowDiagnostics> {
    if common.validate().is_err()
        || driver.validate().is_err()
        || common.scope != RowDiagnosticScope::Read
        || driver.scope != RowDiagnosticScope::Read
        || common.contract != driver.contract
        || common.index_basis != driver.index_basis
    {
        return None;
    }
    let observed_total = common.observed_total.checked_add(driver.observed_total)?;
    let mut counts = common.counts;
    for (cause, count) in driver.counts {
        let merged = counts
            .get(&cause)
            .copied()
            .unwrap_or(0)
            .checked_add(count)?;
        counts.insert(cause, merged);
    }
    let examples_limit = common
        .examples_limit
        .max(driver.examples_limit)
        .min(READ_DIAGNOSTIC_EXAMPLES_LIMIT);
    let unknown = common.completeness == RowDiagnosticsCompleteness::Unknown
        || driver.completeness == RowDiagnosticsCompleteness::Unknown;
    let mut examples_by_index = BTreeMap::new();
    if !unknown {
        for example in common.examples.into_iter().chain(driver.examples) {
            let has_capacity = examples_by_index.len() < examples_limit as usize;
            match examples_by_index.entry(example.source_index) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    if has_capacity {
                        entry.insert(example);
                    }
                }
                std::collections::btree_map::Entry::Occupied(entry) => {
                    if entry.get() != &example {
                        return None;
                    }
                }
            }
        }
    }
    let examples = examples_by_index.into_values().collect::<Vec<_>>();
    let mut knowledge_limits = BTreeSet::<String>::new();
    for limit in common
        .knowledge_limits
        .into_iter()
        .flatten()
        .chain(driver.knowledge_limits.into_iter().flatten())
    {
        knowledge_limits.insert(limit);
    }
    let diagnostics = RowDiagnostics {
        contract: common.contract,
        scope: RowDiagnosticScope::Read,
        index_basis: common.index_basis,
        completeness: if unknown {
            RowDiagnosticsCompleteness::Unknown
        } else {
            RowDiagnosticsCompleteness::Partial
        },
        knowledge_limits: Some(knowledge_limits.into_iter().collect()),
        observed_total,
        total: None,
        input_total: None,
        counts,
        examples_limit,
        examples_truncated: observed_total > examples.len() as u64
            && examples.len() as u64 == examples_limit,
        examples,
        diagnostic_state_counts: None,
        write_outcome: None,
    };
    diagnostics.validate().ok().map(|()| diagnostics)
}

fn geometry_components(
    contract: &LayerContract,
    batch: &RecordBatch,
    budget: &ResourceBudget,
) -> Result<u64> {
    let Some(geometry) = &contract.contract.geometry else {
        return Ok(0);
    };
    let Some(index) = batch
        .schema()
        .fields()
        .iter()
        .position(|field| field.name() == &geometry.name)
    else {
        // Una projection tabellare può escludere legittimamente la geometria.
        return Ok(0);
    };
    let limits = WkbLimits {
        max_cell_bytes: saturating_usize(budget.limits().cell_bytes),
        max_components: saturating_usize(budget.remaining(ResourceKind::GeometryComponents)),
        max_depth: saturating_usize(budget.limits().nesting_depth),
    };
    let array = batch.column(index);
    let mut total = 0_u64;
    let mut inspect = |bytes: &[u8]| -> Result<()> {
        let components = u64::try_from(inspect_wkb(bytes, &limits)?.components).map_err(|_| {
            PlenoraIoError::LimitExceeded("geometria oltre il conteggio supportato".to_owned())
        })?;
        total = total.checked_add(components).ok_or_else(|| {
            PlenoraIoError::LimitExceeded(
                "overflow nel conteggio dei componenti geometrici".to_owned(),
            )
        })?;
        Ok(())
    };
    if let Some(values) = array.as_any().downcast_ref::<BinaryArray>() {
        for row in 0..values.len() {
            if !values.is_null(row) {
                inspect(values.value(row))?;
            }
        }
        return Ok(total);
    }
    if let Some(values) = array.as_any().downcast_ref::<LargeBinaryArray>() {
        for row in 0..values.len() {
            if !values.is_null(row) {
                inspect(values.value(row))?;
            }
        }
        return Ok(total);
    }
    Err(PlenoraIoError::LimitExceeded(
        "colonna geometrica non binaria nel reader budgeted".to_owned(),
    ))
}

/// Adatta i batch prodotti da un reader al target comune di ADR-IO 6.
///
/// Lo slicing Arrow non copia i buffer e quindi limita la cardinalità esposta,
/// non la memoria già allocata dal reader sottostante.
#[must_use]
pub fn with_batch_target(
    reader: Box<dyn LayerReader>,
    target: BatchTarget,
    cancellation: CancellationToken,
) -> Box<dyn LayerReader> {
    let rows_per_batch = effective_batch_rows(reader.contract().contract.schema.as_ref(), target);
    Box::new(BatchTargetReader {
        inner: reader,
        rows_per_batch,
        pending: None,
        cancellation,
        terminal_error: None,
    })
}

/// Collega il token R11 a un reader e rilascia immediatamente il reader
/// sottostante quando la cancellazione viene osservata.
#[must_use]
pub fn with_cancellation(
    reader: Box<dyn LayerReader>,
    cancellation: CancellationToken,
) -> Box<dyn LayerReader> {
    let loss = reader_loss(reader.as_ref());
    Box::new(CancellationReader {
        contract: reader.contract().clone(),
        inner: Some(reader),
        loss,
        cancellation,
        terminal_error: None,
    })
}

struct CancellationReader {
    contract: LayerContract,
    inner: Option<Box<dyn LayerReader>>,
    loss: LossReport,
    cancellation: CancellationToken,
    terminal_error: Option<PlenoraIoError>,
}

impl LayerReader for CancellationReader {
    fn contract(&self) -> &LayerContract {
        &self.contract
    }

    fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        if let Some(error) = &self.terminal_error {
            return Err(error.clone());
        }
        if let Err(error) = super::check_cancelled(&self.cancellation, ErrorPhase::Read) {
            self.inner = None;
            self.terminal_error = Some(error.clone());
            return Err(error);
        }
        let Some(inner) = self.inner.as_mut() else {
            return Ok(None);
        };
        let result = inner.next_batch();
        self.loss = reader_loss(inner.as_ref());
        if !matches!(result, Ok(Some(_))) {
            self.inner = None;
        }
        if let Err(error) = &result {
            self.terminal_error = Some(error.clone());
        }
        result
    }

    fn loss_report(&self) -> LossReport {
        self.loss.clone()
    }
}

struct BatchTargetReader {
    inner: Box<dyn LayerReader>,
    rows_per_batch: usize,
    pending: Option<(RecordBatch, usize)>,
    cancellation: CancellationToken,
    terminal_error: Option<PlenoraIoError>,
}

impl LayerReader for BatchTargetReader {
    fn contract(&self) -> &LayerContract {
        self.inner.contract()
    }

    fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        if let Some(error) = &self.terminal_error {
            return Err(error.clone());
        }
        if let Err(error) = super::check_cancelled(&self.cancellation, ErrorPhase::Read) {
            self.pending = None;
            self.terminal_error = Some(error.clone());
            return Err(error);
        }
        loop {
            if let Some((batch, offset)) = self.pending.take() {
                let remaining = batch.num_rows() - offset;
                let take = remaining.min(self.rows_per_batch);
                let output = batch.slice(offset, take);
                if take < remaining {
                    self.pending = Some((batch, offset + take));
                }
                return Ok(Some(output));
            }

            let next = self.inner.next_batch();
            let Some(batch) = next.inspect_err(|error| {
                self.pending = None;
                self.terminal_error = Some(error.clone());
            })?
            else {
                return Ok(None);
            };
            if batch.num_rows() <= self.rows_per_batch {
                return Ok(Some(batch));
            }
            self.pending = Some((batch, 0));
        }
    }

    fn loss_report(&self) -> LossReport {
        reader_loss(self.inner.as_ref())
    }
}

fn reader_loss(reader: &dyn LayerReader) -> LossReport {
    let mut loss = reader.loss_report();
    declare_crs_inconsistency(reader.contract(), &mut loss);
    loss
}

/// Enforcement runtime di `ReaderConcurrency::SingleActiveReader` (ADR-IO 1).
/// Il lease è per-handle: viene rilasciato a EOF/errore o al drop anticipato.
#[derive(Clone)]
pub struct SingleReaderGate {
    active: Arc<AtomicBool>,
    driver: &'static str,
}

impl SingleReaderGate {
    #[must_use]
    pub fn new(driver: &'static str) -> Self {
        Self {
            active: Arc::new(AtomicBool::new(false)),
            driver,
        }
    }

    /// Apre l'unico reader ammesso dal gate, prendendone il lease.
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::reader_busy`] se un reader è già attivo;
    /// altrimenti propaga l'errore della closure `create`.
    pub fn open<F>(&self, layer: LayerId, create: F) -> Result<Box<dyn LayerReader>>
    where
        F: FnOnce() -> Result<Box<dyn LayerReader>>,
    {
        self.active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| PlenoraIoError::reader_busy(self.driver, layer.0))?;

        let lease = ReaderLease {
            active: self.active.clone(),
            released: false,
        };
        match create() {
            Ok(inner) => Ok(Box::new(SingleActiveLayerReader { inner, lease })),
            Err(error) => {
                drop(lease);
                Err(error)
            }
        }
    }
}

struct ReaderLease {
    active: Arc<AtomicBool>,
    released: bool,
}

impl ReaderLease {
    fn release(&mut self) {
        if !self.released {
            self.active.store(false, Ordering::Release);
            self.released = true;
        }
    }
}

impl Drop for ReaderLease {
    fn drop(&mut self) {
        self.release();
    }
}

struct SingleActiveLayerReader {
    inner: Box<dyn LayerReader>,
    lease: ReaderLease,
}

impl LayerReader for SingleActiveLayerReader {
    fn contract(&self) -> &LayerContract {
        self.inner.contract()
    }

    fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        let result = self.inner.next_batch();
        if !matches!(result, Ok(Some(_))) {
            self.lease.release();
        }
        result
    }

    fn loss_report(&self) -> LossReport {
        self.inner.loss_report()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use std::sync::Arc;

    use arrow_array::{new_empty_array, BinaryArray, Int64Array, UInt8Array};
    use arrow_schema::{DataType, Field, Schema};
    use plenora_io_model::contract::{DataContract, FieldId, GeometryColumnContract, LayerId};
    use plenora_io_model::crs::CrsResolution;
    use plenora_io_model::resource::ResourceLimits;

    use super::*;

    struct OneBatchReader {
        contract: LayerContract,
        batch: Option<RecordBatch>,
    }

    impl OneBatchReader {
        fn new(values: Vec<i64>) -> Self {
            let schema = Arc::new(Schema::new(vec![Field::new(
                "value",
                DataType::Int64,
                false,
            )]));
            let batch =
                RecordBatch::try_new(schema.clone(), vec![Arc::new(Int64Array::from(values))])
                    .unwrap();
            Self {
                contract: LayerContract {
                    id: LayerId(0),
                    name: "values".to_owned(),
                    contract: DataContract {
                        schema,
                        geometry: None,
                    },
                },
                batch: Some(batch),
            }
        }
    }

    impl LayerReader for OneBatchReader {
        fn contract(&self) -> &LayerContract {
            &self.contract
        }

        fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
            Ok(self.batch.take())
        }
    }

    struct SequenceReader {
        contract: LayerContract,
        events: VecDeque<Result<Option<RecordBatch>>>,
    }

    impl LayerReader for SequenceReader {
        fn contract(&self) -> &LayerContract {
            &self.contract
        }

        fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
            self.events.pop_front().unwrap_or(Ok(None))
        }
    }

    fn validating_contract() -> LayerContract {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "geometry",
            DataType::Binary,
            true,
        )]));
        LayerContract {
            id: LayerId(0),
            name: "values".to_owned(),
            contract: DataContract::new(
                schema,
                Some(GeometryColumnContract::wkb_xy(
                    FieldId(0),
                    "geometry",
                    CrsResolution::Missing,
                    true,
                )),
            ),
        }
    }

    fn geometry_batch(contract: &LayerContract, valid: &[bool]) -> RecordBatch {
        const VALID_POINT: &[u8] = &[
            1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        const INVALID: &[u8] = &[1, 1, 0];
        let values = valid
            .iter()
            .map(|is_valid| Some(if *is_valid { VALID_POINT } else { INVALID }))
            .collect::<Vec<_>>();
        RecordBatch::try_new(
            contract.contract.schema.clone(),
            vec![Arc::new(BinaryArray::from(values))],
        )
        .unwrap()
    }

    fn budgeted_sequence(events: VecDeque<Result<Option<RecordBatch>>>) -> BudgetedReader {
        budgeted_sequence_with_scope(events, ReadScope::Complete)
    }

    fn budgeted_sequence_with_budget(
        events: VecDeque<Result<Option<RecordBatch>>>,
        budget: ResourceBudget,
    ) -> BudgetedReader {
        let operation = budget
            .try_lease(ResourceKind::ConcurrentOperations, 1)
            .unwrap();
        BudgetedReader::new(
            Box::new(SequenceReader {
                contract: validating_contract(),
                events,
            }),
            budget,
            true,
            CancellationToken::default(),
            BatchTarget::default(),
            ReadScope::Complete,
            operation,
        )
        .unwrap()
    }

    /// Il caso che ADR-IO 7 esiste per risolvere: prima dello spool i batch
    /// verificati restavano tutti in RAM, quindi un dataset piu' grande della
    /// quota di memoria falliva `LimitExceeded` anche se ogni singolo batch ci
    /// stava comodamente.
    #[test]
    fn dataset_over_memory_bytes_succeeds_via_spool() {
        let contract = validating_contract();
        // 40 batch da ~21 byte di payload ciascuno con una quota di memoria
        // di 4 KiB: la somma supera la quota, il singolo batch no.
        let eventi: VecDeque<Result<Option<RecordBatch>>> = (0..40)
            .map(|_| Ok(Some(geometry_batch(&contract, &[true; 8]))))
            .collect();
        let budget = ResourceBudget::new(ResourceLimits {
            memory_bytes: 4_096,
            cell_bytes: 1_024,
            ..ResourceLimits::default()
        })
        .unwrap();
        let mut reader = budgeted_sequence_with_budget(eventi, budget.clone());

        let mut consegnati = 0_usize;
        while let Some(batch) = reader.next_batch().unwrap() {
            assert_eq!(batch.num_rows(), 8);
            consegnati += 1;
        }
        assert_eq!(consegnati, 40);
        assert_eq!(
            budget.remaining(ResourceKind::MemoryBytes),
            4_096,
            "a fine operazione nessuna quota di memoria deve restare trattenuta"
        );
    }

    /// L0.3: la memoria dei batch bufferizzati non e' consumo definitivo. Con
    /// il vecchio `commit` la quota residua calava monotonicamente e non
    /// tornava piu'.
    #[test]
    fn buffered_batches_do_not_permanently_consume_memory() {
        let contract = validating_contract();
        let eventi: VecDeque<Result<Option<RecordBatch>>> = (0..6)
            .map(|_| Ok(Some(geometry_batch(&contract, &[true; 4]))))
            .collect();
        let budget = ResourceBudget::default();
        let iniziale = budget.remaining(ResourceKind::MemoryBytes);
        let mut reader = budgeted_sequence_with_budget(eventi, budget.clone());

        while reader.next_batch().unwrap().is_some() {}
        assert_eq!(
            budget.remaining(ResourceKind::MemoryBytes),
            iniziale,
            "la memoria deve tornare interamente al termine dell'operazione"
        );
    }

    /// Un dataset di N righe letto con quota esattamente N deve riuscire.
    /// Prima la quota si esauriva sull'ultimo batch e il giro successivo,
    /// fatto solo per scoprire la fine della sorgente, trasformava l'EOF in
    /// un `LimitExceeded`.
    #[test]
    fn reader_of_n_rows_with_max_rows_n_succeeds() {
        let contract = validating_contract();
        let eventi: VecDeque<Result<Option<RecordBatch>>> = (0..4)
            .map(|_| Ok(Some(geometry_batch(&contract, &[true; 5]))))
            .collect();
        let budget = ResourceBudget::new(ResourceLimits {
            rows: 20,
            ..ResourceLimits::default()
        })
        .unwrap();
        let mut reader = budgeted_sequence_with_budget(eventi, budget);
        let mut righe = 0_usize;
        while let Some(batch) = reader.next_batch().unwrap() {
            righe += batch.num_rows();
        }
        assert_eq!(righe, 20);
    }

    /// Una riga oltre la quota deve continuare a fallire: la correzione
    /// dell'EOF non deve allentare il limite.
    #[test]
    fn reader_of_n_plus_one_rows_with_max_rows_n_still_fails() {
        let contract = validating_contract();
        let eventi: VecDeque<Result<Option<RecordBatch>>> = (0..5)
            .map(|_| Ok(Some(geometry_batch(&contract, &[true; 5]))))
            .collect();
        let budget = ResourceBudget::new(ResourceLimits {
            rows: 20,
            ..ResourceLimits::default()
        })
        .unwrap();
        let mut reader = budgeted_sequence_with_budget(eventi, budget);
        let mut esito = Ok(());
        while let Ok(Some(_)) = reader.next_batch() {}
        if let Err(error) = reader.next_batch() {
            esito = Err(error);
        }
        assert!(esito.is_err(), "la riga oltre quota deve fallire");
    }

    /// Criterio di uscita di M2: gli assi che lo spool non governa devono
    /// comportarsi esattamente come prima, e nessuno deve essere contato due
    /// volte dal percorso nuovo.
    #[test]
    fn limit_parity_pre_and_post_m2() {
        let contract = validating_contract();
        let eventi = || -> VecDeque<Result<Option<RecordBatch>>> {
            (0..4)
                .map(|_| Ok(Some(geometry_batch(&contract, &[true; 5]))))
                .collect()
        };

        // `Rows`: 20 righe con quota 20 passano, con quota 19 no.
        let stretto = ResourceBudget::new(ResourceLimits {
            rows: 19,
            ..ResourceLimits::default()
        })
        .unwrap();
        let mut reader = budgeted_sequence_with_budget(eventi(), stretto);
        assert!(
            reader.next_batch().is_err(),
            "una riga in meno di quota deve ancora fallire"
        );

        let esatto = ResourceBudget::new(ResourceLimits {
            rows: 20,
            ..ResourceLimits::default()
        })
        .unwrap();
        let mut reader = budgeted_sequence_with_budget(eventi(), esatto.clone());
        let mut righe = 0_usize;
        while let Some(batch) = reader.next_batch().unwrap() {
            righe += batch.num_rows();
        }
        assert_eq!(
            righe, 20,
            "la quota esatta deve bastare: nessun doppio conteggio"
        );
        assert_eq!(
            esatto.remaining(ResourceKind::Rows),
            0,
            "le righe restano cumulative, consumate una volta sola"
        );

        // `OutputBytes` resta cumulativo e consumato, non restituito.
        let output = ResourceBudget::default();
        let prima = output.remaining(ResourceKind::OutputBytes);
        let mut reader = budgeted_sequence_with_budget(eventi(), output.clone());
        while reader.next_batch().unwrap().is_some() {}
        assert!(
            output.remaining(ResourceKind::OutputBytes) < prima,
            "OutputBytes e' quota consumata, non occupazione trattenuta"
        );

        // `ConcurrentOperations` resta governato dal solo modello legacy.
        let concorrenza = ResourceBudget::default();
        let prima = concorrenza.remaining(ResourceKind::ConcurrentOperations);
        let reader = budgeted_sequence_with_budget(eventi(), concorrenza.clone());
        assert_eq!(
            concorrenza.remaining(ResourceKind::ConcurrentOperations),
            prima - 1,
            "una sola operazione, contata una sola volta"
        );
        drop(reader);
        assert_eq!(
            concorrenza.remaining(ResourceKind::ConcurrentOperations),
            prima
        );
    }

    fn budgeted_sequence_with_scope(
        events: VecDeque<Result<Option<RecordBatch>>>,
        scope: ReadScope,
    ) -> BudgetedReader {
        let budget = ResourceBudget::default();
        let operation = budget
            .try_lease(ResourceKind::ConcurrentOperations, 1)
            .unwrap();
        BudgetedReader::new(
            Box::new(SequenceReader {
                contract: validating_contract(),
                events,
            }),
            budget,
            true,
            CancellationToken::default(),
            BatchTarget::default(),
            scope,
            operation,
        )
        .unwrap()
    }

    #[test]
    fn accepted_rows_scope_stops_before_an_unobserved_tail_error() {
        let contract = validating_contract();
        let mut reader = budgeted_sequence_with_scope(
            VecDeque::from([
                Ok(Some(geometry_batch(&contract, &[true; 12]))),
                Err(PlenoraIoError::LimitExceeded(
                    "la coda non doveva essere letta".to_owned(),
                )),
            ]),
            ReadScope::AcceptedRows(10),
        );

        assert_eq!(reader.next_batch().unwrap().unwrap().num_rows(), 12);
        assert!(reader.next_batch().unwrap().is_none());
    }

    struct CountingReader {
        contract: LayerContract,
        calls: Arc<AtomicUsize>,
    }

    impl LayerReader for CountingReader {
        fn contract(&self) -> &LayerContract {
            &self.contract
        }

        fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
            self.calls.fetch_add(1, AtomicOrdering::SeqCst);
            Err(PlenoraIoError::format("test", "invalid tail observed"))
        }
    }

    #[test]
    fn accepted_rows_zero_never_polls_the_inner_reader() {
        let budget = ResourceBudget::default();
        let operation = budget
            .try_lease(ResourceKind::ConcurrentOperations, 1)
            .unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let mut reader = BudgetedReader::new(
            Box::new(CountingReader {
                contract: validating_contract(),
                calls: calls.clone(),
            }),
            budget,
            true,
            CancellationToken::default(),
            BatchTarget::default(),
            ReadScope::AcceptedRows(0),
            operation,
        )
        .unwrap();

        assert!(reader.next_batch().unwrap().is_none());
        assert!(reader.next_batch().unwrap().is_none());
        assert_eq!(calls.load(AtomicOrdering::SeqCst), 0);
    }

    #[test]
    fn accepted_rows_scope_reports_prefix_rejections_as_partial() {
        let contract = validating_contract();
        let mut reader = budgeted_sequence_with_scope(
            VecDeque::from([
                Ok(Some(geometry_batch(&contract, &[true, false, true]))),
                Ok(Some(geometry_batch(&contract, &[false]))),
                Ok(None),
            ]),
            ReadScope::AcceptedRows(2),
        );

        let error = reader.next_batch().unwrap_err();
        let diagnostics = error.row_diagnostics.as_deref().unwrap();
        assert_eq!(
            diagnostics.completeness,
            RowDiagnosticsCompleteness::Partial
        );
        assert_eq!(diagnostics.observed_total, 1);
        assert_eq!(diagnostics.examples[0].source_index, 1);
        assert_eq!(
            diagnostics.knowledge_limits.as_deref(),
            Some(["read_scope_row_limit_reached".to_owned()].as_slice())
        );
        assert!(diagnostics.validate().is_ok());
    }

    #[test]
    fn standalone_reader_adapters_keep_underlying_errors_sticky() {
        fn failing_reader() -> Box<dyn LayerReader> {
            Box::new(SequenceReader {
                contract: validating_contract(),
                events: VecDeque::from([Err(PlenoraIoError::format("test", "boom"))]),
            })
        }

        let mut cancellation = with_cancellation(failing_reader(), CancellationToken::default());
        let first = cancellation.next_batch().unwrap_err();
        assert_eq!(cancellation.next_batch().unwrap_err(), first);

        let mut targeted = with_batch_target(
            failing_reader(),
            BatchTarget::default(),
            CancellationToken::default(),
        );
        let first = targeted.next_batch().unwrap_err();
        assert_eq!(targeted.next_batch().unwrap_err(), first);
    }

    #[test]
    fn row_quota_is_shared_across_independent_readers() {
        let budget = ResourceBudget::new(ResourceLimits {
            rows: 3,
            columns: 10,
            memory_bytes: 1024 * 1024,
            cell_bytes: 1024,
            output_bytes: 1024 * 1024,
            ..ResourceLimits::default()
        })
        .unwrap();
        let first_operation = budget
            .try_lease(ResourceKind::ConcurrentOperations, 1)
            .unwrap();
        let mut first = BudgetedReader::new(
            Box::new(OneBatchReader::new(vec![1, 2])),
            budget.clone(),
            true,
            CancellationToken::default(),
            BatchTarget::default(),
            ReadScope::Complete,
            first_operation,
        )
        .unwrap();
        assert_eq!(first.next_batch().unwrap().unwrap().num_rows(), 2);
        drop(first);

        let second_operation = budget
            .try_lease(ResourceKind::ConcurrentOperations, 1)
            .unwrap();
        let mut second = BudgetedReader::new(
            Box::new(OneBatchReader::new(vec![3, 4])),
            budget,
            true,
            CancellationToken::default(),
            BatchTarget::default(),
            ReadScope::Complete,
            second_operation,
        )
        .unwrap();
        let error = second.next_batch().unwrap_err();
        assert_eq!(
            error.category,
            plenora_io_model::ErrorCategory::ResourceLimit
        );
    }

    #[test]
    fn read_validation_reports_only_attestable_physical_indices() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "geometry",
            DataType::Binary,
            true,
        )]));
        let contract = LayerContract {
            id: LayerId(0),
            name: "invalid".to_owned(),
            contract: DataContract::new(
                schema,
                Some(GeometryColumnContract::wkb_xy(
                    FieldId(0),
                    "geometry",
                    CrsResolution::Missing,
                    true,
                )),
            ),
        };
        let batch = RecordBatch::try_new(
            contract.contract.schema.clone(),
            vec![Arc::new(BinaryArray::from(vec![
                Some(&[1_u8, 1, 0][..]),
                Some(&[1_u8, 1, 0][..]),
            ]))],
        )
        .unwrap();

        let attestable = validate_read_batch(&contract, &batch, 10, true).unwrap_err();
        let diagnostics = attestable.row_diagnostics.as_deref().unwrap();
        assert_eq!(diagnostics.observed_total, 2);
        assert_eq!(diagnostics.examples[0].source_index, 10);
        assert_eq!(diagnostics.examples[1].source_index, 11);
        assert!(diagnostics.validate().is_ok());

        let non_attestable = validate_read_batch(&contract, &batch, 10, false).unwrap_err();
        let diagnostics = non_attestable.row_diagnostics.as_deref().unwrap();
        assert_eq!(
            diagnostics.completeness,
            RowDiagnosticsCompleteness::Unknown
        );
        assert_eq!(diagnostics.observed_total, 2);
        assert_eq!(
            diagnostics.counts.get("conversion.invalid_geometry"),
            Some(&2)
        );
        assert!(diagnostics.examples.is_empty());
        assert!(diagnostics.validate().is_ok());
    }

    #[test]
    fn read_schema_validation_rejects_structural_and_physical_metadata_drift() {
        use std::collections::HashMap;

        let expected = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false).with_metadata(HashMap::from([(
                "producer.normative".to_owned(),
                "v1".to_owned(),
            )])),
            Field::new("geometry", DataType::Binary, true),
        ]));
        let contract = LayerContract {
            id: LayerId(0),
            name: "strict".to_owned(),
            contract: DataContract {
                schema: expected,
                geometry: None,
            },
        };
        let variants = [
            Schema::new(vec![Field::new("id", DataType::Int64, false)]),
            Schema::new(vec![
                Field::new("geometry", DataType::Binary, true),
                Field::new("id", DataType::Int64, false),
            ]),
            Schema::new(vec![
                Field::new("other", DataType::Int64, false),
                Field::new("geometry", DataType::Binary, true),
            ]),
            Schema::new(vec![
                Field::new("id", DataType::UInt8, false),
                Field::new("geometry", DataType::Binary, true),
            ]),
            Schema::new(vec![
                Field::new("id", DataType::Int64, true),
                Field::new("geometry", DataType::Binary, true),
            ]),
            Schema::new(vec![
                Field::new("id", DataType::Int64, false).with_metadata(HashMap::from([(
                    "producer.normative".to_owned(),
                    "v2".to_owned(),
                )])),
                Field::new("geometry", DataType::Binary, true),
            ]),
        ];

        for schema in variants {
            let schema = Arc::new(schema);
            let columns = schema
                .fields()
                .iter()
                .map(|field| new_empty_array(field.data_type()))
                .collect();
            let batch = RecordBatch::try_new(schema, columns).unwrap();
            let error = validate_read_batch(&contract, &batch, 0, true).unwrap_err();
            assert_eq!(error.category, ErrorCategory::Schema);
            assert_eq!(error.phase, ErrorPhase::Read);
            assert!(with_effective_read_schema(&contract, batch).is_err());
        }
    }

    #[test]
    fn effective_schema_retag_preserves_nonzero_rows_for_empty_projection() {
        use std::collections::HashMap;

        let physical = Arc::new(Schema::new_with_metadata(
            Vec::<Field>::new(),
            HashMap::from([("producer.normative".to_owned(), "v1".to_owned())]),
        ));
        let effective = Arc::new(Schema::new_with_metadata(
            Vec::<Field>::new(),
            HashMap::from([
                ("producer.normative".to_owned(), "v1".to_owned()),
                ("plenora.contract.version".to_owned(), "1".to_owned()),
            ]),
        ));
        let contract = LayerContract {
            id: LayerId(0),
            name: "empty-projection".to_owned(),
            contract: DataContract {
                schema: effective.clone(),
                geometry: None,
            },
        };
        let options = arrow_array::RecordBatchOptions::new().with_row_count(Some(2));
        let batch = RecordBatch::try_new_with_options(physical, Vec::new(), &options).unwrap();

        assert!(read_schemas_are_compatible(
            batch.schema().as_ref(),
            effective.as_ref()
        ));
        let retagged = with_effective_read_schema(&contract, batch).unwrap();
        assert_eq!(retagged.num_rows(), 2);
        assert_eq!(retagged.num_columns(), 0);
        assert_eq!(retagged.schema(), effective);
    }

    #[test]
    fn valid_prefix_is_not_exposed_when_a_late_batch_is_invalid() {
        let contract = validating_contract();
        let mut reader = budgeted_sequence(VecDeque::from([
            Ok(Some(geometry_batch(&contract, &[true, true]))),
            Ok(Some(geometry_batch(&contract, &[true, false]))),
            Ok(Some(geometry_batch(&contract, &[false, true]))),
            Ok(None),
        ]));

        let first = reader.next_batch().unwrap_err();
        let diagnostics = first.row_diagnostics.as_deref().unwrap();
        assert_eq!(
            diagnostics.completeness,
            RowDiagnosticsCompleteness::Complete
        );
        assert_eq!(diagnostics.observed_total, 2);
        assert_eq!(diagnostics.examples[0].source_index, 3);
        assert_eq!(diagnostics.examples[1].source_index, 4);
        assert!(
            reader.spool.is_none(),
            "una violazione non deve lasciare batch consegnabili"
        );

        let repeated = reader.next_batch().unwrap_err();
        assert_eq!(repeated, first);
    }

    #[test]
    fn interruption_after_rejection_preserves_partial_diagnostics() {
        let contract = validating_contract();
        let mut reader = budgeted_sequence(VecDeque::from([
            Ok(Some(geometry_batch(&contract, &[true, false]))),
            Err(PlenoraIoError::cancelled(ErrorPhase::Read, false)),
        ]));

        let error = reader.next_batch().unwrap_err();
        assert_eq!(error.category, ErrorCategory::Cancelled);
        assert_eq!(error.code, plenora_io_model::IoErrorCode::Cancelled);
        assert_eq!(error.phase, ErrorPhase::Read);
        assert_eq!(error.retry, RetryDisposition::Never);
        let diagnostics = error.row_diagnostics.as_deref().unwrap();
        assert_eq!(
            diagnostics.completeness,
            RowDiagnosticsCompleteness::Partial
        );
        assert_eq!(diagnostics.observed_total, 1);
        assert_eq!(diagnostics.examples[0].source_index, 1);
        assert_eq!(
            diagnostics.knowledge_limits.as_deref(),
            Some(["scan_cancelled_before_eof".to_owned()].as_slice())
        );
        assert_eq!(reader.next_batch().unwrap_err(), error);
    }

    #[test]
    fn terminal_driver_diagnostics_merge_with_common_read_rejections() {
        let contract = validating_contract();
        let driver_diagnostics = RowDiagnostics {
            contract: ROW_DIAGNOSTICS_CONTRACT.to_owned(),
            scope: RowDiagnosticScope::Read,
            index_basis: ROW_DIAGNOSTICS_INDEX_BASIS.to_owned(),
            completeness: RowDiagnosticsCompleteness::Partial,
            knowledge_limits: Some(vec!["driver_scan_interrupted".to_owned()]),
            observed_total: 1,
            total: None,
            input_total: None,
            counts: BTreeMap::from([("driver.invalid_attribute".to_owned(), 1)]),
            examples_limit: 64,
            examples_truncated: false,
            examples: vec![RowDiagnosticExample {
                source_index: 50,
                cause: "driver.invalid_attribute".to_owned(),
                column: Some("value".to_owned()),
                key: None,
                write_state: None,
            }],
            diagnostic_state_counts: None,
            write_outcome: None,
        };
        let terminal = PlenoraIoError::cancelled(ErrorPhase::Read, false)
            .with_row_diagnostics(driver_diagnostics);
        let mut reader = budgeted_sequence(VecDeque::from([
            Ok(Some(geometry_batch(&contract, &[true, false]))),
            Err(terminal),
        ]));

        let error = reader.next_batch().unwrap_err();
        assert_eq!(error.category, ErrorCategory::Cancelled);
        let diagnostics = error.row_diagnostics.as_deref().unwrap();
        assert_eq!(diagnostics.observed_total, 2);
        assert_eq!(diagnostics.counts["conversion.invalid_geometry"], 1);
        assert_eq!(diagnostics.counts["driver.invalid_attribute"], 1);
        assert_eq!(
            diagnostics
                .examples
                .iter()
                .map(|example| example.source_index)
                .collect::<Vec<_>>(),
            vec![1, 50]
        );
        assert_eq!(
            diagnostics.completeness,
            RowDiagnosticsCompleteness::Partial
        );
        assert_eq!(diagnostics.total, None);
        assert!(diagnostics
            .knowledge_limits
            .as_deref()
            .unwrap()
            .contains(&"scan_cancelled_before_eof".to_owned()));
        assert!(diagnostics.validate().is_ok());
    }

    #[test]
    fn invalid_driver_diagnostics_fail_closed_without_claiming_complete() {
        let contract = validating_contract();
        let mut invalid = read_rejection_error(
            BTreeMap::from([(50, ("driver.invalid_attribute", "value".to_owned()))]),
            true,
            true,
            None,
        )
        .row_diagnostics
        .unwrap();
        invalid
            .counts
            .insert("driver.invalid_attribute".to_owned(), 2);
        let terminal =
            PlenoraIoError::format("test", "driver failed").with_row_diagnostics(*invalid);
        let mut reader = budgeted_sequence(VecDeque::from([
            Ok(Some(geometry_batch(&contract, &[false]))),
            Err(terminal),
        ]));

        let error = reader.next_batch().unwrap_err();
        let diagnostics = error.row_diagnostics.as_deref().unwrap();
        assert_ne!(
            diagnostics.completeness,
            RowDiagnosticsCompleteness::Complete
        );
        assert_eq!(diagnostics.counts["conversion.invalid_geometry"], 1);
        assert!(diagnostics
            .knowledge_limits
            .as_deref()
            .unwrap()
            .contains(&"driver_row_diagnostics_invalid".to_owned()));
        assert!(diagnostics.validate().is_ok());
    }

    #[test]
    fn hostile_invalid_scan_keeps_only_the_bounded_sorted_examples() {
        let contract = validating_contract();
        let invalid = geometry_batch(&contract, &[false; 100]);
        let mut events = (0..100)
            .map(|_| Ok(Some(invalid.clone())))
            .collect::<VecDeque<_>>();
        events.push_back(Ok(None));
        let mut reader = budgeted_sequence(events);

        let error = reader.next_batch().unwrap_err();
        let diagnostics = error.row_diagnostics.as_deref().unwrap();
        assert_eq!(diagnostics.observed_total, 10_000);
        assert_eq!(diagnostics.counts["conversion.invalid_geometry"], 10_000);
        assert_eq!(diagnostics.examples.len(), 64);
        assert_eq!(diagnostics.examples.first().unwrap().source_index, 0);
        assert_eq!(diagnostics.examples.last().unwrap().source_index, 63);
        assert!(diagnostics.examples_truncated);
        assert!(diagnostics.validate().is_ok());
    }

    #[test]
    fn non_attestable_interruption_preserves_both_knowledge_limits() {
        let contract = validating_contract();
        let budget = ResourceBudget::default();
        let operation = budget
            .try_lease(ResourceKind::ConcurrentOperations, 1)
            .unwrap();
        let mut reader = BudgetedReader::new(
            Box::new(SequenceReader {
                contract: contract.clone(),
                events: VecDeque::from([
                    Ok(Some(geometry_batch(&contract, &[false]))),
                    Err(PlenoraIoError::cancelled(ErrorPhase::Read, false)),
                ]),
            }),
            budget,
            false,
            CancellationToken::default(),
            BatchTarget::default(),
            ReadScope::Complete,
            operation,
        )
        .unwrap();

        let error = reader.next_batch().unwrap_err();
        let diagnostics = error.row_diagnostics.as_deref().unwrap();
        assert_eq!(
            diagnostics.completeness,
            RowDiagnosticsCompleteness::Unknown
        );
        assert!(diagnostics.examples.is_empty());
        assert_eq!(
            diagnostics.knowledge_limits.as_deref(),
            Some(
                [
                    "source_row_identity_unattestable".to_owned(),
                    "scan_cancelled_before_eof".to_owned(),
                ]
                .as_slice()
            )
        );
    }

    #[test]
    fn later_validation_error_preserves_already_observed_diagnostics() {
        let contract = validating_contract();
        let bad_schema = Arc::new(Schema::new(vec![Field::new(
            "other",
            DataType::Binary,
            true,
        )]));
        let mismatched = RecordBatch::try_new(
            bad_schema,
            vec![Arc::new(BinaryArray::from(vec![Some(&[1_u8][..])]))],
        )
        .unwrap();
        let mut reader = budgeted_sequence(VecDeque::from([
            Ok(Some(geometry_batch(&contract, &[false]))),
            Ok(Some(mismatched)),
        ]));

        let error = reader.next_batch().unwrap_err();
        assert_eq!(error.category, ErrorCategory::Schema);
        let diagnostics = error.row_diagnostics.as_deref().unwrap();
        assert_eq!(diagnostics.observed_total, 1);
        assert_eq!(diagnostics.examples[0].source_index, 0);
        assert_eq!(
            diagnostics.completeness,
            RowDiagnosticsCompleteness::Partial
        );
    }

    struct BudgetObservingReader {
        inner: OneBatchReader,
        budget: ResourceBudget,
        observed_memory: Arc<AtomicUsize>,
    }

    impl LayerReader for BudgetObservingReader {
        fn contract(&self) -> &LayerContract {
            self.inner.contract()
        }

        fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
            self.observed_memory.store(
                usize::try_from(self.budget.remaining(ResourceKind::MemoryBytes)).unwrap(),
                AtomicOrdering::SeqCst,
            );
            self.inner.next_batch()
        }
    }

    #[test]
    fn drain_does_not_reserve_the_entire_shared_budget() {
        let budget = ResourceBudget::new(ResourceLimits {
            memory_bytes: 1_048_576,
            cell_bytes: 1_024,
            rows: 1_000,
            output_bytes: 1_048_576,
            concurrent_operations: 2,
            ..ResourceLimits::default()
        })
        .unwrap();
        let observed_memory = Arc::new(AtomicUsize::new(0));
        let operation = budget
            .try_lease(ResourceKind::ConcurrentOperations, 1)
            .unwrap();
        let mut reader = BudgetedReader::new(
            Box::new(BudgetObservingReader {
                inner: OneBatchReader::new(vec![1]),
                budget: budget.clone(),
                observed_memory: observed_memory.clone(),
            }),
            budget,
            true,
            CancellationToken::default(),
            BatchTarget {
                target_bytes: 1_024,
                max_rows: 10,
            },
            ReadScope::Complete,
            operation,
        )
        .unwrap();

        assert!(reader.next_batch().unwrap().is_some());
        assert!(observed_memory.load(AtomicOrdering::SeqCst) > 0);
    }

    #[test]
    fn large_parent_small_slice_is_charged_incrementally_but_large_batch_is_rejected() {
        const LARGE: usize = 73 * 1024 * 1024;
        let schema = Arc::new(Schema::new(vec![Field::new(
            "value",
            DataType::UInt8,
            false,
        )]));
        let contract = LayerContract {
            id: LayerId(0),
            name: "slice".to_owned(),
            contract: DataContract::new(schema.clone(), None),
        };
        let parent: Arc<dyn Array> = Arc::new(UInt8Array::from(vec![0_u8; LARGE]));
        let slice = parent.slice(0, 1);
        let sliced_batch = RecordBatch::try_new(schema.clone(), vec![slice]).unwrap();
        assert!(sliced_batch.get_array_memory_size() > 72 * 1024 * 1024);
        assert!(incremental_batch_memory_size(&sliced_batch) < 1024);

        let budget = ResourceBudget::default();
        let operation = budget
            .try_lease(ResourceKind::ConcurrentOperations, 1)
            .unwrap();
        let mut sliced_reader = BudgetedReader::new(
            Box::new(OneBatchReader {
                contract: contract.clone(),
                batch: Some(sliced_batch),
            }),
            budget,
            true,
            CancellationToken::default(),
            BatchTarget::default(),
            ReadScope::Complete,
            operation,
        )
        .unwrap();
        assert_eq!(sliced_reader.next_batch().unwrap().unwrap().num_rows(), 1);
        drop(sliced_reader);
        drop(parent);

        let large_batch =
            RecordBatch::try_new(schema, vec![Arc::new(UInt8Array::from(vec![0_u8; LARGE]))])
                .unwrap();
        assert!(incremental_batch_memory_size(&large_batch) > 72 * 1024 * 1024);
        let budget = ResourceBudget::new(ResourceLimits {
            rows: u64::try_from(LARGE).unwrap(),
            ..ResourceLimits::default()
        })
        .unwrap();
        let operation = budget
            .try_lease(ResourceKind::ConcurrentOperations, 1)
            .unwrap();
        let mut large_reader = BudgetedReader::new(
            Box::new(OneBatchReader {
                contract,
                batch: Some(large_batch),
            }),
            budget,
            true,
            CancellationToken::default(),
            BatchTarget {
                target_bytes: 8 * 1024 * 1024,
                max_rows: LARGE,
            },
            ReadScope::Complete,
            operation,
        )
        .unwrap();
        assert_eq!(
            large_reader.next_batch().unwrap_err().category,
            plenora_io_model::ErrorCategory::ResourceLimit
        );
    }

    struct CountingDataset {
        layers: Vec<LayerContract>,
        opens: Arc<AtomicUsize>,
    }

    impl OpenDatasetHandle for CountingDataset {
        fn layers(&self) -> &[LayerContract] {
            &self.layers
        }

        fn fidelity_assessment(&self) -> crate::loss::FidelityAssessment {
            crate::loss::FidelityAssessment::lossless()
        }

        fn open_layer_reader(
            &self,
            _request: &crate::request::ReadRequest,
        ) -> Result<Box<dyn LayerReader>> {
            self.opens.fetch_add(1, AtomicOrdering::SeqCst);
            Ok(Box::new(OneBatchReader::new(vec![1])))
        }
    }

    #[test]
    fn concurrency_budget_is_acquired_before_reader_creation() {
        let budget = ResourceBudget::new(ResourceLimits {
            concurrent_operations: 1,
            ..ResourceLimits::default()
        })
        .unwrap();
        let held = budget
            .try_lease(ResourceKind::ConcurrentOperations, 1)
            .unwrap();
        let opens = Arc::new(AtomicUsize::new(0));
        let dataset = BudgetedDataset {
            dataset: Box::new(CountingDataset {
                layers: vec![validating_contract()],
                opens: opens.clone(),
            }),
            budget,
            physical_row_indices_attestable: true,
        };
        let request = crate::request::ReadRequest {
            layer: LayerId(0),
            projected_fields: None,
            projection_mode: crate::request::ProjectionMode::BestEffort,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            scope: ReadScope::Complete,
            batch_target: BatchTarget::default(),
            cancellation: CancellationToken::default(),
        };

        assert!(dataset.open_layer_reader(&request).is_err());
        assert_eq!(opens.load(AtomicOrdering::SeqCst), 0);
        drop(held);
    }
}
