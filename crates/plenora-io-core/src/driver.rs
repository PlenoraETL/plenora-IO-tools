//! Il confine plug-in: `FormatDriver` + handle/reader/writer (`ENGINEERING.md § Interfaccia dei driver`).

use std::collections::BTreeMap;
use std::path::PathBuf;

use arrow_array::{Array, BinaryArray, LargeBinaryArray, RecordBatch};
use arrow_schema::{DataType, SchemaRef};
use plenora_io_model::budget::{
    ConcurrencyLease, CountedLease, InputPermit, InternalMemoryLease, OperationBudget,
    OperationCounter, PipelineContext, PipelineLimits, ReadBudgetParts, SourceEntry,
    SourceFootprintSnapshot, WriteBudgetParts,
};
use plenora_io_model::contract::{
    CoordinateDimensions, FieldId, GeometryColumnContract, GeometryEncoding, GeometryType,
    LayerContract, LayerId,
};
use plenora_io_model::crs::CrsResolution;
use plenora_io_model::geometry::{is_geometry_field, read_geometry_contract_metadata};
use plenora_io_model::limits::WkbLimits;
use plenora_io_model::wkb::{inspect_wkb, WkbInspection};
use plenora_io_model::{
    CancellationReason, CancellationToken, CapabilityReason, ErrorCategory, ErrorPhase,
    KnownOrUnknownCount, PlenoraIoError, RemoteEffect, Result, RetryDisposition,
    RowDiagnosticColumn, RowDiagnosticExample, RowDiagnosticScope, RowDiagnosticWriteOutcome,
    RowDiagnosticWriteState, RowDiagnostics, RowDiagnosticsCompleteness,
    WriteDiagnosticStateCounts, ROW_DIAGNOSTICS_CONTRACT, ROW_DIAGNOSTICS_INDEX_BASIS,
    ROW_DIAGNOSTIC_COLUMN_UNATTESTABLE,
};
use plenora_io_model::{ContractIdentifier, IoErrorCode, NumeroStrutturale, PublicMessage};

use crate::descriptor::{
    ArrowTypeClass, AttributeWriteSupport, CrsRepresentationState, FormatDescriptor,
    GeometryWriteSupport, NullabilitySupport, TypeCoercionPolicy,
};
use crate::loss::{FidelityAssessment, FidelityReasonCode, LossExample, LossReport, Posizione};
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
    /// Preflight della sorgente: enumera, addebita e pubblica il footprint.
    ///
    /// # La forma
    ///
    /// L'enumerazione chiama [`PipelineContext::note_entry_visited`] una
    /// volta per voce **scoperta**, e quella singola chiamata applica insieme
    /// le tre grandezze che descrivono l'insieme osservato:
    /// `max_input_entries`, i byte addebitati contro `max_input_bytes`, e il
    /// digest dell'identita'. Erano tre controlli separati scritti qui;
    /// separarli rendeva osservabile uno stato intermedio e possibile un
    /// aggiornamento parziale.
    ///
    /// Il conteggio avviene alla scoperta e non al prelievo: contando in coda
    /// al pop, una directory con milioni di voci avrebbe gia' allocato
    /// milioni di `PathBuf` prima che il tetto potesse intervenire. Cosi'
    /// `pending` non supera mai `max_input_entries`.
    ///
    /// A enumerazione conclusa il permit viene speso in
    /// [`PipelineContext::observe_input`], che pubblica il footprint
    /// accumulato. Il permit e' preso per `move` e non e' `Clone`: una
    /// seconda osservazione non e' scrivibile.
    ///
    /// # I controlli legacy sono spariti qui dentro
    ///
    /// Non sono stati spostati: sono stati **rimossi nello stesso atto** in
    /// cui `note_entry_visited` ha iniziato ad applicarli. Lasciarli avrebbe
    /// applicato due volte le stesse quote — la seconda contro contatori che
    /// la prima aveva gia' consumato — e un input al limite sarebbe stato
    /// rifiutato per una quota che in realta' bastava.
    ///
    /// # Errors
    ///
    /// [`permit_gia_speso`] se l'osservazione e' gia'
    /// avvenuta; l'errore di enumerazione se una voce supera una quota;
    /// l'errore di cancellazione o deadline; l'errore di I/O se la sorgente
    /// non e' accessibile; `Unsupported` su un symlink.
    pub fn into_path_observed(self, opts: &mut ReadOptions) -> Result<PathBuf> {
        let Self::Path(path) = self;
        let budget = opts.budget().clone();
        let context = budget.context();
        let permit = opts.take_input_permit().ok_or_else(permit_gia_speso)?;

        // La liveness la verifica `scopri` per **ogni** voce, radice
        // compresa: qui non serve un controllo in piu'.
        let mut pending = Vec::new();
        if scopri(context, &path)? {
            pending.push(path.clone());
        }
        while let Some(directory) = pending.pop() {
            for entry in std::fs::read_dir(&directory)? {
                let figlio = entry?.path();
                if scopri(context, &figlio)? {
                    pending.push(figlio);
                }
            }
        }

        context.observe_input(permit)?;
        Ok(path)
    }
}

/// Errore di un preflight che trova il permit gia' speso.
///
/// Non e' un caso ordinario: significa che questa sorgente e' gia' stata
/// osservata con queste opzioni. Fallire e' l'unica risposta corretta —
/// proseguire senza osservare lascerebbe il footprint vuoto e
/// `output_expansion_ratio` senza base su cui derivare il tetto di uscita.
fn permit_gia_speso() -> PlenoraIoError {
    PlenoraIoError::limite_redatto(&PublicMessage::Curated(
        "il permit di osservazione dell'input e' gia' stato speso: la sorgente \
         non puo' essere osservata due volte",
    ))
}

/// Codifica senza perdita del percorso **lessicale**.
///
/// # Non e' una normalizzazione
///
/// Il nome precedente, `byte_identita_percorso`, prometteva cio' che questa
/// funzione non fa: `a/../b` e `b` restano distinti, e due hard link allo
/// stesso inode pure. Produce una codifica **iniettiva e stabile** del
/// percorso cosi' come il chiamante lo ha scritto, nient'altro.
///
/// La canonicalizzazione e' deliberatamente esclusa: `fs::canonicalize` segue
/// i symlink, e il preflight li **rifiuta** invece di seguirli. Farla qui
/// allargherebbe il contratto della sorgente nel punto in cui e' stato
/// ristretto — e introdurrebbe una lettura del filesystem in piu' per ogni
/// voce, prima ancora di sapere se la voce e' ammissibile.
///
/// # Perche' senza perdita
///
/// `to_string_lossy` sostituisce ogni sequenza non valida con U+FFFD: su Unix
/// due percorsi diversi ma entrambi non-UTF-8 possono cosi' collassare sulla
/// **stessa** stringa, e quindi sullo stesso digest. Il footprint direbbe che
/// due sorgenti distinte sono la stessa, ed e' esattamente cio' che il digest
/// esiste per escludere.
///
/// Su Unix si usano i byte nativi dell'`OsStr`. Su Windows le unita' UTF-16
/// vengono serializzate little-endian: non e' una stringa leggibile, ma non
/// deve esserlo — deve solo essere **iniettiva e stabile**, cosi' che due
/// corse sulla stessa sorgente producano lo stesso digest e due sorgenti
/// diverse no. Il prefisso distingue le due codifiche, perche' un digest non
/// deve dipendere dalla piattaforma senza dichiararlo.
#[cfg(unix)]
fn byte_identita_percorso(percorso: &std::path::Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    let mut byte = Vec::with_capacity(percorso.as_os_str().len() + 1);
    byte.push(b'u');
    byte.extend_from_slice(percorso.as_os_str().as_bytes());
    byte
}

#[cfg(windows)]
fn byte_identita_percorso(percorso: &std::path::Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    let mut byte = vec![b'w'];
    for unita in percorso.as_os_str().encode_wide() {
        byte.extend_from_slice(&unita.to_le_bytes());
    }
    byte
}

#[cfg(not(any(unix, windows)))]
fn byte_identita_percorso(percorso: &std::path::Path) -> Vec<u8> {
    // Nessuna piattaforma supportata cade qui. Se una ci cadesse, la forma
    // lossy sarebbe meglio di niente ma non e' iniettiva: il prefisso lo
    // dichiara, cosi' un digest costruito cosi' non si confonde con gli altri.
    let mut byte = vec![b'?'];
    byte.extend_from_slice(percorso.to_string_lossy().as_bytes());
    byte
}

/// Registra una voce appena scoperta e dice se va esplorata.
///
/// Fa quattro cose in quest'ordine, e l'ordine conta: verifica che
/// l'operazione sia viva **prima** di toccare il filesystem, rifiuta i
/// symlink prima di addebitare, addebita alla scoperta, e solo dopo dichiara
/// se la voce e' una directory da mettere in coda.
///
/// Il controllo di liveness sta qui e non solo prima di ogni directory: una
/// singola directory con molte voci comporta altrettante `symlink_metadata`,
/// e senza il controllo per voce una cancellazione non avrebbe effetto fino
/// alla fine di quella directory.
///
/// # Errors
///
/// Cancellazione o deadline scaduta; `Unsupported` su un symlink; l'errore di
/// I/O se i metadata non si leggono; l'errore di quota se la voce supera
/// `max_input_entries` o `max_input_bytes`.
fn scopri(context: &PipelineContext, percorso: &std::path::Path) -> Result<bool> {
    check_cancelled(context.cancellation(), ErrorPhase::Probe)?;
    context.ensure_active()?;
    let metadata = std::fs::symlink_metadata(percorso)?;
    if metadata.file_type().is_symlink() {
        return Err(PlenoraIoError::non_supportato_redatto(
            &PublicMessage::Curated("symlink non ammesso nella sorgente"),
        ));
    }
    let normalizzato = byte_identita_percorso(percorso);
    let modified = metadata.modified().ok();
    let entry = if metadata.is_dir() {
        SourceEntry::directory(&normalizzato, modified)
    } else {
        SourceEntry::file(&normalizzato, metadata.len(), modified)
    };
    context.note_entry_visited(&entry)?;
    Ok(metadata.is_dir())
}

/// Destinazione di scrittura (scheletro Fase 0).
pub enum Sink {
    /// File singolo o directory-dataset (multi-file), risolto dal driver.
    Path(PathBuf),
}

/// Quote che il writer comune applica, senza il tipo legacy.
///
/// Le struct del writer conservano questi valori e non un `Limits` intero:
/// sono gli unici che consultano, e portarsi dietro il tipo vecchio per tre
/// campi lo terrebbe in vita ben oltre la migrazione.
#[derive(Clone, Copy, Debug)]
pub struct WriteLimitsView {
    pub max_columns: usize,
    pub max_rows: usize,
    pub wkb: WkbLimits,
}

