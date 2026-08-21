//! Spool bounded per l'adapter di lettura operation-atomic (ENGINEERING.md § Spool e memoria A).
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
//! ENGINEERING.md § Spool e memoria prevedeva una directory di spill con permessi 0700, una variabile
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
//!
//! # La quota segue le scritture fisiche
//!
//! La prenotazione di spill vive in [`SpillGuard`], che il writer consulta
//! **prima di ogni `write` verso il file**. Applicarla piu' in alto —
//! attorno alla scrittura del batch — significherebbe applicarla a una
//! stima, mentre i byte trattenuti dal buffer raggiungerebbero il volume
//! senza passare da alcun controllo.
//!
//! Le prenotazioni sono a blocchi, per non creare una lease per batch, con
//! ripiego sull'importo esatto quando la quota configurata e' piu' piccola
//! del blocco: altrimenti un tetto piccolo verrebbe arrotondato per eccesso
//! e risulterebbe inutilizzabile.
//!
//! # Rilascio a fine rilettura
//!
//! Quando la rilettura si esaurisce, descrittore e prenotazioni vengono
//! rilasciati subito, senza aspettare il drop dello spool. Il consumer puo'
//! lavorare a lungo sui batch gia' ricevuti, e tenere occupati volume e
//! quota per tutto quel tempo non servirebbe a nulla.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use arrow_array::RecordBatch;
use arrow_ipc::reader::StreamReader;
use arrow_ipc::writer::StreamWriter;
use arrow_schema::SchemaRef;
use plenora_io_model::budget::{InternalMemoryLease, OperationBudget, SpillLease};
use plenora_io_model::PublicMessage;
use plenora_io_model::{
    CancellationToken, ErrorCategory, ErrorPhase, IoErrorCode, PlenoraIoError, RemoteEffect,
    Result, RetryDisposition,
};

use crate::driver::check_cancelled;

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
const SPOOL_QUOTA_EXHAUSTED: &str = "quota di spill esaurita prima della scrittura";

/// Costo minimo attribuito a ogni batch bufferizzato, oltre ai byte dei suoi
/// buffer.
///
/// Un batch senza righe, o senza colonne, occupa comunque un elemento della
/// coda, un `Arc` di schema e i metadati Arrow. Se lo si contasse zero, la
/// soglia non scatterebbe mai e una sorgente che produce batch vuoti in
/// serie farebbe crescere la coda senza alcun tetto: la boundedness dello
/// spool si reggerebbe sull'ipotesi che ogni batch porti dati, che e'
/// esattamente cio' che una sorgente ostile non fa.
pub(crate) const PER_BATCH_OVERHEAD_BYTES: u64 = 1_024;

/// Granularita' delle prenotazioni di spill.
///
/// Prenotare esattamente i byte di ogni batch produrrebbe una lease per
/// batch, cioe' un milione di lease per un milione di batch. Prenotare a
/// blocchi tiene il numero di lease proporzionale alla quota di spill e non
/// al numero di batch, senza mai lasciare scritto piu' di quanto prenotato.
const SPILL_RESERVATION_CHUNK: u64 = 1024 * 1024;

fn spool_error(message: &PublicMessage) -> PlenoraIoError {
    // Non passa da `PlenoraIoError::Io(io::Error)`: quel costruttore riporta
    // il `kind` della dipendenza, mentre qui il messaggio deve restare una
    // costante scelta da noi (INV-10).
    PlenoraIoError::redatto(
        IoErrorCode::Io,
        ErrorCategory::Io,
        ErrorPhase::Read,
        RemoteEffect::None,
        RetryDisposition::Never,
        message,
    )
}

fn contract_error(message: &'static str) -> PlenoraIoError {
    PlenoraIoError::contratto_redatto(&PublicMessage::Curated(message))
}

