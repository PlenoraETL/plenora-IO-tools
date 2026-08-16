//! Spool bounded per l'adapter di lettura operation-atomic (ADR-IO 7 A).
//!
//! L'adapter comune deve consegnare il primo batch solo dopo aver verificato
//! l'intera sorgente: se una violazione emerge in un punto qualsiasi, il
//! chiamante non deve aver mai visto un prefisso accepted. Finora quella
//! garanzia costava memoria `O(dataset)`, perche' i batch verificati
//! restavano tutti in RAM.
//!
//! Lo `StagedSpool` conserva la stessa garanzia cambiando dove stanno i batch:
//! restano in memoria finche' l'occupato sta sotto una soglia adattiva, poi
//! migrano su un file temporaneo in Arrow IPC e non tornano piu' in RAM. Il
//! picco diventa `soglia + batch corrente`, **indipendente dalla dimensione
//! totale** dell'input.
//!
//! # Il file temporaneo non ha nome
//!
//! ADR-IO 7 prevedeva una directory di spill con permessi 0700, una variabile
//! `PLENORA_SPILL_DIR` e uno sweep degli orfani basato su lock esclusivo.
//! L'implementazione adotta una forma piu' forte e piu' semplice: il file e'
//! creato con `tempfile::tempfile_in`, cioe' **scollegato dal filesystem
//! appena aperto** su Unix e con `FILE_FLAG_DELETE_ON_CLOSE` su Windows.
//!
//! Le conseguenze contano piu' del meccanismo:
//!
//! - nessun altro processo puo' aprirlo, perche' non esiste un path da aprire;
//!   il rischio di lettura o iniezione da parte di un altro utente del
//!   filesystem sparisce invece di essere mitigato dai permessi;
//! - non esistono orfani da spazzare, nemmeno dopo un `SIGKILL` o un crollo
//!   dell'alimentazione: il kernel libera l'inode alla chiusura del
//!   descrittore. Lo sweep su lock, i suoi casi limite e la sua superficie di
//!   race non servono piu';
//! - non c'e' finestra TOCTOU fra creazione e apertura, e nessuna possibilita'
//!   di seguire un symlink piazzato da altri.
//!
//! `PLENORA_SPILL_DIR` resta e sceglie la directory che ospita l'inode: serve
//! a mettere lo spill su un volume capiente o veloce. Se e' impostata ma non
//! utilizzabile la creazione **fallisce chiuso**, senza ripiegare su un'altra
//! directory: un ripiego silenzioso metterebbe dati su un volume che
//! l'operatore non ha scelto.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::PathBuf;

use arrow_array::RecordBatch;
use arrow_ipc::reader::StreamReader;
use arrow_ipc::writer::StreamWriter;
use arrow_schema::SchemaRef;
use plenora_io_model::{
    ErrorCategory, ErrorPhase, IoErrorCode, PlenoraIoError, RemoteEffect, ResourceBudget,
    ResourceKind, ResourceLease, Result, RetryDisposition,
};

/// Variabile che sceglie la directory che ospita l'inode dello spool.
pub const SPILL_DIR_ENV: &str = "PLENORA_SPILL_DIR";

// Messaggi curati: nessun testo di errore di terze parti e nessun percorso
// entra nell'errore pubblico (INV-10). Un `ArrowError` o un `io::Error`
// portano misure prese dal file, a volte percorsi: il dettaglio si ferma qui.
const SPOOL_CREATE_FAILED: &str = "creazione del file di spool non riuscita";
const SPOOL_WRITE_FAILED: &str = "scrittura sul file di spool non riuscita";
const SPOOL_SEAL_FAILED: &str = "chiusura del file di spool non riuscita";
const SPOOL_REPLAY_FAILED: &str = "rilettura del file di spool non riuscita";
const SPOOL_CORRUPTION: &str = "il file di spool non rispetta il contratto atteso";
const SPOOL_SCHEMA_MISMATCH: &str = "batch con schema diverso da quello del layer";
const SPOOL_ALREADY_SEALED: &str = "spool gia' sigillato: nessun batch nuovo";
const SPOOL_NOT_SEALED: &str = "spool non ancora sigillato: nessun batch da rileggere";

fn spool_error(message: impl Into<String>) -> PlenoraIoError {
    // Non passa da `PlenoraIoError::Io(io::Error)`: quel costruttore riporta
    // il `kind` della dipendenza, mentre qui il messaggio deve restare una
    // costante scelta da noi (INV-10).
    let mut error = PlenoraIoError::new(
        ErrorCategory::Io,
        ErrorPhase::Read,
        RemoteEffect::None,
        RetryDisposition::Never,
        message,
    );
    error.code = IoErrorCode::Io;
    error
}

