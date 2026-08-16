//! Modello budget unificato del Lotto 0 (`DECISION-PACKAGE-Lotto-0.md`,
//! INV-1..INV-6, INV-9, INV-11..INV-13).
//!
//! Il modulo introduce il modello nuovo **accanto** a [`crate::limits`] e
//! [`crate::resource`], che restano invariati: lo step M1/S1 non cambia il
//! comportamento del core. Nessun tipo di questo modulo governa ancora un
//! percorso di produzione; la migrazione dei driver e della CLI avviene in
//! M3/S4 e la rimozione del modello legacy in M4/S7.
//!
//! Struttura: un [`PipelineContext`] condiviso porta le grandezze
//! pipeline-wide (deadline, input osservato, memoria, spill, entry visitate,
//! cancellazione, pool opzionale); gli [`OperationBudget`] figli portano i
//! contatori cumulativi per operazione, indipendenti fra reader e writer
//! (INV-3).
//!
//! La costruzione ha una sola via (INV-2): [`PipelineBudget::builder`]
//! produce un [`PipelineBundle`] **opaco** che tiene insieme budget e
//! [`InputPermit`]; da li' si ottengono le parti opache che alimenteranno le
//! factory di `plenora-io-core`. Budget e permit non sono mai separabili dal
//! chiamante, quindi non e' possibile incrociare un permit con un budget
//! diverso da quello che lo ha emesso.
//!
//! ```
//! use plenora_io_model::budget::{ObservedInput, PipelineBudget, PipelineLimits};
//!
//! // convert: un solo bundle, due rami con contatori indipendenti.
//! let bundle = PipelineBudget::builder()
//!     .limits(PipelineLimits::default().with_max_rows(1_000))
//!     .build()?;
//! let (mut read, write) = bundle.into_convert_parts().into_parts();
//!
//! // il preflight del core consuma il permit e osserva l'input una volta.
//! let permit = read.take_input_permit().ok_or("permit assente")?;
//! read.budget().context().observe_input(permit, 4_096)?;
//!
//! // il writer vede l'input osservato dal reader: stesso context (INV-6).
//! assert_eq!(
//!     write.budget().context().observed_input(),
//!     ObservedInput::Bytes(4_096),
//! );
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! Il bundle e' opaco: budget e permit non si possono separare a mano.
//!
//! ```compile_fail
//! use plenora_io_model::budget::{PipelineBudget, PipelineBundle};
//! let bundle = PipelineBudget::builder().build().expect("costruito");
//! let PipelineBundle { budget, permit } = bundle;
//! ```
//!
//! Il permit non e' `Clone`: una seconda osservazione non e' scrivibile.
//!
//! ```compile_fail
//! use plenora_io_model::budget::{InputPermit, PipelineBudget};
//! let mut parts = PipelineBudget::builder().build().expect("costruito").into_read_parts();
//! let permit = parts.take_input_permit().expect("permit presente");
//! let secondo: InputPermit = permit.clone();
//! ```
//!
//! `SourceFootprint` non ha costruttori: l'unica fabbrica e'
//! `PipelineContext::observe_input`.
//!
//! ```compile_fail
//! use plenora_io_model::budget::SourceFootprint;
//! let footprint = SourceFootprint { total_bytes: 1, entries_visited: 1 };
//! ```
//!
//! I trait delle parti sono sealed: nessun tipo esterno puo' alimentare le
//! factory del core.
//!
//! ```compile_fail
//! use plenora_io_model::budget::{IntoReadParts, ReadBudgetParts};
//! struct PartiFabbricate;
//! impl IntoReadParts for PartiFabbricate {
//!     fn into_read_budget_parts(self) -> ReadBudgetParts {
//!         todo!()
//!     }
//! }
//! ```

use std::cell::Cell;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::cancellation::CancellationToken;
use crate::error::{ErrorPhase, PlenoraIoError, Result};

/// Identita' di pipeline, usata per legare un [`InputPermit`] al
/// [`PipelineContext`] che lo ha emesso. Un contatore monotono e non un
/// confronto di puntatori: l'indirizzo di un `Arc` liberato puo' essere
/// riusato, un id no.
static NEXT_PIPELINE_ID: AtomicU64 = AtomicU64::new(1);

// Messaggi curati: `&'static str`, mai derivati dal payload (INV-10). La
// tipizzazione in `LimitKind` arriva con S9, che sostituisce il testo libero
// con un enum; qui i messaggi restano costanti di compilazione.
const LIMIT_MUST_BE_POSITIVE: &str = "i limiti di pipeline devono essere maggiori di zero";
const CELL_BYTES_ABOVE_MEMORY: &str = "max_wkb_cell_bytes supera il budget di memoria";
const CELL_BYTES_NOT_REPRESENTABLE: &str = "max_wkb_cell_bytes non rappresentabile in u64";
const DEADLINE_BEYOND_INSTANT: &str = "deadline della pipeline oltre Instant";
const DURATION_EXHAUSTED: &str = "durata della pipeline esaurita";
const LEASE_MUST_BE_POSITIVE: &str = "una lease deve essere maggiore di zero";
const MEMORY_EXHAUSTED: &str = "budget di memoria esaurito";
const SPILL_EXHAUSTED: &str = "budget di spill esaurito";
const CONCURRENCY_EXHAUSTED: &str = "operazioni concorrenti oltre la quota del pool";
const COUNTER_EXHAUSTED: &str = "contatore cumulativo dell'operazione esaurito";
const OUTPUT_LIMIT_EXCEEDED: &str = "output oltre il tetto derivato dall'input osservato";
const COMMIT_NOT_VALID: &str = "consumo non valido per la lease";
const TOO_MANY_ENTRIES: &str = "numero di entry di input oltre il limite";
const ENTRIES_OVERFLOW: &str = "overflow nel conteggio delle entry di input";
const PERMIT_FOREIGN: &str = "permit non emesso da questa pipeline";
const INPUT_ALREADY_OBSERVED: &str = "input gia' osservato per questa pipeline";

fn limit_error(message: &'static str) -> PlenoraIoError {
    PlenoraIoError::LimitExceeded(message.to_owned())
}

// FNV-1a a 64 bit, due volte con basi distinte: il digest e' a 128 bit e non
// richiede una dipendenza nuova (i pin del workspace sono esatti e ogni
// aggiunta passa da un gate). Non e' una funzione crittografica e non deve
// esserlo: serve a rilevare mutazioni ordinarie della sorgente fra due
// scansioni, non a resistere a un avversario che costruisce collisioni —
// chi puo' riscrivere i file puo' comunque cambiarne il contenuto a
// dimensione e mtime invariati, come dichiara la garanzia best-effort.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
const FNV_BASIS_HIGH: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_BASIS_LOW: u64 = 0x9e37_79b9_7f4a_7c15;

fn fnv1a(basis: u64, bytes: &[u8]) -> u64 {
    let mut hash = basis;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Identita' di una entry osservata dal preflight.
///
/// Il path arriva **gia' normalizzato** dal core: la normalizzazione dipende
/// dal filesystem e dalla piattaforma, che il modello non conosce. Il
/// modello si limita a renderla parte del digest.
#[derive(Clone, Copy, Debug)]
pub struct SourceEntry<'a> {
    normalized_path: &'a [u8],
    size_bytes: u64,
    modified: Option<SystemTime>,
}

impl<'a> SourceEntry<'a> {
    #[must_use]
    pub const fn new(
        normalized_path: &'a [u8],
        size_bytes: u64,
        modified: Option<SystemTime>,
    ) -> Self {
        Self {
            normalized_path,
            size_bytes,
            modified,
        }
    }

