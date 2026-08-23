//! Adattatori comuni applicati ai `LayerReader`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use arrow_array::{Array, BinaryArray, LargeBinaryArray, RecordBatch};
use plenora_io_model::budget::{
    ConcurrencyLease, InternalMemoryLease, OperationBudget, OperationCounter,
};
use plenora_io_model::contract::{CoordinateDimensions, GeometryEncoding, LayerContract, LayerId};
use plenora_io_model::limits::WkbLimits;
use plenora_io_model::wkb::inspect_wkb;
use plenora_io_model::{
    CancellationToken, ErrorCategory, ErrorPhase, PlenoraIoError, RemoteEffect, Result,
    RetryDisposition, RowDiagnosticColumn, RowDiagnosticExample, RowDiagnosticScope,
    RowDiagnostics, RowDiagnosticsCompleteness, ROW_DIAGNOSTICS_CONTRACT,
    ROW_DIAGNOSTICS_INDEX_BASIS, ROW_DIAGNOSTIC_COLUMN_UNATTESTABLE,
};
use plenora_io_model::{IoErrorCode, NumeroStrutturale, PublicMessage};

use crate::loss::{declare_crs_inconsistency, LossReport};
use crate::request::{effective_batch_rows, incremental_batch_memory_size, BatchTarget, ReadScope};

use super::{saturating_usize, LayerReader, OpenDatasetHandle};
use crate::driver::spool::StagedSpool;

/// Collega a un dataset il budget dell'operazione.
///
/// Ogni reader consuma colonne, righe, byte e una quota di concorrenza dagli
/// stessi contatori, anche quando il budget attraversa più componenti della
/// pipeline.
///
/// Prende le **opzioni**, non un budget gia' estratto: la scelta di quale
/// modello governi i contatori appartiene al core, non ai tredici driver.
/// Prima di S4.c ogni driver scriveva da se'
/// il proprio accesso ai contatori, cioe' tredici copie della stessa
/// decisione. Concentrarla qui e' cio' che ha reso possibile capovolgerla in
/// un atto solo.
///
/// Non restituisce `Result`: con un solo modello di budget non c'e' piu' nulla
/// da rifiutare. Fino a S4.d la firma portava un errore per le opzioni del
/// modello sbagliato — prima quelle nuove, poi quelle vecchie — e ogni driver
/// doveva propagarlo.
#[must_use]
pub fn with_read_budget(
    dataset: Box<dyn OpenDatasetHandle>,
    opts: &crate::driver::ReadOptions,
    physical_row_indices_attestable: bool,
) -> Box<dyn OpenDatasetHandle> {
    Box::new(BudgetedDataset {
        dataset,
        budget: opts.budget().clone(),
        physical_row_indices_attestable,
    })
}