fn contract_error(message: &'static str) -> PlenoraIoError {
    PlenoraIoError::Contract(message.to_owned())
}

/// Soglia oltre la quale i batch bufferizzati migrano su disco.
///
/// E' meta' della quota di memoria della pipeline, non tutta: l'altra meta'
/// resta al batch che il reader sta materializzando in questo momento. Con la
/// soglia al 100% il buffer potrebbe consumare l'intera quota e far fallire
/// la materializzazione del batch successivo — cioe' rendere lo spool inutile
/// proprio nel caso che dovrebbe risolvere.
#[must_use]
pub fn adaptive_memory_threshold(budget: &ResourceBudget) -> u64 {
    (budget.limits().memory_bytes / 2).max(1)
}

/// Directory che ospitera' l'inode dello spool.
///
/// # Errors
///
/// Restituisce un errore se `PLENORA_SPILL_DIR` e' impostata ma non e' una
/// directory utilizzabile. Non ripiega su un'altra directory: un ripiego
/// silenzioso metterebbe i dati su un volume che l'operatore non ha scelto.
fn spill_directory() -> Result<PathBuf> {
    resolve_spill_directory(std::env::var_os(SPILL_DIR_ENV))
}

/// Risoluzione pura, separata dalla lettura dell'ambiente: mutare una
/// variabile di processo dentro un test la renderebbe visibile agli altri
/// test in parallelo, e il fallimento sarebbe intermittente invece che
/// riproducibile.
fn resolve_spill_directory(configured: Option<std::ffi::OsString>) -> Result<PathBuf> {
    match configured {
        None => Ok(std::env::temp_dir()),
        Some(configured) => {
            let path = PathBuf::from(configured);
            let metadata = std::fs::metadata(&path).map_err(|_| {
                spool_error(format!(
                    "{SPILL_DIR_ENV} non e' accessibile come directory di spool"
                ))
            })?;
            if metadata.is_dir() {
                Ok(path)
            } else {
                Err(spool_error(format!("{SPILL_DIR_ENV} non e' una directory")))
            }
        }
    }
}

/// Dove vivono i batch gia' verificati.
enum Stage {
    /// Sotto soglia: i batch restano in RAM, ognuno con la propria lease di
    /// memoria. La lease viene restituita quando il batch lascia la RAM —
    /// consegnato al consumer o migrato su disco (INV-5).
    Memory {
        batches: VecDeque<(RecordBatch, Option<ResourceLease>)>,
        bytes: u64,
    },
    /// Oltre soglia: i batch sono su file temporaneo senza nome. Una volta
    /// migrati non tornano in RAM.
    Writing {
        writer: Box<StreamWriter<BufWriter<File>>>,
    },
    /// Sigillato: il file e' pronto per la rilettura in ordine.
    Replaying { reader: Box<StreamReader<File>> },
    /// Sigillato e vuoto, oppure esaurito.
    Drained,
}

/// Buffer operation-atomic con migrazione adattiva memoria → disco.
///
/// Il ciclo e' in due fasi: `push` finche' la sorgente non e' esaurita, poi
/// `seal`, poi `next_batch` fino a esaurimento. Le fasi non si sovrappongono:
/// e' cio' che rende l'atomicita' operativa verificabile invece che sperata.
pub struct StagedSpool {
    schema: SchemaRef,
    budget: ResourceBudget,
    memory_threshold: u64,
    stage: Stage,
    sealed: bool,
}

impl StagedSpool {
    #[must_use]
    pub fn new(schema: SchemaRef, budget: ResourceBudget) -> Self {
        let memory_threshold = adaptive_memory_threshold(&budget);
        Self {
            schema,
            budget,
            memory_threshold,
            stage: Stage::Memory {
                batches: VecDeque::new(),
                bytes: 0,
            },
            sealed: false,
        }
    }

    #[cfg(test)]
    const fn with_threshold(
        schema: SchemaRef,
        budget: ResourceBudget,
        memory_threshold: u64,
    ) -> Self {
        Self {
            schema,
            budget,
            memory_threshold,
            stage: Stage::Memory {
                batches: VecDeque::new(),
                bytes: 0,
            },
            sealed: false,
        }
    }

    /// `true` se i batch sono gia' migrati su file temporaneo.
    #[must_use]
    pub const fn spilled(&self) -> bool {
        matches!(self.stage, Stage::Writing { .. } | Stage::Replaying { .. })
    }