/// Soglia oltre la quale i batch bufferizzati migrano su disco.
///
/// E' meta' della quota di memoria della pipeline, non tutta: l'altra meta'
/// resta al batch che il reader sta materializzando in questo momento. Con la
/// soglia al 100% il buffer potrebbe consumare l'intera quota e far fallire
/// la materializzazione del batch successivo — cioe' rendere lo spool inutile
/// proprio nel caso che dovrebbe risolvere.
#[must_use]
pub fn adaptive_memory_threshold(budget: &OperationBudget) -> u64 {
    // Deriva dalla capacita' **effettiva**, non dal solo limite della
    // pipeline: con un pool piu' stretto la meta' del limite locale sarebbe
    // irraggiungibile, la soglia non scatterebbe mai e lo spool non
    // migrerebbe — cioe' resterebbe inutile proprio nel caso in cui serve.
    (budget.context().effective_memory_capacity() / 2).max(1)
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
                spool_error(&PublicMessage::CuratedPair(
                    SPILL_DIR_ENV,
                    "non e' accessibile come directory di spool",
                ))
            })?;
            if metadata.is_dir() {
                Ok(path)
            } else {
                Err(spool_error(&PublicMessage::CuratedPair(
                    SPILL_DIR_ENV,
                    "non e' una directory",
                )))
            }
        }
    }
}

/// Stato della quota di spill: prenotazioni RAII e byte fisici scritti.
///
/// Vive dietro un mutex perche' e' condiviso fra lo spool e il writer che
/// avvolge il file. Il writer e' l'unico punto che sa quanti byte stanno per
/// raggiungere il disco, ed e' quindi l'unico punto dove il controllo di
/// quota puo' precedere davvero la scrittura.
struct SpillGuard {
    budget: OperationBudget,
    leases: Vec<TrackedLease>,
    reserved: u64,
    written: u64,
    /// Errore tipizzato dell'ultima prenotazione fallita. `Write::write` puo'
    /// restituire solo `io::Error`, che perderebbe la categoria di limite: lo
    /// spool lo rilegge da qui e propaga l'errore giusto.
    failure: Option<PlenoraIoError>,
}

impl SpillGuard {
    const fn new(budget: OperationBudget) -> Self {
        Self {
            budget,
            leases: Vec::new(),
            reserved: 0,
            written: 0,
            failure: None,
        }
    }

    /// Garantisce che la quota prenotata copra i byte gia' scritti piu'
    /// `additional`.
    ///
    /// Prova prima una prenotazione a blocchi, che amortizza il costo su
    /// tanti batch, e **ripiega sull'importo esatto** se il blocco non entra
    /// nella quota. Senza il ripiego una quota di spill piu' piccola del
    /// blocco fallirebbe sempre, anche quando basterebbe: il tetto
    /// configurato verrebbe di fatto arrotondato per eccesso al blocco.
    fn reserve_for(&mut self, additional: u64) -> Result<()> {
        let richiesto = self.written.saturating_add(additional);
        if richiesto <= self.reserved {
            return Ok(());
        }
        let mancante = richiesto - self.reserved;
        let blocco = mancante.max(SPILL_RESERVATION_CHUNK);
        let lease = match self.budget.context().lease_spill(blocco) {
            Ok(lease) => lease,
            Err(_) if blocco > mancante => self.budget.context().lease_spill(mancante)?,
            Err(errore) => return Err(errore),
        };
        self.reserved = self.reserved.saturating_add(lease.bytes());
        self.leases.push(TrackedLease(lease));
        Ok(())
    }

    const fn note_written(&mut self, bytes: u64) {
        self.written = self.written.saturating_add(bytes);
    }

    #[cfg(test)]
    const fn reserved(&self) -> u64 {
        self.reserved
    }

    const fn take_failure(&mut self) -> Option<PlenoraIoError> {
        self.failure.take()
    }

    #[cfg(test)]
    const fn written(&self) -> u64 {
        self.written
    }
}

/// Errore da riportare quando una scrittura sul file di spool fallisce.
///
/// `Write::write` puo' restituire solo `io::Error`, che perde la categoria di
/// limite: se il guardiano ha registrato il rifiuto tipizzato, e' quello a
/// dover raggiungere il chiamante. Il ripiego copre il caso in cui la
/// scrittura sia fallita per una ragione del filesystem e non per quota.
fn write_failure(guard: &Mutex<SpillGuard>) -> PlenoraIoError {
    guard_lock(guard)
        .take_failure()
        .unwrap_or_else(|| spool_error(&PublicMessage::Curated(SPOOL_WRITE_FAILED)))
}

fn guard_lock(guard: &Mutex<SpillGuard>) -> MutexGuard<'_, SpillGuard> {
    match guard.lock() {
        Ok(acquisito) => acquisito,
        // Un lock avvelenato significa che un thread e' andato in panico
        // mentre teneva lo stato. Le prenotazioni restano coerenti: sono
        // aggiunte a una lista dopo che la lease e' stata concessa, quindi
        // non esiste il mezzo aggiornamento che il poisoning teme.
        Err(avvelenato) => avvelenato.into_inner(),
    }
}