    #[must_use]
    pub const fn normalized_path(&self) -> &'a [u8] {
        self.normalized_path
    }

    #[must_use]
    pub const fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    #[must_use]
    pub const fn modified(&self) -> Option<SystemTime> {
        self.modified
    }

    /// Codifica canonica dell'entry: lunghezza del path, path, dimensione e
    /// mtime con segno esplicito. La lunghezza in testa impedisce che due
    /// insiemi di path diversi producano la stessa sequenza di byte.
    fn canonical_bytes(&self) -> Vec<u8> {
        let mut encoded = Vec::with_capacity(self.normalized_path.len() + 33);
        encoded.extend_from_slice(&(self.normalized_path.len() as u64).to_le_bytes());
        encoded.extend_from_slice(self.normalized_path);
        encoded.extend_from_slice(&self.size_bytes.to_le_bytes());
        match self.modified {
            None => encoded.push(0),
            Some(instant) => match instant.duration_since(UNIX_EPOCH) {
                Ok(since_epoch) => {
                    encoded.push(1);
                    encoded.extend_from_slice(&since_epoch.as_nanos().to_le_bytes());
                }
                Err(before_epoch) => {
                    encoded.push(2);
                    encoded.extend_from_slice(&before_epoch.duration().as_nanos().to_le_bytes());
                }
            },
        }
        encoded
    }
}

/// Accumulatore del digest: XOR dei valori per-entry.
///
/// Lo XOR e' **insensibile all'ordine**, e deve esserlo: l'ordine di
/// enumerazione di una directory non e' stabile fra due scansioni ne' fra
/// due filesystem, quindi un digest sensibile all'ordine segnalerebbe una
/// mutazione che non c'e'. I path sono unici dentro una sorgente, quindi non
/// esiste la coppia identica che lo XOR annullerebbe.
#[derive(Debug)]
struct DigestAccumulator {
    high: AtomicU64,
    low: AtomicU64,
}

impl DigestAccumulator {
    const fn new() -> Self {
        Self {
            high: AtomicU64::new(0),
            low: AtomicU64::new(0),
        }
    }

    fn absorb(&self, entry: &SourceEntry<'_>) {
        let encoded = entry.canonical_bytes();
        self.high
            .fetch_xor(fnv1a(FNV_BASIS_HIGH, &encoded), Ordering::AcqRel);
        self.low
            .fetch_xor(fnv1a(FNV_BASIS_LOW, &encoded), Ordering::AcqRel);
    }

    fn finish(&self) -> SourceDigest {
        let mut bytes = [0_u8; 16];
        bytes[..8].copy_from_slice(&self.high.load(Ordering::Acquire).to_le_bytes());
        bytes[8..].copy_from_slice(&self.low.load(Ordering::Acquire).to_le_bytes());
        SourceDigest(bytes)
    }
}

/// Digest opaco a 128 bit sull'insieme delle entry osservate: path
/// normalizzati, dimensione e mtime.
///
/// Copre quindi anche aggiunte e rimozioni, non solo le mutazioni in place.
/// La garanzia e' **best-effort per costruzione**: una mutazione che
/// preservi path, dimensione e mtime non e' rilevabile, e il Lotto 0 non
/// ratifica alcuna variante forte.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SourceDigest([u8; 16]);

impl SourceDigest {
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

/// Gauge lease-based: quota residua su una capacita' fissa, restituita al
/// drop della lease. Non e' cumulativo — a differenza dei contatori di
/// [`OperationBudget`] la quota torna disponibile.
#[derive(Debug)]
struct Gauge {
    capacity: u64,
    remaining: AtomicU64,
}

impl Gauge {
    const fn new(capacity: u64) -> Self {
        Self {
            capacity,
            remaining: AtomicU64::new(capacity),
        }
    }

    fn try_take(&self, amount: u64) -> bool {
        let mut current = self.remaining.load(Ordering::Acquire);
        loop {
            let Some(next) = current.checked_sub(amount) else {
                return false;
            };
            match self.remaining.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(observed) => current = observed,
            }
        }
    }

    fn give_back(&self, amount: u64) {
        let mut current = self.remaining.load(Ordering::Acquire);
        loop {
            let next = current.saturating_add(amount).min(self.capacity);
            match self.remaining.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return,
                Err(observed) => current = observed,
            }
        }
    }

    fn remaining(&self) -> u64 {
        self.remaining.load(Ordering::Acquire)
    }

    const fn capacity(&self) -> u64 {
        self.capacity
    }
}

/// Limiti immutabili della pipeline (INV-1): unificano `Limits` e le parti
/// cumulative di `ResourceLimits`.
///
/// Tutti i campi sono privati: si leggono dai getter e si impostano dai
/// setter fluent, cosi' che l'aggiunta di una quota non rompa i call site
/// con uno struct literal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PipelineLimits {
    max_input_bytes: u64,
    max_input_entries: u64,
    max_rows: u64,
    max_columns: u64,
    max_geometry_components: u64,
    max_output_bytes: u64,
    output_expansion_ratio: u64,
    max_wkb_cell_bytes: usize,
    max_wkb_components: usize,
    max_wkb_depth: usize,
    max_vertices: usize,
    memory_bytes: u64,
    spill_bytes: u64,
    duration_ms: u64,
    decompression_ratio: u64,
}

impl Default for PipelineLimits {
    /// I default nascono dai due modelli legacy.
    ///
    /// Dove divergevano — era il finding L0.2 — vince il valore **piu'
    /// stretto**, cosi' l'unificazione non allenta in silenzio una quota
    /// gia' applicata: `max_rows` `10_000_000` e `max_columns` `4_096` da
    /// `Limits`, `max_output_bytes` 1 GiB da `Limits`.
    ///
    /// `max_input_entries` e' nuovo (INV-9) e vale `10_000`: una directory
    /// di migliaia di file legittimi passa, uno scan illimitato no.
    fn default() -> Self {
        Self {
            max_input_bytes: 268_435_456,
            max_input_entries: 10_000,
            max_rows: 10_000_000,
            max_columns: 4_096,
            max_geometry_components: 16_777_216,
            max_output_bytes: 1_073_741_824,
            output_expansion_ratio: 1_000,
            max_wkb_cell_bytes: 64 * 1024 * 1024,
            max_wkb_components: 100_000,
            max_wkb_depth: 64,
            max_vertices: 50_000_000,
            memory_bytes: 512 * 1024 * 1024,
            spill_bytes: 4 * 1024 * 1024 * 1024,
            duration_ms: 30_000,
            decompression_ratio: 1_000,
        }
    }
}

macro_rules! limit_accessors {
    ($($field:ident: $type:ty, $setter:ident);* $(;)?) => {
        $(
            #[must_use]
            pub const fn $field(&self) -> $type {
                self.$field
            }

            #[must_use]
            pub const fn $setter(mut self, value: $type) -> Self {
                self.$field = value;
                self
            }
        )*
    };
}

impl PipelineLimits {
    limit_accessors! {
        max_input_bytes: u64, with_max_input_bytes;
        max_input_entries: u64, with_max_input_entries;
        max_rows: u64, with_max_rows;
        max_columns: u64, with_max_columns;
        max_geometry_components: u64, with_max_geometry_components;
        max_output_bytes: u64, with_max_output_bytes;
        output_expansion_ratio: u64, with_output_expansion_ratio;
        max_wkb_cell_bytes: usize, with_max_wkb_cell_bytes;
        max_wkb_components: usize, with_max_wkb_components;
        max_wkb_depth: usize, with_max_wkb_depth;
        max_vertices: usize, with_max_vertices;
        memory_bytes: u64, with_memory_bytes;
        spill_bytes: u64, with_spill_bytes;
        duration_ms: u64, with_duration_ms;
        decompression_ratio: u64, with_decompression_ratio;
    }

    /// Tetto effettivo dei componenti di **una singola geometria**: il
    /// minimo fra il limite per cella e il tetto globale dei vertici.
    ///
    /// E' la stessa composizione di `Limits::effective_wkb()`, preservata
    /// perche' `--max-vertices` e' un flag vivo della CLI: senza questo
    /// metodo la migrazione al modello unificato allenterebbe in silenzio un
    /// tetto che oggi un utente puo' stringere.
    #[must_use]
    pub fn effective_wkb_components(&self) -> usize {
        self.max_wkb_components.min(self.max_vertices)
    }

