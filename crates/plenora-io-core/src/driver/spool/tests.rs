//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

use std::sync::Arc;

use arrow_array::{Int64Array, StringArray};
use arrow_schema::{DataType, Field, Schema};

use super::*;
use plenora_io_model::budget::{PipelineBudget, PipelineLimits};

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

fn schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("nome", DataType::Utf8, true),
    ]))
}

fn batch(schema: &SchemaRef, base: i64, rows: i64) -> RecordBatch {
    let ids: Vec<i64> = (base..base + rows).collect();
    let nomi: Vec<String> = ids.iter().map(|id| format!("riga-{id}")).collect();
    match RecordBatch::try_new(
        Arc::clone(schema),
        vec![
            Arc::new(Int64Array::from(ids)),
            Arc::new(StringArray::from(nomi)),
        ],
    ) {
        Ok(batch) => batch,
        Err(error) => unreachable!("batch di test non costruibile: {error}"),
    }
}

fn budget_di(memory_bytes: u64, spill_bytes: u64) -> OperationBudget {
    budget_con(
        PipelineLimits::default()
            .with_memory_bytes(memory_bytes)
            .with_spill_bytes(spill_bytes)
            .with_max_wkb_cell_bytes(
                usize::try_from(memory_bytes.min(64 * 1024 * 1024)).unwrap_or(usize::MAX),
            ),
    )
}

/// Spinge un batch nello spool come fa l'adapter: prenota la memoria,
/// la dimensiona all'ingombro contabilizzato, e cede la lease per `move`.
///
/// I test passano i byte del *payload*; l'ingombro strutturale lo aggiunge
/// qui, nello stesso punto in cui lo aggiunge il percorso reale. Chiamare
/// `push` con una lease costruita altrove renderebbe i test verdi su una
/// contabilita' che il codice di produzione non usa.
fn spingi(
    spool: &mut StagedSpool,
    budget: &OperationBudget,
    batch: RecordBatch,
    payload_bytes: u64,
) -> Result<()> {
    let lease = budget
        .context()
        .lease_memory_internal(payload_bytes.saturating_add(PER_BATCH_OVERHEAD_BYTES))?;
    spool.push(batch, lease)
}

fn drain(spool: &mut StagedSpool) -> Vec<RecordBatch> {
    let mut raccolti = Vec::new();
    while let Some(batch) = spool.next_batch().expect("rilettura") {
        raccolti.push(batch);
    }
    raccolti
}

#[test]
fn under_threshold_the_spool_stays_in_memory() {
    let schema = schema();
    let budget = budget_di(1 << 20, 1 << 20);
    let mut spool = StagedSpool::with_threshold(
        Arc::clone(&schema),
        budget.clone(),
        4 * PER_BATCH_OVERHEAD_BYTES,
    );
    spingi(&mut spool, &budget, batch(&schema, 0, 4), 100).expect("push");
    spingi(&mut spool, &budget, batch(&schema, 4, 4), 100).expect("push");
    assert!(!spool.spilled());
    assert_eq!(
        spool.buffered_memory_bytes(),
        2 * (100 + PER_BATCH_OVERHEAD_BYTES)
    );
    spool.seal().expect("seal");
    assert_eq!(drain(&mut spool).len(), 2);
}

#[test]
fn crossing_the_threshold_migrates_to_disk_and_preserves_order() {
    let schema = schema();
    let budget = budget_di(1 << 20, 1 << 20);
    let mut spool = StagedSpool::with_threshold(
        Arc::clone(&schema),
        budget.clone(),
        2 * PER_BATCH_OVERHEAD_BYTES,
    );
    for indice in 0..6_i64 {
        spingi(&mut spool, &budget, batch(&schema, indice * 4, 4), 100).expect("push");
    }
    assert!(spool.spilled(), "oltre soglia i batch devono migrare");
    spool.seal().expect("seal");
    let raccolti = drain(&mut spool);
    assert_eq!(raccolti.len(), 6);
    for (indice, batch) in raccolti.iter().enumerate() {
        let colonna = batch
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .expect("colonna id");
        let atteso = i64::try_from(indice).expect("indice") * 4;
        assert_eq!(colonna.value(0), atteso, "l'ordine deve essere preservato");
    }
}