impl WriteLimitsView {
    /// Estrae le tre quote dai limiti della pipeline.
    ///
    /// `max_columns` e `max_rows` sono `u64` nel modello e `usize` qui: la
    /// conversione satura, e su un target a 64 bit non puo' perdere nulla.
    /// Saturare verso l'alto e' comunque il verso sicuro — il tetto che lega
    /// resta quello del contatore, che rifiuta prima.
    #[must_use]
    pub fn from_pipeline(limits: &PipelineLimits) -> Self {
        Self {
            max_columns: saturating_usize(limits.max_columns()),
            max_rows: saturating_usize(limits.max_rows()),
            wkb: limits.wkb_limits(),
        }
    }
}

/// Opzioni di lettura.
///
/// Non hanno `Default`, e non e' una dimenticanza: le opzioni portano un
/// [`OperationBudget`], che nasce da un `PipelineBudget::builder().build()`
/// e puo' **fallire** — limiti incoerenti, deadline gia' scaduta. Un
/// `Default` avrebbe dovuto scegliere fra il panico e quote inventate, e
/// fino a S4.d la seconda strada era quella presa: costruiva un ramo legacy
/// con i valori storici, che nessun chiamante aveva chiesto.
pub struct ReadOptions {
    /// CRS dichiarato per i formati che non lo portano (CSV/XLSX) — `PRODUCT.md § CRS`.
    pub assume_crs: Option<String>,
    /// Knob specifici del driver (es. csv: `x_column`/`y_column`/`wkt_column`/
    /// `delimiter`).
    pub format_options: BTreeMap<String, String>,
    budget: OperationBudget,
    /// Permit di osservazione, speso dal preflight. `None` dopo la spesa, o
    /// se le parti non ne trasportavano.
    permit: Option<InputPermit>,
    /// Snapshot atteso per la revalidation di `scan`.
    expected: Option<SourceFootprintSnapshot>,
}

impl ReadOptions {
    /// Costruisce le opzioni dalle parti di lettura.
    ///
    /// Permit e snapshot arrivano **per move**: rigenerarli darebbe un permit
    /// che il context non riconosce e uno snapshot che non descrive nulla di
    /// osservato.
    #[must_use]
    pub fn from_read_parts(parts: ReadBudgetParts) -> Self {
        let (budget, permit, expected) = parts.into_components();
        Self {
            assume_crs: None,
            format_options: BTreeMap::new(),
            budget,
            permit,
            expected,
        }
    }

    /// Budget dell'operazione: contatori cumulativi e `PipelineContext`.
    #[must_use]
    pub const fn budget(&self) -> &OperationBudget {
        &self.budget
    }

    #[must_use]
    pub fn cancellation(&self) -> &CancellationToken {
        self.budget.context().cancellation()
    }

    #[must_use]
    pub fn wkb_limits(&self) -> WkbLimits {
        self.budget.context().limits().wkb_limits()
    }

    #[must_use]
    pub fn max_columns(&self) -> usize {
        saturating_usize(self.budget.context().limits().max_columns())
    }

    #[must_use]
    pub fn max_rows(&self) -> usize {
        saturating_usize(self.budget.context().limits().max_rows())
    }

    #[must_use]
    pub fn max_vertices(&self) -> usize {
        self.budget.context().limits().max_vertices()
    }

    #[must_use]
    pub fn max_input_bytes(&self) -> u64 {
        self.budget.context().limits().max_input_bytes()
    }

    #[must_use]
    pub fn max_input_entries(&self) -> u64 {
        self.budget.context().limits().max_input_entries()
    }

    /// Verifica che l'operazione sia ancora viva: non cancellata e dentro la
    /// deadline.
    ///
    /// # Errors
    ///
    /// Cancellazione richiesta o propagata, oppure deadline scaduta.
    pub fn ensure_active(&self) -> Result<()> {
        self.budget.context().ensure_active()
    }

    /// Snapshot atteso, presente solo se le opzioni derivano da
    /// `ScanBudgetParts`.
    #[must_use]
    pub const fn expected_footprint(&self) -> Option<&SourceFootprintSnapshot> {
        self.expected.as_ref()
    }

    /// Estrae il permit di osservazione.
    ///
    /// `pub(crate)` e non `pub`: l'unico chiamante legittimo e'
    /// `Source::into_path_observed`, che vive in questo crate. Esporlo darebbe
    /// a un driver — o domani alla facade — un secondo punto da cui separare
    /// il permit dal proprio context, cioe' cio' che INV-13 esclude.
    pub(crate) const fn take_input_permit(&mut self) -> Option<InputPermit> {
        self.permit.take()
    }

    /// Dichiara il CRS per i formati che non lo trasportano.
    #[must_use]
    pub fn with_assume_crs(mut self, crs: impl Into<String>) -> Self {
        self.assume_crs = Some(crs.into());
        self
    }

    #[must_use]
    pub fn with_format_options(mut self, format_options: BTreeMap<String, String>) -> Self {
        self.format_options = format_options;
        self
    }

    #[must_use]
    pub fn with_format_option(
        mut self,
        chiave: impl Into<String>,
        valore: impl Into<String>,
    ) -> Self {
        self.format_options.insert(chiave.into(), valore.into());
        self
    }
}

/// Opzioni di scrittura. Come [`ReadOptions`], senza `Default`.
pub struct WriteOptions {
    /// Profilo `DurableAtomicPublish` (fsync) invece di `AtomicPublish` — `ENGINEERING.md § Pipeline di scrittura`.
    pub durable: bool,
    /// Knob specifici del driver.
    pub format_options: BTreeMap<String, String>,
    budget: OperationBudget,
}

impl WriteOptions {
    /// Costruisce le opzioni dalle parti di scrittura.
    ///
    /// In una conversione le parti write e read escono dallo stesso
    /// `ConvertBudgetParts`, quindi condividono il `PipelineContext`: memoria,
    /// spill e deadline sono gli stessi, mentre i contatori cumulativi restano
    /// indipendenti. E' cio' che il finding #3 richiedeva — una riga non deve
    /// consumare due volte la stessa quota — senza pero' tornare a due budget
    /// scollegati, che e' quello che la CLI faceva fino a S4.d.
    #[must_use]
    pub fn from_write_parts(parts: WriteBudgetParts) -> Self {
        Self {
            durable: false,
            format_options: BTreeMap::new(),
            budget: parts.into_budget(),
        }
    }

    #[must_use]
    pub const fn budget(&self) -> &OperationBudget {
        &self.budget
    }

    /// Limite fisico effettivo, incluso il fattore massimo di espansione R7.7.
    #[must_use]
    pub fn max_output_bytes(&self) -> u64 {
        self.budget.output_limit()
    }

    #[must_use]
    pub fn cancellation(&self) -> &CancellationToken {
        self.budget.context().cancellation()
    }

    #[must_use]
    pub fn wkb_limits(&self) -> WkbLimits {
        self.budget.context().limits().wkb_limits()
    }

    #[must_use]
    pub fn max_columns(&self) -> usize {
        saturating_usize(self.budget.context().limits().max_columns())
    }

    #[must_use]
    pub fn write_limits(&self) -> WriteLimitsView {
        WriteLimitsView::from_pipeline(self.budget.context().limits())
    }

    /// Seleziona il profilo `DurableAtomicPublish` invece di `AtomicPublish`.
    #[must_use]
    pub const fn with_durable(mut self, durable: bool) -> Self {
        self.durable = durable;
        self
    }

    #[must_use]
    pub fn with_format_options(mut self, format_options: BTreeMap<String, String>) -> Self {
        self.format_options = format_options;
        self
    }