    /// Verifica gli invarianti prima di costruire una pipeline.
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::LimitExceeded`] se una quota e' nulla,
    /// se `max_wkb_cell_bytes` non e' rappresentabile in `u64` o se supera
    /// il budget di memoria.
    pub fn validate(&self) -> Result<()> {
        let positive_u64 = [
            self.max_input_bytes,
            self.max_input_entries,
            self.max_rows,
            self.max_columns,
            self.max_geometry_components,
            self.max_output_bytes,
            self.output_expansion_ratio,
            self.memory_bytes,
            self.spill_bytes,
            self.duration_ms,
            self.decompression_ratio,
        ];
        if positive_u64.contains(&0) {
            return Err(limit_error(LIMIT_MUST_BE_POSITIVE));
        }
        let positive_usize = [
            self.max_wkb_cell_bytes,
            self.max_wkb_components,
            self.max_wkb_depth,
            self.max_vertices,
        ];
        if positive_usize.contains(&0) {
            return Err(limit_error(LIMIT_MUST_BE_POSITIVE));
        }
        let cell_bytes = u64::try_from(self.max_wkb_cell_bytes)
            .map_err(|_| limit_error(CELL_BYTES_NOT_REPRESENTABLE))?;
        if cell_bytes > self.memory_bytes {
            return Err(limit_error(CELL_BYTES_ABOVE_MEMORY));
        }
        Ok(())
    }
}

/// Quote condivise fra piu' pipeline (INV-12).
///
/// Senza pool i gauge memory/spill di una pipeline sono locali e la
/// concorrenza **non esiste**; con pool memory/spill valgono il minimo fra
/// quota locale e quota del pool, e la concorrenza e' governata solo da qui.
#[derive(Clone, Debug)]
pub struct ResourcePool {
    inner: Arc<PoolInner>,
}

#[derive(Debug)]
struct PoolInner {
    memory: Gauge,
    spill: Gauge,
    concurrency: Gauge,
}

impl ResourcePool {
    #[must_use]
    pub fn builder() -> ResourcePoolBuilder {
        ResourcePoolBuilder::default()
    }

    #[must_use]
    pub fn remaining_memory(&self) -> u64 {
        self.inner.memory.remaining()
    }

    #[must_use]
    pub fn remaining_spill(&self) -> u64 {
        self.inner.spill.remaining()
    }

    #[must_use]
    pub fn remaining_concurrency(&self) -> u64 {
        self.inner.concurrency.remaining()
    }

    #[must_use]
    pub fn is_same_pool(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ResourcePoolBuilder {
    memory_bytes: u64,
    spill_bytes: u64,
    concurrent_operations: u64,
}

impl Default for ResourcePoolBuilder {
    fn default() -> Self {
        Self {
            memory_bytes: 512 * 1024 * 1024,
            spill_bytes: 4 * 1024 * 1024 * 1024,
            concurrent_operations: 64,
        }
    }
}

impl ResourcePoolBuilder {
    #[must_use]
    pub const fn memory_bytes(mut self, value: u64) -> Self {
        self.memory_bytes = value;
        self
    }

    #[must_use]
    pub const fn spill_bytes(mut self, value: u64) -> Self {
        self.spill_bytes = value;
        self
    }

    #[must_use]
    pub const fn concurrent_operations(mut self, value: u64) -> Self {
        self.concurrent_operations = value;
        self
    }

    /// Costruisce il pool condiviso.
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::LimitExceeded`] se una delle tre quote
    /// e' nulla: un pool a quota zero rifiuterebbe ogni lease invece di
    /// limitare.
    pub fn build(self) -> Result<ResourcePool> {
        if self.memory_bytes == 0 || self.spill_bytes == 0 || self.concurrent_operations == 0 {
            return Err(limit_error(LIMIT_MUST_BE_POSITIVE));
        }
        Ok(ResourcePool {
            inner: Arc::new(PoolInner {
                memory: Gauge::new(self.memory_bytes),
                spill: Gauge::new(self.spill_bytes),
                concurrency: Gauge::new(self.concurrent_operations),
            }),
        })
    }
}

/// Stato dell'input osservato (INV-6). `NotObserved` e `Bytes(0)` sono
/// stati distinti: il primo dice che nessun preflight ha girato, il secondo
/// che il preflight ha girato su un input vuoto.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ObservedInput {
    NotObserved,
    Bytes(u64),
}

#[derive(Debug)]
struct Observation {
    claimed: AtomicBool,
    published: AtomicBool,
    bytes: AtomicU64,
}

impl Observation {
    const fn new() -> Self {
        Self {
            claimed: AtomicBool::new(false),
            published: AtomicBool::new(false),
            bytes: AtomicU64::new(0),
        }
    }

    fn state(&self) -> ObservedInput {
        if self.published.load(Ordering::Acquire) {
            ObservedInput::Bytes(self.bytes.load(Ordering::Acquire))
        } else {
            ObservedInput::NotObserved
        }
    }
}

/// Permit opaco one-shot dell'osservazione dell'input (INV-13).
///
/// Non e' `Clone` e non ha costruttori pubblici: nasce dentro un
/// [`PipelineBundle`] e ne esce solo trasportato dalle parti. Porta
/// l'identita' del [`PipelineContext`] che lo ha emesso, quindi non e'
/// spendibile su un context diverso.
#[derive(Debug)]
#[non_exhaustive]
pub struct InputPermit {
    pipeline_id: u64,
}

/// Descrizione immutabile dell'input osservato dal preflight.
///
/// Unica fabbrica: [`PipelineContext::observe_input`]. Non esiste un
/// costruttore pubblico, quindi un consumer non puo' dichiarare un input
/// che non ha misurato.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SourceFootprint {
    total_bytes: u64,
    entries_visited: u64,
    digest: SourceDigest,
}

impl SourceFootprint {
    #[must_use]
    pub const fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    #[must_use]
    pub const fn entries_visited(&self) -> u64 {
        self.entries_visited
    }

    #[must_use]
    pub const fn digest(&self) -> SourceDigest {
        self.digest
    }

    /// Snapshot serializzabile, conservato dal consumer come valore
    /// **atteso** di una successiva scansione.
    #[must_use]
    pub const fn snapshot(&self) -> SourceFootprintSnapshot {
        SourceFootprintSnapshot {
            total_bytes: self.total_bytes,
            entries_visited: self.entries_visited,
            digest: self.digest,
        }
    }
}

/// Snapshot serializzabile del footprint. Puo' viaggiare fuori dal processo.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct SourceFootprintSnapshot {
    total_bytes: u64,
    entries_visited: u64,
    digest: SourceDigest,
}

impl SourceFootprintSnapshot {
    #[must_use]
    pub const fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    #[must_use]
    pub const fn entries_visited(&self) -> u64 {
        self.entries_visited
    }

    #[must_use]
    pub const fn digest(&self) -> SourceDigest {
        self.digest
    }

    /// Confronto di revalidation: le tre grandezze insieme, non una sola.
    /// Il core lo usa dopo aver rieseguito il preflight leggero.
    #[must_use]
    pub fn matches(&self, observed: &Self) -> bool {
        self == observed
    }
}

#[derive(Debug)]
struct ContextInner {
    pipeline_id: u64,
    deadline: Instant,
    cancellation: CancellationToken,
    limits: PipelineLimits,
    observation: Observation,
    memory: Gauge,
    spill: Gauge,
    entries_visited: AtomicU64,
    digest: DigestAccumulator,
    pool: Option<ResourcePool>,
}

/// Stato condiviso da tutte le operazioni della stessa pipeline (INV-4).
///
/// E' un handle su stato condiviso: clonarlo non duplica i contatori, li
/// condivide. `Send + Sync`.
#[derive(Clone, Debug)]
pub struct PipelineContext {
    inner: Arc<ContextInner>,
}

impl PipelineContext {
    #[must_use]
    pub fn deadline(&self) -> Instant {
        self.inner.deadline
    }

    #[must_use]
    pub fn limits(&self) -> &PipelineLimits {
        &self.inner.limits
    }

    #[must_use]
    pub fn cancellation(&self) -> &CancellationToken {
        &self.inner.cancellation
    }