#[test]
fn migration_returns_the_memory_it_was_holding() {
    // E' il cuore di L0.3: la memoria dei batch migrati deve tornare al
    // budget, altrimenti lo spool sposta i byte su disco ma continua a
    // pagarli in RAM.
    let schema = schema();
    let budget = budget_di(10_000, 1 << 24);
    let mut spool = StagedSpool::with_threshold(
        Arc::clone(&schema),
        budget.clone(),
        2 * PER_BATCH_OVERHEAD_BYTES,
    );
    spingi(&mut spool, &budget, batch(&schema, 0, 4), 100).expect("push");
    assert_eq!(
        budget.context().remaining_memory(),
        10_000 - (100 + PER_BATCH_OVERHEAD_BYTES)
    );
    spingi(&mut spool, &budget, batch(&schema, 4, 4), 100).expect("push");
    assert!(spool.spilled());
    assert_eq!(
        budget.context().remaining_memory(),
        10_000,
        "dopo la migrazione nessun batch trattiene memoria"
    );
    assert_eq!(spool.buffered_memory_bytes(), 0);
}

#[test]
fn delivering_a_batch_returns_its_memory() {
    let schema = schema();
    let budget = budget_di(10_000, 1 << 20);
    let mut spool = StagedSpool::with_threshold(Arc::clone(&schema), budget.clone(), 10_000);
    let primo = 400 + PER_BATCH_OVERHEAD_BYTES;
    let secondo = 600 + PER_BATCH_OVERHEAD_BYTES;
    spingi(&mut spool, &budget, batch(&schema, 0, 4), 400).expect("push");
    spingi(&mut spool, &budget, batch(&schema, 4, 4), 600).expect("push");
    assert_eq!(
        budget.context().remaining_memory(),
        10_000 - primo - secondo
    );
    spool.seal().expect("seal");
    let _consegnato = spool.next_batch().expect("primo");
    assert_eq!(
        budget.context().remaining_memory(),
        10_000 - secondo,
        "la memoria torna al transfer del batch, non alla fine dell'operazione"
    );
    assert_eq!(spool.buffered_memory_bytes(), secondo);
}

#[test]
fn a_dataset_larger_than_the_memory_quota_still_completes() {
    // Prima dello spool questo caso falliva `LimitExceeded`: i batch
    // verificati restavano tutti in RAM.
    let schema = schema();
    let budget = budget_di(4_096, 1 << 20);
    let mut spool = StagedSpool::new(
        Arc::clone(&schema),
        budget.clone(),
        CancellationToken::default(),
    );
    for indice in 0..64_i64 {
        spingi(&mut spool, &budget, batch(&schema, indice * 8, 8), 1_024)
            .expect("un dataset oltre la quota di memoria deve passare");
    }
    spool.seal().expect("seal");
    assert_eq!(drain(&mut spool).len(), 64);
}

#[test]
fn spill_quota_is_enforced() {
    let schema = schema();
    let budget = budget_di(1 << 20, 500);
    let mut spool = StagedSpool::with_threshold(
        Arc::clone(&schema),
        budget.clone(),
        2 * PER_BATCH_OVERHEAD_BYTES,
    );
    // La quota si applica alle scritture fisiche, che il buffer
    // differisce: il rifiuto arriva quando i byte raggiungono davvero il
    // file, non quando il batch entra nel writer. E' il punto: prima si
    // rifiutava una stima, ora si rifiuta cio' che sta per essere scritto.
    let esito = spingi(&mut spool, &budget, batch(&schema, 0, 4), 200)
        .and_then(|()| spingi(&mut spool, &budget, batch(&schema, 4, 4), 400))
        .and_then(|()| spool.seal());
    let errore = esito.expect_err("lo spill oltre quota deve fallire");
    assert_eq!(errore.code, plenora_io_model::IoErrorCode::LimitExceeded);
    assert!(
        spool.written_spill_bytes() <= 500,
        "scritti {} byte con una quota di 500",
        spool.written_spill_bytes()
    );
}