    /// Byte attualmente trattenuti in RAM dai batch bufferizzati.
    #[must_use]
    pub const fn buffered_memory_bytes(&self) -> u64 {
        match &self.stage {
            Stage::Memory { bytes, .. } => *bytes,
            Stage::Writing { .. } | Stage::Replaying { .. } | Stage::Drained => 0,
        }
    }

    /// Accoda un batch gia' verificato.
    ///
    /// `memory_bytes` e' la stima di occupazione del batch: e' la grandezza su
    /// cui si decide la migrazione e su cui si tiene la lease di memoria.
    ///
    /// # Errors
    ///
    /// Restituisce un errore se lo spool e' gia' sigillato, se lo schema del
    /// batch diverge da quello del layer, se la quota di memoria o di spill
    /// non basta, o se la scrittura sul file temporaneo fallisce.
    pub fn push(&mut self, batch: RecordBatch, memory_bytes: u64) -> Result<()> {
        if self.sealed {
            return Err(contract_error(SPOOL_ALREADY_SEALED));
        }
        if batch.schema() != self.schema {
            return Err(contract_error(SPOOL_SCHEMA_MISMATCH));
        }
        let migrate = match &self.stage {
            Stage::Memory { bytes, .. } => {
                bytes.saturating_add(memory_bytes) > self.memory_threshold
            }
            _ => false,
        };
        if migrate {
            self.migrate_to_disk()?;
        }
        match &mut self.stage {
            Stage::Memory { batches, bytes } => {
                // La lease resta viva quanto il batch: e' la memoria che la
                // libreria detiene davvero (INV-5). Un batch da zero byte non
                // prenota nulla, perche' una lease di zero non e' valida.
                let lease = if memory_bytes > 0 {
                    Some(
                        self.budget
                            .try_lease(ResourceKind::MemoryBytes, memory_bytes)?,
                    )
                } else {
                    None
                };
                batches.push_back((batch, lease));
                *bytes = bytes.saturating_add(memory_bytes);
                Ok(())
            }
            Stage::Writing { writer, .. } => {
                Self::charge_spill(&self.budget, memory_bytes)?;
                writer
                    .write(&batch)
                    .map_err(|_| spool_error(SPOOL_WRITE_FAILED))
            }
            Stage::Replaying { .. } | Stage::Drained => Err(contract_error(SPOOL_ALREADY_SEALED)),
        }
    }

    /// Sigilla lo spool: da qui in poi si legge soltanto.
    ///
    /// # Errors
    ///
    /// Restituisce un errore se la chiusura del writer o il riavvolgimento
    /// del file temporaneo falliscono.
    pub fn seal(&mut self) -> Result<()> {
        if self.sealed {
            return Ok(());
        }
        self.sealed = true;
        let stage = std::mem::replace(&mut self.stage, Stage::Drained);
        self.stage = match stage {
            Stage::Memory { batches, bytes } => Stage::Memory { batches, bytes },
            Stage::Writing { writer } => {
                let mut buffered = writer
                    .into_inner()
                    .map_err(|_| spool_error(SPOOL_SEAL_FAILED))?;
                buffered
                    .flush()
                    .map_err(|_| spool_error(SPOOL_SEAL_FAILED))?;
                let mut file = buffered
                    .into_inner()
                    .map_err(|_| spool_error(SPOOL_SEAL_FAILED))?;
                file.seek(SeekFrom::Start(0))
                    .map_err(|_| spool_error(SPOOL_SEAL_FAILED))?;
                let reader =
                    StreamReader::try_new(file, None).map_err(|_| spool_error(SPOOL_CORRUPTION))?;
                Stage::Replaying {
                    reader: Box::new(reader),
                }
            }
            other => other,
        };
        Ok(())
    }

