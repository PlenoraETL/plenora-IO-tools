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

    /// Il totale e' un fatto **solo** dopo un drenaggio concluso senza errore
    /// terminale.
    ///
    /// `rows_scanned` e le righe consegnate coincidono per costruzione: quando
    /// l'accumulatore delle violazioni resta vuoto ogni batch contato entra
    /// nello spool, e lo spool consegna esattamente cio' che ha ricevuto.
    /// Quando invece una violazione emerge, il drenaggio finisce in `Err` e
    /// `terminal_error` e' impostato: qui si torna a `None`, perche' un totale
    /// che sopravvive all'errore che lo invalida e' peggio di nessun totale.
    ///
    /// Vale anche per una sorgente vuota, dove il drenaggio e' concluso e il
    /// totale e' `Some(0)`: distinguere "zero righe" da "non lo so" e'
    /// precisamente cio' che serve a chi deve dichiarare la cardinalita' prima
    /// di scrivere.
    fn accepted_total(&self) -> Option<u64> {
        (self.drained && self.terminal_error.is_none()).then_some(self.rows_scanned)
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

    /// Inoltra: questo adapter non cambia quante righe passano, e un totale
    /// inventato qui sarebbe una seconda verita'. Dopo la cancellazione il
    /// reader sottostante non c'e' piu' e la risposta torna `None`, che e'
    /// esatta — quel totale non descrive piu' nulla che verra' consegnato.
    fn accepted_total(&self) -> Option<u64> {
        self.inner.as_ref().and_then(|inner| inner.accepted_total())
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

    /// Inoltra: il ritaglio cambia **quanti batch** escono, non quante righe.
    fn accepted_total(&self) -> Option<u64> {
        self.inner.accepted_total()
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

    /// Inoltra: la gate sull'unicita' del reader non tocca la cardinalita'.
    fn accepted_total(&self) -> Option<u64> {
        self.inner.accepted_total()
    }

    fn loss_report(&self) -> LossReport {
        self.inner.loss_report()
    }
}

#[cfg(test)]
mod tests;