#[test]
fn a_batch_with_a_foreign_schema_is_rejected() {
    let schema = schema();
    let altro = Arc::new(Schema::new(vec![Field::new("x", DataType::Int64, false)]));
    let estraneo = match RecordBatch::try_new(
        Arc::clone(&altro),
        vec![Arc::new(Int64Array::from(vec![1_i64]))],
    ) {
        Ok(batch) => batch,
        Err(error) => unreachable!("batch di test: {error}"),
    };
    let budget = budget_di(1 << 20, 1 << 20);
    let mut spool = StagedSpool::new(schema, budget.clone(), CancellationToken::default());
    assert!(spingi(&mut spool, &budget, estraneo, 10).is_err());
}

#[test]
fn pushing_after_seal_is_rejected() {
    let schema = schema();
    let budget = budget_di(1 << 20, 1 << 20);
    let mut spool = StagedSpool::new(
        Arc::clone(&schema),
        budget.clone(),
        CancellationToken::default(),
    );
    spool.seal().expect("seal");
    assert!(spingi(&mut spool, &budget, batch(&schema, 0, 4), 10).is_err());
}

#[test]
fn reading_before_seal_is_rejected() {
    let schema = schema();
    let budget = budget_di(1 << 20, 1 << 20);
    let mut spool = StagedSpool::new(
        Arc::clone(&schema),
        budget.clone(),
        CancellationToken::default(),
    );
    spingi(&mut spool, &budget, batch(&schema, 0, 4), 10).expect("push");
    assert!(spool.next_batch().is_err());
}

#[test]
fn clear_releases_every_reservation() {
    let schema = schema();
    let budget = budget_di(10_000, 1 << 20);
    let mut spool = StagedSpool::with_threshold(Arc::clone(&schema), budget.clone(), 10_000);
    spingi(&mut spool, &budget, batch(&schema, 0, 4), 500).expect("push");
    assert_eq!(
        budget.context().remaining_memory(),
        10_000 - (500 + PER_BATCH_OVERHEAD_BYTES)
    );
    spool.clear();
    assert_eq!(
        budget.context().remaining_memory(),
        10_000,
        "una violazione a meta' scansione non deve lasciare memoria prenotata"
    );
}

/// INV-8: un errore di replay dopo la validazione deve essere un errore
/// tipizzato, non un panico ne' una fine silenziosa che farebbe passare
/// per completa una lettura troncata.
#[test]
fn a_corrupted_spool_fails_typed_instead_of_truncating_silently() {
    use std::io::Write as _;

    let schema = schema();
    // Un preambolo IPC valido seguito da spazzatura: il reader si
    // costruisce, poi inciampa durante la rilettura.
    let mut file = tempfile::tempfile().expect("file temporaneo");
    {
        let mut writer = StreamWriter::try_new(&mut file, schema.as_ref()).expect("writer IPC");
        writer.write(&batch(&schema, 0, 4)).expect("primo batch");
        writer.flush().expect("flush");
    }
    file.write_all(&[0xFF; 64]).expect("coda corrotta");
    file.seek(SeekFrom::Start(0)).expect("riavvolgimento");

    let mut spool = StagedSpool::replaying_from(schema, budget_di(1 << 20, 1 << 20), file)
        .expect("il preambolo e' valido");
    assert!(
        spool
            .next_batch()
            .expect("il primo batch e' integro")
            .is_some(),
        "la corruzione arriva dopo un batch valido: e' il caso che INV-8 descrive"
    );
    let errore = spool
        .next_batch()
        .expect_err("la coda corrotta deve produrre un errore tipizzato");
    assert!(matches!(
        errore.code,
        plenora_io_model::IoErrorCode::Io | plenora_io_model::IoErrorCode::Contract
    ));
}