    /// Restituisce il batch successivo nell'ordine di inserimento.
    ///
    /// # Errors
    ///
    /// Restituisce un errore se lo spool non e' sigillato, o se la rilettura
    /// del file temporaneo fallisce o produce un payload non conforme.
    pub fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        if !self.sealed {
            return Err(contract_error(SPOOL_NOT_SEALED));
        }
        match &mut self.stage {
            Stage::Memory { batches, bytes } => match batches.pop_front() {
                // Il drop della lease restituisce la memoria nello stesso
                // istante in cui il batch lascia la libreria: e' il transfer
                // di INV-5, non un rilascio differito.
                Some((batch, lease)) => {
                    let released = lease.map_or(0, |lease| lease.amount());
                    *bytes = bytes.saturating_sub(released);
                    Ok(Some(batch))
                }
                None => Ok(None),
            },
            Stage::Replaying { reader } => match reader.next() {
                None => Ok(None),
                Some(Ok(batch)) => {
                    if batch.schema() == self.schema {
                        Ok(Some(batch))
                    } else {
                        Err(contract_error(SPOOL_CORRUPTION))
                    }
                }
                Some(Err(_)) => Err(spool_error(SPOOL_REPLAY_FAILED)),
            },
            Stage::Writing { .. } => Err(contract_error(SPOOL_NOT_SEALED)),
            Stage::Drained => Ok(None),
        }
    }

    /// Svuota lo spool restituendo ogni quota trattenuta.
    ///
    /// Serve quando una violazione emerge a meta' scansione: i batch gia'
    /// verificati non devono raggiungere il consumer, e la loro memoria non
    /// deve restare prenotata mentre il drain prosegue per completare i
    /// conteggi.
    pub fn clear(&mut self) {
        self.stage = Stage::Drained;
    }

    /// Costruisce uno spool gia' sigillato che rilegge da `file`.
    ///
    /// Esiste per esercitare il ramo di replay su un payload che il writer
    /// non produrrebbe mai: un file di spool corrotto e' l'unico modo di
    /// provare che INV-8 vale anche quando la rilettura fallisce **dopo** la
    /// validazione, cioe' quando il consumer ha gia' ricevuto un `Ok`.
    #[cfg(test)]
    fn replaying_from(schema: SchemaRef, budget: ResourceBudget, file: File) -> Result<Self> {
        let reader =
            StreamReader::try_new(file, None).map_err(|_| spool_error(SPOOL_CORRUPTION))?;
        Ok(Self {
            schema,
            budget,
            memory_threshold: 0,
            stage: Stage::Replaying {
                reader: Box::new(reader),
            },
            sealed: true,
        })
    }

    fn migrate_to_disk(&mut self) -> Result<()> {
        let stage = std::mem::replace(&mut self.stage, Stage::Drained);
        let Stage::Memory { batches, bytes } = stage else {
            self.stage = stage;
            return Ok(());
        };
        let directory = spill_directory()?;
        let file =
            tempfile::tempfile_in(&directory).map_err(|_| spool_error(SPOOL_CREATE_FAILED))?;
        let mut writer = StreamWriter::try_new(BufWriter::new(file), self.schema.as_ref())
            .map_err(|_| spool_error(SPOOL_CREATE_FAILED))?;
        // Lo spill viene addebitato prima di scrivere: se la quota non basta
        // l'operazione fallisce senza aver toccato il disco.
        Self::charge_spill(&self.budget, bytes)?;
        for (batch, lease) in batches {
            writer
                .write(&batch)
                .map_err(|_| spool_error(SPOOL_WRITE_FAILED))?;
            // Il batch ha lasciato la RAM: la memoria torna subito, ed e'
            // proprio questo che rende il picco indipendente dal dataset.
            drop(lease);
        }
        self.stage = Stage::Writing {
            writer: Box::new(writer),
        };
        Ok(())
    }

    fn charge_spill(budget: &ResourceBudget, bytes: u64) -> Result<()> {
        if bytes == 0 {
            return Ok(());
        }
        // Lo spill e' consumo cumulativo, non una prenotazione restituibile:
        // il file cresce e non si ritira finche' lo spool vive.
        budget
            .try_lease(ResourceKind::SpillBytes, bytes)?
            .commit(bytes)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow_array::{Int64Array, StringArray};
    use arrow_schema::{DataType, Field, Schema};
    use plenora_io_model::ResourceLimits;

    use super::*;

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

    fn budget(memory_bytes: u64, spill_bytes: u64) -> ResourceBudget {
        match ResourceBudget::new(ResourceLimits {
            memory_bytes,
            cell_bytes: memory_bytes.min(64 * 1024 * 1024),
            spill_bytes,
            ..ResourceLimits::default()
        }) {
            Ok(budget) => budget,
            Err(error) => unreachable!("budget di test non costruibile: {error:?}"),
        }
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
        let mut spool =
            StagedSpool::with_threshold(Arc::clone(&schema), budget(1 << 20, 1 << 20), 1_000);
        spool.push(batch(&schema, 0, 4), 100).expect("push");
        spool.push(batch(&schema, 4, 4), 100).expect("push");
        assert!(!spool.spilled());
        assert_eq!(spool.buffered_memory_bytes(), 200);
        spool.seal().expect("seal");
        assert_eq!(drain(&mut spool).len(), 2);
    }

    #[test]
    fn crossing_the_threshold_migrates_to_disk_and_preserves_order() {
        let schema = schema();
        let mut spool =
            StagedSpool::with_threshold(Arc::clone(&schema), budget(1 << 20, 1 << 20), 150);
        for indice in 0..6_i64 {
            spool
                .push(batch(&schema, indice * 4, 4), 100)
                .expect("push");
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
        let budget = budget(10_000, 1 << 20);
        let mut spool = StagedSpool::with_threshold(Arc::clone(&schema), budget.clone(), 150);
        spool.push(batch(&schema, 0, 4), 100).expect("push");
        assert_eq!(budget.remaining(ResourceKind::MemoryBytes), 9_900);
        spool.push(batch(&schema, 4, 4), 100).expect("push");
        assert!(spool.spilled());
        assert_eq!(
            budget.remaining(ResourceKind::MemoryBytes),
            10_000,
            "dopo la migrazione nessun batch trattiene memoria"
        );
        assert_eq!(spool.buffered_memory_bytes(), 0);
    }

    #[test]
    fn delivering_a_batch_returns_its_memory() {
        let schema = schema();
        let budget = budget(10_000, 1 << 20);
        let mut spool = StagedSpool::with_threshold(Arc::clone(&schema), budget.clone(), 10_000);
        spool.push(batch(&schema, 0, 4), 400).expect("push");
        spool.push(batch(&schema, 4, 4), 600).expect("push");
        assert_eq!(budget.remaining(ResourceKind::MemoryBytes), 9_000);
        spool.seal().expect("seal");
        let _primo = spool.next_batch().expect("primo");
        assert_eq!(
            budget.remaining(ResourceKind::MemoryBytes),
            9_400,
            "la memoria torna al transfer del batch, non alla fine dell'operazione"
        );
        assert_eq!(spool.buffered_memory_bytes(), 600);
    }

    #[test]
    fn a_dataset_larger_than_the_memory_quota_still_completes() {
        // Prima dello spool questo caso falliva `LimitExceeded`: i batch
        // verificati restavano tutti in RAM.
        let schema = schema();
        let budget = budget(4_096, 1 << 20);
        let mut spool = StagedSpool::new(Arc::clone(&schema), budget);
        for indice in 0..64_i64 {
            spool
                .push(batch(&schema, indice * 8, 8), 1_024)
                .expect("un dataset oltre la quota di memoria deve passare");
        }
        spool.seal().expect("seal");
        assert_eq!(drain(&mut spool).len(), 64);
    }

    #[test]
    fn spill_quota_is_enforced() {
        let schema = schema();
        let mut spool = StagedSpool::with_threshold(Arc::clone(&schema), budget(1 << 20, 500), 100);
        spool.push(batch(&schema, 0, 4), 200).expect("primo push");
        let errore = spool
            .push(batch(&schema, 4, 4), 400)
            .expect_err("lo spill oltre quota deve fallire");
        assert_eq!(errore.code, plenora_io_model::IoErrorCode::LimitExceeded);
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
        let mut spool = StagedSpool::new(schema, budget(1 << 20, 1 << 20));
        assert!(spool.push(estraneo, 10).is_err());
    }

    #[test]
    fn pushing_after_seal_is_rejected() {
        let schema = schema();
        let mut spool = StagedSpool::new(Arc::clone(&schema), budget(1 << 20, 1 << 20));
        spool.seal().expect("seal");
        assert!(spool.push(batch(&schema, 0, 4), 10).is_err());
    }

    #[test]
    fn reading_before_seal_is_rejected() {
        let schema = schema();
        let mut spool = StagedSpool::new(Arc::clone(&schema), budget(1 << 20, 1 << 20));
        spool.push(batch(&schema, 0, 4), 10).expect("push");
        assert!(spool.next_batch().is_err());
    }

    #[test]
    fn clear_releases_every_reservation() {
        let schema = schema();
        let budget = budget(10_000, 1 << 20);
        let mut spool = StagedSpool::with_threshold(Arc::clone(&schema), budget.clone(), 10_000);
        spool.push(batch(&schema, 0, 4), 500).expect("push");
        assert_eq!(budget.remaining(ResourceKind::MemoryBytes), 9_500);
        spool.clear();
        assert_eq!(
            budget.remaining(ResourceKind::MemoryBytes),
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

        let mut spool = StagedSpool::replaying_from(schema, budget(1 << 20, 1 << 20), file)
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
}