struct BudgetedDataset {
    dataset: Box<dyn OpenDatasetHandle>,
    budget: OperationBudget,
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
        self.budget.context().ensure_active()?;
        // Il lease precede intenzionalmente la creazione del reader: diversi
        // driver avviano qui il worker parser e non devono farlo fuori quota.
        let operation_lease = self.budget.context().lease_concurrency()?;
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
/// Nonostante l'API `LayerReader::next_batch` suggerisca un modello streaming
/// (batch per batch, con backpressure), questo adapter esegue
/// `drain_operation` durante la *prima* chiamata di `next_batch`: itera la
/// sorgente fino a EOF e verifica il contratto su tutti i batch. Solo dopo
/// aver drenato l'intera sorgente restituisce il primo batch al chiamante.
///
/// La ragione e' l'**atomicita' operativa**, dichiarata da `ENGINEERING.md
/// § Pipeline di lettura`: se una violazione emerge in un qualsiasi punto
/// della sorgente, il chiamante
/// non deve aver mai visto un prefisso accepted; l'intera operazione viene
/// rigettata come un blocco unico. Il pattern semplifica il rollback lato
/// consumatore (writer, aggregazioni) al costo della latenza al primo batch,
/// pari alla lettura completa della sorgente.
///
/// **La memoria non e' piu' il prezzo di quella garanzia.** I batch verificati
/// vivono in uno [`StagedSpool`]: restano in RAM sotto una soglia adattiva,
/// poi migrano su un file temporaneo in Arrow IPC e non tornano indietro. Il
/// picco e' `soglia + batch corrente`, indipendente dalla dimensione totale
/// dell'input, e la quota di memoria di ogni batch e' una prenotazione viva
/// che torna quando il batch lascia la RAM — non un consumo definitivo.
///
/// Lo streaming reale, con errore terminale *dopo* batch gia' consegnati,
/// resta `DeliverySemantics::Streaming`: dichiarabile e non implementata,
/// perche' cambia il contratto pubblico — servono una categoria d'errore
/// nuova e un bump di protocollo, non ratificati — e richiede coordinamento
/// cross-component.
struct BudgetedReader {
    inner: Box<dyn LayerReader>,
    budget: OperationBudget,
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
    _operation_lease: ConcurrencyLease,
}

impl BudgetedReader {
    fn new(
        inner: Box<dyn LayerReader>,
        budget: OperationBudget,
        physical_row_indices_attestable: bool,
        cancellation: CancellationToken,
        batch_target: BatchTarget,
        scope: ReadScope,
        operation_lease: ConcurrencyLease,
    ) -> Result<Self> {
        let columns =
            u64::try_from(inner.contract().contract.schema.fields().len()).map_err(|_| {
                PlenoraIoError::limite_redatto(&PublicMessage::Curated("troppe colonne nel reader"))
            })?;
        if columns > 0 {
            budget
                .try_lease(OperationCounter::Columns, columns)?
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
            self.budget.context().ensure_active().map_err(|error| {
                terminal_scan_error(error, &violations, self.physical_row_indices_attestable)
            })?;
            // **Due target distinti, non uno.** La memoria deve coprire anche
            // l'ingombro strutturale del batch in coda allo spool, perche' e'
            // quello che la lease dovra' contabilizzare dopo la riduzione;
            // l'output no, perche' `PER_BATCH_OVERHEAD_BYTES` e' occupazione
            // interna della libreria e non byte prodotti. Sommarlo anche li'
            // avrebbe consumato quota di uscita che nessuno scrive.
            //
            // Senza l'addendo sulla memoria la prenotazione poteva risultare
            // **piu' piccola** dell'ingombro contabilizzato — bastava un
            // `max_wkb_cell_bytes` inferiore all'overhead — e allo spool
            // sarebbe arrivata una lease sottodimensionata: `shrink_to`
            // riduce, non allarga, e il ramo che la chiama scatta solo nel
            // verso opposto.
            let target_batch = u64::try_from(self.batch_target.target_bytes).unwrap_or(u64::MAX);
            let batch_bytes_output =
                target_batch.saturating_add(cell_bytes_u64(self.budget.context()));
            let batch_bytes_memoria =
                batch_bytes_output.saturating_add(crate::driver::spool::PER_BATCH_OVERHEAD_BYTES);
            let batch_rows = u64::try_from(self.batch_target.max_rows).unwrap_or(u64::MAX);
            let available_memory = bounded(
                self.budget.context().effective_remaining_memory(),
                batch_bytes_memoria,
            );
            let available_rows = bounded(self.budget.remaining(OperationCounter::Rows), batch_rows);
            let available_output = bounded(output_disponibile(&self.budget), batch_bytes_output);
            if available_memory == 0 {
                // Senza memoria non si puo' materializzare nulla, nemmeno per
                // scoprire la fine della sorgente: una sonda qui leggerebbe
                // fuori quota, che e' esattamente cio' che il budget vieta.
                return Err(terminal_scan_error(
                    PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                        "budget di memoria esaurito prima della materializzazione del batch",
                    )),
                    &violations,
                    self.physical_row_indices_attestable,
                ));
            }
            if available_rows == 0 || available_output == 0 {
                // Righe o output esauriti possono essere semplicemente la fine
                // della sorgente: un dataset di N righe letto con quota N deve
                // riuscire. La sonda pero' avviene **dentro** quota — la lease
                // di memoria bounda cio' che il driver puo' materializzare
                // mentre scopriamo se ha finito.
                let probe = self
                    .budget
                    .context()
                    .lease_memory_internal(available_memory)
                    .map_err(|error| {
                        terminal_scan_error(
                            error,
                            &violations,
                            self.physical_row_indices_attestable,
                        )
                    })?;
                let next = self.inner.next_batch().map_err(|error| {
                    terminal_scan_error(error, &violations, self.physical_row_indices_attestable)
                })?;
                drop(probe);
                if next.is_some() {
                    return Err(terminal_scan_error(
                        PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                            "budget esaurito prima della materializzazione del batch",
                        )),
                        &violations,
                        self.physical_row_indices_attestable,
                    ));
                }
                break;
            }
            // Ogni operazione prenota al massimo il target bounded del proprio
            // batch, non tutto il residuo condiviso. L'inutilizzato torna
            // subito al budget.
            // Il tipo e' annotato perche' qui inizia la proprieta' della
            // memoria del batch: da questa riga fino allo spool c'e' un solo
            // titolare, e il nome lo rende leggibile senza risalire al
            // context.
            let mut memory_lease: InternalMemoryLease = self
                .budget
                .context()
                .lease_memory_internal(available_memory)
                .map_err(|error| {
                    terminal_scan_error(error, &violations, self.physical_row_indices_attestable)
                })?;
            let rows_lease = self
                .budget
                .try_lease(OperationCounter::Rows, available_rows)
                .map_err(|error| {
                    terminal_scan_error(error, &violations, self.physical_row_indices_attestable)
                })?;
            let output_lease = self
                .budget
                .try_lease(OperationCounter::OutputBytes, available_output)
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
                    PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                        "batch oltre il conteggio supportato",
                    )),
                    &violations,
                    self.physical_row_indices_attestable,
                )
            })?;
            let bytes = u64::try_from(incremental_batch_memory_size(&batch)).map_err(|_| {
                terminal_scan_error(
                    PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                        "batch oltre il conteggio byte supportato",
                    )),
                    &violations,
                    self.physical_row_indices_attestable,
                )
            })?;
            if rows > rows_lease.amount()
                || bytes > memory_lease.bytes()
                || bytes > output_lease.amount()
            {
                return Err(terminal_scan_error(
                    PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                        "batch materializzato oltre la quota prenotata",
                    )),
                    &violations,
                    self.physical_row_indices_attestable,
                ));
            }
            let batch_violations = collect_read_violations(
                self.inner.contract(),
                &batch,
                self.rows_scanned,
                &self.budget.context().limits().wkb_limits(),
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
                        .try_lease(OperationCounter::GeometryComponents, geometry_components)
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
                    PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                        "overflow nel conteggio righe lette",
                    )),
                    &violations,
                    self.physical_row_indices_attestable,
                )
            })?;
            // **Handoff senza finestra.** La prenotazione di
            // materializzazione era larga per necessita' — target del batch
            // piu' tetto per cella, perche' prima di leggere non si sa quanto
            // occupera'. Ora la grandezza e' nota: `shrink_to` porta la
            // prenotazione a quella, restituendo **solo** l'eccedenza, e la
            // lease ridotta si sposta per `move` nello spool.
            //
            // Rilasciare e riacquistare — che e' cio' che questo punto faceva
            // fino a S4.c — lasciava un istante in cui il batch e' in RAM e
            // non lo conta nessuno. Con un budget condiviso, cioe' `convert`,
            // un'altra operazione poteva infilarcisi e prenotare memoria che
            // di fatto non c'era.
            //
            // La grandezza include `PER_BATCH_OVERHEAD_BYTES`: un batch
            // custodito occupa sempre almeno l'ingombro della propria presenza
            // in coda, anche senza righe ne' colonne, ed e' lo stesso valore
            // che lo spool usa per decidere la migrazione.
            let accounted = bytes.saturating_add(crate::driver::spool::PER_BATCH_OVERHEAD_BYTES);
            // Fail-closed prima della cessione. `shrink_to` riduce e basta:
            // se l'ingombro contabilizzato eccedesse la prenotazione, lo
            // spool custodirebbe un batch con una lease che non lo copre, e
            // la contabilita' direbbe meno di quanto la libreria occupa
            // davvero. Meglio fallire qui, dove la causa e' visibile.
            if accounted > memory_lease.bytes() {
                return Err(terminal_scan_error(
                    PlenoraIoError::limite_redatto(&PublicMessage::CuratedBetween(
                        "ingombro contabilizzato del batch",
                        NumeroStrutturale::Conteggio(accounted),
                        "byte oltre la prenotazione di materializzazione di",
                        NumeroStrutturale::Limite(memory_lease.bytes()),
                    )),
                    &violations,
                    self.physical_row_indices_attestable,
                ));
            }
            if accounted < memory_lease.bytes() {
                memory_lease.shrink_to(accounted).map_err(|error| {
                    terminal_scan_error(error, &violations, self.physical_row_indices_attestable)
                })?;
            }
            if violations.is_empty() {
                let spool = match self.spool.as_mut() {
                    Some(spool) => spool,
                    None => self.spool.insert(StagedSpool::new(
                        batch.schema(),
                        self.budget.clone(),
                        self.cancellation.clone(),
                    )),
                };
                spool.push(batch, memory_lease).map_err(|error| {
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

/// Prenotazione bounded: mai piu' del residuo, mai meno di uno.
///
/// Il minimo a uno serve a distinguere "quota finita" da "target nullo": con
/// zero non si potrebbe nemmeno sondare la fine della sorgente.
const fn bounded(residuo: u64, batch_cap: u64) -> u64 {
    let cap = if batch_cap == 0 { 1 } else { batch_cap };
    if residuo < cap {
        residuo
    } else {
        cap
    }
}

/// Quota di output ancora prelevabile, sotto **entrambi** i vincoli.
///
/// `remaining(OutputBytes)` riporta il solo contatore cumulativo, mentre
/// `try_lease` applica anche il tetto derivato dall'input osservato
/// (`output_expansion_ratio`). Prenotare sulla base del solo contatore
/// significherebbe chiedere una quota che la lease rifiuta: un round-trip di
/// pochi byte fallirebbe perche' l'adapter ha chiesto il target del batch
/// invece di cio' che il tetto derivato concede.
///
/// Nel modello legacy la differenza non si vedeva perche' l'osservazione
/// dell'input restringeva direttamente il contatore; qui il tetto e' una
/// proiezione calcolata a ogni lease, e va composta esplicitamente.
fn output_disponibile(budget: &OperationBudget) -> u64 {
    let capacita = budget.context().limits().max_output_bytes();
    let residuo = budget.remaining(OperationCounter::OutputBytes);
    let consumato = capacita.saturating_sub(residuo);
    residuo.min(budget.output_limit().saturating_sub(consumato))
}

/// Tetto per cella in `u64`, saturante.
///
/// `PipelineLimits` lo espone in `usize` perche' e' la grandezza di un buffer;
/// qui serve sommarlo a un target in byte.
fn cell_bytes_u64(context: &plenora_io_model::budget::PipelineContext) -> u64 {
    u64::try_from(context.limits().max_wkb_cell_bytes()).unwrap_or(u64::MAX)
}

#[cfg(test)]
impl BudgetedReader {
    /// Vero se lo spool ha migrato su disco almeno una volta.
    ///
    /// Lo spool e' privato, e a rilettura conclusa il suo stato corrente non
    /// distingue piu' "non ha spillato" da "ha spillato e ha finito". Senza
    /// questo seam, un test che verifica il completamento sotto quota stretta
    /// non potrebbe escludere che la quota fosse in realta' sufficiente.
    fn ha_spillato(&self) -> bool {
        self.spool.as_ref().is_some_and(StagedSpool::spilled_once)
    }
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
    wkb: &WkbLimits,
) -> Result<()> {
    let violations = collect_read_violations(contract, batch, row_offset, wkb)?;
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
/// Raccoglie le violazioni del contratto di lettura su un batch.
///
/// `wkb` sono i limiti **dell'operazione**, non i default del contratto: fino
/// a S5.1 questa funzione usava `WkbLimits::default()` perche' non li
/// riceveva, quindi un `--max-wkb-cell-bytes` piu' stretto era applicato in
/// inferenza e nella materializzazione ma non qui — l'unico punto del percorso
/// comune che ogni driver attraversa.
fn collect_read_violations(
    contract: &LayerContract,
    batch: &RecordBatch,
    row_offset: u64,
    wkb: &WkbLimits,
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
            PlenoraIoError::schema_redatto(&PublicMessage::Curated(
                "field_id geometrico fuori intervallo",
            ))
        })?;
        // Finding #1 review 2026-08-15: `RecordBatch::column` panica su OOB.
        // Il driver IPC verifica ora `field_id` contro la posizione fisica
        // dello schema all'`open`, ma la barriera runtime qui e' comunque
        // necessaria: batch prodotti da driver che non applicano la stessa
        // verifica, batch materializzati in test, o schemi che divergono da
        // quelli attesi dal contratto non devono terminare il processo.
        let array = batch.columns().get(index).ok_or_else(|| {
            PlenoraIoError::schema_redatto(&PublicMessage::CuratedBetween(
                "field_id geometrico fuori dallo schema: indice",
                NumeroStrutturale::Indice(super::saturating_u64(index)),
                "su campi del batch",
                NumeroStrutturale::Conteggio(super::saturating_u64(batch.num_columns())),
            ))
        })?;
        let limits = *wkb;
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
            return Err(PlenoraIoError::redatto(
                IoErrorCode::Generic,
                ErrorCategory::Schema,
                ErrorPhase::Read,
                RemoteEffect::None,
                RetryDisposition::Never,
                &PublicMessage::Curated("colonna geometrica letta non Binary/LargeBinary"),
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
    PlenoraIoError::redatto(
        IoErrorCode::Generic,
        ErrorCategory::Schema,
        ErrorPhase::Read,
        RemoteEffect::None,
        RetryDisposition::Never,
        &PublicMessage::Curated(
            "schema del batch letto diverso dal contratto effettivo dichiarato",
        ),
    )
}

fn physical_index(row_offset: u64, row: usize) -> Result<u64> {
    row_offset
        .checked_add(u64::try_from(row).map_err(|_| {
            PlenoraIoError::limite_redatto(&PublicMessage::Curated("indice riga oltre u64"))
        })?)
        .ok_or_else(|| {
            PlenoraIoError::limite_redatto(&PublicMessage::Curated("overflow nell'indice riga"))
        })
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
                PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                    "overflow nel conteggio delle righe diagnosticate",
                ))
            })?;
            let cause_count = self.counts.entry(cause.to_owned()).or_default();
            *cause_count = cause_count.checked_add(1).ok_or_else(|| {
                PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                    "overflow nel conteggio delle cause diagnostiche",
                ))
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
        let error = PlenoraIoError::redatto(
            IoErrorCode::Generic,
            ErrorCategory::DataMapping,
            ErrorPhase::Read,
            RemoteEffect::None,
            RetryDisposition::Never,
            &PublicMessage::CuratedWith(
                "righe lette non conformi al contratto dichiarato:",
                NumeroStrutturale::Conteggio(observed_total),
            ),
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
    budget: &OperationBudget,
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
    // **Due tetti, entrambi validi.** Il primo e' per singola geometria —
    // `effective_wkb_components()`, gia' composto con `max_vertices` — e il
    // secondo e' il residuo del contatore cumulativo: una geometria non puo'
    // superare ne' il proprio tetto ne' quanto resta all'intera operazione.
    //
    // Fino a S5.1 qui compariva solo il secondo. Con una quota cumulativa
    // ampia — il default e' oltre sedici milioni — il tetto per cella non
    // legava mai, e `--max-wkb-components` non aveva effetto sulla
    // validazione del batch.
    let context_limits = budget.context().limits();
    let limits = WkbLimits {
        max_cell_bytes: context_limits.max_wkb_cell_bytes(),
        max_components: context_limits
            .effective_wkb_components()
            .min(saturating_usize(
                budget.remaining(OperationCounter::GeometryComponents),
            )),
        max_depth: context_limits.max_wkb_depth(),
    };
    let array = batch.column(index);
    let mut total = 0_u64;
    let mut inspect = |bytes: &[u8]| -> Result<()> {
        let components = u64::try_from(inspect_wkb(bytes, &limits)?.components).map_err(|_| {
            PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                "geometria oltre il conteggio supportato",
            ))
        })?;
        total = total.checked_add(components).ok_or_else(|| {
            PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                "overflow nel conteggio dei componenti geometrici",
            ))
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
    Err(PlenoraIoError::limite_redatto(&PublicMessage::Curated(
        "colonna geometrica non binaria nel reader budgeted",
    )))
}

