//! Spool bounded per l'adapter di lettura operation-atomic
//! (`ENGINEERING.md § Pipeline di lettura` e `§ Spool e memoria`).
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
//! Il disegno iniziale prevedeva una directory di spill con permessi 0700,
//! una variabile `PLENORA_SPILL_DIR` e uno sweep degli orfani basato su
//! lock esclusivo. L'implementazione adotta una forma piu' forte e piu'
//! semplice, ed e' quella che `ENGINEERING.md § Spool e memoria` descrive:
//! il file e' creato con `tempfile::tempfile_in`, cioe' **scollegato dal
//! filesystem appena aperto** su Unix e con `FILE_FLAG_DELETE_ON_CLOSE` su
//! Windows.
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
/// Pubblica perche' lo spill non e' solo quello di [`StagedSpool`]: anche i
/// driver che tengono un proprio file temporaneo devono finire **dove
/// l'operatore ha scelto**. Finche' e' stata privata, `driver-xls` creava il
/// proprio spool nella directory temporanea di sistema e `PLENORA_SPILL_DIR`
/// non lo governava, pur dichiarando di governare lo spill.
///
/// # Errors
///
/// Restituisce un errore se `PLENORA_SPILL_DIR` e' impostata ma non e' una
/// directory utilizzabile. Non ripiega su un'altra directory: un ripiego
/// silenzioso metterebbe i dati su un volume che l'operatore non ha scelto.
pub fn spill_directory() -> Result<PathBuf> {
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
mod tests;
