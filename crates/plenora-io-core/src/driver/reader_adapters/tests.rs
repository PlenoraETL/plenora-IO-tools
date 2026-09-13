//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

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
            RecordBatch::try_new(schema.clone(), vec![Arc::new(Int64Array::from(values))]).unwrap();
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

/// Reader che dichiara un totale scelto dal test.
///
/// Serve alle sonde degli inoltri: senza un `accepted_total` osservabile
/// sotto l'adapter, «l'adapter inoltra» non e' distinguibile da «l'adapter
/// restituisce `None` come il default».
struct ReaderConTotale {
    contract: LayerContract,
    totale: Option<u64>,
}

impl ReaderConTotale {
    fn nuovo(totale: Option<u64>) -> Self {
        let schema = Arc::new(Schema::new(vec![Field::new("n", DataType::Int64, false)]));
        Self {
            contract: LayerContract {
                id: LayerId(0),
                name: "totale".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: None,
                },
            },
            totale,
        }
    }
}

impl LayerReader for ReaderConTotale {
    fn contract(&self) -> &LayerContract {
        &self.contract
    }

    fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        Ok(None)
    }

    fn accepted_total(&self) -> Option<u64> {
        self.totale
    }
}

/// I tre adapter di passaggio inoltrano il totale, e non lo inventano.
///
/// Erano tre righe di produzione che **nessuna prova raggiungeva**, e la
/// diagnostica differenziale del checkpoint le ha nominate. Non le raggiunge
/// una conversione vera perche' `BudgetedReader` e' l'adapter piu' esterno e
/// risponde da se': questi tre stanno sotto di lui e nessuno li interroga.
///
/// Che non siano interrogati **oggi** non li rende inerti: il giorno in cui
/// l'ordine di composizione cambia, un inoltro che restituisse `None`
/// farebbe fallire chiuso una conversione con un errore che parla d'altro.
///
/// La sonda verifica i due versi: un totale dichiarato passa, e l'assenza
/// resta assenza. Con il solo primo verso, un inoltro sostituito da
/// `Some(0)` resterebbe verde.
#[test]
fn gli_adapter_di_passaggio_inoltrano_il_totale_accettato() {
    let cancellazione = CancellationToken::new();
    for atteso in [Some(7_u64), None] {
        let con_cancellazione = with_cancellation(
            Box::new(ReaderConTotale::nuovo(atteso)),
            cancellazione.clone(),
        );
        assert_eq!(
            con_cancellazione.accepted_total(),
            atteso,
            "CancellationReader non inoltra {atteso:?}"
        );

        let con_target = with_batch_target(
            Box::new(ReaderConTotale::nuovo(atteso)),
            BatchTarget::default(),
            cancellazione.clone(),
        );
        assert_eq!(
            con_target.accepted_total(),
            atteso,
            "BatchTargetReader non inoltra {atteso:?}"
        );

        let gate = SingleReaderGate::new("prova");
        let con_gate = gate
            .open(LayerId(0), || Ok(Box::new(ReaderConTotale::nuovo(atteso))))
            .expect("il gate apre il primo reader");
        assert_eq!(
            con_gate.accepted_total(),
            atteso,
            "SingleActiveLayerReader non inoltra {atteso:?}"
        );
    }
}

/// Il default del tratto: un reader che non lo implementa dice `None`.
///
/// E' l'altra riga che il differenziale ha trovato scoperta, ed e' quella
/// che rende sicuro l'inoltro: senza un default che ammette di non sapere,
/// ogni driver dovrebbe inventarsi un totale.
#[test]
fn un_reader_che_non_dichiara_il_totale_risponde_none() {
    let reader = OneBatchReader::new(vec![1, 2, 3]);
    assert_eq!(reader.accepted_total(), None);
}

/// Dopo la cancellazione il totale non sopravvive al reader che lo diceva.
#[test]
fn il_totale_sparisce_quando_la_cancellazione_rilascia_il_reader() {
    let cancellazione = CancellationToken::new();
    let mut reader = with_cancellation(
        Box::new(ReaderConTotale::nuovo(Some(3))),
        cancellazione.clone(),
    );
    assert_eq!(reader.accepted_total(), Some(3));

    cancellazione.cancel();
    assert!(reader.next_batch().is_err());
    assert_eq!(
            reader.accepted_total(),
            None,
            "un totale che sopravvive al reader che lo produceva descrive righe che nessuno consegnera'"
        );
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
    let terminal =
        PlenoraIoError::cancelled(ErrorPhase::Read, false).with_row_diagnostics(driver_diagnostics);
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
        RecordBatch::try_new(schema, vec![Arc::new(UInt8Array::from(vec![0_u8; LARGE]))]).unwrap();
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