    #[must_use]
    pub fn resource_pool(&self) -> Option<&ResourcePool> {
        self.inner.pool.as_ref()
    }

    #[must_use]
    pub fn is_same_pipeline(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    #[must_use]
    pub fn remaining_duration(&self) -> Option<Duration> {
        self.inner.deadline.checked_duration_since(Instant::now())
    }

    /// Verifica che la pipeline sia ancora eseguibile.
    ///
    /// Le due condizioni non sono conflate: la cancellazione del chiamante
    /// produce un errore `Cancelled`, la deadline scaduta un errore di
    /// limite. Un consumer distingue "l'utente ha chiesto stop" da "il
    /// budget temporale e' finito".
    ///
    /// # Errors
    ///
    /// Restituisce l'errore di cancellazione se il token e' cancellato, o
    /// [`PlenoraIoError::LimitExceeded`] se la deadline e' passata.
    pub fn ensure_active(&self) -> Result<()> {
        if self.inner.cancellation.is_cancelled() {
            // La fase reale non e' nota al context: la porta l'`ErrorContext`
            // strutturato di S9. `Validate` e' la fase neutra pre-operazione.
            return Err(PlenoraIoError::cancelled(ErrorPhase::Validate, false));
        }
        if self.remaining_duration().is_none() {
            return Err(limit_error(DURATION_EXHAUSTED));
        }
        Ok(())
    }

    #[must_use]
    pub fn observed_input(&self) -> ObservedInput {
        self.inner.observation.state()
    }

    #[must_use]
    pub fn entries_visited(&self) -> u64 {
        self.inner.entries_visited.load(Ordering::Acquire)
    }

    #[must_use]
    pub fn remaining_memory(&self) -> u64 {
        self.inner.memory.remaining()
    }

    #[must_use]
    pub fn remaining_spill(&self) -> u64 {
        self.inner.spill.remaining()
    }

    /// Osserva l'input consumando il `permit` e registra il footprint.
    ///
    /// Unica fabbrica di [`SourceFootprint`] e unico canale di
    /// registrazione (INV-13). One-shot per costruzione: il permit e' preso
    /// per `move` e non e' `Clone`, quindi una seconda osservazione con lo
    /// stesso permit non e' scrivibile.
    ///
    /// Il conteggio delle entry e il digest **non** sono parametri: vengono
    /// dal context, che li ha accumulati durante l'enumerazione via
    /// [`Self::note_entry_visited`]. Passarli dall'esterno avrebbe creato
    /// due sorgenti di verita' per lo stesso dato e reso fabbricabile il
    /// valore che la revalidation confronta.
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::LimitExceeded`] se il permit appartiene
    /// a un'altra pipeline o se un'osservazione risulta gia' registrata su
    /// questo context, e l'errore di [`Self::ensure_active`] se la pipeline
    /// non e' piu' attiva. In ogni caso di errore lo stato resta
    /// [`ObservedInput::NotObserved`].
    // Il passaggio per valore e' l'invariante, non una svista: il permit e'
    // one-shot e non `Clone`, quindi consumarlo qui e' cio' che rende
    // impossibile una seconda osservazione. Prenderlo per riferimento — o
    // renderlo `Copy`, come suggerisce il lint — riaprirebbe esattamente il
    // buco che INV-13 chiude.
    #[allow(clippy::needless_pass_by_value)]
    pub fn observe_input(&self, permit: InputPermit, bytes: u64) -> Result<SourceFootprint> {
        // Destrutturare consuma il permit qui, non al termine dello scope:
        // dopo questa riga non esiste piu' un valore spendibile altrove.
        let InputPermit { pipeline_id } = permit;
        if pipeline_id != self.inner.pipeline_id {
            return Err(limit_error(PERMIT_FOREIGN));
        }
        self.ensure_active()?;
        if self
            .inner
            .observation
            .claimed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(limit_error(INPUT_ALREADY_OBSERVED));
        }
        self.inner.observation.bytes.store(bytes, Ordering::Release);
        self.inner
            .observation
            .published
            .store(true, Ordering::Release);
        Ok(SourceFootprint {
            total_bytes: bytes,
            entries_visited: self.entries_visited(),
            digest: self.inner.digest.finish(),
        })
    }

    /// Registra una entry visitata durante l'enumerazione della sorgente:
    /// applica `max_input_entries` (INV-9) prima che i byte vengano sommati,
    /// e assorbe l'identita' dell'entry nel digest del footprint.
    ///
    /// Le due cose stanno nello stesso metodo perche' sono lo stesso atto:
    /// l'insieme su cui si conta e' l'insieme di cui si calcola il digest.
    /// Tenerle separate avrebbe permesso un conteggio senza identita', cioe'
    /// un footprint che dichiara N entry senza sapere quali.
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::LimitExceeded`] se l'entry supererebbe
    /// il limite configurato o se il conteggio andrebbe in overflow, e
    /// l'errore di [`Self::ensure_active`] se la pipeline non e' attiva. Un
    /// rifiuto non incrementa il contatore e non tocca il digest.
    pub fn note_entry_visited(&self, entry: &SourceEntry<'_>) -> Result<()> {
        self.ensure_active()?;
        let ceiling = self.inner.limits.max_input_entries;
        let mut current = self.inner.entries_visited.load(Ordering::Acquire);
        loop {
            let Some(next) = current.checked_add(1) else {
                return Err(limit_error(ENTRIES_OVERFLOW));
            };
            if next > ceiling {
                return Err(limit_error(TOO_MANY_ENTRIES));
            }
            match self.inner.entries_visited.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    self.inner.digest.absorb(entry);
                    return Ok(());
                }
                Err(observed) => current = observed,
            }
        }
    }

    /// Prenota memoria che la libreria detiene **internamente** (buffer del
    /// batch worker, coda dello spool, staging del writer). La lease e'
    /// restituita al drop, cioe' al transfer del batch al consumer (INV-5).
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::LimitExceeded`] se `bytes` e' zero o se
    /// la quota residua — locale, e quella del pool quando presente — non
    /// basta; l'errore di [`Self::ensure_active`] se la pipeline non e'
    /// attiva.
    pub fn lease_memory_internal(&self, bytes: u64) -> Result<InternalMemoryLease> {
        self.take_shared(bytes, GaugeKind::Memory)?;
        Ok(InternalMemoryLease {
            context: self.clone(),
            bytes,
            not_sync: PhantomData,
        })
    }

    /// Prenota spazio di spill su disco. Restituita al drop, insieme alla
    /// rimozione del file temporaneo da parte del chiamante.
    ///
    /// # Errors
    ///
    /// Come [`Self::lease_memory_internal`], sulla quota di spill.
    pub fn lease_spill(&self, bytes: u64) -> Result<SpillLease> {
        self.take_shared(bytes, GaugeKind::Spill)?;
        Ok(SpillLease {
            context: self.clone(),
            bytes,
            not_sync: PhantomData,
        })
    }

    /// Prenota uno slot di concorrenza.
    ///
    /// Senza [`ResourcePool`] non esiste alcuna quota di concorrenza: la
    /// lease e' un no-op che non conta nulla (INV-12). "No-op" significa
    /// "nessuna quota", non "non fallisce mai": il controllo di
    /// cancellazione e deadline resta.
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::LimitExceeded`] se il pool e' presente
    /// e la sua quota e' esaurita, e l'errore di [`Self::ensure_active`] se
    /// la pipeline non e' attiva.
    pub fn lease_concurrency(&self) -> Result<ConcurrencyLease> {
        self.ensure_active()?;
        let Some(pool) = self.inner.pool.as_ref() else {
            return Ok(ConcurrencyLease {
                pool: None,
                not_sync: PhantomData,
            });
        };
        if pool.inner.concurrency.try_take(1) {
            Ok(ConcurrencyLease {
                pool: Some(pool.clone()),
                not_sync: PhantomData,
            })
        } else {
            Err(limit_error(CONCURRENCY_EXHAUSTED))
        }
    }

    /// Regola unica di composizione locale ⊓ pool (INV-12): la lease passa
    /// solo se sta sotto **entrambe** le quote e consuma **entrambi** i
    /// gauge. Se il pool rifiuta, la quota locale gia' presa torna indietro:
    /// un rifiuto non lascia consumo.
    fn take_shared(&self, amount: u64, kind: GaugeKind) -> Result<()> {
        if amount == 0 {
            return Err(limit_error(LEASE_MUST_BE_POSITIVE));
        }
        self.ensure_active()?;
        let local = kind.local(&self.inner);
        if !local.try_take(amount) {
            return Err(limit_error(kind.exhausted()));
        }
        if let Some(pool) = self.inner.pool.as_ref() {
            if !kind.pooled(pool).try_take(amount) {
                local.give_back(amount);
                return Err(limit_error(kind.exhausted()));
            }
        }
        Ok(())
    }

    fn give_back_shared(&self, amount: u64, kind: GaugeKind) {
        kind.local(&self.inner).give_back(amount);
        if let Some(pool) = self.inner.pool.as_ref() {
            kind.pooled(pool).give_back(amount);
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum GaugeKind {
    Memory,
    Spill,
}

impl GaugeKind {
    const fn local(self, inner: &ContextInner) -> &Gauge {
        match self {
            Self::Memory => &inner.memory,
            Self::Spill => &inner.spill,
        }
    }

    fn pooled(self, pool: &ResourcePool) -> &Gauge {
        match self {
            Self::Memory => &pool.inner.memory,
            Self::Spill => &pool.inner.spill,
        }
    }

    const fn exhausted(self) -> &'static str {
        match self {
            Self::Memory => MEMORY_EXHAUSTED,
            Self::Spill => SPILL_EXHAUSTED,
        }
    }
}

/// Radice della costruzione, **non `Clone`**: e' un token one-shot, e due
/// radici che pretendono la stessa pipeline non avrebbero senso.
///
/// Non e' ottenibile separatamente dal permit: [`PipelineBudgetBuilder::build`]
/// lo consegna dentro un [`PipelineBundle`] opaco.
#[derive(Debug)]
pub struct PipelineBudget {
    context: PipelineContext,
}

impl PipelineBudget {
    #[must_use]
    pub fn builder() -> PipelineBudgetBuilder {
        PipelineBudgetBuilder::default()
    }

    #[must_use]
    pub const fn context(&self) -> &PipelineContext {
        &self.context
    }

    fn operation(&self) -> OperationBudget {
        OperationBudget::new(&self.context)
    }
}

/// Budget e permit emessi dalla stessa costruzione, tenuti insieme.
///
/// Il tipo e' **opaco**: nessun campo pubblico, non `Clone`. Non esiste un
/// punto in cui il chiamante accoppi a mano un permit con un budget, perche'
/// le uniche uscite sono le `into_*_parts`.
#[derive(Debug)]
pub struct PipelineBundle {
    budget: PipelineBudget,
    permit: InputPermit,
}

impl PipelineBundle {
    #[must_use]
    pub const fn context(&self) -> &PipelineContext {
        self.budget.context()
    }

    /// Parti per un `open`: preflight completo della sorgente, permit non
    /// ancora speso.
    #[must_use]
    pub fn into_read_parts(self) -> ReadBudgetParts {
        ReadBudgetParts {
            budget: self.budget.operation(),
            permit: Some(self.permit),
            expected: None,
        }
    }

    /// Parti per un `scan` su un `Dataset` gia' aperto: permit piu' lo
    /// snapshot atteso, che il preflight leggero del core rivalidera'.
    #[must_use]
    pub fn into_scan_parts(self, expected: SourceFootprintSnapshot) -> ScanBudgetParts {
        ScanBudgetParts {
            budget: self.budget.operation(),
            permit: Some(self.permit),
            expected,
        }
    }

    /// Parti per un `convert`: reader e writer con contatori indipendenti
    /// sotto lo stesso context (INV-3). Il permit viaggia sul ramo read.
    #[must_use]
    pub fn into_convert_parts(self) -> ConvertBudgetParts {
        let write = WriteBudgetParts {
            budget: self.budget.operation(),
        };
        let read = ReadBudgetParts {
            budget: self.budget.operation(),
            permit: Some(self.permit),
            expected: None,
        };
        ConvertBudgetParts { read, write }
    }

    /// Parti per una scrittura standalone: il permit **non** entra nelle
    /// parti e viene droppato con il bundle, quindi `observed_input()` resta
    /// `NotObserved` e l'expansion ratio non si applica (INV-6).
    #[must_use]
    pub fn into_write_parts(self) -> WriteBudgetParts {
        WriteBudgetParts {
            budget: self.budget.operation(),
        }
    }
}

/// I default vivono qui e non in `Option::unwrap_or_default` al momento del
/// `build`: un campo concreto rende esplicito quale valore la pipeline
/// ricevera' se il chiamante non lo imposta.
#[derive(Clone, Debug, Default)]
pub struct PipelineBudgetBuilder {
    limits: PipelineLimits,
    cancellation: CancellationToken,
    pool: Option<ResourcePool>,
}

impl PipelineBudgetBuilder {
    #[must_use]
    pub const fn limits(mut self, limits: PipelineLimits) -> Self {
        self.limits = limits;
        self
    }

    #[must_use]
    pub fn cancellation(mut self, token: CancellationToken) -> Self {
        self.cancellation = token;
        self
    }

    /// Aggancia un pool condiviso. Con il pool memory/spill contano contro
    /// **sia** la quota locale **sia** quella del pool (quota effettiva =
    /// minimo dei due) e la concorrenza e' governata solo dal pool; senza
    /// pool memory/spill restano locali e la concorrenza non esiste (INV-12).
    #[must_use]
    pub fn resource_pool(mut self, pool: ResourcePool) -> Self {
        self.pool = Some(pool);
        self
    }

    /// Costruisce la pipeline ed emette il permit di osservazione.
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::LimitExceeded`] se i limiti non
    /// superano [`PipelineLimits::validate`] o se la deadline non e'
    /// rappresentabile da [`Instant`].
    pub fn build(self) -> Result<PipelineBundle> {
        let limits = self.limits;
        limits.validate()?;
        let deadline = Instant::now()
            .checked_add(Duration::from_millis(limits.duration_ms))
            .ok_or_else(|| limit_error(DEADLINE_BEYOND_INSTANT))?;
        let pipeline_id = NEXT_PIPELINE_ID.fetch_add(1, Ordering::Relaxed);
        let context = PipelineContext {
            inner: Arc::new(ContextInner {
                pipeline_id,
                deadline,
                cancellation: self.cancellation,
                limits,
                observation: Observation::new(),
                memory: Gauge::new(limits.memory_bytes),
                spill: Gauge::new(limits.spill_bytes),
                entries_visited: AtomicU64::new(0),
                digest: DigestAccumulator::new(),
                pool: self.pool,
            }),
        };
        Ok(PipelineBundle {
            budget: PipelineBudget { context },
            permit: InputPermit { pipeline_id },
        })
    }
}

/// Contatori cumulativi per operazione (INV-3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum OperationCounter {
    Rows,
    Columns,
    GeometryComponents,
    OutputBytes,
}

#[derive(Debug)]
struct OperationCounters {
    rows: Gauge,
    columns: Gauge,
    geometry_components: Gauge,
    output_bytes: Gauge,
}

impl OperationCounters {
    const fn get(&self, counter: OperationCounter) -> &Gauge {
        match counter {
            OperationCounter::Rows => &self.rows,
            OperationCounter::Columns => &self.columns,
            OperationCounter::GeometryComponents => &self.geometry_components,
            OperationCounter::OutputBytes => &self.output_bytes,
        }
    }
}

/// Budget di una singola operazione (un reader oppure un writer).
///
/// Tipo pubblico ma **opaco** e workspace-internal: serve al boundary
/// model→core perche' i driver operino sui contatori, e non sara'
/// ri-esportato da alcuna facade. `Clone` via `Arc`: tutti i cloni vedono
/// gli stessi contatori, quindi un clone non raddoppia il consumo.
#[derive(Clone, Debug)]
pub struct OperationBudget {
    context: PipelineContext,
    counters: Arc<OperationCounters>,
}

impl OperationBudget {
    fn new(context: &PipelineContext) -> Self {
        let limits = context.inner.limits;
        Self {
            context: context.clone(),
            counters: Arc::new(OperationCounters {
                rows: Gauge::new(limits.max_rows),
                columns: Gauge::new(limits.max_columns),
                geometry_components: Gauge::new(limits.max_geometry_components),
                output_bytes: Gauge::new(limits.max_output_bytes),
            }),
        }
    }

    #[must_use]
    pub const fn context(&self) -> &PipelineContext {
        &self.context
    }

    #[must_use]
    pub fn remaining(&self, counter: OperationCounter) -> u64 {
        self.counters.get(counter).remaining()
    }

    #[must_use]
    pub fn shares_counters_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.counters, &other.counters)
    }

    /// Tetto assoluto dell'output (INV-6).
    ///
    /// `output_expansion_ratio` si applica **solo** con un input osservato e
    /// non vuoto: `NotObserved` significa che nessun preflight ha girato, e
    /// `Bytes(0)` che l'input era vuoto — in nessuno dei due casi un
    /// prodotto per zero deve diventare un tetto che vieta ogni output.
    #[must_use]
    pub fn output_limit(&self) -> u64 {
        let absolute = self.context.inner.limits.max_output_bytes;
        match self.context.observed_input() {
            ObservedInput::NotObserved | ObservedInput::Bytes(0) => absolute,
            ObservedInput::Bytes(observed) => absolute
                .min(observed.saturating_mul(self.context.inner.limits.output_expansion_ratio)),
        }
    }

    /// Preleva quota da un contatore cumulativo dell'operazione.
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::LimitExceeded`] se `amount` e' zero, se
    /// il contatore e' esaurito o se — per [`OperationCounter::OutputBytes`]
    /// — il consumo supererebbe il tetto derivato dall'input osservato;
    /// l'errore di [`PipelineContext::ensure_active`] se la pipeline non e'
    /// attiva.
    pub fn try_lease(&self, counter: OperationCounter, amount: u64) -> Result<CountedLease> {
        if amount == 0 {
            return Err(limit_error(LEASE_MUST_BE_POSITIVE));
        }
        self.context.ensure_active()?;
        if counter == OperationCounter::OutputBytes {
            let gauge = self.counters.get(counter);
            let consumed = gauge.capacity().saturating_sub(gauge.remaining());
            let projected = consumed
                .checked_add(amount)
                .ok_or_else(|| limit_error(COUNTER_EXHAUSTED))?;
            if projected > self.output_limit() {
                return Err(limit_error(OUTPUT_LIMIT_EXCEEDED));
            }
        }
        if self.counters.get(counter).try_take(amount) {
            Ok(CountedLease {
                budget: self.clone(),
                counter,
                amount,
                released: false,
                not_sync: PhantomData,
            })
        } else {
            Err(limit_error(COUNTER_EXHAUSTED))
        }
    }
}

