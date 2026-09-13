//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

use super::*;

/// Il ramo di ritentativo di `try_take_bounded`, **deterministicamente**.
///
/// Non prova a coprire una riga: prova la proprieta' per cui il ciclo
/// esiste. Calcolare il consumo proiettato con una `load` e prelevare con
/// uno scambio separato lascerebbe una finestra fra i due, e due richieste
/// concorrenti potrebbero superare entrambe il controllo del tetto. Qui la
/// prima proiezione passa, un prelievo concorrente cambia il residuo, e il
/// ritentativo deve **riproiettare il tetto sul valore nuovo** e rifiutare.
///
/// Senza la riproiezione questo prelievo riuscirebbe, e il tetto verrebbe
/// sforato senza che nessuno lo veda.
#[test]
fn il_ritentativo_riproietta_il_tetto_sul_valore_osservato() {
    let gauge = Gauge::new(100);
    // Prima proiezione: consumato 0, richiesti 30, tetto 50 — passa.
    // Il prelievo concorrente porta il consumato a 40, quindi la proiezione
    // del ritentativo vale 70 e supera il tetto.
    arma_interferenza(InterferenzaConcorrente::Sottrae(40));

    assert_eq!(gauge.try_take_bounded(30, 50), TakeOutcome::AboveCeiling);
    assert_eq!(
        gauge.remaining(),
        60,
        "il prelievo rifiutato non deve aver tolto niente: restano i 40 \
             del prelievo concorrente"
    );
}

/// Il ritentativo di `allocate_pipeline_id`, **deterministicamente**.
///
/// Prova che due allocazioni concorrenti non ricevono lo stesso
/// identificatore: se il ciclo restituisse il valore **osservato prima**
/// dello scambio invece di quello nuovo, il chiamante che perde la corsa
/// tornerebbe con l'id gia' assegnato all'altro.
#[test]
fn il_ritentativo_non_riassegna_un_identificatore_gia_preso() {
    let contatore = AtomicU64::new(7);
    // Un'allocazione concorrente si prende il 7 mentre stiamo per prenderlo.
    arma_interferenza(InterferenzaConcorrente::Aggiunge(1));

    let assegnato = allocate_pipeline_id(&contatore).expect("identificatore");

    assert_eq!(assegnato, 8, "il 7 se l'e' preso l'allocazione concorrente");
    assert_eq!(contatore.load(Ordering::Acquire), 9);
}

/// Il ritentativo di `Gauge::try_take`, **deterministicamente**.
///
/// Prova che il ciclo non preleva piu' di quanto resti: la sottrazione va
/// ricontrollata sul valore nuovo, altrimenti il prelievo passerebbe sul
/// residuo vecchio e il gauge andrebbe sotto zero.
#[test]
fn il_ritentativo_non_preleva_piu_di_quanto_resti() {
    let gauge = Gauge::new(100);
    // Un prelievo concorrente da 80 lascia 20: i 30 richiesti non ci stanno
    // piu', anche se ci stavano al momento della prima osservazione.
    arma_interferenza(InterferenzaConcorrente::Sottrae(80));

    assert!(!gauge.try_take(30));
    assert_eq!(gauge.remaining(), 20, "il rifiuto non deve togliere niente");
}

/// Il ritentativo di `Gauge::give_back`, **deterministicamente**.
///
/// Prova che la restituzione si somma al valore **osservato**: sommandola
/// a quello vecchio cancellerebbe il prelievo concorrente, e la quota
/// tornerebbe disponibile due volte.
#[test]
fn il_ritentativo_non_cancella_un_prelievo_concorrente() {
    let gauge = Gauge::new(100);
    assert!(gauge.try_take(50));
    // Mentre restituiamo i 50, un altro ne preleva 30.
    arma_interferenza(InterferenzaConcorrente::Sottrae(30));

    gauge.give_back(50);

    assert_eq!(
        gauge.remaining(),
        70,
        "100 meno i 30 del prelievo concorrente: i nostri 50 tornano, i suoi no"
    );
}

/// La controprova: senza interferenza lo stesso prelievo riesce.
///
/// Senza, «rifiuta sempre» supererebbe la sonda precedente, e il tetto
/// riproiettato non sarebbe distinguibile da un tetto sempre superato.
#[test]
fn senza_prelievo_concorrente_lo_stesso_prelievo_riesce() {
    let gauge = Gauge::new(100);
    assert_eq!(gauge.try_take_bounded(30, 50), TakeOutcome::Taken);
    assert_eq!(gauge.remaining(), 70);
}

/// Il ritentativo consegna quando il tetto regge anche sul valore nuovo.
///
/// Distingue «ha ritentato» da «ha rifiutato»: senza questa, un ciclo che
/// dopo un fallimento restituisse sempre `AboveCeiling` passerebbe.
#[test]
fn il_ritentativo_consegna_se_il_tetto_regge_ancora() {
    let gauge = Gauge::new(100);
    arma_interferenza(InterferenzaConcorrente::Sottrae(10));

    assert_eq!(gauge.try_take_bounded(30, 50), TakeOutcome::Taken);
    assert_eq!(
        gauge.remaining(),
        60,
        "10 del prelievo concorrente piu' 30 di questo"
    );
}

fn bundle() -> PipelineBundle {
    PipelineBudget::builder()
        .build()
        .expect("il builder di default deve costruire")
}

fn bundle_with(limits: PipelineLimits) -> PipelineBundle {
    PipelineBudget::builder()
        .limits(limits)
        .build()
        .expect("limiti validi devono costruire")
}

const MTIME: Duration = Duration::from_secs(1_700_000_000);

fn entry(path: &[u8]) -> SourceEntry<'_> {
    SourceEntry::file(path, 1_024, Some(UNIX_EPOCH + MTIME))
}

fn sized_entry(path: &[u8], bytes: u64) -> SourceEntry<'_> {
    SourceEntry::file(path, bytes, Some(UNIX_EPOCH + MTIME))
}