#[test]
fn spill_quota_follows_the_bytes_actually_written() {
    // La quota di spill deve seguire cio' che finisce su disco, non la
    // stima di occupazione in RAM: le due grandezze divergono, e
    // contabilizzare la seconda dichiarerebbe un'occupazione del volume
    // che non corrisponde a quella reale.
    let schema = schema();
    let budget = budget_di(1 << 20, 1 << 24);
    let mut spool = StagedSpool::with_threshold(
        Arc::clone(&schema),
        budget.clone(),
        2 * PER_BATCH_OVERHEAD_BYTES,
    );
    for indice in 0..8_i64 {
        spingi(&mut spool, &budget, batch(&schema, indice * 4, 4), 100).expect("push");
    }
    assert!(spool.spilled());
    // I byte fisici compaiono quando il buffer li consegna: il sigillo
    // forza il flush, quindi e' li' che la contabilita' e' completa.
    spool.seal().expect("seal");
    let scritti = spool.written_spill_bytes();
    assert!(scritti > 0, "l'IPC deve aver prodotto byte reali");
    assert!(
        spool.reserved_spill() >= scritti,
        "prenotato {} < scritto {scritti}: la quota non copre il file",
        spool.reserved_spill()
    );
}

#[test]
fn an_underestimated_batch_cannot_write_beyond_the_quota() {
    // La stima passata a `push` e' deliberatamente ridicola rispetto ai
    // byte che l'IPC produce. Con l'enforcement sulla stima il file
    // sarebbe cresciuto ben oltre la quota; con l'enforcement sulle
    // scritture fisiche la quota tiene comunque.
    const QUOTA: u64 = 4_096;
    let schema = schema();
    let budget = budget_di(1 << 20, QUOTA);
    let mut spool = StagedSpool::with_threshold(
        Arc::clone(&schema),
        budget.clone(),
        2 * PER_BATCH_OVERHEAD_BYTES,
    );
    let mut esito = Ok(());
    for indice in 0..64_i64 {
        // 1 byte dichiarato per un batch da 64 righe: sottostima grossa.
        esito = spingi(&mut spool, &budget, batch(&schema, indice * 64, 64), 1);
        if esito.is_err() {
            break;
        }
    }
    let esito = esito.and_then(|()| spool.seal());
    assert!(
        esito.is_err(),
        "una sottostima non deve poter aggirare la quota di spill"
    );
    assert!(
        spool.written_spill_bytes() <= QUOTA,
        "scritti {} byte con una quota di {QUOTA}",
        spool.written_spill_bytes()
    );
}

#[test]
fn a_quota_smaller_than_the_reservation_chunk_is_usable() {
    // La prenotazione a blocchi da 1 MiB non deve trasformare una quota
    // piu' piccola in un rifiuto sistematico: il tetto configurato
    // verrebbe arrotondato per eccesso al blocco, cioe' ignorato.
    // Il test ha senso solo perche' la quota e' molto piu' piccola del
    // blocco di prenotazione: `SPILL_RESERVATION_CHUNK` e' 1 MiB, la
    // quota qui e' 64 KiB.
    let schema = schema();
    let budget = budget_di(1 << 20, 64 * 1024);
    let mut spool = StagedSpool::with_threshold(
        Arc::clone(&schema),
        budget.clone(),
        2 * PER_BATCH_OVERHEAD_BYTES,
    );
    for indice in 0..4_i64 {
        spingi(&mut spool, &budget, batch(&schema, indice * 4, 4), 100)
            .expect("una quota piccola ma sufficiente deve bastare");
    }
    spool.seal().expect("seal");
    assert_eq!(drain(&mut spool).len(), 4);
    assert!(spool.written_spill_bytes() <= 64 * 1024);
}