/// Adatta i batch prodotti da un reader al target comune di `ENGINEERING.md § Projection e pruning`.
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

/// Enforcement runtime di `ReaderConcurrency::SingleActiveReader` (`ENGINEERING.md § Interfaccia dei driver`).
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

    use super::*;
    use plenora_io_model::budget::{PipelineBudget, PipelineLimits, ResourcePool};
    use plenora_io_model::wkb::{encode_wkb, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue};

    /// Budget dell'operazione per i test, dal modello unificato.
    ///
    /// Passa dal bundle e non costruisce il context a mano: e' l'unica via
    /// che esiste, ed e' quella che i driver useranno.
    fn budget_con(limits: PipelineLimits) -> OperationBudget {
        match PipelineBudget::builder().limits(limits).build() {
            Ok(bundle) => bundle.into_write_parts().into_budget(),
            Err(error) => unreachable!("budget di test non costruibile: {error:?}"),
        }
    }

    fn richiesta_completa() -> crate::request::ReadRequest {
        crate::request::ReadRequest {
            layer: LayerId(0),
            projected_fields: None,
            projection_mode: crate::request::ProjectionMode::BestEffort,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            scope: ReadScope::Complete,
            batch_target: BatchTarget::default(),
            cancellation: CancellationToken::default(),
        }
    }

    /// Prende una lease di concorrenza dal pool, passando da un budget
    /// agganciato: e' l'unica via, il pool non si interroga da solo.
    fn pool_lease(pool: &ResourcePool) -> Result<plenora_io_model::budget::ConcurrencyLease> {
        budget_con_pool(PipelineLimits::default(), pool.clone())
            .context()
            .lease_concurrency()
    }

    /// Come [`budget_con`], ma agganciato a un pool: serve solo dove il test
    /// verifica la concorrenza, che senza pool e' un no-op (INV-12).
    fn budget_con_pool(limits: PipelineLimits, pool: ResourcePool) -> OperationBudget {
        match PipelineBudget::builder()
            .limits(limits)
            .resource_pool(pool)
            .build()
        {
            Ok(bundle) => bundle.into_write_parts().into_budget(),
            Err(error) => unreachable!("budget di test non costruibile: {error:?}"),
        }
    }

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
        budget: OperationBudget,
    ) -> BudgetedReader {
        budgeted_sequence_con_target(events, budget, BatchTarget::default())
    }

    fn budgeted_sequence_con_target(
        events: VecDeque<Result<Option<RecordBatch>>>,
        budget: OperationBudget,
        batch_target: BatchTarget,
    ) -> BudgetedReader {
        let operation = budget.context().lease_concurrency().unwrap();
        BudgetedReader::new(
            Box::new(SequenceReader {
                contract: validating_contract(),
                events,
            }),
            budget,
            true,
            CancellationToken::default(),
            batch_target,
            ReadScope::Complete,
            operation,
        )
        .unwrap()
    }

    /// Il caso che `ENGINEERING.md § Spool e memoria` esiste per risolvere:
    /// prima dello spool i batch verificati restavano tutti in RAM, quindi
    /// un dataset piu' grande della
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
        let budget = budget_con(
            PipelineLimits::default()
                .with_memory_bytes(4_096)
                .with_max_wkb_cell_bytes(1_024),
        );
        let mut reader = budgeted_sequence_with_budget(eventi, budget.clone());

        let mut consegnati = 0_usize;
        while let Some(batch) = reader.next_batch().unwrap() {
            assert_eq!(batch.num_rows(), 8);
            consegnati += 1;
        }
        assert_eq!(consegnati, 40);
        assert_eq!(
            budget.context().remaining_memory(),
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
        let budget = budget_con(PipelineLimits::default());
        let iniziale = budget.context().remaining_memory();
        let mut reader = budgeted_sequence_with_budget(eventi, budget.clone());

        while reader.next_batch().unwrap().is_some() {}
        assert_eq!(
            budget.context().remaining_memory(),
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
        let budget = budget_con(PipelineLimits::default().with_max_rows(20));
        let mut reader = budgeted_sequence_with_budget(eventi, budget);
        let mut righe = 0_usize;
        while let Some(batch) = reader.next_batch().unwrap() {
            righe += batch.num_rows();
        }
        assert_eq!(righe, 20);
    }

    /// La sonda che scopre l'EOF deve restare dentro quota: senza una lease
    /// di memoria il driver materializzerebbe un batch che il budget non
    /// copre, cioe' proprio cio' che il budget esiste per impedire.
    #[test]
    fn eof_probe_requires_memory_quota_instead_of_reading_outside_it() {
        let contract = validating_contract();
        let eventi: VecDeque<Result<Option<RecordBatch>>> =
            VecDeque::from([Ok(Some(geometry_batch(&contract, &[true; 4])))]);
        let budget = budget_con(
            PipelineLimits::default()
                .with_memory_bytes(4_096)
                .with_max_wkb_cell_bytes(1_024),
        );
        // Nessuna memoria residua: non c'e' modo di materializzare nulla,
        // nemmeno per scoprire se la sorgente e' finita.
        let trattenuta = budget.context().lease_memory_internal(4_096).unwrap();
        let mut reader = budgeted_sequence_with_budget(eventi, budget);
        let errore = reader.next_batch().unwrap_err();
        assert!(
            errore.message.contains("memoria"),
            "l'errore deve dire che manca la memoria, non confondersi con le altre quote: {}",
            errore.message
        );
        drop(trattenuta);
    }

    /// Una riga oltre la quota deve continuare a fallire: la correzione
    /// dell'EOF non deve allentare il limite.
    #[test]
    fn reader_of_n_plus_one_rows_with_max_rows_n_still_fails() {
        let contract = validating_contract();
        let eventi: VecDeque<Result<Option<RecordBatch>>> = (0..5)
            .map(|_| Ok(Some(geometry_batch(&contract, &[true; 5]))))
            .collect();
        let budget = budget_con(PipelineLimits::default().with_max_rows(20));
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
        let stretto = budget_con(PipelineLimits::default().with_max_rows(19));
        let mut reader = budgeted_sequence_with_budget(eventi(), stretto);
        assert!(
            reader.next_batch().is_err(),
            "una riga in meno di quota deve ancora fallire"
        );

        let esatto = budget_con(PipelineLimits::default().with_max_rows(20));
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
            esatto.remaining(OperationCounter::Rows),
            0,
            "le righe restano cumulative, consumate una volta sola"
        );

        // `OutputBytes` resta cumulativo e consumato, non restituito.
        let output = budget_con(PipelineLimits::default());
        let prima = output.remaining(OperationCounter::OutputBytes);
        let mut reader = budgeted_sequence_with_budget(eventi(), output.clone());
        while reader.next_batch().unwrap().is_some() {}
        assert!(
            output.remaining(OperationCounter::OutputBytes) < prima,
            "OutputBytes e' quota consumata, non occupazione trattenuta"
        );

        // La concorrenza vive nel pool (INV-12): senza pool la lease e' un
        // no-op, quindi qui si verifica con un pool esplicito che una sola
        // operazione consumi una sola quota.
        let pool = match ResourcePool::builder().concurrent_operations(2).build() {
            Ok(pool) => pool,
            Err(error) => unreachable!("pool di test: {error:?}"),
        };
        let concorrenza = budget_con_pool(PipelineLimits::default(), pool.clone());
        let reader = budgeted_sequence_with_budget(eventi(), concorrenza);
        // Con due posti e uno occupato dal reader, ne resta esattamente uno.
        let secondo = match pool_lease(&pool) {
            Ok(lease) => lease,
            Err(error) => unreachable!("il secondo posto deve essere libero: {error:?}"),
        };
        assert!(
            pool_lease(&pool).is_err(),
            "una sola operazione, contata una sola volta: il terzo posto non esiste"
        );
        drop(secondo);
        drop(reader);
        // Rilasciati entrambi, il pool torna capiente: la quota di
        // concorrenza e' occupazione trattenuta, non consumo definitivo.
        assert!(pool_lease(&pool).is_ok());
        assert!(pool_lease(&pool).is_ok());
    }

    fn budgeted_sequence_with_scope(
        events: VecDeque<Result<Option<RecordBatch>>>,
        scope: ReadScope,
    ) -> BudgetedReader {
        let budget = budget_con(PipelineLimits::default());
        let operation = budget.context().lease_concurrency().unwrap();
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
                Err(PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                    "la coda non doveva essere letta",
                ))),
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
            Err(PlenoraIoError::formato_redatto(
                "test",
                &PublicMessage::Curated("invalid tail observed"),
            ))
        }
    }

    #[test]
    fn accepted_rows_zero_never_polls_the_inner_reader() {
        let budget = budget_con(PipelineLimits::default());
        let operation = budget.context().lease_concurrency().unwrap();
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
                events: VecDeque::from([Err(PlenoraIoError::formato_redatto(
                    "test",
                    &PublicMessage::Curated("boom"),
                ))]),
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
        let budget = budget_con(
            PipelineLimits::default()
                .with_max_rows(3)
                .with_max_columns(10)
                .with_memory_bytes(1024 * 1024)
                .with_max_wkb_cell_bytes(1024)
                .with_max_output_bytes(1024 * 1024),
        );
        let first_operation = budget.context().lease_concurrency().unwrap();
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

        let second_operation = budget.context().lease_concurrency().unwrap();
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

        let attestable =
            validate_read_batch(&contract, &batch, 10, true, &WkbLimits::default()).unwrap_err();
        let diagnostics = attestable.row_diagnostics.as_deref().unwrap();
        assert_eq!(diagnostics.observed_total, 2);
        assert_eq!(diagnostics.examples[0].source_index, 10);
        assert_eq!(diagnostics.examples[1].source_index, 11);
        assert!(diagnostics.validate().is_ok());

        let non_attestable =
            validate_read_batch(&contract, &batch, 10, false, &WkbLimits::default()).unwrap_err();
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
            let error =
                validate_read_batch(&contract, &batch, 0, true, &WkbLimits::default()).unwrap_err();
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
            PlenoraIoError::formato_redatto("test", &PublicMessage::Curated("driver failed"))
                .with_row_diagnostics(*invalid);
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
        let budget = budget_con(PipelineLimits::default());
        let operation = budget.context().lease_concurrency().unwrap();
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
        budget: OperationBudget,
        observed_memory: Arc<AtomicUsize>,
    }

    impl LayerReader for BudgetObservingReader {
        fn contract(&self) -> &LayerContract {
            self.inner.contract()
        }

        fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
            self.observed_memory.store(
                usize::try_from(self.budget.context().remaining_memory()).unwrap(),
                AtomicOrdering::SeqCst,
            );
            self.inner.next_batch()
        }
    }

    #[test]
    fn drain_does_not_reserve_the_entire_shared_budget() {
        // La concorrenza nel modello unificato e' governata dal pool, non
        // dai limiti dell'operazione (INV-12): senza pool non c'e' tetto, ed
        // e' esattamente cio' che questo test vuole — due reader ammessi.
        let budget = budget_con(
            PipelineLimits::default()
                .with_memory_bytes(1_048_576)
                .with_max_wkb_cell_bytes(1_024)
                .with_max_rows(1_000)
                .with_max_output_bytes(1_048_576),
        );
        let observed_memory = Arc::new(AtomicUsize::new(0));
        let operation = budget.context().lease_concurrency().unwrap();
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

        let budget = budget_con(PipelineLimits::default());
        let operation = budget.context().lease_concurrency().unwrap();
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
        let budget = budget_con(PipelineLimits::default().with_max_rows(1_000_000));
        let operation = budget.context().lease_concurrency().unwrap();
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
        // Il tetto di concorrenza vive nel pool (INV-12): senza pool la
        // lease e' un no-op e non ci sarebbe nulla da esaurire.
        let pool = match ResourcePool::builder().concurrent_operations(1).build() {
            Ok(pool) => pool,
            Err(error) => unreachable!("pool di test non costruibile: {error:?}"),
        };
        let budget = budget_con_pool(PipelineLimits::default(), pool);
        let held = match budget.context().lease_concurrency() {
            Ok(lease) => lease,
            Err(error) => unreachable!("la prima lease deve passare: {error:?}"),
        };
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

    /// Reader che segnala quando l'adapter gli chiede il batch successivo.
    ///
    /// Serve a sapere quando **almeno un batch e' gia' custodito** dallo
    /// spool: l'adapter chiede il batch `k+1` solo dopo aver spinto il `k`.
    /// Prima di quel momento non c'e' occupazione da difendere, e un
    /// osservatore che prenotasse allora non starebbe intrudendo.
    struct ReaderSegnalante {
        contract: LayerContract,
        events: VecDeque<Result<Option<RecordBatch>>>,
        consegnati: Arc<std::sync::atomic::AtomicU64>,
        eof: Arc<std::sync::atomic::AtomicBool>,
        /// Tentativi dell'osservatore, letti dal reader per **attendere**
        /// che almeno uno sia avvenuto.
        ///
        /// Senza l'attesa la copertura dipende dallo scheduler: sotto la
        /// suite completa l'osservatore puo' non essere schedulato per
        /// l'intero drenaggio, e il test fallisce sull'asserzione "nessuno ha
        /// guardato" pur essendo il codice corretto. Non e' flakiness da
        /// tollerare: e' una sincronizzazione mancante.
        tentativi: Arc<AtomicUsize>,
    }

    impl LayerReader for ReaderSegnalante {
        fn contract(&self) -> &LayerContract {
            &self.contract
        }

        fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
            let esito = self.events.pop_front().unwrap_or(Ok(None));
            match esito {
                Ok(Some(_)) => {
                    let consegnati = self.consegnati.fetch_add(1, AtomicOrdering::SeqCst) + 1;
                    if consegnati == 1 {
                        // Il primo batch apre la fase in cui l'osservatore ha
                        // qualcosa da difendere: da qui non si procede finche'
                        // non ha guardato almeno una volta.
                        while self.tentativi.load(AtomicOrdering::SeqCst) == 0 {
                            std::hint::spin_loop();
                        }
                    }
                }
                // L'EOF chiude il drenaggio: da qui in poi la memoria torna
                // legittimamente al gauge, batch dopo batch, e l'osservatore
                // deve smettere di guardare.
                Ok(None) => self.eof.store(true, AtomicOrdering::SeqCst),
                Err(_) => {}
            }
            esito
        }
    }

    /// L'handoff sul percorso reale, senza ponte legacy e senza finestra.
    ///
    /// # Cosa dimostra
    ///
    /// **Nessun ponte.** Le opzioni nascono da `from_read_parts`, cioe' dal
    /// modello unificato, e attraversano `with_read_budget` senza toccare
    /// la guardia del modello: se un solo anello del percorso fosse ancora
    /// legacy, `with_read_budget` restituirebbe `Unsupported` e il test
    /// fallirebbe alla prima riga utile.
    ///
    /// **Nessuna finestra.** Un osservatore concorrente prova a prenotare
    /// `capacita - accounted + 1` byte: una prenotazione che entra **solo**
    /// se la memoria custodita e' scesa sotto l'ingombro di un batch. Con
    /// `shrink_to` + `move` la quota contabilizzata passa da RESERVED ad
    /// ACCOUNTED senza mai tornare al gauge, quindi quella soglia non entra
    /// mai. Con il vecchio rilascia-e-riacquista ci sarebbe un istante in cui
    /// entra, ed e' esattamente l'istante in cui il batch e' in RAM senza che
    /// nessuno lo conti.
    ///
    /// L'osservatore conta i propri tentativi e il test verifica che siano
    /// stati piu' di zero: senza, un verde direbbe soltanto che nessuno ha
    /// guardato.
    // Il test descrive una corsa completa: costruzione della pipeline,
    // osservatore concorrente, drenaggio e riconsegna. Spezzarlo in
    // funzioni renderebbe meno leggibile proprio l'ordine dei passi, che e'
    // cio' che dimostra.
    #[allow(clippy::too_many_lines)]
    #[test]
    fn handoff_reale_della_memoria_senza_bridge_legacy() {
        const CAPACITA: u64 = 4 * 1024 * 1024;
        // Molti batch, non pochi: la finestra dura pochi nanosecondi e si
        // riapre a ogni batch. Con sei occasioni un osservatore la coglieva
        // due volte su cinque — non abbastanza per essere evidenza. Con
        // quattrocento le occasioni sono due ordini di grandezza in piu', e
        // l'occupazione totale (400 x ~1,2 KiB) resta comodamente dentro la
        // capacita' e sotto la soglia di migrazione.
        const BATCH: usize = 400;

        let batch = || {
            let schema = Arc::new(Schema::new(vec![Field::new(
                "geometry",
                DataType::Binary,
                true,
            )]));
            let punto = WkbGeometry {
                value: WkbValue::Point(WkbCoordinate {
                    x: 1.0,
                    y: 2.0,
                    z: None,
                    m: None,
                }),
                dimensions: CoordinateDimensions::Xy,
                srid: None,
            };
            let geometria = encode_wkb(&punto, WkbFlavor::Iso).expect("wkb");
            RecordBatch::try_new(
                schema,
                vec![Arc::new(BinaryArray::from(vec![geometria.as_slice()]))],
            )
            .expect("batch")
        };

        // Il percorso nasce sul modello unificato: nessun `from_legacy` qui.
        let bundle = match plenora_io_model::budget::PipelineBudget::builder()
            .limits(
                PipelineLimits::default()
                    .with_memory_bytes(CAPACITA)
                    // Il tetto per cella entra nella prenotazione di
                    // materializzazione: col default da 64 MiB non starebbe
                    // dentro la capacita' di questo test.
                    .with_max_wkb_cell_bytes(4_096),
            )
            .build()
        {
            Ok(bundle) => bundle,
            Err(error) => unreachable!("bundle di test: {error:?}"),
        };
        let contesto = bundle.context().clone();
        let opts = crate::driver::ReadOptions::from_read_parts(bundle.into_read_parts());

        let consegnati = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let eof = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let tentativi = Arc::new(AtomicUsize::new(0));
        let mut eventi: VecDeque<Result<Option<RecordBatch>>> = VecDeque::new();
        for _ in 0..BATCH {
            eventi.push_back(Ok(Some(batch())));
        }
        eventi.push_back(Ok(None));

        // L'ingombro contabilizzato di un batch: e' la soglia che
        // l'osservatore usa per distinguere "custodito" da "scoperto".
        let accounted = u64::try_from(incremental_batch_memory_size(&batch()))
            .expect("ingombro rappresentabile")
            .saturating_add(crate::driver::spool::PER_BATCH_OVERHEAD_BYTES);

        let operation = match contesto.lease_concurrency() {
            Ok(lease) => lease,
            Err(error) => unreachable!("lease di concorrenza: {error:?}"),
        };
        let mut reader = match BudgetedReader::new(
            Box::new(ReaderSegnalante {
                contract: validating_contract(),
                events: eventi,
                consegnati: consegnati.clone(),
                eof: eof.clone(),
                tentativi: tentativi.clone(),
            }),
            opts.budget().clone(),
            true,
            CancellationToken::default(),
            BatchTarget::default(),
            ReadScope::Complete,
            operation,
        ) {
            Ok(reader) => reader,
            Err(error) => unreachable!("reader di test: {error:?}"),
        };

        let intrusioni = Arc::new(AtomicUsize::new(0));
        // L'osservatore attraversa il ramo «nessuna consegna» **per
        // costruzione**: `consegnati` non puo' crescere finche' questo thread
        // non chiama `next_batch`, quindi il primo giro lo trova a zero. Senza
        // l'attesa qui sotto quel ramo si eseguirebbe solo vincendo una corsa,
        // e la copertura di quelle righe cambierebbe fra due misure sullo
        // stesso albero.
        let visto_senza_consegne = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let osservatore = {
            let contesto = contesto.clone();
            let intrusioni = intrusioni.clone();
            let tentativi = tentativi.clone();
            let visto_senza_consegne = visto_senza_consegne.clone();
            std::thread::spawn(move || {
                while !eof.load(AtomicOrdering::SeqCst) {
                    // **La soglia cresce con i batch custoditi.** Quando il
                    // reader ha consegnato `k` batch, l'adapter tiene la
                    // residenza dei primi `k-1` piu' la prenotazione del
                    // `k`-esimo: almeno `k * accounted`. Una prenotazione da
                    // `capacita - k * accounted + 1` entra percio' solo se la
                    // memoria trattenuta e' scesa **sotto** quella soglia,
                    // cioe' solo dentro la finestra.
                    //
                    // Una soglia fissa non discriminerebbe: dal secondo batch
                    // in poi l'occupazione accumulata la supererebbe sempre,
                    // e il test tornerebbe verde anche con il vecchio
                    // rilascia-e-riacquista.
                    let k = consegnati.load(AtomicOrdering::SeqCst);
                    if k == 0 {
                        visto_senza_consegne.store(true, AtomicOrdering::SeqCst);
                        std::hint::spin_loop();
                        continue;
                    }
                    let Some(soglia) = CAPACITA.checked_sub(k * accounted).map(|resto| resto + 1)
                    else {
                        break;
                    };
                    let prima = eof.load(AtomicOrdering::SeqCst);
                    tentativi.fetch_add(1, AtomicOrdering::SeqCst);
                    let esito = contesto.lease_memory_internal(soglia);
                    let dopo = eof.load(AtomicOrdering::SeqCst);
                    if let Ok(lease) = esito {
                        drop(lease);
                        // Scartata se l'EOF e' arrivato durante il tentativo:
                        // li' la riconsegna ha gia' iniziato a restituire
                        // memoria, e non sarebbe un'intrusione.
                        if !prima && !dopo {
                            intrusioni.fetch_add(1, AtomicOrdering::SeqCst);
                        }
                    }
                }
            })
        };

        // La finestra da sorvegliare e' il **drenaggio**, che avviene tutto
        // dentro la prima `next_batch`: e' li' che i batch vengono
        // materializzati e ceduti allo spool. Dopo, la riconsegna restituisce
        // legittimamente la memoria batch per batch, e un osservatore ancora
        // attivo la scambierebbe per un'intrusione.
        while !visto_senza_consegne.load(AtomicOrdering::SeqCst) {
            std::hint::spin_loop();
        }
        let primo = match reader.next_batch() {
            Ok(batch) => batch,
            Err(error) => unreachable!("la lettura deve riuscire: {error:?}"),
        };
        osservatore.join().expect("osservatore");

        let mut letti = 0_usize;
        let mut corrente = primo;
        while let Some(batch) = corrente {
            assert_eq!(batch.num_rows(), 1);
            letti += 1;
            corrente = match reader.next_batch() {
                Ok(batch) => batch,
                Err(error) => unreachable!("la lettura deve riuscire: {error:?}"),
            };
        }

        assert_eq!(letti, BATCH, "tutti i batch devono arrivare al consumer");
        assert!(
            tentativi.load(AtomicOrdering::SeqCst) > 0,
            "l'osservatore non ha mai guardato: il verde non direbbe nulla"
        );
        assert_eq!(
            intrusioni.load(AtomicOrdering::SeqCst),
            0,
            "la memoria del batch non deve mai tornare al gauge durante l'handoff"
        );
        // A batch consegnati e reader chiuso, la quota torna intera: la
        // memoria era occupazione trattenuta, non consumo definitivo.
        drop(reader);
        assert_eq!(contesto.remaining_memory(), CAPACITA);
    }

    /// Il tetto per cella piu' piccolo dell'ingombro strutturale.
    ///
    /// Prima di S4.d.1 la prenotazione di materializzazione valeva
    /// `target_bytes + max_wkb_cell_bytes` e l'ingombro contabilizzato
    /// `bytes + PER_BATCH_OVERHEAD_BYTES`. Con un tetto per cella minuscolo
    /// il secondo poteva superare la prima, e allo spool arrivava una lease
    /// **piu' piccola** del batch che doveva coprire: `shrink_to` riduce e
    /// basta, e il ramo che lo chiama scatta solo nel verso opposto.
    ///
    /// Ora l'overhead entra nella prenotazione di memoria — e **solo** in
    /// quella, non in quella di output, che conta byte prodotti e non
    /// occupazione interna della libreria.
    #[test]
    fn l_ingombro_strutturale_e_coperto_anche_con_tetto_per_cella_minuscolo() {
        // Sotto l'overhead strutturale ma sopra la geometria di prova: e' il
        // caso in cui il vecchio calcolo produceva una prenotazione piu'
        // piccola dell'ingombro contabilizzato.
        const CELL: usize = 64;
        // Anche il target del batch deve essere piccolo: con gli 8 MiB
        // predefiniti la prenotazione coprirebbe l'overhead per caso, e il
        // test non distinguerebbe il calcolo corretto da quello vecchio.
        //
        // `TARGET + CELL` sta sotto l'overhead — quindi il vecchio calcolo
        // produceva una prenotazione insufficiente — ma sopra l'ingombro
        // reale del batch, altrimenti a fallire sarebbe la prenotazione di
        // output e il test misurerebbe un'altra cosa.
        const TARGET: usize = 896;
        assert!(
            u64::try_from(TARGET + CELL).expect("piccolo")
                < crate::driver::spool::PER_BATCH_OVERHEAD_BYTES,
            "il caso ha senso solo se la vecchia prenotazione stava sotto l'overhead"
        );

        let budget = budget_con(
            PipelineLimits::default()
                // Memoria stretta ma sufficiente: due batch custoditi piu'
                // una prenotazione di materializzazione.
                .with_memory_bytes(8 * crate::driver::spool::PER_BATCH_OVERHEAD_BYTES)
                .with_max_wkb_cell_bytes(CELL),
        );
        let contratto = validating_contract();
        let mut eventi: VecDeque<Result<Option<RecordBatch>>> = VecDeque::new();
        for _ in 0..4_u8 {
            eventi.push_back(Ok(Some(geometry_batch(&contratto, &[true]))));
        }
        eventi.push_back(Ok(None));
        let mut reader = budgeted_sequence_con_target(
            eventi,
            budget.clone(),
            BatchTarget {
                target_bytes: TARGET,
                max_rows: 8,
            },
        );

        let mut letti = 0_usize;
        while let Some(batch) = match reader.next_batch() {
            Ok(batch) => batch,
            Err(error) => unreachable!("la lettura deve riuscire: {error:?}"),
        } {
            letti += batch.num_rows();
        }
        assert!(
            letti > 0,
            "il percorso deve consegnare i batch, non fallire"
        );
        drop(reader);
        assert_eq!(
            budget.context().effective_remaining_memory(),
            8 * crate::driver::spool::PER_BATCH_OVERHEAD_BYTES,
            "a lettura conclusa la memoria torna intera"
        );
    }

    /// Un pool piu' stretto della pipeline deve far spillare, non fallire.
    ///
    /// `remaining_memory()` riporta il solo gauge locale, mentre
    /// `lease_memory_internal` compone locale e pool (INV-12). Dimensionando
    /// sul solo residuo locale l'adapter chiedeva piu' di quanto entrasse, e
    /// la lease falliva: il chiamante leggeva "memoria esaurita" dove c'era
    /// soltanto una richiesta mal dimensionata. E la soglia di migrazione,
    /// derivata dal solo limite locale, era irraggiungibile — quindi lo spool
    /// non migrava, cioe' restava inutile proprio nel caso che deve risolvere.
    #[test]
    fn un_pool_piu_stretto_della_pipeline_fa_spillare_e_completare() {
        const POOL_MEMORIA: u64 = 96 * 1024;
        const PIPELINE_MEMORIA: u64 = 8 * 1024 * 1024;

        let pool = match ResourcePool::builder()
            .memory_bytes(POOL_MEMORIA)
            .spill_bytes(8 * 1024 * 1024)
            .concurrent_operations(4)
            .build()
        {
            Ok(pool) => pool,
            Err(error) => unreachable!("pool di test: {error:?}"),
        };
        let budget = budget_con_pool(
            PipelineLimits::default()
                .with_memory_bytes(PIPELINE_MEMORIA)
                .with_max_wkb_cell_bytes(4_096),
            pool,
        );

        assert_eq!(
            budget.context().remaining_memory(),
            PIPELINE_MEMORIA,
            "il residuo locale ignora il pool, ed e' il motivo per cui non basta"
        );
        assert_eq!(
            budget.context().effective_remaining_memory(),
            POOL_MEMORIA,
            "il residuo effettivo e' il minimo fra locale e pool"
        );
        assert_eq!(
            crate::driver::spool::adaptive_memory_threshold(&budget),
            POOL_MEMORIA / 2,
            "la soglia deriva dalla capacita' effettiva, non dal limite locale"
        );

        let contratto = validating_contract();
        let mut eventi: VecDeque<Result<Option<RecordBatch>>> = VecDeque::new();
        for _ in 0..64_u8 {
            eventi.push_back(Ok(Some(geometry_batch(&contratto, &[true]))));
        }
        eventi.push_back(Ok(None));
        let mut reader = budgeted_sequence_with_budget(eventi, budget.clone());

        let mut letti = 0_usize;
        while let Some(batch) = match reader.next_batch() {
            Ok(batch) => batch,
            Err(error) => {
                unreachable!("con il pool stretto si deve spillare, non fallire: {error:?}")
            }
        } {
            letti += batch.num_rows();
        }
        assert_eq!(letti, 64, "tutti i batch devono arrivare al consumer");
        assert!(
            reader.ha_spillato(),
            "sotto la quota del pool i batch devono migrare su disco: senza              questa verifica il completamento potrebbe venire da una quota in              realta' sufficiente, e il test non direbbe nulla sul pool"
        );
        drop(reader);
        assert_eq!(budget.context().effective_remaining_memory(), POOL_MEMORIA);
    }

    /// Il tetto **per cella** dei componenti lega anche con quota cumulativa
    /// ampia.
    ///
    /// Fino a S5.1 `geometry_components` costruiva `max_components` dal solo
    /// residuo del contatore cumulativo. Con il default — oltre sedici
    /// milioni — quel residuo non legava mai, e `--max-wkb-components` non
    /// aveva effetto sulla validazione del batch: una singola geometria
    /// enorme passava, purche' l'operazione nel complesso avesse ancora quota.
    #[test]
    fn il_tetto_per_cella_dei_componenti_lega_anche_con_quota_cumulativa_ampia() {
        let contratto = validating_contract();
        // Una LineString di quattro punti: quattro componenti.
        let punti: Vec<WkbCoordinate> = (0..4)
            .map(|indice| WkbCoordinate {
                x: f64::from(indice),
                y: f64::from(indice),
                z: None,
                m: None,
            })
            .collect();
        let geometria = WkbGeometry {
            value: WkbValue::LineString(punti),
            dimensions: CoordinateDimensions::Xy,
            srid: None,
        };
        let bytes = encode_wkb(&geometria, WkbFlavor::Iso).expect("wkb");
        let batch = RecordBatch::try_new(
            contratto.contract.schema.clone(),
            vec![Arc::new(BinaryArray::from(vec![bytes.as_slice()]))],
        )
        .expect("batch");

        // Quota cumulativa ampia, tetto per cella stretto: il secondo deve
        // legare.
        let stretto = budget_con(
            PipelineLimits::default()
                .with_max_geometry_components(1_000_000)
                .with_max_wkb_components(2),
        );
        let esito = geometry_components(&contratto, &batch, &stretto);
        assert!(
            matches!(
                esito,
                Err(ref errore) if errore.code == plenora_io_model::IoErrorCode::Wkb
                    || errore.code == plenora_io_model::IoErrorCode::LimitExceeded
            ),
            "quattro componenti con tetto per cella due devono fallire: {esito:?}"
        );

        // Con il tetto per cella capiente la stessa geometria passa: il
        // rifiuto sopra viene dal per-cella, non da altro.
        let largo = budget_con(
            PipelineLimits::default()
                .with_max_geometry_components(1_000_000)
                .with_max_wkb_components(16),
        );
        assert_eq!(
            geometry_components(&contratto, &batch, &largo).expect("deve passare"),
            4
        );
    }

    /// `collect_read_violations` usa i limiti che riceve, non un default.
    ///
    /// La funzione e' privata, ma i test del modulo la raggiungono: e' il
    /// punto di enforcement piu' importante del percorso comune, perche' ogni
    /// driver ci passa. Verificarlo indirettamente attraverso un driver
    /// lascerebbe la copertura alla ridondanza con altri controlli.
    #[test]
    fn collect_read_violations_usa_i_limiti_ricevuti() {
        let contratto = validating_contract();
        let punti: Vec<WkbCoordinate> = (0..4)
            .map(|indice| WkbCoordinate {
                x: f64::from(indice),
                y: f64::from(indice),
                z: None,
                m: None,
            })
            .collect();
        let geometria = WkbGeometry {
            value: WkbValue::LineString(punti),
            dimensions: CoordinateDimensions::Xy,
            srid: None,
        };
        let bytes = encode_wkb(&geometria, WkbFlavor::Iso).expect("wkb");
        let batch = RecordBatch::try_new(
            contratto.contract.schema.clone(),
            vec![Arc::new(BinaryArray::from(vec![bytes.as_slice()]))],
        )
        .expect("batch");

        // Tetto capiente: nessuna violazione.
        let violazioni = collect_read_violations(&contratto, &batch, 0, &WkbLimits::default())
            .expect("con il default non ci sono violazioni");
        assert!(violazioni.is_empty());

        // Tetto stretto sui byte della cella: la stessa geometria viola.
        let stretto = WkbLimits {
            max_cell_bytes: bytes.len() - 1,
            ..WkbLimits::default()
        };
        let violazioni = collect_read_violations(&contratto, &batch, 0, &stretto)
            .expect("il tetto produce una violazione, non un errore");
        assert_eq!(
            violazioni.len(),
            1,
            "il tetto ricevuto deve essere applicato, non quello predefinito"
        );
    }

    #[test]
    fn with_read_budget_collega_il_budget_dell_operazione() {
        // Da S4.e esiste un solo modello: le opzioni portano sempre un
        // `OperationBudget`, e l'adapter vi si collega senza alternative da
        // rifiutare. Che il collegamento sia avvenuto lo dimostra il fatto
        // che il reader consumi la quota di concorrenza del pool.
        let pool = match ResourcePool::builder().concurrent_operations(1).build() {
            Ok(pool) => pool,
            Err(error) => unreachable!("pool di test: {error:?}"),
        };
        let bundle = match plenora_io_model::budget::PipelineBudget::builder()
            .resource_pool(pool.clone())
            .build()
        {
            Ok(bundle) => bundle,
            Err(error) => unreachable!("bundle di test: {error:?}"),
        };
        let opts = crate::driver::ReadOptions::from_read_parts(bundle.into_read_parts());

        let dataset = with_read_budget(
            Box::new(CountingDataset {
                layers: vec![validating_contract()],
                opens: Arc::new(AtomicUsize::new(0)),
            }),
            &opts,
            true,
        );

        // Il posto del pool e' libero prima, occupato durante.
        let posto = match pool_lease(&pool) {
            Ok(lease) => lease,
            Err(error) => unreachable!("il pool ha un posto: {error:?}"),
        };
        drop(posto);
        let reader = dataset.open_layer_reader(&richiesta_completa());
        assert!(reader.is_ok());
        assert!(
            pool_lease(&pool).is_err(),
            "il reader deve tenere la quota di concorrenza del context collegato"
        );
    }
}