// Registro dell'ordine di distruzione, attivo solo nei test.
//
// L'ordine con cui file e quota vengono rilasciati e' una garanzia del
// modulo; una garanzia che nessuno verifica e' una speranza. Il registro e'
// per-thread, quindi ogni test vede solo i propri eventi.
#[cfg(test)]
thread_local! {
    static REGISTRO_RILASCI: std::cell::RefCell<Vec<&'static str>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
fn nota_rilascio(evento: &'static str) {
    REGISTRO_RILASCI.with(|registro| registro.borrow_mut().push(evento));
}

/// Lease di spill, avvolta per poterne osservare il rilascio nei test.
///
/// Il registro deve segnare il momento in cui la **quota torna al budget**,
/// non quello in cui muore il guardiano che la conteneva: le due cose
/// coincidono solo se nessuno svuota la lista prima del tempo, ed e'
/// esattamente l'errore che il test deve poter vedere.
struct TrackedLease(#[allow(dead_code)] SpillLease);

#[cfg(test)]
impl Drop for TrackedLease {
    fn drop(&mut self) {
        nota_rilascio("quota");
    }
}

/// Il file temporaneo dello spool.
///
/// E' un newtype e non un `File` nudo per due ragioni: dargli un nome nel
/// tipo, e poterne osservare la chiusura nei test — che e' l'unico modo di
/// verificare davvero l'ordine di rilascio invece di affermarlo.
struct SpoolFile {
    inner: File,
}

impl Write for SpoolFile {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.inner.write(buffer)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl Read for SpoolFile {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buffer)
    }
}

impl Seek for SpoolFile {
    fn seek(&mut self, posizione: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(posizione)
    }
}

#[cfg(test)]
impl Drop for SpoolFile {
    fn drop(&mut self) {
        nota_rilascio("file");
    }
}

/// Writer che applica la quota di spill **prima di ogni scrittura fisica**.
///
/// Avvolge il file, non il `BufWriter`: e' l'ultimo anello prima del disco,
/// quindi `buffer` contiene esattamente i byte che stanno per essere
/// consegnati. Applicare la quota piu' in alto — attorno a
/// `StreamWriter::write` — la applicherebbe a una stima, e i byte del
/// `BufWriter` raggiungerebbero il volume prima che qualcuno li conti.
///
/// La quota segue cosi' cio' che finisce su disco, non l'occupazione in RAM
/// del batch: l'IPC allinea, comprime i buffer di validita' e aggiunge
/// intestazioni, quindi le due grandezze divergono.
struct GuardedWriter<W: Write> {
    inner: W,
    guard: Arc<Mutex<SpillGuard>>,
}

impl<W: Write> GuardedWriter<W> {
    fn into_inner(self) -> W {
        self.inner
    }
}

impl<W: Write> Write for GuardedWriter<W> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let richiesti = buffer.len() as u64;
        let coperto = {
            let mut guard = guard_lock(&self.guard);
            match guard.reserve_for(richiesti) {
                Ok(()) => true,
                Err(errore) => {
                    guard.failure = Some(errore);
                    false
                }
            }
        };
        if !coperto {
            return Err(std::io::Error::other(SPOOL_QUOTA_EXHAUSTED));
        }
        let scritti = self.inner.write(buffer)?;
        guard_lock(&self.guard).note_written(scritti as u64);
        Ok(scritti)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Dove vivono i batch gia' verificati.
enum Stage {
    /// Sotto soglia: i batch restano in RAM, ognuno con la lease ricevuta
    /// dall'adapter. La lease viene restituita quando il batch lascia la RAM
    /// — consegnato al consumer o migrato su disco (INV-5).
    Memory {
        batches: VecDeque<(RecordBatch, Option<InternalMemoryLease>)>,
        bytes: u64,
    },
    /// Oltre soglia: i batch sono su file temporaneo senza nome. Una volta
    /// migrati non tornano in RAM.
    Writing {
        writer: Box<StreamWriter<BufWriter<GuardedWriter<SpoolFile>>>>,
        guard: Arc<Mutex<SpillGuard>>,
    },
    /// Sigillato: il file e' pronto per la rilettura in ordine. La
    /// prenotazione resta viva finche' il file esiste, cioe' fino alla fine
    /// della rilettura.
    Replaying {
        reader: Box<StreamReader<SpoolFile>>,
        /// Non viene mai letto: il suo unico compito e' essere distrutto
        /// **dopo** il reader, restituendo la quota solo quando il file e'
        /// gia' chiuso. Dichiararlo dopo `reader` e' cio' che fissa l'ordine.
        #[allow(dead_code)]
        guard: Arc<Mutex<SpillGuard>>,
    },
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
    budget: OperationBudget,
    cancellation: CancellationToken,
    memory_threshold: u64,
    stage: Stage,
    sealed: bool,
    /// Vero da quando i batch sono migrati su disco, e per sempre dopo.
    ///
    /// `spilled()` guarda lo stato **corrente**, che a rilettura conclusa
    /// torna `Drained`: un test che vuole affermare "lo spill e' avvenuto"
    /// non puo' leggerlo dopo la fine. Questo flag e' l'unico modo di
    /// distinguere "non ha spillato" da "ha spillato e ha finito", e senza
    /// un test che verifichi lo spill sotto quota stretta resterebbe da
    /// dimostrare che il completamento non venga da una quota in realta'
    /// sufficiente.
    spilled_once: bool,
}

impl StagedSpool {
    #[must_use]
    pub fn new(
        schema: SchemaRef,
        budget: OperationBudget,
        cancellation: CancellationToken,
    ) -> Self {
        let memory_threshold = adaptive_memory_threshold(&budget);
        Self {
            schema,
            budget,
            cancellation,
            memory_threshold,
            stage: Stage::Memory {
                batches: VecDeque::new(),
                bytes: 0,
            },
            sealed: false,
            spilled_once: false,
        }
    }

    #[cfg(test)]
    fn with_threshold(schema: SchemaRef, budget: OperationBudget, memory_threshold: u64) -> Self {
        Self {
            schema,
            budget,
            cancellation: CancellationToken::default(),
            memory_threshold,
            stage: Stage::Memory {
                batches: VecDeque::new(),
                bytes: 0,
            },
            sealed: false,
            spilled_once: false,
        }
    }

    /// Quota di spill attualmente prenotata dallo spool.
    /// Quota di spill attualmente **prenotata** dallo spool. Non coincide
    /// con i byte fisici: la prenotazione avviene a blocchi.
    #[cfg(test)]
    fn reserved_spill(&self) -> u64 {
        match &self.stage {
            Stage::Writing { guard, .. } | Stage::Replaying { guard, .. } => {
                guard_lock(guard).reserved()
            }
            Stage::Memory { .. } | Stage::Drained => 0,
        }
    }

    /// Byte realmente consegnati al file di spool.
    #[cfg(test)]
    fn written_spill_bytes(&self) -> u64 {
        match &self.stage {
            Stage::Writing { guard, .. } | Stage::Replaying { guard, .. } => {
                guard_lock(guard).written()
            }
            Stage::Memory { .. } | Stage::Drained => 0,
        }
    }

    /// `true` se i batch sono gia' migrati su file temporaneo.
    #[must_use]
    pub const fn spilled(&self) -> bool {
        matches!(self.stage, Stage::Writing { .. } | Stage::Replaying { .. })
    }

    /// Vero se lo spool ha migrato su disco almeno una volta, anche se ha
    /// gia' finito di rileggere. Vedi il campo omonimo.
    #[cfg(test)]
    #[must_use]
    pub const fn spilled_once(&self) -> bool {
        self.spilled_once
    }

    /// Byte attualmente trattenuti in RAM dai batch bufferizzati.
    #[must_use]
    pub const fn buffered_memory_bytes(&self) -> u64 {
        match &self.stage {
            Stage::Memory { bytes, .. } => *bytes,
            Stage::Writing { .. } | Stage::Replaying { .. } | Stage::Drained => 0,
        }
    }

    /// Accoda un batch gia' verificato, prendendone in custodia la memoria.
    ///
    /// La lease arriva **gia' ridotta** dall'adapter all'ingombro reale del
    /// batch: e' quella la grandezza su cui si decide la migrazione, e non una
    /// stima ricalcolata qui. Lo spool non ne acquisisce una propria — sarebbe
    /// una seconda contabilizzazione dello stesso batch, con in mezzo la
    /// finestra scoperta che l'handoff esiste per chiudere.
    ///
    /// # Errors
    ///
    /// Restituisce un errore se lo spool e' gia' sigillato, se lo schema del
    /// batch diverge da quello del layer, se la quota di memoria o di spill
    /// non basta, o se la scrittura sul file temporaneo fallisce.
    pub fn push(&mut self, batch: RecordBatch, memory_lease: InternalMemoryLease) -> Result<()> {
        if self.sealed {
            return Err(contract_error(SPOOL_ALREADY_SEALED));
        }
        if batch.schema() != self.schema {
            return Err(contract_error(SPOOL_SCHEMA_MISMATCH));
        }
        let accounted = memory_lease.bytes();
        let migrate = match &self.stage {
            Stage::Memory { bytes, .. } => bytes.saturating_add(accounted) > self.memory_threshold,
            _ => false,
        };
        if migrate {
            self.migrate_to_disk()?;
        }
        match &mut self.stage {
            Stage::Memory { batches, bytes } => {
                // La lease resta viva quanto il batch: e' la memoria che la
                // libreria detiene davvero (INV-5).
                batches.push_back((batch, Some(memory_lease)));
                *bytes = bytes.saturating_add(accounted);
                Ok(())
            }
            Stage::Writing { writer, guard } => {
                // Nessuna prenotazione qui: la applica il writer prima di
                // ogni scrittura fisica, sui byte veri invece che su una
                // stima. Qui si traduce soltanto il suo esito nell'errore
                // tipizzato, che `io::Error` non sa trasportare.
                match writer.write(&batch) {
                    Ok(()) => Ok(()),
                    Err(_) => Err(write_failure(guard)),
                }
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
            Stage::Writing { writer, guard } => {
                // Il flush consegna al file i byte che il buffer tratteneva:
                // e' l'ultimo momento in cui la quota puo' essere superata, e
                // il writer la applica anche li'.
                let mut buffered = writer.into_inner().map_err(|_| write_failure(&guard))?;
                buffered.flush().map_err(|_| write_failure(&guard))?;
                let mut file = buffered
                    .into_inner()
                    .map_err(|_| spool_error(&PublicMessage::Curated(SPOOL_SEAL_FAILED)))?
                    .into_inner();
                file.seek(SeekFrom::Start(0))
                    .map_err(|_| spool_error(&PublicMessage::Curated(SPOOL_SEAL_FAILED)))?;
                let reader = StreamReader::try_new(file, None)
                    .map_err(|_| spool_error(&PublicMessage::Curated(SPOOL_CORRUPTION)))?;
                Stage::Replaying {
                    reader: Box::new(reader),
                    guard,
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
        // Il replay di uno spool grande e' una sequenza lunga di letture:
        // senza questo controllo un Ctrl+C o una deadline scaduta non
        // avrebbero effetto fino all'ultimo batch.
        check_cancelled(&self.cancellation, ErrorPhase::Read)?;
        self.budget.context().ensure_active()?;
        let esito = match &mut self.stage {
            Stage::Memory { batches, bytes } => match batches.pop_front() {
                // Il drop della lease restituisce la memoria nello stesso
                // istante in cui il batch lascia la libreria: e' il transfer
                // di INV-5, non un rilascio differito.
                Some((batch, lease)) => {
                    let released = lease.map_or(0, |lease| lease.bytes());
                    *bytes = bytes.saturating_sub(released);
                    Ok(Some(batch))
                }
                None => Ok(None),
            },
            Stage::Replaying { reader, .. } => match reader.next() {
                None => Ok(None),
                Some(Ok(batch)) => {
                    if batch.schema() == self.schema {
                        Ok(Some(batch))
                    } else {
                        Err(contract_error(SPOOL_CORRUPTION))
                    }
                }
                Some(Err(_)) => Err(spool_error(&PublicMessage::Curated(SPOOL_REPLAY_FAILED))),
            },
            Stage::Writing { .. } => Err(contract_error(SPOOL_NOT_SEALED)),
            Stage::Drained => Ok(None),
        };
        // Fine della rilettura: il file e la sua quota non servono piu'. Il
        // passaggio a `Drained` chiude il descrittore — quindi libera lo
        // spazio, perche' l'inode e' gia' scollegato — e restituisce le lease
        // di spill. Aspettare il drop dello spool terrebbe occupati volume e
        // quota per tutto il tempo in cui il consumer lavora sui batch che ha
        // gia' ricevuto.
        if matches!(esito, Ok(None)) && !matches!(self.stage, Stage::Drained) {
            self.release_storage();
        }
        esito
    }

    /// Rilascia file e prenotazioni, portando lo spool a `Drained`.
    /// Rilascia file e prenotazioni portando lo spool a `Drained`.
    ///
    /// L'assegnazione **e' il rilascio**: distrugge il valore precedente, e i
    /// campi di `Stage` sono dichiarati nell'ordine in cui devono sparire —
    /// prima il writer o il reader, che chiudono il descrittore, poi il
    /// guardiano, che restituisce le lease.
    ///
    /// L'ordine conta e non e' cosmetico: restituire la quota prima di aver
    /// chiuso il file annuncerebbe spazio che il volume non ha ancora
    /// liberato, e un'altra operazione potrebbe prenderlo e trovarsi il disco
    /// pieno. Liberare esplicitamente le lease qui era esattamente questo
    /// errore, con l'aggravante di sembrare piu' accurato.
    fn release_storage(&mut self) {
        self.stage = Stage::Drained;
    }

    /// Svuota lo spool restituendo ogni quota trattenuta.
    ///
    /// Serve quando una violazione emerge a meta' scansione: i batch gia'
    /// verificati non devono raggiungere il consumer, e la loro memoria non
    /// deve restare prenotata mentre il drain prosegue per completare i
    /// conteggi.
    pub fn clear(&mut self) {
        self.release_storage();
    }

    /// Costruisce uno spool gia' sigillato che rilegge da `file`.
    ///
    /// Esiste per esercitare il ramo di replay su un payload che il writer
    /// non produrrebbe mai: un file di spool corrotto e' l'unico modo di
    /// provare che INV-8 vale anche quando la rilettura fallisce **dopo** la
    /// validazione, cioe' quando il consumer ha gia' ricevuto un `Ok`.
    #[cfg(test)]
    fn replaying_from(schema: SchemaRef, budget: OperationBudget, file: File) -> Result<Self> {
        let reader = StreamReader::try_new(SpoolFile { inner: file }, None)
            .map_err(|_| spool_error(&PublicMessage::Curated(SPOOL_CORRUPTION)))?;
        let budget_per_guard = budget.clone();
        Ok(Self {
            schema,
            budget,
            cancellation: CancellationToken::default(),
            memory_threshold: 0,
            stage: Stage::Replaying {
                reader: Box::new(reader),
                guard: Arc::new(Mutex::new(SpillGuard::new(budget_per_guard))),
            },
            sealed: true,
            // Uno spool costruito in rilettura da un file esiste perche' lo
            // spill e' gia' avvenuto.
            spilled_once: true,
        })
    }

    fn migrate_to_disk(&mut self) -> Result<()> {
        let stage = std::mem::replace(&mut self.stage, Stage::Drained);
        let Stage::Memory { batches, bytes: _ } = stage else {
            self.stage = stage;
            return Ok(());
        };
        self.spilled_once = true;
        let directory = spill_directory()?;
        let file = tempfile::tempfile_in(&directory)
            .map_err(|_| spool_error(&PublicMessage::Curated(SPOOL_CREATE_FAILED)))?;
        let guard = Arc::new(Mutex::new(SpillGuard::new(self.budget.clone())));
        let guarded = GuardedWriter {
            inner: SpoolFile { inner: file },
            guard: Arc::clone(&guard),
        };
        let mut writer = StreamWriter::try_new(BufWriter::new(guarded), self.schema.as_ref())
            .map_err(|_| spool_error(&PublicMessage::Curated(SPOOL_CREATE_FAILED)))?;
        for (batch, lease) in batches {
            // La migrazione di uno spool pieno e' una sequenza lunga di
            // scritture: va interrompibile come il resto della lettura.
            check_cancelled(&self.cancellation, ErrorPhase::Read)?;
            self.budget.context().ensure_active()?;
            writer.write(&batch).map_err(|_| write_failure(&guard))?;
            // Il batch ha lasciato la RAM: la memoria torna subito, ed e'
            // proprio questo che rende il picco indipendente dal dataset.
            drop(lease);
        }
        self.stage = Stage::Writing {
            writer: Box::new(writer),
            guard,
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
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
}