fn rilasci_registrati() -> Vec<&'static str> {
    REGISTRO_RILASCI.with(|registro| registro.borrow().clone())
}

fn azzera_registro() {
    REGISTRO_RILASCI.with(|registro| registro.borrow_mut().clear());
}

/// Il file deve chiudersi **prima** che la quota torni al budget.
///
/// L'ordine inverso annuncerebbe spazio che il volume non ha ancora
/// liberato: un'altra operazione potrebbe prendere la quota e trovarsi il
/// disco pieno. La garanzia sta nell'ordine di dichiarazione dei campi di
/// `Stage`, quindi e' fragile a un riordino distratto — ed e' esattamente
/// il genere di garanzia che va verificata invece che affermata.
#[test]
fn the_file_closes_before_the_quota_returns() {
    let schema = schema();
    let budget = budget_di(1 << 20, 1 << 24);
    let mut spool = StagedSpool::with_threshold(
        Arc::clone(&schema),
        budget.clone(),
        2 * PER_BATCH_OVERHEAD_BYTES,
    );
    for indice in 0..8_i64 {
        spingi(&mut spool, &budget, batch(&schema, indice * 4, 4), 100).expect("push");
    }
    spool.seal().expect("seal");
    azzera_registro();

    while spool.next_batch().expect("rilettura").is_some() {}

    let registro = rilasci_registrati();
    assert_eq!(
        registro.first(),
        Some(&"file"),
        "il descrittore deve chiudersi prima che le lease tornino al budget: {registro:?}"
    );
    assert!(
        registro.len() >= 2 && registro.iter().skip(1).all(|evento| *evento == "quota"),
        "dopo la chiusura devono seguire solo rilasci di quota: {registro:?}"
    );
}

/// Stessa garanzia sul percorso che non arriva a EOF: una violazione a
/// meta' scansione distrugge lo spool con `clear`, e anche li' l'ordine
/// deve essere quello.
///
/// I batch sono volutamente grandi: con pochi byte il `BufWriter` non
/// consegnerebbe nulla al file prima del drop, nessuna lease esisterebbe
/// al momento di `clear` e il test non potrebbe distinguere l'ordine
/// giusto da quello sbagliato — passerebbe comunque, che e' il modo
/// peggiore di fallire. L'asserzione sui byte scritti tiene ferma questa
/// precondizione.
#[test]
fn clearing_mid_scan_closes_the_file_before_returning_the_quota() {
    let schema = schema();
    let budget = budget_di(1 << 20, 1 << 24);
    let mut spool = StagedSpool::with_threshold(
        Arc::clone(&schema),
        budget.clone(),
        2 * PER_BATCH_OVERHEAD_BYTES,
    );
    for indice in 0..32_i64 {
        spingi(&mut spool, &budget, batch(&schema, indice * 512, 512), 100).expect("push");
    }
    assert!(
        spool.written_spill_bytes() > 0,
        "senza scritture fisiche il test non osserverebbe alcun rilascio di quota"
    );
    azzera_registro();

    spool.clear();

    let registro = rilasci_registrati();
    assert_eq!(
        registro.first(),
        Some(&"file"),
        "anche interrompendo a meta' il descrittore va chiuso per primo: {registro:?}"
    );
    assert!(
        registro.len() >= 2 && registro.iter().skip(1).all(|evento| *evento == "quota"),
        "dopo la chiusura devono seguire solo rilasci di quota: {registro:?}"
    );
}