/// Lease della memoria detenuta internamente dalla libreria. `Send`, non
/// `Sync`: una lease ha un solo proprietario del drop.
///
/// Tipo pubblico ma opaco e workspace-internal: non viene restituito insieme
/// al batch e non sara' ri-esportato dalla facade.
#[derive(Debug)]
pub struct InternalMemoryLease {
    context: PipelineContext,
    bytes: u64,
    not_sync: PhantomData<Cell<()>>,
}

impl InternalMemoryLease {
    #[must_use]
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }
}

impl Drop for InternalMemoryLease {
    fn drop(&mut self) {
        self.context.give_back_shared(self.bytes, GaugeKind::Memory);
    }
}

/// Lease dello spazio di spill. `Send`, non `Sync`.
#[derive(Debug)]
pub struct SpillLease {
    context: PipelineContext,
    bytes: u64,
    not_sync: PhantomData<Cell<()>>,
}

impl SpillLease {
    #[must_use]
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }
}

impl Drop for SpillLease {
    fn drop(&mut self) {
        self.context.give_back_shared(self.bytes, GaugeKind::Spill);
    }
}

/// Slot di concorrenza. Conta solo finche' e' viva, e conta solo se la
/// pipeline ha un [`ResourcePool`]. `Send`, non `Sync`.
#[derive(Debug)]
pub struct ConcurrencyLease {
    pool: Option<ResourcePool>,
    not_sync: PhantomData<Cell<()>>,
}