    #[must_use]
    pub fn with_format_option(
        mut self,
        chiave: impl Into<String>,
        valore: impl Into<String>,
    ) -> Self {
        self.format_options.insert(chiave.into(), valore.into());
        self
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
#[must_use]
pub const fn saturating_u64(value: usize) -> u64 {
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
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(operazione)).unwrap_or_else(|_| {
        Err(PlenoraIoError::formato_redatto(
            driver,
            &PublicMessage::Curated(MESSAGGIO_PANICO_ARROW),
        ))
    })
}

/// Messaggio pubblico del panico di arrow, statico e curato (FZ-0).
///
/// Prima portava un'impronta FNV del messaggio del panico. L'impronta e' stata
/// tolta: e' un valore che nasce dall'input e finisce in un errore
/// serializzato, registrato e passato agli altri componenti, e per un bordo che
/// promette di non far uscire nulla di derivato dal payload e' una promessa in
/// meno. La correlazione fra occorrenze resta possibile dai log del processo,
/// dove l'hook di panico scrive comunque il testo completo — che e' una
/// risorsa del processo, non di questa libreria.
pub(crate) const MESSAGGIO_PANICO_ARROW: &str =
    "arrow e' andata in panico decodificando un input non conforme";

pub trait FormatDriver: Send + Sync {
    fn descriptor(&self) -> &FormatDescriptor;
    /// Statico: header/schema/CRS, nessuna riga.
    ///
    /// Consuma le opzioni **per valore**. Non e' una preferenza stilistica:
    /// le opzioni trasportano l'`InputPermit`, che il preflight deve
    /// estrarre per `move` — e da un `&ReadOptions` non si estrae nulla.
    /// Le alternative che conservano il riferimento condiviso
    /// (`Mutex<Option<InputPermit>>`, o un permit clonato) reintrodurrebbero
    /// proprio cio' che il permit esiste per escludere: uno stato mutabile
    /// nascosto dietro una firma immutabile, e la possibilita' di osservare
    /// due volte lo stesso input.
    ///
    /// L'implementazione dichiara `mut opts` quando chiama il preflight, e
    /// continua a usare `opts` dopo: consumare il permit non consuma le
    /// opzioni.
    ///
    /// # Errors
    ///
    /// Restituisce un errore se la sorgente non è accessibile, non è nel
    /// formato atteso o eccede i limiti dichiarati in `opts`.
    fn open(&self, source: Source, opts: ReadOptions) -> Result<Box<dyn OpenDatasetHandle>>;
    /// Statico: verifica che il contratto sia rappresentabile (`ENGINEERING.md § Pipeline di scrittura (capability-check`)).
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
    /// Valutazione di fedeltà concreta per il dataset aperto (`PRODUCT.md § LossReport`).
    fn fidelity_assessment(&self) -> FidelityAssessment;
    /// Apre un reader indipendente per un layer; lo STATO mutabile vive nel
    /// reader (`ENGINEERING.md § Interfaccia dei driver`).
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
    /// (`ENGINEERING.md § Projection e pruning`). Riflette la projection realmente applicata.
    fn contract(&self) -> &LayerContract;
    /// Pull-based con stato; `None` = fine dello stream.
    ///
    /// # Errors
    ///
    /// Restituisce un errore se il flusso sorgente è malformato, se un limite
    /// viene superato o se l'operazione viene annullata.
    fn next_batch(&mut self) -> Result<Option<RecordBatch>>;
    /// Cardinalità **già accettata e ancora da consegnare più quella già
    /// consegnata**: il numero esatto di righe che questo reader ha ammesso
    /// per l'intero scope della richiesta.
    ///
    /// Contratto, in tre clausole che stanno insieme:
    ///
    /// 1. `Some(n)` solo dopo che il reader ha **completato** l'esame dello
    ///    scope senza violazioni: `n` è allora la somma delle righe di tutti i
    ///    batch che `next_batch` consegnerà, contando anche quelli già
    ///    consegnati. Non è una stima e non è un limite superiore.
    /// 2. `None` finché quel numero non è un fatto: prima che l'esame sia
    ///    concluso, e per ogni reader che consegna in streaming vero e non
    ///    può conoscere il totale senza aver letto tutto.
    /// 3. Dopo un errore terminale è di nuovo `None`: un totale sopravvissuto
    ///    all'errore che lo invalida sarebbe la peggiore delle due risposte.
    ///
    /// Il default è `None`, che è la risposta onesta di un reader che non sa.
    /// Chi ha bisogno del totale **prima** di scrivere — `declare_input_total`
    /// lo esige prima del primo write del layer — lo ottiene chiamando
    /// `next_batch` una volta: l'adapter operation-atomic conclude lì l'esame
    /// dello scope, e da quel momento il totale è noto senza che il chiamante
    /// abbia dovuto trattenere in memoria più di un batch.
    fn accepted_total(&self) -> Option<u64> {
        None
    }
    /// Report di perdita (vuoto per i driver Lossless) — `PRODUCT.md § LossReport`.
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
    /// `LayerId(0)` e delega a `write`; i driver multi-layer fanno override (`ENGINEERING.md § Interfaccia dei driver:`
    /// un dataset-writer coordina tutti i layer con un unico commit atomico).
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::Unsupported`] se il formato non è
    /// multi-layer e `layer` non è `LayerId(0)`; per il resto gli stessi
    /// errori di [`FormatWriter::write`].
    fn write_to_layer(&mut self, layer: LayerId, batch: &RecordBatch) -> Result<()> {
        if layer.0 != 0 {
            return Err(PlenoraIoError::non_supportato_redatto(
                &PublicMessage::Curated("questo formato non supporta la scrittura multi-layer"),
            ));
        }
        self.write(batch)
    }
    /// Publish del dataset a successo. `Ok(Published)` implica che tutte le
    /// componenti dichiarate dal driver sono visibili nella destinazione;
    /// `Err` implica il tentativo di non lasciare nulla di visibile.
    ///
    /// La garanzia d'atomicita' del publish e' documentata per driver
    /// (`ENGINEERING.md § Pipeline di scrittura`). I formati che pubblicano un file singolo o un
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

/// Preflight della sorgente per il percorso di lettura.
///
/// Punto unico per tutti i driver. Prima di S4.c ognuno ripeteva le quattro
/// righe di `Source::into_path_checked`, con l'estrazione del budget legacy
/// scritta a mano: tredici copie di una decisione che S4.d deve cambiare **in
/// un atto solo**, perche' il nuovo preflight enumera la sorgente attraverso
/// `note_entry_visited` e i controlli qui devono sparire nello stesso
/// istante, altrimenti le stesse quote si applicano due volte.
///
/// # Errors
///
/// Propaga l'errore dell'enumerazione: quota superata, cancellazione,
/// deadline, symlink, o permit gia' speso. Fallisce inoltre se le
/// `format_options` non rispettano lo schema dichiarato dal driver (L0.7).
pub fn preflight_source(
    descriptor: &crate::descriptor::FormatDescriptor,
    source: Source,
    opts: &mut ReadOptions,
) -> Result<PathBuf> {
    // Le `format_options` si validano **prima** di toccare il filesystem: una
    // chiave sbagliata e' un errore di configurazione, e diventa un errore di
    // I/O solo se qualcuno la scopre dopo aver aperto il file. Il descrittore
    // e' un parametro e non un campo delle opzioni perche' cosi' un driver che
    // salta la validazione non compila: la firma e' il vincolo.
    plenora_io_model::format_options::valida_opzioni(
        descriptor.id(),
        descriptor.format_options(),
        &opts.format_options,
        plenora_io_model::format_options::FaseOpzione::Lettura,
    )?;
    source.into_path_observed(opts)
}

/// Applica i limiti indipendenti dal formato a qualunque writer. I vincoli
/// specifici (WKB, vertici, dimensione fisica del dataset) restano nel driver.
///
/// Prende le **opzioni** e non una vista gia' estratta: e' da li' che vengono
/// sia le quote sia il budget dei contatori, e chiederli separatamente
/// lascerebbe al chiamante la possibilita' di accoppiare quote di
/// un'operazione con i contatori di un'altra.
#[must_use]
pub fn with_write_limits(
    writer: Box<dyn FormatWriter>,
    opts: &WriteOptions,
) -> Box<dyn FormatWriter> {
    Box::new(LimitedWriter {
        inner: writer,
        driver: "writer",
        limits: opts.write_limits(),
        rows: 0,
        layer_rows: vec![0],
        input_totals: vec![None],
        failed: false,
        contracts: Vec::new(),
        geometry_validation: None,
        planned_loss: LossReport::default(),
        cancellation: opts.cancellation().clone(),
        budget: opts.budget().clone(),
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
        .enumerate()
        .map(
            |(indice_layer, layer)| -> Result<Option<GeometryColumnContract>> {
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
                    // Il nome del layer non entra nel testo: il layer si nomina
                    // per indice nel piano.
                    return Err(PlenoraIoError::contratto_redatto(
                        &PublicMessage::CuratedWith(
                            "più colonne GeoArrow senza contratto geometrico esplicito al layer",
                            NumeroStrutturale::Indice(saturating_u64(indice_layer)),
                        ),
                    ));
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
            },
        )
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
    opts: &WriteOptions,
) -> Result<Box<dyn FormatWriter>> {
    let limits = opts.write_limits();
    let cancellation = opts.cancellation().clone();
    let budget = opts.budget().clone();
    let geometry_support = descriptor
        .write_capabilities()
        .map(|capabilities| capabilities.geometry);
    let layers = geometry_contracts_for_validation(plan)?;
    let planned_loss = planned_write_loss(descriptor, plan);
    let fidelity = assess_write_contract(descriptor, plan).with_loss_report(&planned_loss);
    budget.context().ensure_active()?;
    let operation_lease = budget.context().lease_concurrency()?;
    let columns = plan.layers.iter().try_fold(0_u64, |total, layer| {
        total
            .checked_add(
                u64::try_from(layer.contract.schema.fields().len()).map_err(|_| {
                    PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                        "troppe colonne nel piano",
                    ))
                })?,
            )
            .ok_or_else(|| {
                PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                    "overflow nel conteggio delle colonne",
                ))
            })
    })?;
    if columns > 0 {
        budget
            .try_lease(OperationCounter::Columns, columns)?
            .commit(columns)?;
    }
    Ok(Box::new(LimitedWriter {
        inner: writer,
        driver: descriptor.id(),
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
        budget,
        _operation_lease: Some(operation_lease),
        geometry_validation: geometry_support.map(|support| GeometryValidation {
            driver: descriptor.id(),
            support,
            layers,
        }),
    }))
}

fn planned_write_loss(descriptor: &FormatDescriptor, plan: &WritePlan) -> LossReport {
    let mut loss = LossReport::default();
    let Some(capabilities) = descriptor.write_capabilities() else {
        return loss;
    };

    for (indice_layer, layer) in plan.layers.iter().enumerate() {
        let layer_index = Some(saturating_u64(indice_layer));
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
                Posizione {
                    layer_index,
                    field_index: indice_della_geometria(layer, &geometry.name),
                    type_class: None,
                },
                RappresentazioneDelCrs::CrsId,
                crs_id.map(str::len),
                capabilities.crs_representations.crs_id,
            );
            record_crs_representation_loss(
                &mut loss,
                Posizione {
                    layer_index,
                    field_index: indice_della_geometria(layer, &geometry.name),
                    type_class: None,
                },
                RappresentazioneDelCrs::Srid,
                geometry.srid.map(|srid| srid.to_string().len()),
                capabilities.crs_representations.srid,
            );
            record_crs_representation_loss(
                &mut loss,
                Posizione {
                    layer_index,
                    field_index: indice_della_geometria(layer, &geometry.name),
                    type_class: None,
                },
                RappresentazioneDelCrs::CrsDefinition,
                crs_definition.map(str::len),
                capabilities.crs_representations.crs_definition,
            );
        }

        let geometry_name = layer
            .contract
            .geometry
            .as_ref()
            .map(|geometry| geometry.name.as_str());
        for (indice_campo, field) in layer.contract.schema.fields().iter().enumerate() {
            if geometry_name == Some(field.name().as_str()) || is_geometry_field(field) {
                continue;
            }
            let type_class = crate::capabilities::arrow_type_class(field.data_type());
            let unsupported_text_coercion = !capabilities.allowed_types.contains(&type_class)
                && matches!(
                    capabilities.type_coercion,
                    TypeCoercionPolicy::ExplicitText | TypeCoercionPolicy::LossReported
                );
            let kml_scalar_to_text = descriptor.id() == "kml" && type_class != ArrowTypeClass::Utf8;
            let gpkg_type_normalization = descriptor.id() == "gpkg"
                && !matches!(
                    field.data_type(),
                    DataType::Int64 | DataType::Float64 | DataType::Utf8 | DataType::Binary
                );
            if unsupported_text_coercion || kml_scalar_to_text || gpkg_type_normalization {
                loss.record("coercion tipo attributo", 1);
                loss.add_example(LossExample {
                    category: "coercion tipo attributo".to_owned(),
                    posizione: Posizione {
                        layer_index,
                        field_index: Some(saturating_u64(indice_campo)),
                        type_class: Some(type_class),
                    },
                    context: "il tipo dell'attributo richiede una coercizione".to_owned(),
                });
            }
        }
    }
    loss
}

/// Quale delle tre rappresentazioni del CRS non e' stata preservata.
///
/// Un tipo e non una stringa: la categoria di perdita che ne esce e' una
/// **chiave sul filo**, e una chiave costruita con `format!` e' una chiave che
/// nessuno puo' enumerare leggendo il codice. Le sei combinazioni sono qui,
/// scritte per esteso, ed e' cio' che permette al registro di dichiararle e al
/// gate di verificarle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RappresentazioneDelCrs {
    CrsId,
    Srid,
    CrsDefinition,
}

impl RappresentazioneDelCrs {
    /// Il nome della rappresentazione, per la diagnostica strutturale.
    const fn nome(self) -> &'static str {
        match self {
            Self::CrsId => "crs_id",
            Self::Srid => "srid",
            Self::CrsDefinition => "crs_definition",
        }
    }

    /// La categoria di perdita, una delle **sei** che questa coppia produce.
    const fn categoria(self, stato: CrsRepresentationState) -> Option<&'static str> {
        match (self, stato) {
            (_, CrsRepresentationState::Preserved) => None,
            (Self::CrsId, CrsRepresentationState::Absent) => Some(CRS_ID_NOT_PRESERVED_ABSENT),
            (Self::CrsId, CrsRepresentationState::Derived) => Some(CRS_ID_NOT_PRESERVED_DERIVED),
            (Self::Srid, CrsRepresentationState::Absent) => Some(SRID_NOT_PRESERVED_ABSENT),
            (Self::Srid, CrsRepresentationState::Derived) => Some(SRID_NOT_PRESERVED_DERIVED),
            (Self::CrsDefinition, CrsRepresentationState::Absent) => {
                Some(CRS_DEFINITION_NOT_PRESERVED_ABSENT)
            }
            (Self::CrsDefinition, CrsRepresentationState::Derived) => {
                Some(CRS_DEFINITION_NOT_PRESERVED_DERIVED)
            }
        }
    }
}

const CRS_ID_NOT_PRESERVED_ABSENT: &str = "crs_id_not_preserved_absent";
const CRS_ID_NOT_PRESERVED_DERIVED: &str = "crs_id_not_preserved_derived";
const SRID_NOT_PRESERVED_ABSENT: &str = "srid_not_preserved_absent";
const SRID_NOT_PRESERVED_DERIVED: &str = "srid_not_preserved_derived";
const CRS_DEFINITION_NOT_PRESERVED_ABSENT: &str = "crs_definition_not_preserved_absent";
const CRS_DEFINITION_NOT_PRESERVED_DERIVED: &str = "crs_definition_not_preserved_derived";

/// L'indice della colonna geometrica in `schema.fields()`.
///
/// Il contratto nomina la geometria, la posizione la conta: e' la stessa
/// sequenza che gli altri `field_index` indicizzano, quindi il numero e'
/// confrontabile con i loro. `None` se il contratto nomina una colonna che lo
/// schema non ha -- che sarebbe un'incoerenza da dichiarare altrove, non da
/// nascondere qui con uno zero.
fn indice_della_geometria(layer: &crate::request::WriteLayer, nome: &str) -> Option<u64> {
    layer
        .contract
        .schema
        .fields()
        .iter()
        .position(|field| field.name() == nome)
        .map(saturating_u64)
}

fn record_crs_representation_loss(
    loss: &mut LossReport,
    dove: Posizione,
    representation: RappresentazioneDelCrs,
    value_bytes: Option<usize>,
    state: CrsRepresentationState,
) {
    // Due guardie e non una tupla: la categoria e' una **chiave sul filo**, e
    // legarla da sola la rende leggibile a chi la cerca -- il gate del
    // vocabolario compreso, che deve poter risalire dall'uso alla costante.
    let Some(category) = representation.categoria(state) else {
        return;
    };
    let Some(value_bytes) = value_bytes else {
        return;
    };
    let nome = representation.nome();
    loss.record(category, 1);
    // `nome` viene dal nostro vocabolario chiuso e `value_bytes` e' una
    // lunghezza: nessuno dei due e' un identificatore preso dal file. Dove si
    // sia persa la rappresentazione lo dice `posizione`.
    loss.add_example(LossExample {
        category: category.to_owned(),
        posizione: dove,
        context: format!("representation={nome} value_bytes={value_bytes}"),
    });
}

fn assess_write_contract(descriptor: &FormatDescriptor, plan: &WritePlan) -> FidelityAssessment {
    let mut assessment =
        FidelityAssessment::for_format(descriptor.id(), descriptor.fidelity_class());
    let Some(capabilities) = descriptor.write_capabilities() else {
        return assessment;
    };

    // I quattro siti che portavano nomi presi dal file. Ciascuno emette ora due
    // cose: un testo **curato** con la posizione strutturata, che e' cio' che il
    // v2 pubblica, e la frase congelata alla lettera, che e' cio' che il v1
    // continua a pubblicare. Non si ricostruisce: si conserva, cosi' il
    // congelamento del v1 e' una tautologia invece di un invariante da
    // difendere a ogni ritocco di un `format!`.
    //
    // `field_index` e' l'indice in `schema.fields()` e conta **anche** la
    // colonna geometrica: quella e' la sequenza che questo ciclo attraversa, e
    // un indice che ne saltasse un elemento non sarebbe l'indice di questa
    // sequenza. Il ramo la salta come *ragione*, non come *posizione*.
    for (indice_layer, layer) in plan.layers.iter().enumerate() {
        let layer_index = Some(saturating_u64(indice_layer));
        let geometry_name = layer
            .contract
            .geometry
            .as_ref()
            .map(|geometry| geometry.name.as_str());
        for (indice_campo, field) in layer.contract.schema.fields().iter().enumerate() {
            let field_index = Some(saturating_u64(indice_campo));
            let dove = Posizione {
                layer_index,
                field_index,
                type_class: None,
            };
            let is_geometry = geometry_name == Some(field.name().as_str());
            if !is_geometry && capabilities.attributes == AttributeWriteSupport::LossReported {
                assessment.add_reason_redatta(
                    FidelityReasonCode::AttributeLoss,
                    "l'attributo non e' nativo del formato, o e' dichiarato come perdita",
                    dove,
                    format!(
                        "{}: attributo '{}' non nativo o loss-reported",
                        layer.name,
                        field.name()
                    ),
                );
            }
            let classe = crate::capabilities::arrow_type_class(field.data_type());
            if !capabilities.allowed_types.contains(&classe)
                && capabilities.type_coercion == TypeCoercionPolicy::LossReported
            {
                assessment.add_reason_redatta(
                    FidelityReasonCode::TypeCoercion,
                    "il tipo dell'attributo richiede una coercizione",
                    Posizione {
                        // La **classe**, non la forma `Debug` del tipo di
                        // `arrow`: quella e' di una dipendenza, e un suo
                        // aggiornamento cambierebbe la busta senza che nessuno
                        // tocchi il protocollo.
                        type_class: Some(classe),
                        ..dove
                    },
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
                assessment.add_reason_redatta(
                    FidelityReasonCode::NullabilityChanged,
                    "la nullability dell'attributo la definisce il formato",
                    dove,
                    format!(
                        "{}: nullability di '{}' definita dal formato",
                        layer.name,
                        field.name()
                    ),
                );
            }
        }

        if descriptor.id() == "dxf"
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
            assessment.add_reason_redatta(
                FidelityReasonCode::StructureChanged,
                "le geometrie multipart sono esplose in entita' singole",
                Posizione {
                    layer_index,
                    field_index: None,
                    type_class: None,
                },
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
    limits: WriteLimitsView,
    rows: usize,
    layer_rows: Vec<u64>,
    input_totals: Vec<Option<u64>>,
    failed: bool,
    contracts: Vec<SchemaRef>,
    geometry_validation: Option<GeometryValidation>,
    fidelity: FidelityAssessment,
    planned_loss: LossReport,
    cancellation: CancellationToken,
    budget: OperationBudget,
    _operation_lease: Option<ConcurrencyLease>,
}

struct WriteBatchResources {
    rows: u64,
    bytes: u64,
    rows_lease: Option<CountedLease>,
    output_lease: Option<CountedLease>,
    /// La memoria dello staging del writer: prenotazione viva, restituita al
    /// drop come ogni occupazione interna (INV-5).
    memory_lease: Option<InternalMemoryLease>,
    geometry_components: u64,
    geometry_lease: Option<CountedLease>,
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
                    PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                        "budget geometrico esaurito",
                    ))
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
        self.budget.context().ensure_active()?;
        if let Some(contract) = self.contracts.get(layer) {
            if batch.schema().as_ref() != contract.as_ref() {
                // `redatto` con `Generic` e non `schema_redatto`: il sito usava
                // `PlenoraIoError::new`, che imposta `code = Generic`, mentre
                // `schema_redatto` imposta `code = Schema`. S9 non cambia il wire, e
                // `code` e' parte della chiave di compatibilita' ratificata insieme a
                // category, phase e retry.
                //
                // `Schema` sarebbe piu' preciso di `Generic` per una discordanza di
                // schema, ma renderlo tale e' una decisione da ratificare, non una
                // conseguenza di un refactor sui messaggi.
                return Err(PlenoraIoError::redatto(
                    IoErrorCode::Generic,
                    ErrorCategory::Schema,
                    ErrorPhase::Validate,
                    RemoteEffect::None,
                    RetryDisposition::Never,
                    &PublicMessage::CuratedWith(
                        "batch diverso dal contratto dichiarato (schema, ordine, tipi, \
                         nullability o metadata) al layer",
                        NumeroStrutturale::Indice(saturating_u64(layer)),
                    ),
                ));
            }
        } else if !self.contracts.is_empty() {
            return Err(PlenoraIoError::capability_redatta(
                self.driver,
                None,
                CapabilityReason::MultipleLayers,
                &layer_fuori_dal_piano(saturating_u64(layer)),
            ));
        }
        if batch.num_columns() > self.limits.max_columns {
            return Err(PlenoraIoError::limite_redatto(
                &PublicMessage::CuratedBetween(
                    "batch con",
                    NumeroStrutturale::Conteggio(saturating_u64(batch.num_columns())),
                    "colonne oltre il limite di",
                    NumeroStrutturale::Limite(saturating_u64(self.limits.max_columns)),
                ),
            ));
        }
        let batch_rows = u64::try_from(batch.num_rows()).map_err(|_| {
            PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                "batch oltre il conteggio supportato",
            ))
        })?;
        let layer_rows = *self.layer_rows.get(layer).ok_or_else(|| {
            PlenoraIoError::contratto_redatto(&layer_fuori_dal_piano(saturating_u64(layer)))
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
            return Err(PlenoraIoError::contratto_redatto(&PublicMessage::Curated(
                "write oltre input_total dichiarato",
            )));
        }
        self.rows = self.rows.checked_add(batch.num_rows()).ok_or_else(|| {
            PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                "overflow nel conteggio delle righe",
            ))
        })?;
        if self.rows > self.limits.max_rows {
            return Err(PlenoraIoError::limite_redatto(
                &PublicMessage::CuratedBetween(
                    "scritte",
                    NumeroStrutturale::Conteggio(saturating_u64(self.rows)),
                    "righe oltre il limite di",
                    NumeroStrutturale::Limite(saturating_u64(self.limits.max_rows)),
                ),
            ));
        }
        let geometry_components = if let Some(validation) = &self.geometry_validation {
            let mut effective_limits = self.limits;
            effective_limits.wkb.max_cell_bytes = effective_limits
                .wkb
                .max_cell_bytes
                .min(self.budget.context().limits().max_wkb_cell_bytes());
            effective_limits.wkb.max_components = effective_limits.wkb.max_components.min(
                saturating_usize(self.budget.remaining(OperationCounter::GeometryComponents)),
            );
            effective_limits.wkb.max_depth = effective_limits
                .wkb
                .max_depth
                .min(self.budget.context().limits().max_wkb_depth());
            validate_geometry_batch_at(
                validation.driver,
                validation.support,
                validation
                    .layers
                    .get(layer)
                    .ok_or_else(|| {
                        PlenoraIoError::capability_redatta(
                            validation.driver,
                            None,
                            CapabilityReason::MultipleLayers,
                            &layer_fuori_dal_piano(saturating_u64(layer)),
                        )
                    })?
                    .as_ref(),
                batch,
                effective_limits.wkb,
                *self.layer_rows.get(layer).ok_or_else(|| {
                    PlenoraIoError::contratto_redatto(&layer_fuori_dal_piano(saturating_u64(layer)))
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
            PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                "batch oltre il conteggio byte supportato",
            ))
        })?;
        Ok(WriteBatchResources {
            rows,
            bytes,
            rows_lease: Some(self.budget.try_lease(OperationCounter::Rows, rows)?),
            output_lease: (bytes > 0)
                .then(|| self.budget.try_lease(OperationCounter::OutputBytes, bytes))
                .transpose()?,
            memory_lease: (bytes > 0)
                .then(|| self.budget.context().lease_memory_internal(bytes))
                .transpose()?,
            geometry_components,
            geometry_lease: (geometry_components > 0)
                .then(|| {
                    self.budget
                        .try_lease(OperationCounter::GeometryComponents, geometry_components)
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
            return Err(PlenoraIoError::contratto_redatto(&PublicMessage::Curated(
                "input_total deve essere dichiarato prima del primo write del layer",
            )));
        }
        let slot = self.input_totals.get(layer_index).ok_or_else(|| {
            PlenoraIoError::contratto_redatto(&layer_fuori_dal_piano(u64::from(layer.0)))
        })?;
        if slot.is_some_and(|declared| declared != total) {
            return Err(PlenoraIoError::contratto_redatto(&PublicMessage::Curated(
                "input_total dichiarato in modo incoerente",
            )));
        }
        self.inner.declare_input_total(layer, total)?;
        self.input_totals[layer_index] = Some(total);
        Ok(())
    }

    fn write(&mut self, batch: &RecordBatch) -> Result<()> {
        check_cancelled(&self.cancellation, ErrorPhase::Write)?;
        if self.failed {
            return Err(PlenoraIoError::formato_redatto(
                self.driver,
                &PublicMessage::Curated("writer invalidato da un precedente errore di scrittura"),
            )
            .during(plenora_io_model::ErrorPhase::Write));
        }
        let result = self.account(0, batch).and_then(|resources| {
            let rows = resources.rows;
            self.inner.write(batch)?;
            resources.commit()?;
            self.layer_rows[0] = self.layer_rows[0].checked_add(rows).ok_or_else(|| {
                PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                    "overflow nel conteggio righe layer",
                ))
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
            return Err(PlenoraIoError::formato_redatto(
                self.driver,
                &PublicMessage::Curated("writer invalidato da un precedente errore di scrittura"),
            )
            .during(plenora_io_model::ErrorPhase::Write));
        }
        let result = self.account(layer.0 as usize, batch).and_then(|resources| {
            let rows = resources.rows;
            self.inner.write_to_layer(layer, batch)?;
            resources.commit()?;
            let layer_rows = self.layer_rows.get_mut(layer.0 as usize).ok_or_else(|| {
                PlenoraIoError::contratto_redatto(&layer_fuori_dal_piano(u64::from(layer.0)))
            })?;
            *layer_rows = layer_rows.checked_add(rows).ok_or_else(|| {
                PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                    "overflow nel conteggio righe layer",
                ))
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
        self.budget.context().ensure_active()?;
        if self.failed {
            return Err(PlenoraIoError::formato_redatto(
                self.driver,
                &PublicMessage::Curated("finish vietato dopo un errore di scrittura"),
            )
            .during(plenora_io_model::ErrorPhase::Finalize));
        }
        if self
            .input_totals
            .iter()
            .zip(&self.layer_rows)
            .any(|(declared, observed)| declared.is_some_and(|total| total != *observed))
        {
            return Err(PlenoraIoError::contratto_redatto(&PublicMessage::Curated(
                "EOF prima dell'input_total esatto dichiarato",
            )));
        }
        let mut published = self.inner.finish()?;
        published.loss.merge(&self.planned_loss);
        published.fidelity = self.fidelity.with_loss_report(&published.loss);
        Ok(published)
    }
}

/// Il messaggio del layer runtime che il `WritePlan` non dichiara.
///
/// Cinque siti lo producevano con altrettanti `format!` identici. Uno solo, e
/// il numero e' un indice strutturale: non viene dal payload, viene dal piano.
const fn layer_fuori_dal_piano(layer: u64) -> PublicMessage {
    PublicMessage::CuratedWith(
        "fuori dal WritePlan il layer runtime",
        NumeroStrutturale::Indice(layer),
    )
}

fn geometry_violation(
    driver: &'static str,
    field: Option<&ContractIdentifier>,
    reason: CapabilityReason,
    detail: &PublicMessage,
) -> PlenoraIoError {
    PlenoraIoError::capability_redatta(driver, field, reason, detail)
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
            ContractIdentifier::from_geometry_column(contract).as_ref(),
            CapabilityReason::CoordinateDimensions,
            &PublicMessage::CuratedPair(
                "dimensioni del payload diverse da quelle dichiarate dal contratto:",
                actual_dimensions.nome(),
            ),
        ));
    }
    if !support.dimensions.contains(&actual_dimensions) {
        return Err(geometry_violation(
            driver,
            ContractIdentifier::from_geometry_column(contract).as_ref(),
            CapabilityReason::CoordinateDimensions,
            &PublicMessage::CuratedPair(
                "dimensioni del payload non supportate dal driver:",
                actual_dimensions.nome(),
            ),
        ));
    }

    let allow_srid = contract.encoding == GeometryEncoding::Ewkb;
    if !geometry.nested_dimensions_coherent || (!allow_srid && geometry.contains_srid) {
        return Err(geometry_violation(
            driver,
            ContractIdentifier::from_geometry_column(contract).as_ref(),
            if allow_srid {
                CapabilityReason::CoordinateDimensions
            } else {
                CapabilityReason::GeometryEncoding
            },
            &PublicMessage::Curated(
                "componenti WKB con dimensioni incoerenti o SRID EWKB non dichiarato",
            ),
        ));
    }
    if contract.encoding == GeometryEncoding::Ewkb && geometry.srid != contract.srid {
        return Err(geometry_violation(
            driver,
            ContractIdentifier::from_geometry_column(contract).as_ref(),
            CapabilityReason::GeometryEncoding,
            // Gli SRID non entrano: sono numeri **letti dal payload**, e il
            // vincolo di S9 ammette solo indici, conteggi, tetti e codici
            // strutturali.
            &PublicMessage::Curated("SRID del payload diverso da quello dichiarato"),
        ));
    }
    if !contract.geometry_types.is_empty()
        && !contract.geometry_types.contains(&geometry.geometry_type)
    {
        return Err(geometry_violation(
            driver,
            ContractIdentifier::from_geometry_column(contract).as_ref(),
            CapabilityReason::MixedGeometry,
            &PublicMessage::CuratedPair(
                "tipo geometrico assente da quelli dichiarati:",
                geometry.geometry_type.canonical_name(),
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
    wkb_limits: WkbLimits,
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
                ContractIdentifier::from_geometry_column(contract).as_ref(),
                CapabilityReason::GeometryNotSupported,
                &PublicMessage::Curated("colonna geometrica dichiarata assente dal batch"),
            )
        })?;
    let array = batch.column(index);

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
            ContractIdentifier::from_geometry_column(contract).as_ref(),
            CapabilityReason::GeometryEncoding,
            &PublicMessage::Curated("colonna geometrica runtime non Binary/LargeBinary"),
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
                        PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                            "indice riga oltre u64",
                        ))
                    })?)
                    .ok_or_else(|| {
                        PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                            "overflow nell'indice riga",
                        ))
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
        .checked_add(u64::try_from(row).map_err(|_| {
            PlenoraIoError::limite_redatto(&PublicMessage::Curated("indice riga oltre u64"))
        })?)
        .ok_or_else(|| {
            PlenoraIoError::limite_redatto(&PublicMessage::Curated("overflow nell'indice riga"))
        })?;
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
            PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                "geometria oltre il conteggio supportato",
            ))
        })?)
        .ok_or_else(|| {
            PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                "overflow nel conteggio dei componenti geometrici",
            ))
        })?;
    Ok(())
}

/// L'errore di righe rifiutate quando il report non e' emettibile.
///
/// Conserva **categoria, fase e causa** del rifiuto reale: quelle non dipendono
/// da `input_total`, che governa solo il report. La causa passa nel messaggio
/// perche' senza report non avrebbe dove stare, ed e' un vocabolario chiuso
/// (`contract.nullability`, `conversion.invalid_geometry`, ...) — non un valore
/// derivato dal payload, che non potrebbe uscire.
///
/// Il totale **resta assente**. Il report non viene allegato invece di essere
/// allegato con un totale inventato: un report che dichiara un totale che
/// nessuno ha dichiarato e' peggio di un report che non c'e'.
fn errore_di_rifiuto_senza_report(
    driver: &'static str,
    causa: Option<&'static str>,
    capability_reason: Option<CapabilityReason>,
) -> PlenoraIoError {
    // Il driver esce dal campo `driver`, non dal testo: ripeterlo nel
    // messaggio non aggiungeva niente e faceva sembrare interpolato un valore
    // che era gia' strutturato. La causa resta, ed e' un vocabolario chiuso di
    // `&'static str`.
    let messaggio = causa.map_or(
        PublicMessage::Curated("righe rifiutate prima della scrittura"),
        |causa| PublicMessage::CuratedPair("righe rifiutate prima della scrittura:", causa),
    );
    let mut error = PlenoraIoError::redatto(
        IoErrorCode::Generic,
        ErrorCategory::DataMapping,
        ErrorPhase::Write,
        RemoteEffect::None,
        RetryDisposition::Never,
        &messaggio,
    );
    error.driver = Some(driver.to_owned());
    error.capability_reason = capability_reason;
    error
}

fn write_rejection_error(
    driver: &'static str,
    _batch_rows: u64,
    row_offset: u64,
    violations: &BTreeMap<u64, WriteRowViolation>,
    input_total: Option<u64>,
) -> PlenoraIoError {
    const EXAMPLES_LIMIT: u64 = 64;
    let prima = violations.values().next();
    let first_reason = prima.map(|violation| violation.capability_reason);
    let Some(input_total) = input_total.filter(|total| *total > 0) else {
        // Senza `input_total` il **report** non e' emettibile — il contratto
        // `plenora-io-row-diagnostics-v1` lo pretende positivo, e inventarlo
        // sarebbe peggio che ometterlo. L'**errore** pero' esiste comunque, ed
        // e' lo stesso: righe rifiutate prima della scrittura.
        //
        // Prima si restituiva un `Contract` sull'`input_total` mancante, cioe'
        // si sostituiva la causa primaria con una condizione dell'infrastruttura
        // diagnostica. Chi leggeva l'errore vedeva un problema interno al posto
        // del proprio: la riga era invalida, e il messaggio parlava d'altro.
        return errore_di_rifiuto_senza_report(
            driver,
            prima.map(|violation| violation.cause),
            first_reason,
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
    let mut error = PlenoraIoError::redatto(
        IoErrorCode::Generic,
        ErrorCategory::DataMapping,
        ErrorPhase::Write,
        RemoteEffect::None,
        RetryDisposition::Never,
        &PublicMessage::Curated("righe rifiutate prima della scrittura"),
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
        return PlenoraIoError::redatto(
            IoErrorCode::Generic,
            ErrorCategory::Internal,
            ErrorPhase::Write,
            RemoteEffect::None,
            RetryDisposition::Never,
            &PublicMessage::Curated("rifiuto row-scoped richiesto senza righe attribuibili"),
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
    /// Valutazione specifica della scrittura conclusa (`PRODUCT.md § LossReport`).
    pub fidelity: FidelityAssessment,
    /// Esito di durabilità del publish (`ENGINEERING.md § Pipeline di scrittura`).
    pub outcome: crate::publish::PublishOutcome,
}

#[cfg(test)]
mod tests {

    /// Descrittore minimo per i test del preflight.
    ///
    /// Questi test verificano l'enumerazione della sorgente, non lo schema
    /// delle opzioni: schema vuoto e mappa vuota li lasciano invariati, e la
    /// validazione che `preflight_source` ora esegue non entra in mezzo.
    const DESCRITTORE_DI_PROVA: crate::descriptor::FormatDescriptor =
        crate::descriptor::FormatDescriptor::const_new(
            "prova",
            crate::descriptor::Direction::Read,
            crate::descriptor::ReadMode::StreamingSequential,
            // I tre assi di INV-7: il descrittore di prova dichiara la
            // combinazione che tutti i driver reali dichiarano.
            crate::descriptor::NativeReadMode::StreamingSequential,
            crate::descriptor::DeliverySemantics::OperationAtomic,
            crate::descriptor::BufferingStrategy::AdaptiveMemoryThenDisk,
            crate::descriptor::DeterminismLevel::Semantic,
            None,
            None,
            false,
            false,
            crate::descriptor::ReaderConcurrency::SingleActiveReader,
            crate::descriptor::ProjectionSupport::None,
            crate::descriptor::PredicatePruningSupport::None,
            crate::descriptor::SpatialPruningSupport::None,
            crate::descriptor::CrsHandling::None,
            crate::descriptor::Fidelity::Lossless,
            crate::descriptor::Runtime::PureRust,
            // `hostile_input_hardened`: un descrittore di prova non parla di
            // input ostile: dichiara il valore che non afferma niente.
            false,
            // `spec_version_supported`: un descrittore di prova non parla di
            // nessun formato reale, quindi non ne dichiara la versione.
            None,
            None,
            plenora_io_model::format_options::SchemaOpzioniFormato::VUOTO,
            1,
            1,
            1,
        );
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

    fn scan_dir_with(
        entries: usize,
        limits: plenora_io_model::budget::PipelineLimits,
    ) -> Result<PathBuf> {
        let root = tempfile::tempdir().expect("tempdir");
        for index in 0..entries {
            let mut file = std::fs::File::create(root.path().join(format!("entry-{index}.bin")))
                .expect("file");
            file.write_all(b"x").expect("write");
        }
        let mut opts = opzioni_pipeline(limits);
        preflight_source(
            &DESCRITTORE_DI_PROVA,
            Source::Path(root.path().to_path_buf()),
            &mut opts,
        )
    }

    /// L0.9: senza tetto sulle entry una directory ostile fa crescere la coda
    /// dello scan senza limite, perche' i byte si sommano solo sui file.
    #[test]
    fn directory_scan_over_max_input_entries_rejects_with_typed_error() {
        let limits = plenora_io_model::budget::PipelineLimits::default().with_max_input_entries(4);
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
        let limits = plenora_io_model::budget::PipelineLimits::default().with_max_input_entries(4);
        assert!(
            scan_dir_with(3, limits).is_ok(),
            "radice + 3 file = 4 entry"
        );
    }

    #[test]
    fn max_input_entries_default_admits_a_realistic_directory() {
        // Il default non deve rifiutare una directory di file legittimi:
        // un tetto troppo stretto sarebbe un fail-closed inutile.
        assert!(scan_dir_with(64, plenora_io_model::budget::PipelineLimits::default()).is_ok());
    }

    #[test]
    fn entry_cap_is_checked_before_the_byte_sum() {
        // Con un tetto di entry raggiunto e un limite di byte larghissimo,
        // deve vincere il tetto delle entry: e' l'ordine dichiarato da INV-9.
        let limits = plenora_io_model::budget::PipelineLimits::default()
            .with_max_input_entries(2)
            .with_max_input_bytes(u64::MAX);
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
            Err(PlenoraIoError::formato_redatto(
                "arrow",
                &PublicMessage::Curated("payload troncato"),
            ))
        });

        // Stesso panico del primo: il messaggio pubblico deve coincidere.
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
            formattato.to_string().contains(MESSAGGIO_PANICO_ARROW),
            "il messaggio pubblico e' quello statico curato: {formattato}"
        );

        let letterale = letterale.expect_err("il panico deve diventare errore");
        assert!(
            !letterale.to_string().contains("Type NONE"),
            "nemmeno il payload letterale deve comparire: {letterale}"
        );

        // Il messaggio non distingue piu' un panico dall'altro, ed e' voluto:
        // FZ-0 ha tolto l'impronta perche' era un valore derivato dall'input
        // che finiva in un errore serializzato. Cio' che distingue le
        // occorrenze sono i log del processo, dove l'hook di panico scrive il
        // testo completo.
        let ripetuto = ripetuto.expect_err("il panico deve diventare errore");
        assert_eq!(formattato.to_string(), ripetuto.to_string());
        assert_eq!(
            formattato.to_string(),
            letterale.to_string(),
            "il messaggio e' statico, quindi identico per qualunque panico"
        );

        assert_eq!(riuscito.expect("il percorso normale non e' toccato"), 7);
        assert!(fallito
            .expect_err("l'errore ordinario resta tale")
            .to_string()
            .contains("payload troncato"));
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
        let opts = WriteOptions::from_write_parts(
            match plenora_io_model::budget::PipelineBudget::builder()
                // Il modello rifiuta le quote nulle, quindi il tetto e' uno
                // e il batch ne porta due: il rifiuto scatta comunque alla
                // prima scrittura, che e' cio' che il test verifica.
                .limits(limiti_di_prova().with_max_rows(1))
                .build()
            {
                Ok(bundle) => bundle.into_write_parts(),
                Err(error) => unreachable!("limiti di test: {error:?}"),
            },
        );
        let mut writer = with_write_limits(
            Box::new(FinishTrackingWriter {
                finished: finished.clone(),
            }),
            &opts,
        );
        let schema = Arc::new(Schema::new(vec![Field::new(
            "value",
            DataType::Int64,
            false,
        )]));
        let batch =
            RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from(vec![1, 2]))]).unwrap();

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
            &opzioni_scrittura(),
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
            &opzioni_scrittura(),
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
            limits: opzioni_scrittura().write_limits(),
            rows: 0,
            layer_rows: vec![0, 0],
            input_totals: vec![None, None],
            failed: false,
            contracts: Vec::new(),
            geometry_validation: None,
            fidelity: FidelityAssessment::lossless(),
            planned_loss: LossReport::default(),
            cancellation: CancellationToken::new(),
            budget: opzioni_scrittura().budget().clone(),
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
        let mut opts = opzioni_pipeline(
            plenora_io_model::budget::PipelineLimits::default().with_max_input_bytes(7),
        );
        let result = preflight_source(
            &DESCRITTORE_DI_PROVA,
            Source::Path(file.path().to_owned()),
            &mut opts,
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
        let mut opts = match plenora_io_model::budget::PipelineBudget::builder()
            .cancellation(token)
            .build()
        {
            Ok(bundle) => ReadOptions::from_read_parts(bundle.into_read_parts()),
            Err(error) => unreachable!("bundle di test: {error:?}"),
        };
        let result = preflight_source(
            &DESCRITTORE_DI_PROVA,
            Source::Path(std::path::PathBuf::from("not-observed")),
            &mut opts,
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
            limits: opzioni_scrittura().write_limits(),
            rows: 0,
            layer_rows: vec![0],
            input_totals: vec![None],
            failed: false,
            contracts: Vec::new(),
            geometry_validation: None,
            fidelity: FidelityAssessment::lossless(),
            planned_loss: LossReport::default(),
            cancellation: token.clone(),
            budget: opzioni_scrittura().budget().clone(),
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
                return Err(PlenoraIoError::contratto_redatto(&PublicMessage::Curated(
                    "errore terminale",
                )));
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
                Err(PlenoraIoError::contratto_redatto(&PublicMessage::Curated(
                    "costruzione fallita",
                )))
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
        assert_eq!(loss.esempi_trattenuti(), 1);
        let esempio = loss.esempi_canonici().next().expect("un esempio");
        // `crs_id` non c'e' piu': e' un identificatore che viene dal file. I
        // tre SRID restano, perche' sono codici di autorita' e sono **la cosa**
        // che l'esempio deve dire.
        assert!(!esempio.context.contains("EPSG:4326"));
        assert!(esempio.context.contains("srid=3003"));
        assert!(esempio.context.contains("definition_epsg="));
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
            Posizione {
                layer_index: Some(0),
                field_index: Some(0),
                type_class: None,
            },
            RappresentazioneDelCrs::CrsId,
            Some(9),
            CrsRepresentationState::Derived,
        );
        record_crs_representation_loss(
            &mut loss,
            Posizione {
                layer_index: Some(0),
                field_index: Some(0),
                type_class: None,
            },
            RappresentazioneDelCrs::Srid,
            Some(4),
            CrsRepresentationState::Absent,
        );
        record_crs_representation_loss(
            &mut loss,
            Posizione {
                layer_index: Some(0),
                field_index: Some(0),
                type_class: None,
            },
            RappresentazioneDelCrs::CrsDefinition,
            Some(42),
            CrsRepresentationState::Preserved,
        );

        assert_eq!(loss.counts.get("crs_id_not_preserved_derived"), Some(&1));
        assert_eq!(loss.counts.get("srid_not_preserved_absent"), Some(&1));
        assert!(!loss
            .counts
            .contains_key("crs_definition_not_preserved_absent"));
        assert!(loss
            .esempi_canonici()
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
            PipelineLimits::default().wkb_limits(),
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
            PipelineLimits::default().wkb_limits(),
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
            PipelineLimits::default().wkb_limits(),
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
            PipelineLimits::default().wkb_limits(),
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

    /// Senza `input_total` manca il **report**, non la causa.
    ///
    /// Questo test asseriva l'opposto: che l'errore diventasse un `Contract`
    /// sull'`input_total` mancante, con categoria `InvalidPlan` e fase
    /// `Validate`. Fissava un difetto come requisito — la causa primaria, cioe'
    /// la riga rifiutata, veniva sostituita da una condizione
    /// dell'infrastruttura diagnostica, e chi leggeva l'errore vedeva un
    /// problema interno al posto del proprio.
    ///
    /// Il report resta assente, perche' `plenora-io-row-diagnostics-v1`
    /// pretende `input_total` positivo e inventarlo sarebbe peggio che
    /// ometterlo. Tutto il resto sopravvive: categoria, fase, causa, e la
    /// ragione di capability.
    #[test]
    fn row_scoped_write_rejection_without_input_total_keeps_the_primary_cause() {
        let error = write_row_rejection("test", 0, 1, &[(0, "test.rejected", "value")], None);

        assert_eq!(error.category, ErrorCategory::DataMapping);
        assert_eq!(error.phase, ErrorPhase::Write);
        assert_eq!(error.driver.as_deref(), Some("test"));
        assert!(
            error.message.contains("test.rejected"),
            "senza report la causa deve stare nel messaggio: {error}"
        );
        assert!(
            !error.message.contains("input_total"),
            "l'assenza del totale non e' la causa del rifiuto: {error}"
        );
        // Il totale non viene inventato.
        assert!(error.row_diagnostics.is_none());

        // Controprova: con il totale il report c'e', e porta la stessa causa.
        let con_totale =
            write_row_rejection("test", 0, 1, &[(0, "test.rejected", "value")], Some(1));
        let report = con_totale.row_diagnostics.as_deref().unwrap();
        assert_eq!(report.input_total, Some(1));
        assert_eq!(report.counts["test.rejected"], 1);
        assert_eq!(con_totale.category, ErrorCategory::DataMapping);
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
            PlenoraIoError::formato_redatto("test", &PublicMessage::Curated("bad row")),
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
            PlenoraIoError::formato_redatto("test", &PublicMessage::Curated("bad row")),
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

    /// Un descrittore che dichiara perdita su tutt'e tre gli assi, cosi' che i
    /// tre rami redatti di `assess_write_contract` si accendano tutti.
    fn descrittore_che_dichiara_perdite() -> crate::descriptor::FormatDescriptor {
        crate::descriptor::FormatDescriptor::const_new(
            "prova-con-perdite",
            crate::descriptor::Direction::Bidirectional,
            crate::descriptor::ReadMode::StreamingSequential,
            crate::descriptor::NativeReadMode::StreamingSequential,
            crate::descriptor::DeliverySemantics::OperationAtomic,
            crate::descriptor::BufferingStrategy::AdaptiveMemoryThenDisk,
            crate::descriptor::DeterminismLevel::Semantic,
            Some(crate::descriptor::WriteMode::Streaming),
            Some(crate::descriptor::DeterminismLevel::Semantic),
            false,
            false,
            crate::descriptor::ReaderConcurrency::SingleActiveReader,
            crate::descriptor::ProjectionSupport::None,
            crate::descriptor::PredicatePruningSupport::None,
            crate::descriptor::SpatialPruningSupport::None,
            crate::descriptor::CrsHandling::Embedded,
            crate::descriptor::Fidelity::Conditional,
            crate::descriptor::Runtime::PureRust,
            false,
            None,
            Some(crate::descriptor::FormatWriteCapabilities {
                field_names: crate::descriptor::DBF_FIELD_NAMES,
                // Nessun tipo ammesso: cosi' il ramo della coercizione si
                // accende su ogni attributo invece che su alcuni.
                allowed_types: &[],
                type_coercion: crate::descriptor::TypeCoercionPolicy::LossReported,
                attributes: crate::descriptor::AttributeWriteSupport::LossReported,
                geometry: WKB_XY_GEOMETRY,
                crs: crate::descriptor::CrsWriteSupport::Embedded,
                crs_representations: crate::descriptor::CrsRepresentationCapabilities::new(
                    crate::descriptor::CrsRepresentationState::Preserved,
                    crate::descriptor::CrsRepresentationState::Preserved,
                    crate::descriptor::CrsRepresentationState::Preserved,
                ),
                nullability: crate::descriptor::NullabilitySupport::FormatDefined,
                multi_layer: true,
            }),
            plenora_io_model::format_options::SchemaOpzioniFormato::VUOTO,
            1,
            1,
            1,
        )
    }

    /// Due layer, due attributi ciascuno, con nomi che nessun testo curato
    /// potrebbe contenere per caso.
    fn piano_con_nomi_canary() -> WritePlan {
        let campi = || {
            vec![
                Field::new("CANARY_CAMPO_àèì'\"uno", DataType::Int64, true),
                Field::new("CANARY_CAMPO_àèì'\"due", DataType::Utf8, true),
            ]
        };
        WritePlan {
            layers: vec![
                crate::request::WriteLayer {
                    name: "CANARY_LAYER_àèì'\"alfa".to_owned(),
                    contract: plenora_io_model::contract::DataContract {
                        schema: Arc::new(Schema::new(campi())),
                        geometry: None,
                    },
                },
                crate::request::WriteLayer {
                    name: "CANARY_LAYER_àèì'\"beta".to_owned(),
                    contract: plenora_io_model::contract::DataContract {
                        schema: Arc::new(Schema::new(campi())),
                        geometry: None,
                    },
                },
            ],
        }
    }

    /// I nomi presi dal file restano nel v1 e spariscono dal v2.
    ///
    /// I nomi della fixture sono canary con accenti e apostrofo -- l'apostrofo
    /// perche' e' il carattere che le frasi congelate mettono fra virgolette --
    /// e la sonda pretende che **nessuno** compaia in cio' che il v2 trattiene,
    /// nemmeno dentro una stringa piu' lunga.
    #[test]
    fn i_nomi_del_file_restano_nel_v1_e_spariscono_dal_v2() {
        let valutazione = assess_write_contract(
            &descrittore_che_dichiara_perdite(),
            &piano_con_nomi_canary(),
        );

        // Il v1 li porta, alla lettera e non ricostruiti: sono i dettagli
        // dinamici, non un esempio statico.
        let v1: Vec<&str> = valutazione
            .ragioni_v1()
            .iter()
            .map(crate::loss::FidelityReason::detail_v1)
            .collect();
        assert!(!v1.is_empty(), "la fixture deve produrre ragioni");
        assert!(
            v1.iter().any(|d| d.contains("CANARY_LAYER_àèì'\"alfa")),
            "il v1 deve conservare il nome del layer: {v1:?}"
        );
        assert!(
            v1.iter().any(|d| d.contains("CANARY_CAMPO_àèì'\"due")),
            "il v1 deve conservare il nome dell'attributo: {v1:?}"
        );

        // Il v2 non li porta, e porta invece gli indici.
        //
        // La posizione si pretende sui **quattro codici redatti**, non su ogni
        // ragione: `for_format` ne aggiunge una di livello formato, che una
        // posizione non ce l'ha e non deve averla -- non parla di un layer ne'
        // di un campo. Pretenderla anche li' verificherebbe una cosa falsa.
        let mut con_indice_di_campo = 0_usize;
        for ragione in valutazione.ragioni_canoniche() {
            assert!(
                !ragione.detail.contains("CANARY"),
                "un nome del file e' rimasto nel testo curato: {}",
                ragione.detail
            );
            let redatta = matches!(
                ragione.code,
                FidelityReasonCode::AttributeLoss
                    | FidelityReasonCode::TypeCoercion
                    | FidelityReasonCode::NullabilityChanged
                    | FidelityReasonCode::StructureChanged
            );
            if redatta {
                assert!(
                    ragione.posizione.layer_index.is_some(),
                    "una ragione redatta senza layer_index non dice dove: {ragione:?}"
                );
                if ragione.posizione.field_index.is_some() {
                    con_indice_di_campo += 1;
                }
            }
        }
        assert!(
            con_indice_di_campo >= 2,
            "gli indici di campo devono distinguere cio' che i nomi distinguevano"
        );

        // E i due layer restano distinti: se la redazione li appiattisse,
        // la deduplicazione canonica li fonderebbe in uno.
        let layer_visti: std::collections::BTreeSet<_> = valutazione
            .ragioni_canoniche()
            .filter_map(|r| r.posizione.layer_index)
            .collect();
        assert_eq!(
            layer_visti,
            [0, 1].into_iter().collect(),
            "i due layer devono restare distinti dopo la redazione"
        );
    }

    /// La fusione conserva l'identita' legacy, che `add_reason(code, detail)`
    /// avrebbe buttato via facendo cambiare byte alla sezione v1.
    #[test]
    fn la_fusione_non_perde_la_frase_congelata() {
        let letta = assess_write_contract(
            &descrittore_che_dichiara_perdite(),
            &piano_con_nomi_canary(),
        );
        let mut fusa = FidelityAssessment::con_livello(crate::descriptor::Fidelity::Approximating);
        fusa.merge(&letta);

        let prima: Vec<&str> = letta
            .ragioni_v1()
            .iter()
            .map(crate::loss::FidelityReason::detail_v1)
            .collect();
        let dopo: Vec<&str> = fusa
            .ragioni_v1()
            .iter()
            .map(crate::loss::FidelityReason::detail_v1)
            .collect();
        assert_eq!(prima, dopo, "la fusione deve conservare le frasi del v1");

        let posizioni_prima: Vec<_> = letta.ragioni_canoniche().map(|r| r.posizione).collect();
        let posizioni_dopo: Vec<_> = fusa.ragioni_canoniche().map(|r| r.posizione).collect();
        assert_eq!(
            posizioni_prima, posizioni_dopo,
            "la fusione deve conservare le posizioni del v2"
        );
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

    // ---- Le opzioni sul modello unificato (Lotto 0, S4.e) ----
    //
    // Fino a S4.d qui vivevano i test del ponte transitorio: parita' fra i
    // due rami, guardie direzionali, "il ramo pipeline non consulta Limits".
    // Con un solo modello non descrivono piu' nulla — non c'e' un secondo
    // ramo con cui confrontarsi — e sono stati rimossi invece di essere
    // riscritti in forme che passano per costruzione.
    //
    // Restano i test che dicono ancora qualcosa: che gli scalari arrivano
    // dai limiti della pipeline, e che il permit e' one-shot.

    const QUOTA_INPUT_BYTES: u64 = 4_242;
    const QUOTA_INPUT_ENTRIES: u64 = 37;
    const QUOTA_ROWS: usize = 911;
    const QUOTA_COLUMNS: usize = 17;
    const QUOTA_VERTICES: usize = 5_000;
    const QUOTA_WKB_CELL: usize = 8_192;
    const QUOTA_WKB_COMPONENTS: usize = 640;
    const QUOTA_WKB_DEPTH: usize = 9;
    const QUOTA_OUTPUT_BYTES: u64 = 77_000;

    fn limiti_di_prova() -> PipelineLimits {
        PipelineLimits::default()
            .with_max_input_bytes(QUOTA_INPUT_BYTES)
            .with_max_input_entries(QUOTA_INPUT_ENTRIES)
            .with_max_rows(QUOTA_ROWS as u64)
            .with_max_columns(QUOTA_COLUMNS as u64)
            .with_max_vertices(QUOTA_VERTICES)
            .with_max_output_bytes(QUOTA_OUTPUT_BYTES)
            .with_max_wkb_cell_bytes(QUOTA_WKB_CELL)
            .with_max_wkb_components(QUOTA_WKB_COMPONENTS)
            .with_max_wkb_depth(QUOTA_WKB_DEPTH)
    }

    fn bundle_di_prova() -> plenora_io_model::budget::PipelineBundle {
        match plenora_io_model::budget::PipelineBudget::builder()
            .limits(limiti_di_prova())
            .build()
        {
            Ok(bundle) => bundle,
            Err(error) => unreachable!("limiti di prova non validi: {error:?}"),
        }
    }

    fn opzioni_lettura() -> ReadOptions {
        ReadOptions::from_read_parts(bundle_di_prova().into_read_parts())
    }

    fn opzioni_scrittura() -> WriteOptions {
        WriteOptions::from_write_parts(bundle_di_prova().into_write_parts())
    }

    fn opzioni_pipeline(limits: PipelineLimits) -> ReadOptions {
        match plenora_io_model::budget::PipelineBudget::builder()
            .limits(limits)
            .build()
        {
            Ok(bundle) => ReadOptions::from_read_parts(bundle.into_read_parts()),
            Err(error) => unreachable!("limiti di test non validi: {error:?}"),
        }
    }

    #[test]
    fn gli_scalari_arrivano_dai_limiti_della_pipeline() {
        let opts = opzioni_lettura();

        assert_eq!(opts.max_input_bytes(), QUOTA_INPUT_BYTES);
        assert_eq!(opts.max_input_entries(), QUOTA_INPUT_ENTRIES);
        assert_eq!(opts.max_rows(), QUOTA_ROWS);
        assert_eq!(opts.max_columns(), QUOTA_COLUMNS);
        assert_eq!(opts.max_vertices(), QUOTA_VERTICES);
        assert_eq!(opts.wkb_limits().max_cell_bytes, QUOTA_WKB_CELL);
        // Composto con `max_vertices`, come faceva `Limits::effective_wkb()`.
        assert_eq!(opts.wkb_limits().max_components, QUOTA_WKB_COMPONENTS);
        assert_eq!(opts.wkb_limits().max_depth, QUOTA_WKB_DEPTH);
    }

    #[test]
    fn la_vista_di_scrittura_arriva_dagli_stessi_limiti() {
        let wopts = opzioni_scrittura();
        let vista = wopts.write_limits();

        assert_eq!(vista.max_columns, QUOTA_COLUMNS);
        assert_eq!(vista.max_rows, QUOTA_ROWS);
        assert_eq!(vista.wkb.max_cell_bytes, QUOTA_WKB_CELL);
        assert_eq!(vista.wkb.max_components, QUOTA_WKB_COMPONENTS);
        assert_eq!(vista.wkb.max_depth, QUOTA_WKB_DEPTH);
        // Senza input osservato non si applica alcuna espansione: il tetto e'
        // quello assoluto.
        assert_eq!(wopts.max_output_bytes(), QUOTA_OUTPUT_BYTES);
    }

    #[test]
    fn ensure_active_osserva_la_cancellazione_del_context() {
        let token = CancellationToken::new();
        let bundle = match plenora_io_model::budget::PipelineBudget::builder()
            .limits(limiti_di_prova())
            .cancellation(token.clone())
            .build()
        {
            Ok(bundle) => bundle,
            Err(error) => unreachable!("bundle di prova: {error:?}"),
        };
        let opts = ReadOptions::from_read_parts(bundle.into_read_parts());

        assert!(opts.ensure_active().is_ok());
        token.cancel();
        assert!(opts.ensure_active().is_err());
    }

    #[test]
    fn permit_snapshot_e_budget_attraversano_i_costruttori_senza_rigenerazione() {
        let bundle = bundle_di_prova();
        let contesto = bundle.context().clone();
        let mut opts = ReadOptions::from_read_parts(bundle.into_read_parts());

        // Il budget e' lo stesso, non uno nuovo con gli stessi limiti:
        // `is_same_pipeline` confronta l'identita' del context, non i valori.
        assert!(opts.budget().context().is_same_pipeline(&contesto));

        // Il permit e' l'esemplare unico trasportato dalle parti: lo prova il
        // fatto che il context lo accetti. Un permit rigenerato porterebbe un
        // pipeline id che questo context rifiuta.
        let permit = opts
            .take_input_permit()
            .expect("le parti read trasportano il permit");
        assert!(contesto.observe_input(permit).is_ok());
        assert!(
            opts.take_input_permit().is_none(),
            "il permit e' spendibile una sola volta"
        );

        // La cancellazione e' quella del context, non un token nuovo.
        assert!(!opts.cancellation().is_cancelled());
        contesto.cancellation().cancel();
        assert!(opts.cancellation().is_cancelled());
    }

    #[test]
    fn lo_snapshot_atteso_sopravvive_alla_costruzione_dalle_parti_di_scan() {
        // Lo snapshot atteso viene da un'osservazione precedente: qui basta
        // quello di un footprint qualunque, perche' il test guarda il
        // trasporto e non il contenuto.
        let bundle = bundle_di_prova();
        let contesto = bundle.context().clone();
        let (_budget, permit, _atteso) = bundle.into_read_parts().into_components();
        let footprint = contesto
            .observe_input(permit.expect("permit"))
            .expect("osservazione");
        let parts = bundle_di_prova().into_scan_parts(footprint.snapshot());
        let atteso = *parts.expected_footprint();
        let opts = ReadOptions::from_read_parts(
            plenora_io_model::budget::IntoReadParts::into_read_budget_parts(parts),
        );

        assert_eq!(
            opts.expected_footprint().copied(),
            Some(atteso),
            "lo snapshot attraversa il costruttore invariato"
        );
    }

    #[test]
    fn le_opzioni_per_valore_rendono_estraibile_il_permit_una_volta_sola() {
        // Verifica la **forma**, non il comportamento del preflight: la
        // funzione locale sotto imita la firma che `preflight_source` usa,
        // non e' `preflight_source`. Con `open` che riceve le opzioni per
        // valore, una funzione che le prende `&mut` puo' estrarre il permit
        // per move; con `&ReadOptions` non si estrae nulla, e le vie per
        // aggirarlo — `Mutex<Option<InputPermit>>`, o un permit clonato —
        // reintrodurrebbero l'osservazione doppia che il permit esiste per
        // escludere.
        //
        // Il consumo vero e' esercitato dai test del preflight e da quello
        // end-to-end, non da qui.
        fn con_la_stessa_firma_del_preflight(opts: &mut ReadOptions) -> Option<InputPermit> {
            opts.take_input_permit()
        }

        let mut opts = opzioni_lettura();
        assert!(
            con_la_stessa_firma_del_preflight(&mut opts).is_some(),
            "il permit deve essere estraibile attraverso un prestito mutabile"
        );
        assert!(
            con_la_stessa_firma_del_preflight(&mut opts).is_none(),
            "one-shot: la seconda estrazione non deve dare un secondo permit"
        );

        // E le opzioni restano utilizzabili: dopo il preflight e' l'adapter a
        // leggerle, e consumare il permit non consuma le opzioni.
        assert_eq!(opts.max_columns(), QUOTA_COLUMNS);
        assert_eq!(opts.max_input_bytes(), QUOTA_INPUT_BYTES);
    }

    /// Due percorsi non-UTF-8 distinti non devono collassare sullo stesso
    /// digest.
    ///
    /// `to_string_lossy` sostituisce ogni sequenza non valida con U+FFFD:
    /// `b"\xff"` e `b"\xfe"` diventano **la stessa** stringa, e il footprint
    /// direbbe che due sorgenti diverse sono la stessa. E' il caso che la
    /// rappresentazione a byte esclude.
    ///
    /// Solo Unix: su Windows i nomi sono UTF-16 e la collisione non si pone
    /// nella stessa forma.
    #[cfg(unix)]
    #[test]
    fn percorsi_non_utf8_distinti_non_collidono_nel_digest() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let primo = std::path::PathBuf::from(OsStr::from_bytes(b"/tmp/\xff"));
        let secondo = std::path::PathBuf::from(OsStr::from_bytes(b"/tmp/\xfe"));

        assert_eq!(
            primo.to_string_lossy(),
            secondo.to_string_lossy(),
            "la premessa del test: la forma lossy li rende indistinguibili"
        );
        assert_ne!(
            byte_identita_percorso(&primo),
            byte_identita_percorso(&secondo),
            "la forma normalizzata deve restare iniettiva"
        );
    }

    #[test]
    fn la_forma_normalizzata_del_percorso_e_stabile() {
        let percorso = std::path::PathBuf::from("dati/a.csv");
        assert_eq!(
            byte_identita_percorso(&percorso),
            byte_identita_percorso(&percorso),
            "due corse sullo stesso percorso devono dare lo stesso digest"
        );
        assert_ne!(
            byte_identita_percorso(&percorso),
            byte_identita_percorso(&std::path::PathBuf::from("dati/b.csv"))
        );
    }

    #[test]
    fn il_preflight_applica_le_quote_e_pubblica_il_footprint() {
        let mut file = tempfile::NamedTempFile::new().expect("tempfile");
        file.write_all(&[0_u8; 8]).expect("write");

        let mut stretto = opzioni_pipeline(
            plenora_io_model::budget::PipelineLimits::default().with_max_input_bytes(7),
        );
        let errore = preflight_source(
            &DESCRITTORE_DI_PROVA,
            Source::Path(file.path().to_owned()),
            &mut stretto,
        )
        .expect_err("otto byte non stanno in sette");
        assert_eq!(errore.code, plenora_io_model::IoErrorCode::LimitExceeded);

        // Con quota capiente lo stesso file passa, e il footprint pubblicato
        // descrive cio' che e' stato davvero osservato: un file, otto byte.
        let mut largo = opzioni_pipeline(plenora_io_model::budget::PipelineLimits::default());
        let budget = largo.budget().clone();
        assert_eq!(
            budget.context().observed_input(),
            plenora_io_model::budget::ObservedInput::NotObserved,
            "prima del preflight nulla e' osservato"
        );
        assert!(preflight_source(
            &DESCRITTORE_DI_PROVA,
            Source::Path(file.path().to_owned()),
            &mut largo
        )
        .is_ok());
        assert_eq!(
            budget.context().observed_input(),
            plenora_io_model::budget::ObservedInput::Bytes(8)
        );
        assert_eq!(budget.context().entries_visited(), 1);
    }

    #[test]
    fn il_preflight_spende_il_permit_e_non_osserva_due_volte() {
        let file = tempfile::NamedTempFile::new().expect("tempfile");
        let mut opts = opzioni_pipeline(plenora_io_model::budget::PipelineLimits::default());

        assert!(preflight_source(
            &DESCRITTORE_DI_PROVA,
            Source::Path(file.path().to_owned()),
            &mut opts
        )
        .is_ok());
        // Il permit e' stato speso: una seconda osservazione non ha nulla con
        // cui pubblicare, e fallisce invece di lasciare il footprint vuoto.
        let errore = preflight_source(
            &DESCRITTORE_DI_PROVA,
            Source::Path(file.path().to_owned()),
            &mut opts,
        )
        .expect_err("il permit e' one-shot");
        assert_eq!(errore.code, plenora_io_model::IoErrorCode::LimitExceeded);
    }
}
