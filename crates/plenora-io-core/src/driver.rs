//! Il confine plug-in: `FormatDriver` + handle/reader/writer (`ENGINEERING.md § Interfaccia dei driver`).

use std::collections::BTreeMap;
use std::path::PathBuf;

use arrow_array::{BinaryArray, LargeBinaryArray, RecordBatch};
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
    ArrowTypeClass, AttributeWriteSupport, CrsDerivation, CrsRepresentationState, FormatDescriptor,
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

mod perdita_pianificata;
use perdita_pianificata::{assess_write_contract, planned_write_loss};

mod scrittura_limitata;
use scrittura_limitata::{GeometryValidation, LimitedWriter};

mod validazione_geometrie;
use validazione_geometrie::{validate_geometry_batch_at, write_rejection_error, WriteRowViolation};

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
mod tests;