impl ConcurrencyLease {
    /// `true` se la lease conta contro un pool condiviso; `false` se e' il
    /// no-op di una pipeline senza pool.
    #[must_use]
    pub const fn is_counted(&self) -> bool {
        self.pool.is_some()
    }
}

impl Drop for ConcurrencyLease {
    fn drop(&mut self) {
        if let Some(pool) = self.pool.as_ref() {
            pool.inner.concurrency.give_back(1);
        }
    }
}

/// Lease di un contatore cumulativo. `Send`, non `Sync`.
///
/// Il drop senza `commit`/`release` restituisce l'intera quota: una lease
/// dimenticata non consuma budget in silenzio.
#[derive(Debug)]
pub struct CountedLease {
    budget: OperationBudget,
    counter: OperationCounter,
    amount: u64,
    released: bool,
    not_sync: PhantomData<Cell<()>>,
}

impl CountedLease {
    #[must_use]
    pub const fn amount(&self) -> u64 {
        self.amount
    }

    #[must_use]
    pub const fn counter(&self) -> OperationCounter {
        self.counter
    }

    /// Consuma `used` e restituisce al contatore la parte inutilizzata.
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::LimitExceeded`] se `used` e' zero o
    /// supera la quota prelevata.
    pub fn commit(mut self, used: u64) -> Result<()> {
        if used == 0 || used > self.amount {
            return Err(limit_error(COMMIT_NOT_VALID));
        }
        let unused = self.amount - used;
        if unused > 0 {
            self.budget.counters.get(self.counter).give_back(unused);
        }
        self.released = true;
        Ok(())
    }

    /// Restituisce al contatore l'intera quota prelevata.
    pub fn release(mut self) {
        self.budget
            .counters
            .get(self.counter)
            .give_back(self.amount);
        self.released = true;
    }
}

impl Drop for CountedLease {
    fn drop(&mut self) {
        if !self.released {
            self.budget
                .counters
                .get(self.counter)
                .give_back(self.amount);
            self.released = true;
        }
    }
}

mod sealed {
    pub trait Sealed {}

    impl Sealed for super::ReadBudgetParts {}
    impl Sealed for super::ScanBudgetParts {}
    impl Sealed for super::WriteBudgetParts {}
}

/// Parti opache che alimentano la factory di lettura del core.
///
/// Sealed: implementato **solo** da [`ReadBudgetParts`] e
/// [`ScanBudgetParts`]. Nessun tipo fuori da questo crate puo' implementarlo,
/// quindi `ReadOptions::builder` non e' alimentabile da un budget fabbricato
/// altrove (INV-2).
pub trait IntoReadParts: sealed::Sealed {
    #[doc(hidden)]
    fn into_read_budget_parts(self) -> ReadBudgetParts;
}

/// Parti opache che alimentano la factory di scrittura del core. Sealed:
/// implementato **solo** da [`WriteBudgetParts`].
pub trait IntoWriteParts: sealed::Sealed {
    #[doc(hidden)]
    fn into_write_budget_parts(self) -> WriteBudgetParts;
}

/// Parti di un `open`. Opaco, non `Clone`.
#[derive(Debug)]
pub struct ReadBudgetParts {
    budget: OperationBudget,
    permit: Option<InputPermit>,
    expected: Option<SourceFootprintSnapshot>,
}

impl ReadBudgetParts {
    #[must_use]
    pub const fn budget(&self) -> &OperationBudget {
        &self.budget
    }

    #[must_use]
    pub const fn expected_footprint(&self) -> Option<&SourceFootprintSnapshot> {
        self.expected.as_ref()
    }

    /// Estrae il permit trasportato dalle parti; `None` se gia' estratto o
    /// se le parti non ne trasportavano. L'unico chiamante legittimo e' il
    /// preflight del core: il permit resta legato al context di queste
    /// stesse parti, quindi estrarlo non consente alcun incrocio.
    pub const fn take_input_permit(&mut self) -> Option<InputPermit> {
        self.permit.take()
    }
}

impl IntoReadParts for ReadBudgetParts {
    /// Identita': e' gia' la rappresentazione read interna.
    fn into_read_budget_parts(self) -> Self {
        self
    }
}

/// Parti di un `scan`: come le parti read, piu' lo snapshot atteso.
#[derive(Debug)]
pub struct ScanBudgetParts {
    budget: OperationBudget,
    permit: Option<InputPermit>,
    expected: SourceFootprintSnapshot,
}

impl ScanBudgetParts {
    #[must_use]
    pub const fn budget(&self) -> &OperationBudget {
        &self.budget
    }

    #[must_use]
    pub const fn expected_footprint(&self) -> &SourceFootprintSnapshot {
        &self.expected
    }
}

impl IntoReadParts for ScanBudgetParts {
    /// Conversione, non identita': budget e permit passano **invariati** —
    /// nessun contatore ricreato, nessun permit rigenerato — e lo snapshot
    /// atteso finisce nel campo che il core legge per la revalidation.
    fn into_read_budget_parts(self) -> ReadBudgetParts {
        ReadBudgetParts {
            budget: self.budget,
            permit: self.permit,
            expected: Some(self.expected),
        }
    }
}

/// Parti di una scrittura. Opaco, non `Clone`. Non trasporta permit: il ramo
/// write non osserva input.
#[derive(Debug)]
pub struct WriteBudgetParts {
    budget: OperationBudget,
}

impl WriteBudgetParts {
    #[must_use]
    pub const fn budget(&self) -> &OperationBudget {
        &self.budget
    }
}

impl IntoWriteParts for WriteBudgetParts {
    /// Identita'.
    fn into_write_budget_parts(self) -> Self {
        self
    }
}

/// Parti di un `convert`: i due rami sotto lo stesso context.
#[derive(Debug)]
pub struct ConvertBudgetParts {
    read: ReadBudgetParts,
    write: WriteBudgetParts,
}

impl ConvertBudgetParts {
    /// Divide le parti nei due rami read/write. I contatori cumulativi sono
    /// indipendenti, il [`PipelineContext`] e' lo stesso: e' esattamente la
    /// combinazione che elimina il doppio conteggio di L0.10 senza perdere
    /// le grandezze condivise.
    #[must_use]
    pub fn into_parts(self) -> (ReadBudgetParts, WriteBudgetParts) {
        (self.read, self.write)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn entry(path: &[u8]) -> SourceEntry<'_> {
        SourceEntry::new(
            path,
            1_024,
            Some(UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
        )
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
        let mut opened = bundle().into_read_parts();
        let permit = opened.take_input_permit().expect("il permit deve esserci");
        let footprint = opened
            .budget()
            .context()
            .observe_input(permit, 0)
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
        let mut parts = bundle_with(limits).into_read_parts();
        let permit = parts.take_input_permit().expect("il permit deve esserci");
        parts
            .budget()
            .context()
            .observe_input(permit, 0)
            .expect("un input vuoto e' osservabile");
        assert_eq!(
            parts.budget().output_limit(),
            1_000,
            "un input vuoto non deve produrre un tetto zero"
        );
    }

    #[test]
    fn output_limit_applies_expansion_when_bytes_positive() {
        let limits = PipelineLimits::default()
            .with_max_output_bytes(1_000)
            .with_output_expansion_ratio(3);
        let mut parts = bundle_with(limits).into_read_parts();
        let permit = parts.take_input_permit().expect("il permit deve esserci");
        parts
            .budget()
            .context()
            .observe_input(permit, 100)
            .expect("l'osservazione deve riuscire");
        assert_eq!(parts.budget().output_limit(), 300);
    }

    #[test]
    fn convert_writer_sees_input_observed_by_reader() {
        let limits = PipelineLimits::default()
            .with_max_output_bytes(1_000)
            .with_output_expansion_ratio(3);
        let (mut read, write) = bundle_with(limits).into_convert_parts().into_parts();
        let permit = read.take_input_permit().expect("il permit deve esserci");
        read.budget()
            .context()
            .observe_input(permit, 100)
            .expect("l'osservazione deve riuscire");
        assert_eq!(
            write.budget().output_limit(),
            300,
            "il writer legge l'input osservato dal reader nel context condiviso"
        );
    }

    #[test]
    fn observe_input_consumes_permit_and_yields_footprint() {
        let mut parts = bundle().into_read_parts();
        let permit = parts.take_input_permit().expect("il permit deve esserci");
        for path in [
            b"a.csv".as_slice(),
            b"b.csv".as_slice(),
            b"c.csv".as_slice(),
        ] {
            parts
                .budget()
                .context()
                .note_entry_visited(&entry(path))
                .expect("l'enumerazione deve passare");
        }
        let footprint = parts
            .budget()
            .context()
            .observe_input(permit, 4_096)
            .expect("l'osservazione deve riuscire");
        assert_eq!(footprint.total_bytes(), 4_096);
        assert_eq!(
            footprint.entries_visited(),
            3,
            "le entry vengono dal context, non da un parametro fabbricabile"
        );
        assert_eq!(
            parts.budget().context().observed_input(),
            ObservedInput::Bytes(4_096)
        );
        assert!(
            parts.take_input_permit().is_none(),
            "il permit e' one-shot: una seconda estrazione non deve darne un altro"
        );
    }

    #[test]
    fn observe_input_with_permit_from_other_pipeline_is_rejected() {
        let mut foreign = bundle().into_read_parts();
        let permit = foreign.take_input_permit().expect("il permit deve esserci");
        let target = bundle();
        assert!(target.context().observe_input(permit, 1).is_err());
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
        let mut parts = built.into_read_parts();
        let permit = parts.take_input_permit().expect("il permit deve esserci");
        token.cancel();
        assert!(parts.budget().context().observe_input(permit, 512).is_err());
        assert_eq!(
            parts.budget().context().observed_input(),
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
        let mut opened = bundle().into_read_parts();
        let permit = opened.take_input_permit().expect("il permit deve esserci");
        for index in 0..5_u8 {
            let path = format!("parte-{index}.csv");
            opened
                .budget()
                .context()
                .note_entry_visited(&entry(path.as_bytes()))
                .expect("l'enumerazione deve passare");
        }
        let footprint = opened
            .budget()
            .context()
            .observe_input(permit, 2_048)
            .expect("l'osservazione deve riuscire");

        let scan = bundle().into_scan_parts(footprint.snapshot());
        assert_eq!(scan.expected_footprint().total_bytes(), 2_048);
        assert_eq!(scan.expected_footprint().entries_visited(), 5);
        assert_eq!(scan.expected_footprint().digest(), footprint.digest());

        let mut read = scan.into_read_budget_parts();
        assert!(
            read.expected_footprint().is_some(),
            "la conversione scan->read deve preservare lo snapshot atteso"
        );
        assert!(
            read.take_input_permit().is_some(),
            "la conversione scan->read deve preservare il permit"
        );
    }

    #[test]
    fn convert_parts_split_into_read_and_write() {
        let (mut read, write) = bundle().into_convert_parts().into_parts();
        assert!(
            read.take_input_permit().is_some(),
            "il permit viaggia sul ramo read"
        );
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
        let mut parts = bundle_with(limits).into_read_parts();
        let permit = parts.take_input_permit().expect("il permit deve esserci");
        parts
            .budget()
            .context()
            .observe_input(permit, 100)
            .expect("l'osservazione deve riuscire");
        assert_eq!(parts.budget().output_limit(), 200);
        parts
            .budget()
            .try_lease(OperationCounter::OutputBytes, 200)
            .expect("il tetto derivato consente 200 byte")
            .commit(200)
            .expect("il commit deve riuscire");
        assert!(
            parts
                .budget()
                .try_lease(OperationCounter::OutputBytes, 1)
                .is_err(),
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

    fn digest_of(entries: &[SourceEntry<'_>], bytes: u64) -> SourceFootprintSnapshot {
        let mut parts = bundle().into_read_parts();
        let permit = parts.take_input_permit().expect("il permit deve esserci");
        for visited in entries {
            parts
                .budget()
                .context()
                .note_entry_visited(visited)
                .expect("l'enumerazione deve passare");
        }
        parts
            .budget()
            .context()
            .observe_input(permit, bytes)
            .expect("l'osservazione deve riuscire")
            .snapshot()
    }

    #[test]
    fn footprint_digest_is_stable_for_the_same_entry_set() {
        let first = digest_of(&[entry(b"a.csv"), entry(b"b.csv")], 10);
        let second = digest_of(&[entry(b"a.csv"), entry(b"b.csv")], 10);
        assert_eq!(first.digest(), second.digest());
        assert!(first.matches(&second));
    }

    #[test]
    fn footprint_digest_is_order_insensitive() {
        // L'ordine di enumerazione di una directory non e' stabile: un
        // digest che ne dipendesse segnalerebbe mutazioni inesistenti.
        let ascending = digest_of(&[entry(b"a.csv"), entry(b"b.csv"), entry(b"c.csv")], 10);
        let descending = digest_of(&[entry(b"c.csv"), entry(b"b.csv"), entry(b"a.csv")], 10);
        assert_eq!(ascending.digest(), descending.digest());
    }

    #[test]
    fn footprint_digest_detects_added_and_removed_entries() {
        let two = digest_of(&[entry(b"a.csv"), entry(b"b.csv")], 10);
        let three = digest_of(&[entry(b"a.csv"), entry(b"b.csv"), entry(b"c.csv")], 10);
        let one = digest_of(&[entry(b"a.csv")], 10);
        assert_ne!(two.digest(), three.digest(), "un'aggiunta cambia il digest");
        assert_ne!(two.digest(), one.digest(), "una rimozione cambia il digest");
    }

    #[test]
    fn footprint_digest_detects_rename_size_and_mtime() {
        let base = digest_of(&[entry(b"a.csv")], 10);

        let renamed = digest_of(&[entry(b"a-bis.csv")], 10);
        assert_ne!(base.digest(), renamed.digest());

        let resized = digest_of(
            &[SourceEntry::new(
                b"a.csv",
                2_048,
                Some(UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
            )],
            10,
        );
        assert_ne!(base.digest(), resized.digest());

        let touched = digest_of(
            &[SourceEntry::new(
                b"a.csv",
                1_024,
                Some(UNIX_EPOCH + Duration::from_secs(1_700_000_001)),
            )],
            10,
        );
        assert_ne!(base.digest(), touched.digest());

        let without_mtime = digest_of(&[SourceEntry::new(b"a.csv", 1_024, None)], 10);
        assert_ne!(base.digest(), without_mtime.digest());
    }

    #[test]
    fn footprint_digest_separates_paths_that_share_a_concatenation() {
        // Senza la lunghezza in testa alla codifica, "ab" + "c" e "a" + "bc"
        // darebbero la stessa sequenza di byte.
        let first = digest_of(&[entry(b"ab"), entry(b"c")], 10);
        let second = digest_of(&[entry(b"a"), entry(b"bc")], 10);
        assert_ne!(first.digest(), second.digest());
    }

    #[test]
    fn snapshot_matches_only_when_bytes_entries_and_digest_agree() {
        let base = digest_of(&[entry(b"a.csv")], 10);
        let other_bytes = digest_of(&[entry(b"a.csv")], 11);
        let other_entries = digest_of(&[entry(b"a.csv"), entry(b"b.csv")], 10);
        assert!(
            !base.matches(&other_bytes),
            "i byte fanno parte del confronto"
        );
        assert!(!base.matches(&other_entries));
    }

    #[test]
    fn snapshot_roundtrips_through_serde_without_losing_the_digest() {
        let snapshot = digest_of(&[entry(b"a.csv"), entry(b"b.csv")], 42);
        let encoded = serde_json::to_string(&snapshot).expect("serializzabile");
        let decoded: SourceFootprintSnapshot =
            serde_json::from_str(&encoded).expect("deserializzabile");
        assert!(snapshot.matches(&decoded));
    }

    #[test]
    fn rejected_entry_does_not_enter_the_digest() {
        let limits = PipelineLimits::default().with_max_input_entries(1);
        let mut parts = bundle_with(limits).into_read_parts();
        let permit = parts.take_input_permit().expect("il permit deve esserci");
        let context = parts.budget().context();
        context
            .note_entry_visited(&entry(b"a.csv"))
            .expect("prima entry");
        assert!(context.note_entry_visited(&entry(b"b.csv")).is_err());
        let observed = context
            .observe_input(permit, 10)
            .expect("l'osservazione deve riuscire")
            .snapshot();
        assert!(
            observed.matches(&digest_of(&[entry(b"a.csv")], 10)),
            "l'entry rifiutata non deve lasciare traccia nel digest"
        );
    }

    #[test]
    fn effective_wkb_components_is_tightened_by_max_vertices() {
        // `--max-vertices` e' un flag vivo della CLI: il modello unificato
        // deve applicarlo come lo applica `Limits::effective_wkb()`, o la
        // migrazione allenterebbe un tetto che l'utente ha stretto.
        let limits = PipelineLimits::default()
            .with_max_wkb_components(10)
            .with_max_vertices(3);
        assert_eq!(limits.effective_wkb_components(), 3);

        let legacy = crate::limits::Limits {
            max_vertices: 3,
            wkb: crate::limits::WkbLimits {
                max_components: 10,
                ..crate::limits::WkbLimits::default()
            },
            ..crate::limits::Limits::default()
        };
        assert_eq!(
            limits.effective_wkb_components(),
            legacy.effective_wkb().max_components,
            "la composizione deve coincidere con quella del modello legacy"
        );
    }

    #[test]
    fn unified_defaults_are_never_looser_than_either_legacy_model() {
        // Il finding L0.2 era proprio la divergenza fra i due modelli. La
        // regola di unificazione — vince il piu' stretto — resta verificata
        // contro i default legacy finche' esistono, cosi' una modifica
        // dell'uno o dell'altro non passa inosservata.
        let unified = PipelineLimits::default();
        let legacy = crate::limits::Limits::default();
        let resource = crate::resource::ResourceLimits::default();

        assert!(unified.max_input_bytes() <= legacy.max_input_bytes);
        assert!(unified.max_rows() <= resource.rows);
        assert!(unified.max_rows() <= legacy.max_rows as u64);
        assert!(unified.max_columns() <= resource.columns);
        assert!(unified.max_columns() <= legacy.max_columns as u64);
        assert!(unified.max_output_bytes() <= resource.output_bytes);
        assert!(unified.max_output_bytes() <= legacy.max_output_bytes);
        assert!(unified.max_geometry_components() <= resource.geometry_components);
        assert!(unified.memory_bytes() <= resource.memory_bytes);
        assert!(unified.spill_bytes() <= resource.spill_bytes);
        assert!(unified.duration_ms() <= resource.duration_ms);
        assert!(unified.decompression_ratio() <= resource.decompression_ratio);
        assert!(unified.output_expansion_ratio() <= resource.output_expansion_ratio);
        assert_eq!(unified.max_vertices(), legacy.max_vertices);
        assert_eq!(unified.max_wkb_components(), legacy.wkb.max_components);
        assert_eq!(unified.max_wkb_depth(), legacy.wkb.max_depth);
        assert_eq!(unified.max_wkb_cell_bytes(), legacy.wkb.max_cell_bytes);
        assert_eq!(
            unified.effective_wkb_components(),
            legacy.effective_wkb().max_components
        );
    }

    #[test]
    fn distinct_pipelines_have_distinct_identities() {
        let first = bundle();
        let second = bundle();
        let handle = first.context().clone();
        assert!(first.context().is_same_pipeline(&handle));
        assert!(!first.context().is_same_pipeline(second.context()));
    }
}