#[test]
fn reaching_eof_releases_file_and_quota_while_the_spool_is_still_alive() {
    // Il consumer puo' lavorare a lungo sui batch gia' ricevuti: tenere
    // occupati volume e quota fino al drop dello spool significherebbe
    // tenerli occupati per tutto quel tempo, senza motivo.
    let schema = schema();
    let budget = budget_di(1 << 20, 1 << 24);
    let iniziale = budget.context().remaining_spill();
    let mut spool = StagedSpool::with_threshold(
        Arc::clone(&schema),
        budget.clone(),
        2 * PER_BATCH_OVERHEAD_BYTES,
    );
    for indice in 0..8_i64 {
        spingi(&mut spool, &budget, batch(&schema, indice * 4, 4), 100).expect("push");
    }
    spool.seal().expect("seal");
    assert!(
        budget.context().remaining_spill() < iniziale,
        "durante la rilettura la quota deve risultare impegnata"
    );

    while spool.next_batch().expect("rilettura").is_some() {}

    // Lo spool e' ancora vivo: non e' il suo `Drop` ad aver liberato.
    assert_eq!(
        budget.context().remaining_spill(),
        iniziale,
        "a fine rilettura la quota deve tornare senza aspettare il drop"
    );
    assert!(
        !spool.spilled(),
        "il file di spool non deve essere piu' aperto"
    );
    assert!(
        spool.next_batch().expect("dopo l'esaurimento").is_none(),
        "uno spool esaurito resta esaurito"
    );
}

#[test]
fn spill_quota_returns_to_the_budget_when_the_spool_is_dropped() {
    // Con un `commit` la quota sarebbe consumata per sempre e una
    // pipeline lunga esaurirebbe lo spill accumulando file gia' rimossi.
    let schema = schema();
    let budget = budget_di(1 << 20, 1 << 24);
    let iniziale = budget.context().remaining_spill();
    {
        let mut spool = StagedSpool::with_threshold(
            Arc::clone(&schema),
            budget.clone(),
            2 * PER_BATCH_OVERHEAD_BYTES,
        );
        for indice in 0..8_i64 {
            spingi(&mut spool, &budget, batch(&schema, indice * 4, 4), 100).expect("push");
        }
        spool.seal().expect("seal");
        assert!(
            budget.context().remaining_spill() < iniziale,
            "durante la vita dello spool la quota deve risultare impegnata"
        );
    }
    assert_eq!(
        budget.context().remaining_spill(),
        iniziale,
        "il file sparisce con lo spool: la quota deve tornare"
    );
}

#[test]
fn empty_batches_are_bounded_by_the_per_batch_overhead() {
    // Senza un costo minimo per batch, una sorgente che produce batch
    // vuoti in serie non farebbe mai scattare la soglia e la coda
    // crescerebbe senza tetto.
    let schema = schema();
    let budget = budget_di(1 << 20, 1 << 24);
    let mut spool = StagedSpool::with_threshold(Arc::clone(&schema), budget.clone(), 4_096);
    for _ in 0..16_u8 {
        spingi(&mut spool, &budget, batch(&schema, 0, 0), 0).expect("push vuoto");
    }
    assert!(
        spool.spilled(),
        "batch vuoti in serie devono comunque far scattare la migrazione"
    );
    spool.seal().expect("seal");
    assert_eq!(drain(&mut spool).len(), 16);
}

#[test]
fn empty_batches_still_consume_the_memory_quota() {
    let schema = schema();
    let budget = budget_di(4_096, 1 << 24);
    let mut spool = StagedSpool::with_threshold(Arc::clone(&schema), budget.clone(), 1 << 20);
    spingi(&mut spool, &budget, batch(&schema, 0, 0), 0).expect("push vuoto");
    assert!(
        budget.context().remaining_memory() < 4_096,
        "un batch vuoto occupa comunque un posto in coda"
    );
}