/// Enumera le entry indicate e pubblica il footprint.
///
/// Consuma le parti perche' `into_components` e' l'unica via che separa
/// il permit, e restituisce il budget cosi' che il chiamante possa
/// continuare a interrogare lo stesso context.
fn observe(
    parts: ReadBudgetParts,
    entries: &[SourceEntry<'_>],
) -> (OperationBudget, SourceFootprint) {
    let (budget, permit, _atteso) = parts.into_components();
    let permit = permit.expect("il permit deve esserci");
    for visited in entries {
        budget
            .context()
            .note_entry_visited(visited)
            .expect("l'enumerazione deve passare");
    }
    let footprint = budget
        .context()
        .observe_input(permit)
        .expect("l'osservazione deve riuscire");
    (budget, footprint)
}

fn pool(concurrent: u64, memory: u64) -> ResourcePool {
    ResourcePool::builder()
        .concurrent_operations(concurrent)
        .memory_bytes(memory)
        .build()
        .expect("il pool deve costruire")
}

#[test]
fn pipeline_limits_default_has_no_zero_field() {
    let limits = PipelineLimits::default();
    assert!(limits.validate().is_ok());
    assert_ne!(limits.max_input_entries(), 0);
    assert_ne!(limits.max_wkb_depth(), 0);
    assert_ne!(limits.decompression_ratio(), 0);
}

#[test]
fn fluent_setters_replace_only_the_named_quota() {
    let limits = PipelineLimits::default().with_max_rows(7);
    assert_eq!(limits.max_rows(), 7);
    assert_eq!(
        limits.max_columns(),
        PipelineLimits::default().max_columns(),
        "un setter non deve toccare le altre quote"
    );
}

#[test]
fn every_quota_has_a_setter_and_a_getter_that_agree() {
    // INV-1 chiede una quota unica per grandezza, raggiungibile senza
    // struct literal. Il test scrive un valore distinto su ogni quota e
    // rilegge tutte le altre: un setter che scrivesse il campo sbagliato
    // — l'errore che un modello a 14 quote invita a fare — cambierebbe
    // due letture invece di una.
    let limits = PipelineLimits::default()
        .with_max_input_bytes(101)
        .with_max_input_entries(102)
        .with_max_rows(103)
        .with_max_columns(104)
        .with_max_geometry_components(105)
        .with_max_output_bytes(106)
        .with_output_expansion_ratio(107)
        .with_max_wkb_cell_bytes(108)
        .with_max_wkb_components(109)
        .with_max_wkb_depth(110)
        .with_memory_bytes(111)
        .with_spill_bytes(112)
        .with_duration_ms(113)
        .with_decompression_ratio(114);

    assert_eq!(limits.max_input_bytes(), 101);
    assert_eq!(limits.max_input_entries(), 102);
    assert_eq!(limits.max_rows(), 103);
    assert_eq!(limits.max_columns(), 104);
    assert_eq!(limits.max_geometry_components(), 105);
    assert_eq!(limits.max_output_bytes(), 106);
    assert_eq!(limits.output_expansion_ratio(), 107);
    assert_eq!(limits.max_wkb_cell_bytes(), 108);
    assert_eq!(limits.max_wkb_components(), 109);
    assert_eq!(limits.max_wkb_depth(), 110);
    assert_eq!(limits.memory_bytes(), 111);
    assert_eq!(limits.spill_bytes(), 112);
    assert_eq!(limits.duration_ms(), 113);
    assert_eq!(limits.decompression_ratio(), 114);
}

#[test]
fn context_exposes_limits_deadline_and_pool_to_the_drivers() {
    // I driver, da S4, leggono il modello solo attraverso il context
    // ottenuto dalle parti: queste tre letture sono il loro unico
    // accesso a quote, scadenza e pool.
    let shared = pool(4, 2_048);
    let limits = PipelineLimits::default().with_max_columns(77);
    let built = PipelineBudget::builder()
        .limits(limits)
        .resource_pool(shared.clone())
        .build()
        .expect("il builder deve costruire");
    let context = built.context();

    assert_eq!(context.limits().max_columns(), 77);
    assert!(context.remaining_duration().is_some());
    assert!(context.deadline() > Instant::now());
    assert!(!context.cancellation().is_cancelled());
    let attached = context
        .resource_pool()
        .expect("il pool deve essere agganciato");
    assert!(attached.is_same_pool(&shared));
    let before = shared.remaining_spill();
    let lease = context.lease_spill(1_000).expect("la lease deve passare");
    assert_eq!(
        shared.remaining_spill(),
        before - 1_000,
        "lo spill locale consuma anche il gauge del pool"
    );
    drop(lease);
    assert_eq!(shared.remaining_spill(), before);

    let solitary = bundle();
    assert!(solitary.context().resource_pool().is_none());
}

#[test]
fn counted_lease_reports_its_own_amount_and_counter() {
    let parts = bundle().into_read_parts();
    let lease = parts
        .budget()
        .try_lease(OperationCounter::GeometryComponents, 12)
        .expect("la lease deve passare");
    assert_eq!(lease.amount(), 12);
    assert_eq!(lease.counter(), OperationCounter::GeometryComponents);
}

#[test]
fn scan_parts_expose_their_budget() {
    let opened = bundle().into_read_parts();
    let (budget, permit, _atteso) = opened.into_components();
    let permit = permit.expect("il permit deve esserci");
    let footprint = budget
        .context()
        .observe_input(permit)
        .expect("l'osservazione deve riuscire");
    let scan = bundle().into_scan_parts(footprint.snapshot());
    assert_eq!(
        scan.budget().remaining(OperationCounter::Columns),
        PipelineLimits::default().max_columns()
    );
}

#[test]
fn pipeline_builder_yields_opaque_bundle() {
    let bundle = bundle();
    assert_eq!(
        bundle.context().observed_input(),
        ObservedInput::NotObserved
    );
    assert_eq!(bundle.context().entries_visited(), 0);
}

#[test]
fn builder_rejects_zero_limit() {
    let limits = PipelineLimits::default().with_memory_bytes(0);
    assert!(PipelineBudget::builder().limits(limits).build().is_err());
}

#[test]
fn builder_rejects_cell_bytes_above_memory() {
    let limits = PipelineLimits::default()
        .with_memory_bytes(1_024)
        .with_max_wkb_cell_bytes(4_096);
    assert!(PipelineBudget::builder().limits(limits).build().is_err());
}

#[test]
fn context_arc_is_shared_between_split_children() {
    let (read, write) = bundle().into_convert_parts().into_parts();
    assert!(read
        .budget()
        .context()
        .is_same_pipeline(write.budget().context()));
}

#[test]
fn read_and_write_counters_do_not_share_atomic_ptr() {
    let (read, write) = bundle().into_convert_parts().into_parts();
    assert!(!read.budget().shares_counters_with(write.budget()));
    let lease = read
        .budget()
        .try_lease(OperationCounter::Rows, 10)
        .expect("la lease di righe deve passare");
    assert_eq!(
        write.budget().remaining(OperationCounter::Rows),
        PipelineLimits::default().max_rows(),
        "il ramo write non deve vedere il consumo del ramo read"
    );
    drop(lease);
}

#[test]
fn convert_of_n_rows_with_max_rows_n_succeeds() {
    let limits = PipelineLimits::default().with_max_rows(3);
    let (read, write) = bundle_with(limits).into_convert_parts().into_parts();
    for _ in 0..3_u8 {
        read.budget()
            .try_lease(OperationCounter::Rows, 1)
            .expect("le prime N righe devono passare su read")
            .commit(1)
            .expect("il commit deve riuscire");
        write
            .budget()
            .try_lease(OperationCounter::Rows, 1)
            .expect("le prime N righe devono passare su write")
            .commit(1)
            .expect("il commit deve riuscire");
    }
    assert!(read.budget().try_lease(OperationCounter::Rows, 1).is_err());
}

#[test]
fn cancel_pipeline_cancels_both_operation_budgets() {
    let token = CancellationToken::new();
    let built = PipelineBudget::builder()
        .cancellation(token.clone())
        .build()
        .expect("il builder deve costruire");
    let (read, write) = built.into_convert_parts().into_parts();
    token.cancel();
    assert!(read.budget().context().ensure_active().is_err());
    assert!(write.budget().context().ensure_active().is_err());
    assert!(read.budget().try_lease(OperationCounter::Rows, 1).is_err());
}

#[test]
fn deadline_expiry_is_not_conflated_with_cancellation() {
    let expired = bundle_with(PipelineLimits::default().with_duration_ms(1));
    std::thread::sleep(Duration::from_millis(5));
    let error = expired
        .context()
        .ensure_active()
        .expect_err("la deadline deve essere scaduta");
    assert_eq!(error.code, crate::IoErrorCode::LimitExceeded);

    let token = CancellationToken::new();
    let cancelled = PipelineBudget::builder()
        .cancellation(token.clone())
        .build()
        .expect("il builder deve costruire");
    token.cancel();
    let error = cancelled
        .context()
        .ensure_active()
        .expect_err("il token deve essere cancellato");
    assert_eq!(error.code, crate::IoErrorCode::Cancelled);
}

#[test]
fn output_limit_no_expansion_when_not_observed() {
    let limits = PipelineLimits::default()
        .with_max_output_bytes(1_000)
        .with_output_expansion_ratio(3);
    let parts = bundle_with(limits).into_write_parts();
    assert_eq!(parts.budget().output_limit(), 1_000);
}

#[test]
fn output_limit_no_expansion_when_bytes_zero() {
    let limits = PipelineLimits::default()
        .with_max_output_bytes(1_000)
        .with_output_expansion_ratio(3);
    let parts = bundle_with(limits).into_read_parts();
    // Il preflight ha girato — una directory visitata — ma non ha
    // addebitato byte: e' `Bytes(0)`, non `NotObserved`.
    let (budget, _) = observe(parts, &[SourceEntry::directory(b"vuota", None)]);
    assert_eq!(budget.context().observed_input(), ObservedInput::Bytes(0));
    assert_eq!(
        budget.output_limit(),
        1_000,
        "un input vuoto non deve produrre un tetto zero"
    );
}

#[test]
fn output_limit_applies_expansion_when_bytes_positive() {
    let limits = PipelineLimits::default()
        .with_max_output_bytes(1_000)
        .with_output_expansion_ratio(3);
    let parts = bundle_with(limits).into_read_parts();
    let (budget, _) = observe(parts, &[sized_entry(b"a.csv", 100)]);
    assert_eq!(budget.output_limit(), 300);
}

#[test]
fn convert_writer_sees_input_observed_by_reader() {
    let limits = PipelineLimits::default()
        .with_max_output_bytes(1_000)
        .with_output_expansion_ratio(3);
    let (read, write) = bundle_with(limits).into_convert_parts().into_parts();
    observe(read, &[sized_entry(b"a.csv", 100)]);
    assert_eq!(
        write.budget().output_limit(),
        300,
        "il writer legge l'input osservato dal reader nel context condiviso"
    );
}

#[test]
fn observe_input_consumes_permit_and_yields_footprint() {
    let parts = bundle().into_read_parts();
    let (budget, footprint) = observe(
        parts,
        &[
            sized_entry(b"a.csv", 100),
            sized_entry(b"b.csv", 200),
            SourceEntry::directory(b"sub", None),
            sized_entry(b"sub/c.csv", 300),
        ],
    );
    assert_eq!(
        footprint.total_bytes(),
        600,
        "il footprint usa i byte accumulati dalle entry, non un parametro"
    );
    assert_eq!(
        footprint.entries_visited(),
        4,
        "anche la directory conta come entry, pur senza addebitare byte"
    );
    assert_eq!(budget.context().observed_input(), ObservedInput::Bytes(600));
    // Una seconda estrazione non e' piu' scrivibile: `into_components`
    // consuma le parti, quindi l'unicita' del permit e' garantita dal
    // tipo e non da un `None` restituito a runtime.
}

#[test]
fn observe_input_with_permit_from_other_pipeline_is_rejected() {
    let (_budget, permit, _atteso) = bundle().into_read_parts().into_components();
    let permit = permit.expect("il permit deve esserci");
    let target = bundle();
    assert!(target.context().observe_input(permit).is_err());
    assert_eq!(
        target.context().observed_input(),
        ObservedInput::NotObserved
    );
}

#[test]
fn observe_input_err_leaves_observed_input_not_observed() {
    let token = CancellationToken::new();
    let built = PipelineBudget::builder()
        .cancellation(token.clone())
        .build()
        .expect("il builder deve costruire");
    let parts = built.into_read_parts();
    let (budget, permit, _atteso) = parts.into_components();
    let permit = permit.expect("il permit deve esserci");
    token.cancel();
    assert!(budget.context().observe_input(permit).is_err());
    assert_eq!(
        budget.context().observed_input(),
        ObservedInput::NotObserved
    );
}

#[test]
fn write_standalone_parts_carry_no_permit() {
    let parts = bundle().into_write_parts();
    assert_eq!(
        parts.budget().context().observed_input(),
        ObservedInput::NotObserved,
        "senza permit consumato l'input resta non osservato"
    );
}

#[test]
fn scan_parts_carry_expected_snapshot_and_permit() {
    let opened = bundle().into_read_parts();
    let paths: Vec<String> = (0..5_u8)
        .map(|index| format!("parte-{index}.csv"))
        .collect();
    let entries: Vec<SourceEntry<'_>> = paths
        .iter()
        .map(|path| sized_entry(path.as_bytes(), 400))
        .collect();
    let (_budget, footprint) = observe(opened, &entries);

    let scan = bundle().into_scan_parts(footprint.snapshot());
    assert_eq!(scan.expected_footprint().total_bytes(), 2_000);
    assert_eq!(scan.expected_footprint().entries_visited(), 5);
    assert_eq!(scan.expected_footprint().digest(), footprint.digest());

    let read = scan.into_read_budget_parts();
    assert!(
        read.expected_footprint().is_some(),
        "la conversione scan->read deve preservare lo snapshot atteso"
    );
    let (_budget, permit, atteso) = read.into_components();
    assert!(
        permit.is_some(),
        "la conversione scan->read deve preservare il permit"
    );
    assert!(atteso.is_some());
}

#[test]
fn convert_parts_split_into_read_and_write() {
    let (read, write) = bundle().into_convert_parts().into_parts();
    let (_read_budget, permit, _atteso) = read.into_components();
    assert!(permit.is_some(), "il permit viaggia sul ramo read");
    assert_eq!(
        write.budget().remaining(OperationCounter::OutputBytes),
        PipelineLimits::default().max_output_bytes()
    );
}

#[test]
fn directory_scan_with_10001_entries_rejects_with_typed_error() {
    let context = bundle().into_read_parts().budget().context().clone();
    for index in 0..PipelineLimits::default().max_input_entries() {
        let path = format!("layer-{index}.csv");
        context
            .note_entry_visited(&entry(path.as_bytes()))
            .expect("le entry entro il limite devono passare");
    }
    let error = context
        .note_entry_visited(&entry(b"una-di-troppo.csv"))
        .expect_err("l'entry oltre il limite deve fallire");
    assert_eq!(error.code, crate::IoErrorCode::LimitExceeded);
    assert_eq!(
        context.entries_visited(),
        PipelineLimits::default().max_input_entries(),
        "il rifiuto non deve incrementare il contatore"
    );
}

#[test]
fn custom_max_input_entries_is_honored() {
    let limits = PipelineLimits::default().with_max_input_entries(2);
    let bundle = bundle_with(limits);
    let context = bundle.context();
    context
        .note_entry_visited(&entry(b"a.csv"))
        .expect("prima entry");
    context
        .note_entry_visited(&entry(b"b.csv"))
        .expect("seconda entry");
    assert!(context.note_entry_visited(&entry(b"c.csv")).is_err());
}

#[test]
fn memory_lease_is_local_and_enforced_without_pool() {
    let limits = PipelineLimits::default()
        .with_memory_bytes(1_024)
        .with_max_wkb_cell_bytes(1_024);
    let bundle = bundle_with(limits);
    let context = bundle.context();
    let lease = context
        .lease_memory_internal(600)
        .expect("la prima lease deve passare");
    assert_eq!(context.remaining_memory(), 424);
    assert!(context.lease_memory_internal(500).is_err());
    drop(lease);
    assert_eq!(context.remaining_memory(), 1_024);
}

#[test]
fn shrink_to_returns_only_the_excess() {
    let limits = PipelineLimits::default()
        .with_memory_bytes(10_000)
        .with_max_wkb_cell_bytes(10_000);
    let bundle = bundle_with(limits);
    let context = bundle.context();
    let mut lease = context
        .lease_memory_internal(4_000)
        .expect("la prenotazione larga deve passare");
    assert_eq!(context.remaining_memory(), 6_000);

    lease.shrink_to(2_500).expect("la riduzione deve riuscire");
    assert_eq!(lease.bytes(), 2_500);
    assert_eq!(
        context.remaining_memory(),
        7_500,
        "torna solo l'eccedenza, non l'intera prenotazione"
    );

    drop(lease);
    assert_eq!(context.remaining_memory(), 10_000);
}

#[test]
fn shrink_to_refuses_zero_because_a_custodied_batch_always_occupies_something() {
    let limits = PipelineLimits::default()
        .with_memory_bytes(10_000)
        .with_max_wkb_cell_bytes(10_000);
    let bundle = bundle_with(limits);
    let context = bundle.context();
    let mut lease = context
        .lease_memory_internal(4_000)
        .expect("la prenotazione deve passare");
    let errore = lease
        .shrink_to(0)
        .expect_err("zero byte per un batch vivo non e' un'occupazione plausibile");
    assert_eq!(errore.code, crate::IoErrorCode::LimitExceeded);
    assert_eq!(
        lease.bytes(),
        4_000,
        "il rifiuto non deve alterare la lease"
    );
    assert_eq!(context.remaining_memory(), 6_000);
}

#[test]
fn shrink_to_refuses_to_grow_the_reservation() {
    let limits = PipelineLimits::default()
        .with_memory_bytes(10_000)
        .with_max_wkb_cell_bytes(10_000);
    let bundle = bundle_with(limits);
    let mut lease = bundle
        .context()
        .lease_memory_internal(1_000)
        .expect("la prenotazione deve passare");
    // Ingrandire non e' un handoff: sarebbe una seconda prenotazione, che
    // puo' fallire e lasciare il chiamante in uno stato ambiguo.
    assert!(lease.shrink_to(2_000).is_err());
    assert_eq!(lease.bytes(), 1_000);
}

/// L'handoff non deve lasciare alcun istante in cui il batch e' in RAM e
/// non lo conta nessuno.
///
/// Il test modella la pipeline reale: si prenota largo, si riduce alla
/// dimensione vera, si consegna la lease al custode e solo **dopo** si
/// rilascia quella del batch precedente.
///
/// L'osservazione e' mirata alla **fase** di handoff, non a tutta la
/// corsa: fuori da quella fase e' del tutto legittimo che risulti
/// custodito un solo batch. Durante la fase, invece, devono risultare
/// contabilizzati due batch — quello gia' custodito e quello in
/// transito — quindi una richiesta che entrerebbe solo se ne mancasse
/// uno deve fallire sempre. Con un handoff fatto di rilascio e
/// riacquisizione quella richiesta passa, ed e' esattamente cio' che il
/// test rifiuta.
#[test]
fn memory_handoff_leaves_no_unaccounted_window() {
    use std::sync::atomic::{AtomicBool, AtomicU64};

    const CAPACITY: u64 = 1_000_000;
    const RESERVED: u64 = 400_000;
    const ACTUAL: u64 = 250_000;
    // Entra solo se, durante l'handoff, risulta contabilizzato un solo
    // batch invece di due.
    const INTRUSIVA: u64 = CAPACITY - 2 * ACTUAL + 1;

    let limits = PipelineLimits::default()
        .with_memory_bytes(CAPACITY)
        .with_max_wkb_cell_bytes(64 * 1024);
    let bundle = bundle_with(limits);
    let context = bundle.context().clone();

    let in_handoff = AtomicBool::new(false);
    let osservatore_avviato = AtomicBool::new(false);
    let osservatore_fermo = AtomicBool::new(false);
    let fermati = AtomicBool::new(false);
    let intrusioni = AtomicU64::new(0);
    // Tentativi effettuati **dentro** la fase. Senza contarli il test
    // potrebbe passare per non aver mai guardato, che e' il modo piu'
    // silenzioso di non verificare nulla.
    let tentativi = AtomicU64::new(0);

    std::thread::scope(|ambito| {
        let osservatore = context.clone();
        ambito.spawn(|| {
            let contesto = osservatore;
            osservatore_avviato.store(true, Ordering::Release);
            while !fermati.load(Ordering::Acquire) {
                if in_handoff.load(Ordering::Acquire) {
                    tentativi.fetch_add(1, Ordering::AcqRel);
                    if let Ok(intrusa) = contesto.lease_memory_internal(INTRUSIVA) {
                        // Ricontrolla la fase: la prenotazione puo' essere
                        // riuscita subito dopo la sua fine, e sarebbe
                        // legittima.
                        let dentro_la_fase = in_handoff.load(Ordering::Acquire);
                        drop(intrusa);
                        if dentro_la_fase {
                            intrusioni.fetch_add(1, Ordering::AcqRel);
                            // Una sola prova basta: continuare
                            // sottrarrebbe quota al thread principale e
                            // renderebbe la corsa lentissima senza
                            // aggiungere informazione.
                            break;
                        }
                    }
                }
            }
            osservatore_fermo.store(true, Ordering::Release);
        });

        // Senza questa attesa il ciclo puo' concludersi prima che
        // l'osservatore venga schedulato, e il test passerebbe senza aver
        // guardato nulla.
        while !osservatore_avviato.load(Ordering::Acquire) {
            std::thread::yield_now();
        }

        let mut custodito: Option<InternalMemoryLease> = None;
        for _ in 0..2_000_u32 {
            // L'osservatore puo' avere quota in mano: l'acquisizione
            // ritenta invece di fallire, cosi' l'unico segnale del test
            // resta il contatore delle intrusioni.
            let mut lease = loop {
                if let Ok(lease) = context.lease_memory_internal(RESERVED) {
                    break lease;
                }
                std::thread::yield_now();
            };
            // La fase si apre solo quando esiste gia' un batch custodito:
            // alla prima iterazione ce n'e' uno solo, e la richiesta
            // intrusiva passerebbe legittimamente.
            let osservabile = custodito.is_some();
            if osservabile {
                let prima = tentativi.load(Ordering::Acquire);
                in_handoff.store(true, Ordering::Release);
                // Attende che l'osservatore abbia effettivamente provato
                // dentro questa fase: rende la copertura deterministica
                // invece di affidarla allo scheduler.
                while tentativi.load(Ordering::Acquire) == prima {
                    std::thread::yield_now();
                }
            }
            lease.shrink_to(ACTUAL).expect("la riduzione deve riuscire");
            // Il nuovo batch entra in custodia **prima** che esca il
            // precedente: la copertura non si interrompe.
            let precedente = custodito.replace(lease);
            in_handoff.store(false, Ordering::Release);
            drop(precedente);
        }

        // L'ultimo batch va rilasciato dopo che l'osservatore ha smesso.
        fermati.store(true, Ordering::Release);
        while !osservatore_fermo.load(Ordering::Acquire) {
            std::thread::yield_now();
        }
        drop(custodito);
    });

    assert!(
        tentativi.load(Ordering::Acquire) > 0,
        "l'osservatore non ha mai provato dentro la fase: il test non avrebbe verificato nulla"
    );
    assert_eq!(
            intrusioni.load(Ordering::Acquire),
            0,
            "durante l'handoff e' passata una prenotazione che entra solo se un batch non e' contabilizzato"
        );
}

#[test]
fn internal_memory_lease_returns_quota_on_drop() {
    let limits = PipelineLimits::default()
        .with_memory_bytes(2_048)
        .with_max_wkb_cell_bytes(2_048);
    let bundle = bundle_with(limits);
    let context = bundle.context();
    {
        let lease = context
            .lease_memory_internal(1_000)
            .expect("la lease deve passare");
        assert_eq!(lease.bytes(), 1_000);
        assert_eq!(context.remaining_memory(), 1_048);
    }
    assert_eq!(
        context.remaining_memory(),
        2_048,
        "la memoria torna al transfer del batch, cioe' al drop della lease"
    );
}

#[test]
fn spill_lease_returns_quota_on_drop() {
    let limits = PipelineLimits::default().with_spill_bytes(4_096);
    let bundle = bundle_with(limits);
    let context = bundle.context();
    {
        let lease = context.lease_spill(3_000).expect("la lease deve passare");
        assert_eq!(lease.bytes(), 3_000);
        assert_eq!(context.remaining_spill(), 1_096);
    }
    assert_eq!(context.remaining_spill(), 4_096);
}

#[test]
fn memory_lease_uses_min_of_local_and_pool_quota() {
    let shared = pool(64, 800);
    let limits = PipelineLimits::default()
        .with_memory_bytes(1_024)
        .with_max_wkb_cell_bytes(1_024);
    let built = PipelineBudget::builder()
        .limits(limits)
        .resource_pool(shared.clone())
        .build()
        .expect("il builder deve costruire");
    let context = built.context();
    let lease = context
        .lease_memory_internal(700)
        .expect("700 sta sotto entrambe le quote");
    assert_eq!(context.remaining_memory(), 324);
    assert_eq!(shared.remaining_memory(), 100);
    assert!(
        context.lease_memory_internal(200).is_err(),
        "200 sta sotto la quota locale ma non sotto quella del pool"
    );
    drop(lease);
    assert_eq!(context.remaining_memory(), 1_024);
    assert_eq!(shared.remaining_memory(), 800);
}

#[test]
fn memory_lease_rolls_back_local_quota_when_pool_refuses() {
    let shared = pool(64, 100);
    let limits = PipelineLimits::default()
        .with_memory_bytes(4_096)
        .with_max_wkb_cell_bytes(4_096);
    let built = PipelineBudget::builder()
        .limits(limits)
        .resource_pool(shared.clone())
        .build()
        .expect("il builder deve costruire");
    let context = built.context();
    assert!(context.lease_memory_internal(200).is_err());
    assert_eq!(
        context.remaining_memory(),
        4_096,
        "un rifiuto del pool non deve lasciare consumo locale"
    );
    assert_eq!(shared.remaining_memory(), 100);
}

#[test]
fn lease_concurrency_is_noop_without_pool() {
    let bundle = bundle();
    let context = bundle.context();
    let leases: Vec<ConcurrencyLease> = (0..1_000_u16)
        .map(|_| {
            context
                .lease_concurrency()
                .expect("senza pool la concorrenza non e' limitata")
        })
        .collect();
    assert!(leases.iter().all(|lease| !lease.is_counted()));
}

#[test]
fn two_pipelines_sharing_pool_compete_on_concurrency_gauge() {
    let shared = pool(1, 1_024);
    let first = PipelineBudget::builder()
        .resource_pool(shared.clone())
        .build()
        .expect("il builder deve costruire");
    let second = PipelineBudget::builder()
        .resource_pool(shared.clone())
        .build()
        .expect("il builder deve costruire");
    let held = first
        .context()
        .lease_concurrency()
        .expect("il primo slot e' libero");
    assert!(held.is_counted());
    assert!(
        second.context().lease_concurrency().is_err(),
        "la seconda pipeline compete sullo stesso gauge"
    );
    drop(held);
    assert_eq!(shared.remaining_concurrency(), 1);
    assert!(second.context().lease_concurrency().is_ok());
}

#[test]
fn pipeline_without_pool_does_not_count_against_others() {
    let shared = pool(1, 1_024);
    let pooled = PipelineBudget::builder()
        .resource_pool(shared)
        .build()
        .expect("il builder deve costruire");
    let solitary = bundle();
    let _held = solitary
        .context()
        .lease_concurrency()
        .expect("la pipeline senza pool non e' limitata");
    assert!(
        pooled.context().lease_concurrency().is_ok(),
        "una pipeline senza pool non deve consumare la quota condivisa"
    );
}

#[test]
fn counted_lease_commit_returns_only_unused_quota() {
    let limits = PipelineLimits::default().with_max_rows(100);
    let parts = bundle_with(limits).into_read_parts();
    parts
        .budget()
        .try_lease(OperationCounter::Rows, 80)
        .expect("la lease deve passare")
        .commit(30)
        .expect("il commit deve riuscire");
    assert_eq!(parts.budget().remaining(OperationCounter::Rows), 70);
}

#[test]
fn counted_lease_rejects_invalid_commit_and_zero_amount() {
    let parts = bundle().into_read_parts();
    assert!(parts.budget().try_lease(OperationCounter::Rows, 0).is_err());
    let lease = parts
        .budget()
        .try_lease(OperationCounter::Rows, 10)
        .expect("la lease deve passare");
    assert!(lease.commit(11).is_err());
}

#[test]
fn counted_lease_release_and_drop_return_the_whole_quota() {
    let limits = PipelineLimits::default().with_max_rows(50);
    let parts = bundle_with(limits).into_read_parts();
    parts
        .budget()
        .try_lease(OperationCounter::Rows, 20)
        .expect("la lease deve passare")
        .release();
    assert_eq!(parts.budget().remaining(OperationCounter::Rows), 50);
    drop(
        parts
            .budget()
            .try_lease(OperationCounter::Rows, 20)
            .expect("la lease deve passare"),
    );
    assert_eq!(parts.budget().remaining(OperationCounter::Rows), 50);
}

#[test]
fn operation_budget_clone_does_not_double_the_consumption() {
    let limits = PipelineLimits::default().with_max_rows(10);
    let parts = bundle_with(limits).into_read_parts();
    let clone = parts.budget().clone();
    assert!(clone.shares_counters_with(parts.budget()));
    clone
        .try_lease(OperationCounter::Rows, 4)
        .expect("la lease deve passare")
        .commit(4)
        .expect("il commit deve riuscire");
    assert_eq!(parts.budget().remaining(OperationCounter::Rows), 6);
}

#[test]
fn output_bytes_lease_respects_the_expansion_derived_ceiling() {
    let limits = PipelineLimits::default()
        .with_max_output_bytes(10_000)
        .with_output_expansion_ratio(2);
    let parts = bundle_with(limits).into_read_parts();
    let (budget, _) = observe(parts, &[sized_entry(b"a.csv", 100)]);
    assert_eq!(budget.output_limit(), 200);
    budget
        .try_lease(OperationCounter::OutputBytes, 200)
        .expect("il tetto derivato consente 200 byte")
        .commit(200)
        .expect("il commit deve riuscire");
    assert!(
        budget.try_lease(OperationCounter::OutputBytes, 1).is_err(),
        "oltre il tetto derivato la lease deve fallire, non fermarsi al solo limite assoluto"
    );
}

#[test]
fn pool_builder_rejects_zero_quota() {
    assert!(ResourcePool::builder().memory_bytes(0).build().is_err());
    assert!(ResourcePool::builder()
        .concurrent_operations(0)
        .build()
        .is_err());
}

fn digest_of(entries: &[SourceEntry<'_>]) -> SourceFootprintSnapshot {
    let parts = bundle().into_read_parts();
    observe(parts, entries).1.snapshot()
}

#[test]
fn footprint_digest_is_stable_for_the_same_entry_set() {
    let first = digest_of(&[entry(b"a.csv"), entry(b"b.csv")]);
    let second = digest_of(&[entry(b"a.csv"), entry(b"b.csv")]);
    assert_eq!(first.digest(), second.digest());
    assert!(first.matches(&second));
}

#[test]
fn footprint_digest_is_order_insensitive() {
    // L'ordine di enumerazione di una directory non e' stabile: un
    // digest che ne dipendesse segnalerebbe mutazioni inesistenti.
    let ascending = digest_of(&[entry(b"a.csv"), entry(b"b.csv"), entry(b"c.csv")]);
    let descending = digest_of(&[entry(b"c.csv"), entry(b"b.csv"), entry(b"a.csv")]);
    assert_eq!(ascending.digest(), descending.digest());
}

#[test]
fn footprint_digest_detects_added_and_removed_entries() {
    let two = digest_of(&[entry(b"a.csv"), entry(b"b.csv")]);
    let three = digest_of(&[entry(b"a.csv"), entry(b"b.csv"), entry(b"c.csv")]);
    let one = digest_of(&[entry(b"a.csv")]);
    assert_ne!(two.digest(), three.digest(), "un'aggiunta cambia il digest");
    assert_ne!(two.digest(), one.digest(), "una rimozione cambia il digest");
}

#[test]
fn footprint_digest_detects_rename_size_and_mtime() {
    let base = digest_of(&[entry(b"a.csv")]);

    let renamed = digest_of(&[entry(b"a-bis.csv")]);
    assert_ne!(base.digest(), renamed.digest());

    let resized = digest_of(&[sized_entry(b"a.csv", 2_048)]);
    assert_ne!(base.digest(), resized.digest());

    let touched = digest_of(&[SourceEntry::file(
        b"a.csv",
        1_024,
        Some(UNIX_EPOCH + Duration::from_secs(1_700_000_001)),
    )]);
    assert_ne!(base.digest(), touched.digest());

    let without_mtime = digest_of(&[SourceEntry::file(b"a.csv", 1_024, None)]);
    assert_ne!(base.digest(), without_mtime.digest());
}

#[test]
fn footprint_digest_separates_paths_that_share_a_concatenation() {
    // Senza la lunghezza in testa alla codifica, "ab" + "c" e "a" + "bc"
    // darebbero la stessa sequenza di byte.
    let first = digest_of(&[entry(b"ab"), entry(b"c")]);
    let second = digest_of(&[entry(b"a"), entry(b"bc")]);
    assert_ne!(first.digest(), second.digest());
}

#[test]
fn snapshot_matches_only_when_bytes_entries_and_digest_agree() {
    let base = digest_of(&[entry(b"a.csv")]);
    let other_bytes = digest_of(&[sized_entry(b"a.csv", 2_048)]);
    let other_entries = digest_of(&[entry(b"a.csv"), entry(b"b.csv")]);
    assert_ne!(base.total_bytes(), other_bytes.total_bytes());
    assert!(
        !base.matches(&other_bytes),
        "i byte addebitati fanno parte del confronto"
    );
    assert!(!base.matches(&other_entries));
}

#[test]
fn snapshot_roundtrips_through_serde_without_losing_the_digest() {
    let snapshot = digest_of(&[entry(b"a.csv"), entry(b"b.csv")]);
    let encoded = serde_json::to_string(&snapshot).expect("serializzabile");
    let decoded: SourceFootprintSnapshot =
        serde_json::from_str(&encoded).expect("deserializzabile");
    assert!(snapshot.matches(&decoded));
}

#[test]
fn rejected_entry_does_not_enter_the_digest() {
    let limits = PipelineLimits::default().with_max_input_entries(1);
    let parts = bundle_with(limits).into_read_parts();
    let (budget, permit, _atteso) = parts.into_components();
    let permit = permit.expect("il permit deve esserci");
    let context = budget.context();
    context
        .note_entry_visited(&entry(b"a.csv"))
        .expect("prima entry");
    assert!(context.note_entry_visited(&entry(b"b.csv")).is_err());
    let observed = context
        .observe_input(permit)
        .expect("l'osservazione deve riuscire")
        .snapshot();
    assert!(
        observed.matches(&digest_of(&[entry(b"a.csv")])),
        "l'entry rifiutata non deve lasciare traccia nel digest"
    );
}

#[test]
fn entry_beyond_max_input_bytes_is_rejected() {
    let limits = PipelineLimits::default().with_max_input_bytes(1_000);
    let bundle = bundle_with(limits);
    let context = bundle.context();
    context
        .note_entry_visited(&sized_entry(b"a.csv", 600))
        .expect("la prima entry sta sotto il limite");
    let error = context
        .note_entry_visited(&sized_entry(b"b.csv", 500))
        .expect_err("la somma supera il limite");
    assert_eq!(error.code, crate::IoErrorCode::LimitExceeded);
}

#[test]
fn directories_count_as_entries_without_charging_bytes() {
    let limits = PipelineLimits::default().with_max_input_bytes(10);
    let bundle = bundle_with(limits);
    let context = bundle.context();
    for index in 0..50_u8 {
        let path = format!("livello-{index}");
        context
            .note_entry_visited(&SourceEntry::directory(path.as_bytes(), None))
            .expect("una directory non addebita byte");
    }
    assert_eq!(context.entries_visited(), 50);
    assert_eq!(context.charged_input_bytes(), 0);
}

#[test]
fn rejected_entry_leaves_no_partial_update() {
    // Il rifiuto deve essere totale: ne' conteggio, ne' byte, ne' digest.
    // Un aggiornamento parziale renderebbe il footprint successivo una
    // descrizione di un insieme che nessuno ha osservato.
    let limits = PipelineLimits::default()
        .with_max_input_entries(2)
        .with_max_input_bytes(1_000);
    let bundle = bundle_with(limits);
    let context = bundle.context();
    context
        .note_entry_visited(&sized_entry(b"a.csv", 400))
        .expect("prima entry");

    // Rifiuto per byte.
    assert!(context
        .note_entry_visited(&sized_entry(b"grande.csv", 700))
        .is_err());
    assert_eq!(context.entries_visited(), 1);
    assert_eq!(context.charged_input_bytes(), 400);

    context
        .note_entry_visited(&sized_entry(b"b.csv", 100))
        .expect("seconda entry");

    // Rifiuto per numero di entry.
    assert!(context
        .note_entry_visited(&sized_entry(b"c.csv", 1))
        .is_err());
    assert_eq!(context.entries_visited(), 2);
    assert_eq!(context.charged_input_bytes(), 500);

    // Una pipeline che vede solo le due entry accettate deve produrre
    // esattamente lo stesso footprint: i rifiuti non lasciano traccia.
    let pulita = bundle_with(limits).into_read_parts();
    let (_budget_pulito, atteso) = observe(
        pulita,
        &[sized_entry(b"a.csv", 400), sized_entry(b"b.csv", 100)],
    );
    let permit = InputPermit {
        pipeline_id: context.inner.pipeline_id,
    };
    let osservato = context
        .observe_input(permit)
        .expect("l'osservazione deve riuscire");
    assert_eq!(atteso, osservato);
}

#[test]
fn note_entry_visited_after_publication_is_rejected() {
    let parts = bundle().into_read_parts();
    let (budget, footprint) = observe(parts, &[sized_entry(b"a.csv", 10)]);
    let context = budget.context();
    let error = context
        .note_entry_visited(&sized_entry(b"tardiva.csv", 10))
        .expect_err("dopo la pubblicazione l'insieme e' chiuso");
    assert_eq!(error.code, crate::IoErrorCode::LimitExceeded);
    assert_eq!(
        context.charged_input_bytes(),
        footprint.total_bytes(),
        "l'entry tardiva non deve alterare il footprint gia' pubblicato"
    );
    assert_eq!(context.entries_visited(), footprint.entries_visited());
}

#[test]
fn second_observation_is_rejected_and_keeps_the_published_footprint() {
    let parts = bundle().into_read_parts();
    let (budget, first) = observe(parts, &[sized_entry(b"a.csv", 10)]);
    // Un permit di questa stessa pipeline, ottenuto per altra via, non
    // deve poter ripubblicare: la transizione e' terminale.
    let second = PipelineBudget::builder().build().expect("costruito");
    let (_altro_budget, foreign, _atteso) = second.into_read_parts().into_components();
    let foreign = foreign.expect("permit");
    assert!(budget.context().observe_input(foreign).is_err());
    // E' il caso che smentiva la vecchia doc di `observe_input`: dopo un
    // secondo publish fallito lo stato **non** torna a `Collecting`,
    // resta `Published` con il footprint gia' registrato.
    assert_eq!(
        budget.context().observed_input(),
        ObservedInput::Bytes(first.total_bytes())
    );
}

#[test]
fn output_bytes_ceiling_holds_under_concurrent_requests() {
    // Il tetto derivato e' molto piu' stretto del limite assoluto: se
    // proiezione e prelievo non avvenissero sulla stessa osservazione
    // atomica, piu' richieste concorrenti potrebbero superarlo insieme
    // senza che nessuna se ne accorga.
    const CEILING: u64 = 200;
    const CHUNK: u64 = 7;
    let limits = PipelineLimits::default()
        .with_max_output_bytes(1_000_000)
        .with_output_expansion_ratio(2);
    let parts = bundle_with(limits).into_read_parts();
    let (budget, _) = observe(parts, &[sized_entry(b"a.csv", 100)]);
    assert_eq!(budget.output_limit(), CEILING);

    let concessi = std::sync::atomic::AtomicU64::new(0);
    std::thread::scope(|scope| {
        for _ in 0..8_u8 {
            scope.spawn(|| {
                for _ in 0..64_u8 {
                    if let Ok(lease) = budget.try_lease(OperationCounter::OutputBytes, CHUNK) {
                        concessi.fetch_add(CHUNK, std::sync::atomic::Ordering::AcqRel);
                        lease.commit(CHUNK).expect("commit");
                    }
                }
            });
        }
    });

    let totale = concessi.load(std::sync::atomic::Ordering::Acquire);
    let consumato = 1_000_000 - budget.remaining(OperationCounter::OutputBytes);
    assert!(
        totale <= CEILING,
        "concesso {totale} oltre il tetto derivato {CEILING}"
    );
    assert_eq!(totale, consumato, "consumo e concessioni devono coincidere");
    assert!(totale > 0, "almeno una richiesta deve passare");
    assert!(
        budget
            .try_lease(OperationCounter::OutputBytes, CHUNK)
            .is_err()
            || totale + CHUNK <= CEILING,
        "oltre il tetto nessuna richiesta ulteriore deve passare"
    );
}

#[test]
fn pipeline_id_allocation_fails_closed_instead_of_wrapping() {
    let counter = AtomicU64::new(u64::MAX - 1);
    assert_eq!(
        allocate_pipeline_id(&counter).expect("l'ultimo id disponibile"),
        u64::MAX - 1
    );
    // Il contatore ora vale u64::MAX: non c'e' un successivo, e avvolgere
    // riassegnerebbe identita' gia' consegnate.
    let error = allocate_pipeline_id(&counter).expect_err("niente wrap");
    assert_eq!(error.code, crate::IoErrorCode::LimitExceeded);
    assert_eq!(counter.load(Ordering::Acquire), u64::MAX);
}

#[test]
fn effective_wkb_components_is_tightened_by_max_vertices() {
    // `--max-vertices` e' un flag vivo della CLI: il tetto per cella dei
    // componenti deve restare composto con esso, o un utente che stringe
    // quel flag non otterrebbe nulla.
    let limits = PipelineLimits::default()
        .with_max_wkb_components(10)
        .with_max_vertices(3);
    assert_eq!(limits.effective_wkb_components(), 3);

    // E il verso opposto: quando il tetto per cella e' il piu' stretto,
    // e' lui a vincere.
    let limits = PipelineLimits::default()
        .with_max_wkb_components(3)
        .with_max_vertices(10);
    assert_eq!(limits.effective_wkb_components(), 3);
}

/// I default del modello unificato, fissati ai valori attesi.
///
/// Fino a S4.d questo test confrontava i default con quelli dei **due**
/// modelli legacy, e la regola di unificazione — vince il piu' stretto —
/// era verificata contro le loro strutture. Rimossi quei tipi (S4.e), il
/// confronto non e' scrivibile, ma il requisito resta: un allentamento
/// silenzioso di una di queste quote riaprirebbe il finding L0.2 senza
/// che nulla lo veda.
///
/// I valori sono percio' fissati qui, con l'origine accanto. Cambiarli e'
/// legittimo; cambiarli **senza accorgersene** no.
#[test]
fn unified_defaults_stay_at_the_tightest_historical_values() {
    let unified = PipelineLimits::default();

    // Dal vecchio `Limits`, che era il piu' stretto dei due.
    assert_eq!(unified.max_input_bytes(), 268_435_456);
    assert_eq!(unified.max_rows(), 10_000_000);
    assert_eq!(unified.max_columns(), 4_096);
    assert_eq!(unified.max_output_bytes(), 1_073_741_824);
    assert_eq!(unified.max_vertices(), 50_000_000);
    assert_eq!(unified.max_wkb_components(), 100_000);
    assert_eq!(unified.max_wkb_depth(), 64);
    assert_eq!(unified.max_wkb_cell_bytes(), 64 * 1024 * 1024);

    // Dal vecchio `ResourceLimits`, unico a portarle.
    assert_eq!(unified.max_geometry_components(), 16_777_216);
    assert_eq!(unified.memory_bytes(), 512 * 1024 * 1024);
    assert_eq!(unified.spill_bytes(), 4 * 1024 * 1024 * 1024);
    assert_eq!(unified.duration_ms(), 30_000);
    assert_eq!(unified.decompression_ratio(), 1_000);
    assert_eq!(unified.output_expansion_ratio(), 1_000);

    // Nuova in S1 (INV-9): una directory di migliaia di file legittimi
    // passa, uno scan illimitato no.
    assert_eq!(unified.max_input_entries(), 10_000);
}

#[test]
fn distinct_pipelines_have_distinct_identities() {
    let first = bundle();
    let second = bundle();
    let handle = first.context().clone();
    assert!(first.context().is_same_pipeline(&handle));
    assert!(!first.context().is_same_pipeline(second.context()));
}