#[test]
fn zero_column_batches_are_bounded_too() {
    let vuoto: SchemaRef = Arc::new(Schema::empty());
    let batch_vuoto = match RecordBatch::try_new_with_options(
        Arc::clone(&vuoto),
        Vec::new(),
        &arrow_array::RecordBatchOptions::new().with_row_count(Some(0)),
    ) {
        Ok(batch) => batch,
        Err(error) => unreachable!("batch senza colonne: {error}"),
    };
    let budget = budget_di(1 << 20, 1 << 24);
    let mut spool = StagedSpool::with_threshold(Arc::clone(&vuoto), budget.clone(), 4_096);
    for _ in 0..16_u8 {
        spingi(&mut spool, &budget, batch_vuoto.clone(), 0).expect("push");
    }
    assert!(
        spool.spilled(),
        "anche senza colonne la boundedness non puo' dipendere dai dati"
    );
}

#[test]
fn migration_stops_on_cancellation() {
    let schema = schema();
    let token = CancellationToken::new();
    let budget = budget_di(1 << 20, 1 << 24);
    let mut spool = StagedSpool {
        schema: Arc::clone(&schema),
        budget: budget.clone(),
        cancellation: token.clone(),
        memory_threshold: 2 * PER_BATCH_OVERHEAD_BYTES,
        stage: Stage::Memory {
            batches: VecDeque::new(),
            bytes: 0,
        },
        sealed: false,
        spilled_once: false,
    };
    spingi(&mut spool, &budget, batch(&schema, 0, 4), 100).expect("primo push");
    token.cancel();
    let errore = spingi(&mut spool, &budget, batch(&schema, 4, 4), 100)
        .expect_err("la migrazione deve interrompersi");
    assert_eq!(errore.category, plenora_io_model::ErrorCategory::Cancelled);
}

#[test]
fn replay_stops_on_cancellation() {
    let schema = schema();
    let token = CancellationToken::new();
    let budget = budget_di(1 << 20, 1 << 24);
    let mut spool = StagedSpool::new(Arc::clone(&schema), budget.clone(), token.clone());
    spingi(&mut spool, &budget, batch(&schema, 0, 4), 100).expect("push");
    spool.seal().expect("seal");
    token.cancel();
    let errore = spool
        .next_batch()
        .expect_err("il replay deve interrompersi");
    assert_eq!(errore.category, plenora_io_model::ErrorCategory::Cancelled);
}

#[test]
fn replay_stops_when_the_deadline_expires() {
    let schema = schema();
    let budget = budget_con(PipelineLimits::default().with_duration_ms(1));
    let mut spool = StagedSpool::new(
        Arc::clone(&schema),
        budget.clone(),
        CancellationToken::default(),
    );
    spingi(&mut spool, &budget, batch(&schema, 0, 4), 100).expect("push");
    spool.seal().expect("seal");
    std::thread::sleep(std::time::Duration::from_millis(5));
    let errore = spool
        .next_batch()
        .expect_err("la deadline deve interrompere il replay");
    assert_eq!(errore.code, plenora_io_model::IoErrorCode::LimitExceeded);
}

#[test]
fn an_unset_spill_dir_uses_the_system_temporary_directory() {
    let risolta = resolve_spill_directory(None).expect("il default deve risolvere");
    assert_eq!(risolta, std::env::temp_dir());
}

#[test]
fn a_configured_spill_dir_is_honored() {
    let temporanea = tempfile::tempdir().expect("tempdir");
    let risolta = resolve_spill_directory(Some(temporanea.path().as_os_str().to_owned()))
        .expect("una directory valida deve risolvere");
    assert_eq!(risolta, temporanea.path());
}

#[test]
fn an_unusable_spill_dir_fails_closed_instead_of_falling_back() {
    // Un ripiego silenzioso metterebbe i dati su un volume che
    // l'operatore non ha scelto.
    let inesistente = std::env::temp_dir().join("plenora-spill-che-non-esiste");
    assert!(resolve_spill_directory(Some(inesistente.into_os_string())).is_err());
}

#[test]
fn a_spill_dir_that_is_a_file_is_rejected() {
    let file = tempfile::NamedTempFile::new().expect("file temporaneo");
    assert!(resolve_spill_directory(Some(file.path().as_os_str().to_owned())).is_err());
}
